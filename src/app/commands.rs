#[derive(Debug, Eq, PartialEq)]
pub(super) enum Command {
    Exit,
    Stop,
    Reset,
    Help,
    Prompt(String),
    ListMemories(Option<String>),
    ForgetId(String),
    ForgetFocus,
    ForgetDetail(String),
    Import(String),
    MemoryStatus,
    Retry(String),
    Rebuild,
}

pub(super) fn parse(input: String) -> Command {
    let trimmed = input.trim();
    match trimmed {
        "/exit" => return Command::Exit,
        "/stop" => return Command::Stop,
        "/reset" => return Command::Reset,
        "/help" => return Command::Help,
        "/memories" => return Command::ListMemories(None),
        "/memory status" => return Command::MemoryStatus,
        "/memory rebuild" => return Command::Rebuild,
        _ => {}
    }

    if let Some(query) = trimmed.strip_prefix("/memories ") {
        return Command::ListMemories(Some(query.trim().to_owned()));
    }
    if let Some(id) = trimmed.strip_prefix("/forget ") {
        return Command::ForgetId(id.trim().to_owned());
    }
    if let Some(path) = input.trim_start().strip_prefix("/import ") {
        return Command::Import(path.trim_start().to_owned());
    }
    if let Some(job_id) = trimmed.strip_prefix("/memory retry ") {
        return Command::Retry(job_id.trim().to_owned());
    }

    let natural = trimmed.trim_start();
    if let Some(suffix) = strip_leading_phrase(natural, "forget that")
        && is_deictic_suffix(suffix)
    {
        return Command::ForgetFocus;
    }
    if let Some(detail) = strip_leading_phrase(natural, "forget ") {
        let detail = detail.trim();
        if !detail.is_empty() {
            return Command::ForgetDetail(detail.to_owned());
        }
    }

    Command::Prompt(input)
}

fn is_deictic_suffix(suffix: &str) -> bool {
    let suffix = suffix.trim();
    if suffix.is_empty() {
        return true;
    }

    let suffix = suffix.trim_start_matches(is_punctuation).trim_start();
    if suffix.is_empty() {
        return true;
    }

    strip_leading_phrase(suffix, "please")
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
#[path = "../../tests/unit/app_command_tests.rs"]
mod tests;
