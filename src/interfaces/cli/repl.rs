use std::{future::Future, io, pin::Pin};

use tokio::io::AsyncBufRead;

use crate::assistant::Assistant;
use crate::conversation::ConversationSession;
use crate::interfaces::cli::commands::memory::{
    focus_from_displayed_ids, focus_from_successful_turn, forget_detail, forget_focus,
    forget_memory_id, import_memory, list_memories, print_memory_status, rebuild_memory_index,
    retry_memory_job,
};
use crate::interfaces::cli::commands::{self, Command, print_help};
use crate::interfaces::cli::diagnostics::{
    print_run_diagnostics, report_turn_warnings, safe_error_message,
};
use crate::interfaces::cli::input::{InputLine, InputReader, read_input_line};
use crate::interfaces::cli::presentation::AssistantPresenter;
use crate::memory::MemoryService;
use crate::storage::MemorySummary;

mod idle;
mod speech;
mod turn;

use idle::wait_for_input_or_idle_job;
use speech::{SpeechControl, SpeechWait, report_speech_cleanup, watch_speech};
use turn::stream_turn;

type InputFuture<'a> = Pin<Box<dyn Future<Output = io::Result<InputLine>> + 'a>>;

pub(crate) async fn run_input_loop<R, S, P>(
    assistant: &Assistant,
    session: &mut ConversationSession,
    memory: &mut Option<MemoryService>,
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
    if let Some(memory) = memory.as_mut()
        && let Err(error) = memory.interrupt_idle_work().await
    {
        eprintln!("Warning: local memory work did not settle during shutdown: {error}");
    }
    result
}

async fn run_input_loop_inner<R, S, P>(
    assistant: &Assistant,
    session: &mut ConversationSession,
    memory: &mut Option<MemoryService>,
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
    let mut idle_due = true;
    let mut recent_memories = Vec::<MemorySummary>::new();
    let mut current_memory_focus = None;

    loop {
        if !prompt_is_visible && next_input.is_none() {
            presenter.prompt()?;
        }
        let (line, memory_warning) = if let Some(line) = next_input.take() {
            (line, None)
        } else if idle_due {
            idle_due = false;
            wait_for_input_or_idle_job(&mut input, memory).await?
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
            InputLine::Prompt(raw) => match commands::parse(raw) {
                Command::Exit => return Ok(()),
                Command::Stop => println!("Stopped."),
                Command::Reset => {
                    if let Some(speech) = speech.as_mut() {
                        let cleanup_ok = report_speech_cleanup(speech.cancel().await);
                        if !cleanup_ok || speech.is_disabled() {
                            speech_available = false;
                        }
                    }
                    if let Some(error) = session.reset().await {
                        eprintln!("Warning: local archiving is unavailable: {error}");
                    }
                    current_memory_focus = None;
                    recent_memories.clear();
                    println!("Conversation context reset.");
                }
                Command::Help => print_help(),
                Command::ListMemories(query) => {
                    recent_memories = list_memories(memory, query).await;
                    current_memory_focus =
                        focus_from_displayed_ids(recent_memories.iter().map(|record| record.id));
                }
                Command::ForgetId(id) => {
                    forget_memory_id(
                        memory,
                        session,
                        &id,
                        &mut current_memory_focus,
                        &mut recent_memories,
                    )
                    .await;
                    idle_due = true;
                }
                Command::ForgetFocus => {
                    forget_focus(
                        memory,
                        session,
                        current_memory_focus,
                        &mut current_memory_focus,
                        &mut recent_memories,
                    )
                    .await;
                    idle_due = true;
                }
                Command::ForgetDetail(detail) => {
                    let displayed = recent_memories.clone();
                    forget_detail(
                        memory,
                        session,
                        &displayed,
                        &detail,
                        &mut current_memory_focus,
                        &mut recent_memories,
                    )
                    .await;
                    idle_due = true;
                }
                Command::Import(path) => {
                    import_memory(
                        memory,
                        session,
                        &path,
                        &mut current_memory_focus,
                        &mut recent_memories,
                    )
                    .await;
                    idle_due = true;
                }
                Command::MemoryStatus => {
                    print_memory_status(memory).await;
                    idle_due = true;
                }
                Command::Retry(id) => {
                    retry_memory_job(memory, &id).await;
                    idle_due = true;
                }
                Command::Rebuild => {
                    rebuild_memory_index(memory).await;
                    idle_due = true;
                }
                Command::Prompt(prompt) => {
                    let turn = stream_turn(assistant, session, memory, presenter, &prompt).await?;

                    let crate::conversation::ConversationTurn {
                        run,
                        context_warning,
                        archive_error,
                        memory_state,
                        current_source_id,
                    } = turn;
                    if run.is_ok() {
                        current_memory_focus = focus_from_successful_turn(current_source_id);
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
                            idle_due = true;
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
                            idle_due = true;
                        }
                    }
                }
            },
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/app_tests.rs"]
mod tests;
