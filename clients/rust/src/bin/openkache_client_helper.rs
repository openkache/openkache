//! Length-prefixed native transport helper for the Node.js TypeScript client.

use std::io::{self, BufReader, BufWriter, Read, Write};
use std::net::SocketAddr;

use openkache_client::value::{Compression, ENCRYPTION_KEY_BYTES, ValueCodec, ZstandardOptions};
use openkache_client::{
    Client, ClientIdentity, ClientOptions, SetCondition, SetOptions, SetOutcome,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};

const MAX_HELPER_FRAME_BYTES: usize = 32 * 1024 * 1024;

const COMMAND_CONNECT: u8 = 1;
const COMMAND_EXECUTE: u8 = 2;
const COMMAND_CLOSE: u8 = 3;

const OPERATION_PING: u8 = 1;
const OPERATION_GET: u8 = 2;
const OPERATION_SET: u8 = 3;
const OPERATION_DELETE: u8 = 4;
const OPERATION_STATS: u8 = 5;
const OPERATION_SYNC: u8 = 6;

const RESULT_ERROR: u8 = 0;
const RESULT_OK: u8 = 1;
const RESULT_VALUE: u8 = 2;
const RESULT_NOT_FOUND: u8 = 3;
const RESULT_CREATED: u8 = 4;
const RESULT_REPLACED: u8 = 5;
const RESULT_DELETED: u8 = 6;
const RESULT_NOT_DELETED: u8 = 7;
const RESULT_CONNECTED: u8 = 8;
const RESULT_NOT_STORED: u8 = 9;

fn main() {
    if let Err(error) = run() {
        eprintln!("OpenKache client helper failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), HelperError> {
    let runtime = compio::runtime::Runtime::new()?;
    if !runtime.driver_type().is_iouring() {
        return Err(HelperError::Protocol(
            "OpenKache client requires the Compio io_uring driver".to_string(),
        ));
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    let mut client = None;

    while let Some(request) = read_request(&mut reader)? {
        let request_id = request.request_id;
        let should_close = matches!(request.command, Command::Close);
        let response = match handle_request(&runtime, &mut client, request.command) {
            Ok(response) => response,
            Err(error) => Response::error(error.to_string()),
        };
        write_response(&mut writer, request_id, response)?;
        if should_close {
            break;
        }
    }
    Ok(())
}

fn handle_request(
    runtime: &compio::runtime::Runtime,
    client: &mut Option<Client>,
    command: Command,
) -> Result<Response, HelperError> {
    match command {
        Command::Connect(options) => {
            if client.is_some() {
                return Err(HelperError::Protocol(
                    "native helper is already connected".to_string(),
                ));
            }
            *client = Some(connect(runtime, options)?);
            Ok(Response::success(RESULT_CONNECTED))
        }
        Command::Execute(request) => {
            let client = client.as_ref().ok_or_else(|| {
                HelperError::Protocol("native helper is not connected".to_string())
            })?;
            execute(runtime, client, request)
        }
        Command::Close => {
            *client = None;
            Ok(Response::success(RESULT_OK))
        }
    }
}

fn connect(
    runtime: &compio::runtime::Runtime,
    options: ConnectionOptions,
) -> Result<Client, HelperError> {
    let address: SocketAddr = options
        .address
        .parse()
        .map_err(|error| HelperError::Protocol(format!("invalid server address: {error}")))?;
    let trusted_certificates = parse_certificates(options.certificate)?;
    if trusted_certificates.len() != 1 {
        return Err(HelperError::Protocol(format!(
            "certificate must contain exactly one DER or PEM certificate, got {}",
            trusted_certificates.len()
        )));
    }

    let compression = if options.compression_enabled {
        Compression::Zstandard(ZstandardOptions {
            level: options.compression_level,
            minimum_input_size: options.minimum_input_size,
            minimum_savings: options.minimum_savings,
        })
    } else {
        Compression::Disabled
    };
    let encryption_key: [u8; ENCRYPTION_KEY_BYTES] =
        options.encryption_key.try_into().map_err(|key: Vec<u8>| {
            HelperError::Protocol(format!(
                "encryption key must contain {ENCRYPTION_KEY_BYTES} bytes, got {}",
                key.len()
            ))
        })?;
    let value_codec = ValueCodec::encrypted(encryption_key, compression)?;
    let identity = parse_identity(options.identity)?;

    runtime
        .block_on(Client::connect_with_options(
            address,
            &options.server_name,
            trusted_certificates[0].as_ref(),
            ClientOptions {
                value_codec,
                identity,
            },
        ))
        .map_err(HelperError::from)
}

fn parse_identity(identity: Option<Identity>) -> Result<Option<ClientIdentity>, HelperError> {
    let Some(identity) = identity else {
        return Ok(None);
    };
    let mut certificate_chain = Vec::new();
    for certificate in identity.certificate_chain {
        certificate_chain.extend(parse_certificates(certificate)?);
    }
    if certificate_chain.is_empty() {
        return Err(HelperError::Protocol(
            "client certificate chain must not be empty".to_string(),
        ));
    }
    let private_key = parse_private_key(identity.private_key)?;
    Ok(Some(ClientIdentity::new(certificate_chain, private_key)))
}

fn parse_certificates(bytes: Vec<u8>) -> Result<Vec<CertificateDer<'static>>, HelperError> {
    if bytes.is_empty() {
        return Err(HelperError::Protocol(
            "certificate must not be empty".to_string(),
        ));
    }
    if bytes.starts_with(b"-----BEGIN") {
        let certificates = CertificateDer::pem_slice_iter(&bytes)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| HelperError::Protocol(format!("invalid PEM certificate: {error}")))?;
        if certificates.is_empty() {
            return Err(HelperError::Protocol(
                "PEM input contains no certificates".to_string(),
            ));
        }
        Ok(certificates)
    } else {
        Ok(vec![CertificateDer::from(bytes)])
    }
}

fn parse_private_key(bytes: Vec<u8>) -> Result<PrivateKeyDer<'static>, HelperError> {
    if bytes.is_empty() {
        return Err(HelperError::Protocol(
            "client private key must not be empty".to_string(),
        ));
    }
    if bytes.starts_with(b"-----BEGIN") {
        return PrivateKeyDer::from_pem_slice(&bytes)
            .map_err(|error| HelperError::Protocol(format!("invalid PEM private key: {error}")));
    }
    PrivateKeyDer::try_from(bytes)
        .map_err(|_| HelperError::Protocol("private key DER is invalid".to_string()))
}

fn execute(
    runtime: &compio::runtime::Runtime,
    client: &Client,
    request: ExecuteRequest,
) -> Result<Response, HelperError> {
    runtime
        .block_on(async {
            match request.operation {
                OPERATION_PING => client.ping().await.map(|()| Response::success(RESULT_OK)),
                OPERATION_GET => client.get(&request.key).await.map(|value| match value {
                    Some(value) => Response::value(RESULT_VALUE, value),
                    None => Response::success(RESULT_NOT_FOUND),
                }),
                OPERATION_SET => client
                    .set_owned_with_options(
                        &request.key,
                        request.value,
                        SetOptions::new(request.condition, request.ttl_ms),
                    )
                    .await
                    .map(|outcome| match outcome {
                        SetOutcome::Created => Response::success(RESULT_CREATED),
                        SetOutcome::Replaced => Response::success(RESULT_REPLACED),
                        SetOutcome::NotStored => Response::success(RESULT_NOT_STORED),
                    }),
                OPERATION_DELETE => client.delete(&request.key).await.map(|deleted| {
                    Response::success(if deleted {
                        RESULT_DELETED
                    } else {
                        RESULT_NOT_DELETED
                    })
                }),
                OPERATION_STATS => client
                    .stats()
                    .await
                    .map(|stats| Response::value(RESULT_VALUE, stats.into_bytes())),
                OPERATION_SYNC => client.sync().await.map(|()| Response::success(RESULT_OK)),
                operation => Err(openkache_client::Error::Connection(format!(
                    "unsupported helper operation {operation}"
                ))),
            }
        })
        .map_err(HelperError::from)
}

fn read_request(reader: &mut impl Read) -> Result<Option<Request>, HelperError> {
    let Some(frame) = read_frame(reader)? else {
        return Ok(None);
    };
    let mut decoder = Decoder::new(&frame);
    let request_id = decoder.u32()?;
    let command = match decoder.u8()? {
        COMMAND_CONNECT => Command::Connect(ConnectionOptions {
            address: decoder.string()?,
            server_name: decoder.string()?,
            certificate: decoder.bytes()?,
            identity: decode_identity(&mut decoder)?,
            encryption_key: decoder.bytes()?,
            compression_enabled: decoder.boolean()?,
            compression_level: decoder.i32()?,
            minimum_input_size: decoder.usize()?,
            minimum_savings: decoder.usize()?,
        }),
        COMMAND_EXECUTE => Command::Execute(ExecuteRequest {
            operation: decoder.u8()?,
            condition: match decoder.u8()? {
                0 => SetCondition::None,
                1 => SetCondition::Nx,
                2 => SetCondition::Xx,
                condition => {
                    return Err(HelperError::Protocol(format!(
                        "unsupported SET condition {condition}"
                    )));
                }
            },
            ttl_ms: match decoder.u64()? {
                0 => None,
                ttl_ms => Some(ttl_ms),
            },
            key: decoder.bytes()?,
            value: decoder.bytes()?,
        }),
        COMMAND_CLOSE => Command::Close,
        command => {
            return Err(HelperError::Protocol(format!(
                "unsupported helper command {command}"
            )));
        }
    };
    decoder.finish()?;
    Ok(Some(Request {
        request_id,
        command,
    }))
}

fn decode_identity(decoder: &mut Decoder<'_>) -> Result<Option<Identity>, HelperError> {
    let certificate_count = decoder.u16()? as usize;
    if certificate_count == 0 {
        let private_key = decoder.bytes()?;
        if !private_key.is_empty() {
            return Err(HelperError::Protocol(
                "client private key requires a certificate chain".to_string(),
            ));
        }
        return Ok(None);
    }
    let mut certificate_chain = Vec::with_capacity(certificate_count);
    for _ in 0..certificate_count {
        certificate_chain.push(decoder.bytes()?);
    }
    Ok(Some(Identity {
        certificate_chain,
        private_key: decoder.bytes()?,
    }))
}

fn read_frame(reader: &mut impl Read) -> Result<Option<Vec<u8>>, HelperError> {
    let mut encoded_length = [0_u8; 4];
    if reader.read(&mut encoded_length[..1])? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut encoded_length[1..])?;
    let length = u32::from_be_bytes(encoded_length) as usize;
    if length > MAX_HELPER_FRAME_BYTES {
        return Err(HelperError::Protocol(format!(
            "helper request contains {length} bytes, maximum is {MAX_HELPER_FRAME_BYTES}"
        )));
    }
    let mut frame = vec![0_u8; length];
    reader.read_exact(&mut frame)?;
    Ok(Some(frame))
}

fn write_response(
    writer: &mut impl Write,
    request_id: u32,
    response: Response,
) -> Result<(), HelperError> {
    let frame_length = 6_usize
        .checked_add(response.payload.len())
        .ok_or_else(|| HelperError::Protocol("helper response length overflow".to_string()))?;
    let frame_length = u32::try_from(frame_length)
        .map_err(|_| HelperError::Protocol("helper response is too large".to_string()))?;
    writer.write_all(&frame_length.to_be_bytes())?;
    writer.write_all(&request_id.to_be_bytes())?;
    writer.write_all(&[u8::from(response.ok), response.result_kind])?;
    writer.write_all(&response.payload)?;
    writer.flush()?;
    Ok(())
}

struct Request {
    request_id: u32,
    command: Command,
}

enum Command {
    Connect(ConnectionOptions),
    Execute(ExecuteRequest),
    Close,
}

struct ConnectionOptions {
    address: String,
    server_name: String,
    certificate: Vec<u8>,
    identity: Option<Identity>,
    encryption_key: Vec<u8>,
    compression_enabled: bool,
    compression_level: i32,
    minimum_input_size: usize,
    minimum_savings: usize,
}

struct Identity {
    certificate_chain: Vec<Vec<u8>>,
    private_key: Vec<u8>,
}

struct ExecuteRequest {
    operation: u8,
    condition: SetCondition,
    ttl_ms: Option<u64>,
    key: Vec<u8>,
    value: Vec<u8>,
}

struct Response {
    ok: bool,
    result_kind: u8,
    payload: Vec<u8>,
}

impl Response {
    fn success(result_kind: u8) -> Self {
        Self {
            ok: true,
            result_kind,
            payload: Vec::new(),
        }
    }

    fn value(result_kind: u8, payload: Vec<u8>) -> Self {
        Self {
            ok: true,
            result_kind,
            payload,
        }
    }

    fn error(message: String) -> Self {
        Self {
            ok: false,
            result_kind: RESULT_ERROR,
            payload: message.into_bytes(),
        }
    }
}

struct Decoder<'a> {
    frame: &'a [u8],
    offset: usize,
}

impl<'a> Decoder<'a> {
    fn new(frame: &'a [u8]) -> Self {
        Self { frame, offset: 0 }
    }

    fn u8(&mut self) -> Result<u8, HelperError> {
        Ok(self.take(1)?[0])
    }

    fn boolean(&mut self) -> Result<bool, HelperError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(HelperError::Protocol(format!(
                "invalid helper boolean {value}"
            ))),
        }
    }

    fn u16(&mut self) -> Result<u16, HelperError> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, HelperError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32, HelperError> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, HelperError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn usize(&mut self) -> Result<usize, HelperError> {
        usize::try_from(self.u64()?).map_err(|_| {
            HelperError::Protocol("helper integer exceeds the platform limit".to_string())
        })
    }

    fn bytes(&mut self) -> Result<Vec<u8>, HelperError> {
        let length = self.u32()? as usize;
        Ok(self.take(length)?.to_vec())
    }

    fn string(&mut self) -> Result<String, HelperError> {
        String::from_utf8(self.bytes()?)
            .map_err(|error| HelperError::Protocol(format!("helper string is not UTF-8: {error}")))
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], HelperError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| HelperError::Protocol("helper frame length overflow".to_string()))?;
        let bytes = self.frame.get(self.offset..end).ok_or_else(|| {
            HelperError::Protocol(format!("helper frame is truncated at byte {}", self.offset))
        })?;
        self.offset = end;
        Ok(bytes)
    }

    fn finish(self) -> Result<(), HelperError> {
        if self.offset == self.frame.len() {
            Ok(())
        } else {
            Err(HelperError::Protocol(format!(
                "helper frame has {} trailing bytes",
                self.frame.len() - self.offset
            )))
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum HelperError {
    #[error("I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Protocol(String),
    #[error("{0}")]
    Client(#[from] openkache_client::Error),
    #[error("{0}")]
    Value(#[from] openkache_client::value::Error),
}
