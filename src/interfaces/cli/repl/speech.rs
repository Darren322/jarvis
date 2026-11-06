use std::{
    future::Future,
    io, mem,
    pin::Pin,
    time::{Duration, Instant},
};

use thiserror::Error;
use tokio::sync::{
    mpsc::{self, error::TrySendError},
    oneshot,
};

use crate::features::speech::{SpeechChunker, SpeechError, SpeechOutput};
use crate::interfaces::cli::diagnostics::error_chain;
use crate::interfaces::cli::input::InputLine;

/// Finished chunks that may wait for the speech side while it synthesizes and plays.
///
/// The text side never waits for room (see [`ChunkFeed`]). A full answer is at most
/// 1,024 tokens, so this covers any ordinary answer, including a long story told in
/// short sentences; only a degenerate run of very short sentences can overflow it.
pub(super) const SPEECH_CHUNK_QUEUE: usize = 64;

pub(crate) trait SpeechControl {
    type Error: std::error::Error + 'static;

    fn is_disabled(&self) -> bool;
    /// Speaks one answer from `chunks`, which closes when the text is complete.
    async fn speak(
        &mut self,
        chunks: mpsc::Receiver<String>,
        playback_started: oneshot::Sender<()>,
    ) -> Result<(), Self::Error>;
    async fn cancel(&mut self) -> Result<(), Self::Error>;
    async fn shutdown(&mut self) -> Result<(), Self::Error>;
}

impl SpeechControl for SpeechOutput {
    type Error = SpeechError;

    fn is_disabled(&self) -> bool {
        SpeechOutput::is_disabled(self)
    }

    async fn speak(
        &mut self,
        chunks: mpsc::Receiver<String>,
        playback_started: oneshot::Sender<()>,
    ) -> Result<(), Self::Error> {
        SpeechOutput::speak(self, chunks, playback_started).await
    }

    async fn cancel(&mut self) -> Result<(), Self::Error> {
        SpeechOutput::cancel(self).await
    }

    async fn shutdown(&mut self) -> Result<(), Self::Error> {
        SpeechOutput::shutdown(self).await
    }
}

/// The speech queue was full, so the rest of the answer is not spoken.
#[derive(Debug, Error, PartialEq, Eq)]
#[error(
    "speech fell more than {SPEECH_CHUNK_QUEUE} chunks behind the text; the rest of this answer is not spoken"
)]
pub(super) struct SpeechQueueFull;

/// Splits the streamed answer into chunks and queues them for the speech side.
///
/// It never waits for queue room: the model stream and the terminal must not stall
/// behind speech, and the model run has a wall-clock deadline that a stall would eat.
/// The queue is bounded instead of backpressured: when it is full, the chunk is lost,
/// feeding stops, and the speech side finishes what it already has.
pub(super) struct ChunkFeed {
    chunker: SpeechChunker,
    /// `None` once the text is complete, speech has gone away or the queue overflowed.
    sender: Option<mpsc::Sender<String>>,
}

impl ChunkFeed {
    pub(super) fn new(sender: mpsc::Sender<String>) -> Self {
        Self {
            chunker: SpeechChunker::new(),
            sender: Some(sender),
        }
    }

    /// Hands streamed text to speech. An overflow is reported once; speech then
    /// finishes what it already has.
    pub(super) fn feed_text(&mut self, delta: &str) {
        if let Err(error) = self.push(delta) {
            eprintln!("Warning: local speech: {error}");
        }
    }

    /// Tells speech the text is complete.
    pub(super) fn finish_text(&mut self) {
        if let Err(error) = self.finish() {
            eprintln!("Warning: local speech: {error}");
        }
    }

    /// Adds streamed text and queues every chunk it completes.
    pub(super) fn push(&mut self, delta: &str) -> Result<(), SpeechQueueFull> {
        for chunk in self.chunker.push(delta) {
            self.queue(chunk)?;
        }
        Ok(())
    }

    /// Queues the unfinished rest of the text and closes the queue: the text is complete.
    pub(super) fn finish(&mut self) -> Result<(), SpeechQueueFull> {
        let rest = mem::take(&mut self.chunker).finish();
        let result = match rest {
            Some(chunk) => self.queue(chunk),
            None => Ok(()),
        };
        self.sender = None;
        result
    }

    fn queue(&mut self, chunk: String) -> Result<(), SpeechQueueFull> {
        let Some(sender) = self.sender.as_ref() else {
            return Ok(());
        };
        match sender.try_send(chunk) {
            Ok(()) => Ok(()),
            // The speech future ended; its result is reported through its own branch.
            Err(TrySendError::Closed(_)) => {
                self.sender = None;
                Ok(())
            }
            Err(TrySendError::Full(_)) => {
                self.sender = None;
                Err(SpeechQueueFull)
            }
        }
    }
}

type SpeakFuture<'a, E> = Pin<Box<dyn Future<Output = Result<(), E>> + 'a>>;

/// The speech side of one answer, started before the model's first token.
///
/// It holds the `speak` future instead of the speech backend, so the caller keeps
/// `&mut S` and can run `cancel` once this value is dropped.
pub(super) struct SpokenAnswer<'a, E> {
    pub(super) speaking: SpeakFuture<'a, E>,
    pub(super) feed: ChunkFeed,
    /// `None` once the playback-started signal was observed or its sender dropped.
    pub(super) started: Option<oneshot::Receiver<()>>,
    pub(super) first_audio: Option<Duration>,
}

impl<'a, E> SpokenAnswer<'a, E> {
    pub(super) fn start<S>(speech: &'a mut S) -> Self
    where
        S: SpeechControl<Error = E>,
    {
        let (chunks, chunk_receiver) = mpsc::channel(SPEECH_CHUNK_QUEUE);
        let (started_sender, started) = oneshot::channel();
        Self {
            speaking: Box::pin(speech.speak(chunk_receiver, started_sender)),
            feed: ChunkFeed::new(chunks),
            started: Some(started),
            first_audio: None,
        }
    }

    /// Records the time to first audio when playback starts.
    pub(super) fn note_playback_started(&mut self, signalled: bool, prompt_started: Instant) {
        self.started = None;
        if signalled {
            self.first_audio = Some(prompt_started.elapsed());
        }
    }
}

/// Resolves when the speech side signals that playback started, and is pending when
/// there is nothing left to observe. `true` means the signal was sent rather than dropped.
pub(super) async fn playback_signal(started: &mut Option<oneshot::Receiver<()>>) -> bool {
    match started {
        Some(started) => started.await.is_ok(),
        None => std::future::pending().await,
    }
}

/// Why the speech side of an answer ended.
pub(super) enum SpeechExit<E> {
    /// The speech future completed on its own.
    Finished(Result<(), E>),
    Stopped,
    /// A new prompt (or end of input) arrived while speech was active.
    Input(InputLine),
    /// Writing to the terminal or reading input failed.
    Io(io::Error),
    /// The model run failed, so the rest of the answer is not spoken.
    ModelFailed,
}

pub(super) enum SpeechWait {
    Completed,
    Stopped {
        cleanup_ok: bool,
    },
    Input {
        line: InputLine,
        cleanup_ok: bool,
    },
    /// The answer failed mid-stream; speech was cancelled without a notice.
    Aborted {
        cleanup_ok: bool,
    },
}

/// How an answer's speech ended and when its first audio started, counted from the
/// prompt being accepted. The playback-started signal is the only observation of first
/// audio, so a run where it never fired reports `None` rather than a guess.
pub(super) struct SpokenOutcome {
    pub(super) wait: SpeechWait,
    pub(super) first_audio: Option<Duration>,
}

/// Ends the answer's speech. The `speak` future must already be dropped.
///
/// Anything but a completed future needs `cancel`; a cancel is never a failure. A
/// terminal or input failure is returned after the cancel.
pub(super) async fn settle_speech<S: SpeechControl>(
    speech: &mut S,
    exit: SpeechExit<S::Error>,
) -> io::Result<SpeechWait> {
    Ok(match exit {
        SpeechExit::Finished(result) => {
            if let Err(error) = result {
                eprintln!("Warning: local speech failed: {}", error_chain(&error));
            }
            SpeechWait::Completed
        }
        SpeechExit::Stopped => {
            println!("Stopping speech.");
            let cleanup_ok = report_speech_cleanup(speech.cancel().await);
            SpeechWait::Stopped { cleanup_ok }
        }
        SpeechExit::Input(line) => {
            let cleanup_ok = report_speech_cleanup(speech.cancel().await);
            SpeechWait::Input { line, cleanup_ok }
        }
        SpeechExit::Io(error) => {
            report_speech_cleanup(speech.cancel().await);
            return Err(error);
        }
        SpeechExit::ModelFailed => {
            let cleanup_ok = report_speech_cleanup(speech.cancel().await);
            SpeechWait::Aborted { cleanup_ok }
        }
    })
}

pub(super) fn report_speech_cleanup<E: std::error::Error + 'static>(result: Result<(), E>) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "Warning: local speech cleanup did not complete: {}",
                error_chain(&error)
            );
            false
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/interfaces/cli/repl/speech_tests.rs"]
mod tests;
