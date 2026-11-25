//! A blocking TCP stream over the host's `tcp` import, for a service that
//! speaks a protocol of its own. The host holds it to the manifest's hosts.

use std::io;
use std::time::Duration;

use crate::bindings::kopuz::plugin::tcp as wit;

pub use wit::TcpError as Error;

/// Plain text for a person; the generated `Display` is the debug form.
pub fn message(error: &Error) -> String {
    match error {
        Error::Denied(message) => format!("connection refused: {message}"),
        Error::Connect(message) => format!("could not connect: {message}"),
        Error::Timeout => "the connection timed out".to_string(),
        Error::Closed => "the connection was closed".to_string(),
        Error::Other(message) => message.clone(),
    }
}

impl From<Error> for io::Error {
    fn from(error: Error) -> Self {
        let kind = match &error {
            Error::Denied(_) => io::ErrorKind::PermissionDenied,
            Error::Connect(_) => io::ErrorKind::ConnectionRefused,
            Error::Timeout => io::ErrorKind::TimedOut,
            Error::Closed => io::ErrorKind::UnexpectedEof,
            Error::Other(_) => io::ErrorKind::Other,
        };
        io::Error::new(kind, message(&error))
    }
}

/// Closed when dropped.
#[derive(Debug)]
pub struct Stream {
    id: u32,
    read_timeout: Option<Duration>,
}

fn millis(timeout: Option<Duration>) -> Option<u32> {
    timeout.map(|timeout| timeout.as_millis().min(u32::MAX as u128) as u32)
}

impl Stream {
    pub fn connect(host: &str, port: u16, timeout: Option<Duration>) -> Result<Self, Error> {
        let id = wit::connect(host, port, millis(timeout))?;
        Ok(Self {
            id,
            read_timeout: None,
        })
    }

    /// How long a read waits for its first byte; the host's default when unset.
    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) {
        self.read_timeout = timeout;
    }
}

impl io::Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let max = buf.len().min(u32::MAX as usize) as u32;
        match wit::read(self.id, max, millis(self.read_timeout)) {
            Ok(bytes) => {
                buf[..bytes.len()].copy_from_slice(&bytes);
                Ok(bytes.len())
            }
            Err(Error::Closed) => Ok(0),
            Err(error) => Err(error.into()),
        }
    }
}

impl io::Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        wit::write(self.id, buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        wit::close(self.id);
    }
}
