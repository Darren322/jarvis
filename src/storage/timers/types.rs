use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};

use crate::storage::StorageError;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TimerId(pub(crate) i64);

/// Where a timer is in its life. Stored as text; a CHECK in the table keeps it to
/// these four.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TimerState {
    /// Proposed, waiting for Darren to confirm (timers D4).
    Pending,
    /// Confirmed and counting down.
    Running,
    /// Reached its end time; shown for a while (D19), then dropped from the list.
    Ended,
    /// Rejected, cancelled or removed. Never listed.
    Cancelled,
}

impl TimerState {
    fn as_sql(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Ended => "ended",
            Self::Cancelled => "cancelled",
        }
    }
}

impl ToSql for TimerState {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_sql()))
    }
}

impl FromSql for TimerState {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value.as_str()? {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "ended" => Ok(Self::Ended),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Timer {
    pub(crate) id: TimerId,
    /// What the timer is for. Its length is checked by the caller, not here.
    pub(crate) label: Option<String>,
    pub(crate) ends_at_unix_ms: i64,
    pub(crate) created_at_unix_ms: i64,
    pub(crate) state: TimerState,
}

/// Why a timer operation changed nothing.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TimerError {
    #[error("{limit} timers are already pending or running")]
    LimitReached { limit: usize },
    /// The id is missing (`found` is `None`) or in a state the change does not start from.
    #[error("timer {} is not in a state that allows this", id.0)]
    NotInExpectedState {
        id: TimerId,
        found: Option<TimerState>,
    },
    #[error(transparent)]
    Storage(#[from] StorageError),
}
