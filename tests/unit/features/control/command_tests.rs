use orb_protocol::PanelName;

use crate::storage::{FactId, MemoryId};

use super::{COMMANDS, Command, MemoryCommand, REMEMBER_USAGE, help_text, parse};

const FACT_USAGE: &str =
    "Use /memory forget followed by a fact number from /memory facts. (Memories use /forget <id>.)";
const UNKNOWN: &str = "Unknown command. Type /help for commands.";

fn remember(fact: &str) -> Command {
    Command::Remember(fact.to_owned())
}

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
        ("close it", Command::Close),
        ("Close it.", Command::Close),
        ("hide that", Command::Close),
        ("  HIDE THAT  ", Command::Close),
        ("close", Command::Prompt("close".into())),
        ("close the stats", Command::Prompt("close the stats".into())),
        ("don't close it yet", Command::Prompt("don't close it yet".into())),
        ("close it now", Command::Prompt("close it now".into())),
        ("hide", Command::Prompt("hide".into())),
        ("bring up the stats", Command::Show(PanelName::Stats)),
        ("Bring up the stats.", Command::Show(PanelName::Stats)),
        ("Okay, bring up the stats again please.", Command::Show(PanelName::Stats)),
        ("Please show me the dashboard", Command::Show(PanelName::Stats)),
        ("  OPEN THE STATS!  ", Command::Show(PanelName::Stats)),
        ("bring up the stats now", Command::Prompt("bring up the stats now".into())),
        ("don't bring up the stats", Command::Prompt("don't bring up the stats".into())),
        ("show me the stats for the other pi", Command::Prompt("show me the stats for the other pi".into())),
        ("Bring up the stats. Bring up the stats.", Command::Prompt("Bring up the stats. Bring up the stats.".into())),
        ("/help", Command::Help),
        ("/memories", memory(MemoryCommand::List(None))),
        ("/memories work project", memory(MemoryCommand::List(Some("work project".into())))),
        ("/memory status", memory(MemoryCommand::Status)),
        ("/memory rebuild", memory(MemoryCommand::Rebuild)),
        ("/memory facts", memory(MemoryCommand::Facts)),
        ("/memory forget 7", memory(MemoryCommand::ForgetFact(FactId(7)))),
        ("  /memory forget   12 ", memory(MemoryCommand::ForgetFact(FactId(12)))),
        // Spoken requests to list the facts, with the usual prefixes and punctuation.
        ("what facts do you have", memory(MemoryCommand::Facts)),
        ("What facts do you have?", memory(MemoryCommand::Facts)),
        ("list my facts", memory(MemoryCommand::Facts)),
        ("List facts.", memory(MemoryCommand::Facts)),
        ("Hey Jarvis, list my facts please.", memory(MemoryCommand::Facts)),
        ("Okay, what have you noted?", memory(MemoryCommand::Facts)),
        ("  please list the facts  ", memory(MemoryCommand::Facts)),
        ("what facts do you know about the moon", Command::Prompt("what facts do you know about the moon".into())),
        ("list my facts about cats", Command::Prompt("list my facts about cats".into())),
        ("what facts do you have about cats", Command::Prompt("what facts do you have about cats".into())),
        ("tell me some facts", Command::Prompt("tell me some facts".into())),
        ("list", Command::Prompt("list".into())),
        ("what have you noted down about me today", Command::Prompt("what have you noted down about me today".into())),
        ("/memory forget", Command::Usage(FACT_USAGE)),
        ("/memory forget cat", Command::Usage(FACT_USAGE)),
        ("/memory forget 1 2", Command::Usage(FACT_USAGE)),
        // Memories keep their own command and ids.
        ("/forget 7", memory(MemoryCommand::Forget(MemoryId(7)))),
        ("/forget cat", Command::Usage("Use /forget followed by a numeric memory ID.")),
        // The import path keeps its inner spaces.
        ("/import notes from 2024.md", memory(MemoryCommand::Import("notes from 2024.md".into()))),
        // /recall runs the question as a prompt with a search; without one it shows a hint.
        ("/recall what is my cat called?", Command::Recall("what is my cat called?".into())),
        ("  /recall   last time we met  ", Command::Recall("last time we met".into())),
        ("/recall", Command::Usage("Use /recall followed by what you want to remember.")),
        ("  /recall  ", Command::Usage("Use /recall followed by what you want to remember.")),
        ("/recalled it", Command::Usage(UNKNOWN)),
        // An ordinary line that mentions "recall" is still a prompt (the REPL decides on a search).
        ("do you recall my cat?", Command::Prompt("do you recall my cat?".into())),
        ("recall my cat", Command::Prompt("recall my cat".into())),
        ("what does /recall do", Command::Prompt("what does /recall do".into())),
        // "remember that" and /remember save the rest as said, whatever the case or punctuation.
        ("remember that my cat is Miso", remember("my cat is Miso")),
        ("Remember that my cat is Miso.", remember("my cat is Miso.")),
        ("REMEMBER THAT My Cat Is Miso", remember("My Cat Is Miso")),
        ("  remember that   my cat is Miso  ", remember("my cat is Miso")),
        ("Remember that, my cat is Miso", remember("my cat is Miso")),
        ("remember that: my cat is Miso", remember("my cat is Miso")),
        ("Please remember that my cat is Miso", remember("my cat is Miso")),
        ("Hey Jarvis, remember that my cat is Miso.", remember("my cat is Miso.")),
        ("hey jarvis remember that my cat is Miso", remember("my cat is Miso")),
        ("Okay, Jarvis, please remember that my cat is Miso", remember("my cat is Miso")),
        ("remember that timeout is 5 seconds", remember("timeout is 5 seconds")),
        ("remember that one of my cats is Miso", Command::Prompt("remember that one of my cats is Miso".into())),
        ("/remember my cat is Miso", remember("my cat is Miso")),
        ("  /remember   my cat is Miso  ", remember("my cat is Miso")),
        ("/remember", Command::Usage(REMEMBER_USAGE)),
        ("  /remember  ", Command::Usage(REMEMBER_USAGE)),
        ("remember that", Command::Usage(REMEMBER_USAGE)),
        ("Remember that.", Command::Usage(REMEMBER_USAGE)),
        ("Hey Jarvis, remember that", Command::Usage(REMEMBER_USAGE)),
        ("/remembers it", Command::Usage(UNKNOWN)),
        // Near-misses stay prompts: a question about the past, a different word, a mid-line phrase.
        ("remember when we met", Command::Prompt("remember when we met".into())),
        ("remember that time we met", Command::Prompt("remember that time we met".into())),
        ("Remember that day?", Command::Prompt("Remember that day?".into())),
        ("remember that night, it was cold", Command::Prompt("remember that night, it was cold".into())),
        ("Hey Jarvis, remember that one time", Command::Prompt("Hey Jarvis, remember that one time".into())),
        ("remember that one", Command::Prompt("remember that one".into())),
        ("remembering my cat", Command::Prompt("remembering my cat".into())),
        ("remember thatcher is a name", Command::Prompt("remember thatcher is a name".into())),
        ("remember that's odd", Command::Prompt("remember that's odd".into())),
        ("remember my cat is Miso", Command::Prompt("remember my cat is Miso".into())),
        ("do you remember that my cat is Miso", Command::Prompt("do you remember that my cat is Miso".into())),
        ("I remember that my cat is Miso", Command::Prompt("I remember that my cat is Miso".into())),
        ("okayremember that my cat is Miso", Command::Prompt("okayremember that my cat is Miso".into())),
        // /forget takes exactly a numeric id.
        ("/forget 42", memory(MemoryCommand::Forget(MemoryId(42)))),
        ("/forget 4x2", Command::Usage("Use /forget followed by a numeric memory ID.")),
        // "forget that" plus only punctuation or "please" targets the focus...
        ("Forget that", memory(MemoryCommand::ForgetFocus)),
        ("forget that, please.", memory(MemoryCommand::ForgetFocus)),
        // ...anything more is a detail to search for.
        ("forget that file from yesterday", memory(MemoryCommand::ForgetDetail("that file from yesterday".into()))),
        ("forget my preferred editor", memory(MemoryCommand::ForgetDetail("my preferred editor".into()))),
        // As transcribed on the Pi on 2026-11-21, trailing "you" included.
        ("Can you forget that my sister's name is Mia? you", memory(MemoryCommand::ForgetDetail("that my sister's name is Mia? you".into()))),
        ("Jarvis, could you please forget that?", memory(MemoryCommand::ForgetFocus)),
        ("would you forget my editor", memory(MemoryCommand::ForgetDetail("my editor".into()))),
        ("can you forgetting", Command::Prompt("can you forgetting".into())),
        ("can you tell me what to forget", Command::Prompt("can you tell me what to forget".into())),
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
fn help_lists_remember() {
    assert!(help_text().contains("/remember <fact>"));
}

#[test]
fn help_lists_every_command_once() {
    let help = help_text();
    assert!(help.starts_with("Commands:"));
    for (usage, _) in COMMANDS {
        assert_eq!(help.matches(usage).count(), 1, "{usage}");
    }
}

#[test]
fn help_lists_the_fact_commands() {
    let help = help_text();
    assert!(help.contains("/memory facts"));
    assert!(help.contains("/memory forget <number>"));
}
