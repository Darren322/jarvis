use std::io::Write;

use super::{AssistantPresenter, AssistantState, TerminalPresenter};

impl<W: Write> TerminalPresenter<W> {
    pub(in crate::interfaces::cli) fn into_writer(self) -> W {
        self.output
    }
}

#[test]
fn a_notice_starts_on_its_own_line_and_redraws_an_open_prompt() {
    const NOTICE: &str = "Speech diagnostics (previous answer): time_to_first_audio=3.4s";
    const UNAVAILABLE: &str =
        "Speech diagnostics (previous answer): time_to_first_audio=unavailable";
    let cases: [(&str, bool, &str, String); 3] = [
        ("prompt not visible", false, NOTICE, format!("{NOTICE}\n")),
        (
            "prompt visible",
            true,
            NOTICE,
            format!("You> \n{NOTICE}\nYou> "),
        ),
        (
            "first audio unavailable with prompt visible",
            true,
            UNAVAILABLE,
            format!("You> \n{UNAVAILABLE}\nYou> "),
        ),
    ];
    for (name, prompt_visible, line, expected) in cases {
        let mut presenter = TerminalPresenter::with_writer(Vec::new());
        if prompt_visible {
            presenter.prompt().expect("prompt should write");
        }
        presenter.notice(line).expect("notice should write");
        let output = String::from_utf8(presenter.into_writer()).expect("output is utf-8");
        assert_eq!(output, expected, "{name}");
    }
}

#[test]
fn a_notice_keeps_the_prompt_open_so_it_is_not_drawn_twice() {
    let mut presenter = TerminalPresenter::with_writer(Vec::new());
    presenter.prompt().expect("prompt should write");
    presenter.notice("note").expect("notice should write");
    presenter.prompt().expect("repeat prompt is a no-op");
    let output = String::from_utf8(presenter.into_writer()).expect("output is utf-8");
    assert_eq!(output, "You> \nnote\nYou> ");
}

#[test]
fn a_notice_after_input_does_not_redraw_the_prompt() {
    let mut presenter = TerminalPresenter::with_writer(Vec::new());
    presenter.prompt().expect("prompt should write");
    presenter.input_received();
    presenter.notice("note").expect("notice should write");
    let output = String::from_utf8(presenter.into_writer()).expect("output is utf-8");
    assert_eq!(output, "You> note\n");
}

fn output_after(steps: impl FnOnce(&mut TerminalPresenter<Vec<u8>>)) -> String {
    let mut presenter = TerminalPresenter::with_writer(Vec::new());
    steps(&mut presenter);
    String::from_utf8(presenter.into_writer()).expect("output is utf-8")
}

#[test]
fn a_state_is_one_quiet_line_and_a_repeat_prints_nothing() {
    let output = output_after(|presenter| {
        // The terminal starts idle, so the first Idle is not news.
        presenter.state(AssistantState::Idle).unwrap();
        presenter.state(AssistantState::Thinking).unwrap();
        presenter.state(AssistantState::Thinking).unwrap();
        presenter.state(AssistantState::Idle).unwrap();
    });
    assert_eq!(output, "[thinking]\n[idle]\n");
}

#[test]
fn every_state_has_its_own_line_and_listening_tells_how_to_stop() {
    let output = output_after(|presenter| {
        for state in [
            AssistantState::Listening,
            AssistantState::Transcribing,
            AssistantState::Thinking,
            AssistantState::Speaking,
            AssistantState::Idle,
        ] {
            presenter.state(state).unwrap();
        }
    });
    assert_eq!(
        output,
        "[listening] press Enter to stop, /stop to cancel (15 s max)\n\
         [transcribing]\n[thinking]\n[speaking]\n[idle]\n"
    );
}

#[test]
fn a_state_reached_mid_answer_waits_for_the_line_to_end() {
    let output = output_after(|presenter| {
        presenter.state(AssistantState::Thinking).unwrap();
        presenter.delta("Hello ").unwrap();
        presenter.state(AssistantState::Speaking).unwrap();
        presenter.delta("there").unwrap();
        presenter.completed("Hello there").unwrap();
        presenter.state(AssistantState::Idle).unwrap();
    });
    assert_eq!(
        output,
        "[thinking]\nJarvis: Hello there\n[speaking]\n[idle]\n"
    );
}

#[test]
fn a_state_while_the_prompt_is_open_redraws_the_prompt() {
    let output = output_after(|presenter| {
        presenter.state(AssistantState::Speaking).unwrap();
        presenter.prompt().unwrap();
        presenter.state(AssistantState::Idle).unwrap();
    });
    assert_eq!(output, "[speaking]\nYou> \n[idle]\nYou> ");
}
