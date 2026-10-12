use std::collections::BTreeSet;

use futures_util::StreamExt;
use rig_agent::AgentBuilder;
use rig_core::{
    completion::Usage,
    test_utils::{MockCompletionModel, MockStreamEvent},
};

use super::{ALLOWED_TOOLS, register};

/// Runs one turn with no Jarvis policy attached, so the request lists every
/// tool registered with Rig rather than only the allowed ones.
#[tokio::test]
async fn registered_tools_match_the_allowed_list() {
    let model = MockCompletionModel::from_stream_turns([[
        MockStreamEvent::text("ok"),
        MockStreamEvent::final_response(Usage::new()),
    ]]);
    let handle = model.clone();
    let agent = register(AgentBuilder::new(model)).build();

    let mut stream = agent.runner("hello").stream().await;
    while let Some(item) = stream.next().await {
        item.expect("the scripted turn completes");
    }

    let registered: BTreeSet<_> = handle.requests()[0]
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    let allowed: BTreeSet<_> = ALLOWED_TOOLS
        .iter()
        .map(|rule| rule.name.to_owned())
        .collect();
    assert_eq!(
        registered, allowed,
        "every tool must be both registered in `register` and listed in `ALLOWED_TOOLS`"
    );
    assert_eq!(allowed.len(), ALLOWED_TOOLS.len(), "tool names are unique");
}
