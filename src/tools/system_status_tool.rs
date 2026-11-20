use std::convert::Infallible;

use rig_agent::tool::{Tool, ToolContext};
use rig_core::serde_json::json;
use tokio::sync::Mutex;

use crate::features::stats::machine::{Machine, SystemMachine};
use crate::tools::system_status::{SystemStatus, collect_system_status};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemStatusArgs {}

/// Tells the model what the Dashboard shows, read from the same [`Machine`].
/// Calls queue on the machine: two readings at once would disturb each other's CPU figure.
pub(crate) struct SystemStatusTool<M = SystemMachine> {
    machine: Mutex<M>,
}

impl SystemStatusTool {
    pub(crate) fn new() -> Self {
        Self::with_machine(SystemMachine::new())
    }
}

impl<M: Machine> SystemStatusTool<M> {
    pub(crate) fn with_machine(machine: M) -> Self {
        Self {
            machine: Mutex::new(machine),
        }
    }
}

impl<M: Machine> Tool for SystemStatusTool<M> {
    const NAME: &'static str = "system_status";

    type Args = SystemStatusArgs;
    type Output = SystemStatus;
    type Error = Infallible;

    fn description(&self) -> String {
        "Returns this Pi's current status: storage (disk space) used and total in GB, CPU load \
         in percent, temperature in degrees C, memory used and total in GB, uptime and hostname. \
         Use it for any question about storage, disk space, CPU, temperature, memory, RAM or \
         uptime. A figure the Pi could not read is \"unavailable\"; say so and never guess it."
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
        Ok(collect_system_status(&mut *self.machine.lock().await).await)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/tools/system_status_tool_tests.rs"]
mod tests;
