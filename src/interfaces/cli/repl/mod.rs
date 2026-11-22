use std::{future::Future, io, mem, pin::Pin, time::Duration};

use tokio::io::AsyncBufRead;

use crate::features::{
    assistant::{Assistant, CacheWarmup, ToolUse},
    control::{
        self, AnswerOutcome, Command, MemoryControl, Notice, TapAction, TimerReaction, tap_action,
    },
    conversation::ConversationSession,
    listening::Listener,
    memory::{MemoryService, RecallMode, wants_memory_search},
    panel::PanelHandle,
    state::AssistantState,
    timers::{TimerEvent, TimerLink},
};
use crate::interfaces::cli::diagnostics::{error_chain, format_first_audio, print_notices};
use crate::interfaces::cli::input::{Announcements, InputLine, InputReader, Taps, read_input_line};
use crate::interfaces::cli::presentation::AssistantPresenter;

mod idle;
mod listen;
mod speech;
mod turn;
mod warmup;

use idle::read_line_during_idle_work;
use listen::{ListenEnd, ListenStop, NOT_CONFIGURED, listen};
use speech::{SpeechControl, SpeechWait, SpokenOutcome, report_speech_cleanup, speak_lines};
use turn::{FILLER_LINES, PromptRequest, run_prompt};
use warmup::WarmupTask;

/// How long Jarvis must have heard nothing before the next prompt starts a new
/// conversation. Measured from the last recorded answer (see
/// `ConversationSession::quiet_for`), and checked only when a prompt arrives.
const QUIET_BEFORE_NEW_CONVERSATION: Duration = Duration::from_secs(30 * 60);

type InputFuture<'a> = Pin<Box<dyn Future<Output = io::Result<InputLine>> + 'a>>;

/// Runs the REPL until the input ends or the user exits.
///
/// `taps` is a second input source beside the typed line (see [`Taps`]); pass
/// `Taps::none()` for a keyboard-only session. `announcements` are lines from
/// background features, shown and said while idle (see [`Announcements`]).
/// `timers`, when given, answers "yes" and "no" for a proposed timer and says its
/// events while idle (timers 06); pass `None` for a session without timers.
/// `startup_warmup`, when given, is sent ahead before the first read so the first
/// question does not wait for a cold read of the prompt (fast-memory 13).
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
    announcements: Announcements,
    startup_warmup: Option<CacheWarmup>,
    presenter: &mut P,
    listener: Option<&Listener>,
    panels: &PanelHandle,
    timers: Option<TimerLink>,
) -> Result<(), Box<dyn std::error::Error>>
where
    R: AsyncBufRead + Unpin,
    S: SpeechControl,
    P: AssistantPresenter,
{
    let result = run_input_loop_inner(
        assistant,
        session,
        memory,
        speech,
        input,
        taps,
        announcements,
        startup_warmup,
        presenter,
        listener,
        panels,
        timers,
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
    mut announcements: Announcements,
    startup_warmup: Option<CacheWarmup>,
    presenter: &mut P,
    listener: Option<&Listener>,
    panels: &PanelHandle,
    mut timers: Option<TimerLink>,
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
    // The last fact noted while idle and not yet made the "forget that" focus. It lives
    // outside the read, so a tap that ends the wait does not lose it.
    let mut last_noted = None;
    // A line was handed over (a transcript, or a line typed over speech) and the state is
    // not yet `Idle`; whatever handles the line moves it on.
    let mut state_awaiting_idle = false;
    // A tap ended an answer or its speech; the loop listens instead of reading a line.
    let mut listen_due = false;
    // Which filler line the next search says; it lives as long as the session.
    let mut filler_position = 0;
    // Stopped when the next model run starts and when the loop ends.
    let mut warmup = WarmupTask::default();
    if let Some(startup_warmup) = startup_warmup {
        warmup.start(assistant, startup_warmup);
    }

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
                    read_line_during_idle_work(&mut input, memory, presenter, &mut last_noted).await
                } else {
                    read_input_line(&mut input).await.map(|line| (line, None))
                }
            };
            // Typed input wins over a tap, both over an announcement, and all over a
            // timer event. The losers are safe to drop: a half-read line stays in
            // `input`, idle work commits each vector on its own, and an unread tap,
            // announcement or timer event stays queued.
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
                notice = announcements.next() => Wake::Announcement(notice),
                Some(event) = next_timer_event(&mut timers) => {
                    match control::react_to(&event, &jiff::Zoned::now()) {
                        TimerReaction::Remind(prompt) => Wake::Reminder(prompt),
                        TimerReaction::Say(notice) => Wake::Announcement(notice),
                        TimerReaction::Show(notice) => Wake::Shown(notice),
                    }
                }
            }
        };
        if let Some(fact) = last_noted.take() {
            memory_control.focus_on_noted_fact(fact);
        }
        // A timer's reminder runs as a prompt Jarvis gives itself (timers D6).
        let mut is_reminder = false;
        let (line, memory_warning) = match wake {
            Wake::Line(line, memory_warning) => (line, memory_warning),
            Wake::Reminder(prompt) => {
                // Only the idle read takes one, so the prompt line is open: end it.
                println!();
                is_reminder = true;
                (InputLine::Prompt(prompt), None)
            }
            Wake::Shown(notice) => {
                println!();
                prompt_is_visible = false;
                match &notice {
                    Notice::Info(text) => presenter.notice(text)?,
                    Notice::Warning(text) => eprintln!("Warning: {text}"),
                }
                idle_work_due = true;
                continue;
            }
            Wake::Announcement(notice) => {
                // Only the idle read takes one, and the prompt is open then: end its
                // line, so the notice starts a line of its own.
                println!();
                prompt_is_visible = false;
                presenter.notice(notice.text())?;
                if let Some(speech) = available(speech, speech_available) {
                    speak_lines(speech, &[notice.text()]).await;
                    if speech.is_disabled() {
                        speech_available = false;
                    }
                }
                idle_work_due = true;
                continue;
            }
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
                            available(speech, speech_available),
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
            // A reminder is never parsed as a command: it is Jarvis's own line.
            InputLine::Prompt(raw) if is_reminder => {
                // The 30-minute rule applies as for any prompt (timers D8).
                reset_after_quiet(session, &mut warmup, &mut memory_control).await;
                let speaking = available(speech, speech_available);
                let end = run_prompt(
                    assistant,
                    session,
                    memory,
                    presenter,
                    &mut input,
                    &mut taps,
                    speaking,
                    &mut warmup,
                    PromptRequest {
                        text: &raw,
                        recall: RecallMode::Skip,
                        filler: None,
                        timers: None,
                        tools: ToolUse::Forbidden,
                    },
                )
                .await?;
                if end.succeeded {
                    memory_control.focus_on_turn(end.current_source_id);
                }
                prompt_is_visible = false;
                listen_due = end.listen_next;
                if let Some(outcome) = end.speech {
                    settle_spoken(
                        outcome,
                        speech,
                        presenter,
                        end.prompt_shown,
                        &mut speech_available,
                        &mut prompt_is_visible,
                        &mut state_awaiting_idle,
                        &mut next_input,
                    )?;
                }
                idle_work_due = true;
            }
            InputLine::Prompt(raw) => {
                let command = match control::parse(raw) {
                    // "yes" or "no" answers the proposed timer; with none waiting, the
                    // words are an ordinary prompt (timers D4).
                    Command::TimerAnswer { answer, text } => {
                        let outcome = match timers.as_ref() {
                            Some(link) => control::answer_timer(&link.handle, answer).await,
                            None => AnswerOutcome::NotForATimer,
                        };
                        match outcome {
                            AnswerOutcome::Answered(notice) => {
                                presenter.notice(notice.text())?;
                                if let Some(speech) = available(speech, speech_available) {
                                    speak_lines(speech, &[notice.text()]).await;
                                    if speech.is_disabled() {
                                        speech_available = false;
                                    }
                                }
                                idle_work_due = true;
                                continue;
                            }
                            AnswerOutcome::NotForATimer => Command::Prompt(text),
                        }
                    }
                    command => command,
                };
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
                                    available(speech, speech_available),
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
                    Command::Close => print_notices(&[control::close_panel(panels).await]),
                    Command::Show(panel) => {
                        print_notices(&[control::show_panel(panels, panel).await]);
                    }
                    // Answered before this match; one with no timer became a prompt.
                    Command::TimerAnswer { .. } => {}
                    Command::Help => println!("{}", control::help_text()),
                    Command::Usage(hint) => println!("{hint}"),
                    Command::Reset => {
                        if let Some(speech) = speech.as_mut() {
                            let cleanup_ok = report_speech_cleanup(speech.cancel().await);
                            if !cleanup_ok || speech.is_disabled() {
                                speech_available = false;
                            }
                        }
                        reset_conversation(session, &mut warmup, &mut memory_control).await;
                        println!("Conversation context reset.");
                    }
                    Command::Memory(command) => {
                        let spoken = command.is_spoken();
                        let notices = memory_control.run(command, memory, session).await;
                        print_notices(&notices);
                        if spoken && let Some(speech) = available(speech, speech_available) {
                            // Fact replies are said as well as shown, like "Got it."
                            let lines: Vec<&str> = notices.iter().map(Notice::text).collect();
                            speak_lines(speech, &lines).await;
                            if speech.is_disabled() {
                                speech_available = false;
                            }
                        }
                        idle_work_due = true;
                    }
                    Command::Remember(fact) => {
                        // A save is a command: no model run, no search, so no state
                        // change either. The reply is shown and, with speech on, said.
                        let notices = memory_control.remember(&fact, session).await;
                        let lines: Vec<&str> = notices.iter().map(Notice::text).collect();
                        for line in &lines {
                            presenter.notice(line)?;
                        }
                        if let Some(speech) = available(speech, speech_available) {
                            speak_lines(speech, &lines).await;
                            if speech.is_disabled() {
                                speech_available = false;
                            }
                        }
                        idle_work_due = true;
                    }
                    Command::Prompt(prompt) | Command::Recall(prompt) => {
                        // `run_prompt` reports its own states, starting with `Thinking`.
                        state_awaiting_idle = false;
                        reset_after_quiet(session, &mut warmup, &mut memory_control).await;
                        let speaking = available(speech, speech_available);
                        let end = run_prompt(
                            assistant,
                            session,
                            memory,
                            presenter,
                            &mut input,
                            &mut taps,
                            speaking,
                            &mut warmup,
                            PromptRequest {
                                text: &prompt,
                                recall,
                                filler: next_filler(recall, &mut filler_position),
                                timers: timers.as_ref().map(|link| &link.handle),
                                tools: ToolUse::Offered,
                            },
                        )
                        .await?;
                        if end.succeeded {
                            memory_control.focus_on_turn(end.current_source_id);
                        }
                        prompt_is_visible = false;
                        listen_due = end.listen_next;
                        if let Some(outcome) = end.speech {
                            settle_spoken(
                                outcome,
                                speech,
                                presenter,
                                end.prompt_shown,
                                &mut speech_available,
                                &mut prompt_is_visible,
                                &mut state_awaiting_idle,
                                &mut next_input,
                            )?;
                        }
                        idle_work_due = true;
                    }
                }
            }
        }
    }
}

/// Starts a new conversation when a prompt (Darren's, or a timer's reminder)
/// arrives after [`QUIET_BEFORE_NEW_CONVERSATION`] of quiet.
async fn reset_after_quiet(
    session: &mut ConversationSession,
    warmup: &mut WarmupTask,
    memory_control: &mut MemoryControl,
) {
    if let Some(quiet) = session.quiet_for()
        && quiet >= QUIET_BEFORE_NEW_CONVERSATION
    {
        eprintln!(
            "New conversation after {} minutes of quiet.",
            quiet.as_secs() / 60
        );
        reset_conversation(session, warmup, memory_control).await;
    }
}

/// After a spoken answer: reports the first audio and moves the loop on as the
/// speech ended (finished, stopped, interrupted by a line, aborted or tapped).
#[expect(
    clippy::too_many_arguments,
    reason = "the loop's speech, prompt and input state move together"
)]
fn settle_spoken<S, P>(
    outcome: SpokenOutcome,
    speech: &Option<S>,
    presenter: &mut P,
    prompt_shown: bool,
    speech_available: &mut bool,
    prompt_is_visible: &mut bool,
    state_awaiting_idle: &mut bool,
    next_input: &mut Option<InputLine>,
) -> io::Result<()>
where
    S: SpeechControl,
    P: AssistantPresenter,
{
    let SpokenOutcome { wait, first_audio } = outcome;
    presenter.notice(&format_first_audio(first_audio))?;
    let speech_disabled = speech.as_ref().is_some_and(S::is_disabled);
    match wait {
        SpeechWait::Completed => {
            if speech_disabled {
                *speech_available = false;
            }
            *prompt_is_visible = true;
        }
        SpeechWait::Stopped { cleanup_ok } => {
            if cleanup_ok {
                println!("Speech stopped.");
            }
            if !cleanup_ok || speech_disabled {
                *speech_available = false;
            }
            *prompt_is_visible = true;
        }
        SpeechWait::Input { line, cleanup_ok } => {
            if !cleanup_ok || speech_disabled {
                *speech_available = false;
            }
            *state_awaiting_idle = true;
            *next_input = Some(line);
            *prompt_is_visible = true;
        }
        SpeechWait::Aborted { cleanup_ok } => {
            if !cleanup_ok || speech_disabled {
                *speech_available = false;
            }
        }
        SpeechWait::Tapped { cleanup_ok } => {
            if !cleanup_ok || speech_disabled {
                *speech_available = false;
            }
            *prompt_is_visible = prompt_shown;
        }
    }
    Ok(())
}

/// Ends the conversation: the single path for `/reset` and the quiet reset, so
/// they cannot drift apart. The archive and the memories stay; a failure to open
/// the next archive session is a warning and the conversation goes on without one.
async fn reset_conversation(
    session: &mut ConversationSession,
    warmup: &mut WarmupTask,
    memory_control: &mut MemoryControl,
) {
    // The history the warm-up was sent for is gone.
    warmup.cancel();
    if let Err(error) = session.reset().await {
        eprintln!("Warning: local archiving is unavailable: {error}");
    }
    memory_control.clear();
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
        | Command::Close
        | Command::Show(_)
        | Command::Help
        | Command::Memory(_)
        | Command::Remember(_)
        | Command::Usage(_)
        | Command::TimerAnswer { .. } => RecallMode::Skip,
    }
}

/// The next timer event, or never when this session has no timers. Safe to drop at
/// any `.await`: an event is only taken when this returns.
async fn next_timer_event(timers: &mut Option<TimerLink>) -> Option<TimerEvent> {
    match timers {
        Some(link) => link.events.recv().await,
        None => std::future::pending().await,
    }
}

/// The filler line for a run that searches, advancing the rotation; `None` for a run
/// that skips the search. The position wraps after the last line.
fn next_filler(recall: RecallMode, position: &mut usize) -> Option<&'static str> {
    match recall {
        RecallMode::Search => {
            let line = FILLER_LINES[*position % FILLER_LINES.len()];
            *position = (*position + 1) % FILLER_LINES.len();
            Some(line)
        }
        RecallMode::Skip => None,
    }
}

/// What woke the loop: a line to handle, or a tap that starts listening.
enum Wake {
    Line(InputLine, Option<String>),
    Tap {
        /// Whether an open prompt line is on screen and must be ended first.
        prompt_on_screen: bool,
    },
    /// A background feature's line, shown and said while idle.
    Announcement(Notice),
    /// A timer ended: its reminder prompt, run with no tools (timers D6).
    Reminder(String),
    /// A line only shown, never said (timers D9).
    Shown(Notice),
}

/// The speech output, unless speech has been given up for this session.
fn available<S>(speech: &mut Option<S>, speech_available: bool) -> Option<&mut S> {
    if speech_available {
        speech.as_mut()
    } else {
        None
    }
}

/// Listens once and queues what the loop handles next.
///
/// The "speak now" cue plays out before the recorder starts, so the recording and
/// the silence detector never hear it. Nothing interrupts that wait: a cue is at
/// most 2 s long, and input typed or tapped meanwhile is read straight after.
#[expect(
    clippy::too_many_arguments,
    reason = "the loop hands over the state it keeps between inputs"
)]
async fn listen_once<R, S, P>(
    listener: &Listener,
    speech: Option<&mut S>,
    input: &mut InputReader<R>,
    taps: &mut Taps,
    presenter: &mut P,
    stop: ListenStop,
    state_awaiting_idle: &mut bool,
    next_input: &mut Option<InputLine>,
) -> io::Result<()>
where
    R: AsyncBufRead + Unpin,
    S: SpeechControl,
    P: AssistantPresenter,
{
    // `listen` reports its own states, starting with `Listening`.
    *state_awaiting_idle = false;
    let cue = speech.map_or(Duration::ZERO, S::play_listening_cue);
    if !cue.is_zero() {
        tokio::time::sleep(cue).await;
    }
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
