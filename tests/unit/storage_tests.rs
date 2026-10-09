use std::{
    error::Error,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{ArchiveOutcome, ConversationArchive, StorageError};

fn database_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after Unix epoch")
        .as_nanos();

    std::env::temp_dir().join(format!(
        "jarvis-storage-{label}-{}-{unique}.sqlite3",
        std::process::id()
    ))
}

fn remove_database(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn saves_safe_outcomes_and_preserves_sessions_after_reopen() -> Result<(), Box<dyn Error>> {
    let path = database_path("durability");
    let first = ConversationArchive::open(&path).await?;
    let first_session_id = first.session_id;

    first
        .append_turn(
            "What is the status?",
            ArchiveOutcome::Completed("All systems are available.".to_owned()),
        )
        .await?;
    first
        .append_turn("A request that fails", ArchiveOutcome::PromptFailed)
        .await?;
    first.connection.clone().close().await?;
    drop(first);

    let second = ConversationArchive::open(&path).await?;
    let second_session_id = second.session_id;
    assert_ne!(first_session_id, second_session_id);

    let (sessions, turns) = second
        .connection
        .call(|connection| -> rusqlite::Result<_> {
            let mut session_statement =
                connection.prepare("SELECT id FROM sessions ORDER BY id")?;
            let sessions = session_statement
                .query_map([], |row| row.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let mut turn_statement = connection.prepare(
                "SELECT session_id, user_text, assistant_text, outcome FROM turns ORDER BY id",
            )?;
            let turns = turn_statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            Ok((sessions, turns))
        })
        .await?;

    assert_eq!(sessions, vec![first_session_id, second_session_id]);
    assert_eq!(
        turns,
        vec![
            (
                first_session_id,
                "What is the status?".to_owned(),
                Some("All systems are available.".to_owned()),
                "completed".to_owned(),
            ),
            (
                first_session_id,
                "A request that fails".to_owned(),
                None,
                "prompt_failed".to_owned(),
            ),
        ]
    );

    second.connection.clone().close().await?;
    drop(second);
    remove_database(&path);

    Ok(())
}

#[tokio::test]
async fn rejects_newer_schema_and_rolls_back_partial_initialization() -> Result<(), Box<dyn Error>>
{
    let unsupported_path = database_path("unsupported");
    let connection = rusqlite::Connection::open(&unsupported_path)?;
    connection.pragma_update(None, "user_version", 4)?;
    drop(connection);

    let error = match ConversationArchive::open(&unsupported_path).await {
        Ok(_) => panic!("schema version 4 should be rejected"),
        Err(error) => error,
    };
    assert!(matches!(error, StorageError::UnsupportedSchemaVersion(4)));

    let connection = rusqlite::Connection::open(&unsupported_path)?;
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let table_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(version, 4);
    assert_eq!(table_count, 0);
    drop(connection);
    remove_database(&unsupported_path);

    let partial_path = database_path("partial");
    let connection = rusqlite::Connection::open(&partial_path)?;
    connection.execute_batch("CREATE TABLE turns (existing INTEGER);")?;
    drop(connection);

    assert!(ConversationArchive::open(&partial_path).await.is_err());

    let connection = rusqlite::Connection::open(&partial_path)?;
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let sessions_exist: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sessions')",
        [],
        |row| row.get(0),
    )?;
    let turns_exist: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'turns')",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(version, 0);
    assert!(!sessions_exist);
    assert!(turns_exist);
    drop(connection);
    remove_database(&partial_path);

    Ok(())
}
