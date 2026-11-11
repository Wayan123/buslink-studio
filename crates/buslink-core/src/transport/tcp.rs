//! Modbus TCP client transport over `std::net` (blocking, used from the link worker thread).

use super::{Transport, TransportError};
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub struct TcpTransport {
    stream: TcpStream,
    peer: String,
}

fn map_err(e: std::io::Error, peer: &str) -> TransportError {
    match e.kind() {
        ErrorKind::ConnectionReset
        | ErrorKind::ConnectionAborted
        | ErrorKind::BrokenPipe
        | ErrorKind::NotConnected
        | ErrorKind::UnexpectedEof => TransportError::Disconnected(format!("{peer}: {e}")),
        ErrorKind::PermissionDenied => TransportError::PermissionDenied,
        _ => TransportError::Io(format!("{peer}: {e}")),
    }
}

impl TcpTransport {
    /// Connect with a timeout. `host` may be an IP literal or a name.
    pub fn connect(host: &str, port: u16, timeout_ms: u64) -> Result<TcpTransport, TransportError> {
        let peer = format!("{host}:{port}");
        let addrs = (host, port)
            .to_socket_addrs()
            .map_err(|e| TransportError::Io(format!("{peer}: {e}")))?;
        let timeout = Duration::from_millis(timeout_ms.max(1));
        let mut last = TransportError::Io(format!("{peer}: no address"));
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, timeout) {
                Ok(stream) => {
                    stream.set_nodelay(true).map_err(|e| map_err(e, &peer))?;
                    stream.set_write_timeout(Some(timeout)).map_err(|e| map_err(e, &peer))?;
                    return Ok(TcpTransport { stream, peer });
                }
                Err(e) => last = map_err(e, &peer),
            }
        }
        Err(last)
    }

    pub fn peer(&self) -> &str {
        &self.peer
    }
}

impl Transport for TcpTransport {
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        self.stream.write_all(bytes).map_err(|e| map_err(e, &self.peer))
    }

    fn read(&mut self, buf: &mut [u8], timeout_ms: u64) -> Result<usize, TransportError> {
        self.stream
            .set_read_timeout(Some(Duration::from_millis(timeout_ms.max(1))))
            .map_err(|e| map_err(e, &self.peer))?;
        match self.stream.read(buf) {
            Ok(0) if !buf.is_empty() => Err(TransportError::Disconnected(format!("{}: closed by peer", self.peer))),
            Ok(n) => Ok(n),
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) =>
            {
                Ok(0)
            }
            Err(e) => Err(map_err(e, &self.peer)),
        }
    }

    fn close(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}
