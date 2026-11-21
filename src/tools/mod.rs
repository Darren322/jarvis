pub mod build_panel;
pub mod close_panel;
pub mod current_time;
pub mod show_panel;
pub mod system_status;
pub mod system_status_tool;

use rig_agent::{
    agent::{AgentBuilder, NoToolConfig, WithBuilderTools},
    tool::Tool,
};
use rig_core::serde_json::{self, Value};
use serde::de::DeserializeOwned;

use crate::features::{panel::PanelHandle, stats::machine::SystemMachine};
use build_panel::BuildPanelTool;
use close_panel::ClosePanelTool;
use current_time::CurrentTimeTool;
use show_panel::ShowPanelTool;
use system_status_tool::SystemStatusTool;

/// Policy metadata for one tool the model may call.
///
/// Registering a tool with Rig makes it executable; listing it in
/// [`ALLOWED_TOOLS`] makes Jarvis's policy offer and accept it. A registered
/// tool that is missing from this list is rejected, so the default is deny.
pub(crate) struct ToolRule {
    pub(crate) name: &'static str,
    accepts_arguments: fn(&Value) -> bool,
}

impl ToolRule {
    /// Accepts exactly the JSON objects that deserialize into the tool's own
    /// typed `Args`, so each tool owns its argument contract.
    pub(crate) const fn for_tool<T>() -> Self
    where
        T: Tool,
        T::Args: DeserializeOwned,
    {
        Self {
            name: T::NAME,
            accepts_arguments: accepts_arguments::<T>,
        }
    }

    pub(crate) fn accepts_arguments(&self, arguments: &Value) -> bool {
        (self.accepts_arguments)(arguments)
    }
}

fn accepts_arguments<T>(arguments: &Value) -> bool
where
    T: Tool,
    T::Args: DeserializeOwned,
{
    arguments.is_object() && serde_json::from_value::<T::Args>(arguments.clone()).is_ok()
}

// Adding a tool: write it in its own file, then add it to BOTH `register` and
// `ALLOWED_TOOLS` below. `registered_tools_match_the_allowed_list` fails if
// the two lists drift apart.

/// Registers every Jarvis tool with Rig, making it executable.
pub(crate) fn register(
    builder: AgentBuilder<NoToolConfig>,
    panels: PanelHandle,
) -> AgentBuilder<WithBuilderTools> {
    builder
        .tool(SystemStatusTool::new())
        .tool(ShowPanelTool::new(panels.clone()))
        .tool(ClosePanelTool::new(panels.clone()))
        .tool(CurrentTimeTool)
        .tool(BuildPanelTool::new(panels))
}

/// Every tool the policy offers to the model and accepts calls for.
pub(crate) const ALLOWED_TOOLS: &[ToolRule] = &[
    ToolRule::for_tool::<SystemStatusTool<SystemMachine>>(),
    ToolRule::for_tool::<ShowPanelTool>(),
    ToolRule::for_tool::<ClosePanelTool>(),
    ToolRule::for_tool::<CurrentTimeTool>(),
    ToolRule::for_tool::<BuildPanelTool>(),
];

pub(crate) fn allowed_tool(name: &str) -> Option<&'static ToolRule> {
    ALLOWED_TOOLS.iter().find(|rule| rule.name == name)
}

#[cfg(test)]
#[path = "../../tests/unit/tools/tools_tests.rs"]
mod tests;
