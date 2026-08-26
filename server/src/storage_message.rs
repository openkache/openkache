use std::sync::Arc;

pub(crate) const STORAGE_QUEUE_SLOTS: usize = 4096;

#[derive(Clone, Copy)]
pub(crate) struct ClientId(pub(crate) usize);

pub(crate) struct StorageRequest {
    pub client_id: ClientId,
    pub sequence: u64,
    pub command: Command,
}

pub(crate) enum Command {
    Get { key: Box<[u8]> },
    Set { key: Box<[u8]>, value: Arc<[u8]> },
    Delete { key: Box<[u8]> },
}

pub(crate) struct StorageResponse {
    pub client_id: ClientId,
    pub sequence: u64,
    pub reply: Reply,
}

pub(crate) enum Reply {
    Get(Option<Arc<[u8]>>),
    SetOk,
    Delete(bool),
}
