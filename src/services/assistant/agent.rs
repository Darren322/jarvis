use std::time::{Duration, Instant};

use rig_agent::{Agent, agent::PromptResponse};
use rig_core::message::Message;

use crate::services::assistant::{
    error::AssistantError,
    policy_hook::JarvisPolicyHook,
    run_observer::RunObserver,
    run_report::{AssistantRun, AssistantRunError, RunOutcome},
};

// LEARNING: `Assistant` keeps a reusable Rig `Agent`; each `respond` call
// configures a fresh runner for one bounded conversation turn.
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
        history: &[Message],
    ) -> Result<AssistantRun, Box<AssistantRunError>> {
        let observer = RunObserver::default();
        let started_at = Instant::now();
        // LEARNING: `JarvisPolicyHook` makes policy decisions; `RunObserver`
        // only records events. `observer.clone()` shares `Arc<Mutex<_>>`, so
        // the hook registered with Rig records what this method later finalizes.
        //
        // LEARNING: `.runner(prompt)` configures a fresh `AgentRunner`;
        // `.run().await` is what starts its model/tool loop. Rig 0.42.0's
        // `.history(...)` supplies native prior `Message`s for this run.
        // `.without_memory()` disables Rig's separate memory path so Jarvis's
        // session remains the history owner.
        //
        // LEARNING: `max_turns(2)` caps total model calls; the policy separately
        // allows at most one status-tool execution. `tool_concurrency(1)` caps
        // simultaneous tools, not total attempts. `max_tokens` caps output for
        // each model request, not the whole conversation.
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            self.agent
                .runner(prompt)
                .history(history.iter().cloned())
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

        // LEARNING: `timeout` wraps the run's `Result`: `Ok(Ok(response))` is
        // success, `Ok(Err(error))` is a Rig failure, and `Err(_)` means Tokio
        // stopped waiting. It does not prove the remote model cancelled work.
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

// LEARNING: `Box<AssistantRunError>` owns the typed error value on the heap;
// this is not Java's primitive boxing. The error still carries its run report.
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
#[path = "../../../tests/unit/services/assistant/assistant_tests.rs"]
mod assistant_tests;
