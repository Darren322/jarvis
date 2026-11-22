use orb_protocol::PanelName;

use super::timer::TimerAnswer;
use crate::storage::{FactId, MemoryId};

/// What one line of user input asks for. Parsed the same way whether it was
/// typed or (later) transcribed from speech.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Command {
    Exit,
    Stop,
    /// Record from the microphone and send what was said. Whichever interface
    /// owns the microphone runs it.
    Listen,
    Reset,
    Help,
    Memory(MemoryCommand),
    /// "close it" or "hide that": hide the panel on the orb screen. No model call.
    Close,
    /// "bring up the stats", "show the camera" and their fixed variants: show a
    /// panel. No model call.
    Show(PanelName),
    /// `/recall <question>`: run the question as a prompt with a memory search forced.
    Recall(String),
    /// `/remember <fact>` or "remember that <fact>": save the fact as stated. No model call
    /// and no memory search.
    Remember(String),
    /// A recognized command with unusable arguments; holds the usage hint.
    Usage(&'static str),
    /// "yes" or "no" on its own (timers D4). It answers the pending timer if there
    /// is one; otherwise `text` goes to the model as an ordinary prompt.
    TimerAnswer {
        answer: TimerAnswer,
        text: String,
    },
    Prompt(String),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum MemoryCommand {
    List(Option<String>),
    Forget(MemoryId),
    /// `/memory facts`: list the facts with their numbers.
    Facts,
    /// `/memory forget <number>`: remove the fact with that number.
    ForgetFact(FactId),
    ForgetFocus,
    ForgetDetail(String),
    Import(String),
    Status,
    Rebuild,
}

impl MemoryCommand {
    /// Whether the reply is short enough to say aloud: the fact and forget commands,
    /// whose only audible answer is on a screenless Pi. Memory listings, imports, status
    /// and rebuild are long or technical, so they are only printed.
    pub(crate) fn is_spoken(&self) -> bool {
        match self {
            Self::Facts | Self::ForgetFact(_) | Self::ForgetFocus | Self::ForgetDetail(_) => true,
            Self::List(_) | Self::Forget(_) | Self::Import(_) | Self::Status | Self::Rebuild => {
                false
            }
        }
    }
}

/// The single list of commands; `help_text` is generated from it.
#[rustfmt::skip]
pub(super) const COMMANDS: &[(&str, &str)] = &[
    ("/memories [text]", "list recent memories, optionally filtered by text"),
    ("/forget <id>",     "forget a memory and everything built from it"),
    ("forget that",      "forget the last answer, the single listed memory, or the fact just saved or noted"),
    ("forget <detail>",  "forget the one memory or fact that matches <detail>"),
    ("/recall <question>", "answer <question> after searching old conversations"),
    ("/remember <fact>", "save a fact about you (\"remember that <fact>\" works too)"),
    ("/import <path>",   "remember a .txt or .md file (up to 1 MiB)"),
    ("/memory facts",   "list saved facts: a number, who gave it (stated or noted) and the text (\"list my facts\" works too)"),
    ("/memory forget <number>", "forget the fact with that number (numbers stay the same as facts come and go)"),
    ("/memory status",   "show memory counts and embedder state"),
    ("/memory rebuild",  "re-embed every memory while idle"),
    ("/reset",           "clear the conversation context and start a new session"),
    ("/listen",          "speak instead of typing; Enter sends what you said"),
    ("bring up the stats", "show the stats on the screen (\"show the stats\" works too)"),
    ("show the camera",  "put the camera's live picture on the screen (\"bring up the camera\" works too)"),
    ("close it",         "hide the panel on the screen (\"hide that\" works too)"),
    ("yes / no",         "set or drop the timer Jarvis just proposed (otherwise sent to the model)"),
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
        "/listen" => return Command::Listen,
        "/reset" => return Command::Reset,
        "/help" => return Command::Help,
        "/recall" => return Command::Usage(RECALL_USAGE),
        "/remember" => return Command::Usage(REMEMBER_USAGE),
        "/memories" => return Command::Memory(MemoryCommand::List(None)),
        "/memory facts" => return Command::Memory(MemoryCommand::Facts),
        "/memory forget" => return Command::Usage(FORGET_FACT_USAGE),
        "/memory status" => return Command::Memory(MemoryCommand::Status),
        "/memory rebuild" => return Command::Memory(MemoryCommand::Rebuild),
        _ => {}
    }

    if is_close_phrase(trimmed) {
        return Command::Close;
    }
    if let Some(answer) = timer_answer(trimmed) {
        return Command::TimerAnswer {
            answer,
            text: trimmed.to_owned(),
        };
    }
    if let Some(panel) = shown_by_phrase(trimmed) {
        return Command::Show(panel);
    }

    if is_list_facts_phrase(trimmed) {
        return Command::Memory(MemoryCommand::Facts);
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
    if let Some(number) = trimmed.strip_prefix("/memory forget ") {
        return match number.trim().parse::<i64>() {
            Ok(number) => Command::Memory(MemoryCommand::ForgetFact(FactId(number))),
            Err(_) => Command::Usage(FORGET_FACT_USAGE),
        };
    }
    if let Some(question) = trimmed.strip_prefix("/recall ") {
        return Command::Recall(question.trim().to_owned());
    }
    if let Some(fact) = trimmed.strip_prefix("/remember ") {
        return Command::Remember(fact.trim().to_owned());
    }
    if let Some(command) = spoken_remember(trimmed) {
        return command;
    }
    if let Some(path) = input.trim_start().strip_prefix("/import ") {
        return Command::Memory(MemoryCommand::Import(path.trim_start().to_owned()));
    }

    // "Can you forget that ..." is a forget too: as a prompt, Gemma would only claim
    // it had forgotten.
    let request = strip_request_prefixes(trimmed);
    if let Some(suffix) = strip_leading_phrase(request, "forget that")
        && is_deictic_suffix(suffix)
    {
        return Command::Memory(MemoryCommand::ForgetFocus);
    }
    if let Some(detail) = strip_leading_phrase(request, "forget ") {
        let detail = detail.trim();
        if !detail.is_empty() {
            return Command::Memory(MemoryCommand::ForgetDetail(detail.to_owned()));
        }
    }

    if is_unknown_command(trimmed) {
        return Command::Usage(UNKNOWN_COMMAND);
    }

    Command::Prompt(input)
}

/// Shown for `/memory forget` with no number or one that is not a number. Memories use
/// `/forget <id>`; facts have their own numbers, from `/memory facts`.
const FORGET_FACT_USAGE: &str =
    "Use /memory forget followed by a fact number from /memory facts. (Memories use /forget <id>.)";

const RECALL_USAGE: &str = "Use /recall followed by what you want to remember.";

/// Shown when a save has nothing after the phrase: `/remember` or "remember that" alone.
pub(super) const REMEMBER_USAGE: &str =
    "Use /remember followed by the fact, or say \"remember that\" and the fact.";

/// Words that, right after "remember that", make the line a question about the past
/// ("remember that time we...") and not a fact to save. Matched as whole words.
const PAST_WORDS: &[&str] = &["time", "day", "night", "one"];

/// Words a spoken line may start with before "remember that", each followed by a
/// space or punctuation: "Hey Jarvis, remember that ...".
const SPOKEN_PREFIXES: &[&str] = &["hey jarvis", "jarvis", "okay", "ok", "please"];

/// "remember that <fact>" with any spoken prefix, as a save. `None` for a line that is
/// not a save ("remember when we...", "remember that time...", "remembering...").
/// The fact keeps the words as said, minus the punctuation joining it to the phrase.
fn spoken_remember(trimmed: &str) -> Option<Command> {
    let line = strip_spoken_prefixes(trimmed);
    let after = strip_whole_word(line, "remember that")?;
    let first_word = after.split_whitespace().next().unwrap_or_default();
    let first_word = first_word.trim_end_matches(is_punctuation);
    if PAST_WORDS
        .iter()
        .any(|word| first_word.eq_ignore_ascii_case(word))
    {
        return None;
    }
    let fact = after.trim_start_matches(|c: char| c.is_whitespace() || is_punctuation(c));
    Some(if fact.is_empty() {
        Command::Usage(REMEMBER_USAGE)
    } else {
        Command::Remember(fact.to_owned())
    })
}

/// Polite openings a spoken forget may have ("Can you forget that ..."), each a whole
/// phrase followed by a space or punctuation.
const REQUEST_PREFIXES: &[&str] = &["can you", "could you", "would you", "will you"];

/// `line` without any leading `SPOKEN_PREFIXES` or `REQUEST_PREFIXES`, in any order
/// ("Jarvis, could you please forget ..."), and the commas or spaces after them.
fn strip_request_prefixes(line: &str) -> &str {
    let mut line = strip_spoken_prefixes(line);
    while let Some(rest) = REQUEST_PREFIXES
        .iter()
        .find_map(|prefix| strip_whole_word(line, prefix))
    {
        line =
            strip_spoken_prefixes(rest.trim_start_matches(|c: char| c.is_whitespace() || c == ','));
    }
    line
}

/// `line` without any leading `SPOKEN_PREFIXES` and the commas or spaces after them.
fn strip_spoken_prefixes(line: &str) -> &str {
    let mut line = line;
    while let Some(rest) = SPOKEN_PREFIXES
        .iter()
        .find_map(|prefix| strip_whole_word(line, prefix))
    {
        line = rest.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
    }
    line
}

/// The spoken requests to list the facts, after `request_core`-style clean-up. Anything
/// longer ("what facts do you know about the moon") is left to the model.
const LIST_FACTS_PHRASES: &[&str] = &[
    "what facts do you have",
    "what facts have you saved",
    "what facts have you noted",
    "what have you noted",
    "list my facts",
    "list the facts",
    "list facts",
    "list all my facts",
    "show my facts",
    "show me my facts",
];

/// A request to list the facts, ignoring case, spoken prefixes ("hey Jarvis"),
/// closing punctuation and a trailing "please".
fn is_list_facts_phrase(trimmed: &str) -> bool {
    let line = strip_spoken_prefixes(trimmed)
        .trim_end_matches(is_punctuation)
        .to_lowercase();
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    let line = line
        .strip_suffix(" please")
        .unwrap_or(&line)
        .trim_end_matches(|c: char| c.is_whitespace() || is_punctuation(c));
    LIST_FACTS_PHRASES.contains(&line)
}

/// `strip_leading_phrase`, but the phrase must end at a word boundary, so "ok" does not
/// match "okay" and "remember that" does not match "remember thatcher".
fn strip_whole_word<'a>(input: &'a str, phrase: &str) -> Option<&'a str> {
    let rest = strip_leading_phrase(input, phrase)?;
    rest.chars()
        .next()
        .is_none_or(|next| !(next.is_alphanumeric() || matches!(next, '\'' | '-')))
        .then_some(rest)
}

/// Shown for a mistyped command such as `/iste`, so it is not sent to the model as a prompt.
const UNKNOWN_COMMAND: &str = "Unknown command. Type /help for commands.";

/// Whether the line starts like a command: `/` then letters, digits, `-` or `_`, up to the
/// first space. A path such as `/etc/hosts` has a second `/`, so it stays a prompt; a bare
/// `/etc` or `/` followed by a word is treated as a mistyped command.
fn is_unknown_command(trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix('/') else {
        return false;
    };
    let name = rest.split(char::is_whitespace).next().unwrap_or_default();
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

/// The whole answers that set a proposed timer.
const YES_PHRASES: &[&str] = &[
    "yes",
    "yes please",
    "yeah",
    "yep",
    "yup",
    "sure",
    "set it",
    "do it",
];

/// The whole answers that drop a proposed timer.
const NO_PHRASES: &[&str] = &["no", "no thanks", "no thank you", "nope", "don't", "do not"];

/// "yes" or "no" on its own (and their close variants), ignoring case and any
/// punctuation speech-to-text puts around them ("Yes." "No, thanks."). Anything
/// more ("yes I think the sky is blue") is left to the model.
fn timer_answer(trimmed: &str) -> Option<TimerAnswer> {
    let words: Vec<String> = trimmed
        .split_whitespace()
        .map(|word| word.trim_matches(is_punctuation).to_lowercase())
        .filter(|word| !word.is_empty())
        .collect();
    let phrase = words.join(" ");
    if YES_PHRASES.contains(&phrase.as_str()) {
        Some(TimerAnswer::Yes)
    } else if NO_PHRASES.contains(&phrase.as_str()) {
        Some(TimerAnswer::No)
    } else {
        None
    }
}

/// "close it" or "hide that", ignoring case and any closing punctuation. Anything
/// more ("close the stats", "don't close it yet") is left to the model.
fn is_close_phrase(trimmed: &str) -> bool {
    let phrase = trimmed.trim_end_matches(is_punctuation);
    ["close it", "hide that"]
        .iter()
        .any(|close| phrase.eq_ignore_ascii_case(close))
}

/// The requests for the stats panel that skip the model, after `request_core`.
/// Anything else ("can I see how the Pi is doing?") is left to the model and its
/// `show_panel` tool, which costs a model run.
const SHOW_STATS_PHRASES: &[&str] = &[
    "bring up the stats",
    "bring up stats",
    "bring up the dashboard",
    "show the stats",
    "show me the stats",
    "show stats",
    "show the dashboard",
    "show me the dashboard",
    "open the stats",
    "open the dashboard",
];

/// The requests for the camera panel that skip the model, after `request_core`
/// (jarvis-ui ticket 11, C5). Anything else ("what does the camera see?") is left
/// to the model.
const SHOW_CAMERA_PHRASES: &[&str] = &[
    "show the camera",
    "bring up the camera",
    "show me the camera",
];

/// Polite words around a request that do not change it: a leading "please",
/// "okay" or "jarvis", and a trailing "please" or "again".
const LEADING_FILLERS: &[&str] = &["please ", "okay ", "ok ", "jarvis "];
const TRAILING_FILLERS: &[&str] = &[" please", " again"];

/// A request without its case, closing punctuation and polite words, so
/// "Okay, bring up the stats again please." reads "bring up the stats".
fn request_core(trimmed: &str) -> String {
    let mut core = trimmed
        .trim_end_matches(is_punctuation)
        .replace(',', "")
        .to_lowercase();
    while let Some(rest) = LEADING_FILLERS
        .iter()
        .find_map(|filler| core.strip_prefix(filler))
    {
        core = rest.trim_start().to_owned();
    }
    while let Some(rest) = TRAILING_FILLERS
        .iter()
        .find_map(|filler| core.strip_suffix(filler))
    {
        core = rest.trim_end().to_owned();
    }
    core
}

/// The panel a fixed show phrase asks for, if `trimmed` is one.
fn shown_by_phrase(trimmed: &str) -> Option<PanelName> {
    let core = request_core(trimmed);
    if SHOW_STATS_PHRASES.contains(&core.as_str()) {
        Some(PanelName::Stats)
    } else if SHOW_CAMERA_PHRASES.contains(&core.as_str()) {
        Some(PanelName::Camera)
    } else {
        None
    }
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
