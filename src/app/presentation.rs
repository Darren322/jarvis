use std::io::{self, Write};

pub(super) trait AssistantPresenter {
    fn delta(&mut self, text: &str) -> io::Result<()>;
    fn completed(&mut self, canonical_answer: &str) -> io::Result<()>;
    fn failed(&mut self, safe_message: &str) -> io::Result<()>;
    fn prompt(&mut self) -> io::Result<()>;
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

    fn input_received(&mut self) {}
}

pub(super) struct TerminalPresenter<W = io::Stdout> {
    output: W,
    response_line_open: bool,
    prompt_open: bool,
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
        }
    }

    #[cfg(test)]
    pub(super) fn into_writer(self) -> W {
        self.output
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
        self.output.flush()?;
        self.response_line_open = false;
        Ok(())
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
        self.output.flush()?;
        self.response_line_open = false;
        Ok(())
    }

    fn prompt(&mut self) -> io::Result<()> {
        if self.prompt_open {
            return Ok(());
        }
        if self.response_line_open {
            self.output.write_all(b"\n")?;
            self.response_line_open = false;
        }
        self.output.write_all(b"You> ")?;
        self.output.flush()?;
        self.prompt_open = true;
        Ok(())
    }

    fn input_received(&mut self) {
        self.prompt_open = false;
    }
}
