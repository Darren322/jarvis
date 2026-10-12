//! Interface-neutral commands. Any input (the terminal now, push-to-talk
//! later) is parsed here into a `Command`; memory commands run here and
//! return `Notice`s for the interface to print or speak.
//!
//! Adding a command: add a variant and `parse` arm plus one `COMMANDS` row in
//! `command.rs`, and handle it in `memory.rs` (or a sibling file). Interfaces
//! only need to change when the command controls the interface itself, such
//! as `/stop` or `/exit`.

mod command;
mod memory;

pub(crate) use command::{Command, MemoryCommand, help_text, parse};
pub(crate) use memory::MemoryControl;

/// One user-facing line produced by a command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Notice {
    Info(String),
    Warning(String),
}
