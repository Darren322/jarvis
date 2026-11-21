use orb_protocol::{
    BuiltPanel, BuiltPanelError, GaugeSource, MAX_LIST_ITEMS, MAX_PANEL_PARTS,
    MAX_PANEL_TITLE_CHARS, MAX_PART_TEXT_CHARS, PanelPart,
};
use rig_agent::tool::{Tool, ToolContext, ToolExecutionError};
use rig_core::serde_json::{Value, json};
use thiserror::Error;

use crate::features::panel::{PanelError, PanelHandle};

/// The heading over a note, which the model does not write.
pub(crate) const NOTE_TITLE: &str = "Note";
/// The heading over a list, which the model does not write.
pub(crate) const LIST_TITLE: &str = "Items";

/// A Panel as the model writes it: flat fields, each one Catalog part. Gemma's
/// chat template cannot show the model a schema of nested `anyOf` parts (on
/// the Pi it wrote the call as prose instead of making it), but it shows plain
/// strings, string arrays and enums in its own format, as it does for
/// `current_time`. Only the shape is checked here; the limits are checked when
/// the tool runs, so the model hears which one it broke (jarvis-ui 12, G7).
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildPanelArgs {
    pub title: String,
    /// A note: one card.
    #[serde(default)]
    pub text: Option<String>,
    /// The lines of one list; none means no list.
    #[serde(default)]
    pub items: Vec<String>,
    /// One live gauge each.
    #[serde(default)]
    pub gauges: Vec<GaugeSource>,
}

impl BuildPanelArgs {
    /// The parts in the order they are laid out: the list, the note, then the
    /// gauges. Empty fields add nothing; whether anything is left is checked
    /// by [`BuiltPanel::new`].
    pub(crate) fn into_parts(self) -> (String, Vec<PanelPart>) {
        let mut parts = Vec::new();
        if !self.items.is_empty() {
            parts.push(PanelPart::List {
                title: LIST_TITLE.to_owned(),
                items: self.items,
            });
        }
        if let Some(text) = self.text.filter(|text| !text.trim().is_empty()) {
            parts.push(PanelPart::Card {
                title: NOTE_TITLE.to_owned(),
                text,
            });
        }
        parts.extend(
            self.gauges
                .into_iter()
                .map(|source| PanelPart::Gauge { source }),
        );
        (self.title, parts)
    }
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

/// Builds a Panel from the Catalog (a list, a note and live gauges) and shows
/// it. It only displays, so it needs no approval step (jarvis-ui 12, G1).
pub(crate) struct BuildPanelTool {
    panels: PanelHandle,
}

impl BuildPanelTool {
    pub(crate) fn new(panels: PanelHandle) -> Self {
        Self { panels }
    }
}

impl Tool for BuildPanelTool {
    const NAME: &'static str = "build_panel";

    type Args = BuildPanelArgs;
    type Output = String;
    type Error = BuildPanelError;

    fn description(&self) -> String {
        "Shows a panel on the screen with what the user asks to see: a list (items), a short \
         note (text) and live gauges (gauges). Examples: \"show my facts as a list\" gives items \
         with one fact each; \"make me a shopping list: milk and eggs\" gives items milk and \
         eggs; \"show me the CPU and temperature as gauges\" gives gauges cpu and temperature. \
         Use only what the user said or the facts you were given, and never invent items. \
         Jarvis fills in the gauges' values, so never write or say them. For the stats \
         dashboard or the camera use show_panel. Never say a panel is shown unless this tool \
         has just succeeded."
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "maxLength": MAX_PANEL_TITLE_CHARS,
                    "description": "A short heading for the panel."
                },
                "items": {
                    "type": "array",
                    "maxItems": MAX_LIST_ITEMS,
                    "items": { "type": "string", "maxLength": MAX_PART_TEXT_CHARS },
                    "description": "The lines of a list to show, if any."
                },
                "text": {
                    "type": "string",
                    "maxLength": MAX_PART_TEXT_CHARS,
                    "description": "A short note to show, if any."
                },
                "gauges": {
                    "type": "array",
                    "maxItems": MAX_PANEL_PARTS,
                    "items": {
                        "type": "string",
                        "enum": ["cpu", "temperature", "ram", "storage", "llama", "bge"]
                    },
                    "description": "Live figures to show, if any."
                }
            },
            "required": ["title"],
            "additionalProperties": false
        })
    }

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
        let (title, parts) = args.into_parts();
        let built = BuiltPanel::new(title, parts)?;
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
