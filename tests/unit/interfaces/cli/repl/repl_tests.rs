use std::{
    cell::RefCell,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    pin::pin,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use rig_agent::AgentBuilder;
use rig_core::{
    completion::Usage,
    message::{AssistantContent, Message, UserContent},
    test_utils::{MockCompletionModel, MockStreamEvent},
};
use serde_json::json;
use tempfile::TempDir;
use tokio::{
    io::{AsyncWriteExt, BufReader, DuplexStream},
    sync::{Notify, Semaphore, mpsc, oneshot},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use crate::{
    clients::speech_to_text::SpeechToText,
    config::SpeechToTextConfig,
    features::{
        assistant::Assistant,
        conversation::ConversationSession,
        listening::{Listener, Recorder},
        state::AssistantState,
    },
    tools::system_status_tool::SystemStatusTool,
};

use super::super::presentation::{AssistantPresenter, TerminalPresenter};
use super::{listen::NOT_CONFIGURED, run_input_loop, speech::SpeechControl};

type CapturedAnswers = Arc<Mutex<Vec<String>>>;

fn answer_capture() -> (CapturedAnswers, Arc<Semaphore>, impl FnMut(&str)) {
    let answers = Arc::new(Mutex::new(Vec::new()));
    let presented = Arc::new(Semaphore::new(0));
    let captured_answers = Arc::clone(&answers);
    let presented_signal = Arc::clone(&presented);
    let presenter = move |answer: &str| {
        captured_answers
            .lock()
            .expect("captured answer lock should be available")
            .push(answer.to_owned());
        presented_signal.add_permits(1);
    };
    (answers, presented, presenter)
}

#[tokio::test]
async fn piped_prompts_run_serially_and_failed_stream_closes_its_visible_line() {
    let model = MockCompletionModel::from_stream_turns(vec![
        vec![
            MockStreamEvent::text("partial response"),
            MockStreamEvent::error("injected midstream failure"),
        ],
        vec![
            MockStreamEvent::text("second answer"),
            MockStreamEvent::final_response(Usage::new()),
        ],
    ]);
    let model_handle = model.clone();
    let assistant = Assistant::new(AgentBuilder::new(model).tool(SystemStatusTool).build());
    let archive_dir = tempfile::tempdir().expect("temporary archive directory should open");
    let mut session = ConversationSession::new(archive_dir.path().join("conversation.sqlite3"));
    assert!(session.reset().await.is_ok());

    let mut speech = None::<FailingSpeech>;
    let mut input = BufReader::new(std::io::Cursor::new(
        b"first prompt\nsecond prompt\n/exit\n",
    ));
    let mut presenter = TerminalPresenter::with_writer(Vec::new());
    run_input_loop(
        &assistant,
        &mut session,
        None,
        &mut speech,
        &mut input,
        &mut presenter,
        None,
    )
    .await
    .expect("piped input should complete after both serial model runs");

    let output = String::from_utf8(presenter.into_writer()).expect("terminal output is UTF-8");
    assert!(output.contains("Jarvis: partial response [incomplete response]\n"));
    assert!(output.contains("You> [thinking]\nJarvis: second answer\n[idle]\n"));
    assert_eq!(output.matches("second answer").count(), 1);
    assert_eq!(model_handle.request_count(), 2);
}

struct FailingSpeech {
    calls: Vec<String>,
    started: Option<oneshot::Sender<()>>,
    queue_on_start: bool,
    release: oneshot::Receiver<()>,
    disabled: Arc<AtomicBool>,
    disabled_signal: Arc<Semaphore>,
    cancel_calls: usize,
    shutdown_calls: usize,
}

impl SpeechControl for FailingSpeech {
    type Error = std::io::Error;

    fn is_disabled(&self) -> bool {
        self.disabled.load(Ordering::SeqCst)
    }

    async fn speak(
        &mut self,
        mut chunks: mpsc::Receiver<String>,
        playback_started: oneshot::Sender<()>,
    ) -> Result<(), Self::Error> {
        let mut playback_started = Some(playback_started);
        while let Some(chunk) = chunks.recv().await {
            self.calls.push(chunk);
            // Speech runs alongside the text stream, so the first chunk is what tells
            // the test that speech has begun.
            if let Some(started) = self.started.take() {
                started
                    .send(())
                    .expect("test should still be waiting for speech");
            }
            if self.queue_on_start
                && let Some(playback_started) = playback_started.take()
            {
                playback_started
                    .send(())
                    .expect("App should still be waiting for playback");
            }
        }
        (&mut self.release)
            .await
            .map_err(|_| std::io::Error::other("test release sender dropped"))?;
        drop(playback_started);
        self.disabled.store(true, Ordering::SeqCst);
        self.disabled_signal.add_permits(1);
        Err(std::io::Error::other("injected playback failure"))
    }

    async fn cancel(&mut self) -> Result<(), Self::Error> {
        self.cancel_calls += 1;
        self.disabled.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), Self::Error> {
        self.shutdown_calls += 1;
        Ok(())
    }
}

fn user_message(text: &str) -> Message {
    Message::User {
        content: vec![UserContent::text(text)],
    }
}

#[tokio::test]
async fn loop_speaks_only_successful_final_and_keeps_partial_input_after_speech_failure() {
    let model = MockCompletionModel::from_stream_turns([
        [
            MockStreamEvent::text("visible final one"),
            MockStreamEvent::final_response(Usage::new()),
        ],
        [
            MockStreamEvent::text("visible final two"),
            MockStreamEvent::final_response(Usage::new()),
        ],
    ]);
    let model_handle = model.clone();
    let assistant = Assistant::new(AgentBuilder::new(model).tool(SystemStatusTool).build());
    let archive_dir = tempfile::tempdir().expect("temporary archive directory should open");
    let mut session = ConversationSession::new(archive_dir.path().join("conversation.sqlite3"));
    assert!(session.reset().await.is_ok());

    let (started_tx, started_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let disabled = Arc::new(AtomicBool::new(false));
    let disabled_signal = Arc::new(Semaphore::new(0));
    let mut speech = Some(FailingSpeech {
        calls: Vec::new(),
        started: Some(started_tx),
        queue_on_start: false,
        release: release_rx,
        disabled: Arc::clone(&disabled),
        disabled_signal: Arc::clone(&disabled_signal),
        cancel_calls: 0,
        shutdown_calls: 0,
    });

    let (mut writer, reader) = tokio::io::duplex(512);
    let mut input = BufReader::new(reader);
    writer
        .write_all(b"first prompt\npartial")
        .await
        .expect("initial prompts should fit in duplex buffer");

    let (answers, _presented_signal, mut present_answer) = answer_capture();
    let mut loop_future = Box::pin(run_input_loop(
        &assistant,
        &mut session,
        None,
        &mut speech,
        &mut input,
        &mut present_answer,
        None,
    ));
    tokio::select! {
        result = &mut loop_future => panic!("loop should wait for speech release: {result:?}"),
        result = started_rx => result.expect("speech should start after the first final answer"),
    }
    assert_eq!(
        *answers
            .lock()
            .expect("captured answers should be available"),
        ["visible final one"],
        "the canonical final is visible before speech starts"
    );

    release_tx
        .send(())
        .expect("speech operation should be waiting");
    let _disabled_permit = tokio::select! {
        permit = disabled_signal.acquire() => {
            permit.expect("speech failure should notify the test")
        }
        result = &mut loop_future => {
            panic!("loop should preserve the partial line after speech failure: {result:?}")
        }
    };
    assert!(disabled.load(Ordering::SeqCst));
    writer
        .write_all(b" prompt\n/stop\n/exit\n")
        .await
        .expect("remaining prompts should fit in duplex buffer");

    (&mut loop_future)
        .await
        .expect("the input loop should finish after /exit");
    drop(loop_future);

    let requests = model_handle.requests();
    assert_eq!(model_handle.request_count(), 2);
    assert_eq!(requests[1].chat_history.len(), 3);
    assert_eq!(requests[1].chat_history[0], user_message("first prompt"));
    assert_eq!(
        requests[1].chat_history[1],
        Message::Assistant {
            id: None,
            content: vec![AssistantContent::text("visible final one")],
        }
    );
    assert_eq!(requests[1].chat_history[2], user_message("partial prompt"));

    let speech = speech.expect("configured speech should remain owned through shutdown");
    assert_eq!(speech.calls, ["visible final one"]);
    assert_eq!(speech.cancel_calls, 0);
    assert_eq!(speech.shutdown_calls, 1);
    assert_eq!(
        *answers
            .lock()
            .expect("captured answers should be available"),
        ["visible final one", "visible final two"]
    );
}

#[tokio::test]
async fn queued_playback_presents_once_and_preserves_partial_input() {
    let model = MockCompletionModel::from_stream_turns([
        [
            MockStreamEvent::text("visible final one"),
            MockStreamEvent::final_response(Usage::new()),
        ],
        [
            MockStreamEvent::text("visible final two"),
            MockStreamEvent::final_response(Usage::new()),
        ],
    ]);
    let model_handle = model.clone();
    let assistant = Assistant::new(AgentBuilder::new(model).tool(SystemStatusTool).build());
    let archive_dir = tempfile::tempdir().expect("temporary archive directory should open");
    let mut session = ConversationSession::new(archive_dir.path().join("conversation.sqlite3"));
    assert!(session.reset().await.is_ok());

    let (started_tx, started_rx) = oneshot::channel();
    let (_release_tx, release_rx) = oneshot::channel();
    let disabled = Arc::new(AtomicBool::new(false));
    let disabled_signal = Arc::new(Semaphore::new(0));
    let mut speech = Some(FailingSpeech {
        calls: Vec::new(),
        started: Some(started_tx),
        queue_on_start: true,
        release: release_rx,
        disabled: Arc::clone(&disabled),
        disabled_signal: Arc::clone(&disabled_signal),
        cancel_calls: 0,
        shutdown_calls: 0,
    });

    let (mut writer, reader) = tokio::io::duplex(512);
    let mut input = BufReader::new(reader);
    writer
        .write_all(b"first prompt\npartial")
        .await
        .expect("initial prompt and partial line should fit in duplex buffer");

    let (answers, presented_signal, mut present_answer) = answer_capture();
    let mut loop_future = Box::pin(run_input_loop(
        &assistant,
        &mut session,
        None,
        &mut speech,
        &mut input,
        &mut present_answer,
        None,
    ));
    tokio::select! {
        result = &mut loop_future => panic!("loop should wait while queued speech is active: {result:?}"),
        result = started_rx => result.expect("speech should start after the first final answer"),
    }
    let _presented = tokio::select! {
        permit = presented_signal.acquire() => permit.expect("playback start should present the answer"),
        result = &mut loop_future => panic!("loop should wait for input while speech is active: {result:?}"),
    };
    assert_eq!(
        *answers
            .lock()
            .expect("captured answers should be available"),
        ["visible final one"]
    );

    writer
        .write_all(b" prompt\n/exit\n")
        .await
        .expect("completed partial prompt should fit in duplex buffer");
    (&mut loop_future)
        .await
        .expect("the preserved prompt should run before /exit");
    drop(loop_future);

    let requests = model_handle.requests();
    assert_eq!(model_handle.request_count(), 2);
    assert_eq!(requests[1].chat_history.len(), 3);
    assert_eq!(requests[1].chat_history[0], user_message("first prompt"));
    assert_eq!(
        requests[1].chat_history[1],
        Message::Assistant {
            id: None,
            content: vec![AssistantContent::text("visible final one")],
        }
    );
    assert_eq!(requests[1].chat_history[2], user_message("partial prompt"));

    let speech = speech.expect("configured speech should remain owned through shutdown");
    assert_eq!(speech.calls, ["visible final one"]);
    assert_eq!(speech.cancel_calls, 1);
    assert_eq!(speech.shutdown_calls, 1);
    assert!(disabled.load(Ordering::SeqCst));
    assert_eq!(
        *answers
            .lock()
            .expect("captured answers should be available"),
        ["visible final one", "visible final two"]
    );
}

#[tokio::test]
async fn cancellation_before_playback_presents_the_answer_once() {
    let model = MockCompletionModel::from_stream_turns([[
        MockStreamEvent::text("visible final"),
        MockStreamEvent::final_response(Usage::new()),
    ]]);
    let assistant = Assistant::new(AgentBuilder::new(model).tool(SystemStatusTool).build());
    let archive_dir = tempfile::tempdir().expect("temporary archive directory should open");
    let mut session = ConversationSession::new(archive_dir.path().join("conversation.sqlite3"));
    assert!(session.reset().await.is_ok());

    let (started_tx, started_rx) = oneshot::channel();
    let (_release_tx, release_rx) = oneshot::channel();
    let disabled = Arc::new(AtomicBool::new(false));
    let disabled_signal = Arc::new(Semaphore::new(0));
    let mut speech = Some(FailingSpeech {
        calls: Vec::new(),
        started: Some(started_tx),
        queue_on_start: false,
        release: release_rx,
        disabled: Arc::clone(&disabled),
        disabled_signal: Arc::clone(&disabled_signal),
        cancel_calls: 0,
        shutdown_calls: 0,
    });

    let (mut writer, reader) = tokio::io::duplex(512);
    let mut input = BufReader::new(reader);
    writer
        .write_all(b"first prompt\n")
        .await
        .expect("prompt should fit in duplex buffer");

    let (answers, _presented_signal, mut present_answer) = answer_capture();
    let mut loop_future = Box::pin(run_input_loop(
        &assistant,
        &mut session,
        None,
        &mut speech,
        &mut input,
        &mut present_answer,
        None,
    ));
    tokio::select! {
        result = &mut loop_future => panic!("loop should wait while speech is preparing: {result:?}"),
        result = started_rx => result.expect("speech should start after the final answer"),
    }
    assert_eq!(
        *answers
            .lock()
            .expect("captured answers should be available"),
        ["visible final"],
        "the streamed final is visible before playback is cancelled"
    );

    writer
        .write_all(b"/stop\n/exit\n")
        .await
        .expect("stop command should fit in duplex buffer");
    (&mut loop_future)
        .await
        .expect("the loop should stop speech and then exit");
    drop(loop_future);

    let speech = speech.expect("configured speech should remain owned through shutdown");
    assert_eq!(speech.calls, ["visible final"]);
    assert_eq!(speech.cancel_calls, 1);
    assert_eq!(speech.shutdown_calls, 1);
    assert!(disabled.load(Ordering::SeqCst));
    assert_eq!(
        *answers
            .lock()
            .expect("captured answers should be available"),
        ["visible final"]
    );
}

// ---- /listen -----------------------------------------------------------------------
//
// The shared record-and-transcribe rules are tested in `features/listening`. These
// tests cover what the terminal adds: the input race (Enter, `/stop`, other text),
// the state lines, the transcript reaching the normal prompt path, and the notices.

#[derive(Debug, PartialEq)]
enum Shown {
    State(AssistantState),
    Notice(String),
    Answer(String),
}

/// A presenter that records what the user would see, in order, and wakes the driver.
#[derive(Clone, Default)]
struct Screen {
    shown: Rc<RefCell<Vec<Shown>>>,
    changed: Rc<Notify>,
}

impl Screen {
    fn show(&self, shown: Shown) {
        self.shown.borrow_mut().push(shown);
        self.changed.notify_one();
    }
}

impl AssistantPresenter for Screen {
    fn delta(&mut self, _text: &str) -> std::io::Result<()> {
        Ok(())
    }

    fn completed(&mut self, canonical_answer: &str) -> std::io::Result<()> {
        self.show(Shown::Answer(canonical_answer.to_owned()));
        Ok(())
    }

    fn failed(&mut self, safe_message: &str) -> std::io::Result<()> {
        self.show(Shown::Answer(safe_message.to_owned()));
        Ok(())
    }

    fn prompt(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    fn notice(&mut self, line: &str) -> std::io::Result<()> {
        self.show(Shown::Notice(line.to_owned()));
        Ok(())
    }

    fn state(&mut self, state: AssistantState) -> std::io::Result<()> {
        self.show(Shown::State(state));
        Ok(())
    }

    fn input_received(&mut self) {}
}

fn states(shown: &[Shown]) -> Vec<AssistantState> {
    shown
        .iter()
        .filter_map(|shown| match shown {
            Shown::State(state) => Some(*state),
            Shown::Notice(_) | Shown::Answer(_) => None,
        })
        .collect()
}

fn notices(shown: &[Shown]) -> Vec<&str> {
    shown
        .iter()
        .filter_map(|shown| match shown {
            Shown::Notice(line) => Some(line.as_str()),
            Shown::State(_) | Shown::Answer(_) => None,
        })
        .collect()
}

/// Plays the user: types lines and waits for the loop to reach a state.
struct Driver {
    writer: DuplexStream,
    screen: Screen,
    /// Waits look only at what was shown after the previous wait.
    cursor: usize,
}

impl Driver {
    async fn type_(&mut self, bytes: &[u8]) {
        self.writer
            .write_all(bytes)
            .await
            .expect("input should fit in the duplex buffer");
    }

    async fn wait_for(&mut self, state: AssistantState) {
        loop {
            let found = self.screen.shown.borrow()[self.cursor..]
                .iter()
                .position(|shown| *shown == Shown::State(state));
            if let Some(offset) = found {
                self.cursor += offset + 1;
                return;
            }
            self.screen.changed.notified().await;
        }
    }
}

impl Driver {
    async fn wait_for_notice_containing(&mut self, text: &str) {
        loop {
            let found = self.screen.shown.borrow()[self.cursor..]
                .iter()
                .position(|shown| matches!(shown, Shown::Notice(line) if line.contains(text)));
            if let Some(offset) = found {
                self.cursor += offset + 1;
                return;
            }
            self.screen.changed.notified().await;
        }
    }
}

struct Harness {
    assistant: Assistant,
    session: ConversationSession,
    _archive_dir: TempDir,
    model: MockCompletionModel,
    stt: MockServer,
    script_dir: TempDir,
    listener: Option<Listener>,
}

const REPLY: &str = "It is noon.";

/// No voice settings; the model has one answer ready.
async fn harness() -> Harness {
    let model = MockCompletionModel::from_stream_turns([[
        MockStreamEvent::text(REPLY),
        MockStreamEvent::final_response(Usage::new()),
    ]]);
    let assistant = Assistant::new(
        AgentBuilder::new(model.clone())
            .tool(SystemStatusTool)
            .build(),
    );
    let archive_dir = tempfile::tempdir().expect("temporary archive directory should open");
    let mut session = ConversationSession::new(archive_dir.path().join("conversation.sqlite3"));
    assert!(session.reset().await.is_ok());
    Harness {
        assistant,
        session,
        _archive_dir: archive_dir,
        model,
        stt: MockServer::start().await,
        script_dir: tempfile::tempdir().expect("temporary script directory should open"),
        listener: None,
    }
}

impl Harness {
    /// Gives the harness a listener whose recorder runs `script` and whose
    /// speech-to-text server answers every request with `reply`.
    async fn with_voice(mut self, script: &str, reply: ResponseTemplate) -> Self {
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .respond_with(reply)
            .mount(&self.stt)
            .await;
        let program = self.script_dir.path().join("fake-arecord");
        fs::write(&program, format!("#!/bin/sh\n{script}\n")).expect("script should write");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755))
            .expect("script is runnable");
        let config = SpeechToTextConfig {
            base_url: format!("{}/v1", self.stt.uri()),
            model: "whisper-test".to_owned(),
            capture_device: "test-device".to_owned(),
        };
        self.listener = Some(Listener::new(
            Recorder::new("test-device", program.to_str().expect("path is UTF-8")),
            SpeechToText::new(&config).expect("a localhost URL is allowed"),
        ));
        self
    }

    /// Created by a script that has emitted its audio and now waits to be stopped.
    fn emitted_marker(&self) -> PathBuf {
        self.script_dir.path().join("fake-arecord.emitted")
    }

    async fn stt_requests(&self) -> usize {
        self.stt
            .received_requests()
            .await
            .expect("recording is on")
            .len()
    }
}

/// A recorder that has captured a little audio and then waits to be stopped. `[listening]`
/// is only shown once audio arrives, so a recorder that sends nothing never shows it.
const AUDIBLE_UNTIL_STOPPED: &str = "head -c 4 /dev/zero\nexec sleep 30";

/// A recording that ends by itself, so no timing depends on when Enter arrives.
fn self_ending_script(bytes: usize) -> String {
    format!("head -c {bytes} /dev/zero")
}

fn transcript_reply(text: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "text": text }))
}

/// Runs the loop while `script` plays the user, then types `/exit` and returns
/// everything the user saw.
async fn run_with(
    harness: &mut Harness,
    speech: &mut Option<FailingSpeech>,
    script: impl AsyncFnOnce(&mut Driver),
) -> Vec<Shown> {
    let (writer, reader) = tokio::io::duplex(8192);
    let mut input = BufReader::new(reader);
    let screen = Screen::default();
    let mut presenter = screen.clone();
    let mut driver = Driver {
        writer,
        screen: screen.clone(),
        cursor: 0,
    };
    let mut loop_future = pin!(run_input_loop(
        &harness.assistant,
        &mut harness.session,
        None,
        speech,
        &mut input,
        &mut presenter,
        harness.listener.as_ref(),
    ));
    let user = async {
        script(&mut driver).await;
        driver.type_(b"/exit\n").await;
    };
    tokio::select! {
        result = &mut loop_future => panic!("the loop ended before the user was done: {result:?}"),
        () = user => {}
    }
    loop_future.await.expect("the loop should end at /exit");
    screen.shown.take()
}

async fn run_script(harness: &mut Harness, script: impl AsyncFnOnce(&mut Driver)) -> Vec<Shown> {
    run_with(harness, &mut None, script).await
}

/// Lets a script that touched `marker` after writing its audio be read before Enter
/// stops it: the recorder reads stdout in its own task, and a stop that is already
/// ready wins (`biased;`). This waits on the real clock, like the capture tests do.
async fn wait_until_emitted(marker: &Path) {
    for _ in 0..500 {
        if marker.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
}

fn assert_user_message_reached_model(harness: &Harness, text: &str) {
    let requests = harness.model.requests();
    assert_eq!(requests.len(), 1, "exactly one model request");
    assert_eq!(requests[0].chat_history.last(), Some(&user_message(text)));
}

#[tokio::test]
async fn enter_stops_the_recording_and_the_transcript_is_answered_like_typed_text() {
    let mut harness = harness()
        .await
        .with_voice(
            "head -c 20000 /dev/zero\ntouch \"$0.emitted\"\nexec sleep 30",
            transcript_reply("what time is it"),
        )
        .await;
    let marker = harness.emitted_marker();

    let shown = run_script(&mut harness, async |user| {
        user.type_(b"/listen\n").await;
        user.wait_for(AssistantState::Listening).await;
        wait_until_emitted(&marker).await;
        user.type_(b"\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [
            AssistantState::Listening,
            AssistantState::Transcribing,
            AssistantState::Thinking,
            AssistantState::Idle
        ]
    );
    assert_eq!(notices(&shown), ["You (voice)> what time is it"]);
    assert!(shown.contains(&Shown::Answer(REPLY.to_owned())));
    assert_user_message_reached_model(&harness, "what time is it");
    assert_eq!(harness.stt_requests().await, 1);
}

#[tokio::test]
async fn a_recording_that_ends_by_itself_is_transcribed_without_enter() {
    let mut harness = harness()
        .await
        .with_voice(
            &self_ending_script(20_000),
            transcript_reply("what time is it"),
        )
        .await;

    let shown = run_script(&mut harness, async |user| {
        user.type_(b"/listen\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [
            AssistantState::Listening,
            AssistantState::Transcribing,
            AssistantState::Thinking,
            AssistantState::Idle
        ]
    );
    assert_user_message_reached_model(&harness, "what time is it");
}

#[tokio::test]
async fn stop_cancels_the_recording_and_nothing_is_sent() {
    let mut harness = harness()
        .await
        .with_voice(AUDIBLE_UNTIL_STOPPED, transcript_reply("never asked"))
        .await;

    let shown = run_script(&mut harness, async |user| {
        user.type_(b"/listen\n").await;
        user.wait_for(AssistantState::Listening).await;
        user.type_(b"/stop\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [AssistantState::Listening, AssistantState::Idle]
    );
    assert_eq!(notices(&shown), ["Recording cancelled. Nothing was sent."]);
    assert_eq!(harness.stt_requests().await, 0);
    assert!(harness.model.requests().is_empty());
}

#[tokio::test]
async fn other_text_cancels_the_recording_and_is_handled_as_the_next_input() {
    let mut harness = harness()
        .await
        .with_voice(AUDIBLE_UNTIL_STOPPED, transcript_reply("never asked"))
        .await;

    let shown = run_script(&mut harness, async |user| {
        user.type_(b"/listen\n").await;
        user.wait_for(AssistantState::Listening).await;
        user.type_(b"something else\n").await;
        user.wait_for(AssistantState::Thinking).await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [
            AssistantState::Listening,
            AssistantState::Idle,
            AssistantState::Thinking,
            AssistantState::Idle
        ]
    );
    assert_eq!(harness.stt_requests().await, 0);
    assert_user_message_reached_model(&harness, "something else");
}

#[tokio::test]
async fn listening_is_shown_only_once_the_recorder_sends_audio() {
    let mut harness = harness().await;
    // The recorder sends nothing until the test creates the `.go` file.
    harness = harness
        .with_voice(
            "while [ ! -e \"$0.go\" ]; do sleep 0.02; done\nhead -c 4 /dev/zero\nexec sleep 30",
            transcript_reply("never asked"),
        )
        .await;
    let go = harness.script_dir.path().join("fake-arecord.go");

    let shown = run_script(&mut harness, async |user| {
        user.type_(b"/listen\n").await;
        // The recorder is running but silent: the microphone is not capturing yet.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !user
                .screen
                .shown
                .borrow()
                .contains(&Shown::State(AssistantState::Listening))
        );
        fs::write(&go, "").expect("marker should write");
        user.wait_for(AssistantState::Listening).await;
        user.type_(b"/stop\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [AssistantState::Listening, AssistantState::Idle]
    );
}

#[tokio::test]
async fn end_of_input_while_listening_ends_the_loop() {
    let mut harness = harness()
        .await
        .with_voice(AUDIBLE_UNTIL_STOPPED, transcript_reply("never asked"))
        .await;
    let (writer, reader) = tokio::io::duplex(64);
    let mut input = BufReader::new(reader);
    let screen = Screen::default();
    let mut presenter = screen.clone();
    let mut driver = Driver {
        writer,
        screen: screen.clone(),
        cursor: 0,
    };
    let user = async {
        driver.type_(b"/listen\n").await;
        driver.wait_for(AssistantState::Listening).await;
        drop(driver);
    };

    let mut speech = None::<FailingSpeech>;
    let (result, ()) = tokio::join!(
        run_input_loop(
            &harness.assistant,
            &mut harness.session,
            None,
            &mut speech,
            &mut input,
            &mut presenter,
            harness.listener.as_ref(),
        ),
        user
    );
    result.expect("end of input ends the loop");

    assert_eq!(
        states(&screen.shown.borrow()),
        [AssistantState::Listening, AssistantState::Idle]
    );
}

#[tokio::test]
async fn too_little_audio_or_no_words_is_didnt_catch_that_with_no_model_request() {
    // (recorder script, transcript text, states shown)
    let cases = [
        (
            self_ending_script(100),
            "never asked",
            vec![AssistantState::Listening, AssistantState::Idle],
        ),
        (
            self_ending_script(20_000),
            "   ",
            vec![
                AssistantState::Listening,
                AssistantState::Transcribing,
                AssistantState::Idle,
            ],
        ),
    ];
    for (script, text, expected_states) in cases {
        let mut harness = harness()
            .await
            .with_voice(&script, transcript_reply(text))
            .await;

        let shown = run_script(&mut harness, async |user| {
            user.type_(b"/listen\n").await;
            user.wait_for(AssistantState::Idle).await;
        })
        .await;

        assert_eq!(states(&shown), expected_states, "{script}");
        assert_eq!(
            notices(&shown),
            ["Didn't catch that. Nothing was sent."],
            "{script}"
        );
        assert!(harness.model.requests().is_empty(), "{script}");
    }
}

/// The clock is paused only once transcription starts: a paused clock would also jump
/// the recorder's own deadline while the child process starts.
#[tokio::test]
async fn a_speech_to_text_timeout_is_reported_and_the_loop_keeps_running() {
    let mut harness = harness()
        .await
        .with_voice(
            &self_ending_script(20_000),
            transcript_reply("late").set_delay(Duration::from_secs(21)),
        )
        .await;

    let shown = run_script(&mut harness, async |user| {
        user.type_(b"/listen\n").await;
        user.wait_for(AssistantState::Transcribing).await;
        tokio::time::pause();
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [
            AssistantState::Listening,
            AssistantState::Transcribing,
            AssistantState::Idle
        ]
    );
    assert_eq!(
        notices(&shown),
        ["Speech-to-text timed out; the server may still be working."]
    );
    assert!(harness.model.requests().is_empty());
}

#[tokio::test]
async fn stop_while_transcribing_drops_the_request_and_sends_nothing() {
    let mut harness = harness()
        .await
        .with_voice(
            &self_ending_script(20_000),
            transcript_reply("late").set_delay(Duration::from_secs(60)),
        )
        .await;

    let shown = run_script(&mut harness, async |user| {
        user.type_(b"/listen\n").await;
        user.wait_for(AssistantState::Transcribing).await;
        user.type_(b"/stop\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [
            AssistantState::Listening,
            AssistantState::Transcribing,
            AssistantState::Idle
        ]
    );
    assert_eq!(
        notices(&shown),
        [
            "Transcription cancelled. The speech-to-text server may still be working on it. Nothing was sent."
        ]
    );
    assert!(harness.model.requests().is_empty());
}

#[tokio::test]
async fn failures_are_shown_in_plain_words_and_the_loop_keeps_running() {
    // (recorder script, server reply, a phrase the notice must contain)
    let cases = [
        (
            "exit 3".to_owned(),
            transcript_reply("unused"),
            "check JARVIS_CAPTURE_DEVICE",
        ),
        (
            self_ending_script(20_000),
            ResponseTemplate::new(500),
            "returned 500",
        ),
    ];
    for (script, reply, phrase) in cases {
        let mut harness = harness().await.with_voice(&script, reply).await;

        let shown = run_script(&mut harness, async |user| {
            user.type_(b"/listen\n").await;
            user.wait_for(AssistantState::Idle).await;
        })
        .await;

        let notices = notices(&shown);
        assert_eq!(notices.len(), 1, "{script}: {notices:?}");
        assert!(notices[0].contains(phrase), "{script}: {notices:?}");
        assert_eq!(states(&shown).last(), Some(&AssistantState::Idle));
        assert!(harness.model.requests().is_empty(), "{script}");
    }
}

#[tokio::test]
async fn a_transcript_is_accepted_or_rejected_at_the_typed_input_cap() {
    // The transcript client owns this rule; this checks the wiring at its boundary.
    for (len, reaches_model) in [(2_048, true), (2_049, false)] {
        let text = "a".repeat(len);
        let mut harness = harness()
            .await
            .with_voice(&self_ending_script(20_000), transcript_reply(&text))
            .await;

        let shown = run_script(&mut harness, async |user| {
            user.type_(b"/listen\n").await;
            user.wait_for(AssistantState::Idle).await;
        })
        .await;

        assert_eq!(
            harness.model.requests().len(),
            usize::from(reaches_model),
            "{len}"
        );
        if reaches_model {
            assert_user_message_reached_model(&harness, &text);
        } else {
            let notices = notices(&shown);
            assert!(
                notices[0].contains("over the 2048 byte limit"),
                "{notices:?}"
            );
        }
    }
}

#[tokio::test]
async fn a_spoken_command_runs_as_if_typed_and_returns_to_idle() {
    let mut harness = harness()
        .await
        .with_voice(&self_ending_script(20_000), transcript_reply("/help"))
        .await;

    let shown = run_script(&mut harness, async |user| {
        user.type_(b"/listen\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [
            AssistantState::Listening,
            AssistantState::Transcribing,
            AssistantState::Idle
        ]
    );
    assert_eq!(notices(&shown), ["You (voice)> /help"]);
    assert!(harness.model.requests().is_empty());
}

#[tokio::test]
async fn listen_without_voice_settings_names_them_and_changes_nothing() {
    let mut harness = harness().await;

    let shown = run_script(&mut harness, async |user| {
        user.type_(b"/listen\n").await;
    })
    .await;

    assert!(states(&shown).is_empty());
    assert_eq!(notices(&shown), [NOT_CONFIGURED]);
    for setting in [
        "JARVIS_STT_BASE_URL",
        "JARVIS_STT_MODEL",
        "JARVIS_CAPTURE_DEVICE",
    ] {
        assert!(NOT_CONFIGURED.contains(setting), "{setting}");
    }
}

#[tokio::test]
async fn a_typed_prompt_reports_thinking_then_idle() {
    let mut harness = harness().await;

    let shown = run_script(&mut harness, async |user| {
        user.type_(b"hello\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [AssistantState::Thinking, AssistantState::Idle]
    );
    assert_user_message_reached_model(&harness, "hello");
}

#[tokio::test]
async fn listen_while_speech_is_playing_stops_the_speech_first() {
    let mut harness = harness()
        .await
        .with_voice(AUDIBLE_UNTIL_STOPPED, transcript_reply("never asked"))
        .await;
    let (_release_sender, release) = oneshot::channel();
    let mut speech = Some(FailingSpeech {
        calls: Vec::new(),
        started: None,
        queue_on_start: true,
        release,
        disabled: Arc::new(AtomicBool::new(false)),
        disabled_signal: Arc::new(Semaphore::new(0)),
        cancel_calls: 0,
        shutdown_calls: 0,
    });

    let shown = run_with(&mut harness, &mut speech, async |user| {
        user.type_(b"hello\n").await;
        user.wait_for(AssistantState::Speaking).await;
        user.type_(b"/listen\n").await;
        user.wait_for(AssistantState::Listening).await;
        user.type_(b"/stop\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [
            AssistantState::Thinking,
            AssistantState::Speaking,
            AssistantState::Listening,
            AssistantState::Idle
        ]
    );
    let speech = speech.expect("speech stays owned by the caller");
    assert_eq!(speech.cancel_calls, 1);
    assert_eq!(harness.stt_requests().await, 0);
}

fn speech_that_plays_until_released(release: oneshot::Receiver<()>) -> FailingSpeech {
    FailingSpeech {
        calls: Vec::new(),
        started: None,
        queue_on_start: true,
        release,
        disabled: Arc::new(AtomicBool::new(false)),
        disabled_signal: Arc::new(Semaphore::new(0)),
        cancel_calls: 0,
        shutdown_calls: 0,
    }
}

#[tokio::test]
async fn listen_typed_while_the_answer_streams_starts_listening_and_is_never_a_prompt() {
    let mut harness = harness()
        .await
        .with_voice(AUDIBLE_UNTIL_STOPPED, transcript_reply("never asked"))
        .await;
    let (_release_sender, release) = oneshot::channel();
    let mut speech = Some(speech_that_plays_until_released(release));

    let shown = run_with(&mut harness, &mut speech, async |user| {
        // `/listen` is already waiting when the answer starts to stream.
        user.type_(b"hello\n/listen\n").await;
        user.wait_for(AssistantState::Listening).await;
        user.type_(b"/stop\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    // The speech it interrupted is not followed by an `[idle]` the user did not cause.
    assert_eq!(
        states(&shown),
        [
            AssistantState::Thinking,
            AssistantState::Listening,
            AssistantState::Idle
        ]
    );
    assert_user_message_reached_model(&harness, "hello");
    assert_eq!(harness.stt_requests().await, 0);
}

#[tokio::test]
async fn a_command_typed_over_speech_that_reports_no_state_still_ends_in_idle() {
    let mut harness = harness().await;
    let (_release_sender, release) = oneshot::channel();
    let mut speech = Some(speech_that_plays_until_released(release));

    let shown = run_with(&mut harness, &mut speech, async |user| {
        user.type_(b"hello\n").await;
        user.wait_for(AssistantState::Speaking).await;
        user.type_(b"/help\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [
            AssistantState::Thinking,
            AssistantState::Speaking,
            AssistantState::Idle
        ]
    );
    assert_user_message_reached_model(&harness, "hello");
}

#[tokio::test]
async fn listen_typed_right_after_speech_completes_starts_listening() {
    let mut harness = harness()
        .await
        .with_voice(AUDIBLE_UNTIL_STOPPED, transcript_reply("never asked"))
        .await;
    let (release_sender, release) = oneshot::channel();
    let mut speech = Some(speech_that_plays_until_released(release));

    let shown = run_with(&mut harness, &mut speech, async |user| {
        user.type_(b"hello\n").await;
        user.wait_for(AssistantState::Speaking).await;
        release_sender.send(()).expect("speech is still waiting");
        // The speech diagnostics are the loop's first act once speech has completed.
        user.wait_for_notice_containing("time_to_first_audio").await;
        user.type_(b"/listen\n").await;
        user.wait_for(AssistantState::Listening).await;
        user.type_(b"/stop\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    // Speech had finished, so the answer's own `[idle]` is real and comes first.
    assert_eq!(
        states(&shown),
        [
            AssistantState::Thinking,
            AssistantState::Speaking,
            AssistantState::Idle,
            AssistantState::Listening,
            AssistantState::Idle
        ]
    );
    assert_user_message_reached_model(&harness, "hello");
    assert_eq!(harness.stt_requests().await, 0);
}
