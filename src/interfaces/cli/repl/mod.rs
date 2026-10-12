use std::{future::Future, io, pin::Pin};

use tokio::io::AsyncBufRead;

use crate::features::{
    assistant::Assistant,
    control::{self, Command, MemoryControl},
    conversation::ConversationSession,
    memory::MemoryService,
};
use crate::interfaces::cli::diagnostics::{
    print_notices, print_run_diagnostics, report_turn_warnings, safe_error_message,
};
use crate::interfaces::cli::input::{InputLine, InputReader, read_input_line};
use crate::interfaces::cli::presentation::AssistantPresenter;

mod idle;
mod speech;
mod turn;

use idle::read_line_during_idle_work;
use speech::{SpeechControl, SpeechWait, report_speech_cleanup, watch_speech};
use turn::stream_turn;

type InputFuture<'a> = Pin<Box<dyn Future<Output = io::Result<InputLine>> + 'a>>;

pub(crate) async fn run_input_loop<R, S, P>(
    assistant: &Assistant,
    session: &mut ConversationSession,
    memory: Option<&MemoryService>,
    speech: &mut Option<S>,
    input: &mut R,
    presenter: &mut P,
) -> Result<(), Box<dyn std::error::Error>>
where
    R: AsyncBufRead + Unpin,
    S: SpeechControl,
    P: AssistantPresenter,
{
    let result = run_input_loop_inner(assistant, session, memory, speech, input, presenter).await;
    if let Some(speech) = speech.as_mut()
        && let Err(error) = speech.shutdown().await
    {
        eprintln!("Warning: local speech shutdown did not complete: {error}");
    }
    result
}

async fn run_input_loop_inner<R, S, P>(
    assistant: &Assistant,
    session: &mut ConversationSession,
    memory: Option<&MemoryService>,
    speech: &mut Option<S>,
    input: &mut R,
    presenter: &mut P,
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

    loop {
        if !prompt_is_visible && next_input.is_none() {
            presenter.prompt()?;
        }
        let (line, memory_warning) = if let Some(line) = next_input.take() {
            (line, None)
        } else if idle_work_due {
            idle_work_due = false;
            read_line_during_idle_work(&mut input, memory).await?
        } else {
            (read_input_line(&mut input).await?, None)
        };
        presenter.input_received();
        prompt_is_visible = false;
        if let Some(warning) = memory_warning {
            eprintln!("Warning: {warning}");
        }

        match line {
            InputLine::Eof => return Ok(()),
            InputLine::Rejected(reason) => eprintln!("Input rejected: {}", reason.message()),
            InputLine::Prompt(raw) => match control::parse(raw) {
                Command::Exit => return Ok(()),
                Command::Stop => println!("Stopped."),
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
                Command::Prompt(prompt) => {
                    let turn = stream_turn(assistant, session, memory, presenter, &prompt).await?;

                    let crate::features::conversation::ConversationTurn {
                        run,
                        context_warning,
                        archive_error,
                        memory_state,
                        current_source_id,
                    } = turn;
                    if run.is_ok() {
                        memory_control.focus_on_turn(current_source_id);
                    }
                    match run {
                        Ok(run) => {
                            let answer = run.response.output();
                            presenter.completed(answer)?;
                            prompt_is_visible = false;
                            report_turn_warnings(
                                context_warning.as_deref(),
                                archive_error.as_ref(),
                                memory_state,
                            );
                            print_run_diagnostics(&run.report);
                            if speech_available && let Some(speech) = speech.as_mut() {
                                match watch_speech(
                                    speech,
                                    answer,
                                    &mut input,
                                    &mut speech_available,
                                    presenter,
                                )
                                .await
                                {
                                    SpeechWait::Completed => prompt_is_visible = true,
                                    SpeechWait::Stopped { cleanup_ok } => {
                                        if cleanup_ok {
                                            println!("Speech stopped.");
                                        }
                                        if !cleanup_ok || speech.is_disabled() {
                                            speech_available = false;
                                        }
                                        prompt_is_visible = true;
                                    }
                                    SpeechWait::Input { line, cleanup_ok } => {
                                        if !cleanup_ok || speech.is_disabled() {
                                            speech_available = false;
                                        }
                                        next_input = Some(line);
                                        prompt_is_visible = true;
                                    }
                                    SpeechWait::InputError { error } => return Err(error.into()),
                                }
                            }
                            idle_work_due = true;
                        }
                        Err(error) => {
                            presenter.failed(safe_error_message(&error.error))?;
                            prompt_is_visible = false;
                            report_turn_warnings(
                                context_warning.as_deref(),
                                archive_error.as_ref(),
                                memory_state,
                            );
                            print_run_diagnostics(&error.report);
                            idle_work_due = true;
                        }
                    }
                }
            },
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/interfaces/cli/repl/repl_tests.rs"]
mod tests;
