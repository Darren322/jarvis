//! The orb's side of the socket to Jarvis, with no window types.
//!
//! Jarvis is the server; the orb connects, reads [`ToOrb`] lines and sends
//! [`FromOrb::Tap`]. There is no async runtime in this crate, so one background
//! thread owns the socket. The render loop never touches the socket: it reads the
//! latest view and level from a small mutex-guarded cell and queues taps on a
//! bounded channel, so a frame never waits on I/O.
//!
//! Shutdown: dropping the [`Link`] drops the tap sender and joins the thread. The
//! thread sees the closed channel within one read timeout (connected) or at once
//! (waiting to reconnect), so the join is short. It is joined, not detached, so
//! Jarvis sees the connection close before the orb exits.
//!
//! No deadlock: the thread holds the display lock only to apply one message, never
//! across I/O; the render loop holds it only to copy two values; the tap channel
//! is bounded and sent with `try_send`, so neither side ever waits for the other.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use orb_protocol::{FromOrb, LevelSource, MAX_LINE_BYTES, OrbState, ToOrb};

use crate::look::View;

const SOCKET_VAR: &str = "JARVIS_ORB_SOCKET";
const RUNTIME_DIR_VAR: &str = "XDG_RUNTIME_DIR";
const SOCKET_FILE_NAME: &str = "jarvis-orb.sock";
/// Where the socket goes without a runtime directory, relative to the working directory.
const FALLBACK_SOCKET_PATH: &str = "data/jarvis-orb.sock";

/// How long the orb waits before it tries to reach Jarvis again.
pub const RECONNECT_INTERVAL: Duration = Duration::from_secs(3);
/// How long a read waits, so the thread can send taps and notice shutdown. It is
/// also the longest a tap waits before it is written.
pub const READ_POLL_INTERVAL: Duration = Duration::from_millis(20);
/// How long a write may block. Jarvis drops an orb that stops reading after 1 s.
pub const WRITE_TIMEOUT: Duration = Duration::from_millis(500);
/// Taps queued for the link thread. A tap past this is dropped.
const TAP_QUEUE_LEN: usize = 4;
/// Bytes read from the socket at a time.
const READ_CHUNK_BYTES: usize = 512;

/// The socket path, by the same rule as Jarvis's `config/orb.rs` (keep them in
/// step): `JARVIS_ORB_SOCKET` if not empty, resolved against the working
/// directory; else `$XDG_RUNTIME_DIR/jarvis-orb.sock` when that is absolute and
/// not empty; else `data/jarvis-orb.sock` under the working directory.
pub fn socket_path(
    socket: Option<OsString>,
    runtime_dir: Option<OsString>,
    current_dir: &Path,
) -> PathBuf {
    let set = |value: Option<OsString>| value.filter(|value| !value.is_empty());
    if let Some(socket) = set(socket) {
        current_dir.join(socket)
    } else if let Some(dir) = set(runtime_dir)
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
    {
        dir.join(SOCKET_FILE_NAME)
    } else {
        current_dir.join(FALLBACK_SOCKET_PATH)
    }
}

/// [`socket_path`] from the environment and the working directory.
pub fn socket_path_from_env() -> io::Result<PathBuf> {
    Ok(socket_path(
        std::env::var_os(SOCKET_VAR),
        std::env::var_os(RUNTIME_DIR_VAR),
        &std::env::current_dir()?,
    ))
}

/// What the link knows about Jarvis. Offline carries no state and no level, so
/// they cannot disagree with the connection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Display {
    Offline,
    Online { state: OrbState, level: f32 },
}

impl Display {
    /// The state shown from the moment of connecting until Jarvis says otherwise.
    pub const CONNECTED: Self = Self::Online {
        state: OrbState::Idle,
        level: 0.0,
    };

    /// Applies one message from Jarvis. A state change resets the level; a level
    /// counts only while listening (mic) or speaking (speech). Offline ignores
    /// everything.
    pub fn apply(&mut self, message: ToOrb) {
        let Self::Online { state, level } = self else {
            return;
        };
        match message {
            ToOrb::State { state: next } => {
                *state = next;
                *level = 0.0;
            }
            ToOrb::Level {
                level: next,
                source,
            } => {
                let matches = matches!(
                    (*state, source),
                    (OrbState::Listening, LevelSource::Mic)
                        | (OrbState::Speaking, LevelSource::Speech)
                );
                if matches {
                    *level = next.value();
                }
            }
        }
    }

    pub fn view(&self) -> View {
        match self {
            Self::Offline => View::Offline,
            Self::Online { state, .. } => View::State(*state),
        }
    }

    pub fn level(&self) -> f32 {
        match self {
            Self::Offline => 0.0,
            Self::Online { level, .. } => *level,
        }
    }

    pub fn is_online(&self) -> bool {
        matches!(self, Self::Online { .. })
    }
}

/// Cuts a byte stream into lines of at most [`MAX_LINE_BYTES`], holding at most
/// that many bytes. A longer line is discarded up to its newline; the next line
/// is read normally.
#[derive(Debug, Default)]
pub struct LineSplitter {
    line: Vec<u8>,
    /// The current line already went over the limit and is being skipped.
    skipping: bool,
}

impl LineSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the next bytes and returns the complete, in-limit lines they finish
    /// (without the newline). A line that is not UTF-8 is dropped.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        let mut rest = bytes;
        loop {
            let (part, found_newline) = match rest.iter().position(|byte| *byte == b'\n') {
                Some(end) => (&rest[..end], true),
                None => (rest, false),
            };
            if !self.skipping {
                if self.line.len() + part.len() > MAX_LINE_BYTES {
                    self.skipping = true;
                    self.line.clear();
                } else {
                    self.line.extend_from_slice(part);
                }
            }
            if !found_newline {
                return lines;
            }
            if !self.skipping
                && let Ok(line) = String::from_utf8(std::mem::take(&mut self.line))
            {
                lines.push(line);
            }
            self.line.clear();
            self.skipping = false;
            rest = &rest[part.len() + 1..];
        }
    }
}

/// Timings for the link thread. Production uses [`Timings::DEFAULT`]; tests use
/// short ones.
#[derive(Clone, Copy, Debug)]
pub struct Timings {
    pub reconnect: Duration,
    pub read_poll: Duration,
    pub write: Duration,
}

impl Timings {
    pub const DEFAULT: Self = Self {
        reconnect: RECONNECT_INTERVAL,
        read_poll: READ_POLL_INTERVAL,
        write: WRITE_TIMEOUT,
    };
}

type SharedDisplay = Arc<Mutex<Display>>;

fn lock(display: &SharedDisplay) -> MutexGuard<'_, Display> {
    // The cell holds plain values, so a poisoned lock is still consistent.
    display.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The orb's connection to Jarvis. Every method returns at once.
pub struct Link {
    display: SharedDisplay,
    /// `None` only while dropping.
    taps: Option<SyncSender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Link {
    pub fn start(path: PathBuf) -> Self {
        Self::start_with(path, Timings::DEFAULT)
    }

    pub fn start_with(path: PathBuf, timings: Timings) -> Self {
        let display = Arc::new(Mutex::new(Display::Offline));
        let (taps, queue) = mpsc::sync_channel(TAP_QUEUE_LEN);
        let thread = thread::spawn({
            let display = Arc::clone(&display);
            move || run(&path, timings, &display, &queue)
        });
        Self {
            display,
            taps: Some(taps),
            thread: Some(thread),
        }
    }

    pub fn view(&self) -> View {
        lock(&self.display).view()
    }

    /// The loudness to draw, 0 while offline or when the state has no level.
    pub fn level(&self) -> f32 {
        lock(&self.display).level()
    }

    /// Reports a tap to Jarvis. Dropped while offline or when the queue is full.
    pub fn tap(&self) {
        if !lock(&self.display).is_online() {
            return;
        }
        if let Some(taps) = &self.taps {
            // Full or closed: the tap is lost, which is the rule.
            let _: Result<(), TrySendError<()>> = taps.try_send(());
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        // Closing the channel is the stop signal; then wait for the thread so the
        // socket is closed before we return.
        self.taps = None;
        if let Some(thread) = self.thread.take() {
            // A panicked link thread has nothing left to clean up.
            let _ = thread.join();
        }
    }
}

/// Why a connection ended.
enum Ended {
    /// The peer closed or the socket failed: reconnect later.
    Lost,
    /// The `Link` was dropped.
    Stopped,
}

fn run(path: &Path, timings: Timings, display: &SharedDisplay, taps: &Receiver<()>) {
    loop {
        if let Ok(stream) = UnixStream::connect(path) {
            // Taps queued before this connection are stale.
            while taps.try_recv().is_ok() {}
            *lock(display) = Display::CONNECTED;
            let ended = serve(stream, timings, display, taps);
            *lock(display) = Display::Offline;
            // Taps queued while the connection was dying are stale too.
            while taps.try_recv().is_ok() {}
            if matches!(ended, Ended::Stopped) {
                return;
            }
        }
        if !wait_to_retry(timings.reconnect, taps) {
            return;
        }
    }
}

/// Waits out the retry interval, discarding taps. False when the `Link` was dropped.
fn wait_to_retry(interval: Duration, taps: &Receiver<()>) -> bool {
    let deadline = Instant::now() + interval;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return true;
        }
        match taps.recv_timeout(remaining) {
            Ok(()) => {}
            Err(RecvTimeoutError::Timeout) => return true,
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
}

fn serve(
    mut stream: UnixStream,
    timings: Timings,
    display: &SharedDisplay,
    taps: &Receiver<()>,
) -> Ended {
    if stream.set_read_timeout(Some(timings.read_poll)).is_err()
        || stream.set_write_timeout(Some(timings.write)).is_err()
    {
        return Ended::Lost;
    }
    let Ok(tap_line) = FromOrb::Tap {}.encode() else {
        return Ended::Lost;
    };
    let mut splitter = LineSplitter::new();
    let mut chunk = [0_u8; READ_CHUNK_BYTES];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => return Ended::Lost,
            Ok(read) => {
                for line in splitter.feed(&chunk[..read]) {
                    // A line that does not decode is ignored; the link stays up.
                    if let Ok(message) = ToOrb::decode(&line) {
                        lock(display).apply(message);
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return Ended::Lost,
        }
        loop {
            match taps.try_recv() {
                Ok(()) => {
                    if stream.write_all(tap_line.as_bytes()).is_err() {
                        return Ended::Lost;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ended::Stopped,
            }
        }
    }
}
