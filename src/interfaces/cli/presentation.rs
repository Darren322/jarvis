use std::io::{self, Write};

use crate::features::{listening::MAX_RECORDING_SECONDS, state::AssistantState};

const PROMPT: &str = "You> ";

pub(crate) trait AssistantPresenter {
    fn delta(&mut self, text: &str) -> io::Result<()>;
    fn completed(&mut self, canonical_answer: &str) -> io::Result<()>;
    fn failed(&mut self, safe_message: &str) -> io::Result<()>;
    fn prompt(&mut self) -> io::Result<()>;
    /// Shows one out-of-band line (a diagnostic) on its own line. If the prompt
    /// is open it is redrawn afterwards, so the screen still ends on a prompt.
    fn notice(&mut self, line: &str) -> io::Result<()>;
    /// Reports that the assistant moved to `state`. Called once per transition.
    fn state(&mut self, state: AssistantState) -> io::Result<()>;
    fn input_received(&mut self);
}

// Test and alternate front ends that only accept a final callback retain the
// old completed-answer seam. The production terminal presenter below handles
// incremental text as it arrives.
impl<F> AssistantPresenter for F
where
    F: FnMut(&str),
{
    fn delta(&mut self, _text: &str) -> io::Result<()> {
        Ok(())
    }

    fn completed(&mut self, canonical_answer: &str) -> io::Result<()> {
        self(canonical_answer);
        Ok(())
    }

    fn failed(&mut self, safe_message: &str) -> io::Result<()> {
        self(safe_message);
        Ok(())
    }

    fn prompt(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn notice(&mut self, _line: &str) -> io::Result<()> {
        Ok(())
    }

    fn state(&mut self, _state: AssistantState) -> io::Result<()> {
        Ok(())
    }

    fn input_received(&mut self) {}
}

/// The quiet line the terminal shows when the assistant enters `state`.
fn state_line(state: AssistantState) -> String {
    match state {
        AssistantState::Idle => "[idle]".to_owned(),
        AssistantState::Listening => format!(
            "[listening] press Enter to stop, /stop to cancel ({MAX_RECORDING_SECONDS} s max)"
        ),
        AssistantState::Transcribing => "[transcribing]".to_owned(),
        AssistantState::Thinking => "[thinking]".to_owned(),
        AssistantState::Speaking => "[speaking]".to_owned(),
    }
}

pub(crate) struct TerminalPresenter<W = io::Stdout> {
    output: W,
    response_line_open: bool,
    prompt_open: bool,
    /// The last state reported; a repeat prints nothing. The terminal starts idle.
    state: AssistantState,
    /// States reached while an answer was streaming. Printing them there would split
    /// the text, so they wait for the line to end.
    deferred_states: Vec<AssistantState>,
}

impl Default for TerminalPresenter<io::Stdout> {
    fn default() -> Self {
        Self::with_writer(io::stdout())
    }
}

impl<W: Write> TerminalPresenter<W> {
    pub(super) fn with_writer(output: W) -> Self {
        Self {
            output,
            response_line_open: false,
            prompt_open: false,
            state: AssistantState::Idle,
            deferred_states: Vec::new(),
        }
    }

    /// Prints states that waited for the answer line to end. The line must be closed.
    fn write_deferred_states(&mut self) -> io::Result<()> {
        for state in self.deferred_states.drain(..) {
            writeln!(self.output, "{}", state_line(state))?;
        }
        Ok(())
    }
}

impl<W: Write> AssistantPresenter for TerminalPresenter<W> {
    fn delta(&mut self, text: &str) -> io::Result<()> {
        if self.prompt_open {
            self.output.write_all(b"\n")?;
            self.prompt_open = false;
        }
        if !self.response_line_open {
            self.output.write_all(b"Jarvis: ")?;
            self.response_line_open = true;
        }
        self.output.write_all(text.as_bytes())?;
        self.output.flush()
    }

    fn completed(&mut self, canonical_answer: &str) -> io::Result<()> {
        if self.prompt_open {
            self.output.write_all(b"\n")?;
            self.prompt_open = false;
        }
        if !self.response_line_open {
            self.output.write_all(b"Jarvis: ")?;
            self.output.write_all(canonical_answer.as_bytes())?;
        }
        self.output.write_all(b"\n")?;
        self.response_line_open = false;
        self.write_deferred_states()?;
        self.output.flush()
    }

    fn failed(&mut self, safe_message: &str) -> io::Result<()> {
        if self.prompt_open {
            self.output.write_all(b"\n")?;
            self.prompt_open = false;
        }
        if self.response_line_open {
            self.output.write_all(b" [incomplete response]")?;
        } else {
            self.output.write_all(b"Jarvis: ")?;
            self.output.write_all(safe_message.as_bytes())?;
        }
        self.output.write_all(b"\n")?;
        self.response_line_open = false;
        self.write_deferred_states()?;
        self.output.flush()
    }

    fn prompt(&mut self) -> io::Result<()> {
        if self.prompt_open {
            return Ok(());
        }
        if self.response_line_open {
            self.output.write_all(b"\n")?;
            self.response_line_open = false;
        }
        self.write_deferred_states()?;
        self.output.write_all(PROMPT.as_bytes())?;
        self.output.flush()?;
        self.prompt_open = true;
        Ok(())
    }

    fn notice(&mut self, line: &str) -> io::Result<()> {
        if self.prompt_open || self.response_line_open {
            self.output.write_all(b"\n")?;
            self.response_line_open = false;
        }
        self.write_deferred_states()?;
        self.output.write_all(line.as_bytes())?;
        self.output.write_all(b"\n")?;
        if self.prompt_open {
            // The terminal owns any half-typed input; only the prompt is redrawn.
            self.output.write_all(PROMPT.as_bytes())?;
        }
        self.output.flush()
    }

    fn state(&mut self, state: AssistantState) -> io::Result<()> {
        if state == self.state {
            return Ok(());
        }
        self.state = state;
        if self.response_line_open {
            self.deferred_states.push(state);
            return Ok(());
        }
        self.notice(&state_line(state))
    }

    fn input_received(&mut self) {
        self.prompt_open = false;
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/interfaces/cli/presentation_tests.rs"]
mod tests;
