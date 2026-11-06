use super::*;

/// Feeds `deltas` in order and returns every chunk, including the flushed remainder.
fn chunks_of<'a>(deltas: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut chunker = SpeechChunker::new();
    let mut chunks = Vec::new();
    for delta in deltas {
        chunks.extend(chunker.push(delta));
    }
    chunks.extend(chunker.finish());
    chunks
}

fn whole(text: &str) -> Vec<String> {
    chunks_of([text])
}

fn one_char_per_delta(text: &str) -> Vec<String> {
    let mut chunker = SpeechChunker::new();
    let mut chunks = Vec::new();
    for c in text.chars() {
        chunks.extend(chunker.push(c.encode_utf8(&mut [0; 4])));
    }
    chunks.extend(chunker.finish());
    chunks
}

fn letters(count: usize, ch: char) -> String {
    ch.to_string().repeat(count)
}

#[test]
fn limits_are_the_values_decided_in_s5() {
    assert_eq!(MIN_FIRST_CHUNK_CHARS, 40);
    assert_eq!(MAX_CHUNK_CHARS, 300);
}

#[test]
fn the_first_chunk_needs_40_characters_before_a_clause_break() {
    // (characters before the comma, whether the comma ends the first chunk)
    let cases = [(39, false), (40, true), (41, true)];
    for (before, splits) in cases {
        let head = letters(before, 'a');
        let text = format!("{head}, and then more words follow");
        let mut chunker = SpeechChunker::new();
        let emitted = chunker.push(&text);
        if splits {
            assert_eq!(emitted, vec![format!("{head},")], "{before}");
            assert_eq!(
                chunker.finish().as_deref(),
                Some("and then more words follow")
            );
        } else {
            assert!(emitted.is_empty(), "{before}");
            assert_eq!(chunker.finish().as_deref(), Some(text.as_str()));
        }
    }
}

#[test]
fn the_first_chunk_also_ends_at_a_sentence_end_once_long_enough() {
    let long = letters(40, 'a');
    let short = letters(39, 'a');
    assert_eq!(
        whole(&format!("{long}. Next one.")),
        vec![format!("{long}."), "Next one.".to_owned()]
    );
    // A short sentence stays with the text after it until 40 characters have arrived.
    assert_eq!(
        whole(&format!("{short}. {short}. Tail.")),
        vec![format!("{short}. {short}."), "Tail.".to_owned()]
    );
}

#[test]
fn later_chunks_ignore_clause_breaks_and_end_only_at_sentences() {
    let lead = letters(45, 'a');
    let text = format!("{lead}. Ok, fine. One, two; three: four. Five!");
    assert_eq!(
        whole(&text),
        vec![
            format!("{lead}."),
            "Ok, fine.".to_owned(),
            "One, two; three: four.".to_owned(),
            "Five!".to_owned()
        ]
    );
}

#[test]
fn question_and_exclamation_marks_end_sentences() {
    let lead = letters(40, 'a');
    assert_eq!(
        whole(&format!("{lead}? Really! Yes.")),
        vec![format!("{lead}?"), "Really!".to_owned(), "Yes.".to_owned()]
    );
}

#[test]
fn the_pending_text_is_force_split_only_beyond_300_characters() {
    let at_limit = letters(300, 'a');
    let mut chunker = SpeechChunker::new();
    assert!(chunker.push(&at_limit).is_empty());
    assert_eq!(chunker.finish(), Some(at_limit));

    let over = letters(301, 'a');
    assert_eq!(whole(&over), vec![letters(300, 'a'), "a".to_owned()]);
}

#[test]
fn a_force_split_prefers_the_last_whitespace_at_or_before_300() {
    // Whitespace at index 300 keeps a full 300-character chunk.
    let text = format!("{} tail", letters(300, 'a'));
    assert_eq!(whole(&text), vec![letters(300, 'a'), "tail".to_owned()]);

    // Whitespace at index 299: the word after it moves to the next chunk.
    let text = format!("{} {} tail", letters(299, 'a'), letters(5, 'b'));
    assert_eq!(
        whole(&text),
        vec![letters(299, 'a'), "bbbbb tail".to_owned()]
    );

    // Whitespace only at index 301 is too late: split exactly at 300.
    let text = format!("{} next", letters(301, 'a'));
    assert_eq!(whole(&text), vec![letters(300, 'a'), "a next".to_owned()]);
}

#[test]
fn a_force_split_counts_characters_not_bytes() {
    let text = "é".repeat(301);
    assert_eq!(whole(&text), vec!["é".repeat(300), "é".to_owned()]);
}

#[test]
fn a_sentence_that_ends_within_300_characters_is_not_force_split() {
    let sentence = format!("{}.", letters(299, 'a'));
    assert_eq!(
        whole(&format!("{sentence} Next sentence here.")),
        vec![sentence, "Next sentence here.".to_owned()]
    );
}

#[test]
fn abbreviations_do_not_end_a_sentence() {
    let cases = [
        "Dr. Smith met Mr. Jones and Mrs. Lee and Ms. Park today.",
        "Use fruit, e.g. apples and pears, or i.e. anything sweet today.",
        "We bought milk, eggs, etc. and then went home to cook dinner.",
        "It was Alice vs. Bob in the final round of the long tournament.",
        "dr. lowercase works and DR. uppercase works as well too.",
    ];
    for text in cases {
        assert_eq!(whole(text), vec![text.to_owned()], "{text}");
    }
}

#[test]
fn decimals_and_versions_do_not_end_a_sentence() {
    let cases = [
        "Python 3.5 was released a long time ago and is now retired.",
        "Upgrade to v1.2 before the release candidate lands for real.",
        "The value of pi is roughly 3.14159 and e is about 2.71828.",
    ];
    for text in cases {
        assert_eq!(whole(text), vec![text.to_owned()], "{text}");
    }
}

#[test]
fn a_list_marker_stays_with_its_item_but_a_number_ending_a_sentence_does_not() {
    let lead = letters(45, 'a');
    assert_eq!(
        whole(&format!(
            "{lead}. 1. First step. 12. Second step. 3. Third."
        )),
        vec![
            format!("{lead}."),
            "1. First step.".to_owned(),
            "12. Second step.".to_owned(),
            "3. Third.".to_owned(),
        ]
    );
    assert_eq!(
        whole(&format!("{lead}. The answer is 42. Then we rest.")),
        vec![
            format!("{lead}."),
            "The answer is 42.".to_owned(),
            "Then we rest.".to_owned(),
        ]
    );
}

#[test]
fn a_period_at_the_end_of_input_is_held_until_the_next_character_or_finish() {
    let lead = letters(40, 'a');
    let mut chunker = SpeechChunker::new();
    assert!(chunker.push(&format!("{lead}.")).is_empty());
    assert_eq!(chunker.push(" "), vec![format!("{lead}.")]);
    assert_eq!(chunker.finish(), None);

    let mut chunker = SpeechChunker::new();
    assert!(chunker.push(&format!("{lead}.")).is_empty());
    assert_eq!(chunker.finish(), Some(format!("{lead}.")));
}

#[test]
fn an_abbreviation_split_across_deltas_is_still_recognised() {
    let text = "Dr. Smith went home early to rest after a long day.";
    assert_eq!(
        chunks_of([
            "D",
            "r",
            ".",
            " Smith went home early",
            " to rest after a long day."
        ]),
        vec![text.to_owned()]
    );
}

#[test]
fn finish_flushes_the_remainder_trimmed() {
    assert_eq!(
        whole("  just a few words  "),
        vec!["just a few words".to_owned()]
    );
}

#[test]
fn finish_returns_none_after_everything_was_emitted() {
    let lead = letters(40, 'a');
    let mut chunker = SpeechChunker::new();
    assert_eq!(chunker.push(&format!("{lead}. ")), vec![format!("{lead}.")]);
    assert_eq!(chunker.finish(), None);
}

#[test]
fn empty_and_whitespace_input_yields_no_chunks() {
    for text in ["", " ", "\n\t  \n"] {
        assert!(whole(text).is_empty(), "{text:?}");
        assert!(chunks_of(["", text, ""]).is_empty(), "{text:?}");
    }
}

#[test]
fn newlines_count_as_whitespace_after_a_sentence_end() {
    let lead = letters(40, 'a');
    assert_eq!(
        whole(&format!("{lead}.\nNext.")),
        vec![format!("{lead}."), "Next.".to_owned()]
    );
}

#[test]
fn chunks_are_never_empty_or_untrimmed() {
    let text = format!("  {}.  \n\n  Two.   Three!   ", letters(41, 'a'));
    for chunk in whole(&text) {
        assert!(!chunk.is_empty());
        assert_eq!(chunk, chunk.trim());
    }
}

#[test]
fn splitting_the_stream_anywhere_gives_the_same_chunks() {
    let long = format!(
        "{} {}. {}",
        letters(200, 'a'),
        letters(200, 'b'),
        letters(350, 'c')
    );
    let newline = format!(
        "{}\n\nshort, tail words go here without end",
        letters(310, 'x')
    );
    let texts = [
        "Hello there. How are you doing today? I am fine, thank you very much for asking, friend. Dr. Who is 3.5 times better than v1.2 of it, e.g. most days. Done!",
        "Café déjà vu, naïve señor — ça va très bien aujourd'hui, mon ami; voilà! Next one.",
        long.as_str(),
        newline.as_str(),
    ];
    for text in texts {
        let expected = whole(text);
        assert_eq!(one_char_per_delta(text), expected, "per char: {text}");
        for (split, _) in text.char_indices() {
            let (a, b) = text.split_at(split);
            assert_eq!(chunks_of([a, b]), expected, "split at {split}: {text}");
        }
    }
}
