use rig_agent::agent::StreamingError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum AssistantError {
    #[error("assistant run timed out")]
    RunTimeout,
    #[error("assistant stream failed: {0}")]
    Stream(#[source] StreamingError),
    #[error("assistant stream ended without a final response")]
    MissingFinalResponse,
    #[error("assistant text receiver was closed")]
    DeltaReceiverClosed,
    #[error("assistant returned an invalid response")]
    InvalidResponse,
}
