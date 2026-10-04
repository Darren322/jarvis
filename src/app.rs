use rig_agent::AgentBuilder;

use crate::clients::local_llm::LocalLlm;
use crate::config::AppConfig;
use crate::services::assistant::{Assistant, CallUsage, RunReport};
use crate::tools::system_status_tool::SystemStatusTool;

pub struct App {
    assistant: Assistant,
    local_llm: LocalLlm,
}

impl App {
    pub fn new(config: &AppConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let local_llm = LocalLlm::new(config)?;
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

        Ok(Self {
            assistant,
            local_llm,
        })
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.local_llm.health_check().await?;

        match self
            .assistant
            .respond("Check the current system status using the system_status tool.")
            .await
        {
            Ok(run) => {
                print_run_diagnostics(&run.report);
                println!("Response: {}", run.response);
                Ok(())
            }
            Err(error) => {
                print_run_diagnostics(&error.report);
                Err(error as Box<dyn std::error::Error>)
            }
        }
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
