//! The panel commands: "bring up the stats", "show the camera" and "close it",
//! without the model.

use orb_protocol::PanelName;

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

/// Runs `Command::Show`: shows `panel` (again, if it is already up, which
/// restarts its timeout) and says what happened. No model is involved.
pub(crate) async fn show_panel(panels: &PanelHandle, panel: PanelName) -> Notice {
    match panels.show(panel).await {
        Ok(()) => Notice::Info(match panel {
            PanelName::Stats => "Showing the stats.".to_owned(),
            PanelName::Camera => "Showing the camera.".to_owned(),
            // Never reached: `show` refuses the built Panel by name, and no
            // phrase names it.
            PanelName::Built => "Showing the panel.".to_owned(),
        }),
        Err(error) => Notice::Warning(format!("could not show the panel: {error}")),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/control/panel_tests.rs"]
mod tests;
