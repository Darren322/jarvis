use std::collections::{HashMap, HashSet};

use super::*;
use crate::storage::{
    MemoryAttribution, MemoryEvidence, MemoryRecordKind, MemoryRecordStatus, SourceKind,
    SourcePartRole,
};

fn memory(id: i64, text: &str, source: i64, start: u32, end: u32) -> HydratedMemory {
    memory_in_part(id, text, source, 0, SourcePartRole::User, start, end)
}

fn memory_in_part(
    id: i64,
    text: &str,
    source: i64,
    part_index: u32,
    role: SourcePartRole,
    start: u32,
    end: u32,
) -> HydratedMemory {
    HydratedMemory {
        id: MemoryId(id),
        text: text.to_owned(),
        kind: MemoryRecordKind::Preference,
        attribution: MemoryAttribution::UserStatement,
        status: MemoryRecordStatus::Active,
        correction_state: CorrectionState::None,
        supersedes_id: None,
        source_created_at_unix_ms: 1_800_000_000_000,
        evidence: vec![MemoryEvidence {
            source_id: SourceId(source),
            part_index,
            source_kind: SourceKind::Conversation,
            source_name: None,
            role,
            start_byte: start,
            end_byte: end,
            quote: Some(text.to_owned()),
            source_created_at_unix_ms: 1_800_000_000_000,
        }],
    }
}

#[test]
fn lexical_exact_record_survives_but_generic_overlap_and_weak_semantics_do_not() {
    let candidates = SearchCandidates {
        fused: vec![MemoryId(1), MemoryId(2), MemoryId(3)],
        cosine: HashMap::from([(MemoryId(2), 0.56), (MemoryId(3), 0.74)]),
        lexical: HashSet::from([MemoryId(1)]),
    };
    let snapshot = select_context(
        vec![
            memory(1, "My dog's name is Miso.", 10, 0, 22),
            memory(2, "My favorite project name is Asana.", 11, 0, 35),
            memory(3, "My dog is called Miso.", 12, 0, 22),
        ],
        &candidates,
        &[],
    );

    assert_eq!(snapshot.documents.len(), 2);
    assert!(
        snapshot.documents[0]
            .to_string()
            .contains("source_timestamp_unix_ms")
    );
}

#[test]
fn unrelated_query_with_only_weak_vector_candidates_returns_empty() {
    let candidates = SearchCandidates {
        fused: vec![MemoryId(1), MemoryId(2)],
        cosine: HashMap::from([(MemoryId(1), 0.56), (MemoryId(2), 0.63)]),
        lexical: HashSet::new(),
    };
    let snapshot = select_context(
        vec![
            memory(1, "My dog's name is Miso.", 10, 0, 22),
            memory(2, "My favorite project name is Asana.", 11, 0, 35),
        ],
        &candidates,
        &[],
    );

    assert_eq!(snapshot.state, RecallState::Empty);
    assert!(snapshot.documents.is_empty());
    assert!(snapshot.source_ids.is_empty());
}

#[test]
fn same_source_duplicate_or_nearly_identical_spans_are_collapsed() {
    let candidates = SearchCandidates {
        fused: vec![MemoryId(1), MemoryId(2), MemoryId(3)],
        cosine: HashMap::new(),
        lexical: HashSet::from([MemoryId(1), MemoryId(2), MemoryId(3)]),
    };
    let snapshot = select_context(
        vec![
            memory(1, "I prefer green tea.", 10, 0, 50),
            memory(2, "  I   prefer green tea. ", 10, 0, 50),
            memory(3, "I prefer tea.", 10, 1, 49),
        ],
        &candidates,
        &[],
    );
    assert_eq!(snapshot.documents.len(), 1);
}

#[test]
fn same_offsets_in_distinct_parts_and_roles_are_not_deduplicated() {
    let candidates = SearchCandidates {
        fused: vec![MemoryId(1), MemoryId(2), MemoryId(3)],
        cosine: HashMap::new(),
        lexical: HashSet::from([MemoryId(1), MemoryId(2), MemoryId(3)]),
    };
    let snapshot = select_context(
        vec![
            memory_in_part(1, "I prefer green tea.", 10, 0, SourcePartRole::User, 0, 19),
            memory_in_part(2, "I prefer black tea.", 10, 1, SourcePartRole::User, 0, 19),
            memory_in_part(
                3,
                "I prefer green tea.",
                10,
                2,
                SourcePartRole::Assistant,
                0,
                19,
            ),
        ],
        &candidates,
        &[],
    );

    assert_eq!(snapshot.documents.len(), 3);
}
