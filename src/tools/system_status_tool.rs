use std::convert::Infallible;

use rig_agent::tool::{Tool, ToolContext};
use rig_core::serde_json::json;

use crate::tools::system_status::{SystemStatus, collect_system_status};

#[derive(Debug, serde::Deserialize)]
pub struct SystemStatusArgs {}

#[derive(Debug, Default)]
pub struct SystemStatusTool;

impl Tool for SystemStatusTool {
    const NAME: &'static str = "system_status";

    type Args = SystemStatusArgs;
    type Output = SystemStatus;
    type Error = Infallible;

    fn description(&self) -> String {
        "Returns the current system status, including hostname, uptime, and memory usage."
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
        Ok(collect_system_status())
    }
}
