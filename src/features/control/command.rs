use crate::storage::MemoryId;

/// What one line of user input asks for. Parsed the same way whether it was
/// typed or (later) transcribed from speech.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Command {
    Exit,
    Stop,
    Reset,
    Help,
    Memory(MemoryCommand),
    /// A recognized command with unusable arguments; holds the usage hint.
    Usage(&'static str),
    Prompt(String),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum MemoryCommand {
    List(Option<String>),
    Forget(MemoryId),
    ForgetFocus,
    ForgetDetail(String),
    Import(String),
    Status,
    Rebuild,
}

/// The single list of commands; `help_text` is generated from it.
#[rustfmt::skip]
pub(super) const COMMANDS: &[(&str, &str)] = &[
    ("/memories [text]", "list recent memories, optionally filtered by text"),
    ("/forget <id>",     "forget a memory and everything built from it"),
    ("forget that",      "forget the last answer or the single listed memory"),
    ("forget <detail>",  "forget the one memory that matches <detail>"),
    ("/import <path>",   "remember a .txt or .md file (up to 1 MiB)"),
    ("/memory status",   "show memory counts and embedder state"),
    ("/memory rebuild",  "re-embed every memory while idle"),
    ("/reset",           "clear the conversation context and start a new session"),
    ("/stop",            "stop speaking"),
    ("/help",            "show this list"),
    ("/exit",            "quit"),
];

pub(crate) fn help_text() -> String {
    let width = COMMANDS
        .iter()
        .map(|(usage, _)| usage.len())
        .max()
        .unwrap_or(0);
    let mut text = String::from("Commands:");
    for (usage, summary) in COMMANDS {
        text.push_str(&format!("\n  {usage:<width$}  {summary}"));
    }
    text.push_str("\nNatural forgetting only proceeds for one current target or one unique match.");
    text
}

pub(crate) fn parse(input: String) -> Command {
    let trimmed = input.trim();
    match trimmed {
        "/exit" => return Command::Exit,
        "/stop" => return Command::Stop,
        "/reset" => return Command::Reset,
        "/help" => return Command::Help,
        "/memories" => return Command::Memory(MemoryCommand::List(None)),
        "/memory status" => return Command::Memory(MemoryCommand::Status),
        "/memory rebuild" => return Command::Memory(MemoryCommand::Rebuild),
        _ => {}
    }

    if let Some(query) = trimmed.strip_prefix("/memories ") {
        return Command::Memory(MemoryCommand::List(Some(query.trim().to_owned())));
    }
    if let Some(id) = trimmed.strip_prefix("/forget ") {
        return match id.trim().parse::<i64>() {
            Ok(id) => Command::Memory(MemoryCommand::Forget(MemoryId(id))),
            Err(_) => Command::Usage("Use /forget followed by a numeric memory ID."),
        };
    }
    if let Some(path) = input.trim_start().strip_prefix("/import ") {
        return Command::Memory(MemoryCommand::Import(path.trim_start().to_owned()));
    }

    if let Some(suffix) = strip_leading_phrase(trimmed, "forget that")
        && is_deictic_suffix(suffix)
    {
        return Command::Memory(MemoryCommand::ForgetFocus);
    }
    if let Some(detail) = strip_leading_phrase(trimmed, "forget ") {
        let detail = detail.trim();
        if !detail.is_empty() {
            return Command::Memory(MemoryCommand::ForgetDetail(detail.to_owned()));
        }
    }

    Command::Prompt(input)
}

/// "forget that", optionally followed by punctuation and/or "please".
fn is_deictic_suffix(suffix: &str) -> bool {
    let suffix = suffix
        .trim()
        .trim_start_matches(is_punctuation)
        .trim_start();
    suffix.is_empty()
        || strip_leading_phrase(suffix, "please")
            .is_some_and(|rest| rest.trim().chars().all(is_punctuation))
}

fn is_punctuation(character: char) -> bool {
    ".!?,;:".contains(character)
}

fn strip_leading_phrase<'a>(input: &'a str, phrase: &str) -> Option<&'a str> {
    input
        .get(..phrase.len())
        .filter(|prefix| prefix.eq_ignore_ascii_case(phrase))
        .map(|_| &input[phrase.len()..])
}

#[cfg(test)]
#[path = "../../../tests/unit/features/control/command_tests.rs"]
mod tests;
