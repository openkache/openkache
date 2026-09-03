//! RESP (REdis Serialization Protocol) parser and response formatter.
//!
//! This module parses incoming RESP arrays (commands) from client socket buffers
//! and constructs RESP bulk-string responses for values returned by storage. The
//! parser is stateful, so partial receives spanning multiple socket reads are
//! handled naturally without re-buffering. SET values allocate straight into an
//! Arc-backed buffer to avoid an extra copy when passing to storage.

/*
  GET hello:
      header_bytes = "$5\r\n"
      value_bytes  = "hello"
      ending_bytes = "\r\n"

  GET missing key:
      header_bytes = "$-1\r\n"
      value_bytes  = None
      ending_bytes = ""

  Successful SET:
      header_bytes = "+OK\r\n"
      value_bytes  = None
      ending_bytes = ""

*/

use std::borrow::Cow;
use std::collections::VecDeque;
use std::mem::MaybeUninit;
use std::sync::Arc;

use crate::storage_message::{Command, Reply, StorageKey};

/// Which kind of RESP header is being parsed. RESP prefixes each element with a
/// type byte: `*` for arrays and `$` for bulk strings.
#[derive(Clone, Copy)]
enum Header {
    Array,
    Bulk,
}

/// The parser's position within a RESP message. Modeling this as an explicit
/// state machine lets `feed` resume mid-message when a socket read splits a
/// message, without buffering the whole thing first.
#[derive(Clone, Copy)]
enum State {
    /// Expecting the header type byte (`*` or `$`).
    HeaderPrefix(Header),
    /// Accumulating the header's decimal length digits.
    HeaderNumber { kind: Header, value: Option<usize> },
    /// Expecting the `\n` that completes a `\r\n` header terminator.
    HeaderLf { kind: Header, value: usize },
    /// Reading the raw bytes of a bulk string body.
    BulkData { remaining: usize },
    /// Consuming the trailing `\r\n` after a bulk string body (`pos` tracks which).
    BulkCrlf { pos: u8 },
}

/// Where the currently-parsed argument's bytes accumulate. A SET value goes
/// directly into a shared Arc buffer so storage can take ownership without a
/// copy; every other argument collects into a plain Vec.
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

/// A streaming RESP parser. One instance lives per client, so partial messages
/// across successive socket reads are parsed incrementally without re-buffering.
pub(super) struct StatefulRespParser {
    state: State,
    arg_count: usize,   // 3
    args: Vec<Vec<u8>>, // ["SET", "key"] -> break when len(args) == arg_count
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

    /// Feeds received bytes through the state machine, pushing each completed
    /// command onto `pending_commands`. Returns an error on malformed RESP, which
    /// the caller treats as a fatal client error. State persists in `self` between
    /// calls, so a message split across reads resumes exactly where it left off.
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
                            // The third argument of a SET is the value. Allocate it
                            // as a shared Arc buffer now so storage takes ownership
                            // with no copy; all other args use a plain growable Vec.
                            if self.args.len() == 2 && self.args[0].eq_ignore_ascii_case(b"SET") {
                                self.current_arg = CurrentArg::SetValue {
                                    bytes: Arc::<[u8]>::new_uninit_slice(value),
                                    written: 0,
                                };
                            } else {
                                self.current_arg = CurrentArg::VecBuffer(Vec::with_capacity(value));
                            }

                            // A zero-length bulk string has no body, so skip
                            // straight to consuming its trailing CRLF.
                            self.state = if value == 0 {
                                State::BulkCrlf { pos: 0 }
                            } else {
                                State::BulkData { remaining: value }
                            };
                        }
                    }
                }
                State::BulkData { remaining } => {
                    // Copy only as much as this input chunk holds; a large value may
                    // arrive over several reads, so `remaining` carries across calls.
                    let take = remaining.min(input.len() - pos);
                    let source = &input[pos..pos + take];

                    match &mut self.current_arg {
                        CurrentArg::VecBuffer(bytes) => bytes.extend_from_slice(source),
                        CurrentArg::SetValue { bytes, written } => {
                            // Write directly into the Arc buffer. `get_mut` succeeds
                            // because the parser is still its sole owner until the
                            // value is finalized and handed off.
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
                                } else if command_name.eq_ignore_ascii_case(b"FLUSH") {
                                    self.arg_count == 1
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

/// Builds a `Command` from the fully-parsed argument list, validating each
/// command's shape (GET/DEL take a key; SET takes a key and value; FLUSH takes
/// nothing) and rejecting anything unsupported or malformed.
fn command_from_args(
    args: Vec<Vec<u8>>,
    set_value: Option<Arc<[u8]>>,
) -> Result<Command, &'static str> {
    let mut args = args.into_iter();
    let command_name = args.next().ok_or("missing command name")?;

    // FLUSH takes no key or value; handle it before the key is required.
    if command_name.eq_ignore_ascii_case(b"FLUSH") {
        if set_value.is_some() {
            return Err("FLUSH cannot have a value");
        }
        if args.next().is_some() {
            return Err("too many command arguments");
        }
        return Ok(Command::Flush);
    }

    let key = StorageKey::from_client_key(&args.next().ok_or("missing key")?);

    if args.next().is_some() {
        return Err("too many command arguments");
    }

    if command_name.eq_ignore_ascii_case(b"GET") {
        if set_value.is_some() {
            return Err("GET cannot have a value");
        }
        return Ok(Command::Get { key });
    }

    if command_name.eq_ignore_ascii_case(b"SET") {
        return Ok(Command::Set {
            key,
            value: set_value.ok_or("SET is missing value")?,
        });
    }

    if command_name.eq_ignore_ascii_case(b"DEL") {
        if set_value.is_some() {
            return Err("DEL cannot have a value");
        }
        return Ok(Command::Delete { key });
    }

    Err("unsupported command")
}

// Pre-encoded RESP response fragments for common fixed replies. Using static
// slices avoids re-formatting these on every request.
static GET_NOT_FOUND_RESPONSE_BYTES: &[u8] = b"$-1\r\n";
static SET_OK_RESPONSE_BYTES: &[u8] = b"+OK\r\n";
static DELETE_FOUND_RESPONSE_BYTES: &[u8] = b":1\r\n";
static DELETE_NOT_FOUND_RESPONSE_BYTES: &[u8] = b":0\r\n";
static RESPONSE_END: &[u8] = b"\r\n";

/// A response split into three slices so the network layer can write them all
/// with a single vectored write (writev) without concatenating first. The value
/// is kept Arc-backed, so it can be shared with storage without copying.
pub(crate) struct ResponseToWrite {
    pub header_bytes: Cow<'static, [u8]>,
    pub value_bytes: Option<Arc<[u8]>>,
    pub ending_bytes: &'static [u8],
}

/// Converts a storage `Reply` into RESP bytes ready to write. Fixed replies borrow
/// static slices; only a GET hit needs an owned header (the `$length\r\n` line).
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
        Reply::Flush(Ok(())) => ResponseToWrite {
            header_bytes: Cow::Borrowed(SET_OK_RESPONSE_BYTES),
            value_bytes: None,
            ending_bytes: &[],
        },
        Reply::Flush(Err(reason)) => ResponseToWrite {
            header_bytes: Cow::Owned(format!("-ERR {reason}\r\n").into_bytes()),
            value_bytes: None,
            ending_bytes: &[],
        },
    }
}
