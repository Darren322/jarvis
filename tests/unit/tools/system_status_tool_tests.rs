use orb_protocol::StorageUsage;
use rig_agent::tool::{Tool, ToolContext};

use crate::features::stats::machine::{Machine, MachineError, MachineReading};
use crate::tools::system_status::Figure;
use crate::tools::system_status_tool::{SystemStatusArgs, SystemStatusTool};

struct Scripted(MachineReading);

impl Machine for Scripted {
    async fn read(&mut self) -> Result<MachineReading, MachineError> {
        Ok(self.0)
    }
}

fn scripted_tool() -> SystemStatusTool<Scripted> {
    SystemStatusTool::with_machine(Scripted(MachineReading {
        cpu_percent: 7.0,
        temperature_c: None,
        ram_used_bytes: 1_073_741_824,
        ram_total_bytes: 2_147_483_648,
        storage: Some(StorageUsage {
            available_bytes: 4_000_000_000,
            total_bytes: 10_000_000_000,
        }),
    }))
}

#[test]
fn system_status_tool_has_expected_contract() {
    let tool = scripted_tool();

    assert_eq!(SystemStatusTool::<Scripted>::NAME, "system_status");

    assert_eq!(
        tool.parameters(),
        rig_core::serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    );
}

#[test]
fn the_description_names_every_figure_a_question_may_ask_about() {
    let description = scripted_tool().description();

    for word in [
        "storage",
        "disk space",
        "CPU",
        "temperature",
        "memory",
        "uptime",
    ] {
        assert!(description.contains(word), "{word}");
    }
}

#[tokio::test(start_paused = true)]
async fn system_status_tool_returns_the_machines_figures() {
    let tool = scripted_tool();
    let mut context = ToolContext::new();

    let status = tool.call(&mut context, SystemStatusArgs {}).await.unwrap();

    assert_eq!(status.cpu_percent, Figure::Known(7));
    assert_eq!(status.temperature_c, Figure::Unavailable);
    assert_eq!(status.memory_used_gb, Figure::Known(1.0));
    assert_eq!(status.memory_total_gb, Figure::Known(2.0));
    assert_eq!(status.storage_used_gb, Figure::Known(6));
    assert_eq!(status.storage_total_gb, Figure::Known(10));
}

#[test]
fn system_status_args_accepts_empty_object() {
    let result =
        rig_core::serde_json::from_value::<SystemStatusArgs>(rig_core::serde_json::json!({}));

    assert!(result.is_ok());
}

#[test]
fn system_status_args_rejects_unknown_fields() {
    let result =
        rig_core::serde_json::from_value::<SystemStatusArgs>(rig_core::serde_json::json!({
            "unexpected": true
        }));

    assert!(result.is_err());
}
