//! The Unix socket door, for the macroquad `jarvis-orb`. Ticket 06 of jarvis-ui
//! retires it; until then it carries the same messages as the WebSocket. One JSON
//! message per line.

use std::{
    convert::Infallible,
    fs, io,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
};

use futures_util::{StreamExt, stream::FuturesUnordered};
use orb_protocol::{FromOrb, MAX_LINE_BYTES, ToOrb};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{
        UnixListener, UnixStream,
        unix::{OwnedReadHalf, OwnedWriteHalf},
    },
    time::timeout,
};

use super::{
    ACCEPT_RETRY_DELAY, Inbox, MAX_CONNECTIONS, OrbError, WRITE_TIMEOUT,
    hub::{Gone, Hub},
};

/// Only the user may connect: read and write for the owner, nothing for others.
const SOCKET_MODE: u32 = 0o600;

/// The bound socket. Dropping it (or the `serve` future) closes every
/// connection and removes the socket file.
pub(super) struct UnixEndpoint {
    listener: UnixListener,
    _file: SocketFile,
}

/// Removes the socket file when the server goes away.
struct SocketFile(PathBuf);

impl Drop for SocketFile {
    fn drop(&mut self) {
        // Already gone is fine; nothing useful can be done about any other failure.
        let _ = fs::remove_file(&self.0);
    }
}

impl UnixEndpoint {
    /// Opens the socket at `path`, for the owner only.
    ///
    /// A socket file left by a Jarvis that did not shut down cleanly is removed
    /// first. A live one (something accepts connections on it) is left alone and
    /// reported, and so is any file that is not a socket: this never deletes
    /// something it did not create. Must be called inside the Tokio runtime.
    pub(super) fn bind(path: &Path) -> Result<Self, OrbError> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if !metadata.file_type().is_socket() => {
                return Err(OrbError::NotASocket(path.to_owned()));
            }
            Ok(_) => {
                // A local connect answers at once: accepted, or refused when stale.
                if std::os::unix::net::UnixStream::connect(path).is_ok() {
                    return Err(OrbError::InUse(path.to_owned()));
                }
                fs::remove_file(path).map_err(|source| OrbError::RemoveStale {
                    path: path.to_owned(),
                    source,
                })?;
            }
            // Nothing there (or not visible): binding reports any real problem.
            Err(_) => {}
        }
        let listener = UnixListener::bind(path).map_err(|source| OrbError::Bind {
            path: path.to_owned(),
            source,
        })?;
        let file = SocketFile(path.to_owned());
        fs::set_permissions(path, fs::Permissions::from_mode(SOCKET_MODE)).map_err(|source| {
            OrbError::Restrict {
                path: path.to_owned(),
                source,
            }
        })?;
        Ok(Self {
            listener,
            _file: file,
        })
    }

    /// Serves orbs until dropped. It never completes and never fails: a connection
    /// that breaks is closed and the next one is accepted.
    ///
    /// Every connection runs inside this future (no spawned tasks), so dropping it
    /// at any `.await` closes them all and loses nothing: the hub holds the state.
    pub(super) async fn serve(self, hub: &Hub, inbox: &Inbox) -> Infallible {
        let mut connections = FuturesUnordered::new();
        loop {
            tokio::select! {
                accepted = self.listener.accept() => match accepted {
                    // Over the limit the stream is dropped, which closes it.
                    Ok((stream, _)) if connections.len() < MAX_CONNECTIONS => {
                        connections.push(serve_orb(stream, hub, inbox));
                    }
                    Ok(_) => {}
                    Err(_) => tokio::time::sleep(ACCEPT_RETRY_DELAY).await,
                },
                Some(()) = connections.next(), if !connections.is_empty() => {}
            }
        }
    }
}

/// One orb, from its first byte to its last. Returns when the orb goes away or
/// stops reading.
async fn serve_orb(stream: UnixStream, hub: &Hub, inbox: &Inbox) {
    let (read, mut write) = stream.into_split();
    let mut lines = OrbLines::new(read);
    let mut outbox = hub.subscribe();
    loop {
        let message = tokio::select! {
            // The user's tap first, then what the orb is to be shown.
            biased;
            line = lines.next() => match line {
                Ok(Incoming::Message(message)) => {
                    inbox.deliver(message);
                    continue;
                }
                Ok(Incoming::Rejected) => continue,
                Ok(Incoming::Closed) | Err(_) => return,
            },
            next = outbox.next() => match next {
                Ok(message) => message,
                Err(Gone) => return,
            },
        };
        if send(&mut write, message).await.is_err() {
            return;
        }
    }
}

/// The orb did not take a message: it is gone, or stuck.
struct OrbStuck;

async fn send(write: &mut OwnedWriteHalf, message: ToOrb) -> Result<(), OrbStuck> {
    // Every `ToOrb` is far under the line limit, so encoding cannot fail; if it
    // ever did, closing the connection is the safe answer.
    let line = message.encode().map_err(|_| OrbStuck)?;
    match timeout(WRITE_TIMEOUT, write.write_all(line.as_bytes())).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) | Err(_) => Err(OrbStuck),
    }
}

/// What one line from the orb turned out to be.
enum Incoming {
    Message(FromOrb),
    /// Too long, not UTF-8, not JSON, or not a known message: ignored.
    Rejected,
    Closed,
}

/// Reads the orb's lines, holding at most `MAX_LINE_BYTES` of any of them.
struct OrbLines {
    reader: BufReader<OwnedReadHalf>,
    line: Vec<u8>,
    /// The current line is over the limit; its bytes are skipped up to the newline.
    discarding: bool,
}

impl OrbLines {
    fn new(read: OwnedReadHalf) -> Self {
        Self {
            reader: BufReader::new(read),
            line: Vec::with_capacity(MAX_LINE_BYTES),
            discarding: false,
        }
    }

    /// The next line. Safe to drop at any `.await`: a partly read line stays in
    /// `self` and the next call carries on with it.
    async fn next(&mut self) -> io::Result<Incoming> {
        loop {
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                return Ok(Incoming::Closed);
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let content = newline.unwrap_or(available.len());
            if !self.discarding {
                if self.line.len() + content > MAX_LINE_BYTES {
                    self.line.clear();
                    self.discarding = true;
                } else {
                    self.line.extend_from_slice(&available[..content]);
                }
            }
            let consumed = newline.map_or(content, |index| index + 1);
            self.reader.consume(consumed);
            if newline.is_none() {
                continue;
            }
            if std::mem::take(&mut self.discarding) {
                return Ok(Incoming::Rejected);
            }
            let message = std::str::from_utf8(&self.line)
                .ok()
                .and_then(|line| FromOrb::decode(line).ok());
            self.line.clear();
            return Ok(message.map_or(Incoming::Rejected, Incoming::Message));
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/interfaces/orb/unix_tests.rs"]
mod tests;
