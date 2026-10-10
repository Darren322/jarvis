use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use rig_agent::AgentBuilder;
use rig_core::{
    completion::Usage,
    message::{AssistantContent, Message, UserContent},
    test_utils::{MockCompletionModel, MockStreamEvent},
};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    sync::{Semaphore, oneshot},
};

use crate::{
    assistant::Assistant,
    conversation::ConversationSession,
    storage::{MemoryId, SourceId},
    tools::system_status_tool::SystemStatusTool,
};

use super::super::commands::memory::{
    CurrentMemoryFocus, focus_from_displayed_ids, focus_from_successful_turn,
};
use super::super::commands::{Command, parse};
use super::super::presentation::TerminalPresenter;
use super::{run_input_loop, speech::SpeechControl};

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

#[test]
fn forget_focus_uses_only_an_exact_deictic_phrase_and_trusted_foreground_ids() {
    assert_eq!(parse("forget that, please.".into()), Command::ForgetFocus);
    assert_eq!(
        parse("forget that file from yesterday".into()),
        Command::ForgetDetail("that file from yesterday".into())
    );
    assert_eq!(
        focus_from_displayed_ids([MemoryId(12)].into_iter()),
        Some(CurrentMemoryFocus::Memory(MemoryId(12)))
    );
    assert_eq!(
        focus_from_displayed_ids([MemoryId(12), MemoryId(13)].into_iter()),
        Some(CurrentMemoryFocus::AmbiguousDisplay)
    );
    assert_eq!(focus_from_displayed_ids(std::iter::empty()), None);

    let foreground_source = SourceId(3);
    let later_backfill_source = SourceId(90);
    assert_eq!(
        focus_from_successful_turn(Some(foreground_source)),
        Some(CurrentMemoryFocus::Source(foreground_source))
    );
    assert_ne!(
        focus_from_successful_turn(Some(foreground_source)),
        Some(CurrentMemoryFocus::Source(later_backfill_source)),
        "backfill IDs do not replace the foreground receipt focus"
    );
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
    assert!(session.reset().await.is_none());

    let mut speech = None::<FailingSpeech>;
    let mut memory = None;
    let mut input = BufReader::new(std::io::Cursor::new(
        b"first prompt\nsecond prompt\n/exit\n",
    ));
    let mut presenter = TerminalPresenter::with_writer(Vec::new());
    run_input_loop(
        &assistant,
        &mut session,
        &mut memory,
        &mut speech,
        &mut input,
        &mut presenter,
    )
    .await
    .expect("piped input should complete after both serial model runs");

    let output = String::from_utf8(presenter.into_writer()).expect("terminal output is UTF-8");
    assert!(output.contains("Jarvis: partial response [incomplete response]\n"));
    assert!(output.contains("You> Jarvis: second answer\n"));
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
    type Error = &'static str;

    fn is_disabled(&self) -> bool {
        self.disabled.load(Ordering::SeqCst)
    }

    async fn speak(
        &mut self,
        text: &str,
        playback_started: oneshot::Sender<()>,
    ) -> Result<(), Self::Error> {
        self.calls.push(text.to_owned());
        self.started
            .take()
            .expect("speech should start once")
            .send(())
            .expect("test should still be waiting for speech");
        let mut playback_started = Some(playback_started);
        if self.queue_on_start {
            playback_started
                .take()
                .expect("playback notification should be owned once")
                .send(())
                .expect("App should still be waiting for playback");
        }
        (&mut self.release)
            .await
            .map_err(|_| "test release sender dropped")?;
        drop(playback_started);
        self.disabled.store(true, Ordering::SeqCst);
        self.disabled_signal.add_permits(1);
        Err("injected playback failure")
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
    assert!(session.reset().await.is_none());

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
    let mut memory = None;
    let mut loop_future = Box::pin(run_input_loop(
        &assistant,
        &mut session,
        &mut memory,
        &mut speech,
        &mut input,
        &mut present_answer,
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
    assert!(session.reset().await.is_none());

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
    let mut memory = None;
    let mut loop_future = Box::pin(run_input_loop(
        &assistant,
        &mut session,
        &mut memory,
        &mut speech,
        &mut input,
        &mut present_answer,
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
    assert!(session.reset().await.is_none());

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
    let mut memory = None;
    let mut loop_future = Box::pin(run_input_loop(
        &assistant,
        &mut session,
        &mut memory,
        &mut speech,
        &mut input,
        &mut present_answer,
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
