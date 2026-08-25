/*
지금은 무조건 순서대로이니 sequence 가 필요 없지만 샤딩 -> sequence 필요!
*/

use std::sync::Arc;

pub(crate) const STORAGE_QUEUE_SLOTS: usize = 4096;

#[derive(Clone, Copy)]
pub(crate) struct ClientId(pub(crate) usize);

pub(crate) struct StorageRequest {
    pub client_id: ClientId,
    pub command: Command,
}

pub(crate) enum Command {
    Get { key: Box<[u8]> },
    Set { key: Box<[u8]>, value: Arc<[u8]> },
}

pub(crate) struct StorageResponse {
    pub client_id: ClientId,
    pub reply: Reply,
}

pub(crate) enum Reply {
    Get(Option<Arc<[u8]>>),
    SetOk,
}
