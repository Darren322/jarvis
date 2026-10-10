use super::*;
use crate::storage::{SourceId, SourceKind, SourcePart};

const CORRECTED: &str = "Actually my favourite drink is tea instead of coffee.";

fn span(text: &str, quote: &str, part_index: u32) -> ExtractedEvidence {
    let start = text.find(quote).expect("fixture contains quoted span");
    ExtractedEvidence {
        part_index,
        start_byte: start as u32,
        end_byte: (start + quote.len()) as u32,
        quote: quote.to_owned(),
    }
}

fn source(text: &str) -> SourceMaterial {
    SourceMaterial {
        id: SourceId(8),
        kind: SourceKind::Conversation,
        source_key: "turn:8".to_owned(),
        revision_sha256: "fixture".to_owned(),
        created_at_unix_ms: 1_800_000_000_000,
        display_source: None,
        parts: vec![SourcePart {
            index: 0,
            role: SourcePartRole::User,
            text: text.to_owned(),
        }],
    }
}

#[test]
fn clear_current_user_correction_keeps_exact_grounded_spans() {
    let source = source(CORRECTED);
    let output = ExtractionOutput {
        memories: vec![ExtractedMemory {
            kind: ExtractKind::Preference,
            evidence: vec![span(CORRECTED, CORRECTED, 0)],
            correction: Some(ExtractedCorrection {
                marker: ExtractedCorrectionMarker::Actually,
                subject: span(CORRECTED, "my", 0),
                property: span(CORRECTED, "favourite drink", 0),
                old_value: span(CORRECTED, "coffee", 0),
                new_value: span(CORRECTED, "tea", 0),
            }),
            correction_needs_review: false,
        }],
    };

    let records = validate_output(output, &source).expect("valid exact evidence");
    let correction = records[0]
        .correction
        .as_ref()
        .expect("correction evidence is retained");
    assert_eq!(correction.marker, CorrectionMarker::Actually);
    assert_eq!(correction.subject.quote, "my");
    assert_eq!(correction.property.quote, "favourite drink");
    assert_eq!(correction.old_value.quote, "coffee");
    assert_eq!(correction.new_value.quote, "tea");
    assert!(!records[0].correction_needs_review);
}

#[test]
fn correction_span_outside_the_claim_is_kept_for_review_not_applied() {
    let text = "Earlier I liked coffee. Actually my favourite drink is tea instead of coffee.";
    let source = source(text);
    let claim = "Actually my favourite drink is tea instead of coffee.";
    let output = ExtractionOutput {
        memories: vec![ExtractedMemory {
            kind: ExtractKind::Preference,
            evidence: vec![span(text, claim, 0)],
            correction: Some(ExtractedCorrection {
                marker: ExtractedCorrectionMarker::Actually,
                subject: span(text, "my", 0),
                property: span(text, "favourite drink", 0),
                old_value: span(text, "coffee", 0),
                new_value: span(text, "tea", 0),
            }),
            correction_needs_review: false,
        }],
    };

    let records = validate_output(output, &source).expect("the claim itself is grounded");
    assert!(records[0].correction.is_none());
    assert!(records[0].correction_needs_review);
}
