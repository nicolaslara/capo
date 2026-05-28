use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::Path;

use capo_core::ProjectId;

mod codec;
mod wire;

use crate::{CapoServer, ServerError, ServerRequest, ServerResponse};

const MAX_TRANSPORT_FRAME_BYTES: usize = 384 * 1024;

pub fn serve_tcp(
    listener: TcpListener,
    project_id: ProjectId,
    state_root: impl AsRef<Path>,
    max_requests: Option<usize>,
) -> TransportResult<usize> {
    let bound_address = listener.local_addr().map_err(TransportError::Io)?;
    if !bound_address.ip().is_loopback() {
        return Err(TransportError::Protocol(format!(
            "server listener must be loopback, got {bound_address}"
        )));
    }
    let server = CapoServer::open(project_id, state_root).map_err(TransportError::Server)?;
    let mut served = 0;
    while max_requests.map(|max| served < max).unwrap_or(true) {
        let (stream, _) = listener.accept().map_err(TransportError::Io)?;
        handle_stream(&server, stream)?;
        served += 1;
    }
    Ok(served)
}

pub fn send_tcp(
    address: impl ToSocketAddrs,
    request: &ServerRequest,
) -> TransportResult<ServerResponse> {
    let resolved = address
        .to_socket_addrs()
        .map_err(TransportError::Io)?
        .collect::<Vec<_>>();
    if resolved.is_empty() {
        return Err(TransportError::Protocol(
            "server address resolved to no endpoints".to_string(),
        ));
    }
    if !resolved.iter().all(|address| address.ip().is_loopback()) {
        return Err(TransportError::Protocol(format!(
            "server connect address must resolve only to loopback addresses, got {resolved:?}"
        )));
    }
    let mut stream = TcpStream::connect(resolved.as_slice()).map_err(TransportError::Io)?;
    let request_json = codec::encode_request(request);
    stream
        .write_all(request_json.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(TransportError::Io)?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(TransportError::Io)?;
    codec::decode_transport_response(&line)
}

pub type TransportResult<T> = Result<T, TransportError>;

#[derive(Debug)]
pub enum TransportError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Protocol(String),
    Server(ServerError),
    Remote { kind: String, message: String },
}

fn handle_stream(server: &CapoServer, mut stream: TcpStream) -> TransportResult<()> {
    let mut line = Vec::new();
    let read_result = {
        let mut reader = BufReader::new(&mut stream);
        read_bounded_line(&mut reader, &mut line)
    };
    let line = read_result.and_then(|_| {
        String::from_utf8(line)
            .map_err(|_| TransportError::Protocol("request frame is not valid utf-8".to_string()))
    });
    let response_line =
        match line
            .and_then(|line| codec::decode_request(&line))
            .and_then(|request| {
                server
                    .handle(request)
                    .map_err(TransportError::Server)
                    .map(|response| codec::encode_success_response(&response))
            }) {
            Ok(response) => response,
            Err(error) => codec::encode_error_response(&error),
        };
    stream
        .write_all(response_line.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(TransportError::Io)
}

fn read_bounded_line<R: BufRead>(reader: &mut R, line: &mut Vec<u8>) -> TransportResult<()> {
    loop {
        let available = reader.fill_buf().map_err(TransportError::Io)?;
        if available.is_empty() {
            return Ok(());
        }
        let consumed = match available.iter().position(|byte| *byte == b'\n') {
            Some(index) => index + 1,
            None => available.len(),
        };
        if line.len() + consumed > MAX_TRANSPORT_FRAME_BYTES {
            reader.consume(consumed);
            drain_to_line_end(reader)?;
            return Err(TransportError::Protocol(format!(
                "request frame is too large: > {MAX_TRANSPORT_FRAME_BYTES} bytes"
            )));
        }
        line.extend_from_slice(&available[..consumed]);
        reader.consume(consumed);
        if line.ends_with(b"\n") {
            return Ok(());
        }
    }
}

fn drain_to_line_end<R: BufRead>(reader: &mut R) -> TransportResult<()> {
    loop {
        let available = reader.fill_buf().map_err(TransportError::Io)?;
        if available.is_empty() {
            return Ok(());
        }
        let consumed = match available.iter().position(|byte| *byte == b'\n') {
            Some(index) => index + 1,
            None => available.len(),
        };
        let has_newline = available[..consumed].contains(&b'\n');
        reader.consume(consumed);
        if has_newline {
            return Ok(());
        }
    }
}
