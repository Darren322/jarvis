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

use super::{ClosePanelArgs, ClosePanelTool};
use crate::features::{
    assistant::Assistant,
    panel::{PANEL_TIMEOUT, PanelHandle, channel},
};
use crate::interfaces::orb::OrbHandle;
use crate::tools::{ALLOWED_TOOLS, allowed_tool, register};

#[test]
fn no_arguments_are_accepted_and_any_argument_is_rejected() {
    assert!(from_value::<ClosePanelArgs>(json!({})).is_ok());
    for arguments in [json!({"panel": "stats"}), json!({"extra": 1}), json!(null)] {
        assert!(
            from_value::<ClosePanelArgs>(arguments.clone()).is_err(),
            "{arguments}"
        );
    }
}

#[test]
fn close_panel_is_on_the_allowed_list_with_the_same_argument_rules() {
    assert!(ALLOWED_TOOLS.iter().any(|rule| rule.name == "close_panel"));
    let rule = allowed_tool("close_panel").expect("close_panel is allowed");
    assert!(rule.accepts_arguments(&json!({})));
    assert!(!rule.accepts_arguments(&json!({"panel": "stats"})));
}

#[test]
fn the_schema_takes_no_arguments() {
    let (panels, _inbox) = channel();
    let schema = ClosePanelTool::new(panels).parameters();
    assert_eq!(schema["properties"], json!({}));
    assert_eq!(schema["additionalProperties"], json!(false));
}

/// Runs `body` beside a panel service whose orb is a pair of channels, then
/// returns what the body gave and everything the orb was sent.
async fn beside_a_panel_service<T>(body: impl AsyncFnOnce(PanelHandle) -> T) -> (T, Vec<ToOrb>) {
    let (panels, inbox) = channel();
    let (to_orb, mut sent_to_orb) = mpsc::channel(16);
    let (_events, from_orb) = mpsc::channel(16);
    let task = tokio::spawn(async move {
        match inbox
            .run(Some(OrbHandle { to_orb, from_orb }), PANEL_TIMEOUT)
            .await {}
    });
    let result = body(panels).await;
    // Let the panel task pass on what it was asked.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    task.abort();
    let _ = task.await;
    let mut sent = Vec::new();
    while let Ok(message) = sent_to_orb.try_recv() {
        sent.push(message);
    }
    (result, sent)
}

/// Shows the stats panel and waits until the panel state has it.
async fn show_stats(panels: &PanelHandle) {
    panels
        .show(PanelName::Stats)
        .await
        .expect("the panel task is running");
    let mut shown = panels.watch_shown();
    shown
        .wait_for(|panel| panel.is_some())
        .await
        .expect("the panel task is running");
}

#[tokio::test(start_paused = true)]
async fn calling_the_tool_closes_the_shown_panel_and_tells_the_model() {
    let (output, sent) = beside_a_panel_service(async |panels| {
        show_stats(&panels).await;
        ClosePanelTool::new(panels)
            .call(&mut ToolContext::new(), ClosePanelArgs {})
            .await
            .expect("the panel task is running")
    })
    .await;

    assert_eq!(output, "The panel is now closed.");
    assert_eq!(
        sent,
        [
            ToOrb::ShowPanel {
                panel: PanelName::Stats
            },
            ToOrb::HidePanel {}
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn calling_the_tool_with_no_panel_shown_sends_nothing_and_says_so() {
    let (output, sent) = beside_a_panel_service(async |panels| {
        ClosePanelTool::new(panels)
            .call(&mut ToolContext::new(), ClosePanelArgs {})
            .await
            .expect("the panel task is running")
    })
    .await;

    assert_eq!(output, "No panel was open on the screen.");
    assert_eq!(sent, []);
}

#[tokio::test(start_paused = true)]
async fn the_policy_accepts_close_panel_as_the_runs_one_tool_call() {
    let turns = vec![
        vec![
            MockStreamEvent::tool_call("call-1", "close_panel", json!({})),
            MockStreamEvent::final_response(Usage::new()),
        ],
        vec![
            MockStreamEvent::text("Closed."),
            MockStreamEvent::final_response(Usage::new()),
        ],
    ];
    let (succeeded, sent) = beside_a_panel_service(async |panels| {
        show_stats(&panels).await;
        let model = MockCompletionModel::from_stream_turns(turns);
        let assistant = Assistant::new(register(AgentBuilder::new(model), panels).build());
        assistant
            .respond("You may close the panel.", &[])
            .await
            .is_ok()
    })
    .await;

    assert!(succeeded);
    assert_eq!(
        sent,
        [
            ToOrb::ShowPanel {
                panel: PanelName::Stats
            },
            ToOrb::HidePanel {}
        ]
    );
}
