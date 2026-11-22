use rig_agent::tool::Tool;
use rig_core::serde_json::{Value, from_value, json};

use super::{CancelTimerArgs, CancelTimerTool};
use crate::features::timers::{CancelOutcome, channel};
use crate::storage::Timer;
use crate::tools::allowed_tool;
use crate::tools::timers::tests::{Service, now, now_ms};

fn args(value: Value) -> CancelTimerArgs {
    from_value(value).expect("the arguments are accepted")
}

/// What is still pending or running, read from storage after the tool's call.
async fn still_there(service: &Service) -> Vec<Timer> {
    match service
        .handle
        .cancel_matching(Some("no-such-label-anywhere".to_owned()))
        .await
        .expect("answered")
    {
        CancelOutcome::NoMatch(timers) => timers,
        other => panic!("nothing matches that label: {other:?}"),
    }
}

fn labels(timers: &[Timer]) -> Vec<Option<&str>> {
    timers.iter().map(|timer| timer.label.as_deref()).collect()
}

fn listed_labels(output: &Value) -> Vec<Value> {
    output["timers"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|view| view["label"].clone())
        .collect()
}

#[test]
fn an_optional_label_is_accepted_and_nothing_else() {
    let rule = allowed_tool("cancel_timer").expect("cancel_timer is allowed");
    for arguments in [json!({}), json!({"label": "sleep"}), json!({"label": null})] {
        assert!(
            from_value::<CancelTimerArgs>(arguments.clone()).is_ok(),
            "{arguments}"
        );
        assert!(rule.accepts_arguments(&arguments), "{arguments}");
    }
    for arguments in [
        json!({"label": 3}),
        json!({"label": "x", "extra": 1}),
        json!({"id": 3}),
        json!(null),
    ] {
        assert!(
            from_value::<CancelTimerArgs>(arguments.clone()).is_err(),
            "{arguments}"
        );
        assert!(!rule.accepts_arguments(&arguments), "{arguments}");
    }
}

#[test]
fn the_name_schema_and_description_are_for_the_model() {
    let tool = CancelTimerTool::new(channel().0);
    assert_eq!(CancelTimerTool::NAME, "cancel_timer");
    let schema = tool.parameters();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], json!(false));
    assert_eq!(schema["properties"]["label"]["type"], "string");
    assert_eq!(schema["properties"].as_object().map(|p| p.len()), Some(1));
    assert!(schema.get("required").is_none());
    let description = tool.description();
    for phrase in [
        "cancel the timer",
        "cancel the sleep one",
        "does not ask the user first",
        "which one",
    ] {
        assert!(description.contains(phrase), "{phrase}: {description}");
    }
}

#[tokio::test]
async fn with_no_label_and_one_timer_that_timer_is_cancelled() {
    let service = Service::start().await;
    service.run_timer(Some("Time to sleep"), 300).await;
    let tool = CancelTimerTool::new(service.handle.clone());

    let output = tool.cancel(&now(), args(json!({}))).await.expect("done");

    assert_eq!(output["status"], "cancelled");
    assert_eq!(output["label"], "Time to sleep");
    assert!(still_there(&service).await.is_empty());
}

#[tokio::test]
async fn a_label_cancels_the_one_timer_it_names() {
    let service = Service::start().await;
    service.run_timer(Some("Time to sleep"), 300).await;
    service.run_timer(Some("Time to do work"), 25).await;
    let tool = CancelTimerTool::new(service.handle.clone());

    let output = tool
        .cancel(&now(), args(json!({"label": "the sleep one"})))
        .await
        .expect("done");

    assert_eq!(output["status"], "cancelled");
    assert_eq!(output["label"], "Time to sleep");
    assert_eq!(
        labels(&still_there(&service).await),
        [Some("Time to do work")]
    );
}

#[tokio::test]
async fn a_pending_timer_can_be_cancelled_too() {
    let service = Service::start().await;
    service
        .handle
        .propose(Some("Tea".to_owned()), now_ms() + 60_000)
        .await
        .expect("proposed");
    let tool = CancelTimerTool::new(service.handle.clone());

    let output = tool
        .cancel(&now(), args(json!({"label": "tea"})))
        .await
        .expect("done");

    assert_eq!(output["status"], "cancelled");
    assert!(still_there(&service).await.is_empty());
}

#[tokio::test]
async fn a_label_that_matches_nothing_cancels_nothing_and_lists_the_timers() {
    let service = Service::start().await;
    service.run_timer(Some("Time to sleep"), 300).await;
    service.run_timer(Some("Time to do work"), 25).await;
    let tool = CancelTimerTool::new(service.handle.clone());

    let output = tool
        .cancel(&now(), args(json!({"label": "gym"})))
        .await
        .expect("answered");

    assert_eq!(output["status"], "no_timer_matched_nothing_cancelled");
    assert_eq!(
        listed_labels(&output),
        [json!("Time to do work"), json!("Time to sleep")]
    );
    assert_eq!(still_there(&service).await.len(), 2);
}

#[tokio::test]
async fn a_label_that_matches_several_cancels_nothing_and_lists_those() {
    let service = Service::start().await;
    service.run_timer(Some("Time to do work"), 25).await;
    service.run_timer(Some("Work break"), 60).await;
    service.run_timer(Some("Time to sleep"), 300).await;
    let tool = CancelTimerTool::new(service.handle.clone());

    let output = tool
        .cancel(&now(), args(json!({"label": "work"})))
        .await
        .expect("answered");

    assert_eq!(output["status"], "several_timers_matched_nothing_cancelled");
    assert_eq!(
        listed_labels(&output),
        [json!("Time to do work"), json!("Work break")]
    );
    assert_eq!(still_there(&service).await.len(), 3);
}

#[tokio::test]
async fn with_no_label_and_several_timers_nothing_is_cancelled() {
    let service = Service::start().await;
    service.run_timer(Some("Tea"), 5).await;
    service.run_timer(None, 10).await;
    let tool = CancelTimerTool::new(service.handle.clone());

    let output = tool
        .cancel(&now(), args(json!({"label": "  "})))
        .await
        .expect("answered");

    assert_eq!(output["status"], "several_timers_matched_nothing_cancelled");
    assert_eq!(still_there(&service).await.len(), 2);
}

#[tokio::test]
async fn with_no_timers_there_is_nothing_to_cancel() {
    let service = Service::start().await;
    let tool = CancelTimerTool::new(service.handle.clone());

    let output = tool
        .cancel(&now(), args(json!({})))
        .await
        .expect("answered");

    assert_eq!(output["status"], "no_timer_matched_nothing_cancelled");
    assert_eq!(output["timers"], json!([]));
}
