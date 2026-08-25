/*
  GET hello:
      header_bytes = "$5\r\n"
      value_bytes  = "hello"
      ending_bytes = "\r\n"

  GET 없는 키:
      header_bytes = "$-1\r\n"
      value_bytes  = None
      ending_bytes = ""

  SET 성공:
      header_bytes = "+OK\r\n"
      value_bytes  = None
      ending_bytes = ""

*/

use std::borrow::Cow;
use std::collections::VecDeque;
use std::mem::MaybeUninit;
use std::sync::Arc;

use crate::storage_message::{Command, Reply};

#[derive(Clone, Copy)]
enum Header {
    Array,
    Bulk,
}

#[derive(Clone, Copy)]
enum State {
    HeaderPrefix(Header),
    HeaderNumber { kind: Header, value: Option<usize> },
    HeaderLf { kind: Header, value: usize },
    BulkData { remaining: usize },
    BulkCrlf { pos: u8 },
}

enum CurrentArg {
    VecBuffer(Vec<u8>),
    SetValue {
        bytes: Arc<[MaybeUninit<u8>]>,
        written: usize,
    },
}

/*
*3\r\n
$3\r\nSET\r\n
$3\r\nkey\r\n
$5\r\nvalue\r\n
*/

pub(super) struct StatefulRespParser {
    state: State,
    arg_count: usize,   // 3
    args: Vec<Vec<u8>>, // ["SET", "key"] -> len(args) == arg_count 에서 break
    current_arg: CurrentArg,
}

impl StatefulRespParser {
    pub(super) fn new() -> Self {
        Self {
            state: State::HeaderPrefix(Header::Array),
            arg_count: 0,
            args: Vec::new(),
            current_arg: CurrentArg::VecBuffer(Vec::new()),
        }
    }

    pub(super) fn feed(
        &mut self,
        input: &[u8],
        pending_commands: &mut VecDeque<Command>,
    ) -> Result<(), &'static str> {
        let mut pos = 0;

        while pos < input.len() {
            match self.state {
                State::HeaderPrefix(kind) => {
                    let prefix = match kind {
                        Header::Array => b'*',
                        Header::Bulk => b'$',
                    };
                    if input[pos] != prefix {
                        return Err("invalid RESP prefix");
                    }
                    pos += 1;
                    self.state = State::HeaderNumber { kind, value: None };
                }
                State::HeaderNumber { kind, value } => match input[pos] {
                    byte @ b'0'..=b'9' => {
                        let digit = usize::from(byte - b'0');
                        let next_value = value
                            .unwrap_or(0)
                            .checked_mul(10)
                            .and_then(|value| value.checked_add(digit))
                            .ok_or("RESP length overflow")?;

                        pos += 1;
                        self.state = State::HeaderNumber {
                            kind,
                            value: Some(next_value),
                        };
                    }
                    b'\r' => {
                        let Some(value) = value else {
                            return Err("missing RESP number");
                        };

                        pos += 1;
                        self.state = State::HeaderLf { kind, value };
                    }
                    _ => return Err("invalid RESP number"),
                },
                State::HeaderLf { kind, value } => {
                    if input[pos] != b'\n' {
                        return Err("invalid RESP header terminator");
                    }
                    pos += 1;

                    match kind {
                        Header::Array => {
                            if value == 0 {
                                return Err("empty RESP command");
                            }

                            self.arg_count = value;
                            self.args.clear();
                            self.state = State::HeaderPrefix(Header::Bulk);
                        }
                        Header::Bulk => {
                            if self.args.len() == 2 && self.args[0].eq_ignore_ascii_case(b"SET") {
                                self.current_arg = CurrentArg::SetValue {
                                    bytes: Arc::<[u8]>::new_uninit_slice(value),
                                    written: 0,
                                };
                            } else {
                                self.current_arg = CurrentArg::VecBuffer(Vec::with_capacity(value));
                            }

                            self.state = if value == 0 {
                                State::BulkCrlf { pos: 0 }
                            } else {
                                State::BulkData { remaining: value }
                            };
                        }
                    }
                }
                State::BulkData { remaining } => {
                    let take = remaining.min(input.len() - pos);
                    let source = &input[pos..pos + take];

                    match &mut self.current_arg {
                        CurrentArg::VecBuffer(bytes) => bytes.extend_from_slice(source),
                        CurrentArg::SetValue { bytes, written } => {
                            let destination = Arc::get_mut(bytes)
                                .ok_or("SET value buffer is unexpectedly shared")?;

                            for (slot, byte) in destination[*written..*written + take]
                                .iter_mut()
                                .zip(source)
                            {
                                slot.write(*byte);
                            }
                            *written += take;
                        }
                    }
                    pos += take;

                    self.state = if take == remaining {
                        State::BulkCrlf { pos: 0 }
                    } else {
                        State::BulkData {
                            remaining: remaining - take,
                        }
                    };
                }
                State::BulkCrlf { pos: crlf_pos } => {
                    let expected = if crlf_pos == 0 { b'\r' } else { b'\n' };
                    if input[pos] != expected {
                        return Err("invalid bulk terminator");
                    }
                    pos += 1;

                    if crlf_pos == 0 {
                        self.state = State::BulkCrlf { pos: 1 };
                        continue;
                    }

                    let command = match std::mem::replace(
                        &mut self.current_arg,
                        CurrentArg::VecBuffer(Vec::new()),
                    ) {
                        CurrentArg::VecBuffer(bytes) => {
                            self.args.push(bytes);

                            if self.args.len() == 1 {
                                let command_name = &self.args[0];
                                let valid_count = if command_name.eq_ignore_ascii_case(b"GET")
                                    || command_name.eq_ignore_ascii_case(b"DEL")
                                {
                                    self.arg_count == 2
                                } else if command_name.eq_ignore_ascii_case(b"SET") {
                                    self.arg_count == 3
                                } else {
                                    return Err("unsupported command");
                                };

                                if !valid_count {
                                    return Err("invalid command argument count");
                                }
                            }

                            if self.args.len() == self.arg_count {
                                Some(command_from_args(std::mem::take(&mut self.args), None)?)
                            } else {
                                self.state = State::HeaderPrefix(Header::Bulk);
                                None
                            }
                        }
                        CurrentArg::SetValue { bytes, written } => {
                            if written != bytes.len() {
                                return Err("incomplete SET value");
                            }

                            // SAFETY: `written == bytes.len()` proves every element was initialized.
                            let value = unsafe { bytes.assume_init() };
                            Some(command_from_args(
                                std::mem::take(&mut self.args),
                                Some(value),
                            )?)
                        }
                    };

                    if let Some(command) = command {
                        pending_commands.push_back(command);
                        self.state = State::HeaderPrefix(Header::Array);
                    }
                }
            }
        }

        Ok(())
    }
}

fn command_from_args(
    args: Vec<Vec<u8>>,
    set_value: Option<Arc<[u8]>>,
) -> Result<Command, &'static str> {
    let mut args = args.into_iter();
    let command_name = args.next().ok_or("missing command name")?;
    let key = args.next().ok_or("missing key")?;

    if args.next().is_some() {
        return Err("too many command arguments");
    }

    if command_name.eq_ignore_ascii_case(b"GET") {
        if set_value.is_some() {
            return Err("GET cannot have a value");
        }
        return Ok(Command::Get {
            key: key.into_boxed_slice(),
        });
    }

    if command_name.eq_ignore_ascii_case(b"SET") {
        return Ok(Command::Set {
            key: key.into_boxed_slice(),
            value: set_value.ok_or("SET is missing value")?,
        });
    }

    if command_name.eq_ignore_ascii_case(b"DEL") {
        if set_value.is_some() {
            return Err("DEL cannot have a value");
        }
        return Ok(Command::Delete {
            key: key.into_boxed_slice(),
        });
    }

    Err("unsupported command")
}

static GET_NOT_FOUND_RESPONSE_BYTES: &[u8] = b"$-1\r\n";
static SET_OK_RESPONSE_BYTES: &[u8] = b"+OK\r\n";
static DELETE_FOUND_RESPONSE_BYTES: &[u8] = b":1\r\n";
static DELETE_NOT_FOUND_RESPONSE_BYTES: &[u8] = b":0\r\n";
static RESPONSE_END: &[u8] = b"\r\n";

pub(crate) struct ResponseToWrite {
    pub header_bytes: Cow<'static, [u8]>,
    pub value_bytes: Option<Arc<[u8]>>,
    pub ending_bytes: &'static [u8],
}

pub(crate) fn make_response_to_write(reply: Reply) -> ResponseToWrite {
    match reply {
        Reply::Get(Some(value_bytes)) => ResponseToWrite {
            header_bytes: Cow::Owned(format!("${}\r\n", value_bytes.len()).into_bytes()),
            value_bytes: Some(value_bytes),
            ending_bytes: RESPONSE_END,
        },
        Reply::Get(None) => ResponseToWrite {
            header_bytes: Cow::Borrowed(GET_NOT_FOUND_RESPONSE_BYTES),
            value_bytes: None,
            ending_bytes: &[],
        },
        Reply::SetOk => ResponseToWrite {
            header_bytes: Cow::Borrowed(SET_OK_RESPONSE_BYTES),
            value_bytes: None,
            ending_bytes: &[],
        },
        Reply::Delete(existed) => ResponseToWrite {
            header_bytes: Cow::Borrowed(if existed {
                DELETE_FOUND_RESPONSE_BYTES
            } else {
                DELETE_NOT_FOUND_RESPONSE_BYTES
            }),
            value_bytes: None,
            ending_bytes: &[],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stateful_parser_builds_set_value_across_chunks() {
        let request_bytes = b"*3\r\n$3\r\nSET\r\n$3\r\nkey\r\n$5\r\nvalue\r\n";
        let mut parser = StatefulRespParser::new();
        let mut pending_commands = VecDeque::new();

        for byte in request_bytes {
            parser
                .feed(std::slice::from_ref(byte), &mut pending_commands)
                .expect("each request byte should parse");
        }

        let command = pending_commands
            .pop_front()
            .expect("SET should enter the pending command queue");
        let Command::Set { key, value } = command else {
            panic!("stateful parser should produce SET");
        };
        assert!(pending_commands.is_empty());
        assert_eq!(&*key, b"key");
        assert_eq!(&*value, b"value");
    }

    #[test]
    fn stateful_parser_emits_every_pipelined_command() {
        let mut parser = StatefulRespParser::new();
        let mut pending_commands = VecDeque::new();
        let requests = b"*2\r\n$3\r\nGET\r\n$3\r\none\r\n*2\r\n$3\r\nGET\r\n$3\r\ntwo\r\n";

        parser
            .feed(requests, &mut pending_commands)
            .expect("pipelined requests should parse");

        let Command::Get { key } = pending_commands.pop_front().unwrap() else {
            panic!("first command should be GET");
        };
        assert_eq!(&*key, b"one");

        let Command::Get { key } = pending_commands.pop_front().unwrap() else {
            panic!("second command should be GET");
        };
        assert_eq!(&*key, b"two");
        assert!(pending_commands.is_empty());
    }

    #[test]
    fn stateful_parser_builds_delete_across_chunks() {
        let request_bytes = b"*2\r\n$3\r\nDEL\r\n$3\r\nkey\r\n";
        let mut parser = StatefulRespParser::new();
        let mut pending_commands = VecDeque::new();

        for byte in request_bytes {
            parser
                .feed(std::slice::from_ref(byte), &mut pending_commands)
                .expect("each request byte should parse");
        }

        let Command::Delete { key } = pending_commands.pop_front().unwrap() else {
            panic!("stateful parser should produce DEL");
        };
        assert_eq!(&*key, b"key");
        assert!(pending_commands.is_empty());
    }

    #[test]
    fn get_found_response_owns_only_its_header() {
        let response = make_response_to_write(Reply::Get(Some(Arc::from(&b"value"[..]))));

        assert!(matches!(&response.header_bytes, Cow::Owned(_)));
        assert_eq!(response.header_bytes.as_ref(), b"$5\r\n");
        assert_eq!(response.value_bytes.as_deref(), Some(&b"value"[..]));
        assert_eq!(response.ending_bytes, b"\r\n");
    }

    #[test]
    fn get_not_found_responses_share_static_bytes() {
        let first_response = make_response_to_write(Reply::Get(None));
        let second_response = make_response_to_write(Reply::Get(None));

        assert!(matches!(&first_response.header_bytes, Cow::Borrowed(_)));
        assert_eq!(first_response.header_bytes.as_ref(), b"$-1\r\n");
        assert_eq!(
            first_response.header_bytes.as_ptr(),
            second_response.header_bytes.as_ptr()
        );
    }

    #[test]
    fn set_ok_responses_share_static_bytes() {
        let first_response = make_response_to_write(Reply::SetOk);
        let second_response = make_response_to_write(Reply::SetOk);

        assert!(matches!(&first_response.header_bytes, Cow::Borrowed(_)));
        assert_eq!(first_response.header_bytes.as_ref(), b"+OK\r\n");
        assert_eq!(
            first_response.header_bytes.as_ptr(),
            second_response.header_bytes.as_ptr()
        );
    }

    #[test]
    fn delete_responses_use_resp_integers() {
        let deleted = make_response_to_write(Reply::Delete(true));
        let not_found = make_response_to_write(Reply::Delete(false));

        assert_eq!(deleted.header_bytes.as_ref(), b":1\r\n");
        assert_eq!(not_found.header_bytes.as_ref(), b":0\r\n");
        assert!(deleted.value_bytes.is_none());
        assert!(not_found.value_bytes.is_none());
    }
}
