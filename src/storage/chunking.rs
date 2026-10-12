/// Upper bound for one stored memory chunk. BGE-small truncates input at 512
/// tokens; 1,500 bytes of English text stays comfortably below that.
pub(super) const MAX_CHUNK_BYTES: usize = 1500;

/// Formats one completed exchange as the text that is stored and searched.
pub(super) fn conversation_text(user_text: &str, assistant_text: &str) -> String {
    format!(
        "User: {}\nJarvis: {}",
        user_text.trim(),
        assistant_text.trim()
    )
}

/// Splits text into trimmed, non-empty chunks of at most `max_bytes`, preferring
/// paragraph, line, sentence, then word boundaries before a hard UTF-8 cut.
pub(super) fn split(text: &str, max_bytes: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        if rest.len() <= max_bytes {
            chunks.push(rest.to_owned());
            break;
        }
        let cut = boundary(rest, max_bytes);
        let chunk = rest[..cut].trim();
        if !chunk.is_empty() {
            chunks.push(chunk.to_owned());
        }
        rest = rest[cut..].trim_start();
    }
    chunks
}

fn boundary(text: &str, max_bytes: usize) -> usize {
    let mut limit = max_bytes;
    while !text.is_char_boundary(limit) {
        limit -= 1;
    }
    let window = &text[..limit];
    for separator in ["\n\n", "\n", ". ", "? ", "! ", " "] {
        if let Some(index) = window.rfind(separator)
            && index > 0
        {
            return index + separator.len();
        }
    }
    limit
}

#[cfg(test)]
#[path = "../../tests/unit/storage/chunking_tests.rs"]
mod tests;
