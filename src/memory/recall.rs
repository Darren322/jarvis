use std::collections::HashSet;

use rig_core::completion::Document;

use crate::storage::{CorrectionState, HydratedMemory, MemoryId, MemoryRepository, SourceId};

use super::{
    index::{MemoryIndex, SearchCandidates},
    types::{RecallSnapshot, RecallState},
};

const MAX_RECALL_RECORDS: usize = 20;
const MAX_CONTEXT_DOCUMENTS: usize = 5;
const MAX_CONTEXT_BYTES: usize = 4096;
// Provisional guardrail for the pinned normalized BGE-small-en-v1.5 model only:
// the small offline public fixture measured 0.74 for a semantic paraphrase and
// at most 0.56 for unrelated passages. This is not a confidence score or a
// universal similarity threshold. FTS is a separate all-substantive-terms path.
pub(super) const BGE_MIN_COSINE: f32 = 0.68;

pub(crate) async fn recall(
    repository: &MemoryRepository,
    index: &MemoryIndex,
    query: &str,
    vector: &[f32],
    excluded_sources: &[SourceId],
) -> Result<RecallSnapshot, String> {
    if query.trim().is_empty() {
        return Ok(RecallSnapshot::empty());
    }
    let candidates = index
        .search(query, vector)
        .await
        .map_err(|error| error.to_string())?;
    let ordered = candidates
        .fused
        .iter()
        .copied()
        .take(MAX_RECALL_RECORDS)
        .collect::<Vec<_>>();
    let active = repository
        .hydrate_active(&ordered)
        .await
        .map_err(|error| error.to_string())?;
    Ok(select_context(active, &candidates, excluded_sources))
}

fn select_context(
    active: Vec<HydratedMemory>,
    candidates: &SearchCandidates,
    excluded_sources: &[SourceId],
) -> RecallSnapshot {
    let excluded = excluded_sources.iter().copied().collect::<HashSet<_>>();
    let active_by_id = active
        .into_iter()
        .map(|memory| (memory.id, memory))
        .collect::<std::collections::HashMap<_, _>>();
    let mut seen = HashSet::<MemoryId>::new();
    let mut documents = Vec::new();
    let mut source_ids = Vec::new();
    let mut selected_spans = Vec::<(
        SourceId,
        u32,
        crate::storage::SourcePartRole,
        u32,
        u32,
        String,
    )>::new();
    let mut bytes: usize = 0;

    for id in candidates.fused.iter().take(MAX_RECALL_RECORDS) {
        if !seen.insert(*id) {
            continue;
        }
        let lexical = candidates.lexical.contains(id);
        let semantic = candidates
            .cosine
            .get(id)
            .is_some_and(|cosine| *cosine >= BGE_MIN_COSINE);
        if !lexical && !semantic {
            continue;
        }
        let Some(memory) = active_by_id.get(id) else {
            continue;
        };
        let Some(evidence) = memory
            .evidence
            .iter()
            .find(|evidence| !excluded.contains(&evidence.source_id))
        else {
            continue;
        };
        let label = evidence
            .source_name
            .as_deref()
            .unwrap_or(match evidence.source_kind {
                crate::storage::SourceKind::Conversation => "conversation",
                crate::storage::SourceKind::Import => "imported document",
                crate::storage::SourceKind::Action => "action record",
            });
        let normalized_text = normalize_for_dedup(&memory.text);
        if selected_spans
            .iter()
            .any(|(source_id, part_index, role, start, end, text)| {
                *source_id == evidence.source_id
                    && *part_index == evidence.part_index
                    && *role == evidence.role
                    && (text == &normalized_text
                        || overlap_is_duplicate(
                            (*start, *end),
                            (evidence.start_byte, evidence.end_byte),
                        ))
            })
        {
            continue;
        }
        let mut additional_props = std::collections::HashMap::new();
        additional_props.insert("source".to_owned(), label.to_owned());
        additional_props.insert(
            "source_timestamp_unix_ms".to_owned(),
            evidence.source_created_at_unix_ms.to_string(),
        );
        additional_props.insert("kind".to_owned(), kind_label(memory.kind).to_owned());
        additional_props.insert(
            "attribution".to_owned(),
            attribution_label(memory.attribution).to_owned(),
        );
        additional_props.insert(
            "correction".to_owned(),
            correction_label(memory.correction_state).to_owned(),
        );
        let document = Document {
            id: id.0.to_string(),
            text: memory.text.clone(),
            additional_props,
        };
        let document_bytes = document.to_string().len();
        if documents.len() >= MAX_CONTEXT_DOCUMENTS
            || bytes.saturating_add(document_bytes) > MAX_CONTEXT_BYTES
        {
            continue;
        }
        bytes += document_bytes;
        selected_spans.push((
            evidence.source_id,
            evidence.part_index,
            evidence.role,
            evidence.start_byte,
            evidence.end_byte,
            normalized_text,
        ));
        documents.push(document);
        source_ids.push(evidence.source_id);
    }

    RecallSnapshot {
        state: if documents.is_empty() {
            RecallState::Empty
        } else {
            RecallState::Available
        },
        documents,
        source_ids,
    }
}

fn normalize_for_dedup(text: &str) -> String {
    text.split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

fn overlap_is_duplicate(first: (u32, u32), second: (u32, u32)) -> bool {
    let intersection = first.1.min(second.1).saturating_sub(first.0.max(second.0));
    let smaller = first
        .1
        .saturating_sub(first.0)
        .min(second.1.saturating_sub(second.0));
    smaller > 0 && intersection.saturating_mul(5) >= smaller.saturating_mul(4)
}

fn kind_label(kind: crate::storage::MemoryRecordKind) -> &'static str {
    match kind {
        crate::storage::MemoryRecordKind::SourceExcerpt => "source excerpt",
        crate::storage::MemoryRecordKind::ImportedChunk => "imported excerpt",
        crate::storage::MemoryRecordKind::Preference => "preference",
        crate::storage::MemoryRecordKind::Relationship => "relationship",
        crate::storage::MemoryRecordKind::Event => "event",
        crate::storage::MemoryRecordKind::Project => "project",
        crate::storage::MemoryRecordKind::Experience => "experience",
        crate::storage::MemoryRecordKind::Other => "memory",
    }
}

fn attribution_label(attribution: crate::storage::MemoryAttribution) -> &'static str {
    match attribution {
        crate::storage::MemoryAttribution::UserStatement => "user statement",
        crate::storage::MemoryAttribution::AssistantAnswer => "assistant answer",
        crate::storage::MemoryAttribution::ImportedClaim => "imported claim",
        crate::storage::MemoryAttribution::ActionResult => "action result",
        crate::storage::MemoryAttribution::Inference => "inference",
    }
}

fn correction_label(state: CorrectionState) -> &'static str {
    match state {
        CorrectionState::None => "none",
        CorrectionState::Applied => "applied",
        CorrectionState::NeedsReview => "needs review",
    }
}

#[cfg(test)]
#[path = "../../tests/unit/memory/recall_tests.rs"]
mod tests;
