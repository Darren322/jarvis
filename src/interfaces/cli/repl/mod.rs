use std::{future::Future, io, mem, pin::Pin};

use tokio::io::AsyncBufRead;

use crate::features::{
    assistant::Assistant,
    control::{self, Command, MemoryControl, TapAction, tap_action},
    conversation::ConversationSession,
    listening::Listener,
    memory::{MemoryService, RecallMode, wants_memory_search},
    state::AssistantState,
};
use crate::interfaces::cli::diagnostics::{error_chain, format_first_audio, print_notices};
use crate::interfaces::cli::input::{InputLine, InputReader, Taps, read_input_line};
use crate::interfaces::cli::presentation::AssistantPresenter;

mod idle;
mod listen;
mod speech;
mod turn;

use idle::read_line_during_idle_work;
use listen::{ListenEnd, ListenStop, NOT_CONFIGURED, listen};
use speech::{SpeechControl, SpeechWait, SpokenOutcome, report_speech_cleanup};
use turn::{PromptRequest, run_prompt};

type InputFuture<'a> = Pin<Box<dyn Future<Output = io::Result<InputLine>> + 'a>>;

/// Runs the REPL until the input ends or the user exits.
///
/// `taps` is a second input source beside the typed line (see [`Taps`]); pass
/// `Taps::none()` for a keyboard-only session.
#[expect(
    clippy::too_many_arguments,
    reason = "the composition root hands over each service once"
)]
pub(crate) async fn run_input_loop<R, S, P>(
    assistant: &Assistant,
    session: &mut ConversationSession,
    memory: Option<&MemoryService>,
    speech: &mut Option<S>,
    input: &mut R,
    taps: Taps,
    presenter: &mut P,
    listener: Option<&Listener>,
) -> Result<(), Box<dyn std::error::Error>>
where
    R: AsyncBufRead + Unpin,
    S: SpeechControl,
    P: AssistantPresenter,
{
    let result = run_input_loop_inner(
        assistant, session, memory, speech, input, taps, presenter, listener,
    )
    .await;
    if let Some(speech) = speech.as_mut()
        && let Err(error) = speech.shutdown().await
    {
        eprintln!(
            "Warning: local speech shutdown did not complete: {}",
            error_chain(&error)
        );
    }
    result
}

#[expect(
    clippy::too_many_arguments,
    reason = "the composition root hands over each service once"
)]
async fn run_input_loop_inner<R, S, P>(
    assistant: &Assistant,
    session: &mut ConversationSession,
    memory: Option<&MemoryService>,
    speech: &mut Option<S>,
    input: &mut R,
    mut taps: Taps,
    presenter: &mut P,
    listener: Option<&Listener>,
) -> Result<(), Box<dyn std::error::Error>>
where
    R: AsyncBufRead + Unpin,
    S: SpeechControl,
    P: AssistantPresenter,
{
    let mut input = InputReader::new(input);
    let mut speech_available = speech.as_ref().is_some_and(|speech| !speech.is_disabled());
    let mut prompt_is_visible = false;
    let mut next_input = None;
    let mut idle_work_due = true;
    let mut memory_control = MemoryControl::default();
    // A line was handed over (a transcript, or a line typed over speech) and the state is
    // not yet `Idle`; whatever handles the line moves it on.
    let mut state_awaiting_idle = false;
    // A tap ended an answer or its speech; the loop listens instead of reading a line.
    let mut listen_due = false;

    loop {
        if !prompt_is_visible && next_input.is_none() && !listen_due {
            if mem::take(&mut state_awaiting_idle) {
                // The handed-over line was a command that reports no state of its own.
                presenter.state(AssistantState::Idle)?;
            }
            presenter.prompt()?;
        }
        let wake = if let Some(line) = next_input.take() {
            Wake::Line(line, None)
        } else if mem::take(&mut listen_due) {
            Wake::Tap {
                prompt_on_screen: prompt_is_visible,
            }
        } else {
            let with_idle_work = mem::take(&mut idle_work_due);
            let read = async {
                if with_idle_work {
                    read_line_during_idle_work(&mut input, memory).await
                } else {
                    read_input_line(&mut input).await.map(|line| (line, None))
                }
            };
            // Typed input wins over a tap. Both losers are safe to drop: a half-read
            // line stays in `input`, and idle work commits each vector on its own.
            tokio::select! {
                biased;
                read = read => {
                    let (line, memory_warning) = read?;
                    Wake::Line(line, memory_warning)
                }
                _tap = taps.next() => match tap_action(AssistantState::Idle) {
                    TapAction::Listen => Wake::Tap { prompt_on_screen: true },
                    // Only `Listen` belongs to the idle state.
                    TapAction::SendNow
                    | TapAction::Ignore
                    | TapAction::DropAnswerThenListen
                    | TapAction::StopSpeakingThenListen => continue,
                },
            }
        };
        let (line, memory_warning) = match wake {
            Wake::Line(line, memory_warning) => (line, memory_warning),
            Wake::Tap { prompt_on_screen } => {
                if prompt_on_screen {
                    // End the open prompt line, as Enter does for a typed line.
                    println!();
                }
                presenter.input_received();
                prompt_is_visible = false;
                match listener {
                    None => {
                        presenter.notice(NOT_CONFIGURED)?;
                        // A tap that ended an answer left its state showing.
                        state_awaiting_idle = true;
                    }
                    Some(listener) => {
                        listen_once(
                            listener,
                            &mut input,
                            &mut taps,
                            presenter,
                            ListenStop::OnSilence,
                            &mut state_awaiting_idle,
                            &mut next_input,
                        )
                        .await?;
                    }
                }
                idle_work_due = true;
                continue;
            }
        };
        presenter.input_received();
        prompt_is_visible = false;
        if let Some(warning) = memory_warning {
            eprintln!("Warning: {warning}");
        }

        match line {
            InputLine::Eof => return Ok(()),
            InputLine::Rejected(reason) => eprintln!("Input rejected: {}", reason.message()),
            InputLine::Prompt(raw) => {
                let command = control::parse(raw);
                let recall = recall_mode(&command);
                match command {
                    Command::Exit => return Ok(()),
                    Command::Stop => println!("Stopped."),
                    Command::Listen => {
                        match listener {
                            None => presenter.notice(NOT_CONFIGURED)?,
                            Some(listener) => {
                                listen_once(
                                    listener,
                                    &mut input,
                                    &mut taps,
                                    presenter,
                                    ListenStop::OnRequest,
                                    &mut state_awaiting_idle,
                                    &mut next_input,
                                )
                                .await?;
                            }
                        }
                        idle_work_due = true;
                    }
                    Command::Help => println!("{}", control::help_text()),
                    Command::Usage(hint) => println!("{hint}"),
                    Command::Reset => {
                        if let Some(speech) = speech.as_mut() {
                            let cleanup_ok = report_speech_cleanup(speech.cancel().await);
                            if !cleanup_ok || speech.is_disabled() {
                                speech_available = false;
                            }
                        }
                        if let Err(error) = session.reset().await {
                            eprintln!("Warning: local archiving is unavailable: {error}");
                        }
                        memory_control.clear();
                        println!("Conversation context reset.");
                    }
                    Command::Memory(command) => {
                        print_notices(&memory_control.run(command, memory, session).await);
                        idle_work_due = true;
                    }
                    Command::Prompt(prompt) | Command::Recall(prompt) => {
                        // `run_prompt` reports its own states, starting with `Thinking`.
                        state_awaiting_idle = false;
                        let speaking = if speech_available {
                            speech.as_mut()
                        } else {
                            None
                        };
                        let end = run_prompt(
                            assistant,
                            session,
                            memory,
                            presenter,
                            &mut input,
                            &mut taps,
                            speaking,
                            PromptRequest {
                                text: &prompt,
                                recall,
                            },
                        )
                        .await?;
                        if end.succeeded {
                            memory_control.focus_on_turn(end.current_source_id);
                        }
                        prompt_is_visible = false;
                        listen_due = end.listen_next;
                        if let Some(SpokenOutcome { wait, first_audio }) = end.speech {
                            presenter.notice(&format_first_audio(first_audio))?;
                            let speech_disabled = speech.as_ref().is_some_and(S::is_disabled);
                            match wait {
                                SpeechWait::Completed => {
                                    if speech_disabled {
                                        speech_available = false;
                                    }
                                    prompt_is_visible = true;
                                }
                                SpeechWait::Stopped { cleanup_ok } => {
                                    if cleanup_ok {
                                        println!("Speech stopped.");
                                    }
                                    if !cleanup_ok || speech_disabled {
                                        speech_available = false;
                                    }
                                    prompt_is_visible = true;
                                }
                                SpeechWait::Input { line, cleanup_ok } => {
                                    if !cleanup_ok || speech_disabled {
                                        speech_available = false;
                                    }
                                    state_awaiting_idle = true;
                                    next_input = Some(line);
                                    prompt_is_visible = true;
                                }
                                SpeechWait::Aborted { cleanup_ok } => {
                                    if !cleanup_ok || speech_disabled {
                                        speech_available = false;
                                    }
                                }
                                SpeechWait::Tapped { cleanup_ok } => {
                                    if !cleanup_ok || speech_disabled {
                                        speech_available = false;
                                    }
                                    prompt_is_visible = end.prompt_shown;
                                }
                            }
                        }
                        idle_work_due = true;
                    }
                }
            }
        }
    }
}

/// Whether the run for `command` searches memory: `/recall` always does, and a
/// plain prompt does only when it asks (see [`wants_memory_search`]).
fn recall_mode(command: &Command) -> RecallMode {
    match command {
        Command::Recall(_) => RecallMode::Search,
        Command::Prompt(prompt) if wants_memory_search(prompt) => RecallMode::Search,
        Command::Prompt(_)
        | Command::Exit
        | Command::Stop
        | Command::Listen
        | Command::Reset
        | Command::Help
        | Command::Memory(_)
        | Command::Usage(_) => RecallMode::Skip,
    }
}

/// What woke the loop: a line to handle, or a tap that starts listening.
enum Wake {
    Line(InputLine, Option<String>),
    Tap {
        /// Whether an open prompt line is on screen and must be ended first.
        prompt_on_screen: bool,
    },
}

/// Listens once and queues what the loop handles next.
async fn listen_once<R, P>(
    listener: &Listener,
    input: &mut InputReader<R>,
    taps: &mut Taps,
    presenter: &mut P,
    stop: ListenStop,
    state_awaiting_idle: &mut bool,
    next_input: &mut Option<InputLine>,
) -> io::Result<()>
where
    R: AsyncBufRead + Unpin,
    P: AssistantPresenter,
{
    // `listen` reports its own states, starting with `Listening`.
    *state_awaiting_idle = false;
    match listen(listener, input, taps, presenter, stop).await? {
        ListenEnd::Done => {}
        ListenEnd::Heard(line) => {
            *state_awaiting_idle = true;
            *next_input = Some(line);
        }
        ListenEnd::Interrupted(line) => *next_input = Some(line),
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../../tests/unit/interfaces/cli/repl/repl_tests.rs"]
mod tests;
