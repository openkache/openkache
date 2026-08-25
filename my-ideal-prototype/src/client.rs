// 1. packet 을 resp 형태로 하나 읽어내기 -> resp_status ?
// 2. sqe 로 read sq 제출
// 3. sock

use crate::resp::{ResponseToWrite, StatefulRespParser};
use crate::storage_message::Command;
use std::collections::VecDeque;
use std::net::TcpStream;

pub(super) const WRITE_IOVEC_CAPACITY: usize = 128 * 3;

pub(super) struct WriteState {
    pub(super) pending: VecDeque<ResponseToWrite>, // 아직 완전히 전송되지 않은 RESP 응답들
    pub(super) front_bytes_sent: usize,            // pending.front()에서 이미 전송된 바이트 수
    pub(super) in_flight: bool, // 현재 Send SQE를 제출했고 아직 CQE를 받지 않은 상태
    pub(super) in_flight_iovecs: Vec<libc::iovec>,
}

pub(super) struct ReadState {
    pub(super) resp_parser: StatefulRespParser,
    pub(super) pending_commands: VecDeque<Command>,
    pub(super) submission_queued: bool,
    pub(super) recv_in_flight: bool,
    pub(super) is_closed: bool,
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
            },
            write_state: WriteState {
                pending: VecDeque::new(),
                front_bytes_sent: 0,
                in_flight: false,
                in_flight_iovecs: Vec::with_capacity(WRITE_IOVEC_CAPACITY),
            },
        }
    }
}
