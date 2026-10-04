use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use rig_agent::Agent;

use crate::services::assistant::{
    error::AssistantError,
    policy_hook::JarvisPolicyHook,
    run_observer::RunObserver,
    run_report::{AssistantRun, AssistantRunError, ObservedRun, RunOutcome},
};

pub struct Assistant {
    agent: Agent,
}
impl Assistant {
    pub fn new(agent: Agent) -> Self {
        Self { agent }
    }
    pub async fn respond(&self, prompt: &str) -> Result<AssistantRun, AssistantRunError> {
        let observed = Arc::new(Mutex::new(ObservedRun::new()));
        let observer = RunObserver::new(Arc::clone(&observed));
        let started_at = Instant::now();
        let finish_report = |outcome: RunOutcome| {
            let mut observed = observed.lock().unwrap();

            std::mem::replace(&mut *observed, ObservedRun::new())
                .finish(outcome, started_at.elapsed())
        };

        let run = self
            .agent
            .runner(prompt)
            .add_hook(JarvisPolicyHook)
            .add_hook(observer)
            .max_turns(2)
            .tool_concurrency(1)
            .max_tokens(1026)
            .max_invalid_tool_call_retries(0)
            .without_memory()
            .run();

        let response = match within_deadline(std::time::Duration::from_secs(30), run).await {
            Ok(Ok(response)) => response,

            Ok(Err(error)) => {
                return Err(AssistantRunError {
                    error: AssistantError::Prompt(error),
                    report: finish_report(RunOutcome::PromptFailed),
                });
            }

            Err(_) => {
                return Err(AssistantRunError {
                    error: AssistantError::RunTimeout,
                    report: finish_report(RunOutcome::TimedOut),
                });
            }
        };

        if response.output.trim().is_empty() {
            return Err(AssistantRunError {
                error: AssistantError::InvalidResponse,
                report: finish_report(RunOutcome::InvalidResponse),
            });
        }

        let report = finish_report(RunOutcome::Success);

        Ok(AssistantRun { response, report })
    }
}
async fn within_deadline<F: std::future::Future>(
    duration: std::time::Duration,
    future: F,
) -> Result<F::Output, tokio::time::error::Elapsed> {
    tokio::time::timeout(duration, future).await
}
#[cfg(test)]
#[path = "tests/assistant_tests.rs"]
mod assistant_tests;
