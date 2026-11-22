use rig_agent::tool::Tool;
use rig_core::serde_json::{from_value, json};

use super::{ListTimersArgs, ListTimersTool, time_left, views};
use crate::features::timers::channel;
use crate::storage::TimerState;
use crate::tools::allowed_tool;
use crate::tools::timers::tests::{MINUTE_MS, Service, now, now_ms, timer};

#[test]
fn time_left_is_said_in_whole_minutes_rounded_down() {
    let cases = [
        (i64::MIN, "less than a minute"),
        (-1, "less than a minute"),
        (0, "less than a minute"),
        (MINUTE_MS - 1, "less than a minute"),
        (MINUTE_MS, "1 minute"),
        (2 * MINUTE_MS - 1, "1 minute"),
        (25 * MINUTE_MS, "25 minutes"),
        (59 * MINUTE_MS, "59 minutes"),
        (60 * MINUTE_MS, "1 hour"),
        (65 * MINUTE_MS, "1 hour 5 minutes"),
        (121 * MINUTE_MS, "2 hours 1 minute"),
        (24 * 60 * MINUTE_MS, "24 hours"),
    ];
    for (left_ms, said) in cases {
        assert_eq!(time_left(left_ms), said, "{left_ms} ms");
    }
}

#[test]
fn each_state_is_told_in_words_with_time_left_only_for_a_running_timer() {
    let timers = [
        timer(1, Some("Time to sleep"), 300, TimerState::Running),
        timer(2, None, 10, TimerState::Pending),
        timer(3, Some("Tea"), -1, TimerState::Ended),
    ];

    let told = views(&now(), &timers).expect("told");

    let told: Vec<_> = told
        .iter()
        .map(|view| {
            (
                view.label.as_deref(),
                view.state,
                view.ends_at.as_str(),
                view.time_left.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        told,
        [
            (
                Some("Time to sleep"),
                "running",
                "11:00 pm today",
                Some("5 hours")
            ),
            (None, "waiting_for_yes", "6:10 pm today", None),
            (Some("Tea"), "ended", "5:59 pm today", None),
        ]
    );
}

#[test]
fn no_arguments_are_accepted_and_any_argument_is_rejected() {
    let rule = allowed_tool("list_timers").expect("list_timers is allowed");
    assert!(from_value::<ListTimersArgs>(json!({})).is_ok());
    assert!(rule.accepts_arguments(&json!({})));
    for arguments in [json!({"label": "x"}), json!({"extra": 1}), json!(null)] {
        assert!(
            from_value::<ListTimersArgs>(arguments.clone()).is_err(),
            "{arguments}"
        );
        assert!(!rule.accepts_arguments(&arguments), "{arguments}");
    }
}

#[test]
fn the_name_schema_and_description_are_for_the_model() {
    let tool = ListTimersTool::new(channel().0);
    assert_eq!(ListTimersTool::NAME, "list_timers");
    assert_eq!(
        tool.parameters(),
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    );
    let description = tool.description();
    for phrase in [
        "how long is left",
        "what timers do I have",
        "Say only what it returns",
    ] {
        assert!(description.contains(phrase), "{phrase}: {description}");
    }
}

#[tokio::test]
async fn no_timers_is_told_as_an_empty_list() {
    let service = Service::start().await;

    let output = ListTimersTool::new(service.handle.clone())
        .list(&now())
        .expect("listed");

    assert_eq!(output["timers"], json!([]));
    assert_eq!(output["note"], "There are no timers.");
}

#[tokio::test]
async fn the_running_timers_are_listed_soonest_first_with_the_time_left() {
    let service = Service::start().await;
    service.run_timer(None, 90).await;
    service.run_timer(Some("Time to do work"), 25).await;
    service
        .shown_until(|timers| {
            timers.len() == 2
                && timers
                    .iter()
                    .all(|timer| timer.state == TimerState::Running)
        })
        .await;

    let output = ListTimersTool::new(service.handle.clone())
        .list(&now())
        .expect("listed");

    assert_eq!(
        output["timers"],
        json!([
            {
                "label": "Time to do work",
                "state": "running",
                "ends_at": "6:25 pm today",
                "time_left": "25 minutes"
            },
            {
                "label": null,
                "state": "running",
                "ends_at": "7:30 pm today",
                "time_left": "1 hour 30 minutes"
            },
        ])
    );
}

#[tokio::test]
async fn a_timer_waiting_for_a_yes_is_listed_as_not_set() {
    let service = Service::start().await;
    service
        .handle
        .propose(Some("Tea".to_owned()), now_ms() + MINUTE_MS)
        .await
        .expect("proposed");
    service.shown_until(|timers| timers.len() == 1).await;

    let output = ListTimersTool::new(service.handle.clone())
        .list(&now())
        .expect("listed");

    assert_eq!(output["timers"][0]["state"], "waiting_for_yes");
    assert_eq!(output["timers"][0]["time_left"], json!(null));
}
