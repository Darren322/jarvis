//! `/listen` in the terminal: what the typed line does while the microphone is open.
//!
//! The shared record-and-transcribe work is `features::listening::Listener`. This
//! file adds only what is the terminal's own: Enter stops recording, `/stop`
//! cancels, any other line cancels and runs as the next input, and every stage is
//! shown through the presenter.

use std::io;

use tokio::{io::AsyncBufRead, sync::oneshot};

use crate::clients::speech_to_text::SpeechToTextError;
use crate::features::{
    control::{self, Command},
    listening::{Heard, ListenError, Listener},
    state::AssistantState,
};
use crate::interfaces::cli::diagnostics::error_chain;
use crate::interfaces::cli::input::{InputLine, InputReader, InputRejection, read_input_line};
use crate::interfaces::cli::presentation::AssistantPresenter;

/// Shown when `/listen` is used without the three speech-to-text settings.
pub(super) const NOT_CONFIGURED: &str = "Voice input is not configured. Set JARVIS_STT_BASE_URL, JARVIS_STT_MODEL and JARVIS_CAPTURE_DEVICE to enable /listen.";
const DIDNT_CATCH: &str = "Didn't catch that. Nothing was sent.";
const RECORDING_CANCELLED: &str = "Recording cancelled. Nothing was sent.";
const TRANSCRIPTION_CANCELLED: &str = "Transcription cancelled. The speech-to-text server may still be working on it. Nothing was sent.";
const TIMED_OUT: &str = "Speech-to-text timed out; the server may still be working.";

/// What the loop does after `/listen`.
pub(super) enum ListenEnd {
    /// Nothing more: back at the prompt.
    Done,
    /// The transcript, to be handled exactly like a typed line. The state is still
    /// `Transcribing`; whatever handles the line moves it on.
    Heard(InputLine),
    /// The user typed something else; handle it as the next input.
    Interrupted(InputLine),
}

/// Records one utterance and returns what the loop should do with it.
///
/// User input always wins (`biased;`). The listening future is dropped to cancel,
/// which kills the recorder or stops waiting for the server; both are drop-safe.
pub(super) async fn listen<R, P>(
    listener: &Listener,
    input: &mut InputReader<R>,
    presenter: &mut P,
) -> io::Result<ListenEnd>
where
    R: AsyncBufRead + Unpin,
    P: AssistantPresenter,
{
    presenter.state(AssistantState::Listening)?;
    let (stop_sender, stop_receiver) = oneshot::channel::<()>();
    let (transcribing_sender, mut transcribing_receiver) = oneshot::channel::<()>();
    // The sender stays in the `Option` until Enter is pressed: a dropped sender would
    // also complete the stop future.
    let mut stop_sender = Some(stop_sender);
    let mut transcribing_pending = true;
    let mut is_transcribing = false;
    let mut listening = Box::pin(listener.listen(
        async {
            let _ = stop_receiver.await;
        },
        transcribing_sender,
    ));

    let result = loop {
        tokio::select! {
            biased;
            line = read_input_line(&mut *input) => match line? {
                // Just Enter stops recording; once transcribing it changes nothing.
                InputLine::Rejected(InputRejection::Blank) => {
                    if let Some(sender) = stop_sender.take() {
                        let _ = sender.send(());
                    }
                }
                InputLine::Prompt(raw) if is_stop(&raw) => {
                    drop(listening);
                    let notice = if is_transcribing {
                        TRANSCRIPTION_CANCELLED
                    } else {
                        RECORDING_CANCELLED
                    };
                    presenter.notice(notice)?;
                    presenter.state(AssistantState::Idle)?;
                    return Ok(ListenEnd::Done);
                }
                other => {
                    drop(listening);
                    presenter.state(AssistantState::Idle)?;
                    return Ok(ListenEnd::Interrupted(other));
                }
            },
            sent = &mut transcribing_receiver, if transcribing_pending => {
                transcribing_pending = false;
                if sent.is_ok() {
                    is_transcribing = true;
                    presenter.state(AssistantState::Transcribing)?;
                }
            }
            result = &mut listening => break result,
        }
    };

    match result {
        Ok(Heard::Text(text)) => {
            presenter.notice(&format!("You (voice)> {text}"))?;
            return Ok(ListenEnd::Heard(InputLine::from_text(text)));
        }
        Ok(Heard::DidntCatch) => presenter.notice(DIDNT_CATCH)?,
        Err(error) => presenter.notice(&failure_message(&error))?,
    }
    presenter.state(AssistantState::Idle)?;
    Ok(ListenEnd::Done)
}

/// Whether a typed line is the `/stop` command, as `control::parse` decides.
fn is_stop(raw: &str) -> bool {
    matches!(control::parse(raw.to_owned()), Command::Stop)
}

fn failure_message(error: &ListenError) -> String {
    match error {
        ListenError::Capture(_) => format!(
            "Warning: could not record; check JARVIS_CAPTURE_DEVICE: {}",
            error_chain(error)
        ),
        ListenError::Transcription(SpeechToTextError::TimedOut(_)) => TIMED_OUT.to_owned(),
        ListenError::Transcription(_) => format!("Warning: {}", error_chain(error)),
    }
}
