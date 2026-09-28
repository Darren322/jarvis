use rig_agent::completion::PromptError;
use std::{
    error::Error,
    fmt::{Display, Formatter},
};
#[derive(Debug)]
pub enum AssistantError {
    RunTimeout,
    Prompt(PromptError),
    InvalidResponse,
}

impl Display for AssistantError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RunTimeout => write!(f, "assistant run timed out"),
            Self::Prompt(error) => write!(f, "assistant prompt failed: {error}"),
            Self::InvalidResponse => write!(f, "assistant returned an invalid response"),
        }
    }
}

impl Error for AssistantError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Prompt(error) => Some(error),
            _ => None,
        }
    }
}
