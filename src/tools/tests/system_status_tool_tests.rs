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
}
