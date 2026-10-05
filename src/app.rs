use std::{io, io::Write, path::PathBuf};

use rig_agent::AgentBuilder;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::clients::local_llm::LocalLlm;
use crate::config::AppConfig;
use crate::services::assistant::{Assistant, AssistantError, CallUsage, RunReport};
use crate::services::conversation::ConversationSession;
use crate::tools::system_status_tool::SystemStatusTool;

const MAX_INPUT_BYTES: usize = 2_048;
const MAX_FRAMED_INPUT_BYTES: usize = MAX_INPUT_BYTES + 2;

// LEARNING: `App` is like a Java class's related fields; its `impl` block below
// supplies methods, while Rust declares data and behavior in separate blocks.
pub struct App {
    assistant: Assistant,
    local_llm: LocalLlm,
    archive_path: PathBuf,
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
        })
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.local_llm.health_check().await?;

        println!("Conversation archive: {}", self.archive_path.display());
        let mut session = ConversationSession::new(self.archive_path.clone());
        if let Some(error) = session.reset().await {
            eprintln!(
                "Warning: local archiving is unavailable; this session will use process-only context: {error}"
            );
        }
        println!("Enter a message, /reset to clear conversation context, or /exit to quit.");

        let stdin = tokio::io::stdin();
        let mut input = BufReader::new(stdin);
        loop {
            print!("You> ");
            io::stdout().flush()?;

            // LEARNING: `InputLine` is an enum, so this `match` handles each
            // variant, much like a Java sealed-type switch. Trimming is only
            // for command recognition; ordinary prompt whitespace is preserved.
            match read_input_line(&mut input).await? {
                InputLine::Eof => return Ok(()),
                InputLine::Rejected(reason) => eprintln!("Input rejected: {}", reason.message()),
                InputLine::Prompt(prompt) => match prompt.trim() {
                    "/exit" => return Ok(()),
                    "/reset" => {
                        if let Some(error) = session.reset().await {
                            eprintln!(
                                "Warning: local archiving is unavailable; this session will use process-only context: {error}"
                            );
                        }
                        println!("Conversation context reset.");
                    }
                    _ => {
                        // LEARNING: Accepted input reaches model execution only
                        // here; this await finishes the turn before the next
                        // line is read.
                        let turn = session.respond(&self.assistant, &prompt).await;
                        // LEARNING: `Option<T>` is `Some(T)` or `None`; these
                        // branches print a warning only when one is present.
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
                                println!("Jarvis: {}", run.response.output());
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
    use super::{InputLine, InputRejection, MAX_INPUT_BYTES, read_input_line};
    use tokio::io::BufReader;

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
}
