//! Dedicated management-plane HTTP listener for health and metrics.

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::service::ObservabilityState;

const MAX_HTTP_REQUEST_BYTES: usize = 8 * 1024;
const HTTP_ACCEPT_POLL: Duration = Duration::from_millis(25);
const HTTP_READ_TIMEOUT: Duration = Duration::from_secs(1);

fn status_response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        503 => "Service Unavailable",
        _ => "Bad Request",
    };
    let mut response = Vec::with_capacity(body.len() + 256);
    write!(
        response,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("writing a Vec cannot fail");
    response.extend_from_slice(body);
    response
}

fn handle_http_connection(
    mut stream: TcpStream,
    state: &ObservabilityState,
    metrics_scrapes: &mut u64,
) {
    let _ = stream.set_read_timeout(Some(HTTP_READ_TIMEOUT));
    let mut request = [0u8; MAX_HTTP_REQUEST_BYTES];
    let mut bytes = 0;
    loop {
        let Ok(read) = stream.read(&mut request[bytes..]) else {
            return;
        };
        if read == 0 {
            return;
        }
        bytes += read;
        if request[..bytes].windows(4).any(|window| window == b"\r\n\r\n")
            || bytes == request.len()
        {
            break;
        }
    }
    let request = &request[..bytes];
    let first_line = request
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let first_line = first_line.strip_suffix(b"\r").unwrap_or(first_line);
    let mut parts = first_line.split(|byte| *byte == b' ');
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    let response = if method != b"GET" || !version.starts_with(b"HTTP/1.") {
        status_response(405, "text/plain; charset=utf-8", b"method not allowed\n")
    } else if path == b"/metrics" {
        *metrics_scrapes = metrics_scrapes.saturating_add(1);
        let body = state.render_prometheus_with_scrapes(*metrics_scrapes);
        status_response(
            200,
            "text/plain; version=0.0.4; charset=utf-8",
            body.as_bytes(),
        )
    } else if path == b"/livez" {
        status_response(200, "text/plain; charset=utf-8", b"ok\n")
    } else if path == b"/readyz" {
        if state.is_ready() {
            status_response(200, "text/plain; charset=utf-8", b"ready\n")
        } else {
            status_response(503, "text/plain; charset=utf-8", b"not ready\n")
        }
    } else {
        status_response(404, "text/plain; charset=utf-8", b"not found\n")
    };
    let _ = stream.write_all(&response);
    let _ = stream.shutdown(Shutdown::Write);
}

fn run_metrics_listener(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    state: Arc<ObservabilityState>,
) {
    let _ = listener.set_nonblocking(true);
    let mut metrics_scrapes = 0;
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => handle_http_connection(stream, &state, &mut metrics_scrapes),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(HTTP_ACCEPT_POLL);
            }
            Err(_) => break,
        }
    }
}

/// A dedicated local management listener for metrics and health probes.
pub(crate) struct MetricsEndpoint {
    listener: TcpListener,
    local_addr: SocketAddr,
    state: Arc<ObservabilityState>,
}

impl MetricsEndpoint {
    pub(crate) fn bind(
        listen: Option<SocketAddr>,
        allow_remote: bool,
        state: Arc<ObservabilityState>,
    ) -> std::io::Result<Option<Self>> {
        let Some(listen) = listen else {
            return Ok(None);
        };
        if !allow_remote && !listen.ip().is_loopback() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "observability.metrics_listen must be loopback unless metrics_allow_remote is true",
            ));
        }
        let listener = TcpListener::bind(listen)?;
        listener.set_nonblocking(true)?;
        let local_addr = listener.local_addr()?;
        Ok(Some(Self {
            listener,
            local_addr,
            state,
        }))
    }

    pub(crate) fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub(crate) fn start(self) -> MetricsEndpointHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let state = Arc::clone(&self.state);
        let listener = self.listener;
        let thread = thread::Builder::new()
            .name("openkache-observability".into())
            .spawn(move || run_metrics_listener(listener, thread_stop, state))
            .expect("observability listener thread must start");
        MetricsEndpointHandle {
            stop,
            thread: Some(thread),
        }
    }
}

pub(crate) struct MetricsEndpointHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MetricsEndpointHandle {
    pub(crate) fn stop(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
