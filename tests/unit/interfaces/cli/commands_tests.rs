use super::{Command, parse};

#[test]
fn parses_memory_controls_without_rewriting_import_paths() {
    assert_eq!(parse("/memories".into()), Command::ListMemories(None));
    assert_eq!(
        parse("/memories work project".into()),
        Command::ListMemories(Some("work project".into()))
    );
    assert_eq!(parse("/forget 42".into()), Command::ForgetId("42".into()));
    assert_eq!(
        parse("/import notes from 2024.md".into()),
        Command::Import("notes from 2024.md".into())
    );
    assert_eq!(parse("/memory status".into()), Command::MemoryStatus);
    assert_eq!(parse("/memory retry 9".into()), Command::Retry("9".into()));
    assert_eq!(parse("/memory rebuild".into()), Command::Rebuild);
}

#[test]
fn parses_natural_forgetting_only_when_it_leads_the_prompt() {
    assert_eq!(parse("Forget that".into()), Command::ForgetFocus);
    assert_eq!(parse("forget that, please.".into()), Command::ForgetFocus);
    assert_eq!(
        parse("forget that I mentioned earlier".into()),
        Command::ForgetDetail("that I mentioned earlier".into())
    );
    assert_eq!(
        parse("forget my preferred editor".into()),
        Command::ForgetDetail("my preferred editor".into())
    );
    assert_eq!(
        parse("I forget that the model is local".into()),
        Command::Prompt("I forget that the model is local".into())
    );
}
