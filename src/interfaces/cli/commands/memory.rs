use std::path::PathBuf;

use crate::conversation::ConversationSession;
use crate::memory::MemoryService;
use crate::storage::{
    CorrectionState, JobId, MemoryId, MemoryListFilter, MemoryRecordStatus, MemorySummary, SourceId,
};

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
    memory: &Option<MemoryService>,
    query: Option<String>,
) -> Vec<MemorySummary> {
    let Some(memory) = memory.as_ref() else {
        println!("Local memory storage is unavailable.");
        return Vec::new();
    };
    match memory
        .list(MemoryListFilter {
            query,
            status: Some(MemoryRecordStatus::Active),
            limit: 20,
        })
        .await
    {
        Ok(records) if records.is_empty() => {
            println!("No active memories matched.");
            Vec::new()
        }
        Ok(records) => {
            for record in &records {
                if record.correction_state == CorrectionState::NeedsReview {
                    println!(
                        "{} [source {}] [NeedsReview] {}",
                        record.id.0, record.source_id.0, record.text
                    );
                } else {
                    println!(
                        "{} [source {}] {}",
                        record.id.0, record.source_id.0, record.text
                    );
                }
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
    memory: &mut Option<MemoryService>,
    session: &mut ConversationSession,
    raw_id: &str,
    focus: &mut Option<CurrentMemoryFocus>,
    recent: &mut Vec<MemorySummary>,
) {
    let Ok(id) = raw_id.parse::<i64>() else {
        println!("Use /forget followed by a numeric memory ID.");
        return;
    };
    let Some(memory) = memory.as_mut() else {
        println!("Local memory storage is unavailable; nothing was forgotten.");
        return;
    };
    match memory.forget(MemoryId(id)).await {
        Ok(receipt) => apply_forget_receipt(&receipt, session, focus, recent),
        Err(error) => eprintln!("Warning: memory was not forgotten: {error}"),
    }
}

pub(in crate::interfaces::cli) async fn forget_focus(
    memory: &mut Option<MemoryService>,
    session: &mut ConversationSession,
    target: Option<CurrentMemoryFocus>,
    focus: &mut Option<CurrentMemoryFocus>,
    recent: &mut Vec<MemorySummary>,
) {
    let Some(memory) = memory.as_mut() else {
        println!("Local memory storage is unavailable; nothing was forgotten.");
        return;
    };
    match target {
        Some(CurrentMemoryFocus::Source(source_id)) => {
            match memory.forget_source(source_id).await {
                Ok(receipt) => apply_forget_receipt(&receipt, session, focus, recent),
                Err(error) => eprintln!("Warning: the current source was not forgotten: {error}"),
            }
        }
        Some(CurrentMemoryFocus::Memory(memory_id)) => match memory.forget(memory_id).await {
            Ok(receipt) => apply_forget_receipt(&receipt, session, focus, recent),
            Err(error) => eprintln!("Warning: the displayed memory was not forgotten: {error}"),
        },
        Some(CurrentMemoryFocus::AmbiguousDisplay) => println!(
            "The current /memories display has multiple active records. Use /forget <id> to choose one; nothing was forgotten."
        ),
        None => println!(
            "There is no current foreground memory target. Use /memories to choose a record; nothing was forgotten."
        ),
    }
}

pub(in crate::interfaces::cli) async fn forget_detail(
    memory: &mut Option<MemoryService>,
    session: &mut ConversationSession,
    recent: &[MemorySummary],
    detail: &str,
    focus: &mut Option<CurrentMemoryFocus>,
    recent_display: &mut Vec<MemorySummary>,
) {
    let Some(memory) = memory.as_mut() else {
        println!("Local memory storage is unavailable; nothing was forgotten.");
        return;
    };
    let needle = detail.to_lowercase();
    let active = match memory
        .list(MemoryListFilter {
            query: None,
            status: Some(MemoryRecordStatus::Active),
            limit: 50,
        })
        .await
    {
        Ok(records) => records,
        Err(error) => {
            eprintln!("Warning: forget detail could not be resolved: {error}");
            return;
        }
    };
    let mut candidates = recent
        .iter()
        .filter(|record| {
            record.status == MemoryRecordStatus::Active
                && record.text.to_lowercase().contains(&needle)
                && active.iter().any(|current| current.id == record.id)
        })
        .map(|record| record.id)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|id| id.0);
    candidates.dedup();
    match memory
        .list(MemoryListFilter {
            query: Some(detail.to_owned()),
            status: Some(MemoryRecordStatus::Active),
            limit: 2,
        })
        .await
    {
        Ok(records) => candidates.extend(records.into_iter().map(|record| record.id)),
        Err(error) => {
            eprintln!("Warning: forget detail could not be resolved: {error}");
            return;
        }
    }
    candidates.sort_by_key(|id| id.0);
    candidates.dedup();
    match candidates.as_slice() {
        [id] => match memory.forget(*id).await {
            Ok(receipt) => apply_forget_receipt(&receipt, session, focus, recent_display),
            Err(error) => eprintln!("Warning: the selected memory was not forgotten: {error}"),
        },
        [] => println!(
            "I could not find one active memory matching that detail; nothing was forgotten."
        ),
        _ => println!(
            "That detail matches multiple active memories. Use /memories to choose an ID; nothing was forgotten."
        ),
    }
}

fn apply_forget_receipt(
    receipt: &crate::storage::ForgetReceipt,
    session: &mut ConversationSession,
    focus: &mut Option<CurrentMemoryFocus>,
    recent: &mut Vec<MemorySummary>,
) {
    if print_forget_receipt(receipt) {
        session.clear_history();
        *focus = None;
        recent.clear();
    }
}

fn print_forget_receipt(receipt: &crate::storage::ForgetReceipt) -> bool {
    if receipt.forgotten_memory_count == 0 && receipt.suppressed_source_count == 0 {
        println!("No active memory matched; nothing was forgotten.");
        return false;
    }
    println!(
        "Forgot {} memories and suppressed {} sources; {} archived turns were affected{}.",
        receipt.forgotten_memory_count,
        receipt.suppressed_source_count,
        receipt.affected_turn_count,
        if receipt.ids_truncated {
            " (ID lists truncated)"
        } else {
            ""
        },
    );
    true
}

pub(in crate::interfaces::cli) async fn import_memory(
    memory: &mut Option<MemoryService>,
    session: &mut ConversationSession,
    raw_path: &str,
    focus: &mut Option<CurrentMemoryFocus>,
    recent: &mut Vec<MemorySummary>,
) {
    let Some(memory) = memory.as_mut() else {
        println!("Local memory storage is unavailable; the import was not saved.");
        return;
    };
    let path = PathBuf::from(raw_path);
    match memory.import_file(&path).await {
        Ok(receipt) => {
            if receipt.state != "already_current" {
                // An updated or reactivated file can invalidate facts retained by
                // the current native history, so discard complete batches.
                session.clear_history();
            }
            *focus = Some(CurrentMemoryFocus::Source(receipt.source_id));
            recent.clear();
            println!(
                "Imported source {} ({} parts, state {}).",
                receipt.source_id.0, receipt.parts, receipt.state
            );
        }
        Err(error) => eprintln!("Warning: import was not saved: {error}"),
    }
}

pub(in crate::interfaces::cli) async fn print_memory_status(memory: &mut Option<MemoryService>) {
    let Some(memory) = memory.as_mut() else {
        println!("Local memory storage is unavailable.");
        return;
    };
    match memory.status().await {
        Ok(status) => {
            println!(
                "Memory: backend_available={} queued={} running={} failed={} pending_sources={} pending_parts={} pending_projection={} backfill_complete={}",
                status.backend_available,
                status.stats.queued_extractions,
                status.stats.running_extractions,
                status.stats.failed_extractions,
                status.stats.pending_sources,
                status.stats.pending_source_parts,
                status.stats.pending_projections,
                status.stats.backfill_complete,
            );
            if let Some(warning) = status.warning {
                println!("Memory warning: {warning}");
            }
        }
        Err(error) => eprintln!("Warning: memory status is unavailable: {error}"),
    }
}

pub(in crate::interfaces::cli) async fn retry_memory_job(
    memory: &Option<MemoryService>,
    raw_id: &str,
) {
    let Ok(id) = raw_id.parse::<i64>() else {
        println!("Use /memory retry followed by a numeric job ID.");
        return;
    };
    let Some(memory) = memory.as_ref() else {
        println!("Local memory storage is unavailable.");
        return;
    };
    match memory.retry(JobId(id)).await {
        Ok(disposition) => println!("Memory job retry: {disposition:?}."),
        Err(error) => eprintln!("Warning: memory job could not be retried: {error}"),
    }
}

pub(in crate::interfaces::cli) async fn rebuild_memory_index(memory: &mut Option<MemoryService>) {
    let Some(memory) = memory.as_mut() else {
        println!("Local memory storage is unavailable.");
        return;
    };
    match memory.rebuild().await {
        Ok(updated) => println!("Memory index rebuild processed {updated} records."),
        Err(error) => eprintln!("Warning: memory index rebuild did not complete: {error}"),
    }
}
