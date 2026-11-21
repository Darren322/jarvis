use orb_protocol::{
    BuiltPanel, BuiltPanelError, MAX_LIST_ITEMS, MAX_PANEL_PARTS, MAX_PANEL_TITLE_CHARS,
    MAX_PART_TEXT_CHARS, MIN_LIST_ITEMS, MIN_PANEL_PARTS, PanelPart,
};
use rig_agent::tool::{Tool, ToolContext, ToolExecutionError};
use rig_core::serde_json::{Value, json};
use thiserror::Error;

use crate::features::panel::{PanelError, PanelHandle};

/// A Panel as the model writes it: a title and parts from the Catalog. Only
/// the shape is checked here (a part not in the Catalog, an unknown gauge
/// source or an unknown field fails); the limits are checked when the tool
/// runs, so the model hears which one it broke (jarvis-ui 12, G7).
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildPanelArgs {
    pub title: String,
    pub parts: Vec<PanelPart>,
}

/// Why no Panel was shown.
#[derive(Debug, Error)]
pub(crate) enum BuildPanelError {
    /// A limit was broken; the message names it and is what the model is told.
    #[error(transparent)]
    Rejected(#[from] BuiltPanelError),
    #[error(transparent)]
    Panel(#[from] PanelError),
}

/// Builds a Panel from the Catalog (cards, lists and live gauges) and shows
/// it. It only displays, so it needs no approval step (jarvis-ui 12, G1).
pub(crate) struct BuildPanelTool {
    panels: PanelHandle,
}

impl BuildPanelTool {
    pub(crate) fn new(panels: PanelHandle) -> Self {
        Self { panels }
    }
}

/// A JSON Schema string of at most `chars` characters.
fn text(chars: usize) -> Value {
    json!({ "type": "string", "maxLength": chars })
}

impl Tool for BuildPanelTool {
    const NAME: &'static str = "build_panel";

    type Args = BuildPanelArgs;
    type Output = String;
    type Error = BuildPanelError;

    fn description(&self) -> String {
        "Builds a panel from parts and shows it on the screen. Use it when the user asks to see \
         something as a list, a card or gauges, for example \"show my facts as a list\", \"make \
         me a shopping list\" or \"show me the CPU and temperature as gauges\". A `card` has a \
         title and a short text. A `list` has a title and its items. A `gauge` shows one live \
         figure: `cpu`, `temperature`, `ram`, `storage`, or whether the `llama` or `bge` \
         server is up; Jarvis fills in its value and keeps it current, so never write a number \
         for it and never say what it shows. Write cards and lists only from what you already \
         know, such as the facts you were given. The panel only shows: nothing on it is saved. \
         It closes by itself after a short while. For the stats dashboard or the camera use \
         `show_panel` instead. If this tool fails, tell the user what was wrong; never say a \
         panel is shown unless this tool has just succeeded."
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": text(MAX_PANEL_TITLE_CHARS),
                "parts": {
                    "type": "array",
                    "minItems": MIN_PANEL_PARTS,
                    "maxItems": MAX_PANEL_PARTS,
                    "items": {
                        "anyOf": [
                            {
                                "type": "object",
                                "properties": {
                                    "kind": { "const": "card" },
                                    "title": text(MAX_PART_TEXT_CHARS),
                                    "text": text(MAX_PART_TEXT_CHARS)
                                },
                                "required": ["kind", "title", "text"],
                                "additionalProperties": false
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "kind": { "const": "list" },
                                    "title": text(MAX_PART_TEXT_CHARS),
                                    "items": {
                                        "type": "array",
                                        "minItems": MIN_LIST_ITEMS,
                                        "maxItems": MAX_LIST_ITEMS,
                                        "items": text(MAX_PART_TEXT_CHARS)
                                    }
                                },
                                "required": ["kind", "title", "items"],
                                "additionalProperties": false
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "kind": { "const": "gauge" },
                                    "source": {
                                        "type": "string",
                                        "enum": ["cpu", "temperature", "ram", "storage", "llama", "bge"]
                                    }
                                },
                                "required": ["kind", "source"],
                                "additionalProperties": false
                            }
                        ]
                    }
                }
            },
            "required": ["title", "parts"],
            "additionalProperties": false
        })
    }

    /// A broken limit keeps its message for the model, so Gemma can say what
    /// was wrong; anything else gets Rig's safe default.
    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        match error {
            BuildPanelError::Rejected(rejected) => {
                ToolExecutionError::invalid_args(rejected.to_string()).with_source(rejected)
            }
            BuildPanelError::Panel(_) => ToolExecutionError::from_error(error),
        }
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        // Every limit is checked before anything is shown.
        let built = BuiltPanel::new(args.title, args.parts)?;
        self.panels.show_built(built).await?;
        Ok(
            "The panel is now shown on the screen. It will close by itself. You cannot see \
            the values of its gauges."
                .to_owned(),
        )
    }
}

#[cfg(test)]
#[path = "../../tests/unit/tools/build_panel_tests.rs"]
mod tests;
