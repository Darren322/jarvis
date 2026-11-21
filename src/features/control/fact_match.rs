//! The rule "forget <detail>" uses to find a fact by what Darren said.
//!
//! Speech is transcribed loosely ("my sister name is Mia" for "has a sister named
//! Mia"), so a whole-phrase match is too strict. Instead each meaningful word of the
//! detail must begin a word of the fact.

/// Words that carry no meaning in a "forget" request and are dropped from the detail.
const FILLER_WORDS: &[&str] = &[
    "that", "my", "me", "i", "the", "a", "an", "is", "are", "was", "of", "about", "please",
    "called", "named",
];

/// Whether `fact` is what `detail` points at: after dropping filler words from
/// `detail`, at least one word remains and each begins some word of `fact`, ignoring
/// case and punctuation ("name" matches "named"). A detail of only filler matches nothing.
pub(super) fn detail_matches_fact(detail: &str, fact: &str) -> bool {
    let fact_words = words(fact);
    let mut wanted = words(detail)
        .into_iter()
        .filter(|word| !FILLER_WORDS.contains(&word.as_str()))
        .peekable();
    wanted.peek().is_some()
        && wanted.all(|word| {
            fact_words
                .iter()
                .any(|candidate| candidate.starts_with(&word))
        })
}

/// Lowercase words with punctuation removed; a possessive "'s" is dropped so
/// "sister's" and "sister" agree.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('\u{2019}', "'")
        .split(|character: char| !(character.is_alphanumeric() || character == '\''))
        .map(|word| {
            word.strip_suffix("'s")
                .unwrap_or(word)
                .trim_matches('\'')
                .to_owned()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/features/control/fact_match_tests.rs"]
mod tests;
