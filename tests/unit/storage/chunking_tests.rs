use super::{MAX_CHUNK_BYTES, conversation_text, split};

#[test]
fn short_text_is_one_trimmed_chunk() {
    assert_eq!(split("  hello there \n", 100), vec!["hello there"]);
    assert!(split("   \n ", 100).is_empty());
}

#[test]
fn text_at_the_cap_is_one_chunk_and_one_byte_more_is_two() {
    let at_cap = format!(
        "{} {}",
        "a".repeat(MAX_CHUNK_BYTES / 2),
        "b".repeat(MAX_CHUNK_BYTES / 2 - 1)
    );
    assert_eq!(at_cap.len(), MAX_CHUNK_BYTES);
    assert_eq!(split(&at_cap, MAX_CHUNK_BYTES).len(), 1);

    let over_cap = format!("{at_cap}c");
    let chunks = split(&over_cap, MAX_CHUNK_BYTES);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0], "a".repeat(MAX_CHUNK_BYTES / 2));
}

#[test]
fn prefers_paragraph_then_sentence_then_word_boundaries() {
    let text = "First paragraph here.\n\nSecond paragraph is a bit longer.";
    assert_eq!(
        split(text, 30),
        vec![
            "First paragraph here.",
            "Second paragraph is a bit",
            "longer."
        ]
    );
    assert_eq!(
        split("One. Two three four five.", 12),
        vec!["One.", "Two three", "four five."]
    );
}

#[test]
fn every_chunk_respects_the_byte_cap_and_utf8_boundaries() {
    let text = "é".repeat(2000) + " " + &"word ".repeat(800);
    let chunks = split(&text, MAX_CHUNK_BYTES);
    assert!(chunks.len() > 2);
    assert!(
        chunks
            .iter()
            .all(|chunk| !chunk.is_empty() && chunk.len() <= MAX_CHUNK_BYTES)
    );
    let rejoined: String = chunks.concat().split_whitespace().collect();
    let original: String = text.split_whitespace().collect();
    assert_eq!(rejoined, original);
}

#[test]
fn conversation_text_labels_both_sides() {
    assert_eq!(
        conversation_text(" My cat is Miso. ", "Noted, master."),
        "User: My cat is Miso.\nJarvis: Noted, master."
    );
}
