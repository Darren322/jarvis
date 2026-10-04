use std::time::{Duration, Instant};

use rig_agent::{Agent, agent::PromptResponse};

use crate::services::assistant::{
    error::AssistantError,
    policy_hook::JarvisPolicyHook,
    run_observer::RunObserver,
    run_report::{AssistantRun, AssistantRunError, RunOutcome},
};

pub struct Assistant {
    agent: Agent,
}

impl Assistant {
    pub fn new(agent: Agent) -> Self {
        Self { agent }
    }

    pub(crate) async fn respond(
        &self,
        prompt: &str,
    ) -> Result<AssistantRun, Box<AssistantRunError>> {
        let observer = RunObserver::default();
        let started_at = Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            self.agent
                .runner(prompt)
                .add_hook(JarvisPolicyHook)
                .add_hook(observer.clone())
                .max_turns(2)
                .tool_concurrency(1)
                .max_tokens(1026)
                .max_invalid_tool_call_retries(0)
                .without_memory()
                .run(),
        )
        .await;

        match result {
            Ok(Ok(response)) => finalize_response(response, observer, started_at.elapsed()),
            Ok(Err(error)) => Err(Box::new(AssistantRunError {
                error: AssistantError::Prompt(error),
                report: observer.finish(RunOutcome::PromptFailed, started_at.elapsed()),
            })),
            Err(_) => Err(Box::new(AssistantRunError {
                error: AssistantError::RunTimeout,
                report: observer.finish(RunOutcome::TimedOut, started_at.elapsed()),
            })),
        }
    }
}

fn finalize_response(
    response: PromptResponse,
    observer: RunObserver,
    elapsed: Duration,
) -> Result<AssistantRun, Box<AssistantRunError>> {
    let invalid_response = response.output.trim().is_empty();
    let outcome = if invalid_response {
        RunOutcome::InvalidResponse
    } else {
        RunOutcome::Success
    };
    let report = observer.finish(outcome, elapsed);

    if invalid_response {
        Err(Box::new(AssistantRunError {
            error: AssistantError::InvalidResponse,
            report,
        }))
    } else {
        Ok(AssistantRun { response, report })
    }
}

#[cfg(test)]
#[path = "tests/assistant_tests.rs"]
mod assistant_tests;
