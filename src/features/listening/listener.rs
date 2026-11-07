//! Record, then transcribe: the middle every way of starting to listen shares.
//!
//! Nothing here knows about the terminal. The caller decides when recording
//! stops and what to show; this only turns a stop signal into text.

use thiserror::Error;
use tokio::sync::oneshot;

use super::{CaptureError, Recorder};
use crate::clients::speech_to_text::{SpeechToText, SpeechToTextError, Transcript};

/// What a finished listen produced.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Heard {
    /// Trimmed, non-empty text.
    Text(String),
    /// Too little audio or no words in it: say "didn't catch that" and send nothing.
    DidntCatch,
}

#[derive(Debug, Error)]
pub(crate) enum ListenError {
    #[error("recording failed")]
    Capture(#[from] CaptureError),
    #[error("transcription failed")]
    Transcription(#[from] SpeechToTextError),
}

/// Owns the microphone recorder and the speech-to-text client.
pub(crate) struct Listener {
    recorder: Recorder,
    speech_to_text: SpeechToText,
}

impl Listener {
    pub(crate) fn new(recorder: Recorder, speech_to_text: SpeechToText) -> Self {
        Self {
            recorder,
            speech_to_text,
        }
    }

    /// Probes the speech-to-text server once.
    pub(crate) async fn health_check(&self) -> Result<(), SpeechToTextError> {
        self.speech_to_text.health_check().await
    }

    /// Records until `stop` completes (or the recorder's own limits end it), then
    /// transcribes. `recording_started` is sent when the first audio arrives, so
    /// the caller can tell the user to speak. `transcribing` is sent once recording
    /// is done and the request is about to start, so the caller can show that stage;
    /// it is dropped unsent when there is nothing to transcribe.
    ///
    /// Safe to drop at any `.await`: dropping during recording kills the recorder,
    /// and dropping during transcription only stops waiting for the server, which may
    /// still be working. The audio lives in memory for the length of this call.
    pub(crate) async fn listen(
        &self,
        stop: impl Future<Output = ()>,
        recording_started: oneshot::Sender<()>,
        transcribing: oneshot::Sender<()>,
    ) -> Result<Heard, ListenError> {
        let recording = match self.recorder.record(stop, recording_started).await {
            Ok(recording) => recording,
            Err(CaptureError::TooShort { .. }) => return Ok(Heard::DidntCatch),
            Err(error) => return Err(error.into()),
        };
        // A receiver that is gone just means nobody is showing the stage.
        let _ = transcribing.send(());
        Ok(
            match self.speech_to_text.transcribe(&recording.pcm).await? {
                Transcript::Text(text) => Heard::Text(text),
                Transcript::Nothing => Heard::DidntCatch,
            },
        )
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/listening/listener_tests.rs"]
mod tests;
