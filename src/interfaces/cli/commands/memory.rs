use std::path::PathBuf;

use crate::features::{
    conversation::ConversationSession,
    memory::{EmbedderState, MemoryService},
};
use crate::storage::{ForgetReceipt, ImportState, MemoryId, MemoryKind, MemorySummary, SourceId};

const UNAVAILABLE: &str = "Local memory storage is unavailable.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::interfaces::cli) enum CurrentMemoryFocus {
    Source(SourceId),
    Memory(MemoryId),
    AmbiguousDisplay,
}

pub(in crate::interfaces::cli) fn focus_from_displayed_ids(
    mut ids: impl Iterator<Item = MemoryId>,
) -> Option<CurrentMemoryFocus> {
    let first = ids.next()?;
    if ids.next().is_some() {
        Some(CurrentMemoryFocus::AmbiguousDisplay)
    } else {
        Some(CurrentMemoryFocus::Memory(first))
    }
}

pub(in crate::interfaces::cli) fn focus_from_successful_turn(
    source_id: Option<SourceId>,
) -> Option<CurrentMemoryFocus> {
    source_id.map(CurrentMemoryFocus::Source)
}

pub(in crate::interfaces::cli) async fn list_memories(
    memory: Option<&MemoryService>,
    query: Option<String>,
) -> Vec<MemorySummary> {
    let Some(memory) = memory else {
        println!("{UNAVAILABLE}");
        return Vec::new();
    };
    match memory.list(query, 20).await {
        Ok(records) if records.is_empty() => {
            println!("No memories matched.");
            Vec::new()
        }
        Ok(records) => {
            for record in &records {
                let fact = match record.kind {
                    MemoryKind::Fact => " [fact]",
                    MemoryKind::Excerpt => "",
                };
                println!(
                    "{} [source {}]{fact} {}",
                    record.id.0, record.source_id.0, record.text
                );
            }
            records
        }
        Err(error) => {
            eprintln!("Warning: memories could not be listed: {error}");
            Vec::new()
        }
    }
}

pub(in crate::interfaces::cli) async fn forget_memory_id(
    memory: Option<&MemoryService>,
    session: &mut ConversationSession,
    raw_id: &str,
    focus: &mut Option<CurrentMemoryFocus>,
    recent: &mut Vec<MemorySummary>,
) {
    let Ok(id) = raw_id.parse::<i64>() else {
        println!("Use /forget followed by a numeric memory ID.");
        return;
    };
    let Some(memory) = memory else {
        println!("{UNAVAILABLE} Nothing was forgotten.");
        return;
    };
    match memory.forget(MemoryId(id)).await {
        Ok(receipt) => apply_forget_receipt(&receipt, session, focus, recent),
        Err(error) => eprintln!("Warning: memory was not forgotten: {error}"),
    }
}

pub(in crate::interfaces::cli) async fn forget_focus(
    memory: Option<&MemoryService>,
    session: &mut ConversationSession,
    focus: &mut Option<CurrentMemoryFocus>,
    recent: &mut Vec<MemorySummary>,
) {
    let Some(memory) = memory else {
        println!("{UNAVAILABLE} Nothing was forgotten.");
        return;
    };
    let result = match *focus {
        Some(CurrentMemoryFocus::Source(source_id)) => memory.forget_source(source_id).await,
        Some(CurrentMemoryFocus::Memory(memory_id)) => memory.forget(memory_id).await,
        Some(CurrentMemoryFocus::AmbiguousDisplay) => {
            println!(
                "The current /memories display has multiple records. Use /forget <id> to choose one; nothing was forgotten."
            );
            return;
        }
        None => {
            println!(
                "There is no current memory to forget. Use /memories to choose a record; nothing was forgotten."
            );
            return;
        }
    };
    match result {
        Ok(receipt) => apply_forget_receipt(&receipt, session, focus, recent),
        Err(error) => eprintln!("Warning: the memory was not forgotten: {error}"),
    }
}

/// Forgets the single memory whose text contains `detail`, searching the last
/// /memories display first and then the store. Ambiguity forgets nothing.
pub(in crate::interfaces::cli) async fn forget_detail(
    memory: Option<&MemoryService>,
    session: &mut ConversationSession,
    detail: &str,
    focus: &mut Option<CurrentMemoryFocus>,
    recent: &mut Vec<MemorySummary>,
) {
    let Some(memory) = memory else {
        println!("{UNAVAILABLE} Nothing was forgotten.");
        return;
    };
    let needle = detail.to_lowercase();
    let mut candidates = recent
        .iter()
        .filter(|record| record.text.to_lowercase().contains(&needle))
        .map(|record| record.id)
        .collect::<Vec<_>>();
    match memory.list(Some(detail.to_owned()), 2).await {
        Ok(records) => candidates.extend(records.into_iter().map(|record| record.id)),
        Err(error) => {
            eprintln!("Warning: forget detail could not be resolved: {error}");
            return;
        }
    }
    candidates.sort();
    candidates.dedup();
    match candidates.as_slice() {
        [id] => match memory.forget(*id).await {
            Ok(receipt) => apply_forget_receipt(&receipt, session, focus, recent),
            Err(error) => eprintln!("Warning: the selected memory was not forgotten: {error}"),
        },
        [] => println!("I could not find one memory matching that detail; nothing was forgotten."),
        _ => println!(
            "That detail matches multiple memories. Use /memories to choose an ID; nothing was forgotten."
        ),
    }
}

fn apply_forget_receipt(
    receipt: &ForgetReceipt,
    session: &mut ConversationSession,
    focus: &mut Option<CurrentMemoryFocus>,
    recent: &mut Vec<MemorySummary>,
) {
    if receipt.is_empty() {
        println!("No memory matched; nothing was forgotten.");
        return;
    }
    println!(
        "Forgot {} memories from {} sources; {} archived turns were affected.",
        receipt.forgotten_memories, receipt.forgotten_sources, receipt.affected_turns,
    );
    // Active history may quote what was just forgotten; drop it whole.
    session.clear_history();
    *focus = None;
    recent.clear();
}

pub(in crate::interfaces::cli) async fn import_memory(
    memory: Option<&MemoryService>,
    session: &mut ConversationSession,
    raw_path: &str,
    focus: &mut Option<CurrentMemoryFocus>,
    recent: &mut Vec<MemorySummary>,
) {
    let Some(memory) = memory else {
        println!("{UNAVAILABLE} The import was not saved.");
        return;
    };
    match memory.import_file(&PathBuf::from(raw_path)).await {
        Ok(receipt) => {
            let state = match receipt.state {
                ImportState::Created => "created",
                ImportState::AlreadyCurrent => "already current",
                ImportState::Reactivated => "reactivated",
                ImportState::Forgotten => {
                    println!("This file version was forgotten earlier and was not imported again.");
                    return;
                }
            };
            if receipt.state != ImportState::AlreadyCurrent {
                // A changed file can contradict facts in the active history.
                session.clear_history();
            }
            *focus = Some(CurrentMemoryFocus::Source(receipt.source_id));
            recent.clear();
            println!(
                "Imported source {} ({} chunks, {state}).",
                receipt.source_id.0, receipt.chunks
            );
        }
        Err(error) => eprintln!("Warning: import was not saved: {error}"),
    }
}

pub(in crate::interfaces::cli) async fn print_memory_status(memory: Option<&MemoryService>) {
    let Some(memory) = memory else {
        println!("{UNAVAILABLE}");
        return;
    };
    match memory.status().await {
        Ok(status) => {
            let embedder = match status.embedder {
                EmbedderState::Loading => "loading",
                EmbedderState::Ready => "ready",
                EmbedderState::Unavailable => "unavailable (keyword-only recall)",
            };
            println!(
                "Memory: {} memories from {} conversations and {} imports; {} awaiting embedding; embedder {embedder}.",
                status.stats.memories,
                status.stats.conversation_sources,
                status.stats.import_sources,
                status.stats.pending_embeddings,
            );
        }
        Err(error) => eprintln!("Warning: memory status is unavailable: {error}"),
    }
}

pub(in crate::interfaces::cli) async fn rebuild_memory_index(memory: Option<&MemoryService>) {
    let Some(memory) = memory else {
        println!("{UNAVAILABLE}");
        return;
    };
    match memory.rebuild().await {
        Ok(reset) => println!("Queued {reset} memories for re-embedding while idle."),
        Err(error) => eprintln!("Warning: memory rebuild did not start: {error}"),
    }
}
