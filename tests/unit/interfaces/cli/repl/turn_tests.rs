use std::{
    cell::RefCell,
    io,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use rig_agent::{
    AgentBuilder,
    prelude::{CompletionError, CompletionModel},
};
use rig_core::{
    completion::{CompletionRequest, CompletionResponse, Usage},
    message::Message,
    streaming::StreamingCompletionResponse,
    test_utils::{MockCompletionModel, MockStreamEvent},
};
use tokio::{
    io::{AsyncWriteExt, BufReader, DuplexStream},
    sync::{mpsc, oneshot},
};

use crate::{
    features::control::Tap,
    features::{
        assistant::{Assistant, CacheWarmup},
        conversation::ConversationSession,
        state::AssistantState,
    },
    interfaces::cli::{
        input::{InputReader, Taps},
        presentation::AssistantPresenter,
    },
};

use super::*;

type Log = Rc<RefCell<Vec<String>>>;

/// Records what reached the terminal, in order, next to what reached speech.
struct LoggingPresenter {
    log: Log,
    fail_on_delta: bool,
}

impl AssistantPresenter for LoggingPresenter {
    fn delta(&mut self, text: &str) -> io::Result<()> {
        if self.fail_on_delta {
            return Err(io::Error::other("terminal closed"));
        }
        self.log.borrow_mut().push(format!("delta:{text}"));
        Ok(())
    }

    fn completed(&mut self, _canonical_answer: &str) -> io::Result<()> {
        self.log.borrow_mut().push("completed".to_owned());
        Ok(())
    }

    fn failed(&mut self, _safe_message: &str) -> io::Result<()> {
        self.log.borrow_mut().push("failed".to_owned());
        Ok(())
    }

    fn prompt(&mut self) -> io::Result<()> {
        self.log.borrow_mut().push("prompt".to_owned());
        Ok(())
    }

    fn notice(&mut self, line: &str) -> io::Result<()> {
        self.log.borrow_mut().push(format!("notice:{line}"));
        Ok(())
    }

    fn state(&mut self, state: AssistantState) -> io::Result<()> {
        self.log.borrow_mut().push(format!("state:{state:?}"));
        Ok(())
    }

    fn input_received(&mut self) {}
}

enum Behaviour {
    /// Speaks every chunk and finishes when the text is complete.
    SpeaksUntilTextEnds,
    FailsOnFirstChunk,
    /// Starts playback on the first chunk, then ends without waiting for the rest.
    EndsAfterFirstChunk,
    /// Plays forever once the text is complete.
    NeverFinishes,
    /// Once the text is complete, the user types `line` and playback never finishes.
    UserTypesAfterText {
        line: &'static str,
        writer: DuplexStream,
    },
    /// Once the text is complete, the orb is tapped and playback never finishes.
    TappedAfterText(mpsc::Sender<Tap>),
    /// The orb is tapped as the first chunk starts playing; playback never finishes.
    TappedOnFirstChunk(mpsc::Sender<Tap>),
}

struct ScriptedSpeech {
    log: Log,
    behaviour: Behaviour,
    signals_playback: bool,
    cancel_calls: usize,
}

impl SpeechControl for ScriptedSpeech {
    type Error = io::Error;

    fn is_disabled(&self) -> bool {
        false
    }

    async fn speak(
        &mut self,
        mut chunks: mpsc::Receiver<String>,
        playback_started: oneshot::Sender<()>,
    ) -> Result<(), Self::Error> {
        let mut playback_started = Some(playback_started);
        while let Some(chunk) = chunks.recv().await {
            self.log.borrow_mut().push(format!("chunk:{chunk}"));
            if self.signals_playback
                && let Some(started) = playback_started.take()
            {
                let _ = started.send(());
            }
            if matches!(self.behaviour, Behaviour::FailsOnFirstChunk) {
                return Err(io::Error::other("injected speech failure"));
            }
            if let Behaviour::TappedOnFirstChunk(taps) = &self.behaviour {
                // Let the caller see the playback signal before the tap.
                tokio::task::yield_now().await;
                let _ = taps.try_send(Tap);
            }
            if matches!(self.behaviour, Behaviour::EndsAfterFirstChunk) {
                // Let the caller see the playback signal before the speech result.
                tokio::task::yield_now().await;
                return Ok(());
            }
        }
        self.log.borrow_mut().push("text complete".to_owned());
        match &mut self.behaviour {
            Behaviour::SpeaksUntilTextEnds
            | Behaviour::FailsOnFirstChunk
            | Behaviour::EndsAfterFirstChunk => Ok(()),
            Behaviour::NeverFinishes | Behaviour::TappedOnFirstChunk(_) => {
                std::future::pending().await
            }
            Behaviour::TappedAfterText(taps) => {
                taps.send(Tap).await.expect("the prompt reads taps");
                std::future::pending().await
            }
            Behaviour::UserTypesAfterText { line, writer } => {
                writer
                    .write_all(format!("{line}\n").as_bytes())
                    .await
                    .expect("the test reader is open");
                std::future::pending().await
            }
        }
    }

    async fn cancel(&mut self) -> Result<(), Self::Error> {
        self.cancel_calls += 1;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn speech(log: &Log, behaviour: Behaviour) -> ScriptedSpeech {
    ScriptedSpeech {
        log: Rc::clone(log),
        behaviour,
        signals_playback: true,
        cancel_calls: 0,
    }
}

const FIRST_SENTENCE: &str = "The first sentence is long enough to be spoken early.";
const SECOND_SENTENCE: &str = "The second one ends the answer.";

fn answer_turn() -> Vec<MockStreamEvent> {
    vec![
        MockStreamEvent::text(format!("{FIRST_SENTENCE} ")),
        MockStreamEvent::text(SECOND_SENTENCE),
        MockStreamEvent::final_response(Usage::new()),
    ]
}

struct Run {
    end: PromptEnd,
    log: Log,
    speech: Option<ScriptedSpeech>,
}

/// Runs one prompt against a scripted model. `typed` is waiting in the input before the
/// prompt starts. Something must hold the writing end of `reader` open for the whole
/// run, or the input would end: `writer`, or the speech double when it types by itself.
async fn run_one_prompt(
    model_turn: Vec<MockStreamEvent>,
    log: &Log,
    speech: Option<ScriptedSpeech>,
    reader: DuplexStream,
    writer: Option<DuplexStream>,
    typed: &[u8],
) -> Run {
    run_one_prompt_with_taps(model_turn, log, speech, reader, writer, typed, Taps::none())
        .await
        .0
}

/// `run_one_prompt` with a touch front end. Also returns how many turns the
/// archive holds afterwards.
async fn run_one_prompt_with_taps(
    model_turn: Vec<MockStreamEvent>,
    log: &Log,
    speech: Option<ScriptedSpeech>,
    reader: DuplexStream,
    writer: Option<DuplexStream>,
    typed: &[u8],
    taps: Taps,
) -> (Run, i64) {
    let (run, archived_turns, _) = run_request(
        model_turn,
        log,
        speech,
        reader,
        writer,
        typed,
        taps,
        (RecallMode::Skip, None),
    )
    .await;
    (run, archived_turns)
}

/// The most general runner: also takes the run's recall mode and filler line, and
/// returns the requests the model received.
#[expect(clippy::too_many_arguments, reason = "a test runner with every knob")]
async fn run_request(
    model_turn: Vec<MockStreamEvent>,
    log: &Log,
    speech: Option<ScriptedSpeech>,
    reader: DuplexStream,
    mut writer: Option<DuplexStream>,
    typed: &[u8],
    mut taps: Taps,
    (recall, filler): (RecallMode, Option<&'static str>),
) -> (Run, i64, Vec<rig_core::completion::CompletionRequest>) {
    let model = MockCompletionModel::from_stream_turns([model_turn]);
    let model_handle = model.clone();
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
        )
        .build(),
    );
    let archive_dir = tempfile::tempdir().expect("temporary archive directory should open");
    let mut session = ConversationSession::new(archive_dir.path().join("conversation.sqlite3"));
    assert!(session.reset().await.is_ok());
    if let Some(writer) = writer.as_mut() {
        writer.write_all(typed).await.expect("input should fit");
    }

    let mut input = InputReader::new(BufReader::new(reader));
    let mut presenter = LoggingPresenter {
        log: Rc::clone(log),
        fail_on_delta: false,
    };
    let mut speech = speech;
    let end = run_prompt(
        &assistant,
        &mut session,
        None,
        &mut presenter,
        &mut input,
        &mut taps,
        speech.as_mut(),
        &mut WarmupTask::default(),
        PromptRequest {
            text: "a question",
            recall,
            filler,
        },
    )
    .await
    .expect("the prompt should run");
    drop(writer);
    drop(session);
    let archived_turns =
        rusqlite::Connection::open(archive_dir.path().join("conversation.sqlite3"))
            .expect("the archive opens")
            .query_row("SELECT COUNT(*) FROM turns", [], |row| row.get(0))
            .expect("the turns can be counted");
    (
        Run {
            end,
            log: Rc::clone(log),
            speech,
        },
        archived_turns,
        model_handle.requests(),
    )
}

/// Runs one prompt with a tap queue; `speech` is given the sending half.
async fn run_with_taps(
    speech: impl FnOnce(&Log, mpsc::Sender<Tap>) -> Option<ScriptedSpeech>,
    tap_before_start: bool,
) -> (Run, i64) {
    let log = Log::default();
    let (tap, receiver) = mpsc::channel(1);
    if tap_before_start {
        tap.try_send(Tap).expect("the queue is empty");
    }
    let speech = speech(&log, tap);
    let (writer, reader) = tokio::io::duplex(512);
    run_one_prompt_with_taps(
        answer_turn(),
        &log,
        speech,
        reader,
        Some(writer),
        b"",
        Taps::new(receiver),
    )
    .await
}

async fn run_with_input(
    model_turn: Vec<MockStreamEvent>,
    speech: impl FnOnce(&Log) -> Option<ScriptedSpeech>,
    typed: &str,
) -> Run {
    let log = Log::default();
    let speech = speech(&log);
    let (writer, reader) = tokio::io::duplex(512);
    run_one_prompt(
        model_turn,
        &log,
        speech,
        reader,
        Some(writer),
        typed.as_bytes(),
    )
    .await
}

fn events(run: &Run) -> Vec<String> {
    run.log.borrow().clone()
}

/// The state names in the order they were reported.
fn states(events: &[String]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| event.strip_prefix("state:"))
        .collect()
}

fn position(events: &[String], event: &str) -> usize {
    events
        .iter()
        .position(|candidate| candidate == event)
        .unwrap_or_else(|| panic!("{event:?} not in {events:?}"))
}

fn wait(run: &Run) -> &SpeechWait {
    &run.end.speech.as_ref().expect("the answer was spoken").wait
}

#[tokio::test]
async fn the_first_chunk_reaches_speech_while_the_text_is_still_streaming() {
    let run = run_with_input(
        answer_turn(),
        |log| Some(speech(log, Behaviour::SpeaksUntilTextEnds)),
        "",
    )
    .await;

    let events = events(&run);
    let first_chunk = position(&events, &format!("chunk:{FIRST_SENTENCE}"));
    let second_chunk = position(&events, &format!("chunk:{SECOND_SENTENCE}"));
    assert!(
        first_chunk < position(&events, &format!("delta:{SECOND_SENTENCE}")),
        "speech has the first sentence before the second one streams in: {events:?}"
    );
    assert!(first_chunk < position(&events, "completed"));
    assert!(
        second_chunk > first_chunk,
        "the unfinished rest is flushed when the text ends"
    );
    assert!(
        position(&events, "text complete") > second_chunk,
        "the queue closes after the last chunk"
    );
    assert!(matches!(wait(&run), SpeechWait::Completed));
    assert!(run.end.succeeded);
}

#[tokio::test]
async fn a_spoken_answer_goes_from_thinking_to_speaking_when_audio_starts_then_idle() {
    let run = run_with_input(
        answer_turn(),
        |log| Some(speech(log, Behaviour::SpeaksUntilTextEnds)),
        "",
    )
    .await;

    let events = events(&run);
    assert_eq!(states(&events), ["Thinking", "Speaking", "Idle"]);
    assert!(
        position(&events, "state:Speaking") < position(&events, "completed"),
        "speaking is reported when audio starts, not when the text ends: {events:?}"
    );
}

#[tokio::test]
async fn speech_that_never_starts_playback_is_never_reported_as_speaking() {
    let run = run_with_input(
        answer_turn(),
        |log| {
            Some(ScriptedSpeech {
                signals_playback: false,
                ..speech(log, Behaviour::SpeaksUntilTextEnds)
            })
        },
        "",
    )
    .await;

    assert_eq!(states(&events(&run)), ["Thinking", "Idle"]);
}

#[tokio::test]
async fn speech_that_ends_while_the_text_streams_returns_to_thinking() {
    let run = run_with_input(
        answer_turn(),
        |log| Some(speech(log, Behaviour::EndsAfterFirstChunk)),
        "",
    )
    .await;

    assert_eq!(
        states(&events(&run)),
        ["Thinking", "Speaking", "Thinking", "Idle"]
    );
}

#[tokio::test]
async fn time_to_first_audio_is_measured_when_playback_starts() {
    let run = run_with_input(
        answer_turn(),
        |log| Some(speech(log, Behaviour::SpeaksUntilTextEnds)),
        "",
    )
    .await;

    assert!(run.end.speech.expect("spoken").first_audio.is_some());
}

#[tokio::test]
async fn time_to_first_audio_is_unavailable_when_playback_never_starts() {
    let run = run_with_input(
        answer_turn(),
        |log| {
            Some(ScriptedSpeech {
                signals_playback: false,
                ..speech(log, Behaviour::SpeaksUntilTextEnds)
            })
        },
        "",
    )
    .await;

    assert_eq!(run.end.speech.expect("spoken").first_audio, None);
}

#[tokio::test]
async fn an_answer_without_speech_is_printed_and_reports_no_speech() {
    let run = run_with_input(answer_turn(), |_| None, "").await;

    let events = events(&run);
    assert_eq!(
        events,
        [
            "state:Thinking".to_owned(),
            format!("delta:{FIRST_SENTENCE} "),
            format!("delta:{SECOND_SENTENCE}"),
            "completed".to_owned(),
            "state:Idle".to_owned(),
        ]
    );
    assert!(run.end.speech.is_none());
    assert!(run.end.succeeded);
}

#[tokio::test]
async fn a_speech_failure_mid_stream_does_not_stop_the_text_and_the_prompt_follows_it() {
    let run = run_with_input(
        answer_turn(),
        |log| Some(speech(log, Behaviour::FailsOnFirstChunk)),
        "",
    )
    .await;

    let events = events(&run);
    assert!(events.contains(&format!("delta:{SECOND_SENTENCE}")));
    assert!(position(&events, "completed") < position(&events, "prompt"));
    assert!(position(&events, "prompt") < position(&events, "state:Idle"));
    assert!(matches!(wait(&run), SpeechWait::Completed));
    assert_eq!(run.speech.expect("speech").cancel_calls, 0);
}

#[tokio::test]
async fn stopping_while_the_text_streams_cancels_speech_but_the_answer_still_finishes() {
    // `/stop` is already waiting, so it is read before the first word of text.
    let run = run_with_input(
        answer_turn(),
        |log| Some(speech(log, Behaviour::NeverFinishes)),
        "/stop\n",
    )
    .await;

    let events = events(&run);
    assert!(events.contains(&format!("delta:{FIRST_SENTENCE} ")));
    assert!(events.contains(&"completed".to_owned()));
    assert!(
        !events.iter().any(|event| event.starts_with("chunk:")),
        "nothing is spoken after the stop: {events:?}"
    );
    assert!(position(&events, "completed") < position(&events, "prompt"));
    assert_eq!(states(&events), ["Thinking", "Idle"]);
    assert!(matches!(
        wait(&run),
        SpeechWait::Stopped { cleanup_ok: true }
    ));
    assert_eq!(run.speech.expect("speech").cancel_calls, 1);
    assert!(run.end.succeeded);
}

#[tokio::test]
async fn a_new_prompt_while_the_text_streams_cancels_speech_and_is_kept_as_the_next_input() {
    let run = run_with_input(
        answer_turn(),
        |log| Some(speech(log, Behaviour::NeverFinishes)),
        "another question\n",
    )
    .await;

    assert!(matches!(
        wait(&run),
        SpeechWait::Input {
            line: InputLine::Prompt(line),
            cleanup_ok: true,
        } if line == "another question"
    ));
    assert!(events(&run).contains(&"completed".to_owned()));
    assert_eq!(run.speech.expect("speech").cancel_calls, 1);
}

#[tokio::test]
async fn stopping_after_the_text_ended_cancels_the_chunks_still_playing() {
    let log = Log::default();
    let (typist, reader) = tokio::io::duplex(512);
    let speech = speech(
        &log,
        Behaviour::UserTypesAfterText {
            line: "/stop",
            writer: typist,
        },
    );

    let run = run_one_prompt(answer_turn(), &log, Some(speech), reader, None, b"").await;

    let events = events(&run);
    assert!(position(&events, "text complete") > position(&events, "completed"));
    assert!(matches!(
        wait(&run),
        SpeechWait::Stopped { cleanup_ok: true }
    ));
    assert_eq!(run.speech.expect("speech").cancel_calls, 1);
}

#[tokio::test]
async fn a_failed_model_run_cancels_speech_without_speaking_the_rest() {
    let run = run_with_input(
        vec![
            MockStreamEvent::text("A partial answer that never reaches a sentence end"),
            MockStreamEvent::error("injected midstream failure"),
        ],
        |log| Some(speech(log, Behaviour::NeverFinishes)),
        "",
    )
    .await;

    let events = events(&run);
    assert!(events.contains(&"failed".to_owned()));
    assert!(
        !events.contains(&"text complete".to_owned()),
        "the text was abandoned, not completed: {events:?}"
    );
    assert!(!run.end.succeeded);
    assert!(matches!(
        wait(&run),
        SpeechWait::Aborted { cleanup_ok: true }
    ));
    assert_eq!(run.speech.expect("speech").cancel_calls, 1);
}

#[tokio::test]
async fn a_terminal_failure_mid_stream_cancels_speech_and_stops_the_run() {
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(MockCompletionModel::from_stream_turns([answer_turn()])),
            crate::features::panel::channel().0,
        )
        .build(),
    );
    let archive_dir = tempfile::tempdir().expect("temporary archive directory should open");
    let mut session = ConversationSession::new(archive_dir.path().join("conversation.sqlite3"));
    assert!(session.reset().await.is_ok());
    let log = Log::default();
    let mut speech = speech(&log, Behaviour::NeverFinishes);
    let (_writer, reader) = tokio::io::duplex(64);
    let mut input = InputReader::new(BufReader::new(reader));
    let mut presenter = LoggingPresenter {
        log: Rc::clone(&log),
        fail_on_delta: true,
    };

    let result = run_prompt(
        &assistant,
        &mut session,
        None,
        &mut presenter,
        &mut input,
        &mut Taps::none(),
        Some(&mut speech),
        &mut WarmupTask::default(),
        PromptRequest {
            text: "a question",
            recall: RecallMode::Skip,
            filler: None,
        },
    )
    .await;

    assert!(result.is_err());
    assert_eq!(speech.cancel_calls, 1);
}

#[tokio::test]
async fn a_tap_while_thinking_drops_the_answer_and_nothing_is_archived() {
    // No speech; the tap is waiting, so it is seen before the first word of text.
    let (run, archived_turns) = run_with_taps(|_, _| None, true).await;

    let events = events(&run);
    assert!(events.contains(&"failed".to_owned()));
    assert!(!events.contains(&"completed".to_owned()));
    assert!(!events.iter().any(|event| event.starts_with("delta:")));
    // No `Idle`: listening reports the next state.
    assert_eq!(states(&events), ["Thinking"]);
    assert!(run.end.listen_next);
    assert!(!run.end.succeeded);
    assert_eq!(run.end.current_source_id, None);
    assert!(!run.end.prompt_shown);
    assert_eq!(archived_turns, 0);
}

#[tokio::test]
async fn a_tap_before_the_voice_starts_drops_the_answer_and_cancels_speech() {
    let (run, archived_turns) =
        run_with_taps(|log, _| Some(speech(log, Behaviour::NeverFinishes)), true).await;

    let events = events(&run);
    assert!(events.contains(&"failed".to_owned()));
    assert!(!events.contains(&"completed".to_owned()));
    assert!(!events.iter().any(|event| event.starts_with("chunk:")));
    assert_eq!(states(&events), ["Thinking"]);
    assert!(matches!(
        wait(&run),
        SpeechWait::Tapped { cleanup_ok: true }
    ));
    assert_eq!(run.speech.expect("speech").cancel_calls, 1);
    assert!(run.end.listen_next);
    assert!(!run.end.succeeded);
    assert_eq!(archived_turns, 0);
}

#[tokio::test]
async fn a_tap_while_speaking_a_finished_answer_stops_the_voice_and_keeps_the_answer() {
    let (run, archived_turns) = run_with_taps(
        |log, tap| Some(speech(log, Behaviour::TappedAfterText(tap))),
        false,
    )
    .await;

    let events = events(&run);
    assert!(events.contains(&"completed".to_owned()));
    assert!(!events.contains(&"failed".to_owned()));
    assert_eq!(states(&events), ["Thinking", "Speaking"]);
    assert!(matches!(
        wait(&run),
        SpeechWait::Tapped { cleanup_ok: true }
    ));
    assert_eq!(run.speech.expect("speech").cancel_calls, 1);
    assert!(run.end.listen_next);
    assert!(run.end.succeeded);
    assert!(run.end.prompt_shown);
    assert_eq!(archived_turns, 1);
}

#[tokio::test]
async fn an_answer_dropped_by_a_tap_mid_speech_is_archived_only_if_it_had_finished() {
    let (run, archived_turns) = run_with_taps(
        |log, tap| Some(speech(log, Behaviour::TappedOnFirstChunk(tap))),
        false,
    )
    .await;

    // Whether the model had ended when the tap came is up to the stream. What must
    // hold either way: an answer is stored exactly when it was shown whole.
    let events = events(&run);
    let completed = events.contains(&"completed".to_owned());
    let dropped = events.contains(&"failed".to_owned());
    assert_ne!(completed, dropped, "{events:?}");
    assert_eq!(run.end.succeeded, completed);
    assert_eq!(archived_turns, i64::from(completed));
    assert!(run.end.listen_next);
    assert!(matches!(
        wait(&run),
        SpeechWait::Tapped { cleanup_ok: true }
    ));
    assert_eq!(run.speech.expect("speech").cancel_calls, 1);
}

#[tokio::test]
async fn without_a_touch_front_end_a_prompt_never_asks_to_listen() {
    let run = run_with_input(answer_turn(), |_| None, "").await;
    assert!(!run.end.listen_next);
    assert!(run.end.succeeded);
}

const FILLER: &str = "Let me check.";

/// A searching run with `filler`, and speech when `speech` is given.
async fn run_searching(
    speech: impl FnOnce(&Log) -> Option<ScriptedSpeech>,
    recall: RecallMode,
    filler: Option<&'static str>,
    typed: &[u8],
) -> (Run, i64, Vec<rig_core::completion::CompletionRequest>) {
    let log = Log::default();
    let speech = speech(&log);
    let (writer, reader) = tokio::io::duplex(512);
    run_request(
        answer_turn(),
        &log,
        speech,
        reader,
        Some(writer),
        typed,
        Taps::none(),
        (recall, filler),
    )
    .await
}

#[tokio::test]
async fn a_search_shows_the_filler_before_any_text_and_speaks_it_first() {
    let (run, archived_turns, requests) = run_searching(
        |log| Some(speech(log, Behaviour::SpeaksUntilTextEnds)),
        RecallMode::Search,
        Some(FILLER),
        b"",
    )
    .await;

    let events = events(&run);
    let notice = position(&events, &format!("notice:{FILLER}"));
    let first_delta = events
        .iter()
        .position(|event| event.starts_with("delta:"))
        .expect("the answer streamed");
    assert!(notice < first_delta, "{events:?}");
    let chunks: Vec<&String> = events
        .iter()
        .filter(|event| event.starts_with("chunk:"))
        .collect();
    assert_eq!(chunks[0], &format!("chunk:{FILLER}"), "{events:?}");
    assert!(
        chunks.len() >= 2,
        "the answer follows the filler: {events:?}"
    );
    assert!(chunks[1..].iter().all(|chunk| !chunk.contains(FILLER)));
    assert!(run.end.succeeded);
    assert_eq!(archived_turns, 1);
    // The model saw only the question.
    assert_eq!(requests.len(), 1);
    assert!(!format!("{:?}", requests[0].chat_history).contains(FILLER));
}

#[tokio::test]
async fn a_skipped_run_shows_no_filler_and_speaks_the_answer_first() {
    let (run, _, _) = run_searching(
        |log| Some(speech(log, Behaviour::SpeaksUntilTextEnds)),
        RecallMode::Skip,
        None,
        b"",
    )
    .await;

    let events = events(&run);
    assert!(!events.iter().any(|event| event.starts_with("notice:")));
    let first_chunk = events
        .iter()
        .find(|event| event.starts_with("chunk:"))
        .expect("the answer was spoken");
    assert_eq!(first_chunk, &format!("chunk:{FIRST_SENTENCE}"));
}

#[tokio::test]
async fn with_speech_off_a_search_still_shows_the_filler() {
    let (run, archived_turns, requests) =
        run_searching(|_| None, RecallMode::Search, Some(FILLER), b"").await;

    let events = events(&run);
    assert!(position(&events, &format!("notice:{FILLER}")) < position(&events, "completed"));
    assert!(!events.iter().any(|event| event.starts_with("chunk:")));
    assert_eq!(archived_turns, 1);
    assert!(!format!("{:?}", requests[0].chat_history).contains(FILLER));
}

#[tokio::test]
async fn stop_during_a_searching_run_cancels_speech_and_the_answer_still_finishes() {
    let (run, archived_turns, _) = run_searching(
        |log| Some(speech(log, Behaviour::NeverFinishes)),
        RecallMode::Search,
        Some(FILLER),
        b"/stop\n",
    )
    .await;

    assert!(matches!(wait(&run), SpeechWait::Stopped { .. }));
    assert_eq!(run.speech.as_ref().expect("speech").cancel_calls, 1);
    let events = events(&run);
    assert!(events.contains(&format!("notice:{FILLER}")));
    assert!(events.contains(&"completed".to_owned()), "{events:?}");
    assert!(run.end.succeeded);
    assert_eq!(archived_turns, 1);
}

/// Answers real runs from a script and never answers a cache warm-up, counting
/// the warm-ups that started and the ones that were dropped while waiting.
#[derive(Clone)]
struct WarmupCountingModel {
    answers: MockCompletionModel,
    started: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}

struct CountsDrop(Arc<AtomicUsize>);

impl Drop for CountsDrop {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl CompletionModel for WarmupCountingModel {
    async fn completion(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, CompletionError> {
        unreachable!("Assistant uses the streaming path")
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse, CompletionError> {
        // Only a warm-up asks for a single token.
        if request.max_tokens == Some(1) {
            self.started.fetch_add(1, Ordering::SeqCst);
            let _waiting = CountsDrop(Arc::clone(&self.dropped));
            return std::future::pending().await;
        }
        self.answers.stream(request).await
    }
}

/// Lets the spawned warm-up task run, or be cancelled, on the test's one thread.
async fn let_background_tasks_run() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn a_turn_that_trims_the_history_starts_a_warm_up_and_the_next_model_run_stops_it() {
    // `MAX_EXCHANGES` in `features/conversation/history.rs`.
    const UPPER: usize = 12;
    let trimming_turn = UPPER + 1;
    let model = WarmupCountingModel {
        answers: MockCompletionModel::from_stream_turns(
            (0..trimming_turn + 2).map(|_| answer_turn()),
        ),
        started: Arc::default(),
        dropped: Arc::default(),
    };
    let (started, dropped) = (Arc::clone(&model.started), Arc::clone(&model.dropped));
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
        )
        .build(),
    );
    let archive_dir = tempfile::tempdir().expect("temporary archive directory should open");
    let mut session = ConversationSession::new(archive_dir.path().join("conversation.sqlite3"));
    assert!(session.reset().await.is_ok());
    let log = Log::default();
    let (_writer, reader) = tokio::io::duplex(64);
    let mut input = InputReader::new(BufReader::new(reader));
    let mut presenter = LoggingPresenter {
        log: Rc::clone(&log),
        fail_on_delta: false,
    };
    let mut warmup = WarmupTask::default();
    let mut taps = Taps::none();
    let mut ask = async |warmup: &mut WarmupTask| {
        let end = run_prompt(
            &assistant,
            &mut session,
            None,
            &mut presenter,
            &mut input,
            &mut taps,
            None::<&mut ScriptedSpeech>,
            warmup,
            PromptRequest {
                text: "a question",
                recall: RecallMode::Skip,
                filler: None,
            },
        )
        .await
        .expect("the prompt should run");
        assert!(end.succeeded);
        let_background_tasks_run().await;
    };

    // The last turn that does not trim starts nothing.
    for _ in 0..UPPER {
        ask(&mut warmup).await;
    }
    assert_eq!(started.load(Ordering::SeqCst), 0);

    // The first that does starts one warm-up, and it keeps running after the turn.
    ask(&mut warmup).await;
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);

    // The next model run stops it, and starts none of its own.
    ask(&mut warmup).await;
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_warm_up_still_running_when_the_loop_ends_is_stopped() {
    let model = WarmupCountingModel {
        answers: MockCompletionModel::from_stream_turns([answer_turn()]),
        started: Arc::default(),
        dropped: Arc::default(),
    };
    let (started, dropped) = (Arc::clone(&model.started), Arc::clone(&model.dropped));
    let assistant = Assistant::new(AgentBuilder::new(model).build());
    let mut warmup = WarmupTask::default();

    warmup.start(
        &assistant,
        CacheWarmup::new(Vec::new(), Message::user("a question")),
    );
    let_background_tasks_run().await;
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);

    drop(warmup);
    let_background_tasks_run().await;
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}
