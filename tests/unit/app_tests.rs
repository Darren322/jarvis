use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use rig_agent::AgentBuilder;
use rig_core::{
    message::{AssistantContent, Message, UserContent},
    test_utils::{MockCompletionModel, MockTurn},
};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    sync::{Semaphore, oneshot},
};

use crate::{
    services::{assistant::Assistant, conversation::ConversationSession},
    tools::system_status_tool::SystemStatusTool,
};

use super::{
    InputLine, InputRejection, MAX_INPUT_BYTES, SpeechControl, read_input_line, run_input_loop,
};

fn reader(bytes: Vec<u8>) -> BufReader<std::io::Cursor<Vec<u8>>> {
    BufReader::new(std::io::Cursor::new(bytes))
}

#[tokio::test]
async fn input_reader_handles_caps_framing_and_eof() {
    let exact = "a".repeat(MAX_INPUT_BYTES);
    let cases = [
        (format!("{exact}\n").into_bytes(), exact.clone()),
        (format!("{exact}\r\n").into_bytes(), exact.clone()),
        (exact.as_bytes().to_vec(), exact),
    ];

    for (bytes, expected) in cases {
        let mut input = reader(bytes);
        assert!(matches!(
            read_input_line(&mut input).await.expect("input should read"),
            InputLine::Prompt(prompt) if prompt == expected
        ));
        assert!(matches!(
            read_input_line(&mut input).await.expect("EOF should read"),
            InputLine::Eof
        ));
    }

    let mut input = reader(format!("{}\n", "a".repeat(MAX_INPUT_BYTES + 1)).into_bytes());
    assert!(matches!(
        read_input_line(&mut input)
            .await
            .expect("input should read"),
        InputLine::Rejected(InputRejection::TooLong)
    ));
}

#[tokio::test]
async fn input_reader_rejects_locally_and_drains_overflow_before_next_line() {
    let mut bytes = vec![b'a'; MAX_INPUT_BYTES + 10];
    bytes.extend_from_slice(b"\n  keep whitespace  \n\t \r\n");
    let mut input = reader(bytes);

    assert!(matches!(
        read_input_line(&mut input)
            .await
            .expect("oversized input should read"),
        InputLine::Rejected(InputRejection::TooLong)
    ));
    assert!(matches!(
        read_input_line(&mut input).await.expect("next line should read"),
        InputLine::Prompt(prompt) if prompt == "  keep whitespace  "
    ));
    assert!(matches!(
        read_input_line(&mut input)
            .await
            .expect("blank line should read"),
        InputLine::Rejected(InputRejection::Blank)
    ));

    let mut invalid = reader(vec![0xff, b'\n']);
    assert!(matches!(
        read_input_line(&mut invalid)
            .await
            .expect("invalid UTF-8 should read"),
        InputLine::Rejected(InputRejection::InvalidUtf8)
    ));
}

struct FailingSpeech {
    calls: Vec<String>,
    started: Option<oneshot::Sender<()>>,
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

    async fn speak(&mut self, text: &str) -> Result<(), Self::Error> {
        self.calls.push(text.to_owned());
        self.started
            .take()
            .expect("speech should start once")
            .send(())
            .expect("test should still be waiting for speech");
        (&mut self.release)
            .await
            .map_err(|_| "test release sender dropped")?;
        self.disabled.store(true, Ordering::SeqCst);
        self.disabled_signal.add_permits(1);
        Err("injected playback failure")
    }

    async fn cancel(&mut self) -> Result<(), Self::Error> {
        self.cancel_calls += 1;
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
    let model = MockCompletionModel::from_turns([
        MockTurn::text("visible final one"),
        MockTurn::text("visible final two"),
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

    let mut loop_future = Box::pin(run_input_loop(
        &assistant,
        &mut session,
        &mut speech,
        &mut input,
    ));
    tokio::select! {
        result = &mut loop_future => panic!("loop should wait for speech release: {result:?}"),
        result = started_rx => result.expect("speech should start after the first final answer"),
    }

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
}
