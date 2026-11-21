use orb_protocol::PanelName;
use rig_agent::tool::{Tool, ToolContext};
use rig_core::serde_json::json;

use crate::features::panel::{PanelError, PanelHandle};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowPanelArgs {
    /// Which panel to bring up: `stats` or `camera`.
    pub panel: PanelName,
}

/// Brings a panel up on the orb screen. It only displays, so it needs no
/// approval step (jarvis-ui D8).
pub(crate) struct ShowPanelTool {
    panels: PanelHandle,
}

impl ShowPanelTool {
    pub(crate) fn new(panels: PanelHandle) -> Self {
        Self { panels }
    }
}

impl Tool for ShowPanelTool {
    const NAME: &'static str = "show_panel";

    type Args = ShowPanelArgs;
    type Output = String;
    type Error = PanelError;

    fn description(&self) -> String {
        "Shows a panel on the screen. Use it when the user asks to see something, for example \
         \"bring up the stats\", \"show me the system stats\" or \"show the camera\". The panel \
         `stats` shows the Pi's CPU, temperature, memory and whether the language and embedding \
         servers are up; it closes by itself after a short while. The panel `camera` shows the \
         camera's live picture to the user and stays until the user closes it; you cannot see \
         that picture, so never describe what the camera shows. The user can close a panel \
         without telling you, so call this tool every time the user asks for a panel, even if \
         you showed it earlier. Never say a panel is shown unless this tool has just succeeded."
            .to_string()
    }

    fn parameters(&self) -> rig_core::serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "panel": {
                    "type": "string",
                    "enum": ["stats", "camera"],
                    "description": "The panel to show."
                }
            },
            "required": ["panel"],
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        self.panels.show(args.panel).await?;
        Ok(match args.panel {
            PanelName::Stats => {
                "The stats panel is now shown on the screen. It will close by itself.".to_owned()
            }
            PanelName::Camera => "The camera panel is now shown on the screen. It stays until \
                                  the user closes it. You cannot see its picture."
                .to_owned(),
        })
    }
}

#[cfg(test)]
#[path = "../../tests/unit/tools/show_panel_tests.rs"]
mod tests;
