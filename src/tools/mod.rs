pub mod build_panel;
pub mod close_panel;
pub mod current_time;
pub mod show_panel;
pub mod system_status;
pub mod system_status_tool;
pub mod timers;

use rig_agent::{
    agent::{AgentBuilder, NoToolConfig, WithBuilderTools},
    tool::Tool,
};
use rig_core::serde_json::{self, Value};
use serde::de::DeserializeOwned;

use crate::features::{panel::PanelHandle, stats::machine::SystemMachine, timers::TimersHandle};
use build_panel::BuildPanelTool;
use close_panel::ClosePanelTool;
use current_time::CurrentTimeTool;
use show_panel::ShowPanelTool;
use system_status_tool::SystemStatusTool;
use timers::{cancel_timer::CancelTimerTool, list_timers::ListTimersTool, set_timer::SetTimerTool};

/// Whether a tool needs the user's approval before what it proposes takes effect
/// (timers D5). The approval is the interface's: a tool never asks, and the
/// model never grants it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolAccess {
    /// Reads, displays, or acts in a way the user asked for in the same breath.
    NoApproval,
    /// Only proposes: the user must say yes (or tap) before anything happens.
    NeedsApproval,
}

/// Policy metadata for one tool the model may call.
///
/// Registering a tool with Rig makes it executable; listing it in
/// [`ALLOWED_TOOLS`] makes Jarvis's policy offer and accept it. A registered
/// tool that is missing from this list is rejected, so the default is deny.
pub(crate) struct ToolRule {
    pub(crate) name: &'static str,
    accepts_arguments: fn(&Value) -> bool,
    access: ToolAccess,
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
            access: ToolAccess::NoApproval,
        }
    }

    /// Marks the tool as one that only proposes, until the user approves.
    pub(crate) const fn needing_approval(mut self) -> Self {
        self.access = ToolAccess::NeedsApproval;
        self
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "timers 06 asks it before a confirmation")
    )]
    pub(crate) fn access(&self) -> ToolAccess {
        self.access
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
    timers: TimersHandle,
) -> AgentBuilder<WithBuilderTools> {
    builder
        .tool(SystemStatusTool::new())
        .tool(ShowPanelTool::new(panels.clone()))
        .tool(ClosePanelTool::new(panels.clone()))
        .tool(CurrentTimeTool)
        .tool(BuildPanelTool::new(panels))
        .tool(SetTimerTool::new(timers.clone()))
        .tool(CancelTimerTool::new(timers.clone()))
        .tool(ListTimersTool::new(timers))
}

/// Every tool the policy offers to the model and accepts calls for.
pub(crate) const ALLOWED_TOOLS: &[ToolRule] = &[
    ToolRule::for_tool::<SystemStatusTool<SystemMachine>>(),
    ToolRule::for_tool::<ShowPanelTool>(),
    ToolRule::for_tool::<ClosePanelTool>(),
    ToolRule::for_tool::<CurrentTimeTool>(),
    ToolRule::for_tool::<BuildPanelTool>(),
    ToolRule::for_tool::<SetTimerTool>().needing_approval(),
    ToolRule::for_tool::<CancelTimerTool>(),
    ToolRule::for_tool::<ListTimersTool>(),
];

pub(crate) fn allowed_tool(name: &str) -> Option<&'static ToolRule> {
    ALLOWED_TOOLS.iter().find(|rule| rule.name == name)
}

#[cfg(test)]
#[path = "../../tests/unit/tools/tools_tests.rs"]
mod tests;
