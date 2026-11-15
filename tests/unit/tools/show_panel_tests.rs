use orb_protocol::{PanelName, ToOrb};
use rig_agent::{
    AgentBuilder,
    tool::{Tool, ToolContext},
};
use rig_core::{
    completion::Usage,
    serde_json::{from_value, json},
    test_utils::{MockCompletionModel, MockStreamEvent},
};
use tokio::sync::mpsc;

use super::{ShowPanelArgs, ShowPanelTool};
use crate::features::{
    assistant::Assistant,
    panel::{PANEL_TIMEOUT, channel},
};
use crate::interfaces::orb::OrbHandle;
use crate::tools::{ALLOWED_TOOLS, allowed_tool, register};

#[test]
fn a_panel_of_stats_is_accepted_and_nothing_else_is() {
    let accepted = [json!({"panel": "stats"})];
    let rejected = [
        json!({"panel": "weather"}),
        json!({"panel": "Stats"}),
        json!({"panel": ""}),
        json!({"panel": null}),
        json!({}),
        json!({"panel": "stats", "extra": 1}),
    ];
    for arguments in accepted {
        assert!(
            from_value::<ShowPanelArgs>(arguments.clone()).is_ok(),
            "{arguments}"
        );
    }
    for arguments in rejected {
        assert!(
            from_value::<ShowPanelArgs>(arguments.clone()).is_err(),
            "{arguments}"
        );
    }
}

#[test]
fn show_panel_is_on_the_allowed_list_with_the_same_argument_rules() {
    assert!(ALLOWED_TOOLS.iter().any(|rule| rule.name == "show_panel"));
    let rule = allowed_tool("show_panel").expect("show_panel is allowed");
    assert!(rule.accepts_arguments(&json!({"panel": "stats"})));
    assert!(!rule.accepts_arguments(&json!({"panel": "weather"})));
    assert!(!rule.accepts_arguments(&json!({"panel": "stats", "extra": 1})));
}

#[test]
fn the_schema_offers_only_stats() {
    let (panels, _inbox) = channel();
    let schema = ShowPanelTool::new(panels).parameters();
    assert_eq!(schema["properties"]["panel"]["enum"], json!(["stats"]));
    assert_eq!(schema["additionalProperties"], json!(false));
}

#[tokio::test(start_paused = true)]
async fn calling_the_tool_shows_the_panel_and_tells_the_model() {
    let (panels, inbox) = channel();
    let (to_orb, mut shown_to_orb) = mpsc::channel(16);
    let (_events, from_orb) = mpsc::channel(16);
    let task = tokio::spawn(async move {
        match inbox
            .run(Some(OrbHandle { to_orb, from_orb }), PANEL_TIMEOUT)
            .await {}
    });

    let output = ShowPanelTool::new(panels)
        .call(
            &mut ToolContext::new(),
            ShowPanelArgs {
                panel: PanelName::Stats,
            },
        )
        .await
        .expect("the panel task is running");

    assert_eq!(output, "The stats panel is now shown on the screen.");
    assert_eq!(
        shown_to_orb.recv().await,
        Some(ToOrb::ShowPanel {
            panel: PanelName::Stats
        })
    );
    task.abort();
}

fn call_turn(id: &str) -> Vec<MockStreamEvent> {
    vec![
        MockStreamEvent::tool_call(id, "show_panel", json!({"panel": "stats"})),
        MockStreamEvent::final_response(Usage::new()),
    ]
}

fn text_turn(text: &str) -> Vec<MockStreamEvent> {
    vec![
        MockStreamEvent::text(text),
        MockStreamEvent::final_response(Usage::new()),
    ]
}

/// A streamed run through the real registration and the real policy hook.
async fn run(turns: Vec<Vec<MockStreamEvent>>) -> (bool, Vec<ToOrb>) {
    let (panels, inbox) = channel();
    let (to_orb, mut shown_to_orb) = mpsc::channel(16);
    let (_events, from_orb) = mpsc::channel(16);
    let task = tokio::spawn(async move {
        match inbox
            .run(Some(OrbHandle { to_orb, from_orb }), PANEL_TIMEOUT)
            .await {}
    });
    let model = MockCompletionModel::from_stream_turns(turns);
    let assistant = Assistant::new(register(AgentBuilder::new(model), panels).build());

    let succeeded = assistant.respond("Bring up the stats.", &[]).await.is_ok();
    // Let the panel task pass on what it was asked.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    task.abort();
    let _ = task.await;
    let mut sent = Vec::new();
    while let Ok(message) = shown_to_orb.try_recv() {
        sent.push(message);
    }
    (succeeded, sent)
}

#[tokio::test(start_paused = true)]
async fn the_policy_accepts_show_panel_as_the_runs_one_tool_call() {
    let (succeeded, sent) = run(vec![call_turn("call-1"), text_turn("Here are the stats.")]).await;

    assert!(succeeded);
    assert_eq!(
        sent,
        [ToOrb::ShowPanel {
            panel: PanelName::Stats
        }]
    );
}

#[tokio::test(start_paused = true)]
async fn a_second_tool_call_is_still_rejected() {
    let (succeeded, sent) = run(vec![call_turn("call-1"), call_turn("call-2")]).await;

    assert!(!succeeded);
    // The first call ran; the second never did.
    assert_eq!(sent.len(), 1);
}
