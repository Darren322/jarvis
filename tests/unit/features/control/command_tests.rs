use crate::storage::MemoryId;

use super::{COMMANDS, Command, MemoryCommand, help_text, parse};

const UNKNOWN: &str = "Unknown command. Type /help for commands.";

fn memory(command: MemoryCommand) -> Command {
    Command::Memory(command)
}

#[rustfmt::skip]
#[test]
fn parses_each_command_and_its_nearest_non_command() {
    let cases: Vec<(&str, Command)> = vec![
        ("/exit", Command::Exit),
        ("/stop", Command::Stop),
        (" /listen ", Command::Listen),
        ("/listen now", Command::Usage(UNKNOWN)),
        ("  /reset  ", Command::Reset),
        ("/help", Command::Help),
        ("/memories", memory(MemoryCommand::List(None))),
        ("/memories work project", memory(MemoryCommand::List(Some("work project".into())))),
        ("/memory status", memory(MemoryCommand::Status)),
        ("/memory rebuild", memory(MemoryCommand::Rebuild)),
        // The import path keeps its inner spaces.
        ("/import notes from 2024.md", memory(MemoryCommand::Import("notes from 2024.md".into()))),
        // /forget takes exactly a numeric id.
        ("/forget 42", memory(MemoryCommand::Forget(MemoryId(42)))),
        ("/forget 4x2", Command::Usage("Use /forget followed by a numeric memory ID.")),
        // "forget that" plus only punctuation or "please" targets the focus...
        ("Forget that", memory(MemoryCommand::ForgetFocus)),
        ("forget that, please.", memory(MemoryCommand::ForgetFocus)),
        // ...anything more is a detail to search for.
        ("forget that file from yesterday", memory(MemoryCommand::ForgetDetail("that file from yesterday".into()))),
        ("forget my preferred editor", memory(MemoryCommand::ForgetDetail("my preferred editor".into()))),
        // Only a leading "forget" is a command.
        ("I forget that the model is local", Command::Prompt("I forget that the model is local".into())),
        // A mistyped command is never sent to the model; a path or a lone slash still is.
        ("/exits", Command::Usage(UNKNOWN)),
        ("/iste", Command::Usage(UNKNOWN)),
        ("  /speak  ", Command::Usage(UNKNOWN)),
        ("/memory-x", Command::Usage(UNKNOWN)),
        ("/etc/hosts is a file", Command::Prompt("/etc/hosts is a file".into())),
        ("/", Command::Prompt("/".into())),
        ("/ 2 plus 2", Command::Prompt("/ 2 plus 2".into())),
        ("//comment", Command::Prompt("//comment".into())),
        ("what does /listen do", Command::Prompt("what does /listen do".into())),
    ];
    for (input, expected) in cases {
        assert_eq!(parse(input.to_owned()), expected, "{input}");
    }
}

#[test]
fn help_lists_every_command_once() {
    let help = help_text();
    assert!(help.starts_with("Commands:"));
    for (usage, _) in COMMANDS {
        assert_eq!(help.matches(usage).count(), 1, "{usage}");
    }
}
