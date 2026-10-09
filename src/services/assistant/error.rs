use rig_agent::agent::StreamingError;
#[cfg(test)]
use rig_agent::completion::PromptError;
use std::{
    error::Error,
    fmt::{Display, Formatter},
};
#[derive(Debug)]
pub enum AssistantError {
    RunTimeout,
    #[cfg(test)]
    Prompt(PromptError),
    Stream(StreamingError),
    MissingFinalResponse,
    DeltaReceiverClosed,
    InvalidResponse,
}

impl Display for AssistantError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RunTimeout => write!(f, "assistant run timed out"),
            #[cfg(test)]
            Self::Prompt(error) => write!(f, "assistant prompt failed: {error}"),
            Self::Stream(error) => write!(f, "assistant stream failed: {error}"),
            Self::MissingFinalResponse => {
                write!(f, "assistant stream ended without a final response")
            }
            Self::DeltaReceiverClosed => write!(f, "assistant text receiver was closed"),
            Self::InvalidResponse => write!(f, "assistant returned an invalid response"),
        }
    }
}

impl Error for AssistantError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            #[cfg(test)]
            Self::Prompt(error) => Some(error),
            Self::Stream(error) => Some(error),
            _ => None,
        }
    }
}
