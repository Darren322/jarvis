use rig_agent::tool::Tool;

use crate::tools::system_status_tool::{SystemStatusArgs, SystemStatusTool};

#[test]
fn system_status_tool_has_expected_contract() {
    let tool = SystemStatusTool;

    assert_eq!(SystemStatusTool::NAME, "system_status");

    assert_eq!(
        tool.parameters(),
        rig_core::serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    );
}

#[tokio::test]
async fn system_status_tool_returns_status() {
    let tool = SystemStatusTool;
    let mut context = rig_agent::tool::ToolContext::new();

    let result = tool.call(&mut context, SystemStatusArgs {}).await.unwrap();

    assert!(result.total_memory_bytes > 0);
    assert!(result.used_memory_bytes <= result.total_memory_bytes);

    let json =
        rig_core::serde_json::to_value(&result).expect("system status should serialize to JSON");

    assert!(json.get("source_node").is_some());
    assert!(json.get("uptime_seconds").is_some());
    assert!(json.get("total_memory_bytes").is_some());
    assert!(json.get("used_memory_bytes").is_some());
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
