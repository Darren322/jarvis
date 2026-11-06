use std::{
    io,
    time::{Duration, Instant},
};

use tokio::{io::AsyncBufRead, sync::oneshot};

use crate::features::speech::{SpeechError, SpeechOutput};
use crate::interfaces::cli::diagnostics::{error_chain, format_first_audio};
use crate::interfaces::cli::input::{InputLine, InputReader, read_input_line};
use crate::interfaces::cli::presentation::AssistantPresenter;

use super::InputFuture;

pub(crate) trait SpeechControl {
    type Error: std::error::Error + 'static;

    fn is_disabled(&self) -> bool;
    async fn speak(
        &mut self,
        text: &str,
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
        text: &str,
        playback_started: oneshot::Sender<()>,
    ) -> Result<(), Self::Error> {
        SpeechOutput::speak(self, text, playback_started).await
    }

    async fn cancel(&mut self) -> Result<(), Self::Error> {
        SpeechOutput::cancel(self).await
    }

    async fn shutdown(&mut self) -> Result<(), Self::Error> {
        SpeechOutput::shutdown(self).await
    }
}

pub(super) enum SpeechWait {
    Completed,
    Stopped { cleanup_ok: bool },
    Input { line: InputLine, cleanup_ok: bool },
    InputError { error: io::Error },
}

/// Watches one spoken answer and prints how long playback took to start,
/// counted from `prompt_started`.
pub(super) async fn watch_speech<R, S, P>(
    speech: &mut S,
    text: &str,
    input: &mut InputReader<R>,
    speech_available: &mut bool,
    presenter: &mut P,
    prompt_started: Instant,
) -> SpeechWait
where
    R: AsyncBufRead + Unpin,
    S: SpeechControl,
    P: AssistantPresenter,
{
    let (wait, first_audio) = watch_playback(
        speech,
        text,
        input,
        speech_available,
        presenter,
        prompt_started,
    )
    .await;
    eprintln!("{}", format_first_audio(first_audio));
    wait
}

/// The playback-started signal is the only observation of first audio, so a
/// run where it never fires reports `None` rather than a guess.
async fn watch_playback<R, S, P>(
    speech: &mut S,
    text: &str,
    input: &mut InputReader<R>,
    speech_available: &mut bool,
    presenter: &mut P,
    prompt_started: Instant,
) -> (SpeechWait, Option<Duration>)
where
    R: AsyncBufRead + Unpin,
    S: SpeechControl,
    P: AssistantPresenter,
{
    let mut first_audio = None;
    if let Err(error) = presenter.prompt() {
        let _ = speech.cancel().await;
        return (SpeechWait::InputError { error }, first_audio);
    }
    let (playback_started_tx, mut playback_started_rx) = oneshot::channel();
    let mut speech_future = Box::pin(speech.speak(text, playback_started_tx));
    let mut input_future: InputFuture<'_> = Box::pin(read_input_line(input));
    let mut playback_started = true;

    loop {
        enum Event<E> {
            Speech(Result<(), E>),
            Input(io::Result<InputLine>),
            PlaybackStarted,
        }

        let event = tokio::select! {
            biased;
            line = input_future.as_mut() => Event::Input(line),
            _ = &mut playback_started_rx, if playback_started => {
                Event::PlaybackStarted
            }
            result = &mut speech_future => Event::Speech(result),
        };

        match event {
            Event::PlaybackStarted => {
                playback_started = false;
                first_audio = Some(prompt_started.elapsed());
            }
            Event::Speech(result) => {
                drop(speech_future);
                if let Err(error) = result {
                    eprintln!("Warning: local speech failed: {}", error_chain(&error));
                    if speech.is_disabled() {
                        *speech_available = false;
                    }
                }
                return (SpeechWait::Completed, first_audio);
            }
            Event::Input(Ok(InputLine::Rejected(reason))) => {
                presenter.input_received();
                eprintln!("Input rejected: {}", reason.message());
                drop(input_future);
                input_future = Box::pin(read_input_line(&mut *input));
                if let Err(error) = presenter.prompt() {
                    drop(speech_future);
                    report_speech_cleanup(speech.cancel().await);
                    return (SpeechWait::InputError { error }, first_audio);
                }
            }
            Event::Input(Ok(InputLine::Prompt(prompt))) => {
                presenter.input_received();
                if prompt.trim() == "/stop" {
                    println!("Stopping speech.");
                    drop(speech_future);
                    let cleanup_ok = report_speech_cleanup(speech.cancel().await);
                    return (SpeechWait::Stopped { cleanup_ok }, first_audio);
                }
                drop(speech_future);
                let cleanup_ok = report_speech_cleanup(speech.cancel().await);
                return (
                    SpeechWait::Input {
                        line: InputLine::Prompt(prompt),
                        cleanup_ok,
                    },
                    first_audio,
                );
            }
            Event::Input(Ok(InputLine::Eof)) => {
                presenter.input_received();
                drop(speech_future);
                let cleanup_ok = report_speech_cleanup(speech.cancel().await);
                return (
                    SpeechWait::Input {
                        line: InputLine::Eof,
                        cleanup_ok,
                    },
                    first_audio,
                );
            }
            Event::Input(Err(error)) => {
                drop(speech_future);
                report_speech_cleanup(speech.cancel().await);
                return (SpeechWait::InputError { error }, first_audio);
            }
        }
    }
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
