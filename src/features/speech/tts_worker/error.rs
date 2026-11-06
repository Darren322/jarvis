use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum WorkerError {
    #[error("failed to spawn tts worker")]
    Spawn(#[source] std::io::Error),
    #[error("tts worker stdin unavailable")]
    MissingStdin,
    #[error("tts worker stdout unavailable")]
    MissingStdout,
    #[error("tts worker startup timed out")]
    StartupTimeout,
    #[error("tts worker exited before readiness")]
    StartupEof,
    #[error("tts worker response exceeded limit")]
    ResponseTooLarge,
    #[error("failed to read from tts worker")]
    Read(#[source] std::io::Error),
    #[error("tts worker returned invalid json")]
    InvalidJson(#[source] serde_json::Error),
    #[error("tts worker returned unexpected startup frame")]
    UnexpectedStartupFrame,
    #[error("unsupported tts worker protocol: {0}")]
    UnsupportedProtocol(u32),
    #[error("tts worker reported unexpected configuration")]
    InvalidConfiguration,
    #[error("failed to inspect tts worker process")]
    ChildStatus(#[source] std::io::Error),
    #[error("tts worker exited unexpectedly")]
    WorkerExited,
    #[error("tts worker is not ready for this operation")]
    InvalidState,
    #[error("tts worker cleanup failed")]
    CleanupFailed,
    #[error("tts worker cleanup timed out")]
    CleanupTimedOut,
    #[error("failed to write to tts worker")]
    Write(#[source] std::io::Error),
    #[error("tts synthesis timed out")]
    SynthesisTimeout,
    #[error("tts worker exited during synthesis")]
    WorkerEof,
    #[error("tts worker reported a synthesis error")]
    WorkerReported,
    #[error("tts worker returned unexpected response")]
    UnexpectedResponse,
    #[error("tts worker returned mismatched response id")]
    MismatchedResponseId,
    #[error("tts worker returned invalid audio metadata")]
    InvalidAudioMetadata,
    #[error("tts worker request exceeded limit")]
    RequestTooLarge,
}
