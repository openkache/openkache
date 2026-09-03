//! Storage shard entry point.
//!
//! One storage thread owns one [`StorageState`], one [`Scheduler`], and one
//! compio runtime. Network workers communicate with the shard through one SPSC
//! channel each. Requests for the same [`StorageKey`] are serialized by the
//! scheduler; requests for different keys may wait on SSD I/O concurrently.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;

use compio::driver::ProactorBuilder;
use compio::fs::File;
use compio::runtime::Runtime;

use crate::config::StorageConfig;
use crate::spsc::{Consumer, Producer};
use crate::storage_message::{Command, STORAGE_QUEUE_SLOTS, StorageRequest, StorageResponse};

mod scheduler;
mod storage_state;

use scheduler::Scheduler;
use storage_state::StorageState;

/// The request and response endpoints connecting one network worker to this
/// storage shard.
pub(crate) struct StorageChannel {
    pub(crate) request_consumer: Consumer<StorageRequest, STORAGE_QUEUE_SLOTS>,
    pub(crate) response_producer: Producer<StorageResponse, STORAGE_QUEUE_SLOTS>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ChannelId(usize);

/// Storage request paired with the request/response channel index it arrived on.
struct RoutedRequest {
    channel_id: ChannelId,
    request: StorageRequest,
}

pub(crate) struct Storage {
    compio_runtime: Runtime,
    storage_state: Rc<RefCell<StorageState>>,
    key_scheduler: Rc<RefCell<Scheduler>>,
    channels: Box<[StorageChannel]>,
    next_channel_index: usize,
    pending_responses_by_channel: Rc<RefCell<Box<[VecDeque<io::Result<StorageResponse>>]>>>,
}

impl Storage {
    pub(crate) fn build(
        config: StorageConfig,
        channels: Box<[StorageChannel]>,
    ) -> io::Result<Self> {
        use std::os::fd::{AsRawFd as _, FromRawFd as _, IntoRawFd as _};
        use std::os::unix::fs::OpenOptionsExt;

        if channels.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a storage shard needs at least one network channel",
            ));
        }

        let mut proactor = ProactorBuilder::new();
        proactor
            .capacity(config.io_queue_entries)
            .single_issuer(true)
            .defer_taskrun(true)
            .taskrun_flag(true);
        let mut runtime_builder = Runtime::builder();
        runtime_builder.with_proactor(proactor);
        let compio_runtime = runtime_builder.build()?;

        let storage_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .custom_flags(libc::O_DIRECT)
            .open(&config.storage_file_path)?;
        if config.preallocate_file {
            // SAFETY: `storage_file` owns a valid descriptor and the configured
            // length was validated before the storage thread started.
            let result = unsafe {
                libc::fallocate(
                    storage_file.as_raw_fd(),
                    0,
                    0,
                    config.storage_file_bytes as libc::off_t,
                )
            };
            if result != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        storage_file.set_len(config.storage_file_bytes)?;
        compio_runtime.attach(storage_file.as_raw_fd())?;
        let raw_fd = storage_file.into_raw_fd();
        // SAFETY: ownership of the descriptor moves into compio. The descriptor
        // was attached to this runtime immediately above.
        let storage_file = Rc::new(unsafe { File::from_raw_fd(raw_fd) });

        let channel_count = channels.len();
        Ok(Self {
            compio_runtime,
            storage_state: Rc::new(RefCell::new(StorageState::new(&config, storage_file))),
            key_scheduler: Rc::new(RefCell::new(Scheduler::new())),
            channels,
            next_channel_index: 0,
            pending_responses_by_channel: Rc::new(RefCell::new(
                (0..channel_count).map(|_| VecDeque::new()).collect(),
            )),
        })
    }

    pub(crate) fn run(self) -> io::Result<()> {
        use std::os::fd::AsRawFd as _;
        use std::time::Duration;

        let Self {
            compio_runtime,
            storage_state,
            key_scheduler,
            mut channels,
            mut next_channel_index,
            pending_responses_by_channel,
        } = self;
        let io_uring_fd = compio_runtime.as_raw_fd();

        compio_runtime.enter(|| {
            loop {
                receive_requests(&mut channels, &mut next_channel_index, &key_scheduler);
                spawn_ready_requests(
                    &compio_runtime,
                    &key_scheduler,
                    &storage_state,
                    &pending_responses_by_channel,
                );

                compio_runtime.run();
                compio_runtime.flush();

                // DEFER_TASKRUN does not post deferred completions to the CQ
                // until the ring is entered with GETEVENTS. compio exposes the
                // ring fd but not its submission queue or TASKRUN flag, so reap
                // through the raw fd before asking compio to consume the CQEs.
                // SAFETY: `io_uring_fd` belongs to the live compio runtime;
                // to_submit=0 does not interfere with compio's SQ bookkeeping.
                let result = unsafe {
                    libc::syscall(
                        libc::SYS_io_uring_enter,
                        io_uring_fd,
                        0,
                        0,
                        io_uring::EnterFlags::GETEVENTS.bits(),
                        std::ptr::null::<libc::c_void>(),
                        0,
                    )
                };
                if result < 0 {
                    return Err(io::Error::last_os_error());
                }

                compio_runtime.poll_with(Some(Duration::ZERO));
                compio_runtime.run();
                if let Some(error) = storage_state.borrow_mut().take_fatal_io_error() {
                    return Err(error);
                }
                drain_responses(&mut channels, &pending_responses_by_channel)?;
            }
        })
    }
}

fn receive_requests(
    channels: &mut [StorageChannel],
    next_channel_index: &mut usize,
    key_scheduler: &Rc<RefCell<Scheduler>>,
) {
    while todo!("define the request intake boundary") {
        let channel_id = ChannelId(*next_channel_index);
        *next_channel_index = (*next_channel_index + 1) % channels.len();

        if let Some(request) = channels[channel_id.0].request_consumer.pop() {
            key_scheduler.borrow_mut().enqueue(RoutedRequest {
                channel_id,
                request,
            });
        }
    }
}

fn spawn_ready_requests(
    compio_runtime: &Runtime,
    key_scheduler: &Rc<RefCell<Scheduler>>,
    storage_state: &Rc<RefCell<StorageState>>,
    pending_responses_by_channel: &Rc<RefCell<Box<[VecDeque<io::Result<StorageResponse>>]>>>,
) {
    loop {
        let Some((schedule_key, value)) = key_scheduler.borrow_mut().take_ready() else {
            break;
        };

        let storage_state = Rc::clone(storage_state);
        let key_scheduler = Rc::clone(key_scheduler);
        let pending_responses_by_channel = Rc::clone(pending_responses_by_channel);
        let channel_id = value.channel_id;

        compio_runtime
            .spawn(async move {
                let response = execute_request(storage_state, value).await;
                key_scheduler.borrow_mut().finish(schedule_key);
                pending_responses_by_channel.borrow_mut()[channel_id.0].push_back(response);
            })
            .detach();
    }
}

fn drain_responses(
    channels: &mut [StorageChannel],
    pending_responses_by_channel: &Rc<RefCell<Box<[VecDeque<io::Result<StorageResponse>>]>>>,
) -> io::Result<()> {
    let mut pending_responses_by_channel = pending_responses_by_channel.borrow_mut();
    for (channel, pending_responses) in channels
        .iter_mut()
        .zip(pending_responses_by_channel.iter_mut())
    {
        while let Some(response) = pending_responses.pop_front() {
            let response = response?;
            if let Err(response) = channel.response_producer.push(response) {
                pending_responses.push_front(Ok(response));
                break;
            }
        }
    }
    Ok(())
}

async fn execute_request(
    storage_state: Rc<RefCell<StorageState>>,
    routed_request: RoutedRequest,
) -> io::Result<StorageResponse> {
    let StorageRequest {
        client_id,
        sequence,
        command,
    } = routed_request.request;

    let reply = match command {
        Command::Get { key } => StorageState::get(storage_state, key).await?,
        Command::Set { key, value } => StorageState::set(storage_state, key, value).await?,
        Command::Delete { key } => StorageState::delete(storage_state, key).await?,
        Command::Flush => StorageState::flush(storage_state).await?,
    };

    Ok(StorageResponse {
        client_id,
        sequence,
        reply,
    })
}
