//! The facts list: short facts about the user that go into every request.
//!
//! Facts are their own table, apart from the searchable `memories`, so no search
//! returns them. Everything is ordered by row id (timestamps can tie), and the
//! size rules of fast-memory D8 are enforced here, inside the insert transaction.

// Tickets 04 to 06 add the callers of `insert` and `remove`. Once nothing here is
// unused this expectation fails the build, which is the cue to delete it.
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "fast-memory 04 to 06 add the callers")
)]

use rusqlite::{Transaction, TransactionBehavior, params};
use tokio_rusqlite::Connection;

use super::error::StorageError;
use super::time::unix_time_ms;

/// The most bytes one fact's text may have, after trimming.
pub(crate) const MAX_FACT_BYTES: usize = 200;
/// The most bytes the facts list may take in a request.
///
/// It counts the fact lines of the request message: for each fact, its text bytes
/// plus `FACT_LINE_OVERHEAD_BYTES`. The message adds one fixed header line, so a
/// full list is the header plus at most this many bytes (about 300 tokens).
pub(crate) const MAX_FACTS_BYTES: usize = 1_200;
/// What the request message adds around each fact's text: the `- ` bullet and the
/// line's newline. `features/memory/facts.rs` renders the lines and tests that it
/// adds exactly this.
pub(crate) const FACT_LINE_OVERHEAD_BYTES: usize = 3;

// A lone fact of the largest size must fit, or an empty list could refuse it.
const _: () = assert!(MAX_FACT_BYTES + FACT_LINE_OVERHEAD_BYTES <= MAX_FACTS_BYTES);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct FactId(pub(crate) i64);

/// Where a fact came from. D8 drops `Noted` facts to make room and never `Stated`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FactOrigin {
    /// Darren gave it ("remember that ...").
    Stated,
    /// Jarvis noted it by itself while idle.
    Noted,
}

impl FactOrigin {
    fn as_sql(self) -> &'static str {
        match self {
            Self::Stated => "stated",
            Self::Noted => "noted",
        }
    }

    fn from_sql(value: &str) -> Result<Self, rusqlite::Error> {
        match value {
            "stated" => Ok(Self::Stated),
            "noted" => Ok(Self::Noted),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Fact {
    pub(crate) id: FactId,
    pub(crate) text: String,
    pub(crate) origin: FactOrigin,
    pub(crate) created_at_unix_ms: i64,
}

impl Fact {
    /// What this fact takes of `MAX_FACTS_BYTES`.
    pub(crate) fn line_bytes(&self) -> usize {
        self.text.len() + FACT_LINE_OVERHEAD_BYTES
    }
}

/// What a successful insert did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AddedFact {
    pub(crate) id: FactId,
    /// Noted facts dropped to make room, oldest first.
    pub(crate) dropped: Vec<Fact>,
}

/// Why a fact was not added. Every refusal leaves the list unchanged.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FactError {
    #[error("a fact cannot be empty")]
    Empty,
    #[error("a fact must be one line")]
    NotOneLine,
    #[error("a fact exceeds the {MAX_FACT_BYTES}-byte limit")]
    TooLong,
    #[error("the facts list is full of stated facts ({MAX_FACTS_BYTES}-byte cap)")]
    ListFull,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// SQL for the facts list.
#[derive(Clone)]
pub(crate) struct FactsRepository {
    pub(super) connection: Connection,
}

impl FactsRepository {
    /// Adds a fact (the text is trimmed) and drops noted facts, oldest first, until
    /// it fits the list cap. Stated facts are never dropped: if they alone leave no
    /// room the insert is refused and nothing is dropped. The whole decision is one
    /// immediate transaction.
    pub(crate) async fn insert(
        &self,
        text: &str,
        origin: FactOrigin,
    ) -> Result<AddedFact, FactError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(FactError::Empty);
        }
        if text.contains(['\n', '\r']) {
            return Err(FactError::NotOneLine);
        }
        if text.len() > MAX_FACT_BYTES {
            return Err(FactError::TooLong);
        }
        let text = text.to_owned();
        let created_at_unix_ms = unix_time_ms()?;
        self.connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let outcome = insert_within(&transaction, &text, origin, created_at_unix_ms)?;
                // A refusal drops the transaction unfinished, which rolls it back.
                if outcome.is_ok() {
                    transaction.commit()?;
                }
                Ok(outcome)
            })
            .await
            .map_err(StorageError::from)?
    }

    /// Every fact, oldest first (by id).
    pub(crate) async fn list(&self) -> Result<Vec<Fact>, StorageError> {
        let facts = self
            .connection
            .call(|connection| list_all(connection))
            .await?;
        Ok(facts)
    }

    /// Removes one fact and returns it, or `None` when no fact has that id.
    pub(crate) async fn remove(&self, id: FactId) -> Result<Option<Fact>, StorageError> {
        let removed = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let fact = list_all(&transaction)?
                    .into_iter()
                    .find(|fact| fact.id == id);
                if fact.is_some() {
                    transaction.execute("DELETE FROM facts WHERE id = ?1", [id.0])?;
                }
                transaction.commit()?;
                Ok(fact)
            })
            .await?;
        Ok(removed)
    }
}

fn list_all(connection: &rusqlite::Connection) -> rusqlite::Result<Vec<Fact>> {
    let mut statement =
        connection.prepare("SELECT id, text, origin, created_at_unix_ms FROM facts ORDER BY id")?;
    statement
        .query_map([], |row| {
            Ok(Fact {
                id: FactId(row.get(0)?),
                text: row.get(1)?,
                origin: FactOrigin::from_sql(&row.get::<_, String>(2)?)?,
                created_at_unix_ms: row.get(3)?,
            })
        })?
        .collect()
}

/// D8 inside the caller's transaction. The outer `Result` is SQLite failing; the
/// inner one is the refusal, decided before anything is deleted.
fn insert_within(
    transaction: &Transaction<'_>,
    text: &str,
    origin: FactOrigin,
    created_at_unix_ms: i64,
) -> rusqlite::Result<Result<AddedFact, FactError>> {
    let facts = list_all(transaction)?;
    let used: usize = facts.iter().map(Fact::line_bytes).sum();
    let new_line = text.len() + FACT_LINE_OVERHEAD_BYTES;
    let mut missing = (used + new_line).saturating_sub(MAX_FACTS_BYTES);

    // Both origins make room the same way: only noted facts are candidates, and
    // every one of them is older than the new fact.
    let mut dropped = Vec::new();
    for fact in facts.into_iter().filter(|f| f.origin == FactOrigin::Noted) {
        if missing == 0 {
            break;
        }
        missing = missing.saturating_sub(fact.line_bytes());
        dropped.push(fact);
    }
    if missing > 0 {
        return Ok(Err(FactError::ListFull));
    }

    for fact in &dropped {
        transaction.execute("DELETE FROM facts WHERE id = ?1", [fact.id.0])?;
    }
    transaction.execute(
        "INSERT INTO facts (text, origin, created_at_unix_ms) VALUES (?1, ?2, ?3)",
        params![text, origin.as_sql(), created_at_unix_ms],
    )?;
    Ok(Ok(AddedFact {
        id: FactId(transaction.last_insert_rowid()),
        dropped,
    }))
}

#[cfg(test)]
#[path = "../../tests/unit/storage/facts_tests.rs"]
mod tests;
