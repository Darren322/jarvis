use rusqlite::params;

use super::sources::load_source_part;
use crate::storage::{
    CorrectionEvidence, CorrectionMarker, CorrectionState, MemoryId, MemoryRecordKind, NewMemory,
    SourceId, SourcePartRole,
};

pub(super) fn current_correction_is_grounded(
    source_parts: &std::collections::HashMap<u32, (SourcePartRole, &str)>,
    record: &NewMemory,
    correction: &CorrectionEvidence,
) -> bool {
    let spans = [
        &correction.subject,
        &correction.property,
        &correction.old_value,
        &correction.new_value,
    ];
    let Some((role, source_text)) = source_parts.get(&correction.subject.part_index) else {
        return false;
    };
    if *role != SourcePartRole::User
        || spans
            .iter()
            .any(|span| span.part_index != correction.subject.part_index)
    {
        return false;
    }
    for span in spans {
        let Some(literal) = source_text.get(span.start_byte as usize..span.end_byte as usize)
        else {
            return false;
        };
        if span.start_byte >= span.end_byte
            || literal != span.quote
            || !record.evidence.iter().any(|evidence| {
                evidence.part_index == span.part_index
                    && evidence.start_byte <= span.start_byte
                    && evidence.end_byte >= span.end_byte
            })
        {
            return false;
        }
    }
    if !matches!(
        correction.subject.quote.to_ascii_lowercase().as_str(),
        "i" | "my"
    ) || correction.property.quote.trim().is_empty()
        || correction.old_value.quote.trim().is_empty()
        || correction.new_value.quote.trim().is_empty()
        || correction
            .old_value
            .quote
            .trim()
            .eq_ignore_ascii_case(correction.new_value.quote.trim())
        || correction.old_value.start_byte < correction.property.end_byte
        || correction.new_value.start_byte < correction.property.end_byte
        || !adjacent_by_whitespace(
            source_text,
            correction.subject.end_byte,
            correction.property.start_byte,
        )
        || is_inside_quotes(source_text, correction.subject.start_byte as usize)
        || !has_first_person_assertion_prefix(
            source_text,
            correction.subject.start_byte as usize,
            Some(correction.marker),
        )
    {
        return false;
    }

    let span_start = correction.subject.start_byte;
    let span_end = correction
        .old_value
        .end_byte
        .max(correction.new_value.end_byte);
    let Some(evidence_quote) = record.evidence.iter().find(|evidence| {
        evidence.part_index == correction.subject.part_index
            && evidence.start_byte <= span_start
            && evidence.end_byte >= span_end
    }) else {
        return false;
    };
    if !contains_phrase(&evidence_quote.quote, correction.marker.phrase()) {
        return false;
    }
    true
}

fn context_has_property_value(
    quote: &str,
    full_source: &str,
    quote_start: u32,
    subject: &str,
    property: &str,
    value: &str,
) -> bool {
    for subject_start in phrase_positions(quote, subject) {
        let subject_end = subject_start + subject.len();
        let absolute_subject_start = quote_start as usize + subject_start;
        if is_inside_quotes(full_source, absolute_subject_start)
            || !has_first_person_assertion_prefix(full_source, absolute_subject_start, None)
        {
            continue;
        }
        for property_start in phrase_positions(quote, property) {
            if property_start < subject_end
                || !adjacent_by_whitespace(quote, subject_end as u32, property_start as u32)
            {
                continue;
            }
            let property_end = property_start + property.len();
            if phrase_positions(quote, value)
                .into_iter()
                .any(|value_start| {
                    value_start >= property_end
                        && value_gap_is_plain(quote, property_end, value_start)
                })
            {
                return true;
            }
        }
    }
    false
}

fn value_gap_is_plain(text: &str, start: usize, end: usize) -> bool {
    let Some(gap) = text.get(start..end) else {
        return false;
    };
    let gap = gap.trim().to_ascii_lowercase();
    gap.is_empty()
        || matches!(
            gap.as_str(),
            "is" | "are" | "was" | "were" | "am" | ":" | "="
        )
}

fn adjacent_by_whitespace(text: &str, left_end: u32, right_start: u32) -> bool {
    if right_start < left_end {
        return false;
    }
    text.get(left_end as usize..right_start as usize)
        .is_some_and(|between| between.chars().all(char::is_whitespace))
}

fn contains_phrase(text: &str, phrase: &str) -> bool {
    phrase_positions(text, phrase).into_iter().any(|start| {
        let end = start + phrase.len();
        let before_is_word = text[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric);
        let after_is_word = text[end..]
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric);
        !before_is_word && !after_is_word
    })
}

fn phrase_positions(text: &str, phrase: &str) -> Vec<usize> {
    if phrase.is_empty() || phrase.len() > text.len() {
        return Vec::new();
    }
    text.char_indices()
        .filter_map(|(start, _)| {
            text.get(start..start + phrase.len())
                .filter(|candidate| candidate.eq_ignore_ascii_case(phrase))
                .map(|_| start)
        })
        .collect()
}

fn has_first_person_assertion_prefix(
    text: &str,
    byte_offset: usize,
    required_marker: Option<CorrectionMarker>,
) -> bool {
    let Some(prefix) = text.get(..byte_offset) else {
        return false;
    };
    let line_start = prefix.rfind(['\n', '\r']).map_or(0, |index| index + 1);
    let preceding_line_text = &prefix[line_start..];
    let preceding = preceding_line_text.trim_start();
    if preceding.trim().is_empty() {
        return true;
    }

    const MARKERS: [CorrectionMarker; 7] = [
        CorrectionMarker::InsteadOf,
        CorrectionMarker::RatherThan,
        CorrectionMarker::NoLonger,
        CorrectionMarker::IMeant,
        CorrectionMarker::Actually,
        CorrectionMarker::Correction,
        CorrectionMarker::Instead,
    ];
    MARKERS.into_iter().any(|marker| {
        if required_marker.is_some_and(|required| required != marker) {
            return false;
        }
        let phrase = marker.phrase();
        let Some(found) = preceding.get(..phrase.len()) else {
            return false;
        };
        if !found.eq_ignore_ascii_case(phrase) {
            return false;
        }
        let suffix = &preceding[phrase.len()..];
        if suffix.chars().next().is_some_and(char::is_alphanumeric) {
            return false;
        }
        suffix.chars().all(|character| {
            character.is_whitespace()
                || character.is_ascii_punctuation()
                || matches!(character, '—' | '–')
        })
    })
}

fn is_inside_quotes(text: &str, byte_offset: usize) -> bool {
    let mut double_quoted = false;
    let mut single_quoted = false;
    for (index, character) in text.char_indices() {
        if index >= byte_offset {
            break;
        }
        match character {
            '"' => double_quoted = !double_quoted,
            '“' => double_quoted = true,
            '”' => double_quoted = false,
            '\'' => {
                let before_is_word = text[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric);
                let after_is_word = text[index + character.len_utf8()..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphanumeric);
                if !(before_is_word && after_is_word) {
                    single_quoted = !single_quoted;
                }
            }
            '‘' => single_quoted = true,
            '’' => {
                let before_is_word = text[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric);
                let after_is_word = text[index + character.len_utf8()..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphanumeric);
                if !(before_is_word && after_is_word) {
                    single_quoted = false;
                }
            }
            _ => {}
        }
    }
    double_quoted || single_quoted
}

#[cfg(test)]
#[path = "../../../tests/unit/storage/memory/corrections_tests.rs"]
mod tests;

const MAX_CORRECTION_CANDIDATES: usize = 33;

#[derive(Clone, Copy)]
pub(super) struct CorrectionDecision {
    pub(super) state: CorrectionState,
    pub(super) supersedes_id: Option<MemoryId>,
}

impl CorrectionDecision {
    pub(super) fn needs_review() -> Self {
        Self {
            state: CorrectionState::NeedsReview,
            supersedes_id: None,
        }
    }
}

pub(super) fn find_correction_candidates(
    transaction: &rusqlite::Transaction<'_>,
    source_id: SourceId,
    kind: MemoryRecordKind,
    correction: &CorrectionEvidence,
) -> Result<Vec<i64>, rusqlite::Error> {
    let ids = {
        let mut statement = transaction.prepare(
            "SELECT DISTINCT m.id
             FROM memory_records m
             JOIN memory_sources s ON s.id = m.source_id
             WHERE m.source_id < ?1 AND m.record_kind = ?2
               AND m.attribution = 'user_statement' AND m.status = 'active'
               AND s.kind = 'conversation' AND s.state = 'active'
               AND NOT EXISTS (
                   SELECT 1 FROM source_suppressions ss WHERE ss.source_id = s.id
               )
               AND EXISTS (
                   SELECT 1 FROM memory_evidence e
                   JOIN source_parts sp
                     ON sp.source_id = e.source_id AND sp.part_index = e.part_index
                   WHERE e.record_id = m.id AND sp.role = 'user'
                     AND e.quote IS NOT NULL
                     AND instr(e.quote, ?3) > 0 AND instr(e.quote, ?4) > 0
               )
             ORDER BY m.id DESC LIMIT ?5",
        )?;
        statement
            .query_map(
                params![
                    source_id.0,
                    kind.as_str(),
                    correction.property.quote,
                    correction.old_value.quote,
                    MAX_CORRECTION_CANDIDATES as i64,
                ],
                |row| row.get::<_, i64>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    if ids.len() >= MAX_CORRECTION_CANDIDATES {
        return Ok(ids);
    }

    let mut candidates = Vec::new();
    for id in ids {
        if correction_matches_prior_memory(transaction, MemoryId(id), correction)? {
            candidates.push(id);
            if candidates.len() > 1 {
                break;
            }
        }
    }
    Ok(candidates)
}

fn correction_matches_prior_memory(
    transaction: &rusqlite::Transaction<'_>,
    memory_id: MemoryId,
    correction: &CorrectionEvidence,
) -> Result<bool, rusqlite::Error> {
    let evidence = {
        let mut statement = transaction.prepare(
            "SELECT e.source_id, e.part_index, e.start_byte, e.end_byte, e.quote
             FROM memory_evidence e
             JOIN source_parts sp
               ON sp.source_id = e.source_id AND sp.part_index = e.part_index
             WHERE e.record_id = ?1 AND sp.role = 'user' AND e.quote IS NOT NULL
             LIMIT 8",
        )?;
        statement
            .query_map([memory_id.0], |row| {
                Ok((
                    SourceId(row.get(0)?),
                    row.get::<_, u32>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (source_id, part_index, start_byte, end_byte, quote) in evidence {
        let Some((role, source_text)) = load_source_part(transaction, source_id, part_index)?
        else {
            continue;
        };
        if role != SourcePartRole::User
            || source_text
                .get(start_byte as usize..end_byte as usize)
                .is_none_or(|slice| slice != quote)
        {
            continue;
        }
        if context_has_property_value(
            &quote,
            &source_text,
            start_byte,
            &correction.subject.quote,
            &correction.property.quote,
            &correction.old_value.quote,
        ) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn supersede_corrected_memory(
    transaction: &rusqlite::Transaction<'_>,
    memory_id: MemoryId,
    now: i64,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "UPDATE memory_records
         SET status = 'superseded', projection_generation = projection_generation + 1,
             projection_op = 'delete', projection_lease_token = projection_lease_token + 1,
             projection_lease_until_unix_ms = NULL, projection_retry_after_unix_ms = ?2
         WHERE id = ?1 AND status = 'active'",
        params![memory_id.0, now],
    )?;
    transaction.execute(
        "UPDATE memory_records
         SET status = 'superseded', projection_generation = projection_generation + 1,
             projection_op = 'delete', projection_lease_token = projection_lease_token + 1,
             projection_lease_until_unix_ms = NULL, projection_retry_after_unix_ms = ?2
         WHERE status = 'active' AND record_kind = 'source_excerpt'
           AND attribution = 'user_statement'
           AND id IN (
               SELECT chunk.record_id FROM memory_evidence chunk
               JOIN source_parts sp ON sp.source_id = chunk.source_id
                    AND sp.part_index = chunk.part_index AND sp.role = 'user'
               WHERE EXISTS (
                   SELECT 1 FROM memory_evidence fact
                   WHERE fact.record_id = ?1
                     AND fact.source_id = chunk.source_id
                     AND fact.part_index = chunk.part_index
                     AND fact.start_byte < chunk.end_byte
                     AND fact.end_byte > chunk.start_byte
               )
           )",
        params![memory_id.0, now],
    )?;
    Ok(())
}
