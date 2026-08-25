use std::collections::VecDeque;
use std::io;
use std::mem::MaybeUninit;
use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd};

use io_uring::{IoUring, cqueue, opcode, squeue, types};
use slab::Slab;

use crate::client::Client;
use crate::resp::make_response_to_write;
use crate::spsc::{Consumer, Producer};
use crate::storage_message::{ClientId, STORAGE_QUEUE_SLOTS, StorageRequest, StorageResponse};

mod provided_buffer_ring;
use provided_buffer_ring::ProvidedBufferRing;

#[cfg(all(feature = "uring-coop-taskrun", feature = "uring-defer-taskrun"))]
compile_error!("uring-coop-taskrun and uring-defer-taskrun cannot be enabled together");

const MAX_CLIENTS: usize = 2_000;
const IO_URING_QUEUE_ENTRIES: u32 = 4_096;
const CQE_BATCH_SIZE: usize = 256;
const ACCEPT_CQE: u64 = 0;
const READ_CQE: u64 = 1;
const WRITE_CQE: u64 = 2;
const PROVIDED_BUFFER_GROUP_ID: u16 = 0;
const PROVIDED_BUFFER_COUNT: u16 = 2_048;
const PROVIDED_BUFFER_SIZE: usize = 4 * 1_024;

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

pub(crate) struct Network {
    listener: TcpListener,
    io_uring: IoUring,
    provided_buffers: ProvidedBufferRing,
    clients: Slab<Client>,
    request_sender: Producer<StorageRequest, STORAGE_QUEUE_SLOTS>,
    response_receiver: Consumer<StorageResponse, STORAGE_QUEUE_SLOTS>,
    clients_waiting_for_submission: VecDeque<ClientId>,
    multishot_accept_is_active: bool,
}

impl Network {
    pub(crate) fn new(
        listener: TcpListener,
        request_sender: Producer<StorageRequest, STORAGE_QUEUE_SLOTS>,
        response_receiver: Consumer<StorageResponse, STORAGE_QUEUE_SLOTS>,
    ) -> io::Result<Self> {
        let io_uring = create_io_uring()?;
        let clients = Slab::with_capacity(MAX_CLIENTS);
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

    pub(crate) fn run(&mut self) -> io::Result<()> {
        let mut cqe_buffer: [MaybeUninit<cqueue::Entry>; CQE_BATCH_SIZE] =
            std::array::from_fn(|_| MaybeUninit::uninit());

        loop {
            if !self.multishot_accept_is_active {
                self.try_queue_multishot_accept();
            }

            #[cfg(feature = "uring-defer-taskrun")]
            {
                let (to_submit, taskrun) = {
                    let submission = self.io_uring.submission();
                    (submission.len() as u32, submission.taskrun())
                };

                if to_submit != 0 || taskrun {
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

            #[cfg(not(feature = "uring-defer-taskrun"))]
            self.io_uring.submit()?;

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
        self.multishot_accept_is_active = cqueue::more(cqe.flags());

        let accepted_fd = cqe.result();
        if accepted_fd < 0 {
            return;
        }

        // SAFETY: accept CQE가 반환한 새 FD의 소유권을 TcpStream으로 옮긴다.
        let stream = unsafe { TcpStream::from_raw_fd(accepted_fd) };

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

        if received == -libc::ENOBUFS {
            return;
        }

        if received <= 0 {
            self.clients[client_id.0].read_state.is_closed = true;
            return;
        }

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

    fn process_storage_responses(&mut self) {
        while let Some(storage_response) = self.response_receiver.pop() {
            let client_id = storage_response.client_id;
            let sequence = storage_response.sequence;
            let response_to_write = make_response_to_write(storage_response.reply);
            let client = &mut self.clients[client_id.0];
            let write_state = &mut client.write_state;

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

            while let Some(response) = write_state
                .completed_out_of_order
                .remove(&write_state.next_response_sequence)
            {
                write_state.pending.push_back(response);
                write_state.next_response_sequence += 1;
            }
        }
    }

    fn queue_read_and_write_sqes(&mut self) {
        let io_uring = &mut self.io_uring;
        let clients = &mut self.clients;
        let mut submission_queue = io_uring.submission();

        'clients: for (client_index, client) in clients.iter_mut() {
            let client_id = ClientId(client_index);
            let client_socket_fd = client.stream.as_raw_fd();

            let client_read_state = &mut client.read_state;

            let can_queue_read = !client_read_state.is_closed
                && !client_read_state.recv_in_flight
                && client_read_state.pending_commands.is_empty()
                && !client_read_state.submission_queued;

            if can_queue_read {
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

            client_write_state.in_flight_iovecs.clear();

            let front_bytes_sent = client_write_state.front_bytes_sent;
            let pending_responses = &client_write_state.pending;
            let in_flight_iovecs = &mut client_write_state.in_flight_iovecs;

            'responses: for (response_index, response) in pending_responses.iter().enumerate() {
                let mut number_of_bytes_to_skip = if response_index == 0 {
                    front_bytes_sent
                } else {
                    0
                };

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

fn make_cqe_user_data(client_id: ClientId, operation: u64) -> u64 {
    ((client_id.0 as u64) << 2) | operation
}
