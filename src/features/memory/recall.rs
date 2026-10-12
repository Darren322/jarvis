use std::collections::{HashMap, HashSet};

use rig_core::completion::Document;

use crate::storage::{MemoryId, MemoryKind, RecalledMemory, SourceId, SourceKind};

/// Candidates fetched from each index before fusion.
pub(super) const SEARCH_LIMIT: usize = 20;
const MAX_CONTEXT_DOCUMENTS: usize = 5;
const MAX_CONTEXT_BYTES: usize = 4096;
/// Standard reciprocal-rank-fusion constant.
const RRF_K: f32 = 60.0;
// Provisional guardrail for normalized BGE-small-en-v1.5 vectors only: a small
// offline fixture measured 0.74 for a paraphrase and at most 0.56 for unrelated
// passages. It is not a confidence score. Keyword hits bypass it.
pub(super) const BGE_MIN_COSINE: f32 = 0.68;

const STOP_WORDS: &[&str] = &[
    "a", "about", "am", "an", "and", "any", "are", "as", "at", "be", "been", "but", "by", "can",
    "could", "did", "do", "does", "for", "from", "had", "has", "have", "he", "her", "him", "his",
    "how", "i", "if", "in", "into", "is", "it", "its", "me", "my", "of", "on", "or", "our", "she",
    "so", "that", "the", "their", "them", "then", "there", "these", "they", "this", "to", "was",
    "we", "were", "what", "when", "where", "which", "who", "why", "will", "with", "would", "you",
    "your",
];

/// Builds an FTS5 query that requires every substantive term. Each term is
/// quoted, so user text can never inject FTS5 operators.
pub(super) fn keyword_query(query: &str) -> Option<String> {
    let mut seen = HashSet::new();
    let terms = query
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|term| term.len() > 1 && !STOP_WORDS.contains(&term.as_str()))
        .filter(|term| seen.insert(term.clone()))
        .map(|term| format!("\"{term}\""))
        .collect::<Vec<_>>();
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// Ranked candidates from both indexes.
#[derive(Debug, Default)]
pub(super) struct Candidates {
    pub(super) keyword: Vec<MemoryId>,
    pub(super) semantic: Vec<(MemoryId, f32)>,
}

impl Candidates {
    /// Reciprocal rank fusion of both lists, keeping only memories that matched
    /// every keyword or cleared the semantic similarity floor.
    pub(super) fn fused(&self) -> Vec<MemoryId> {
        let mut scores = HashMap::<MemoryId, f32>::new();
        for (rank, id) in self.keyword.iter().enumerate() {
            *scores.entry(*id).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
        }
        for (rank, (id, cosine)) in self.semantic.iter().enumerate() {
            if *cosine >= BGE_MIN_COSINE {
                *scores.entry(*id).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
            }
        }
        let mut fused = scores.into_iter().collect::<Vec<_>>();
        fused.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(&right.0)));
        fused.into_iter().map(|(id, _)| id).collect()
    }
}

/// Turns fused, hydrated memories into bounded run-local Rig documents.
pub(super) fn select_context(
    ordered: &[MemoryId],
    hydrated: Vec<RecalledMemory>,
    excluded_sources: &[SourceId],
) -> (Vec<Document>, Vec<SourceId>) {
    let excluded = excluded_sources.iter().copied().collect::<HashSet<_>>();
    let mut by_id = hydrated
        .into_iter()
        .map(|memory| (memory.id, memory))
        .collect::<HashMap<_, _>>();
    let mut seen_text = HashSet::new();
    let mut documents = Vec::new();
    let mut source_ids = Vec::new();
    let mut bytes = 0usize;

    for id in ordered {
        if documents.len() >= MAX_CONTEXT_DOCUMENTS {
            break;
        }
        let Some(memory) = by_id.remove(id) else {
            continue;
        };
        // Sources already in the active history are in the model's context.
        if excluded.contains(&memory.source_id) || !seen_text.insert(normalize(&memory.text)) {
            continue;
        }
        let source_id = memory.source_id;
        let document = to_document(memory);
        let document_bytes = document.to_string().len();
        if bytes.saturating_add(document_bytes) > MAX_CONTEXT_BYTES {
            continue;
        }
        bytes += document_bytes;
        documents.push(document);
        if !source_ids.contains(&source_id) {
            source_ids.push(source_id);
        }
    }
    (documents, source_ids)
}

fn to_document(memory: RecalledMemory) -> Document {
    let source = memory.source_name.unwrap_or_else(|| {
        match memory.source_kind {
            SourceKind::Conversation => "conversation",
            SourceKind::Import => "imported document",
        }
        .to_owned()
    });
    let kind = match memory.kind {
        MemoryKind::Excerpt => "excerpt",
        MemoryKind::Fact => "extracted fact",
    };
    Document {
        id: memory.id.0.to_string(),
        text: memory.text,
        additional_props: HashMap::from([
            ("source".to_owned(), source),
            (
                "source_timestamp_unix_ms".to_owned(),
                memory.source_created_at_unix_ms.to_string(),
            ),
            ("kind".to_owned(), kind.to_owned()),
        ]),
    }
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
#[path = "../../../tests/unit/memory/recall_tests.rs"]
mod tests;
