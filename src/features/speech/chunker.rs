//! Incremental splitting of streamed model text into speakable chunks (spec S5).
//!
//! Pure and synchronous: no I/O, no timers. All lengths are counted in `char`s
//! (Unicode scalar values), never bytes.
//!
//! The speech pipeline chunks the raw streamed text first. Any speech-text
//! cleanup (Markdown stripping and similar) would run on each finished chunk,
//! because it needs whole spans (a `**bold**` pair or a code fence can straddle
//! deltas) while chunking needs only punctuation and whitespace. The repository
//! has no such cleanup yet; `SpeechOutput` only skips chunks with nothing to say.

/// Characters of text, before the punctuation mark, that the first chunk needs
/// before it may end at a sentence end or clause break.
pub(crate) const MIN_FIRST_CHUNK_CHARS: usize = 40;

/// Pending text longer than this many characters is force-split.
pub(crate) const MAX_CHUNK_CHARS: usize = 300;

/// Lowercase tokens (including the final period) that never end a sentence.
const ABBREVIATIONS: [&str; 8] = ["dr.", "mr.", "mrs.", "ms.", "e.g.", "i.e.", "etc.", "vs."];

/// Splits streamed text deltas into chunks for speech synthesis.
///
/// Rules: the first chunk ends at the first sentence end (`.` `!` `?`) or
/// clause break (`,` `;` `:`) once at least [`MIN_FIRST_CHUNK_CHARS`]
/// characters precede the mark; later chunks end only at sentence ends; pending
/// text beyond [`MAX_CHUNK_CHARS`] is split at the last whitespace at or before
/// that length (or exactly there). A mark ends a chunk only when whitespace
/// follows it, so the trailing mark is held back until the next character
/// arrives or [`SpeechChunker::finish`] is called. `Dr.`-style abbreviations,
/// decimals such as `3.5` and list markers such as `1.` never end a sentence.
///
/// Feeding the same text in any delta split yields the same chunks.
#[derive(Debug, Default)]
pub(crate) struct SpeechChunker {
    /// Unemitted text; never starts with whitespace.
    pending: String,
    has_emitted: bool,
}

impl SpeechChunker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Adds a streamed delta and returns the chunks it completed, trimmed and
    /// never empty.
    pub(crate) fn push(&mut self, delta: &str) -> Vec<String> {
        self.pending.push_str(delta);
        self.drop_leading_whitespace();

        let mut chunks = Vec::new();
        while let Some(end) = self.next_cut() {
            let chunk = self.pending[..end].trim();
            if !chunk.is_empty() {
                chunks.push(chunk.to_owned());
                self.has_emitted = true;
            }
            self.pending.drain(..end);
            self.drop_leading_whitespace();
        }
        chunks
    }

    /// Flushes the remaining text, trimmed, or `None` if nothing is left.
    pub(crate) fn finish(self) -> Option<String> {
        let rest = self.pending.trim();
        (!rest.is_empty()).then(|| rest.to_owned())
    }

    fn drop_leading_whitespace(&mut self) {
        let trimmed = self.pending.len() - self.pending.trim_start().len();
        self.pending.drain(..trimmed);
    }

    /// Byte offset where the next chunk ends, if one is complete.
    fn next_cut(&self) -> Option<usize> {
        self.boundary_end().or_else(|| self.force_split_end())
    }

    /// End (exclusive, in bytes) of the first confirmed boundary whose chunk
    /// fits within [`MAX_CHUNK_CHARS`].
    fn boundary_end(&self) -> Option<usize> {
        let mut chars = self.pending.char_indices().enumerate().peekable();
        while let Some((index, (byte, mark))) = chars.next() {
            if index >= MAX_CHUNK_CHARS {
                return None;
            }
            let ends_sentence = matches!(mark, '.' | '!' | '?');
            let ends_clause = matches!(mark, ',' | ';' | ':');
            if !(ends_sentence || (ends_clause && !self.has_emitted)) {
                continue;
            }
            if !self.has_emitted && index < MIN_FIRST_CHUNK_CHARS {
                continue;
            }
            // No next character yet: the mark stays pending until one arrives.
            let &(_, (_, next)) = chars.peek()?;
            if !next.is_whitespace() {
                continue;
            }
            let end = byte + mark.len_utf8();
            if mark == '.'
                && (is_abbreviation(&self.pending[..end]) || is_list_marker(&self.pending[..end]))
            {
                continue;
            }
            return Some(end);
        }
        None
    }

    /// Cut point (bytes) when pending text exceeds [`MAX_CHUNK_CHARS`].
    fn force_split_end(&self) -> Option<usize> {
        let mut indices = self.pending.char_indices();
        // The character after the limit: its presence means the text is too long.
        let (limit_byte, limit_char) = indices.nth(MAX_CHUNK_CHARS)?;
        if limit_char.is_whitespace() {
            return Some(limit_byte);
        }
        let head = &self.pending[..limit_byte];
        Some(
            head.char_indices()
                .rev()
                .find(|(_, c)| c.is_whitespace())
                .map_or(limit_byte, |(byte, _)| byte),
        )
    }
}

/// Whether `text_ending_in_period` is only a number and its period, such as the
/// `1.` that starts a numbered list item. The pending text starts at the chunk
/// start, so a number inside a sentence (`it costs 5.`) never matches.
fn is_list_marker(text_ending_in_period: &str) -> bool {
    text_ending_in_period
        .strip_suffix('.')
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether the last whitespace-delimited word of `text_ending_in_period` is a
/// known abbreviation. Leading punctuation such as `(` is ignored.
fn is_abbreviation(text_ending_in_period: &str) -> bool {
    let word = text_ending_in_period
        .rsplit(char::is_whitespace)
        .next()
        .unwrap_or_default();
    let word = word.trim_start_matches(|c: char| !c.is_alphanumeric());
    ABBREVIATIONS
        .iter()
        .any(|abbreviation| word.eq_ignore_ascii_case(abbreviation))
}

#[cfg(test)]
#[path = "../../../tests/unit/features/speech/chunker_tests.rs"]
mod tests;
