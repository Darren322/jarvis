use crate::storage::{MemoryId, MemoryKind, RecalledMemory, SourceId, SourceKind};

use super::{BGE_MIN_COSINE, Candidates, keyword_query, select_context, wants_memory_search};

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
    // One-character terms are dropped; two characters are the shortest kept.
    assert_eq!(keyword_query("x go").as_deref(), Some("\"go\""));
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
fn a_search_keeps_at_most_two_memories() {
    // The last accepted count (2) and the first rejected one (3), plus fewer.
    for (available, kept) in [(1, 1), (2, 2), (3, 2), (6, 2)] {
        let ordered = (1..=available).map(MemoryId).collect::<Vec<_>>();
        let hydrated = (1..=available)
            .map(|id| memory(id, id, &format!("memory number {id}")))
            .collect();
        let (documents, sources) = select_context(&ordered, hydrated, &[]);
        assert_eq!(documents.len(), kept, "{available} available");
        assert_eq!(sources.len(), kept, "{available} available");
    }
}

#[test]
fn a_search_keeps_at_most_1024_bytes_of_documents() {
    // The bytes counted are the rendered document, so measure its fixed overhead.
    let (probe, _) = select_context(&[MemoryId(1)], vec![memory(1, 1, "x")], &[]);
    let overhead = probe[0].to_string().len() - 1;
    // Two documents of 512 bytes each: 1,024 in total.
    let text_of = |letter: &str, extra: usize| letter.repeat(512 - overhead + extra);
    let ordered = [MemoryId(1), MemoryId(2)];

    let exactly = vec![
        memory(1, 1, &text_of("a", 0)),
        memory(2, 2, &text_of("b", 0)),
    ];
    let (documents, _) = select_context(&ordered, exactly, &[]);
    assert_eq!(documents.len(), 2);
    assert_eq!(
        documents.iter().map(|d| d.to_string().len()).sum::<usize>(),
        1024
    );

    // One byte more: the second no longer fits.
    let one_over = vec![
        memory(1, 1, &text_of("a", 0)),
        memory(2, 2, &text_of("b", 1)),
    ];
    let (documents, _) = select_context(&ordered, one_over, &[]);
    assert_eq!(documents.len(), 1);

    // A document too big for the budget is skipped, and a smaller one after it still fits.
    let oversized = vec![memory(1, 1, &text_of("a", 600)), memory(2, 2, "small")];
    let (documents, _) = select_context(&ordered, oversized, &[]);
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].id, "2");
}

#[rustfmt::skip]
#[test]
fn a_prompt_asks_for_a_search_only_by_a_whole_word_trigger_phrase() {
    let cases: &[(&str, bool)] = &[
        // Every trigger phrase.
        ("Do you remember my cat?", true),
        ("recall my cat's name", true),
        ("What did we decide last time?", true),
        ("what did I tell you about Tokyo", true),
        ("What did I say about the budget?", true),
        ("Did I tell you where I parked?", true),
        // Case does not matter.
        ("REMEMBER the cat", true),
        ("LaSt TiMe we spoke", true),
        ("WHAT DID I TELL YOU", true),
        // Punctuation and extra spaces between the words of a phrase do not defeat it.
        ("Last  time, we met", true),
        ("last-time", true),
        ("what did I tell... you?", true),
        ("what   did I\tsay", true),
        ("(remember)", true),
        // A phrase inside a longer word is not a match.
        ("I am remembering the cat", false),
        ("It was recalled last week", false),
        ("Remembrance Day", false),
        ("the recaller", false),
        ("blast times are long", false),
        ("what did I tell your friend", false),
        // "remember that ..." is a save, reserved for a later ticket: no search.
        ("remember that I like tea", false),
        ("Remember that my sister is called Mei", false),
        ("remember that", false),
        // ...unless it points back at an occasion, which is a search.
        ("Remember that time we talked about Tokyo?", true),
        ("remember that day at the beach", true),
        ("remember that night", true),
        ("remember that one time in Kyoto", true),
        // A greeting before the phrase changes nothing.
        ("Hey Jarvis, remember that time we went to Tokyo", true),
        ("Jarvis, remember that I like tea", false),
        ("hey, Jarvis: remember that I like tea", false),
        ("Hey Jarvis, do you remember my cat?", true),
        // Only a leading "remember that" is a save.
        ("I will remember that I like tea", true),
        // Plain prompts and nothing at all.
        ("Hello", false),
        ("How's the weather?", false),
        ("What time is it?", false),
        ("", false),
        ("   ", false),
    ];
    for (prompt, expected) in cases {
        assert_eq!(wants_memory_search(prompt), *expected, "{prompt:?}");
    }
}
