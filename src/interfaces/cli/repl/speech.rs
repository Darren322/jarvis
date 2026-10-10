use std::io;

use tokio::{io::AsyncBufRead, sync::oneshot};

use crate::interfaces::cli::input::{InputLine, InputReader, read_input_line};
use crate::interfaces::cli::presentation::AssistantPresenter;
use crate::speech::{SpeechError, SpeechOutput};

use super::InputFuture;

pub(crate) trait SpeechControl {
    type Error: std::fmt::Display;

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

pub(super) async fn watch_speech<R, S, P>(
    speech: &mut S,
    text: &str,
    input: &mut InputReader<R>,
    speech_available: &mut bool,
    presenter: &mut P,
) -> SpeechWait
where
    R: AsyncBufRead + Unpin,
    S: SpeechControl,
    P: AssistantPresenter,
{
    if let Err(error) = presenter.prompt() {
        let _ = speech.cancel().await;
        return SpeechWait::InputError { error };
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
            Event::PlaybackStarted => playback_started = false,
            Event::Speech(result) => {
                drop(speech_future);
                if let Err(error) = result {
                    eprintln!("Warning: local speech failed: {error}");
                    if speech.is_disabled() {
                        *speech_available = false;
                    }
                }
                return SpeechWait::Completed;
            }
            Event::Input(Ok(InputLine::Rejected(reason))) => {
                presenter.input_received();
                eprintln!("Input rejected: {}", reason.message());
                drop(input_future);
                input_future = Box::pin(read_input_line(&mut *input));
                if let Err(error) = presenter.prompt() {
                    drop(speech_future);
                    report_speech_cleanup(speech.cancel().await);
                    return SpeechWait::InputError { error };
                }
            }
            Event::Input(Ok(InputLine::Prompt(prompt))) => {
                presenter.input_received();
                if prompt.trim() == "/stop" {
                    println!("Stopping speech.");
                    drop(speech_future);
                    let cleanup_ok = report_speech_cleanup(speech.cancel().await);
                    return SpeechWait::Stopped { cleanup_ok };
                }
                drop(speech_future);
                let cleanup_ok = report_speech_cleanup(speech.cancel().await);
                return SpeechWait::Input {
                    line: InputLine::Prompt(prompt),
                    cleanup_ok,
                };
            }
            Event::Input(Ok(InputLine::Eof)) => {
                presenter.input_received();
                drop(speech_future);
                let cleanup_ok = report_speech_cleanup(speech.cancel().await);
                return SpeechWait::Input {
                    line: InputLine::Eof,
                    cleanup_ok,
                };
            }
            Event::Input(Err(error)) => {
                drop(speech_future);
                report_speech_cleanup(speech.cancel().await);
                return SpeechWait::InputError { error };
            }
        }
    }
}

pub(super) fn report_speech_cleanup<E: std::fmt::Display>(result: Result<(), E>) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => {
            eprintln!("Warning: local speech cleanup did not complete: {error}");
            false
        }
    }
}
