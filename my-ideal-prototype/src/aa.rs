//! Compio storage 구조를 다시 잡기 위한 작업용 모듈이다.

#![allow(dead_code)]

use std::io;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Arc;

use compio::buf::IoBuf;
use compio::fs::File;
use compio::io::AsyncWriteAtExt;

use crate::storage::StorageKey;
use crate::storage::bucket::BUCKET_BYTES;
use crate::storage::sg::MutableSg;
use crate::storage::table::{Table, TableConfig, TableCreateError};
use crate::storage_message::Reply;

const MUTABLE_SG_COUNT: usize = 3;
const BUCKETS_PER_SG: usize = 65_536;
const BUCKET_CHOICE_COUNT: u8 = 4;
const SG_BYTES: u64 = (BUCKETS_PER_SG * BUCKET_BYTES) as u64;

struct Storage {
    table: Table,
    sgs: Box<[SgState]>,

    /// 세 Mutable SG 중 가장 오래된 논리 SG index다.
    oldest_mutable_sg_index: usize,

    /// set 경로에서 새로 할당하지 않도록 시작할 때 하나 더 만든 버퍼다.
    /// flush가 끝난 MutableSg를 clear해서 이 자리에 돌려놓는다.
    spare_mutable_sg: Option<MutableSg>,
}

enum SgState {
    Unused,
    Mutable(MutableSg),
    Flushing(FlushBuffer),
    Ssd,
}

/// Storage의 Flushing 상태와 Compio write가 같은 SG allocation을 공유한다.
#[derive(Clone)]
struct FlushBuffer(Rc<MutableSg>);

impl IoBuf for FlushBuffer {
    fn as_init(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

struct FlushJob {
    sg_index: usize,
    buffer: FlushBuffer,
}

struct SetPlan {
    /// SSD await 전에 관찰한 Table value들이다.
    candidates: Box<[u32]>,
}

struct SetObservation {
    /// full StorageKey가 일치한 기존 Table value다.
    previous: Option<u32>,
}

enum CommitSet {
    /// SSD await 중 같은 key의 Table 후보가 바뀌었다.
    Stale,
    Stored { flush: Option<FlushJob> },
}

#[derive(Debug)]
enum SetError {
    ValueTooLarge,
    TableFull,
    Invariant(&'static str),
}

impl Storage {
    fn new(table_config: TableConfig, sg_count: usize) -> Result<Self, TableCreateError> {
        assert!(
            sg_count > MUTABLE_SG_COUNT,
            "the SSD must have room after the three mutable SGs"
        );

        let table = Table::new(table_config)?;
        let mut sgs = (0..sg_count)
            .map(|_| SgState::Unused)
            .collect::<Vec<_>>()
            .into_boxed_slice();

        for state in &mut sgs[..MUTABLE_SG_COUNT] {
            *state = SgState::Mutable(Self::new_mutable_sg());
        }

        Ok(Self {
            table,
            sgs,
            oldest_mutable_sg_index: 0,
            spare_mutable_sg: Some(Self::new_mutable_sg()),
        })
    }

    fn new_mutable_sg() -> MutableSg {
        MutableSg::new(BUCKETS_PER_SG, BUCKET_CHOICE_COUNT)
    }

    fn file_offset(sg_index: usize) -> u64 {
        sg_index as u64 * SG_BYTES
    }

    fn prepare_set(&self, key: &StorageKey) -> SetPlan {
        let candidates = self
            .table
            .values(key.table_hash())
            .collect::<Vec<_>>()
            .into_boxed_slice();

        SetPlan { candidates }
    }

    /// await이 없는 commit 구간이다. Table 후보가 그대로라면 SET을 저장하고,
    /// rollover가 필요할 때 FlushJob을 함께 반환한다.
    fn commit_set(
        &mut self,
        key: &StorageKey,
        value: &[u8],
        plan: SetPlan,
        observation: SetObservation,
    ) -> Result<CommitSet, SetError> {
        let current_candidates = self.table.values(key.table_hash()).collect::<Vec<_>>();
        if current_candidates.as_slice() != plan.candidates.as_ref() {
            return Ok(CommitSet::Stale);
        }

        let _ = (value, observation);

        // TODO: 기존 위치 replace, 세 Mutable insert, 필요하면 rotate_mutable,
        // 마지막으로 Table replace/insert를 한 동기 구간에서 수행한다.
        todo!()
    }

    /// oldest만 flush하고 그 바로 뒤 논리 SG를 새 Mutable로 연다.
    fn rotate_mutable(&mut self) -> FlushJob {
        let old_index = self.oldest_mutable_sg_index;
        let new_index = (old_index + MUTABLE_SG_COUNT) % self.sgs.len();

        assert!(
            matches!(self.sgs[new_index], SgState::Unused),
            "the circular mutable head must never catch an unwritten SSD slot"
        );

        let new_mutable = self
            .spare_mutable_sg
            .take()
            .expect("the previous flush must return the spare MutableSg before rollover");

        let old_state = std::mem::replace(&mut self.sgs[old_index], SgState::Unused);
        let SgState::Mutable(old_mutable) = old_state else {
            unreachable!("oldest_mutable_sg_index must select a Mutable SG");
        };

        let flush_buffer = FlushBuffer(Rc::new(old_mutable));
        self.sgs[old_index] = SgState::Flushing(flush_buffer.clone());
        self.sgs[new_index] = SgState::Mutable(new_mutable);
        self.oldest_mutable_sg_index = (old_index + 1) % self.sgs.len();

        FlushJob {
            sg_index: old_index,
            buffer: flush_buffer,
        }
    }

    /// 성공한 write의 RAM allocation을 회수해 다음 rollover용 spare로 만든다.
    fn complete_flush(
        &mut self,
        sg_index: usize,
        result: io::Result<()>,
        returned_buffer: FlushBuffer,
    ) -> io::Result<()> {
        // write 실패면 Flushing 상태를 유지한다.
        result?;

        // Compio가 돌려준 Rc를 먼저 제거한다.
        // 이제 SgState::Flushing 안의 Rc만 남아야 한다.
        drop(returned_buffer);

        let old_state = std::mem::replace(&mut self.sgs[sg_index], SgState::Ssd);
        let SgState::Flushing(buffer) = old_state else {
            unreachable!("completed SG must be Flushing");
        };

        // 마지막 Rc에서 MutableSg allocation을 복사 없이 회수한다.
        let mut reusable = Rc::try_unwrap(buffer.0)
            .unwrap_or_else(|_| panic!("Flushing state must be the only buffer owner"));
        reusable.clear();

        assert!(
            self.spare_mutable_sg.is_none(),
            "only one flush may be in flight"
        );
        self.spare_mutable_sg = Some(reusable);

        Ok(())
    }
}

struct WorkerState {
    storage: Storage,
    fatal_io_error: Option<io::Error>,
}

#[derive(Clone, Copy)]
struct WorkerHandle(NonNull<WorkerState>);

impl WorkerHandle {
    fn access<R>(self, operation: impl FnOnce(&mut WorkerState) -> R) -> R {
        // SAFETY: WorkerState는 고정된 Box 안에 있고 같은 storage thread만 접근한다.
        unsafe { operation(&mut *self.0.as_ptr()) }
    }
}

/// prepare_set에서 얻은 후보 중 SSD에 있는 Bucket만 읽어 full key를 확인한다.
async fn resolve_set_plan(
    file: Rc<File>,
    key: &StorageKey,
    plan: &SetPlan,
) -> io::Result<SetObservation> {
    let _ = (file, key, plan);
    // TODO: candidate를 RAM에서 먼저 확인하고 필요한 SSD Bucket만 await한다.
    todo!()
}

async fn flush_sg(worker: WorkerHandle, file: Rc<File>, flush: FlushJob) {
    let file_offset = Storage::file_offset(flush.sg_index);
    let mut file = &*file;
    let (result, returned_buffer) = file
        .write_all_at(flush.buffer, file_offset)
        .await
        .into_parts();

    worker.access(|worker| {
        if let Err(error) =
            worker
                .storage
                .complete_flush(flush.sg_index, result, returned_buffer)
        {
            worker.fatal_io_error = Some(error);
        }
    });
}

async fn execute_set(
    worker: WorkerHandle,
    file: Rc<File>,
    key: StorageKey,
    value: Arc<[u8]>,
) -> io::Result<Reply> {
    loop {
        let plan = worker.access(|worker| worker.storage.prepare_set(&key));
        let observation = resolve_set_plan(Rc::clone(&file), &key, &plan).await?;

        let commit = worker
            .access(|worker| {
                worker
                    .storage
                    .commit_set(&key, &value, plan, observation)
            })
            .map_err(|error| io::Error::other(format!("storage SET failed: {error:?}")))?;

        match commit {
            CommitSet::Stale => continue,
            CommitSet::Stored { flush } => {
                if let Some(flush) = flush {
                    compio::runtime::spawn(flush_sg(worker, Rc::clone(&file), flush)).detach();
                }
                return Ok(Reply::SetOk);
            }
        }
    }
}
