use std::io::Write;

use super::TerminalPresenter;

impl<W: Write> TerminalPresenter<W> {
    pub(in crate::interfaces::cli) fn into_writer(self) -> W {
        self.output
    }
}
