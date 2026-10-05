use std::{
    io::{self, Write},
    path::PathBuf,
};

use rig_agent::AgentBuilder;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::clients::local_llm::LocalLlm;
use crate::config::{AppConfig, OptionalTtsConfig};
use crate::services::assistant::{Assistant, AssistantError, CallUsage, RunReport};
use crate::services::conversation::ConversationSession;
use crate::services::tts::speech::{SpeechError, SpeechOutput};
use crate::tools::system_status_tool::SystemStatusTool;

const MAX_INPUT_BYTES: usize = 2_048;
const MAX_FRAMED_INPUT_BYTES: usize = MAX_INPUT_BYTES + 2;

// LEARNING: `App` is like a Java class's related fields; its `impl` block below
// supplies methods, while Rust declares data and behavior in separate blocks.
pub struct App {
    assistant: Assistant,
    local_llm: LocalLlm,
    archive_path: PathBuf,
    tts_config: OptionalTtsConfig,
}

impl App {
    // LEARNING: `App::new` is an associated constructor (a naming convention,
    // not a special Rust constructor). `&AppConfig` borrows settings so this
    // call need not take ownership of config values it only reads.
    pub fn new(config: &AppConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let local_llm = LocalLlm::new(config)?;
        let configured_archive_path = PathBuf::from(&config.archive_path);
        let archive_path = if configured_archive_path.is_absolute() {
            configured_archive_path
        } else {
            std::env::current_dir()?.join(configured_archive_path)
        };
        // LEARNING: `r#"..."#` is a raw multiline string: quotes and line
        // breaks remain literal, similar to a Java `"""..."""` text block.
        //
        // LEARNING: Java fluent builders often mutate and return `this`; these
        // Rust methods take and return the owned builder. `.build()` makes a
        // reusable `Agent` but sends no request. The preamble guides the model;
        // policy hooks enforce rules. `.tool(SystemStatusTool)` passes a
        // zero-field struct value (like `new SystemStatusTool()`), registering
        // its schema and implementation for Rig dispatch.
        let agent = AgentBuilder::new(local_llm.model())
            .preamble(
                r#"You are Jarvis. Respond briefly in natural language.

Only call an available tool when its documented purpose matches
the user's request. Never invent tools or call an unrelated tool.

If no available tool can perform the requested action, explain
that limitation in plain language and do not call any tool.

Never print raw tool-call markup. Only claim an action succeeded
when a successful tool result confirms it."#,
            )
            .tool(SystemStatusTool)
            .build();
        let assistant = Assistant::new(agent);

        // LEARNING: `Self` means `App` here. Field shorthand moves each local
        // into its same-named field; it does not make an extra copy.
        Ok(Self {
            assistant,
            local_llm,
            archive_path,
            tts_config: config.tts.clone(),
        })
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.local_llm.health_check().await?;

        let mut speech = match &self.tts_config {
            OptionalTtsConfig::Disabled => None,
            OptionalTtsConfig::Invalid(reason) => {
                eprintln!("Warning: local speech is disabled: {reason}");
                None
            }
            OptionalTtsConfig::Configured(config) => Some(SpeechOutput::new(config.clone())),
        };

        println!("Conversation archive: {}", self.archive_path.display());
        let mut session = ConversationSession::new(self.archive_path.clone());
        if let Some(error) = session.reset().await {
            eprintln!(
                "Warning: local archiving is unavailable; this session will use process-only context: {error}"
            );
        }
        println!(
            "Enter a message, /reset to clear conversation context, /stop to stop speech, or /exit to quit."
        );

        let stdin = tokio::io::stdin();
        let mut input = BufReader::new(stdin);
        run_input_loop(&self.assistant, &mut session, &mut speech, &mut input).await
    }
}

trait SpeechControl {
    type Error: std::fmt::Display;

    fn is_disabled(&self) -> bool;
    async fn speak(&mut self, text: &str) -> Result<(), Self::Error>;
    async fn cancel(&mut self) -> Result<(), Self::Error>;
    async fn shutdown(&mut self) -> Result<(), Self::Error>;
}

impl SpeechControl for SpeechOutput {
    type Error = SpeechError;

    fn is_disabled(&self) -> bool {
        SpeechOutput::is_disabled(self)
    }

    async fn speak(&mut self, text: &str) -> Result<(), Self::Error> {
        SpeechOutput::speak(self, text).await
    }

    async fn cancel(&mut self) -> Result<(), Self::Error> {
        SpeechOutput::cancel(self).await
    }

    async fn shutdown(&mut self) -> Result<(), Self::Error> {
        SpeechOutput::shutdown(self).await
    }
}

enum SpeechWait {
    Completed(InputLine),
    Stopped { cleanup_ok: bool },
    Input { line: InputLine, cleanup_ok: bool },
    InputError { error: io::Error },
}

async fn run_input_loop<R, S>(
    assistant: &Assistant,
    session: &mut ConversationSession,
    speech: &mut Option<S>,
    input: &mut R,
) -> Result<(), Box<dyn std::error::Error>>
where
    R: AsyncBufRead + Unpin,
    S: SpeechControl,
{
    let result = run_input_loop_inner(assistant, session, speech, input).await;

    if let Some(speech) = speech.as_mut()
        && let Err(error) = speech.shutdown().await
    {
        eprintln!("Warning: local speech shutdown did not complete: {error}");
    }

    result
}

async fn run_input_loop_inner<R, S>(
    assistant: &Assistant,
    session: &mut ConversationSession,
    speech: &mut Option<S>,
    input: &mut R,
) -> Result<(), Box<dyn std::error::Error>>
where
    R: AsyncBufRead + Unpin,
    S: SpeechControl,
{
    let mut speech_available = speech.as_ref().is_some_and(|speech| !speech.is_disabled());
    let mut next_input = None;

    loop {
        let line = if let Some(line) = next_input.take() {
            line
        } else {
            print!("You> ");
            io::stdout().flush()?;
            read_input_line(input).await?
        };

        match line {
            InputLine::Eof => return Ok(()),
            InputLine::Rejected(reason) => eprintln!("Input rejected: {}", reason.message()),
            InputLine::Prompt(prompt) => match prompt.trim() {
                "/exit" => return Ok(()),
                "/stop" => println!("No speech is active."),
                "/reset" => {
                    if let Some(speech) = speech.as_mut() {
                        let cleanup_ok = report_speech_cleanup(speech.cancel().await);
                        if !cleanup_ok || speech.is_disabled() {
                            speech_available = false;
                        }
                    }
                    if let Some(error) = session.reset().await {
                        eprintln!(
                            "Warning: local archiving is unavailable; this session will use process-only context: {error}"
                        );
                    }
                    println!("Conversation context reset.");
                }
                _ => {
                    let turn = session.respond(assistant, &prompt).await;
                    if let Some(warning) = turn.context_warning {
                        eprintln!("Warning: {warning}");
                    }
                    if let Some(error) = &turn.archive_error {
                        eprintln!(
                            "Warning: this turn could not be saved to the local archive: {error}"
                        );
                    }

                    match turn.run {
                        Ok(run) => {
                            print_run_diagnostics(&run.report);
                            let answer = run.response.output();
                            println!("Jarvis: {answer}");

                            if speech_available && let Some(speech) = speech.as_mut() {
                                match watch_speech(speech, answer, input, &mut speech_available)
                                    .await
                                {
                                    SpeechWait::Completed(line) => {
                                        next_input = Some(line);
                                    }
                                    SpeechWait::Stopped { cleanup_ok } => {
                                        if cleanup_ok {
                                            println!("Speech stopped.");
                                        }
                                        if !cleanup_ok || speech.is_disabled() {
                                            speech_available = false;
                                        }
                                    }
                                    SpeechWait::Input { line, cleanup_ok } => {
                                        if !cleanup_ok || speech.is_disabled() {
                                            speech_available = false;
                                        }
                                        next_input = Some(line);
                                    }
                                    SpeechWait::InputError { error } => {
                                        return Err(error.into());
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            print_run_diagnostics(&error.report);
                            eprintln!("Jarvis: {}", safe_error_message(&error.error));
                        }
                    }
                }
            },
        }
    }
}

async fn watch_speech<R, S>(
    speech: &mut S,
    text: &str,
    input: &mut R,
    speech_available: &mut bool,
) -> SpeechWait
where
    R: AsyncBufRead + Unpin,
    S: SpeechControl,
{
    print!("You> ");
    if let Err(error) = io::stdout().flush() {
        return SpeechWait::InputError { error };
    }

    let mut speech_future = Box::pin(speech.speak(text));

    loop {
        enum Event<E> {
            Speech(Result<(), E>),
            Input(io::Result<InputLine>),
        }

        let mut input_future = Box::pin(read_input_line(&mut *input));
        let event = tokio::select! {
            biased;
            line = input_future.as_mut() => Event::Input(line),
            result = &mut speech_future => Event::Speech(result),
        };

        match event {
            Event::Speech(result) => {
                drop(speech_future);
                if let Err(error) = result {
                    eprintln!("Warning: local speech failed: {error}");
                    if speech.is_disabled() {
                        *speech_available = false;
                    }
                }

                return match input_future.as_mut().await {
                    Ok(line) => SpeechWait::Completed(line),
                    Err(error) => SpeechWait::InputError { error },
                };
            }
            Event::Input(Ok(InputLine::Rejected(reason))) => {
                eprintln!("Input rejected: {}", reason.message());
                print!("You> ");
                if let Err(error) = io::stdout().flush() {
                    drop(speech_future);
                    report_speech_cleanup(speech.cancel().await);
                    return SpeechWait::InputError { error };
                }
            }
            Event::Input(Ok(InputLine::Prompt(prompt))) if prompt.trim() == "/stop" => {
                println!("Stopping speech.");
                drop(speech_future);
                let cleanup_ok = report_speech_cleanup(speech.cancel().await);
                return SpeechWait::Stopped { cleanup_ok };
            }
            Event::Input(Ok(line)) => {
                drop(speech_future);
                let cleanup_ok = report_speech_cleanup(speech.cancel().await);
                return SpeechWait::Input { line, cleanup_ok };
            }
            Event::Input(Err(error)) => {
                drop(speech_future);
                report_speech_cleanup(speech.cancel().await);
                return SpeechWait::InputError { error };
            }
        }
    }
}

fn report_speech_cleanup<E: std::fmt::Display>(result: Result<(), E>) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => {
            eprintln!("Warning: local speech cleanup did not complete: {error}");
            false
        }
    }
}

enum InputLine {
    Eof,
    Prompt(String),
    Rejected(InputRejection),
}

#[derive(Clone, Copy)]
enum InputRejection {
    TooLong,
    InvalidUtf8,
    Blank,
}

impl InputRejection {
    fn message(self) -> &'static str {
        match self {
            Self::TooLong => "the line exceeds the 2,048-byte limit.",
            Self::InvalidUtf8 => "the line is not valid UTF-8.",
            Self::Blank => "blank messages are not sent to the assistant.",
        }
    }
}

async fn read_input_line<R>(reader: &mut R) -> io::Result<InputLine>
where
    R: AsyncBufRead + Unpin,
{
    // LEARNING: `R` may be any reader with these capabilities, like a Java
    // generic method bounded by interfaces. `Unpin` means its address need not
    // stay fixed while async helpers poll it; Tokio requires this bound here.
    let mut bytes = Vec::with_capacity(MAX_FRAMED_INPUT_BYTES);
    // LEARNING: `AsyncReadExt` and `AsyncBufReadExt` provide `.take` and
    // `.read_until`. `&mut *reader` makes a short reborrow so `Take` wraps the
    // reader without moving it. `b'\n'` is a newline byte; two spare bytes allow
    // LF plus the optional CR framing byte.
    let bytes_read = (&mut *reader)
        .take(MAX_FRAMED_INPUT_BYTES as u64)
        .read_until(b'\n', &mut bytes)
        .await?;

    if bytes_read == 0 {
        return Ok(InputLine::Eof);
    }

    let terminated = bytes.last() == Some(&b'\n');
    if !terminated && bytes_read == MAX_FRAMED_INPUT_BYTES {
        drain_line(reader).await?;
        return Ok(InputLine::Rejected(InputRejection::TooLong));
    }

    if terminated {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }

    if bytes.len() > MAX_INPUT_BYTES {
        return Ok(InputLine::Rejected(InputRejection::TooLong));
    }

    let prompt = match String::from_utf8(bytes) {
        Ok(prompt) => prompt,
        Err(_) => return Ok(InputLine::Rejected(InputRejection::InvalidUtf8)),
    };
    if prompt.trim().is_empty() {
        return Ok(InputLine::Rejected(InputRejection::Blank));
    }

    Ok(InputLine::Prompt(prompt))
}

async fn drain_line<R>(reader: &mut R) -> io::Result<()>
where
    R: AsyncBufRead + Unpin,
{
    // LEARNING: If the bounded read stops before LF, drain the rest of that
    // line so its tail cannot become a phantom next prompt. `fill_buf` and
    // `consume` do this in chunks, keeping the input buffer bounded.
    loop {
        let (consumed, found_newline) = {
            let available = reader.fill_buf().await?;
            if available.is_empty() {
                return Ok(());
            }

            match available.iter().position(|byte| *byte == b'\n') {
                Some(index) => (index + 1, true),
                None => (available.len(), false),
            }
        };
        reader.consume(consumed);

        if found_newline {
            return Ok(());
        }
    }
}

fn safe_error_message(error: &AssistantError) -> &'static str {
    match error {
        AssistantError::Prompt(_) => "The model request failed.",
        AssistantError::RunTimeout => "The request timed out.",
        AssistantError::InvalidResponse => "The assistant returned an invalid response.",
    }
}

fn print_run_diagnostics(report: &RunReport) {
    eprintln!(
        "Run diagnostics: outcome={:?} assistant_elapsed={:?} observations_available={}",
        report.outcome, report.elapsed, report.observations_available
    );
    for stage in &report.model_stages {
        let usage = match &stage.usage {
            CallUsage::Unavailable => "unavailable".to_string(),
            CallUsage::Normalized(usage) => format!(
                "rig-normalized input={} output={} total={} cached_input={} cache_creation_input={} tool_use_prompt={} reasoning={}",
                usage.input_tokens,
                usage.output_tokens,
                usage.total_tokens,
                usage.cached_input_tokens,
                usage.cache_creation_input_tokens,
                usage.tool_use_prompt_tokens,
                usage.reasoning_tokens,
            ),
        };
        eprintln!(
            "Model stage: turn={} completed={} elapsed={:?} usage={usage}",
            stage.turn, stage.completed, stage.elapsed
        );
    }
    for stage in &report.tool_stages {
        eprintln!(
            "Tool stage: turn={} internal_call_id={:?} name={:?} outcome={:?} elapsed={:?}",
            stage.turn, stage.internal_call_id, stage.name, stage.outcome, stage.elapsed
        );
    }
}

#[cfg(test)]
mod tests {
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
}
