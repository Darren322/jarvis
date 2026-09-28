use rig_core::serde_json;

use crate::tools::system_status;

#[derive(Debug)]
pub enum ToolError {
    UnknownTool,
    InvalidArguments,
    ExecutionFailed,
}

pub fn validate_tool(name: &str, arguments: &serde_json::Value) -> Result<(), ToolError> {
    if name != "system_status" {
        return Err(ToolError::UnknownTool);
    }

    if arguments != &serde_json::json!({}) {
        return Err(ToolError::InvalidArguments);
    }

    Ok(())
}

pub fn dispatch_tool(
    name: &str,
    arguments: &serde_json::Value,
) -> Result<serde_json::Value, ToolError> {
    validate_tool(name, arguments)?;

    match name {
        "system_status" => system_status::execute().map_err(|_| ToolError::ExecutionFailed),
        _ => unreachable!(),
    }
}
