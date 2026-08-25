use std::io;
use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const MAX_RESPONSE_LINE_BYTES: usize = 128;

pub(super) struct RespBackend {
    address: SocketAddr,
    stream: Option<TcpStream>,
}

impl RespBackend {
    pub(super) const fn new(address: SocketAddr) -> Self {
        Self {
            address,
            stream: None,
        }
    }

    pub(super) async fn get(&mut self, key: &[u8]) -> io::Result<Option<Vec<u8>>> {
        let request = command(&[b"GET", key]);
        let stream = self.stream().await?;
        stream.write_all(&request).await?;

        let line = read_line(stream).await?;
        let Some(length_text) = line.strip_prefix(b"$") else {
            return Err(resp_error("GET", &line));
        };
        if length_text == b"-1" {
            return Ok(None);
        }
        let length = parse_length(length_text)?;
        let mut value = vec![0; length];
        stream.read_exact(&mut value).await?;
        let mut ending = [0; 2];
        stream.read_exact(&mut ending).await?;
        if ending != *b"\r\n" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "RESP GET value has an invalid terminator",
            ));
        }
        Ok(Some(value))
    }

    pub(super) async fn set(&mut self, key: &[u8], value: &[u8]) -> io::Result<()> {
        let request = command(&[b"SET", key, value]);
        let stream = self.stream().await?;
        stream.write_all(&request).await?;

        let line = read_line(stream).await?;
        if line == b"+OK" {
            Ok(())
        } else {
            Err(resp_error("SET", &line))
        }
    }

    async fn stream(&mut self) -> io::Result<&mut TcpStream> {
        if self.stream.is_none() {
            let stream = TcpStream::connect(self.address).await?;
            stream.set_nodelay(true)?;
            self.stream = Some(stream);
        }
        Ok(self
            .stream
            .as_mut()
            .expect("RESP backend stream was initialized"))
    }
}

fn command(arguments: &[&[u8]]) -> Vec<u8> {
    let payload_bytes = arguments
        .iter()
        .map(|argument| argument.len())
        .sum::<usize>();
    let mut output = Vec::with_capacity(payload_bytes + arguments.len() * 16 + 16);
    output.extend_from_slice(format!("*{}\r\n", arguments.len()).as_bytes());
    for argument in arguments {
        output.extend_from_slice(format!("${}\r\n", argument.len()).as_bytes());
        output.extend_from_slice(argument);
        output.extend_from_slice(b"\r\n");
    }
    output
}

async fn read_line(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        if line.len() == MAX_RESPONSE_LINE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "RESP response line is too long",
            ));
        }

        let byte = stream.read_u8().await?;
        if byte == b'\r' {
            if stream.read_u8().await? != b'\n' {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "RESP response line has an invalid terminator",
                ));
            }
            return Ok(line);
        }
        line.push(byte);
    }
}

fn parse_length(input: &[u8]) -> io::Result<usize> {
    std::str::from_utf8(input)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "RESP length is not ASCII"))?
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "RESP length is invalid"))
}

fn resp_error(operation: &str, line: &[u8]) -> io::Error {
    io::Error::other(format!(
        "RESP backend rejected {operation}: {}",
        String::from_utf8_lossy(line)
    ))
}
