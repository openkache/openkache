//! Storage runtime design example.
//!
//! 실제 모듈에 연결하지 않은 예시다. 하나의 storage thread에서 다음 흐름만 보여준다.
//!
//! request 수신 -> task poll -> SQE 제출 -> CQE 수신 -> 같은 task 재개 -> response 전송

use std::collections::VecDeque;
use std::fs::File;
use std::future::Future;
use std::io;
use std::os::fd::AsRawFd;
use std::pin::Pin;
use std::ptr::NonNull;
use std::task::{Context, Poll, Waker};

use io_uring::{IoUring, opcode, squeue, types};

const BUCKET_BYTES: usize = 4 * 1024;

/// io_uring read에 직접 넘기는 4KiB 정렬 버퍼다.
#[repr(C, align(4096))]
struct BucketBuffer {
    bytes: [u8; BUCKET_BYTES],
}

impl BucketBuffer {
    const fn new() -> Self {
        Self {
            bytes: [0; BUCKET_BYTES],
        }
    }
}

/// Task slab의 한 칸을 가리킨다.
///
/// index가 재사용되더라도 이전 CQE가 새 task를 깨우지 못하도록 generation을 함께 비교한다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TaskId {
    index: u32,
    generation: u32,
}

/// I/O slab의 한 칸을 가리킨다.
/// SQE user_data에는 이 값을 u64로 인코딩해서 넣는다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IoId {
    index: u32,
    generation: u32,
}

impl IoId {
    const fn encode(self) -> u64 {
        (self.generation as u64) << 32 | self.index as u64
    }

    const fn decode(encoded: u64) -> Self {
        Self {
            index: encoded as u32,
            generation: (encoded >> 32) as u32,
        }
    }
}

/// 어느 network core의 어떤 request에 응답해야 하는지 나타낸다.
#[derive(Clone, Copy)]
struct ResponseTarget {
    network_core_index: usize,
    sequence: u32,
}

enum StorageRequest {
    ReadBucket { file_offset: u64 },
}

enum StorageResponse {
    Bucket(Box<BucketBuffer>),
    IoError(io::Error),
}

/// Table과 SG 상태가 들어갈 자리다.
/// 이 예시에서는 빌림이 await 전에 끝나고 CQE 뒤에 다시 시작되는 것만 확인한다.
#[derive(Default)]
struct Storage {
    started_reads: u64,
    completed_reads: u64,
}

/// 하나의 request future가 storage와 I/O에 접근할 때 사용하는 작은 손잡이다.
#[derive(Clone, Copy)]
struct StorageContext {
    task_id: TaskId,
    storage: NonNull<Storage>,
    io: NonNull<IoDriver>,
}

impl StorageContext {
    /// Storage 빌림은 closure가 반환될 때 반드시 끝난다.
    fn with_storage<R>(&self, operation: impl FnOnce(&mut Storage) -> R) -> R {
        // SAFETY: Runtime은 한 thread에서 한 번에 하나의 task만 poll한다.
        // Storage는 Box에 들어 있어 모든 task가 끝날 때까지 주소가 바뀌지 않는다.
        unsafe { operation(&mut *self.storage.as_ptr()) }
    }

    fn read_bucket(&self, file_offset: u64) -> ReadBucket {
        ReadBucket {
            context: *self,
            file_offset,
            io_id: None,
        }
    }
}

/// 첫 poll에서 SQE를 만들고, CQE가 기록된 뒤의 poll에서 결과를 가져간다.
struct ReadBucket {
    context: StorageContext,
    file_offset: u64,
    io_id: Option<IoId>,
}

impl Future for ReadBucket {
    type Output = io::Result<Box<BucketBuffer>>;

    fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: Runtime은 한 번에 하나의 task만 poll하고 그 사이 CQE를 처리하지 않는다.
        let io = unsafe { &mut *self.context.io.as_ptr() };

        if let Some(io_id) = self.io_id {
            return match io.take_read_result(io_id) {
                Some(result) => Poll::Ready(result),
                None => Poll::Pending,
            };
        }

        match io.submit_bucket_read(self.context.task_id, self.file_offset) {
            Ok(io_id) => {
                self.io_id = Some(io_id);
                Poll::Pending
            }
            Err(error) => Poll::Ready(Err(error)),
        }
    }
}

/// async fn의 local 변수와 현재 await 위치는 컴파일러가 이 future 안에 보관한다.
async fn execute(context: StorageContext, request: StorageRequest) -> StorageResponse {
    match request {
        StorageRequest::ReadBucket { file_offset } => {
            context.with_storage(|storage| {
                storage.started_reads += 1;
            });

            // 이 await 동안에는 &mut Storage가 존재하지 않는다.
            let result = context.read_bucket(file_offset).await;

            context.with_storage(|storage| {
                storage.completed_reads += 1;
            });

            match result {
                Ok(bucket) => StorageResponse::Bucket(bucket),
                Err(error) => StorageResponse::IoError(error),
            }
        }
    }
}

struct PendingRead {
    waiting_task: TaskId,
    buffer: Box<BucketBuffer>,
    cqe_result: Option<i32>,
}

struct IoSlot {
    generation: u32,
    read: Option<PendingRead>,
}

/// SQE, CQE와 I/O 중인 버퍼의 수명을 소유한다.
struct IoDriver {
    file: File,
    ring: IoUring,

    /// 고정 크기가 아니다. 동시 I/O가 늘어나면 새로운 slot을 추가한다.
    slots: Vec<IoSlot>,
    free_slots: Vec<u32>,
}

impl IoDriver {
    fn new(file: File, ring_entries: u32) -> io::Result<Self> {
        Ok(Self {
            file,
            ring: IoUring::new(ring_entries)?,
            slots: Vec::new(),
            free_slots: Vec::new(),
        })
    }

    fn submit_bucket_read(&mut self, waiting_task: TaskId, file_offset: u64) -> io::Result<IoId> {
        let mut buffer = Box::new(BucketBuffer::new());
        let buffer_pointer = buffer.bytes.as_mut_ptr();
        let io_id = self.reserve_slot(PendingRead {
            waiting_task,
            buffer,
            cqe_result: None,
        });

        let entry = opcode::Read::new(
            types::Fd(self.file.as_raw_fd()),
            buffer_pointer,
            BUCKET_BYTES as u32,
        )
        .offset(file_offset)
        .build()
        .user_data(io_id.encode());

        if let Err(error) = self.push_sqe(entry) {
            self.release_unsubmitted_slot(io_id);
            return Err(error);
        }

        Ok(io_id)
    }

    /// task들이 등록한 SQE를 커널에 전달한다. CQE를 기다리며 block하지는 않는다.
    fn submit(&self) -> io::Result<usize> {
        self.ring.submitter().submit()
    }

    /// 도착한 CQE를 IoSlot에 기록하고 해당 task를 ready queue에 넣는다.
    fn handle_completions(&mut self, ready_tasks: &mut VecDeque<TaskId>) {
        let slots = &mut self.slots;
        let mut completion_queue = self.ring.completion();

        for cqe in &mut completion_queue {
            let io_id = IoId::decode(cqe.user_data());
            let Some(slot) = slots.get_mut(io_id.index as usize) else {
                continue;
            };
            if slot.generation != io_id.generation {
                continue;
            }

            let Some(read) = slot.read.as_mut() else {
                continue;
            };
            read.cqe_result = Some(cqe.result());
            ready_tasks.push_back(read.waiting_task);
        }
    }

    /// CQE가 아직 오지 않았으면 None, 도착했으면 버퍼 또는 오류를 반환한다.
    fn take_read_result(&mut self, io_id: IoId) -> Option<io::Result<Box<BucketBuffer>>> {
        let slot = self.slots.get_mut(io_id.index as usize)?;
        if slot.generation != io_id.generation {
            return Some(Err(io::Error::other("stale IoId")));
        }

        let cqe_result = slot.read.as_ref()?.cqe_result?;
        let read = slot.read.take().expect("the read was checked above");

        slot.generation = slot.generation.wrapping_add(1);
        self.free_slots.push(io_id.index);

        if cqe_result < 0 {
            return Some(Err(io::Error::from_raw_os_error(-cqe_result)));
        }
        if cqe_result as usize != BUCKET_BYTES {
            return Some(Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("expected {BUCKET_BYTES} bytes, read {cqe_result}"),
            )));
        }

        Some(Ok(read.buffer))
    }

    fn reserve_slot(&mut self, read: PendingRead) -> IoId {
        let index = match self.free_slots.pop() {
            Some(index) => index,
            None => {
                let index = self.slots.len() as u32;
                self.slots.push(IoSlot {
                    generation: 0,
                    read: None,
                });
                index
            }
        };

        let slot = &mut self.slots[index as usize];
        slot.read = Some(read);

        IoId {
            index,
            generation: slot.generation,
        }
    }

    fn push_sqe(&mut self, entry: squeue::Entry) -> io::Result<()> {
        let pushed = {
            let mut submission_queue = self.ring.submission();
            // SAFETY: PendingRead가 완료될 때까지 IoSlot이 buffer를 소유한다.
            unsafe { submission_queue.push(&entry) }.is_ok()
        };
        if pushed {
            return Ok(());
        }

        // 기존 SQE를 먼저 커널에 보내 공간을 만든 뒤 한 번 더 시도한다.
        self.ring.submitter().submit()?;
        let mut submission_queue = self.ring.submission();
        // SAFETY: PendingRead가 완료될 때까지 IoSlot이 buffer를 소유한다.
        unsafe { submission_queue.push(&entry) }.map_err(|_| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "io_uring submission queue is full",
            )
        })
    }

    fn release_unsubmitted_slot(&mut self, io_id: IoId) {
        let slot = &mut self.slots[io_id.index as usize];
        debug_assert_eq!(slot.generation, io_id.generation);
        slot.read = None;
        slot.generation = slot.generation.wrapping_add(1);
        self.free_slots.push(io_id.index);
    }
}

type TaskFuture = Pin<Box<dyn Future<Output = StorageResponse> + 'static>>;

struct TaskSlot {
    generation: u32,
    occupied: bool,
    future: Option<TaskFuture>,
    response_target: Option<ResponseTarget>,
}

/// Vec가 필요한 만큼 자라는 동적 task slab이다.
///
/// Vec가 재할당되더라도 future 본체는 Pin<Box<_>> 안에 있으므로 주소가 바뀌지 않는다.
struct TaskSlab {
    slots: Vec<TaskSlot>,
    free_slots: Vec<u32>,
}

impl TaskSlab {
    const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_slots: Vec::new(),
        }
    }

    fn reserve(&mut self, response_target: ResponseTarget) -> TaskId {
        let index = match self.free_slots.pop() {
            Some(index) => index,
            None => {
                let index = self.slots.len() as u32;
                self.slots.push(TaskSlot {
                    generation: 0,
                    occupied: false,
                    future: None,
                    response_target: None,
                });
                index
            }
        };

        let slot = &mut self.slots[index as usize];
        debug_assert!(!slot.occupied);
        slot.occupied = true;
        slot.response_target = Some(response_target);

        TaskId {
            index,
            generation: slot.generation,
        }
    }

    fn install(&mut self, task_id: TaskId, future: TaskFuture) {
        self.valid_slot_mut(task_id)
            .expect("a newly reserved task slot must exist")
            .future = Some(future);
    }

    /// poll하는 동안 task slab의 mutable borrow가 남지 않도록 future를 잠깐 꺼낸다.
    fn take_future(&mut self, task_id: TaskId) -> Option<TaskFuture> {
        self.valid_slot_mut(task_id)?.future.take()
    }

    fn restore_future(&mut self, task_id: TaskId, future: TaskFuture) {
        if let Some(slot) = self.valid_slot_mut(task_id) {
            slot.future = Some(future);
        }
    }

    fn finish(&mut self, task_id: TaskId) -> Option<ResponseTarget> {
        let slot = self.valid_slot_mut(task_id)?;
        debug_assert!(slot.future.is_none());

        slot.occupied = false;
        let response_target = slot.response_target.take();
        slot.generation = slot.generation.wrapping_add(1);
        self.free_slots.push(task_id.index);
        response_target
    }

    fn valid_slot_mut(&mut self, task_id: TaskId) -> Option<&mut TaskSlot> {
        let slot = self.slots.get_mut(task_id.index as usize)?;
        if !slot.occupied || slot.generation != task_id.generation {
            return None;
        }
        Some(slot)
    }
}

/// 한 storage thread가 소유하는 전체 runtime이다.
struct Runtime {
    // Box 안의 주소는 Runtime struct가 이동해도 바뀌지 않는다.
    storage: Box<Storage>,
    io: Box<IoDriver>,

    tasks: TaskSlab,
    ready_tasks: VecDeque<TaskId>,

    // 실제 구현에서는 network core별 SPSC receiver/sender 배열이 들어간다.
    incoming: VecDeque<(StorageRequest, ResponseTarget)>,
    outgoing: VecDeque<(ResponseTarget, StorageResponse)>,
}

impl Runtime {
    fn new(file: File, ring_entries: u32) -> io::Result<Self> {
        Ok(Self {
            storage: Box::new(Storage::default()),
            io: Box::new(IoDriver::new(file, ring_entries)?),
            tasks: TaskSlab::new(),
            ready_tasks: VecDeque::new(),
            incoming: VecDeque::new(),
            outgoing: VecDeque::new(),
        })
    }

    fn receive(&mut self, request: StorageRequest, response_target: ResponseTarget) {
        self.incoming.push_back((request, response_target));
    }

    fn spawn(&mut self, request: StorageRequest, response_target: ResponseTarget) {
        let task_id = self.tasks.reserve(response_target);
        let context = StorageContext {
            task_id,
            storage: NonNull::from(self.storage.as_mut()),
            io: NonNull::from(self.io.as_mut()),
        };

        self.tasks
            .install(task_id, Box::pin(execute(context, request)));
        self.ready_tasks.push_back(task_id);
    }

    fn poll_task(&mut self, task_id: TaskId) {
        let Some(mut future) = self.tasks.take_future(task_id) else {
            // generation이 다른 오래된 CQE이거나 이미 끝난 task다.
            return;
        };

        // Future::poll의 형식을 맞추기 위한 no-op Waker다.
        // 이 runtime에서는 CQE 처리 코드가 TaskId를 직접 ready queue에 넣는다.
        let mut context = Context::from_waker(Waker::noop());

        match future.as_mut().poll(&mut context) {
            Poll::Pending => self.tasks.restore_future(task_id, future),
            Poll::Ready(response) => {
                if let Some(response_target) = self.tasks.finish(task_id) {
                    self.outgoing.push_back((response_target, response));
                }
            }
        }
    }

    /// storage thread의 busy loop에서 계속 호출한다.
    fn tick(&mut self) -> io::Result<()> {
        while let Some((request, response_target)) = self.incoming.pop_front() {
            self.spawn(request, response_target);
        }

        // 이전 tick에서 이미 도착한 CQE가 있으면 먼저 task를 깨운다.
        self.io.handle_completions(&mut self.ready_tasks);

        while let Some(task_id) = self.ready_tasks.pop_front() {
            self.poll_task(task_id);
        }

        // 이번 poll에서 만들어진 SQE를 제출한다.
        self.io.submit()?;

        // 즉시 끝난 I/O가 있다면 다음 tick을 기다리지 않고 ready queue에 넣는다.
        self.io.handle_completions(&mut self.ready_tasks);
        Ok(())
    }
}
