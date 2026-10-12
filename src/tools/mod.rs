pub mod system_status;
pub mod system_status_tool;

use rig_agent::tool::Tool;
use rig_core::serde_json::{self, Value};
use serde::de::DeserializeOwned;

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

/// Every tool the model may call. Keep in sync with the tools registered on
/// the Rig agent in `App`.
pub(crate) const ALLOWED_TOOLS: &[ToolRule] = &[ToolRule::for_tool::<SystemStatusTool>()];

pub(crate) fn allowed_tool(name: &str) -> Option<&'static ToolRule> {
    ALLOWED_TOOLS.iter().find(|rule| rule.name == name)
}

#[cfg(test)]
#[path = "../../tests/unit/tools/mod.rs"]
mod tests;
