#[derive(Debug)]
pub enum WorkerError {
    Spawn(std::io::Error),
    MissingStdin,
    MissingStdout,
    StartupTimeout,
    StartupEof,
    ResponseTooLarge,
    Read(std::io::Error),
    InvalidJson(serde_json::Error),
    UnexpectedStartupFrame,
    UnsupportedProtocol(u32),
    InvalidConfiguration,
    ChildStatus(std::io::Error),
    WorkerExited,
    InvalidState,
    CleanupFailed,
    CleanupTimedOut,
    Write(std::io::Error),
    SynthesisTimeout,
    WorkerEof,
    WorkerReported,
    UnexpectedResponse,
    MismatchedResponseId,
    InvalidAudioMetadata,
    RequestTooLarge,
}

impl std::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(_) => write!(f, "failed to spawn TTS worker"),
            Self::MissingStdin => write!(f, "TTS worker stdin unavailable"),
            Self::MissingStdout => write!(f, "TTS worker stdout unavailable"),
            Self::StartupTimeout => write!(f, "TTS worker startup timed out"),
            Self::StartupEof => write!(f, "TTS worker exited before readiness"),
            Self::ResponseTooLarge => write!(f, "TTS worker response exceeded limit"),
            Self::Read(_) => write!(f, "failed to read from TTS worker"),
            Self::InvalidJson(_) => write!(f, "TTS worker returned invalid JSON"),
            Self::UnexpectedStartupFrame => {
                write!(f, "TTS worker returned unexpected startup frame")
            }
            Self::UnsupportedProtocol(version) => {
                write!(f, "unsupported TTS worker protocol: {version}")
            }
            Self::InvalidConfiguration => {
                write!(f, "TTS worker reported unexpected configuration")
            }
            Self::ChildStatus(_) => write!(f, "failed to inspect TTS worker process"),
            Self::WorkerExited => write!(f, "TTS worker exited unexpectedly"),
            Self::InvalidState => write!(f, "TTS worker is not ready for this operation"),
            Self::CleanupFailed => write!(f, "TTS worker cleanup failed"),
            Self::CleanupTimedOut => write!(f, "TTS worker cleanup timed out"),
            Self::Write(_) => write!(f, "failed to write to TTS worker"),
            Self::SynthesisTimeout => write!(f, "TTS synthesis timed out"),
            Self::WorkerEof => write!(f, "TTS worker exited during synthesis"),
            Self::WorkerReported => write!(f, "TTS worker reported a synthesis error"),
            Self::UnexpectedResponse => write!(f, "TTS worker returned unexpected response"),
            Self::MismatchedResponseId => write!(f, "TTS worker returned mismatched response ID"),
            Self::InvalidAudioMetadata => write!(f, "TTS worker returned invalid audio metadata"),
            Self::RequestTooLarge => write!(f, "TTS worker request exceeded limit"),
        }
    }
}

impl std::error::Error for WorkerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn(error)
            | Self::Read(error)
            | Self::ChildStatus(error)
            | Self::Write(error) => Some(error),
            Self::InvalidJson(error) => Some(error),
            _ => None,
        }
    }
}
