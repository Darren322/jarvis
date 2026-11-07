use std::{future::Future, io, pin::Pin, time::Instant};

use tokio::{io::AsyncBufRead, sync::mpsc};

use crate::features::{
    assistant::{Assistant, AssistantTextDelta},
    conversation::{ConversationSession, ConversationTurn},
    memory::MemoryService,
    state::AssistantState,
};
use crate::interfaces::cli::diagnostics::{
    print_run_diagnostics, report_turn_warnings, safe_error_message,
};
use crate::interfaces::cli::input::{InputLine, InputReader, read_input_line};
use crate::interfaces::cli::presentation::AssistantPresenter;
use crate::storage::SourceId;

use super::InputFuture;
use super::speech::{
    ChunkFeed, SpeechControl, SpeechExit, SpeechWait, SpokenAnswer, SpokenOutcome, playback_signal,
    settle_speech,
};

/// Text deltas the model run may have buffered ahead of the terminal.
const TEXT_DELTA_QUEUE: usize = 32;

/// What the loop needs to know about a finished prompt.
pub(super) struct PromptEnd {
    /// Whether the model run succeeded, so the turn can become the "forget that" focus.
    pub(super) succeeded: bool,
    pub(super) current_source_id: Option<SourceId>,
    /// How speech ended; `None` when this answer was not spoken.
    pub(super) speech: Option<SpokenOutcome>,
}

enum StreamItem {
    Delta(AssistantTextDelta),
    Done(Box<ConversationTurn>),
}

/// The model run's text stream and the response that ends it.
///
/// The response future and bounded receiver stay in the foreground scope. A terminal
/// write failure drops the native Rig run.
struct TextStream<'a> {
    response: Pin<Box<dyn Future<Output = ConversationTurn> + 'a>>,
    deltas: mpsc::Receiver<AssistantTextDelta>,
    deltas_open: bool,
    is_done: bool,
}

impl TextStream<'_> {
    /// The next streamed delta, or the finished turn. Cancel-safe: nothing is held
    /// across the await that a drop would lose.
    async fn next(&mut self) -> StreamItem {
        loop {
            tokio::select! {
                biased;
                delta = self.deltas.recv(), if self.deltas_open => match delta {
                    Some(delta) => return StreamItem::Delta(delta),
                    None => self.deltas_open = false,
                },
                turn = &mut self.response => {
                    self.is_done = true;
                    return StreamItem::Done(Box::new(turn));
                }
            }
        }
    }
}

/// Runs one prompt: streams the answer to the terminal and, when `speech` is given,
/// speaks it while it is still streaming.
///
/// One loop drives the model stream, the speech future and the user's input together,
/// so a `/stop` or a new prompt cancels speech immediately, even mid-stream. The text
/// itself cannot be cancelled: the run finishes, is archived and printed as usual.
/// Input that arrives while speech is not active stays queued, as before.
pub(super) async fn run_prompt<R, S, P>(
    assistant: &Assistant,
    session: &mut ConversationSession,
    memory: Option<&MemoryService>,
    presenter: &mut P,
    input: &mut InputReader<R>,
    speech: Option<&mut S>,
    prompt: &str,
) -> io::Result<PromptEnd>
where
    R: AsyncBufRead + Unpin,
    S: SpeechControl,
    P: AssistantPresenter,
{
    let prompt_started = Instant::now();
    presenter.state(AssistantState::Thinking)?;
    let (sender, deltas) = mpsc::channel(TEXT_DELTA_QUEUE);
    let mut text = TextStream {
        response: Box::pin(session.respond_stream(assistant, memory, prompt, sender)),
        deltas,
        deltas_open: true,
        is_done: false,
    };
    let mut finished: Option<FinishedTurn> = None;
    let mut spoken = None;
    let mut prompt_shown = false;

    if let Some(speech) = speech {
        let mut answer = SpokenAnswer::start(&mut *speech);
        let mut input_future: InputFuture<'_> = Box::pin(read_input_line(&mut *input));

        enum Event<E> {
            Input(io::Result<InputLine>),
            PlaybackStarted(bool),
            Speech(Result<(), E>),
            Text(StreamItem),
        }

        let exit = loop {
            let event = tokio::select! {
                biased;
                line = input_future.as_mut() => Event::Input(line),
                signalled = playback_signal(&mut answer.started), if answer.started.is_some() => {
                    Event::PlaybackStarted(signalled)
                }
                result = &mut answer.speaking => Event::Speech(result),
                item = text.next(), if finished.is_none() => Event::Text(item),
            };

            match event {
                Event::PlaybackStarted(signalled) => {
                    answer.note_playback_started(signalled, prompt_started);
                    if signalled && let Err(error) = presenter.state(AssistantState::Speaking) {
                        break SpeechExit::Io(error);
                    }
                }
                Event::Speech(result) => break SpeechExit::Finished(result),
                Event::Input(Ok(InputLine::Rejected(reason))) => {
                    presenter.input_received();
                    eprintln!("Input rejected: {}", reason.message());
                    drop(input_future);
                    input_future = Box::pin(read_input_line(&mut *input));
                    // While text is streaming, a prompt would land in the middle of it.
                    if finished.is_some()
                        && let Err(error) = presenter.prompt()
                    {
                        break SpeechExit::Io(error);
                    }
                }
                Event::Input(Ok(InputLine::Prompt(line))) => {
                    presenter.input_received();
                    break if line.trim() == "/stop" {
                        SpeechExit::Stopped
                    } else {
                        SpeechExit::Input(InputLine::Prompt(line))
                    };
                }
                Event::Input(Ok(InputLine::Eof)) => {
                    presenter.input_received();
                    break SpeechExit::Input(InputLine::Eof);
                }
                Event::Input(Err(error)) => break SpeechExit::Io(error),
                Event::Text(StreamItem::Delta(delta)) => {
                    if let Err(error) = presenter.delta(&delta.text) {
                        break SpeechExit::Io(error);
                    }
                    answer.feed.feed_text(&delta.text);
                }
                Event::Text(StreamItem::Done(turn)) => {
                    let end =
                        match complete_turn(*turn, &mut text, presenter, Some(&mut answer.feed)) {
                            Ok(end) => end,
                            Err(error) => break SpeechExit::Io(error),
                        };
                    let succeeded = end.succeeded;
                    finished = Some(end);
                    if !succeeded {
                        break SpeechExit::ModelFailed;
                    }
                    if let Err(error) = presenter.prompt() {
                        break SpeechExit::Io(error);
                    }
                    prompt_shown = true;
                }
            }
        };

        // The speech future must be gone before `speech` can be used again.
        drop(input_future);
        let first_audio = answer.first_audio;
        drop(answer);
        match settle_speech(speech, exit).await {
            Ok(wait) => {
                spoken = Some(SpokenOutcome { wait, first_audio });
                // Speech is over but the model is still writing.
                if first_audio.is_some()
                    && finished.is_none()
                    && let Err(error) = presenter.state(AssistantState::Thinking)
                {
                    // Same as a terminal write failure: stop the run, keep the prompt on record.
                    drop(text);
                    let _ = session.record_interrupted(prompt).await;
                    return Err(error);
                }
            }
            Err(error) => {
                // Same as a terminal write failure: stop the run and keep the prompt on record.
                if !text.is_done {
                    drop(text);
                    let _ = session.record_interrupted(prompt).await;
                }
                return Err(error);
            }
        }
    }

    let end = match finished {
        Some(end) => end,
        None => loop {
            match text.next().await {
                StreamItem::Delta(delta) => {
                    if let Err(error) = presenter.delta(&delta.text) {
                        drop(text);
                        let _ = session.record_interrupted(prompt).await;
                        return Err(error);
                    }
                }
                StreamItem::Done(turn) => {
                    break complete_turn(*turn, &mut text, presenter, None)?;
                }
            }
        },
    };

    // Speech ended before the text did, so the prompt has not been shown yet.
    if end.succeeded
        && !prompt_shown
        && matches!(
            spoken.as_ref().map(|spoken| &spoken.wait),
            Some(SpeechWait::Completed | SpeechWait::Stopped { .. })
        )
    {
        presenter.prompt()?;
    }
    presenter.state(AssistantState::Idle)?;

    Ok(PromptEnd {
        succeeded: end.succeeded,
        current_source_id: end.current_source_id,
        speech: spoken,
    })
}

struct FinishedTurn {
    succeeded: bool,
    current_source_id: Option<SourceId>,
}

/// Handles the response completing: shows the text still buffered, tells speech the
/// text is complete (only for a successful run), then prints the result.
fn complete_turn<P>(
    turn: ConversationTurn,
    text: &mut TextStream<'_>,
    presenter: &mut P,
    mut speech_text: Option<&mut ChunkFeed>,
) -> io::Result<FinishedTurn>
where
    P: AssistantPresenter,
{
    while let Ok(delta) = text.deltas.try_recv() {
        presenter.delta(&delta.text)?;
        if let Some(feed) = speech_text.as_deref_mut() {
            feed.feed_text(&delta.text);
        }
    }

    let ConversationTurn {
        run,
        context_warning,
        archive_error,
        memory_state,
        current_source_id,
    } = turn;
    let succeeded = run.is_ok();
    if succeeded && let Some(feed) = speech_text {
        feed.finish_text();
    }
    match run {
        Ok(run) => {
            presenter.completed(run.response.output())?;
            report_turn_warnings(
                context_warning.as_deref(),
                archive_error.as_ref(),
                memory_state,
            );
            print_run_diagnostics(&run.report);
        }
        Err(error) => {
            presenter.failed(safe_error_message(&error.error))?;
            report_turn_warnings(
                context_warning.as_deref(),
                archive_error.as_ref(),
                memory_state,
            );
            print_run_diagnostics(&error.report);
        }
    }
    Ok(FinishedTurn {
        succeeded,
        current_source_id,
    })
}

#[cfg(test)]
#[path = "../../../../tests/unit/interfaces/cli/repl/turn_tests.rs"]
mod tests;
