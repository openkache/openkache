//! Linux network I/O engine built on io_uring.
//!
//! A single-threaded, zero-copy network worker that drives all socket I/O
//! through one io_uring instance. It serves thousands of concurrent clients
//! with minimal syscalls by batching accepts, reads, and writes into the
//! submission queue and reaping completions in bulk. This thread is pinned to a
//! dedicated core, so all state lives without locks in the owning thread.

use std::collections::VecDeque;
use std::io;
use std::mem::MaybeUninit;
use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd};

use io_uring::{IoUring, cqueue, opcode, squeue, types};
use slab::Slab;

use super::super::client::Client;
use super::super::resp::make_response_to_write;
use super::super::spsc::{Consumer, Producer};
use super::super::storage_message::{
    ClientId, STORAGE_QUEUE_SLOTS, StorageRequest, StorageResponse,
};
use super::provided_buffer_ring::ProvidedBufferRing;

#[cfg(all(feature = "uring-coop-taskrun", feature = "uring-defer-taskrun"))]
compile_error!("uring-coop-taskrun and uring-defer-taskrun cannot be enabled together");

const MAX_CLIENTS: usize = 2_000;
const IO_URING_QUEUE_ENTRIES: u32 = 4_096;
/// How many completions we drain per `fill` call. Reaping in bulk keeps the hot
/// completion loop cache-friendly and amortizes access to the ring's shared memory.
const CQE_BATCH_SIZE: usize = 256;
// The low two bits of every CQE `user_data` encode the operation type, so the
// completion handler can dispatch without a side table. The remaining bits hold
// the client id (see `make_cqe_user_data`).
pub(crate) const ACCEPT_CQE: u64 = 0;
pub(crate) const READ_CQE: u64 = 1;
pub(crate) const WRITE_CQE: u64 = 2;
// Provided buffers let the kernel choose a receive buffer at completion time
// rather than us pinning one buffer per in-flight read. This bounds memory
// regardless of client count and enables efficient multishot-style receives.
const PROVIDED_BUFFER_GROUP_ID: u16 = 0;
const PROVIDED_BUFFER_COUNT: u16 = 2_048;
const PROVIDED_BUFFER_SIZE: usize = 4 * 1_024;

/// Builds the io_uring, opting into kernel setup flags chosen at compile time.
/// Each flag trades generality for throughput on the single network thread:
/// - `single_issuer`: promises only one thread submits, so the kernel skips
///   internal submission locking.
/// - `coop_taskrun` / `defer_taskrun`: defer completion task work so it runs
///   when we reap rather than interrupting us, cutting inter-processor interrupts.
fn create_io_uring() -> io::Result<IoUring> {
    #[cfg(not(any(
        feature = "uring-single-issuer",
        feature = "uring-coop-taskrun",
        feature = "uring-defer-taskrun"
    )))]
    let builder = IoUring::builder();

    #[cfg(any(
        feature = "uring-single-issuer",
        feature = "uring-coop-taskrun",
        feature = "uring-defer-taskrun"
    ))]
    let mut builder = IoUring::builder();

    #[cfg(any(feature = "uring-single-issuer", feature = "uring-defer-taskrun"))]
    builder.setup_single_issuer();

    #[cfg(feature = "uring-coop-taskrun")]
    builder.setup_coop_taskrun();

    #[cfg(feature = "uring-defer-taskrun")]
    {
        builder.setup_defer_taskrun();
        builder.setup_taskrun_flag();
    }

    builder.build(IO_URING_QUEUE_ENTRIES)
}

/// The network worker: owns the TCP listener, io_uring ring, client state, and
/// the SPSC endpoints to the storage thread. All I/O runs on one thread, so
/// `clients` is a `Slab` (direct index access) and the ring needs no locking.
pub(crate) struct Network {
    listener: TcpListener,
    io_uring: IoUring,
    /// Zero-copy provided-buffer pool. The kernel writes received bytes directly
    /// into a buffer it selects from this ring, avoiding per-client pinned buffers.
    provided_buffers: ProvidedBufferRing,
    /// Client state indexed by client id. `Slab` recycles vacant slots, so memory
    /// tracks peak concurrency rather than total connections ever accepted.
    clients: Slab<Client>,
    /// Sends parsed commands to the storage thread.
    request_sender: Producer<StorageRequest, STORAGE_QUEUE_SLOTS>,
    /// Receives storage replies to forward back to clients.
    response_receiver: Consumer<StorageResponse, STORAGE_QUEUE_SLOTS>,
    /// Clients with a parsed command that could not be pushed because the request
    /// queue was full. Queuing the id (instead of blocking on the push) keeps the
    /// event loop non-blocking while preserving per-client submission order.
    clients_waiting_for_submission: VecDeque<ClientId>,
    /// Whether a multishot accept is currently armed. We re-arm whenever it ends
    /// or errors, so the ring never runs out of inbound connection capacity.
    multishot_accept_is_active: bool,
}

impl Network {
    /// Creates the worker: sets up the io_uring and the provided-buffer ring.
    /// No operations are submitted yet — that happens in `run`.
    pub(crate) fn new(
        listener: TcpListener,
        request_sender: Producer<StorageRequest, STORAGE_QUEUE_SLOTS>,
        response_receiver: Consumer<StorageResponse, STORAGE_QUEUE_SLOTS>,
    ) -> io::Result<Self> {
        let io_uring = create_io_uring()?;
        let clients = Slab::with_capacity(MAX_CLIENTS);
        // SAFETY: `ProvidedBufferRing::new` registers a buffer region with the
        // kernel; the ring is unregistered on drop, before its backing memory frees.
        let provided_buffers = unsafe {
            ProvidedBufferRing::new(
                &io_uring,
                PROVIDED_BUFFER_GROUP_ID,
                PROVIDED_BUFFER_COUNT,
                PROVIDED_BUFFER_SIZE,
            )?
        };

        Ok(Self {
            listener,
            io_uring,
            provided_buffers,
            clients,
            request_sender,
            response_receiver,
            clients_waiting_for_submission: VecDeque::new(),
            multishot_accept_is_active: false,
        })
    }

    /// The main event loop. Each iteration: submit pending SQEs, reap all ready
    /// completions, move storage responses into client write buffers, drain any
    /// commands that were waiting for request-queue capacity, then queue the next
    /// round of reads and writes. This is a busy poll with no blocking waits — on
    /// a dedicated pinned core, spinning beats syscall and wakeup latency.
    pub(crate) fn run(&mut self) -> io::Result<()> {
        // Reusable stack scratch for reaped completions, so draining does not
        // repeatedly touch the ring's shared memory.
        let mut cqe_buffer: [MaybeUninit<cqueue::Entry>; CQE_BATCH_SIZE] =
            std::array::from_fn(|_| MaybeUninit::uninit());

        loop {
            if super::shutdown_requested() {
                break;
            }

            // Keep exactly one multishot accept armed so new connections are
            // always acceptable without submitting an accept SQE per connection.
            if !self.multishot_accept_is_active {
                self.try_queue_multishot_accept();
            }

            #[cfg(feature = "uring-defer-taskrun")]
            {
                // With DEFER_TASKRUN, completions only surface when we enter the
                // ring with GETEVENTS. Enter only when there is work to do: SQEs to
                // submit, or the kernel signalled deferred task work via the
                // TASKRUN flag. Otherwise spin, avoiding a pointless syscall.
                let (to_submit, taskrun) = {
                    let submission = self.io_uring.submission();
                    (submission.len() as u32, submission.taskrun())
                };

                if to_submit != 0 || taskrun {
                    // SAFETY: the ring is live; GETEVENTS asks the kernel to run
                    // deferred task work and post any ready completions.
                    unsafe {
                        self.io_uring.submitter().enter::<libc::sigset_t>(
                            to_submit,
                            0,
                            io_uring::EnterFlags::GETEVENTS.bits(),
                            None,
                        )?;
                    }
                } else {
                    std::hint::spin_loop();
                }
            }

            // Default path: a plain submit flushes SQEs and reaps in one call.
            #[cfg(not(feature = "uring-defer-taskrun"))]
            self.io_uring.submit()?;

            // Drain the completion queue fully, in batches. Reaping everything now
            // keeps the ring from backpressuring and maximizes work per iteration.
            loop {
                let completed_cqes = {
                    let mut completion_queue = self.io_uring.completion();
                    completion_queue.fill(&mut cqe_buffer)
                };

                if completed_cqes.is_empty() {
                    break;
                }

                for cqe in completed_cqes {
                    self.handle_cqe(cqe);
                }
            }

            self.process_storage_responses();

            // Retry clients that had commands ready but hit a full request queue
            // earlier. Per-client FIFO order matters: responses are reordered back
            // into sequence downstream, but requests must go out in send order.
            while self.request_sender.has_capacity() {
                let Some(client_id) = self.clients_waiting_for_submission.pop_front() else {
                    break;
                };

                let client_read_state = &mut self.clients[client_id.0].read_state;
                let command = client_read_state
                    .pending_commands
                    .pop_front()
                    .expect("queued client should have a pending command");
                let sequence = client_read_state.next_request_sequence;

                assert!(
                    self.request_sender
                        .push(StorageRequest {
                            client_id,
                            sequence,
                            command,
                        })
                        .is_ok()
                );
                client_read_state.next_request_sequence += 1;

                if client_read_state.pending_commands.is_empty() {
                    client_read_state.submission_queued = false;
                } else {
                    self.clients_waiting_for_submission.push_back(client_id);
                }
            }

            self.queue_read_and_write_sqes();
        }

        #[allow(unreachable_code)]
        Ok(())
    }

    /// Submits a single multishot accept. Multishot means one SQE yields many
    /// CQEs (one per new connection) until the kernel reports an error or the op
    /// is cancelled — far cheaper than an accept SQE per connection. SOCK_NONBLOCK
    /// makes the accepted fd immediately usable for ring-based reads and writes.
    fn try_queue_multishot_accept(&mut self) {
        let accept_sqe = opcode::AcceptMulti::new(types::Fd(self.listener.as_raw_fd()))
            .flags(libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC)
            .build()
            .user_data(ACCEPT_CQE);

        let queued = unsafe { self.io_uring.submission().push(&accept_sqe).is_ok() };

        if queued {
            self.multishot_accept_is_active = true;
        }
    }

    /// Dispatches a completion by the operation type packed in its low two bits.
    fn handle_cqe(&mut self, cqe: &cqueue::Entry) {
        let user_data = cqe.user_data();
        let operation = user_data & 0b11;

        match operation {
            ACCEPT_CQE => self.handle_accept_cqe(cqe),
            READ_CQE => self.handle_read_cqe(cqe),
            WRITE_CQE => self.handle_write_cqe(cqe),
            _ => panic!("cannot reach here"),
        }
    }

    fn handle_accept_cqe(&mut self, cqe: &cqueue::Entry) {
        // The MORE flag means the multishot accept is still active. If it is clear,
        // the operation ended and the next loop iteration must re-arm it.
        self.multishot_accept_is_active = cqueue::more(cqe.flags());

        let accepted_fd = cqe.result();
        if accepted_fd < 0 {
            return;
        }

        // SAFETY: Transfer ownership of the new FD returned by the accept CQE to TcpStream.
        let stream = unsafe { TcpStream::from_raw_fd(accepted_fd) };

        // At capacity: drop the connection. This avoids overflowing the Slab and
        // applies natural backpressure to new clients.
        if self.clients.len() == MAX_CLIENTS {
            return;
        }

        let vacant_entry = self.clients.vacant_entry();
        vacant_entry.insert(Client::new(stream));
    }

    fn handle_read_cqe(&mut self, cqe: &cqueue::Entry) {
        let client_id = ClientId((cqe.user_data() >> 2) as usize);
        let received = cqe.result();

        self.clients[client_id.0].read_state.recv_in_flight = false;

        // ENOBUFS: the kernel had no provided buffer free for this receive. Buffers
        // are recycled after parsing, so a later iteration re-submits the read.
        if received == -libc::ENOBUFS {
            return;
        }

        // EOF (0) or any error: mark the client closed so we stop submitting reads.
        // Its slot is reclaimed when the Slab releases it.
        if received <= 0 {
            self.clients[client_id.0].read_state.is_closed = true;
            return;
        }

        // The kernel chose a provided buffer; its id is in the CQE flags. Feed the
        // received bytes straight into the client's RESP parser (no copy), then
        // recycle the buffer back to the ring for reuse.
        let bid = cqueue::buffer_select(cqe.flags())
            .expect("provided-buffer receive CQE should contain a buffer ID");
        let parse_result = {
            let input = self.provided_buffers.received(bid, received as usize);
            let client_read_state = &mut self.clients[client_id.0].read_state;

            client_read_state
                .resp_parser
                .feed(input, &mut client_read_state.pending_commands)
        };

        self.provided_buffers.recycle(bid);
        parse_result.unwrap_or_else(|error| panic!("RESP parse failed: {error}"));

        let client_read_state = &mut self.clients[client_id.0].read_state;

        // Push as many parsed commands as the request queue will accept right now.
        while self.request_sender.has_capacity() {
            let Some(command) = client_read_state.pending_commands.pop_front() else {
                break;
            };
            let sequence = client_read_state.next_request_sequence;

            assert!(
                self.request_sender
                    .push(StorageRequest {
                        client_id,
                        sequence,
                        command,
                    })
                    .is_ok()
            );
            client_read_state.next_request_sequence += 1;
        }

        // Commands left over after the request queue filled are queued for a later
        // iteration, once capacity frees up.
        if !client_read_state.pending_commands.is_empty() {
            client_read_state.submission_queued = true;
            self.clients_waiting_for_submission.push_back(client_id);
        }
    }

    fn handle_write_cqe(&mut self, cqe: &cqueue::Entry) {
        let client_id = ClientId((cqe.user_data() >> 2) as usize);
        let write_result = cqe.result();
        assert!(write_result > 0, "write failed: {write_result}");

        let client = &mut self.clients[client_id.0];
        let client_write_state = &mut client.write_state;

        client_write_state.in_flight = false;
        let mut number_of_bytes_left_to_process = write_result as usize;

        // Writev may report a short write (e.g. the socket buffer filled). Pop
        // fully-sent responses from the front; for a partially-sent front response
        // record how far we got in `front_bytes_sent`. A single write can span
        // multiple responses, so loop until all sent bytes are accounted for.
        while number_of_bytes_left_to_process > 0 {
            let response = client_write_state.pending.front().unwrap();
            let total_response_bytes = response.header_bytes.len()
                + response.value_bytes.as_ref().map_or(0, |value| value.len())
                + response.ending_bytes.len();

            let number_of_unsent_bytes_in_front_response =
                total_response_bytes - client_write_state.front_bytes_sent;

            if number_of_bytes_left_to_process < number_of_unsent_bytes_in_front_response {
                client_write_state.front_bytes_sent += number_of_bytes_left_to_process;
                break;
            }

            number_of_bytes_left_to_process -= number_of_unsent_bytes_in_front_response;
            client_write_state.pending.pop_front();
            client_write_state.front_bytes_sent = 0;
        }
    }

    /// Moves completed storage responses into their clients' write buffers,
    /// preserving per-client FIFO order. Storage may finish out of order (e.g.
    /// concurrent operations on different keys), but each client must receive
    /// responses in the sequence its requests were submitted.
    fn process_storage_responses(&mut self) {
        while let Some(storage_response) = self.response_receiver.pop() {
            let client_id = storage_response.client_id;
            let sequence = storage_response.sequence;
            let response_to_write = make_response_to_write(storage_response.reply);
            let client = &mut self.clients[client_id.0];
            let write_state = &mut client.write_state;

            // Hold out-of-order responses in `completed_out_of_order` keyed by
            // sequence until every earlier one has arrived. The asserts catch a
            // storage bug that returned the same sequence twice.
            assert!(
                sequence >= write_state.next_response_sequence,
                "storage returned the same response sequence twice"
            );
            assert!(
                write_state
                    .completed_out_of_order
                    .insert(sequence, response_to_write)
                    .is_none(),
                "storage returned the same response sequence twice"
            );

            // Flush every consecutive response starting at next_response_sequence
            // into the write queue, advancing the sequence as we go.
            while let Some(response) = write_state
                .completed_out_of_order
                .remove(&write_state.next_response_sequence)
            {
                write_state.pending.push_back(response);
                write_state.next_response_sequence += 1;
            }
        }
    }

    /// Queues read and write SQEs for every client that needs them, once per loop
    /// iteration after completions are processed. Breaking out early when the
    /// submission queue fills is safe: the next iteration resumes where we stopped.
    fn queue_read_and_write_sqes(&mut self) {
        let io_uring = &mut self.io_uring;
        let clients = &mut self.clients;
        let mut submission_queue = io_uring.submission();

        'clients: for (client_index, client) in clients.iter_mut() {
            let client_id = ClientId(client_index);
            let client_socket_fd = client.stream.as_raw_fd();

            let client_read_state = &mut client.read_state;

            // Submit a read only when: the connection is alive, no recv is already
            // in flight, the parser holds no pending commands (so the buffer is
            // free), and the client is not already waiting in the submission queue.
            let can_queue_read = !client_read_state.is_closed
                && !client_read_state.recv_in_flight
                && client_read_state.pending_commands.is_empty()
                && !client_read_state.submission_queued;

            if can_queue_read {
                // Recv with a null buffer and BUFFER_SELECT tells the kernel to pick
                // a buffer from our provided-buffer group; the chosen id comes back
                // in the CQE, so we know which buffer holds the received bytes.
                let read_sqe =
                    opcode::Recv::new(types::Fd(client_socket_fd), std::ptr::null_mut(), 0)
                        .buf_group(PROVIDED_BUFFER_GROUP_ID)
                        .build()
                        .flags(squeue::Flags::BUFFER_SELECT)
                        .user_data(make_cqe_user_data(client_id, READ_CQE));

                if unsafe { submission_queue.push(&read_sqe) }.is_err() {
                    break 'clients;
                }

                client_read_state.recv_in_flight = true;
            }

            let client_write_state = &mut client.write_state;

            if client_write_state.in_flight || client_write_state.pending.is_empty() {
                continue;
            }

            // Build an iovec array over the pending responses so writev can send
            // many responses (or the remaining parts of one) in a single syscall,
            // skipping bytes already sent via front_bytes_sent.
            client_write_state.in_flight_iovecs.clear();

            let front_bytes_sent = client_write_state.front_bytes_sent;
            let pending_responses = &client_write_state.pending;
            let in_flight_iovecs = &mut client_write_state.in_flight_iovecs;

            'responses: for (response_index, response) in pending_responses.iter().enumerate() {
                // Only the front response may be partially sent; later ones start
                // at byte 0.
                let mut number_of_bytes_to_skip = if response_index == 0 {
                    front_bytes_sent
                } else {
                    0
                };

                // Each response is three logical slices (RESP header, value, CRLF).
                // Pointing iovecs directly at them keeps the write zero-copy.
                let response_byte_slices: [&[u8]; 3] = [
                    response.header_bytes.as_ref(),
                    response.value_bytes.as_deref().unwrap_or(&[]),
                    response.ending_bytes,
                ];

                for response_bytes in response_byte_slices {
                    if number_of_bytes_to_skip >= response_bytes.len() {
                        number_of_bytes_to_skip -= response_bytes.len();
                        continue;
                    }

                    let response_bytes_to_write = &response_bytes[number_of_bytes_to_skip..];
                    number_of_bytes_to_skip = 0;

                    if response_bytes_to_write.is_empty() {
                        continue;
                    }

                    if in_flight_iovecs.len() == in_flight_iovecs.capacity() {
                        break 'responses;
                    }

                    in_flight_iovecs.push(libc::iovec {
                        iov_base: response_bytes_to_write.as_ptr() as *mut libc::c_void,
                        iov_len: response_bytes_to_write.len(),
                    });
                }
            }

            assert!(!in_flight_iovecs.is_empty());

            let write_sqe = opcode::Writev::new(
                types::Fd(client_socket_fd),
                in_flight_iovecs.as_ptr(),
                in_flight_iovecs.len() as u32,
            )
            .build()
            .user_data(make_cqe_user_data(client_id, WRITE_CQE));

            if unsafe { submission_queue.push(&write_sqe) }.is_err() {
                break;
            }

            client_write_state.in_flight = true;
        }
    }
}

/// Packs a client id and operation type into a CQE `user_data`. The low two bits
/// hold the operation (see the `*_CQE` constants); the rest hold the client id.
/// This lets the completion handler recover both without any lookup table.
pub(crate) fn make_cqe_user_data(client_id: ClientId, operation: u64) -> u64 {
    ((client_id.0 as u64) << 2) | operation
}
