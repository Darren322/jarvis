use orb_protocol::{
    BuiltPanel, GaugeSource, MAX_LIST_ITEMS, MAX_PANEL_PARTS, MAX_PANEL_TITLE_CHARS,
    MAX_PART_TEXT_CHARS, MIN_LIST_ITEMS, MIN_PANEL_PARTS, PanelName, PanelPart, ToOrb,
};
use rig_agent::{
    AgentBuilder,
    tool::{Tool, ToolContext},
};
use rig_core::{
    completion::Usage,
    message::{Message, ToolResultContent, UserContent},
    serde_json::{Value, from_value, json},
    test_utils::{MockCompletionModel, MockStreamEvent},
};
use tokio::{sync::mpsc, task::JoinHandle};

use super::{BuildPanelArgs, BuildPanelError, BuildPanelTool};
use crate::features::{
    assistant::Assistant,
    panel::{PANEL_TIMEOUT, PanelHandle, channel},
};
use crate::interfaces::orb::OrbHandle;
use crate::tools::allowed_tool;

/// Darren's E1: his facts as a list.
fn facts_arguments() -> Value {
    json!({
        "title": "Facts",
        "parts": [
            {"kind": "list", "title": "What I know", "items": ["Darren likes tea", "Darren lives in Penang"]}
        ]
    })
}

/// A Panel one part over the limit: five gauges.
fn five_parts_arguments() -> Value {
    json!({
        "title": "Pi",
        "parts": [
            {"kind": "gauge", "source": "cpu"},
            {"kind": "gauge", "source": "temperature"},
            {"kind": "gauge", "source": "ram"},
            {"kind": "gauge", "source": "storage"},
            {"kind": "gauge", "source": "llama"}
        ]
    })
}

#[test]
fn the_argument_rule_accepts_every_catalog_part_and_rejects_anything_else() {
    let rule = allowed_tool("build_panel").expect("build_panel is allowed");
    let accepted = [
        facts_arguments(),
        json!({"title": "Note", "parts": [{"kind": "card", "title": "Plants", "text": "Water them"}]}),
        json!({"title": "Pi", "parts": [{"kind": "gauge", "source": "cpu"}, {"kind": "gauge", "source": "bge"}]}),
        // Over a limit is still the right shape: the tool rejects it with a
        // message, so the run goes on and the model can say what was wrong.
        five_parts_arguments(),
        json!({"title": "Empty", "parts": []}),
    ];
    let rejected = [
        json!({"title": "T", "parts": [{"kind": "chart", "title": "CPU"}]}),
        json!({"title": "T", "parts": [{"kind": "timer", "seconds": 600}]}),
        json!({"title": "T", "parts": [{"kind": "camera"}]}),
        json!({"title": "T", "parts": [{"kind": "gauge", "source": "weather"}]}),
        json!({"title": "T", "parts": [{"kind": "gauge", "source": "cpu", "value": 12}]}),
        json!({"title": "T", "parts": [{"kind": "card", "title": "a", "text": "b", "html": "<b>"}]}),
        json!({"title": "T", "parts": [{"kind": "list", "title": "a"}]}),
        json!({"title": "T", "parts": [{"kind": "gauge", "source": "cpu"}], "extra": 1}),
        json!({"parts": [{"kind": "gauge", "source": "cpu"}]}),
        json!({"title": "T"}),
        json!([]),
    ];
    for arguments in accepted {
        assert!(rule.accepts_arguments(&arguments), "{arguments}");
    }
    for arguments in rejected {
        assert!(!rule.accepts_arguments(&arguments), "{arguments}");
    }
}

/// Every gauge source, with its wire name. Exhaustive, so a new source fails
/// to compile here until the schema test below knows it.
fn wire_name(source: GaugeSource) -> &'static str {
    match source {
        GaugeSource::Cpu => "cpu",
        GaugeSource::Temperature => "temperature",
        GaugeSource::Ram => "ram",
        GaugeSource::Storage => "storage",
        GaugeSource::Llama => "llama",
        GaugeSource::Bge => "bge",
    }
}

#[test]
fn the_schema_states_the_three_parts_their_sources_and_every_limit() {
    let (panels, _inbox) = channel();
    let schema = BuildPanelTool::new(panels).parameters();
    assert_eq!(schema["additionalProperties"], json!(false));
    assert_eq!(schema["required"], json!(["title", "parts"]));
    assert_eq!(
        schema["properties"]["title"]["maxLength"],
        json!(MAX_PANEL_TITLE_CHARS)
    );
    let parts = &schema["properties"]["parts"];
    assert_eq!(parts["minItems"], json!(MIN_PANEL_PARTS));
    assert_eq!(parts["maxItems"], json!(MAX_PANEL_PARTS));

    let kinds = parts["items"]["anyOf"]
        .as_array()
        .expect("the parts are alternatives");
    let kind = |name: &str| {
        kinds
            .iter()
            .find(|part| part["properties"]["kind"]["const"] == json!(name))
            .unwrap_or_else(|| panic!("the schema has a {name} part"))
    };
    assert_eq!(kinds.len(), 3);
    for part in kinds {
        assert_eq!(part["additionalProperties"], json!(false));
    }

    let card = kind("card");
    assert_eq!(card["required"], json!(["kind", "title", "text"]));
    for field in ["title", "text"] {
        assert_eq!(
            card["properties"][field]["maxLength"],
            json!(MAX_PART_TEXT_CHARS)
        );
    }

    let list = kind("list");
    assert_eq!(list["required"], json!(["kind", "title", "items"]));
    assert_eq!(
        list["properties"]["title"]["maxLength"],
        json!(MAX_PART_TEXT_CHARS)
    );
    let items = &list["properties"]["items"];
    assert_eq!(items["minItems"], json!(MIN_LIST_ITEMS));
    assert_eq!(items["maxItems"], json!(MAX_LIST_ITEMS));
    assert_eq!(items["items"]["maxLength"], json!(MAX_PART_TEXT_CHARS));

    let gauge = kind("gauge");
    assert_eq!(gauge["required"], json!(["kind", "source"]));
    let all_sources = [
        GaugeSource::Cpu,
        GaugeSource::Temperature,
        GaugeSource::Ram,
        GaugeSource::Storage,
        GaugeSource::Llama,
        GaugeSource::Bge,
    ];
    let expected: Vec<_> = all_sources.into_iter().map(wire_name).collect();
    assert_eq!(gauge["properties"]["source"]["enum"], json!(expected));
    for name in expected {
        assert!(from_value::<GaugeSource>(json!(name)).is_ok(), "{name}");
    }
}

#[test]
fn the_description_names_darrens_examples_and_says_jarvis_fills_the_gauges() {
    let (panels, _inbox) = channel();
    let description = BuildPanelTool::new(panels).description();
    for phrase in [
        "show my facts as a list",
        "make me a shopping list",
        "show me the CPU and temperature as gauges",
        "Jarvis fills in its value",
        "never write a number",
        "nothing on it is saved",
    ] {
        assert!(description.contains(phrase), "{phrase}: {description}");
    }
}

/// A running panel state with the orb's side of its channel.
fn panel_service() -> (PanelHandle, mpsc::Receiver<ToOrb>, JoinHandle<()>) {
    let (panels, inbox) = channel();
    let (to_orb, shown_to_orb) = mpsc::channel(16);
    let (_events, from_orb) = mpsc::channel(16);
    let task = tokio::spawn(async move {
        match inbox
            .run(Some(OrbHandle { to_orb, from_orb }), PANEL_TIMEOUT)
            .await {}
    });
    (panels, shown_to_orb, task)
}

#[tokio::test(start_paused = true)]
async fn calling_the_tool_shows_the_built_panel_and_tells_the_model() {
    let (panels, mut shown_to_orb, task) = panel_service();
    let args: BuildPanelArgs = from_value(facts_arguments()).expect("valid arguments");

    let output = BuildPanelTool::new(panels.clone())
        .call(&mut ToolContext::new(), args)
        .await
        .expect("the panel is within the limits");

    assert_eq!(
        output,
        "The panel is now shown on the screen. It will close by itself. You cannot see the \
         values of its gauges."
    );
    let expected = BuiltPanel::new(
        "Facts".to_owned(),
        vec![PanelPart::List {
            title: "What I know".to_owned(),
            items: vec![
                "Darren likes tea".to_owned(),
                "Darren lives in Penang".to_owned(),
            ],
        }],
    )
    .expect("within the limits");
    assert_eq!(
        shown_to_orb.recv().await,
        Some(ToOrb::ShowBuiltPanel(expected))
    );
    assert_eq!(panels.shown(), Some(PanelName::Built));
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn a_rejected_panel_shows_nothing_and_its_error_names_the_limit() {
    let (panels, mut shown_to_orb, task) = panel_service();
    let tool = BuildPanelTool::new(panels.clone());
    let args: BuildPanelArgs = from_value(five_parts_arguments()).expect("the right shape");

    let error = tool
        .call(&mut ToolContext::new(), args)
        .await
        .expect_err("five parts is over the limit");

    assert!(matches!(error, BuildPanelError::Rejected(_)));
    assert_eq!(error.to_string(), "a panel has at most 4 parts, not 5");
    // What the model is told is the same message, not Rig's redacted default.
    let for_model = tool.map_error(error);
    assert_eq!(
        for_model.model_feedback(),
        Some("a panel has at most 4 parts, not 5")
    );
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(shown_to_orb.try_recv().is_err());
    assert_eq!(panels.shown(), None);
    task.abort();
}

fn call_turn(arguments: Value) -> Vec<MockStreamEvent> {
    vec![
        MockStreamEvent::tool_call("call-1", "build_panel", arguments),
        MockStreamEvent::final_response(Usage::new()),
    ]
}

fn text_turn(text: &str) -> Vec<MockStreamEvent> {
    vec![
        MockStreamEvent::text(text),
        MockStreamEvent::final_response(Usage::new()),
    ]
}

/// What a streamed run through the real registration and policy hook did.
struct Run {
    answer: Option<String>,
    sent: Vec<ToOrb>,
    /// The text of every tool result the second request carried.
    tool_results: Vec<String>,
}

async fn run(turns: Vec<Vec<MockStreamEvent>>) -> Run {
    let (panels, mut shown_to_orb, task) = panel_service();
    let model = MockCompletionModel::from_stream_turns(turns);
    let handle = model.clone();
    let assistant =
        Assistant::new(crate::tools::register(AgentBuilder::new(model), panels).build());

    let answer = assistant
        .respond("Show my facts as a list.", &[])
        .await
        .ok()
        .map(|run| run.response.output);
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    task.abort();
    let _ = task.await;
    let mut sent = Vec::new();
    while let Ok(message) = shown_to_orb.try_recv() {
        sent.push(message);
    }
    let tool_results = handle
        .requests()
        .get(1)
        .map(|request| {
            request
                .chat_history
                .iter()
                .flat_map(|message| match message {
                    Message::User { content } => content.to_vec(),
                    _ => Vec::new(),
                })
                .filter_map(|content| match content {
                    UserContent::ToolResult(result) => Some(result),
                    _ => None,
                })
                .flat_map(|result| result.content.into_iter())
                .filter_map(|content| match content {
                    ToolResultContent::Text(text) => Some(text.text),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    Run {
        answer,
        sent,
        tool_results,
    }
}

#[tokio::test(start_paused = true)]
async fn a_build_panel_call_shows_the_panel_and_the_second_turn_answers() {
    let run = run(vec![
        call_turn(facts_arguments()),
        text_turn("Here are your facts."),
    ])
    .await;

    assert_eq!(run.answer.as_deref(), Some("Here are your facts."));
    assert!(
        matches!(run.sent.as_slice(), [ToOrb::ShowBuiltPanel(built)] if built.title() == "Facts"),
        "{:?}",
        run.sent
    );
}

#[tokio::test(start_paused = true)]
async fn a_rejected_panel_sends_nothing_and_the_model_hears_which_limit_it_broke() {
    let run = run(vec![
        call_turn(five_parts_arguments()),
        text_turn("I could not build that panel."),
    ])
    .await;

    assert_eq!(run.answer.as_deref(), Some("I could not build that panel."));
    assert!(run.sent.is_empty(), "{:?}", run.sent);
    assert!(
        run.tool_results
            .iter()
            .any(|text| text.contains("a panel has at most 4 parts")),
        "{:?}",
        run.tool_results
    );
}

#[tokio::test(start_paused = true)]
async fn a_part_not_in_the_catalog_is_stopped_by_the_argument_rule() {
    let run = run(vec![
        call_turn(json!({"title": "T", "parts": [{"kind": "chart", "title": "CPU"}]})),
        text_turn("never reached"),
    ])
    .await;

    assert_eq!(run.answer, None);
    assert!(run.sent.is_empty(), "{:?}", run.sent);
}
