use std::time::Instant;

use tokio::{io::BufReader, sync::oneshot};

use crate::interfaces::cli::input::InputReader;

use super::{SpeechControl, SpeechWait, watch_playback};

/// Speech double whose `speak` either signals playback start or fails first.
struct ScriptedSpeech {
    starts_playback: bool,
}

impl SpeechControl for ScriptedSpeech {
    type Error = std::io::Error;

    fn is_disabled(&self) -> bool {
        false
    }

    async fn speak(
        &mut self,
        _text: &str,
        playback_started: oneshot::Sender<()>,
    ) -> Result<(), Self::Error> {
        if !self.starts_playback {
            return Err(std::io::Error::other("injected synthesis failure"));
        }
        playback_started
            .send(())
            .map_err(|()| std::io::Error::other("watcher dropped the playback signal"))?;
        // Real playback lasts a while after it starts; yielding lets the
        // watcher observe the signal before this future completes.
        tokio::task::yield_now().await;
        Ok(())
    }

    async fn cancel(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

async fn watch(starts_playback: bool) -> Option<std::time::Duration> {
    let mut speech = ScriptedSpeech { starts_playback };
    // The writer stays open and silent so input never wins the race.
    let (_writer, reader) = tokio::io::duplex(64);
    let mut input = InputReader::new(BufReader::new(reader));
    let mut speech_available = true;
    let mut presenter = |_: &str| {};

    let (wait, first_audio) = watch_playback(
        &mut speech,
        "an answer",
        &mut input,
        &mut speech_available,
        &mut presenter,
        Instant::now(),
    )
    .await;
    assert!(matches!(wait, SpeechWait::Completed));
    first_audio
}

#[tokio::test]
async fn time_to_first_audio_is_measured_when_playback_starts() {
    assert!(watch(true).await.is_some());
}

#[tokio::test]
async fn time_to_first_audio_is_unavailable_when_playback_never_starts() {
    assert_eq!(watch(false).await, None);
}
