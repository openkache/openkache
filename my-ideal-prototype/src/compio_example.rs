//! Compio를 사용하는 storage thread의 최소 구조 예시다.
//!
//! Storage 내부 함수는 아직 구현하지 않고 필요한 상태와 인터페이스만 정의한다.

use std::collections::VecDeque;
use std::io;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use compio::buf::IoBuf;
use compio::fs::File;
use compio::io::{AsyncReadAtExt, AsyncWriteAtExt};
use compio::runtime::Runtime;

use crate::spsc::{Consumer, Producer};
use crate::storage::StorageKey;
use crate::storage::bucket::{Bucket, BucketValue};
use crate::storage::sg::MutableSg;
use crate::storage::table::{Table, TableConfig, TableCreateError};
use crate::storage_message::{
    Command, Reply, STORAGE_QUEUE_SLOTS, StorageRequest, StorageResponse,
};

struct Storage {
    table: Table,

    // index 자체가 sg_index다.
    sgs: Box<[SgState]>,

    // 현재 세 Mutable 중 가장 오래된 SG다.
    oldest_mutable_sg_index: usize,

    bucket_count: usize,
    bucket_choice_bits: u8,

    // 오래된 flush 완료가 재사용된 SG를 건드리지 못하게 한다.
    next_flush_generation: u32,
}

enum SgState {
    Unused,

    Mutable(MutableSg),

    Flushing {
        generation: u32,
        file_offset: u64,

        // flush 중에도 GET은 이 RAM SG를 읽는다.
        buffer: FlushBuffer,
    },

    Ssd {
        file_offset: u64,
    },
}

/// Storage의 Flushing 상태와 Compio write task가 같은 SG를 소유하게 한다.
/// Rc::clone은 SG byte를 복사하지 않고 단일 thread 참조 개수만 올린다.
#[derive(Clone)]
struct FlushBuffer(Rc<MutableSg>);

impl IoBuf for FlushBuffer {
    fn as_init(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

struct FlushJob {
    sg_index: usize,
    generation: u32,
    file_offset: u64,
    buffer: FlushBuffer,
}

enum Lookup {
    // RAM에 있었고 full StorageKey까지 일치했다.
    Value(Arc<[u8]>),

    // 정확히 같은 key의 Tombstone이다.
    Tombstone,

    // Table fingerprint만 우연히 같았고 실제 key는 없었다.
    CandidateMiss,

    // 이 candidate는 SSD Bucket을 읽어야 확인할 수 있다.
    ReadBucket {
        file_offset: u64,
    },
}

enum SetResult {
    // RAM 저장과 Table 갱신이 끝났다.
    Stored {
        // SG가 교체됐다면 background write가 필요하다.
        flush: Option<FlushJob>,
    },

    // SSD await 중 다른 SET이 Table을 변경했다.
    Retry,

    // 새 Mutable SG에도 Item을 넣을 수 없다.
    Full,
}

impl Storage {
    fn new(
        table_config: TableConfig,
        sg_count: usize,
        bucket_count: usize,
        bucket_choice_count: u8,
    ) -> Result<Self, TableCreateError> {
        todo!()
    }

    // await 동안 Table iterator를 들고 있을 수 없으므로
    // u32 candidate들을 task가 소유하게 만든다.
    fn candidates(&self, key: &StorageKey) -> Vec<u32> {
        todo!()
    }

    // Mutable/Flushing이면 RAM을 확인하고,
    // SSD이면 물리 Bucket offset을 계산한다.
    fn lookup(&self, key: &StorageKey, table_value: u32) -> Lookup {
        todo!()
    }

    // value를 Mutable SG에 복사하고 Table을 갱신한다.
    //
    // observed_candidates는 SSD await 중 Table이 바뀌었는지 확인하기 위한 값이다.
    // previous는 full StorageKey가 일치했던 기존 위치다.
    fn set(
        &mut self,
        key: &StorageKey,
        value: &[u8],
        observed_candidates: &[u32],
        previous: Option<u32>,
    ) -> SetResult {
        todo!()
    }

    // write CQE가 성공하면 Flushing -> Ssd로 바꾼다.
    fn complete_flush(
        &mut self,
        sg_index: usize,
        generation: u32,
        result: io::Result<()>,
    ) {
        todo!()
    }
}

struct WorkerState {
    storage: Storage,
    completed: VecDeque<CompletedResponse>,
    fatal_io_error: Option<io::Error>,
}

struct CompletedResponse {
    network_core_index: usize,
    response: StorageResponse,
}

/// 같은 storage thread의 async task가 await 전후로 WorkerState에 접근하는 손잡이다.
#[derive(Clone, Copy)]
struct WorkerHandle(NonNull<WorkerState>);

impl WorkerHandle {
    fn access<R>(self, operation: impl FnOnce(&mut WorkerState) -> R) -> R {
        // SAFETY: WorkerState는 Box 안에 고정되어 있고 같은 storage thread에서만
        // 접근한다. operation 내부에서는 await하지 않는다.
        unsafe { operation(&mut *self.0.as_ptr()) }
    }
}

// Bucket에는 Compio의 IoBuf, IoBufMut, SetLen 구현이 추가될 예정이다.
async fn read_bucket(file: Rc<File>, file_offset: u64) -> io::Result<Box<Bucket>> {
    let (result, bucket) = file
        .read_exact_at(Box::new(Bucket::new()), file_offset)
        .await
        .into_parts();

    result?;
    Ok(bucket)
}

async fn execute_get(
    worker: WorkerHandle,
    file: Rc<File>,
    key: StorageKey,
) -> io::Result<Reply> {
    let candidates = worker.access(|worker| worker.storage.candidates(&key));

    for candidate in candidates {
        match worker.access(|worker| worker.storage.lookup(&key, candidate)) {
            Lookup::Value(value) => {
                return Ok(Reply::Get(Some(value)));
            }

            Lookup::Tombstone => {
                return Ok(Reply::Get(None));
            }

            Lookup::CandidateMiss => continue,

            Lookup::ReadBucket { file_offset } => {
                let bucket = read_bucket(Rc::clone(&file), file_offset).await?;

                // SSD read는 끝났다. RAM에 있는 Bucket을 바로 확인한다.
                match bucket.get(&key) {
                    Some(BucketValue::Value(value)) => {
                        return Ok(Reply::Get(Some(Arc::from(value))));
                    }

                    Some(BucketValue::Tombstone) => {
                        return Ok(Reply::Get(None));
                    }

                    None => continue,
                }
            }
        }
    }

    Ok(Reply::Get(None))
}

async fn execute_set(
    worker: WorkerHandle,
    file: Rc<File>,
    key: StorageKey,
    value: Arc<[u8]>,
) -> io::Result<Reply> {
    loop {
        let candidates = worker.access(|worker| worker.storage.candidates(&key));
        let mut previous = None;

        for candidate in candidates.iter().copied() {
            match worker.access(|worker| worker.storage.lookup(&key, candidate)) {
                Lookup::Value(_) | Lookup::Tombstone => {
                    previous = Some(candidate);
                    break;
                }

                Lookup::CandidateMiss => {}

                Lookup::ReadBucket { file_offset } => {
                    let bucket = read_bucket(Rc::clone(&file), file_offset).await?;

                    if bucket.get(&key).is_some() {
                        previous = Some(candidate);
                        break;
                    }
                }
            }
        }

        let result = worker.access(|worker| {
            worker
                .storage
                .set(&key, &value, &candidates, previous)
        });

        match result {
            SetResult::Stored { flush: None } => {
                return Ok(Reply::SetOk);
            }

            SetResult::Stored {
                flush: Some(flush),
            } => {
                compio::runtime::spawn(flush_sg(worker, Rc::clone(&file), flush)).detach();

                // RAM 저장과 Table 갱신은 이미 끝났으므로
                // SSD flush를 기다리지 않고 응답한다.
                return Ok(Reply::SetOk);
            }

            SetResult::Retry => continue,

            SetResult::Full => {
                return Err(io::Error::other("storage is full"));
            }
        }
    }
}

async fn flush_sg(worker: WorkerHandle, file: Rc<File>, flush: FlushJob) {
    let mut file = &*file;

    let (result, _) = file
        .write_all_at(flush.buffer.clone(), flush.file_offset)
        .await
        .into_parts();

    worker.access(|worker| {
        worker
            .storage
            .complete_flush(flush.sg_index, flush.generation, result);
    });
}

async fn execute_request(
    worker: WorkerHandle,
    file: Rc<File>,
    request: StorageRequest,
) -> io::Result<StorageResponse> {
    let client_id = request.client_id;
    let sequence = request.sequence;
    let reply = match request.command {
        Command::Get { key } => {
            execute_get(worker, file, StorageKey::from_key(&key)).await?
        }

        Command::Set { key, value } => {
            execute_set(worker, file, StorageKey::from_key(&key), value).await?
        }

        Command::Delete { .. } => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "DELETE is implemented by storage.rs, not this design example",
            ));
        }
    };

    Ok(StorageResponse {
        client_id,
        sequence,
        reply,
    })
}

fn run_storage_worker(
    runtime: Runtime,
    mut worker: Box<WorkerState>,
    file: Rc<File>,
    mut request_queues: Box<[Consumer<StorageRequest, STORAGE_QUEUE_SLOTS>]>,
    mut response_queues: Box<[Producer<StorageResponse, STORAGE_QUEUE_SLOTS>]>,
) -> io::Result<()> {
    let worker_handle = WorkerHandle(NonNull::from(worker.as_mut()));

    runtime.enter(|| loop {
        for (network_core_index, request_queue) in request_queues.iter_mut().enumerate() {
            while let Some(request) = request_queue.pop() {
                let file = Rc::clone(&file);

                runtime
                    .spawn(async move {
                        let response = execute_request(worker_handle, file, request).await;

                        worker_handle.access(|worker| match response {
                            Ok(response) => {
                                worker.completed.push_back(CompletedResponse {
                                    network_core_index,
                                    response,
                                });
                            }
                            Err(error) => {
                                worker.fatal_io_error = Some(error);
                            }
                        });
                    })
                    .detach();
            }
        }

        runtime.run();
        runtime.flush();

        runtime.poll_with(Some(Duration::ZERO));
        runtime.run();

        while let Some(completed) =
            worker_handle.access(|worker| worker.completed.pop_front())
        {
            let _ = response_queues[completed.network_core_index].push(completed.response);
        }

        if let Some(error) = worker_handle.access(|worker| worker.fatal_io_error.take()) {
            return Err(error);
        }
    })
}
