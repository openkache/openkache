// 1. Read one packet as a RESP message -> resp_status?
// 2. Submit a read SQE.
// 3. sock

use crate::resp::{ResponseToWrite, StatefulRespParser};
use crate::storage_message::Command;
use std::collections::{BTreeMap, VecDeque};
use std::net::TcpStream;

pub(super) const WRITE_IOVEC_CAPACITY: usize = 128 * 3;

pub(super) struct WriteState {
    pub(super) pending: VecDeque<ResponseToWrite>, // RESP responses not yet fully transmitted
    pub(super) completed_out_of_order: BTreeMap<u64, ResponseToWrite>,
    pub(super) next_response_sequence: u64,
    pub(super) front_bytes_sent: usize, // Bytes already sent from pending.front()
    pub(super) in_flight: bool,         // A send SQE is awaiting its CQE
    pub(super) in_flight_iovecs: Vec<libc::iovec>,
}

pub(super) struct ReadState {
    pub(super) resp_parser: StatefulRespParser,
    pub(super) pending_commands: VecDeque<Command>,
    pub(super) submission_queued: bool,
    pub(super) recv_in_flight: bool,
    pub(super) is_closed: bool,
    pub(super) next_request_sequence: u64,
}

pub(super) struct Client {
    pub(super) stream: TcpStream,
    pub(super) read_state: ReadState,
    pub(super) write_state: WriteState,
}

impl Client {
    pub(super) fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            read_state: ReadState {
                resp_parser: StatefulRespParser::new(),
                pending_commands: VecDeque::new(),
                submission_queued: false,
                recv_in_flight: false,
                is_closed: false,
                next_request_sequence: 0,
            },
            write_state: WriteState {
                pending: VecDeque::new(),
                completed_out_of_order: BTreeMap::new(),
                next_response_sequence: 0,
                front_bytes_sent: 0,
                in_flight: false,
                in_flight_iovecs: Vec::with_capacity(WRITE_IOVEC_CAPACITY),
            },
        }
    }
}
