//! Temporary in-memory storage for the runnable RESP compatibility prototype.
//!
//! The SSD storage and table experiments remain unchanged. They currently
//! describe different prototype generations, so the executable uses this
//! minimal backend while the native-over-QUIC adapter is evaluated.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use crate::spsc::{Consumer, Producer};
use crate::storage_message::{
    Command, Reply, STORAGE_QUEUE_SLOTS, StorageRequest, StorageResponse,
};

pub(crate) fn run(
    mut request_receiver: Consumer<StorageRequest, STORAGE_QUEUE_SLOTS>,
    mut response_sender: Producer<StorageResponse, STORAGE_QUEUE_SLOTS>,
) -> io::Result<()> {
    let mut values: HashMap<Vec<u8>, Arc<[u8]>> = HashMap::new();

    loop {
        while !response_sender.has_capacity() {
            std::hint::spin_loop();
        }

        let request = loop {
            if let Some(request) = request_receiver.pop() {
                break request;
            }
            std::hint::spin_loop();
        };

        let reply = match request.command {
            Command::Get { key } => Reply::Get(values.get(key.as_ref()).cloned()),
            Command::Set { key, value } => {
                values.insert(key.into_vec(), value);
                Reply::SetOk
            }
        };
        let response = StorageResponse {
            client_id: request.client_id,
            reply,
        };
        if response_sender.push(response).is_err() {
            unreachable!("response queue had capacity and storage is its sole producer");
        }
    }
}
