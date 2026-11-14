//! The orb's side door into Jarvis: a Unix socket the `jarvis-orb` screen connects to.
//!
//! Jarvis is the server. It sends the assistant's state on every change and a
//! loudness about 30 times a second while listening or speaking, and it reads
//! taps. One JSON message per line (`orb-protocol`).
//!
//! This interface shares only `features/` with the terminal: it is handed a
//! [`StateFeed`] observer, two loudness readers and the sending half of a tap
//! queue, and knows nothing about who is on the other end of them. What a tap
//! means is decided by whoever reads that queue, from the real state
//! (`features::control::tap_action`); here a tap is only passed on.
//!
//! The orb is optional. No orb connected, an orb that disconnects, and an orb
//! that sends rubbish are all normal and silent: orb input is untrusted, and the
//! only thing it can cause is one `Tap` in a bounded queue.

use std::{
    convert::Infallible,
    fs, io,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

use futures_util::{StreamExt, stream::FuturesUnordered};
use orb_protocol::{FromOrb, Level, LevelSource, MAX_LINE_BYTES, OrbState, ToOrb};
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{
        UnixListener, UnixStream,
        unix::{OwnedReadHalf, OwnedWriteHalf},
    },
    sync::{mpsc, watch},
    time::{MissedTickBehavior, timeout},
};

use crate::features::{
    control::Tap,
    listening::LEVEL_INTERVAL,
    speech::LevelReceiver,
    state::{AssistantState, StateFeed, StateObserver},
};

/// Orbs served at once. One screen is expected; the spare slots let a restarted
/// orb connect before its old connection is noticed as gone.
const MAX_CONNECTIONS: usize = 4;
/// How long one message may take to write before the orb counts as stuck and its
/// connection is closed. The orb reconnects by itself.
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);
/// Pause after a failed `accept` (for example, out of file descriptors), so a
/// persistent failure cannot spin.
const ACCEPT_RETRY_DELAY: Duration = Duration::from_secs(1);
/// Taps waiting to be handled. A tap is a request to act now, so a second one
/// arriving before the first is handled adds nothing and is dropped.
pub(crate) const TAP_QUEUE: usize = 1;
/// Only the user may connect: read and write for the owner, nothing for others.
const SOCKET_MODE: u32 = 0o600;

#[derive(Debug, Error)]
pub(crate) enum OrbError {
    #[error("{} exists and is not a socket", .0.display())]
    NotASocket(PathBuf),
    #[error("another program is already serving {}", .0.display())]
    InUse(PathBuf),
    #[error("could not remove the stale socket {}", path.display())]
    RemoveStale {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not open the socket {}", path.display())]
    Bind {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not restrict the socket {} to its owner", path.display())]
    Restrict {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// What the server shows the orb and where its taps go.
pub(crate) struct OrbFeeds<'a> {
    pub(crate) states: &'a StateFeed,
    /// Microphone loudness, 0 to 1; sent while listening.
    pub(crate) mic_level: watch::Receiver<f32>,
    /// Loudness of Jarvis's voice; `None` when speech is not configured.
    pub(crate) speech_level: Option<LevelReceiver>,
    pub(crate) taps: mpsc::Sender<Tap>,
}

/// The bound socket. Dropping it (or the `serve` future) closes every
/// connection and removes the socket file.
pub(crate) struct OrbServer {
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

impl OrbServer {
    /// Opens the socket at `path`, for the owner only.
    ///
    /// A socket file left by a Jarvis that did not shut down cleanly is removed
    /// first. A live one (something accepts connections on it) is left alone and
    /// reported, and so is any file that is not a socket: this never deletes
    /// something it did not create. Must be called inside the Tokio runtime.
    pub(crate) fn bind(path: &Path) -> Result<Self, OrbError> {
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
    /// at any `.await` closes them all and loses nothing: the server holds no
    /// state of its own.
    pub(crate) async fn serve(self, feeds: OrbFeeds<'_>) -> Infallible {
        let mut connections = FuturesUnordered::new();
        loop {
            tokio::select! {
                accepted = self.listener.accept() => match accepted {
                    // Over the limit the stream is dropped, which closes it.
                    Ok((stream, _)) if connections.len() < MAX_CONNECTIONS => {
                        connections.push(serve_orb(stream, &feeds));
                    }
                    Ok(_) => {}
                    Err(_) => tokio::time::sleep(ACCEPT_RETRY_DELAY).await,
                },
                Some(()) = connections.next(), if !connections.is_empty() => {}
            }
        }
    }
}

fn orb_state(state: AssistantState) -> OrbState {
    match state {
        AssistantState::Idle => OrbState::Idle,
        AssistantState::Listening => OrbState::Listening,
        AssistantState::Transcribing => OrbState::Transcribing,
        AssistantState::Thinking => OrbState::Thinking,
        AssistantState::Speaking => OrbState::Speaking,
    }
}

/// Whose loudness the orb should see in `state`, if anyone's.
fn level_source(state: AssistantState) -> Option<LevelSource> {
    match state {
        AssistantState::Listening => Some(LevelSource::Mic),
        AssistantState::Speaking => Some(LevelSource::Speech),
        AssistantState::Idle | AssistantState::Transcribing | AssistantState::Thinking => None,
    }
}

/// One orb, from its first byte to its last. Returns when the orb goes away or
/// stops reading.
async fn serve_orb(stream: UnixStream, feeds: &OrbFeeds<'_>) {
    let (read, mut write) = stream.into_split();
    let mut lines = OrbLines::new(read);
    let mut states: StateObserver = feeds.states.observe();
    let mut ticks = tokio::time::interval(LEVEL_INTERVAL);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let state = orb_state(states.current());
    if send(&mut write, ToOrb::State { state }).await.is_err() {
        return;
    }
    loop {
        let message = tokio::select! {
            // The user's tap first, then the state it may depend on, then loudness.
            biased;
            line = lines.next() => match line {
                Ok(Incoming::Message(FromOrb::Tap {})) => {
                    // Full means a tap is already waiting; closed means nobody acts
                    // on taps. Neither is the orb's problem.
                    let _ = feeds.taps.try_send(Tap);
                    continue;
                }
                Ok(Incoming::Rejected) => continue,
                Ok(Incoming::Closed) | Err(_) => return,
            },
            state = states.next() => ToOrb::State { state: orb_state(state) },
            _ = ticks.tick() => {
                let Some(source) = level_source(states.current()) else {
                    continue;
                };
                let level = match source {
                    LevelSource::Mic => *feeds.mic_level.borrow(),
                    LevelSource::Speech => feeds
                        .speech_level
                        .as_ref()
                        .map_or(0.0, LevelReceiver::level),
                };
                ToOrb::Level { level: Level::clamped(level), source }
            }
        };
        if send(&mut write, message).await.is_err() {
            return;
        }
    }
}

/// The orb did not take a message: it is gone, or stuck.
struct OrbGone;

async fn send(write: &mut OwnedWriteHalf, message: ToOrb) -> Result<(), OrbGone> {
    // Every `ToOrb` is far under the line limit, so encoding cannot fail; if it
    // ever did, closing the connection is the safe answer.
    let line = message.encode().map_err(|_| OrbGone)?;
    match timeout(WRITE_TIMEOUT, write.write_all(line.as_bytes())).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) | Err(_) => Err(OrbGone),
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
#[path = "../../../tests/unit/interfaces/orb/orb_tests.rs"]
mod tests;
