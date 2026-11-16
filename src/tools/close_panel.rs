use rig_agent::tool::{Tool, ToolContext};
use rig_core::serde_json::json;

use crate::features::panel::{PanelError, PanelHandle};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClosePanelArgs {}

/// Closes the panel on the orb screen, for every wording the fixed phrases
/// ("close it", "hide that") do not catch. Like `show_panel` it only changes
/// what is displayed, so it needs no approval step (jarvis-ui D8).
pub(crate) struct ClosePanelTool {
    panels: PanelHandle,
}

impl ClosePanelTool {
    pub(crate) fn new(panels: PanelHandle) -> Self {
        Self { panels }
    }
}

impl Tool for ClosePanelTool {
    const NAME: &'static str = "close_panel";

    type Args = ClosePanelArgs;
    type Output = String;
    type Error = PanelError;

    fn description(&self) -> String {
        "Closes the panel that is shown on the screen. Use it when the user asks to close, \
         hide or dismiss the panel, the stats or the dashboard, for example \"you may close \
         the panel\" or \"take the stats down\"."
            .to_string()
    }

    fn parameters(&self) -> rig_core::serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        _args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        if self.panels.shown().is_none() {
            return Ok("No panel was open on the screen.".to_owned());
        }
        self.panels.close().await?;
        Ok("The panel is now closed.".to_owned())
    }
}

#[cfg(test)]
#[path = "../../tests/unit/tools/close_panel_tests.rs"]
mod tests;
