//! Interface-neutral commands. Any input (the terminal now, push-to-talk
//! later) is parsed here into a `Command`; memory commands run here and
//! return `Notice`s for the interface to print or speak.
//!
//! Adding a command: add a variant and `parse` arm plus one `COMMANDS` row in
//! `command.rs`, and handle it in `memory.rs` (or a sibling file). Interfaces
//! only need to change when the command controls the interface itself, such
//! as `/stop` or `/exit`.

mod command;
mod fact_match;
mod memory;
mod panel;
mod tap;
mod timer;

pub(crate) use command::{Command, MemoryCommand, help_text, parse};
pub(crate) use memory::MemoryControl;
pub(crate) use panel::{close_panel, show_panel};
pub(crate) use tap::{Tap, TapAction, tap_action};
pub(crate) use timer::{AnswerOutcome, answer_timer, event_notice, proposal_line};

/// One user-facing line produced by a command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Notice {
    Info(String),
    Warning(String),
}

impl Notice {
    /// The line itself, for an interface that shows or speaks it its own way.
    pub(crate) fn text(&self) -> &str {
        match self {
            Self::Info(text) | Self::Warning(text) => text,
        }
    }
}
