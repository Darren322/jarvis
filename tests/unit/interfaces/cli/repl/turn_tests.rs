use std::{cell::RefCell, io, rc::Rc};

use rig_agent::AgentBuilder;
use rig_core::{
    completion::Usage,
    test_utils::{MockCompletionModel, MockStreamEvent},
};
use tokio::{
    io::{AsyncWriteExt, BufReader, DuplexStream},
    sync::{mpsc, oneshot},
};

use crate::{
    features::{assistant::Assistant, conversation::ConversationSession},
    interfaces::cli::{input::InputReader, presentation::AssistantPresenter},
    tools::system_status_tool::SystemStatusTool,
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

    fn input_received(&mut self) {}
}

enum Behaviour {
    /// Speaks every chunk and finishes when the text is complete.
    SpeaksUntilTextEnds,
    FailsOnFirstChunk,
    /// Plays forever once the text is complete.
    NeverFinishes,
    /// Once the text is complete, the user types `line` and playback never finishes.
    UserTypesAfterText {
        line: &'static str,
        writer: DuplexStream,
    },
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
        }
        self.log.borrow_mut().push("text complete".to_owned());
        match &mut self.behaviour {
            Behaviour::SpeaksUntilTextEnds | Behaviour::FailsOnFirstChunk => Ok(()),
            Behaviour::NeverFinishes => std::future::pending().await,
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
    mut writer: Option<DuplexStream>,
    typed: &[u8],
) -> Run {
    let assistant = Assistant::new(
        AgentBuilder::new(MockCompletionModel::from_stream_turns([model_turn]))
            .tool(SystemStatusTool)
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
        speech.as_mut(),
        "a question",
    )
    .await
    .expect("the prompt should run");
    drop(writer);
    Run {
        end,
        log: Rc::clone(log),
        speech,
    }
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
            format!("delta:{FIRST_SENTENCE} "),
            format!("delta:{SECOND_SENTENCE}"),
            "completed".to_owned(),
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
    assert_eq!(events.last().map(String::as_str), Some("prompt"));
    assert!(position(&events, "completed") < position(&events, "prompt"));
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
    assert_eq!(events.last().map(String::as_str), Some("prompt"));
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
        AgentBuilder::new(MockCompletionModel::from_stream_turns([answer_turn()]))
            .tool(SystemStatusTool)
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
        Some(&mut speech),
        "a question",
    )
    .await;

    assert!(result.is_err());
    assert_eq!(speech.cancel_calls, 1);
}
