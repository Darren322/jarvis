mod agent;
mod error;
mod policy;
mod policy_hook;
mod run_observer;
mod run_report;

pub(crate) use agent::Assistant;
pub(crate) use agent::AssistantTextDelta;
pub(crate) use error::AssistantError;
pub(crate) use run_report::{AssistantRun, AssistantRunError};
pub(crate) use run_report::{CallUsage, RunReport};

/// Jarvis's system prompt. Kept as text so it can be edited without touching code.
pub(crate) const SYSTEM_PREAMBLE: &str = include_str!("preamble.txt");
