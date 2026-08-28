//! Portable polling network frontend used by the Apple Silicon build.
//!
//! Linux keeps the completion-oriented io_uring frontend in
//! `network_linux.rs`. macOS has no io_uring ABI, so this module uses Tokio's
//! native polling driver while preserving the same RESP parser and storage
//! queue contract. The fallback intentionally favors portability over the
//! Linux frontend's throughput optimizations, but it still services multiple
//! TCP connections concurrently so the QUIC compatibility proxy can share the
//! listener with direct RESP clients.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::net::TcpListener as StdTcpListener;
use std::rc::Rc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::LocalSet;

use super::super::resp::{ResponseToWrite, StatefulRespParser, make_response_to_write};
use super::super::spsc::{Consumer, Producer};
use super::super::storage_message::{
    ClientId, Command, STORAGE_QUEUE_SLOTS, StorageRequest, StorageResponse,
};

type ResponseKey = (usize, u64);
const MAX_CLIENTS: usize = 2_000;

struct NetworkState {
    request_sender: Producer<StorageRequest, STORAGE_QUEUE_SLOTS>,
    response_receiver: Consumer<StorageResponse, STORAGE_QUEUE_SLOTS>,
    active_clients: HashSet<usize>,
    pending_responses: HashMap<ResponseKey, ResponseToWrite>,
    next_client_id: usize,
}

pub(crate) struct Network {
    listener: Option<StdTcpListener>,
    state: Rc<RefCell<NetworkState>>,
}

impl Network {
    pub(crate) fn new(
        listener: StdTcpListener,
        request_sender: Producer<StorageRequest, STORAGE_QUEUE_SLOTS>,
        response_receiver: Consumer<StorageResponse, STORAGE_QUEUE_SLOTS>,
    ) -> io::Result<Self> {
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener: Some(listener),
            state: Rc::new(RefCell::new(NetworkState {
                request_sender,
                response_receiver,
                active_clients: HashSet::new(),
                pending_responses: HashMap::new(),
                next_client_id: 0,
            })),
        })
    }

    pub(crate) fn run(&mut self) -> io::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(io::Error::other)?;
        let listener = self
            .listener
            .take()
            .ok_or_else(|| io::Error::other("macOS network frontend was already started"))?;
        runtime.block_on(async {
            let listener = TcpListener::from_std(listener)?;
            self.run_async(listener).await
        })
    }

    async fn run_async(&mut self, listener: TcpListener) -> io::Result<()> {
        let local = LocalSet::new();
        let response_state = Rc::clone(&self.state);
        local.spawn_local(pump_storage_responses(response_state));

        let state = Rc::clone(&self.state);
        local
            .run_until(async move {
                loop {
                    if super::shutdown_requested() {
                        return Ok(());
                    }
                    let (stream, peer) =
                        match tokio::time::timeout(Duration::from_millis(100), listener.accept())
                            .await
                        {
                            Ok(result) => result?,
                            Err(_) => continue,
                        };
                    let client_id = {
                        let mut state = state.borrow_mut();
                        if state.active_clients.len() >= MAX_CLIENTS {
                            continue;
                        }
                        let client_id = ClientId(state.next_client_id);
                        state.next_client_id = state
                            .next_client_id
                            .checked_add(1)
                            .ok_or_else(|| io::Error::other("macOS client ID space exhausted"))?;
                        state.active_clients.insert(client_id.0);
                        client_id
                    };
                    let connection_state = Rc::clone(&state);
                    tokio::task::spawn_local(async move {
                        let result =
                            serve_connection(Rc::clone(&connection_state), client_id, stream).await;
                        let mut state = connection_state.borrow_mut();
                        state.active_clients.remove(&client_id.0);
                        state
                            .pending_responses
                            .retain(|(pending_client_id, _), _| *pending_client_id != client_id.0);
                        drop(state);
                        if let Err(error) = result {
                            eprintln!("RESP connection {peer} stopped: {error}");
                        }
                    });
                }
            })
            .await
    }
}

async fn pump_storage_responses(state: Rc<RefCell<NetworkState>>) {
    loop {
        let response = {
            let mut state = state.borrow_mut();
            state.response_receiver.pop().and_then(|response| {
                let key = (response.client_id.0, response.sequence);
                state
                    .active_clients
                    .contains(&key.0)
                    .then(|| (key, make_response_to_write(response.reply)))
            })
        };

        if let Some((key, response)) = response {
            let previous = state.borrow_mut().pending_responses.insert(key, response);
            assert!(
                previous.is_none(),
                "storage returned the same response sequence twice"
            );
        } else {
            // The SPSC queue has no async wakeup primitive. A short timer keeps
            // the portability frontend responsive without burning a core while
            // the storage worker is idle.
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
}

async fn serve_connection(
    state: Rc<RefCell<NetworkState>>,
    client_id: ClientId,
    mut stream: TcpStream,
) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let mut parser = StatefulRespParser::new();
    let mut pending_commands = VecDeque::new();
    let mut read_buffer = [0_u8; 64 * 1024];
    let mut next_sequence = 0_u64;

    loop {
        let received = stream.read(&mut read_buffer).await?;
        if received == 0 {
            return Ok(());
        }
        parser
            .feed(&read_buffer[..received], &mut pending_commands)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

        while let Some(command) = pending_commands.pop_front() {
            let sequence = next_sequence;
            next_sequence += 1;
            let response = submit_and_receive(&state, client_id, sequence, command).await?;
            write_response(&mut stream, &response).await?;
        }
    }
}

async fn submit_and_receive(
    state: &Rc<RefCell<NetworkState>>,
    client_id: ClientId,
    sequence: u64,
    command: Command,
) -> io::Result<ResponseToWrite> {
    let mut request = Some(StorageRequest {
        client_id,
        sequence,
        command,
    });
    loop {
        let submitted = {
            let mut state = state.borrow_mut();
            if state.request_sender.has_capacity() {
                let pushed = state
                    .request_sender
                    .push(request.take().expect("request was not submitted"));
                assert!(
                    pushed.is_ok(),
                    "capacity was checked immediately before push"
                );
                true
            } else {
                false
            }
        };
        if submitted {
            break;
        }
        tokio::task::yield_now().await;
    }

    let key = (client_id.0, sequence);
    loop {
        if let Some(response) = state.borrow_mut().pending_responses.remove(&key) {
            return Ok(response);
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

async fn write_response(stream: &mut TcpStream, response: &ResponseToWrite) -> io::Result<()> {
    stream.write_all(response.header_bytes.as_ref()).await?;
    if let Some(value) = response.value_bytes.as_deref() {
        stream.write_all(value).await?;
    }
    stream.write_all(response.ending_bytes).await
}
