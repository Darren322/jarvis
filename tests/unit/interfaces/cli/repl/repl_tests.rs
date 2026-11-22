use std::{
    cell::RefCell,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    pin::pin,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use rig_agent::{
    AgentBuilder,
    prelude::{CompletionError, CompletionModel},
};
use rig_core::{
    completion::{CompletionRequest, CompletionResponse, Usage},
    message::{AssistantContent, Message, UserContent},
    streaming::StreamingCompletionResponse,
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
    clients::{remote_embeddings::RemoteEmbeddings, speech_to_text::SpeechToText},
    config::{EmbeddingConfig, SpeechToTextConfig},
    features::{
        assistant::{Assistant, CacheWarmup},
        control::MemoryControl,
        conversation::ConversationSession,
        listening::{Listener, Recorder},
        memory::MemoryService,
        state::AssistantState,
    },
    storage::{ArchiveOutcome, ConversationArchive, MemoryEligibility},
};

use super::super::input::{Announcements, Taps};
use super::super::presentation::{AssistantPresenter, TerminalPresenter};
use super::{
    QUIET_BEFORE_NEW_CONVERSATION, listen::NOT_CONFIGURED, reset_conversation, run_input_loop,
    speech::SpeechControl, warmup::WarmupTask,
};
use crate::features::control::{Notice, Tap};

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
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
            crate::features::timers::channel().0,
        )
        .build(),
    );
    let archive_dir = tempfile::tempdir().expect("temporary archive directory should open");
    let mut session = ConversationSession::new(archive_dir.path().join("conversation.sqlite3"));
    assert!(session.reset().await.is_ok());

    let mut speech = None::<FailingSpeech>;
    let mut input = BufReader::new(std::io::Cursor::new(
        b"first prompt\nsecond prompt\n/exit\n",
    ));
    let mut presenter = TerminalPresenter::with_writer(Vec::new());
    let panels = crate::features::panel::channel().0;
    run_input_loop(
        &assistant,
        &mut session,
        None,
        &mut speech,
        &mut input,
        Taps::none(),
        no_announcements(),
        None,
        &mut presenter,
        None,
        &panels,
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
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
            crate::features::timers::channel().0,
        )
        .build(),
    );
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
    let panels = crate::features::panel::channel().0;
    let mut loop_future = Box::pin(run_input_loop(
        &assistant,
        &mut session,
        None,
        &mut speech,
        &mut input,
        Taps::none(),
        no_announcements(),
        None,
        &mut present_answer,
        None,
        &panels,
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
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
            crate::features::timers::channel().0,
        )
        .build(),
    );
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
    let panels = crate::features::panel::channel().0;
    let mut loop_future = Box::pin(run_input_loop(
        &assistant,
        &mut session,
        None,
        &mut speech,
        &mut input,
        Taps::none(),
        no_announcements(),
        None,
        &mut present_answer,
        None,
        &panels,
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
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
            crate::features::timers::channel().0,
        )
        .build(),
    );
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
    let panels = crate::features::panel::channel().0;
    let mut loop_future = Box::pin(run_input_loop(
        &assistant,
        &mut session,
        None,
        &mut speech,
        &mut input,
        Taps::none(),
        no_announcements(),
        None,
        &mut present_answer,
        None,
        &panels,
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

/// A background source that never says anything: its sender is already gone.
fn no_announcements() -> Announcements {
    Announcements::new(tokio::sync::mpsc::channel(1).1)
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
    panels: crate::features::panel::PanelHandle,
}

const REPLY: &str = "It is noon.";

/// No voice settings; the model has one answer ready.
async fn harness() -> Harness {
    harness_answering(vec![answer()]).await
}

/// One streamed model turn that answers with `REPLY`.
fn answer() -> Vec<MockStreamEvent> {
    vec![
        MockStreamEvent::text(REPLY),
        MockStreamEvent::final_response(Usage::new()),
    ]
}

/// No voice settings; the model plays `turns` in order, one per request.
async fn harness_answering(turns: Vec<Vec<MockStreamEvent>>) -> Harness {
    let model = MockCompletionModel::from_stream_turns(turns);
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model.clone()),
            crate::features::panel::channel().0,
            crate::features::timers::channel().0,
        )
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
        panels: crate::features::panel::channel().0,
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
    run_with_taps(harness, speech, Taps::none(), script).await
}

/// `run_with`, with a touch front end: the script taps through the sender it kept.
async fn run_with_taps(
    harness: &mut Harness,
    speech: &mut Option<FailingSpeech>,
    taps: Taps,
    script: impl AsyncFnOnce(&mut Driver),
) -> Vec<Shown> {
    run_with_sources(harness, speech, taps, no_announcements(), None, script).await
}

/// `run_with_taps`, with a background source of announcements and a start-up
/// warm-up as well.
async fn run_with_sources(
    harness: &mut Harness,
    speech: &mut Option<FailingSpeech>,
    taps: Taps,
    announcements: Announcements,
    startup_warmup: Option<CacheWarmup>,
    script: impl AsyncFnOnce(&mut Driver),
) -> Vec<Shown> {
    run_with_timers(
        harness,
        speech,
        taps,
        announcements,
        startup_warmup,
        None,
        script,
    )
    .await
}

/// `run_with_sources`, with a timer service as well (timers 06 and 08).
async fn run_with_timers(
    harness: &mut Harness,
    speech: &mut Option<FailingSpeech>,
    taps: Taps,
    announcements: Announcements,
    startup_warmup: Option<CacheWarmup>,
    timers: Option<crate::features::timers::TimerLink>,
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
        taps,
        announcements,
        startup_warmup,
        &mut presenter,
        harness.listener.as_ref(),
        &harness.panels,
        timers,
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
            Taps::none(),
            no_announcements(),
            None,
            &mut presenter,
            harness.listener.as_ref(),
            &harness.panels,
            None,
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

/// A tap queue as the orb interface makes it: one tap may wait.
fn tap_queue() -> (mpsc::Sender<Tap>, Taps) {
    let (sender, receiver) = mpsc::channel(1);
    (sender, Taps::new(receiver))
}

/// The recorder plays two seconds of speech, then silence, and never stops by
/// itself: only something else can end the recording.
fn speech_then_silence_script() -> String {
    let speech = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/features/listening/fixtures/speech_16k_mono_s16le.raw"
    );
    format!("cat '{speech}'\nhead -c 48000 /dev/zero\ntouch \"$0.emitted\"\nexec sleep 30")
}

fn archived_turns(harness: &Harness) -> i64 {
    let archive = harness._archive_dir.path().join("conversation.sqlite3");
    rusqlite::Connection::open(archive)
        .expect("the archive opens")
        .query_row("SELECT COUNT(*) FROM turns", [], |row| row.get(0))
        .expect("the turns can be counted")
}

#[tokio::test]
async fn a_tap_at_idle_listens_and_the_recording_ends_when_the_speaker_falls_silent() {
    let mut harness = harness()
        .await
        .with_voice(
            &speech_then_silence_script(),
            transcript_reply("what time is it"),
        )
        .await;
    let (tap, taps) = tap_queue();
    let started = std::time::Instant::now();

    let shown = run_with_taps(&mut harness, &mut None, taps, async |user| {
        tap.send(Tap).await.expect("the loop reads taps");
        // Nobody presses Enter or taps again: silence ends it.
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
    // Without the silence stop this recording would run to the 15 s cap.
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn the_typed_listen_does_not_end_on_silence() {
    let mut harness = harness()
        .await
        .with_voice(
            &speech_then_silence_script(),
            transcript_reply("what time is it"),
        )
        .await;
    let marker = harness.emitted_marker();

    let shown = run_script(&mut harness, async |user| {
        user.type_(b"/listen\n").await;
        user.wait_for(AssistantState::Listening).await;
        // The speech and the silence after it have been read, and it still records.
        wait_until_emitted(&marker).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(
            user.screen.shown.borrow().last(),
            Some(&Shown::State(AssistantState::Listening))
        );
        user.type_(b"/stop\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [AssistantState::Listening, AssistantState::Idle]
    );
    assert_eq!(harness.stt_requests().await, 0);
}

#[tokio::test]
async fn a_tap_while_listening_sends_what_was_heard() {
    let mut harness = harness()
        .await
        .with_voice(
            "head -c 20000 /dev/zero\ntouch \"$0.emitted\"\nexec sleep 30",
            transcript_reply("what time is it"),
        )
        .await;
    let marker = harness.emitted_marker();
    let (tap, taps) = tap_queue();

    let shown = run_with_taps(&mut harness, &mut None, taps, async |user| {
        tap.send(Tap).await.expect("the loop reads taps");
        user.wait_for(AssistantState::Listening).await;
        wait_until_emitted(&marker).await;
        tap.send(Tap).await.expect("the loop reads taps");
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
    assert_eq!(harness.stt_requests().await, 1);
}

#[tokio::test]
async fn a_tap_while_transcribing_is_ignored() {
    let mut harness = harness()
        .await
        .with_voice(
            &self_ending_script(20_000),
            transcript_reply("what time is it").set_delay(Duration::from_millis(400)),
        )
        .await;
    let (tap, taps) = tap_queue();

    let shown = run_with_taps(&mut harness, &mut None, taps, async |user| {
        tap.send(Tap).await.expect("the loop reads taps");
        user.wait_for(AssistantState::Transcribing).await;
        tap.send(Tap).await.expect("the loop reads taps");
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    // The tap neither cancelled the transcription nor started a second recording.
    assert_eq!(
        states(&shown),
        [
            AssistantState::Listening,
            AssistantState::Transcribing,
            AssistantState::Thinking,
            AssistantState::Idle
        ]
    );
    assert_eq!(harness.stt_requests().await, 1);
}

#[tokio::test]
async fn a_tap_while_thinking_drops_the_answer_unarchived_and_listens() {
    let mut harness = harness()
        .await
        .with_voice(
            &self_ending_script(20_000),
            transcript_reply("what time is it"),
        )
        .await;
    let (tap, taps) = tap_queue();

    let shown = run_with_taps(&mut harness, &mut None, taps, async |user| {
        // Both are waiting when the loop looks: the typed line is read first and
        // the tap arrives while its answer is being thought about.
        tap.try_send(Tap).expect("the queue is empty");
        user.type_(b"tell me a long story\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [
            AssistantState::Thinking,
            AssistantState::Listening,
            AssistantState::Transcribing,
            AssistantState::Thinking,
            AssistantState::Idle
        ]
    );
    assert_eq!(
        shown
            .iter()
            .filter(|shown| matches!(shown, Shown::Answer(_)))
            .collect::<Vec<_>>(),
        [
            &Shown::Answer("Answer dropped.".to_owned()),
            &Shown::Answer(REPLY.to_owned())
        ]
    );
    // The dropped prompt is in neither the history nor the archive: the one model
    // request carries only the spoken question, and one turn was stored.
    let requests = harness.model.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].chat_history.iter().collect::<Vec<_>>(),
        [&user_message("what time is it")]
    );
    assert_eq!(archived_turns(&harness), 1);
}

#[tokio::test]
async fn a_tap_while_speaking_stops_the_voice_keeps_the_answer_and_listens() {
    let mut harness = harness()
        .await
        .with_voice(AUDIBLE_UNTIL_STOPPED, transcript_reply("never asked"))
        .await;
    let (_release_sender, release) = oneshot::channel();
    let mut speech = Some(speech_that_plays_until_released(release));
    let (tap, taps) = tap_queue();

    let shown = run_with_taps(&mut harness, &mut speech, taps, async |user| {
        user.type_(b"hello\n").await;
        user.wait_for(AssistantState::Speaking).await;
        // The scripted answer is one short text, complete before its speech is.
        while !user
            .screen
            .shown
            .borrow()
            .contains(&Shown::Answer(REPLY.to_owned()))
        {
            user.screen.changed.notified().await;
        }
        tap.send(Tap).await.expect("the loop reads taps");
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
    // The finished answer was kept: shown, and stored as a turn.
    assert!(shown.contains(&Shown::Answer(REPLY.to_owned())));
    assert_eq!(archived_turns(&harness), 1);
    assert_eq!(harness.stt_requests().await, 0);
}

#[tokio::test]
async fn a_tap_without_voice_settings_names_them_and_stays_idle() {
    let mut harness = harness().await;
    let (tap, taps) = tap_queue();

    let shown = run_with_taps(&mut harness, &mut None, taps, async |user| {
        tap.send(Tap).await.expect("the loop reads taps");
        user.wait_for_notice_containing("Voice input is not configured")
            .await;
    })
    .await;

    assert_eq!(notices(&shown), [NOT_CONFIGURED]);
    assert!(harness.model.requests().is_empty());
}

#[tokio::test]
async fn only_a_trigger_prompt_or_slash_recall_searches_memory() {
    // The embedding server refuses, so recall is keyword-only; that is enough to
    // find "Miso" and to count the searches (each one asks the server once).
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let embeddings = RemoteEmbeddings::new(&EmbeddingConfig {
        base_url: format!("{}/v1", server.uri()),
        model: "bge-test".to_owned(),
    })
    .expect("a localhost URL is allowed");
    let model = MockCompletionModel::from_stream_turns((0..3).map(|_| {
        vec![
            MockStreamEvent::text("an answer"),
            MockStreamEvent::final_response(Usage::new()),
        ]
    }));
    let model_handle = model.clone();
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
            crate::features::timers::channel().0,
        )
        .build(),
    );
    let archive_dir = tempfile::tempdir().expect("temporary archive directory should open");
    let database = archive_dir.path().join("conversation.sqlite3");
    let seed = ConversationArchive::open(database.clone())
        .await
        .expect("archive opens");
    seed.append_turn(
        "Remember my cat is called Miso.",
        ArchiveOutcome::Completed("Noted, master.".to_owned()),
        MemoryEligibility::Eligible,
        &[],
    )
    .await
    .expect("remembered turn archives");
    // An answer that recalled a memory keeps it in the history, so a later search is
    // not shown it again; the third prompt below therefore needs a memory of its own.
    seed.append_turn(
        "Remember my dog is called Rex.",
        ArchiveOutcome::Completed("Noted, master.".to_owned()),
        MemoryEligibility::Eligible,
        &[],
    )
    .await
    .expect("remembered turn archives");
    drop(seed);
    let mut session = ConversationSession::new(database);
    assert!(session.reset().await.is_ok());
    let memory = MemoryService::open(
        session.memory_repository().expect("archive is open"),
        embeddings,
    );
    memory.wait_until_embedder_idle().await;

    let mut speech = None::<FailingSpeech>;
    let mut input = BufReader::new(std::io::Cursor::new(
        b"What is my cat called?\n/recall What is my cat called?\nDo you remember my dog called?\n/exit\n",
    ));
    let mut presenter = TerminalPresenter::with_writer(Vec::new());
    let panels = crate::features::panel::channel().0;
    run_input_loop(
        &assistant,
        &mut session,
        Some(&memory),
        &mut speech,
        &mut input,
        Taps::none(),
        no_announcements(),
        None,
        &mut presenter,
        None,
        &panels,
        None,
    )
    .await
    .expect("piped input should complete");

    let requests = model_handle.requests();
    assert_eq!(requests.len(), 3);
    let mentions =
        |index: usize, name: &str| format!("{:?}", requests[index].chat_history).contains(name);
    // A plain question finds nothing in memory, though a matching memory exists.
    assert!(!mentions(0, "Miso"));
    assert_eq!(
        requests[0].chat_history,
        vec![user_message("What is my cat called?")]
    );
    // `/recall` searches, and the question reaches the model without the command.
    assert!(mentions(1, "Miso"));
    assert_eq!(
        requests[1].chat_history.last(),
        Some(&user_message("What is my cat called?"))
    );
    assert!(!format!("{:?}", requests[1].chat_history).contains("/recall"));
    // So does a prompt that asks Jarvis to remember.
    assert!(mentions(2, "Rex"));
    // Only those two runs asked the embedding server anything.
    let embedding_requests = server
        .received_requests()
        .await
        .expect("request recording is on")
        .iter()
        .filter(|request| request.url.path() == "/v1/embeddings")
        .count();
    assert_eq!(embedding_requests, 2);
}

#[tokio::test]
async fn filler_lines_rotate_across_searches_and_a_plain_prompt_does_not_advance_them() {
    let model = MockCompletionModel::from_stream_turns((0..5).map(|_| {
        vec![
            MockStreamEvent::text("an answer"),
            MockStreamEvent::final_response(Usage::new()),
        ]
    }));
    let model_handle = model.clone();
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
            crate::features::timers::channel().0,
        )
        .build(),
    );
    let archive_dir = tempfile::tempdir().expect("temporary archive directory should open");
    let mut session = ConversationSession::new(archive_dir.path().join("conversation.sqlite3"));
    assert!(session.reset().await.is_ok());
    let mut speech = None::<FailingSpeech>;
    let mut input = BufReader::new(std::io::Cursor::new(
        b"/recall one\n/recall two\njust chatting\n/recall three\n/recall four\n/exit\n",
    ));
    let screen = Screen::default();
    let mut presenter = screen.clone();
    let panels = crate::features::panel::channel().0;
    run_input_loop(
        &assistant,
        &mut session,
        None,
        &mut speech,
        &mut input,
        Taps::none(),
        no_announcements(),
        None,
        &mut presenter,
        None,
        &panels,
        None,
    )
    .await
    .expect("piped input should complete");

    let shown = screen.shown.take();
    assert_eq!(
        notices(&shown),
        [
            "Let me check.",
            "One moment.",
            "Let me look that up.",
            "Let me check."
        ]
    );
    // The plain prompt came third and showed nothing between the second and third lines.
    let answers_before_third = shown
        .iter()
        .take_while(|shown| **shown != Shown::Notice("Let me look that up.".to_owned()))
        .filter(|shown| matches!(shown, Shown::Answer(_)))
        .count();
    assert_eq!(answers_before_third, 3);
    // No filler reached the model or the history it was sent.
    let requests = model_handle.requests();
    assert_eq!(requests.len(), 5);
    for request in &requests {
        let history = format!("{:?}", request.chat_history);
        for line in ["Let me check.", "One moment.", "Let me look that up."] {
            assert!(!history.contains(line), "{line:?} reached the model");
        }
    }
    // Nor was it archived.
    drop(session);
    let archive = archive_dir.path().join("conversation.sqlite3");
    let connection = rusqlite::Connection::open(archive).expect("the archive opens");
    let archived_fillers: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM turns WHERE user_text LIKE '%Let me%' OR user_text LIKE '%One moment%' \
             OR assistant_text LIKE '%Let me%' OR assistant_text LIKE '%One moment%'",
            [],
            |row| row.get(0),
        )
        .expect("the turns can be counted");
    assert_eq!(archived_fillers, 0);
}

// ---- the "speak now" cue ---------------------------------------------------------------
//
// The cue is played before the recorder starts, so the recording never contains it. The
// recorder is a script whose first action is to create a marker file, so these tests can tell
// when it started.

/// One call of `play_listening_cue`.
struct CueCall {
    at: Instant,
    /// Whether the recorder had already started when the cue was asked for.
    recorder_started: bool,
}

/// A speech output that only records the listening cue it is asked for.
struct CueSpeech {
    cue_length: Duration,
    disabled: bool,
    recorder_marker: PathBuf,
    calls: Arc<Mutex<Vec<CueCall>>>,
}

impl CueSpeech {
    fn new(cue_length: Duration, recorder_marker: PathBuf) -> (Self, Arc<Mutex<Vec<CueCall>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let speech = Self {
            cue_length,
            disabled: false,
            recorder_marker,
            calls: Arc::clone(&calls),
        };
        (speech, calls)
    }
}

impl SpeechControl for CueSpeech {
    type Error = std::io::Error;

    fn is_disabled(&self) -> bool {
        self.disabled
    }

    async fn speak(
        &mut self,
        _chunks: mpsc::Receiver<String>,
        _playback_started: oneshot::Sender<()>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn cancel(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn play_listening_cue(&mut self) -> Duration {
        self.calls
            .lock()
            .expect("the call log lock should be available")
            .push(CueCall {
                at: Instant::now(),
                recorder_started: self.recorder_marker.exists(),
            });
        self.cue_length
    }
}

/// `run_with_taps` for the cue double, which the shared runner's `FailingSpeech` type rules out.
async fn run_with_cue_speech(
    harness: &mut Harness,
    speech: &mut Option<CueSpeech>,
    taps: Taps,
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
        taps,
        no_announcements(),
        None,
        &mut presenter,
        harness.listener.as_ref(),
        &harness.panels,
        None,
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

/// A recorder that creates its marker file first, then captures a little audio and waits.
const MARKS_ITS_START_THEN_AUDIBLE: &str =
    "touch \"$0.emitted\"\nhead -c 4 /dev/zero\nexec sleep 30";

/// A cue long enough that a recorder started early could not hide behind scheduling noise.
const TEST_CUE_LENGTH: Duration = Duration::from_millis(400);

/// Asserts that the one cue call was made before the recorder started, and that
/// `[listening]` appeared only after the whole cue had played.
fn assert_listening_waited_for_the_cue(calls: &Mutex<Vec<CueCall>>, listening_seen: Instant) {
    let calls = calls.lock().expect("the call log lock should be available");
    assert_eq!(calls.len(), 1, "one listening cue per listen");
    assert!(
        !calls[0].recorder_started,
        "the cue was asked for before the recorder started"
    );
    assert!(
        listening_seen.duration_since(calls[0].at) >= TEST_CUE_LENGTH,
        "listening began {:?} after the cue started, before its {TEST_CUE_LENGTH:?} ended",
        listening_seen.duration_since(calls[0].at)
    );
}

#[tokio::test]
async fn a_typed_listen_plays_the_cue_out_before_the_recorder_starts() {
    let mut harness = harness()
        .await
        .with_voice(
            MARKS_ITS_START_THEN_AUDIBLE,
            transcript_reply("never asked"),
        )
        .await;
    let (speech, calls) = CueSpeech::new(TEST_CUE_LENGTH, harness.emitted_marker());
    let mut speech = Some(speech);
    let mut listening_seen = None;

    let shown = run_with_cue_speech(&mut harness, &mut speech, Taps::none(), async |user| {
        user.type_(b"/listen\n").await;
        user.wait_for(AssistantState::Listening).await;
        listening_seen = Some(Instant::now());
        user.type_(b"/stop\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [AssistantState::Listening, AssistantState::Idle]
    );
    assert_listening_waited_for_the_cue(&calls, listening_seen.expect("listening was reached"));
}

#[tokio::test]
async fn a_tap_plays_the_cue_out_before_the_recorder_starts() {
    let mut harness = harness()
        .await
        .with_voice(
            MARKS_ITS_START_THEN_AUDIBLE,
            transcript_reply("never asked"),
        )
        .await;
    let (speech, calls) = CueSpeech::new(TEST_CUE_LENGTH, harness.emitted_marker());
    let mut speech = Some(speech);
    let (tap, taps) = tap_queue();
    let mut listening_seen = None;

    let shown = run_with_cue_speech(&mut harness, &mut speech, taps, async |user| {
        tap.send(Tap).await.expect("the loop reads taps");
        user.wait_for(AssistantState::Listening).await;
        listening_seen = Some(Instant::now());
        user.type_(b"/stop\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [AssistantState::Listening, AssistantState::Idle]
    );
    assert_listening_waited_for_the_cue(&calls, listening_seen.expect("listening was reached"));
}

#[tokio::test]
async fn a_zero_length_cue_adds_no_wait_and_listening_proceeds() {
    let mut harness = harness()
        .await
        .with_voice(
            MARKS_ITS_START_THEN_AUDIBLE,
            transcript_reply("never asked"),
        )
        .await;
    let (speech, calls) = CueSpeech::new(Duration::ZERO, harness.emitted_marker());
    let mut speech = Some(speech);

    let shown = run_with_cue_speech(&mut harness, &mut speech, Taps::none(), async |user| {
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
    let calls = calls.lock().expect("the call log lock should be available");
    assert_eq!(calls.len(), 1, "the cue is still asked for once");
    assert!(!calls[0].recorder_started);
}

#[tokio::test]
async fn no_cue_is_played_when_speech_is_disabled_and_listening_still_works() {
    let mut harness = harness()
        .await
        .with_voice(
            MARKS_ITS_START_THEN_AUDIBLE,
            transcript_reply("never asked"),
        )
        .await;
    let (mut speech, calls) = CueSpeech::new(TEST_CUE_LENGTH, harness.emitted_marker());
    speech.disabled = true;
    let mut speech = Some(speech);
    let (tap, taps) = tap_queue();

    let shown = run_with_cue_speech(&mut harness, &mut speech, taps, async |user| {
        user.type_(b"/listen\n").await;
        user.wait_for(AssistantState::Listening).await;
        user.type_(b"/stop\n").await;
        user.wait_for(AssistantState::Idle).await;
        tap.send(Tap).await.expect("the loop reads taps");
        user.wait_for(AssistantState::Listening).await;
        user.type_(b"/stop\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [
            AssistantState::Listening,
            AssistantState::Idle,
            AssistantState::Listening,
            AssistantState::Idle
        ]
    );
    assert!(
        calls
            .lock()
            .expect("the call log lock should be available")
            .is_empty(),
        "a disabled speech output plays no cue"
    );
}

#[tokio::test]
async fn close_it_hides_the_panel_that_is_shown_and_makes_no_model_request() {
    let mut harness = harness().await;
    let (panels, inbox) = crate::features::panel::channel();
    let (to_orb, mut shown_to_orb) = mpsc::channel(16);
    let (_events, from_orb) = mpsc::channel(16);
    let orb = crate::interfaces::orb::OrbHandle { to_orb, from_orb };
    let service = tokio::spawn(async move {
        match inbox
            .run(Some(orb), crate::features::panel::PANEL_TIMEOUT)
            .await {}
    });
    harness.panels = panels.clone();
    panels
        .show(orb_protocol::PanelName::Stats)
        .await
        .expect("the panel service is running");
    assert_eq!(
        shown_to_orb.recv().await,
        Some(orb_protocol::ToOrb::ShowPanel {
            panel: orb_protocol::PanelName::Stats
        })
    );

    run_script(&mut harness, async |driver| {
        driver.type_(b"close it\n").await;
        // The loop runs lines in order: `/exit` comes after the close was sent.
    })
    .await;

    assert_eq!(
        shown_to_orb.recv().await,
        Some(orb_protocol::ToOrb::HidePanel {})
    );
    assert_eq!(panels.shown(), None);
    assert_eq!(harness.model.request_count(), 0);
    service.abort();
}

// A new conversation after 30 minutes of quiet (fast-memory 09). The clock is paused,
// so the gaps are exact and nothing waits in real time.

const ONE_MS: Duration = Duration::from_millis(1);

fn failing_turn() -> Vec<MockStreamEvent> {
    vec![
        MockStreamEvent::text("partial"),
        MockStreamEvent::error("injected failure"),
    ]
}

/// How many messages each model request carried as chat history, in order.
fn histories_sent(harness: &Harness) -> Vec<usize> {
    harness
        .model
        .requests()
        .iter()
        .map(|request| request.chat_history.len())
        .collect()
}

/// Types `line` and waits until the loop is idle again.
async fn say(driver: &mut Driver, line: &str) {
    driver.type_(format!("{line}\n").as_bytes()).await;
    driver.wait_for(AssistantState::Idle).await;
}

#[tokio::test(start_paused = true)]
async fn a_prompt_after_exactly_thirty_minutes_of_quiet_starts_with_an_empty_history() {
    assert_eq!(QUIET_BEFORE_NEW_CONVERSATION, Duration::from_secs(30 * 60));
    // (quiet before the second prompt, request messages: first prompt + answer + second
    // prompt when kept, the second prompt alone when the conversation started afresh)
    let cases = [
        (QUIET_BEFORE_NEW_CONVERSATION - ONE_MS, 3),
        (QUIET_BEFORE_NEW_CONVERSATION, 1),
    ];
    for (quiet, second_request_messages) in cases {
        let mut harness = harness_answering(vec![answer(), answer()]).await;
        run_script(&mut harness, async |driver| {
            say(driver, "first question").await;
            tokio::time::advance(quiet).await;
            say(driver, "second question").await;
        })
        .await;

        assert_eq!(
            histories_sent(&harness),
            [1, second_request_messages],
            "quiet of {quiet:?}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn the_first_request_after_a_quiet_reset_carries_only_the_new_prompt() {
    let mut harness = harness_answering(vec![answer(), answer(), answer()]).await;
    run_script(&mut harness, async |driver| {
        say(driver, "first question").await;
        say(driver, "second question").await;
        tokio::time::advance(QUIET_BEFORE_NEW_CONVERSATION).await;
        say(driver, "third question").await;
    })
    .await;

    let requests = harness.model.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1].chat_history.len(), 3);
    assert_eq!(
        requests[2].chat_history.iter().collect::<Vec<_>>(),
        [&user_message("third question")]
    );
    assert_eq!(requests[2].preamble, requests[0].preamble);
}

#[tokio::test(start_paused = true)]
async fn a_quiet_reset_deletes_neither_the_archive_nor_the_memories() {
    let mut harness = harness_answering(vec![answer(), answer()]).await;
    run_script(&mut harness, async |driver| {
        say(driver, "first question").await;
        tokio::time::advance(QUIET_BEFORE_NEW_CONVERSATION).await;
        say(driver, "second question").await;
    })
    .await;

    let database = harness._archive_dir.path().join("conversation.sqlite3");
    let connection = rusqlite::Connection::open(database).expect("the archive opens");
    let count = |sql: &str| -> i64 {
        connection
            .query_row(sql, [], |row| row.get(0))
            .expect("the rows can be counted")
    };
    assert_eq!(count("SELECT COUNT(*) FROM turns"), 2);
    assert_eq!(
        count("SELECT COUNT(*) FROM sources WHERE state = 'active'"),
        2
    );
    assert_eq!(
        count("SELECT COUNT(DISTINCT source_id) FROM memories"),
        2,
        "each turn, the old conversation's included, keeps its memories"
    );
}

#[tokio::test(start_paused = true)]
async fn the_first_prompt_after_start_up_does_not_reset_however_long_jarvis_waited() {
    let mut harness = harness_answering(vec![answer(), answer()]).await;
    assert_eq!(harness.session.quiet_for(), None);
    run_script(&mut harness, async |driver| {
        tokio::time::advance(QUIET_BEFORE_NEW_CONVERSATION * 4).await;
        say(driver, "first question").await;
        say(driver, "second question").await;
    })
    .await;

    assert_eq!(histories_sent(&harness), [1, 3]);
}

#[tokio::test(start_paused = true)]
async fn a_failed_turn_does_not_restart_the_quiet_count() {
    let mut harness = harness_answering(vec![answer(), failing_turn(), answer()]).await;
    run_script(&mut harness, async |driver| {
        say(driver, "first question").await;
        tokio::time::advance(QUIET_BEFORE_NEW_CONVERSATION / 3).await;
        say(driver, "failing question").await;
        // 30 minutes since the answer, only 20 since the failure.
        tokio::time::advance(QUIET_BEFORE_NEW_CONVERSATION * 2 / 3).await;
        say(driver, "third question").await;
    })
    .await;

    assert_eq!(histories_sent(&harness), [1, 3, 1]);
}

#[tokio::test(start_paused = true)]
async fn a_command_that_never_reaches_the_model_neither_resets_nor_restarts_the_quiet_count() {
    let third = QUIET_BEFORE_NEW_CONVERSATION / 3;
    // A command 20 minutes after the answer does not restart the count, so the prompt
    // 10 minutes later is 30 minutes after the answer and starts afresh.
    for command in ["/help", "/memories", "close it"] {
        let mut harness = harness_answering(vec![answer(), answer()]).await;
        run_script(&mut harness, async |driver| {
            say(driver, "first question").await;
            tokio::time::advance(third * 2).await;
            driver.type_(format!("{command}\n").as_bytes()).await;
            tokio::time::advance(third).await;
            say(driver, "second question").await;
        })
        .await;

        assert_eq!(histories_sent(&harness), [1, 1], "after {command}");
    }
}

/// Never answers, counting the requests that started and the ones dropped while waiting.
#[derive(Clone)]
struct WarmupCountingModel {
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
        _request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse, CompletionError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        let _waiting = CountsDrop(Arc::clone(&self.dropped));
        std::future::pending().await
    }
}

#[tokio::test]
async fn ending_the_conversation_stops_a_warm_up_that_is_still_running() {
    let model = WarmupCountingModel {
        started: Arc::default(),
        dropped: Arc::default(),
    };
    let (started, dropped) = (Arc::clone(&model.started), Arc::clone(&model.dropped));
    let assistant = Assistant::new(AgentBuilder::new(model).build());
    let archive_dir = tempfile::tempdir().expect("temporary archive directory should open");
    let mut session = ConversationSession::new(archive_dir.path().join("conversation.sqlite3"));
    assert!(session.reset().await.is_ok());
    let mut warmup = WarmupTask::default();
    let mut memory_control = MemoryControl::default();
    warmup.start(
        &assistant,
        CacheWarmup::new(Vec::new(), Message::user("a question")),
    );
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);

    reset_conversation(&mut session, &mut warmup, &mut memory_control).await;
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }

    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

async fn saved_facts(harness: &Harness) -> Vec<String> {
    harness
        .session
        .facts_repository()
        .expect("the archive is open")
        .list()
        .await
        .expect("facts list")
        .into_iter()
        .map(|fact| fact.text)
        .collect()
}

#[tokio::test]
async fn remember_that_saves_a_fact_and_says_got_it_without_a_model_request() {
    let mut harness = harness().await;

    let shown = run_script(&mut harness, async |user| {
        user.type_(b"Hey Jarvis, remember that my cat is Miso.\n")
            .await;
        user.wait_for_notice_containing("Got it.").await;
        user.type_(b"/remember my dog is Rex\n").await;
        user.wait_for_notice_containing("Got it.").await;
    })
    .await;

    assert_eq!(notices(&shown), ["Got it.", "Got it."]);
    assert!(states(&shown).is_empty(), "a save is not a model run");
    assert!(harness.model.requests().is_empty());
    assert_eq!(
        saved_facts(&harness).await,
        ["my cat is Miso.", "my dog is Rex"]
    );
}

#[tokio::test]
async fn a_saved_fact_is_spoken_when_speech_is_on() {
    let mut harness = harness().await;
    let (release_sender, release) = oneshot::channel();
    release_sender.send(()).expect("the receiver is alive");
    let mut speech = Some(speech_that_plays_until_released(release));

    let shown = run_with(&mut harness, &mut speech, async |user| {
        user.type_(b"remember that my cat is Miso\n").await;
        user.wait_for_notice_containing("Got it.").await;
    })
    .await;

    assert_eq!(notices(&shown), ["Got it."]);
    let speech = speech.expect("speech stays owned by the caller");
    assert_eq!(speech.calls, ["Got it."]);
    assert!(harness.model.requests().is_empty());
}

#[tokio::test]
async fn a_refused_save_says_why_saves_nothing_and_sends_nothing_to_the_model() {
    let mut harness = harness().await;
    let too_long = "z".repeat(crate::storage::MAX_FACT_BYTES + 1);

    let shown = run_script(&mut harness, async |user| {
        user.type_(format!("remember that {too_long}\n").as_bytes())
            .await;
        user.wait_for_notice_containing("too long").await;
    })
    .await;

    assert_eq!(
        notices(&shown),
        ["That is too long for me to remember. Keep it to 200 characters or fewer."]
    );
    assert!(saved_facts(&harness).await.is_empty());
    assert!(harness.model.requests().is_empty());
}

#[tokio::test]
async fn remember_that_time_is_a_prompt_and_saves_no_fact() {
    let mut harness = harness().await;

    let shown = run_script(&mut harness, async |user| {
        user.type_(b"remember that time we met\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(
        states(&shown),
        [AssistantState::Thinking, AssistantState::Idle]
    );
    assert_user_message_reached_model(&harness, "remember that time we met");
    assert!(saved_facts(&harness).await.is_empty());
}

#[tokio::test]
async fn forget_that_right_after_a_save_removes_the_saved_fact() {
    let mut harness = harness().await;

    let shown = run_script(&mut harness, async |user| {
        user.type_(b"remember that my cat is Miso\n").await;
        user.wait_for_notice_containing("Got it.").await;
        // Lines are handled in order, so the `/exit` that follows runs after this one;
        // its reply goes to stdout like every memory command's, not to the presenter.
        user.type_(b"forget that\n").await;
    })
    .await;

    assert_eq!(notices(&shown), ["Got it."]);
    assert!(saved_facts(&harness).await.is_empty());
    assert!(harness.model.requests().is_empty());
}

async fn seed_fact(harness: &Harness, text: &str) {
    harness
        .session
        .facts_repository()
        .expect("the archive is open")
        .insert(text, crate::storage::FactOrigin::Noted)
        .await
        .expect("fact is saved");
}

/// The fake speech disables itself after its first playback, so each test checks one
/// spoken reply. Replies of memory commands go to stdout, not the presenter, so the
/// script cannot wait for them: lines are handled in order and the `/exit` after the
/// last one ends the loop.
fn speech_for_one_reply() -> Option<FailingSpeech> {
    let (release_sender, release) = oneshot::channel();
    release_sender.send(()).expect("the receiver is alive");
    Some(speech_that_plays_until_released(release))
}

#[tokio::test]
async fn the_spoken_facts_listing_is_said_with_its_number_and_origin() {
    let mut harness = harness().await;
    seed_fact(&harness, "The user lives in Leeds.").await;
    let mut speech = speech_for_one_reply();

    run_with(&mut harness, &mut speech, async |user| {
        user.type_(b"Hey Jarvis, what facts do you have?\n").await;
    })
    .await;

    let speech = speech.expect("speech stays owned by the caller");
    assert_eq!(speech.calls, ["Fact 1, noted: The user lives in Leeds."]);
    assert!(harness.model.requests().is_empty());
}

#[tokio::test]
async fn a_forget_reply_is_spoken_but_a_memory_listing_is_not() {
    let mut harness = harness().await;
    seed_fact(&harness, "The user has a cat named Miso.").await;
    let mut speech = speech_for_one_reply();

    run_with(&mut harness, &mut speech, async |user| {
        user.type_(b"/memories\n").await;
        user.type_(b"Forget that my cat name is Miso.\n").await;
    })
    .await;

    let speech = speech.expect("speech stays owned by the caller");
    assert_eq!(
        speech.calls,
        ["Forgot that: The user has a cat named Miso."]
    );
    assert!(saved_facts(&harness).await.is_empty());
}

#[tokio::test]
async fn a_spoken_forget_with_loose_wording_removes_the_fact_with_speech_off() {
    let mut harness = harness().await;

    run_script(&mut harness, async |user| {
        user.type_(b"remember that my sister is named Mia\n").await;
        user.wait_for_notice_containing("Got it.").await;
        user.type_(b"Forget that my sister name is Mia.\n").await;
    })
    .await;

    assert!(saved_facts(&harness).await.is_empty());
}

/// Whether any message of the first model request mentions `text`.
fn first_request_mentions(harness: &Harness, text: &str) -> bool {
    format!("{:?}", harness.model.requests()[0].chat_history).contains(text)
}

#[tokio::test]
async fn a_fact_removed_by_number_is_absent_from_the_next_model_request() {
    let mut harness = harness().await;

    run_script(&mut harness, async |user| {
        user.type_(b"remember that my cat is Miso\n").await;
        user.wait_for_notice_containing("Got it.").await;
        user.type_(b"remember that my dog is Rex\n").await;
        user.wait_for_notice_containing("Got it.").await;
        // The first fact is number 1 in a new database; a bad number removes nothing.
        user.type_(b"/memory forget 99\n").await;
        user.type_(b"/memory forget 1\n").await;
        user.type_(b"what pets do I have?\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(saved_facts(&harness).await, ["my dog is Rex"]);
    assert_eq!(harness.model.requests().len(), 1);
    assert!(first_request_mentions(&harness, "- my dog is Rex"));
    assert!(!first_request_mentions(&harness, "my cat is Miso"));
}

#[tokio::test]
async fn forget_detail_removes_a_unique_fact_before_the_next_model_request() {
    let mut harness = harness().await;

    run_script(&mut harness, async |user| {
        user.type_(b"remember that my cat is Miso\n").await;
        user.wait_for_notice_containing("Got it.").await;
        user.type_(b"remember that my dog is Rex\n").await;
        user.wait_for_notice_containing("Got it.").await;
        // "my" matches both facts, so nothing is removed; "miso" matches one.
        user.type_(b"forget my\n").await;
        user.type_(b"forget miso\n").await;
        user.type_(b"what pets do I have?\n").await;
        user.wait_for(AssistantState::Idle).await;
    })
    .await;

    assert_eq!(saved_facts(&harness).await, ["my dog is Rex"]);
    assert!(!first_request_mentions(&harness, "my cat is Miso"));
}

const CAMERA_LINE: &str = "The camera isn't available.";

/// A background source holding one line already, and its sender (kept alive so the
/// source stays open).
fn one_announcement() -> (tokio::sync::mpsc::Sender<Notice>, Announcements) {
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    sender
        .try_send(Notice::Warning(CAMERA_LINE.to_owned()))
        .expect("the queue has room");
    (sender, Announcements::new(receiver))
}

#[tokio::test]
async fn an_announcement_while_idle_is_shown_and_said() {
    let mut harness = harness().await;
    let mut speech = speech_for_one_reply();
    let (_sender, announcements) = one_announcement();

    let shown = run_with_sources(
        &mut harness,
        &mut speech,
        Taps::none(),
        announcements,
        None,
        async |user| user.wait_for_notice_containing(CAMERA_LINE).await,
    )
    .await;

    assert_eq!(notices(&shown), [CAMERA_LINE]);
    let speech = speech.expect("speech stays owned by the caller");
    assert_eq!(speech.calls, [CAMERA_LINE]);
    assert!(harness.model.requests().is_empty());
}

#[tokio::test]
async fn with_speech_off_an_announcement_is_only_shown_and_the_next_prompt_still_runs() {
    let mut harness = harness().await;
    let (_sender, announcements) = one_announcement();

    let shown = run_with_sources(
        &mut harness,
        &mut None,
        Taps::none(),
        announcements,
        None,
        async |user| {
            user.wait_for_notice_containing(CAMERA_LINE).await;
            user.type_(b"hello there\n").await;
            user.wait_for(AssistantState::Idle).await;
        },
    )
    .await;

    assert_eq!(notices(&shown), [CAMERA_LINE]);
    assert_user_message_reached_model(&harness, "hello there");
}

#[tokio::test]
async fn a_startup_warmup_goes_first_and_the_first_prompt_still_runs() {
    // The warm-up takes the first scripted turn; the prompt is answered by the second.
    let mut harness = harness_answering(vec![answer(), answer()]).await;
    let model = harness.model.clone();
    let startup_warmup = harness.session.startup_warmup().await;

    let shown = run_with_sources(
        &mut harness,
        &mut None,
        Taps::none(),
        no_announcements(),
        Some(startup_warmup),
        async |user| {
            for _ in 0..500 {
                if !model.requests().is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            user.type_(b"hello there\n").await;
            user.wait_for(AssistantState::Idle).await;
        },
    )
    .await;

    let requests = harness.model.requests();
    assert_eq!(requests.len(), 2, "the warm-up, then the prompt");
    let warmup = &requests[0];
    // A fresh database has no facts, so the warm-up is the stand-in prompt alone.
    assert_eq!(warmup.chat_history.len(), 1, "{:?}", warmup.chat_history);
    assert!(matches!(warmup.chat_history[0], Message::User { .. }));
    assert_ne!(warmup.chat_history[0], user_message("hello there"));
    assert_eq!(warmup.max_tokens, Some(1), "a warm-up only reads");
    assert_eq!(
        requests[1].chat_history.last(),
        Some(&user_message("hello there"))
    );
    assert_eq!(
        shown
            .iter()
            .filter(|shown| matches!(shown, Shown::Answer(text) if text.contains(REPLY)))
            .count(),
        1
    );
}

/// A timer service on its own temporary database, on the real clock, and the
/// link the REPL holds. Tests finish well inside a confirmation window.
async fn timer_link() -> (
    crate::features::timers::TimerLink,
    tokio::task::JoinHandle<()>,
    TempDir,
) {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let archive = ConversationArchive::open(directory.path().join("timers.sqlite3"))
        .await
        .expect("archive opens");
    let repository = archive.timers_repository();
    let (handle, inbox, events) = crate::features::timers::channel();
    let task = tokio::spawn(async move {
        match inbox
            .run(repository, crate::features::timers::SystemClock)
            .await {}
    });
    (
        crate::features::timers::TimerLink { handle, events },
        task,
        directory,
    )
}

#[tokio::test]
async fn yes_answers_a_pending_timer_without_the_model_and_is_a_prompt_otherwise() {
    let model = MockCompletionModel::from_stream_turns(vec![vec![
        MockStreamEvent::text("Yes to what?"),
        MockStreamEvent::final_response(Usage::new()),
    ]]);
    let model_handle = model.clone();
    let (link, service, _timers_directory) = timer_link().await;
    let handle = link.handle.clone();
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
            handle.clone(),
        )
        .build(),
    );
    let archive_dir = tempfile::tempdir().expect("temporary archive directory should open");
    let mut session = ConversationSession::new(archive_dir.path().join("conversation.sqlite3"));
    assert!(session.reset().await.is_ok());
    let proposed = handle
        .propose(
            Some("Time to sleep".to_owned()),
            jiff::Timestamp::now().as_millisecond() + 10 * 60_000,
        )
        .await
        .expect("proposed");
    handle
        .watch_shown()
        .wait_for(|timers| timers.iter().any(|timer| timer.id == proposed.id))
        .await
        .expect("the service is running");

    let mut speech = None::<FailingSpeech>;
    // The first "yes" sets the timer; the second has nothing to answer.
    let mut input = BufReader::new(std::io::Cursor::new(b"Yes.\nyes\n/exit\n"));
    let mut presenter = TerminalPresenter::with_writer(Vec::new());
    let panels = crate::features::panel::channel().0;
    run_input_loop(
        &assistant,
        &mut session,
        None,
        &mut speech,
        &mut input,
        Taps::none(),
        no_announcements(),
        None,
        &mut presenter,
        None,
        &panels,
        Some(link),
    )
    .await
    .expect("piped input should complete");
    service.abort();

    let output = String::from_utf8(presenter.into_writer()).expect("terminal output is UTF-8");
    assert!(output.contains("Set. 'Time to sleep' ends at "), "{output}");
    assert_eq!(model_handle.request_count(), 1, "only the second yes runs");
    assert_eq!(
        model_handle.requests()[0].chat_history,
        vec![user_message("yes")]
    );
    assert!(output.contains("Jarvis: Yes to what?"), "{output}");
    let running = handle.shown();
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].state, crate::storage::TimerState::Running);
}

#[tokio::test]
async fn a_timer_that_ends_while_idle_gets_a_reminder_run_with_no_tools() {
    let mut harness = harness_answering(vec![vec![
        MockStreamEvent::text("Time to sleep, sir."),
        MockStreamEvent::final_response(Usage::new()),
    ]])
    .await;
    let (link, service, _timers_directory) = timer_link().await;
    let handle = link.handle.clone();
    // A real timer on the real clock, ending a moment from now.
    let proposed = handle
        .propose(
            Some("Time to sleep".to_owned()),
            jiff::Timestamp::now().as_millisecond() + 300,
        )
        .await
        .expect("proposed");
    handle.confirm(proposed.id).await.expect("confirmed");

    let shown = run_with_timers(
        &mut harness,
        &mut None,
        Taps::none(),
        no_announcements(),
        None,
        Some(link),
        async |driver| {
            driver.wait_for(AssistantState::Thinking).await;
            driver.wait_for(AssistantState::Idle).await;
        },
    )
    .await;
    service.abort();

    assert!(
        shown.contains(&Shown::Answer("Time to sleep, sir.".to_owned())),
        "{shown:?}"
    );
    let requests = harness.model.requests();
    assert_eq!(requests.len(), 1);
    // Every tool listed, none callable (timers D20).
    assert_eq!(requests[0].tools.len(), crate::tools::ALLOWED_TOOLS.len());
    assert_eq!(
        requests[0].tool_choice,
        Some(rig_core::message::ToolChoice::None)
    );
    let prompt = format!("{:?}", requests[0].chat_history.last());
    assert!(
        prompt.contains("My timer 'Time to sleep', set at ") && prompt.contains("has just ended"),
        "{prompt}"
    );
}
