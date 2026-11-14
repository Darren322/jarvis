use std::{future::Future, io, pin::Pin, time::Instant};

use tokio::{io::AsyncBufRead, sync::mpsc};

use crate::features::{
    assistant::{Assistant, AssistantTextDelta},
    control::{TapAction, tap_action},
    conversation::{ConversationSession, ConversationTurn},
    memory::{MemoryService, RecallMode},
    state::AssistantState,
};
use crate::interfaces::cli::diagnostics::{
    print_run_diagnostics, report_turn_warnings, safe_error_message,
};
use crate::interfaces::cli::input::{InputLine, InputReader, Taps, read_input_line};
use crate::interfaces::cli::presentation::AssistantPresenter;
use crate::storage::SourceId;

use super::InputFuture;
use super::speech::{
    ChunkFeed, SpeechControl, SpeechExit, SpeechWait, SpokenAnswer, SpokenOutcome, playback_signal,
    settle_speech,
};
use super::warmup::WarmupTask;

/// Text deltas the model run may have buffered ahead of the terminal.
const TEXT_DELTA_QUEUE: usize = 32;

/// Shown in place of an answer that a tap dropped before any of it was printed.
const ANSWER_DROPPED: &str = "Answer dropped.";

/// The canned lines said when a search starts (spec D4), used in this order and
/// wrapping after the last. They are never sent to the model or stored.
pub(super) const FILLER_LINES: [&str; 3] = ["Let me check.", "One moment.", "Let me look that up."];

/// One prompt and whether its run searches memory first.
pub(super) struct PromptRequest<'a> {
    pub(super) text: &'a str,
    pub(super) recall: RecallMode,
    /// The line to show (and speak) at once because this run searches; `None` for a
    /// run that skips the search. It is not part of the answer.
    pub(super) filler: Option<&'static str>,
}

/// What the loop needs to know about a finished prompt.
pub(super) struct PromptEnd {
    /// Whether the model run succeeded, so the turn can become the "forget that" focus.
    pub(super) succeeded: bool,
    pub(super) current_source_id: Option<SourceId>,
    /// How speech ended; `None` when this answer was not spoken.
    pub(super) speech: Option<SpokenOutcome>,
    /// A tap ended this prompt, so the loop listens next. The state is left where it
    /// was; listening reports the next one.
    pub(super) listen_next: bool,
    /// Whether the prompt is already on screen below the answer.
    pub(super) prompt_shown: bool,
}

/// Whether a tap in `state` ends the prompt that is running. Only `Thinking` and
/// `Speaking` occur while one runs.
fn tap_ends_prompt(state: AssistantState) -> bool {
    match tap_action(state) {
        TapAction::DropAnswerThenListen | TapAction::StopSpeakingThenListen => true,
        TapAction::Listen | TapAction::SendNow | TapAction::Ignore => false,
    }
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
    /// Whether the model has finished and only storing the turn remains.
    ///
    /// The run owns the sending half of `deltas` and drops it when the model ends,
    /// before the turn is archived. While this is false the response future has not
    /// reached the archive, so dropping it stores nothing. Once it is true the
    /// archive write may already be under way and can no longer be taken back.
    fn is_storing(&self) -> bool {
        !self.deltas_open || self.deltas.is_closed()
    }

    /// Finishes a run whose model has ended: shows the text still buffered and
    /// returns the stored turn.
    async fn land<P: AssistantPresenter>(
        &mut self,
        presenter: &mut P,
        warming: Warming<'_>,
    ) -> io::Result<FinishedTurn> {
        loop {
            match self.next().await {
                StreamItem::Delta(delta) => presenter.delta(&delta.text)?,
                StreamItem::Done(turn) => {
                    return complete_turn(*turn, self, presenter, None, warming);
                }
            }
        }
    }

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
/// so a `/stop` or a new prompt cancels speech immediately, even mid-stream. Typed
/// input cannot cancel the text: the run finishes, is archived and printed as usual,
/// and a line typed while speech is not active stays queued, as before.
///
/// A tap can (spec D7). While the answer is still being written, a tap drops the
/// model run: nothing of it is archived or added to the history, and the voice
/// stops with it. Once the text is complete, a tap only stops the voice. Either
/// way the loop listens next (`PromptEnd::listen_next`). A tap that lands after the
/// model has ended but before its turn is stored comes too late to drop anything:
/// that answer is kept whole (see `TextStream::is_storing`), so an answer is never
/// stored without also being shown and added to the history.
///
/// A cache warm-up still running from an earlier turn is stopped before this run
/// starts, and one is started as soon as this turn is stored if it trimmed the
/// history (see [`WarmupTask`]).
#[expect(
    clippy::too_many_arguments,
    reason = "one prompt touches every service the loop holds, plus both input sources"
)]
pub(super) async fn run_prompt<R, S, P>(
    assistant: &Assistant,
    session: &mut ConversationSession,
    memory: Option<&MemoryService>,
    presenter: &mut P,
    input: &mut InputReader<R>,
    taps: &mut Taps,
    speech: Option<&mut S>,
    warmup: &mut WarmupTask,
    request: PromptRequest<'_>,
) -> io::Result<PromptEnd>
where
    R: AsyncBufRead + Unpin,
    S: SpeechControl,
    P: AssistantPresenter,
{
    let PromptRequest {
        text: prompt,
        recall,
        filler,
    } = request;
    let prompt_started = Instant::now();
    warmup.cancel();
    presenter.state(AssistantState::Thinking)?;
    if let Some(line) = filler {
        // Shown before the model has produced anything. A terminal write failure here
        // ends the prompt before the run starts, so nothing is stored.
        presenter.notice(line)?;
    }
    let (sender, deltas) = mpsc::channel(TEXT_DELTA_QUEUE);
    let mut text = TextStream {
        response: Box::pin(session.respond_stream(assistant, memory, recall, prompt, sender)),
        deltas,
        deltas_open: true,
        is_done: false,
    };
    let mut finished: Option<FinishedTurn> = None;
    let mut spoken = None;
    let mut prompt_shown = false;

    if let Some(speech) = speech {
        let mut answer = SpokenAnswer::start(&mut *speech);
        if let Some(line) = filler {
            // The first chunk of this answer's speech, ahead of the model's text.
            answer.feed.feed_line(line);
        }
        let mut input_future: InputFuture<'_> = Box::pin(read_input_line(&mut *input));

        enum Event<E> {
            Input(io::Result<InputLine>),
            Tap,
            PlaybackStarted(bool),
            Speech(Result<(), E>),
            Text(StreamItem),
        }

        let exit = loop {
            let event = tokio::select! {
                biased;
                line = input_future.as_mut() => Event::Input(line),
                _tap = taps.next() => Event::Tap,
                signalled = playback_signal(&mut answer.started), if answer.started.is_some() => {
                    Event::PlaybackStarted(signalled)
                }
                result = &mut answer.speaking => Event::Speech(result),
                item = text.next(), if finished.is_none() => Event::Text(item),
            };

            match event {
                Event::PlaybackStarted(signalled) => {
                    // With a search the first audio is usually the filler line, so this
                    // measures the wait for the filler, not for the answer's first word.
                    answer.note_playback_started(signalled, prompt_started);
                    if signalled && let Err(error) = presenter.state(AssistantState::Speaking) {
                        break SpeechExit::Io(error);
                    }
                }
                Event::Speech(result) => break SpeechExit::Finished(result),
                Event::Tap => {
                    // `Speaking` is shown from the first audio; until then it is `Thinking`.
                    let state = if answer.first_audio.is_some() {
                        AssistantState::Speaking
                    } else {
                        AssistantState::Thinking
                    };
                    if tap_ends_prompt(state) {
                        break SpeechExit::Tapped;
                    }
                }
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
                    let end = match complete_turn(
                        *turn,
                        &mut text,
                        presenter,
                        Some(&mut answer.feed),
                        Warming {
                            assistant,
                            task: &mut *warmup,
                        },
                    ) {
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
                let tapped = matches!(wait, SpeechWait::Tapped { .. });
                spoken = Some(SpokenOutcome { wait, first_audio });
                if tapped {
                    let (succeeded, current_source_id) = match finished {
                        Some(end) => (end.succeeded, end.current_source_id),
                        None if text.is_storing() => {
                            let end = text
                                .land(
                                    presenter,
                                    Warming {
                                        assistant,
                                        task: &mut *warmup,
                                    },
                                )
                                .await?;
                            (end.succeeded, end.current_source_id)
                        }
                        // Still being written: dropped with the voice, never archived.
                        None => {
                            drop(text);
                            presenter.failed(ANSWER_DROPPED)?;
                            (false, None)
                        }
                    };
                    return Ok(PromptEnd {
                        succeeded,
                        current_source_id,
                        speech: spoken,
                        listen_next: true,
                        prompt_shown,
                    });
                }
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
            let item = tokio::select! {
                biased;
                _tap = taps.next() => None,
                item = text.next() => Some(item),
            };
            let Some(item) = item else {
                if !tap_ends_prompt(AssistantState::Thinking) {
                    continue;
                }
                let (succeeded, current_source_id) = if text.is_storing() {
                    let end = text
                        .land(
                            presenter,
                            Warming {
                                assistant,
                                task: &mut *warmup,
                            },
                        )
                        .await?;
                    (end.succeeded, end.current_source_id)
                } else {
                    // Dropped mid-run: never archived, and the history is untouched.
                    drop(text);
                    presenter.failed(ANSWER_DROPPED)?;
                    (false, None)
                };
                return Ok(PromptEnd {
                    succeeded,
                    current_source_id,
                    speech: spoken,
                    listen_next: true,
                    prompt_shown: false,
                });
            };
            match item {
                StreamItem::Delta(delta) => {
                    if let Err(error) = presenter.delta(&delta.text) {
                        drop(text);
                        let _ = session.record_interrupted(prompt).await;
                        return Err(error);
                    }
                }
                StreamItem::Done(turn) => {
                    break complete_turn(
                        *turn,
                        &mut text,
                        presenter,
                        None,
                        Warming {
                            assistant,
                            task: &mut *warmup,
                        },
                    )?;
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
    // A line typed over the speech is handled next and reports the state it moves to; an
    // `[idle]` here would only flash between the two. The loop reports `Idle` if that
    // line turns out to be a command that reports no state of its own.
    if !matches!(
        spoken.as_ref().map(|spoken| &spoken.wait),
        Some(SpeechWait::Input { .. })
    ) {
        presenter.state(AssistantState::Idle)?;
    }

    Ok(PromptEnd {
        succeeded: end.succeeded,
        current_source_id: end.current_source_id,
        speech: spoken,
        listen_next: false,
        prompt_shown,
    })
}

/// What a finished turn needs to start a cache warm-up.
struct Warming<'a> {
    assistant: &'a Assistant,
    task: &'a mut WarmupTask,
}

struct FinishedTurn {
    succeeded: bool,
    current_source_id: Option<SourceId>,
}

/// Handles the response completing: shows the text still buffered, tells speech the
/// text is complete (only for a successful run), starts the cache warm-up the turn
/// asked for, then prints the result.
fn complete_turn<P>(
    turn: ConversationTurn,
    text: &mut TextStream<'_>,
    presenter: &mut P,
    mut speech_text: Option<&mut ChunkFeed>,
    warming: Warming<'_>,
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
        cache_warmup,
    } = turn;
    if let Some(cache_warmup) = cache_warmup {
        warming.task.start(warming.assistant, cache_warmup);
    }
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
