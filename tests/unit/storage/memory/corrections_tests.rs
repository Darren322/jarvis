use std::collections::HashMap;

use crate::storage::{
    CorrectionEvidence, CorrectionMarker, EvidenceInput, MemoryAttribution, MemoryRecordKind,
    NewMemory, SourcePartRole,
};

use super::{contains_phrase, current_correction_is_grounded, phrase_positions};

#[test]
fn phrase_matching_uses_utf8_byte_offsets_case_insensitive_text_and_word_boundaries() {
    let cases = [
        ("🙂 Actually ACTUALLY!", "actually", vec![5, 14], true),
        ("myActually Actuallyy", "actually", vec![2, 11], false),
        ("aCtUaLlY", "ACTUALLY", vec![0], true),
    ];

    for (text, phrase, expected_positions, expected_match) in cases {
        assert_eq!(phrase_positions(text, phrase), expected_positions);
        assert_eq!(contains_phrase(text, phrase), expected_match);
    }
}

#[test]
fn grounded_first_person_user_correction_is_accepted() {
    let text = "Actually my favourite drink is tea instead of coffee.";
    let (source_parts, record, correction) = correction_fixture(text, SourcePartRole::User);

    assert!(current_correction_is_grounded(
        &source_parts,
        &record,
        &correction
    ));
}

#[test]
fn quoted_or_non_user_corrections_are_rejected() {
    let quoted_text = "Actually \"my favourite drink is tea instead of coffee.\"";
    let (source_parts, record, correction) = correction_fixture(quoted_text, SourcePartRole::User);
    assert!(!current_correction_is_grounded(
        &source_parts,
        &record,
        &correction
    ));

    let text = "Actually my favourite drink is tea instead of coffee.";
    let (source_parts, record, correction) = correction_fixture(text, SourcePartRole::Assistant);
    assert!(!current_correction_is_grounded(
        &source_parts,
        &record,
        &correction
    ));
}

fn correction_fixture(
    text: &str,
    role: SourcePartRole,
) -> (
    HashMap<u32, (SourcePartRole, &str)>,
    NewMemory,
    CorrectionEvidence,
) {
    let correction = CorrectionEvidence {
        marker: CorrectionMarker::Actually,
        subject: evidence_span(text, "my"),
        property: evidence_span(text, "favourite drink"),
        old_value: evidence_span(text, "coffee"),
        new_value: evidence_span(text, "tea"),
    };
    let record = NewMemory {
        text: "I prefer tea".to_owned(),
        kind: MemoryRecordKind::Preference,
        attribution: MemoryAttribution::UserStatement,
        evidence: vec![EvidenceInput {
            part_index: 0,
            start_byte: 0,
            end_byte: text.len() as u32,
            quote: text.to_owned(),
        }],
        correction: Some(correction.clone()),
        correction_needs_review: false,
    };

    (HashMap::from([(0, (role, text))]), record, correction)
}

fn evidence_span(text: &str, quote: &str) -> EvidenceInput {
    let start_byte = text.find(quote).expect("fixture quote exists") as u32;
    EvidenceInput {
        part_index: 0,
        start_byte,
        end_byte: start_byte + quote.len() as u32,
        quote: quote.to_owned(),
    }
}
