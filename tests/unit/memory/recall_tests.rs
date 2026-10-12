use crate::storage::{MemoryId, MemoryKind, RecalledMemory, SourceId, SourceKind};

use super::{BGE_MIN_COSINE, Candidates, keyword_query, select_context};

fn memory(id: i64, source: i64, text: &str) -> RecalledMemory {
    RecalledMemory {
        id: MemoryId(id),
        source_id: SourceId(source),
        source_kind: SourceKind::Conversation,
        source_name: None,
        source_created_at_unix_ms: 1_000,
        kind: MemoryKind::Excerpt,
        text: text.to_owned(),
    }
}

#[test]
fn keyword_query_quotes_substantive_terms_only() {
    assert_eq!(
        keyword_query("What is my cat's name?").as_deref(),
        Some("\"cat\" \"name\"")
    );
    assert_eq!(keyword_query("what is it?"), None);
    assert_eq!(keyword_query("   "), None);
}

#[test]
fn keyword_query_cannot_inject_fts_syntax() {
    let query = keyword_query("cat\" OR NEAR(dog*) -x").expect("terms remain");
    assert_eq!(query, "\"cat\" \"near\" \"dog\"");
    assert!(!query.contains('*') && !query.contains('('));
}

#[test]
fn fusion_rewards_agreement_and_drops_weak_semantic_matches() {
    let candidates = Candidates {
        keyword: vec![MemoryId(1), MemoryId(2)],
        semantic: vec![
            (MemoryId(2), 0.9),
            (MemoryId(3), BGE_MIN_COSINE),
            (MemoryId(4), BGE_MIN_COSINE - 0.01),
        ],
    };
    assert_eq!(
        candidates.fused(),
        vec![MemoryId(2), MemoryId(1), MemoryId(3)]
    );
}

#[test]
fn context_follows_fused_order_and_skips_excluded_and_duplicate_text() {
    let ordered = [MemoryId(3), MemoryId(1), MemoryId(2), MemoryId(4)];
    let hydrated = vec![
        memory(1, 10, "My cat is Miso."),
        memory(2, 20, "my   CAT is miso."),
        memory(3, 30, "Already in history."),
        // Memory 4 was forgotten after search, so it is missing here.
    ];

    let (documents, sources) = select_context(&ordered, hydrated, &[SourceId(30)]);

    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].id, "1");
    assert_eq!(documents[0].additional_props["source"], "conversation");
    assert_eq!(sources, vec![SourceId(10)]);
}

#[test]
fn context_is_bounded_by_document_count_and_bytes() {
    let ordered = (1..=8).map(MemoryId).collect::<Vec<_>>();
    let hydrated = (1..=8)
        .map(|id| memory(id, id, &format!("memory number {id}")))
        .collect();
    let (documents, _) = select_context(&ordered, hydrated, &[]);
    assert_eq!(documents.len(), 5);

    let large = "x".repeat(3000);
    let hydrated = vec![memory(1, 1, &large), memory(2, 2, &(large.clone() + "y"))];
    let (documents, _) = select_context(&[MemoryId(1), MemoryId(2)], hydrated, &[]);
    assert_eq!(documents.len(), 1);
}
