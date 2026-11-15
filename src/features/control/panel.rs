//! The "close it" command: hide the panel on the orb screen.

use super::Notice;
use crate::features::panel::PanelHandle;

/// Runs `Command::Close`: closes the shown panel, if any, and says what happened.
/// No model is involved.
pub(crate) async fn close_panel(panels: &PanelHandle) -> Notice {
    if panels.shown().is_none() {
        return Notice::Info("No panel is open.".to_owned());
    }
    match panels.close().await {
        Ok(()) => Notice::Info("Panel closed.".to_owned()),
        Err(error) => Notice::Warning(format!("could not close the panel: {error}")),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/control/panel_tests.rs"]
mod tests;
