//! The built Panel (jarvis-ui 12): its message, its parts and its limits.

use orb_protocol::{
    BuiltPanel, BuiltPanelError, GaugeSource, MAX_LIST_ITEMS, MAX_PANEL_PARTS,
    MAX_PANEL_TITLE_CHARS, MAX_PART_TEXT_CHARS, MAX_TO_ORB_LINE_BYTES, PanelName, PanelPart,
    ProtocolError, ToOrb,
};

fn card(title: &str, text: &str) -> PanelPart {
    PanelPart::Card {
        title: title.to_owned(),
        text: text.to_owned(),
    }
}

fn list(title: &str, items: &[&str]) -> PanelPart {
    PanelPart::List {
        title: title.to_owned(),
        items: items.iter().map(|&item| item.to_owned()).collect(),
    }
}

fn gauge(source: GaugeSource) -> PanelPart {
    PanelPart::Gauge { source }
}

fn panel(title: &str, parts: Vec<PanelPart>) -> BuiltPanel {
    BuiltPanel::new(title.to_owned(), parts).expect("the test panel is within the limits")
}

#[test]
fn the_built_panel_message_and_each_part_have_their_documented_line_and_round_trip() {
    let cases = [
        (
            panel(
                "Facts",
                vec![list(
                    "Facts",
                    &["Darren likes tea", "Darren lives in Penang"],
                )],
            ),
            r#"{"type":"show_built_panel","title":"Facts","parts":[{"kind":"list","title":"Facts","items":["Darren likes tea","Darren lives in Penang"]}]}"#,
        ),
        (
            panel("Note", vec![card("Reminder", "Water the plants")]),
            r#"{"type":"show_built_panel","title":"Note","parts":[{"kind":"card","title":"Reminder","text":"Water the plants"}]}"#,
        ),
        (
            panel(
                "Pi",
                vec![
                    gauge(GaugeSource::Cpu),
                    gauge(GaugeSource::Temperature),
                    gauge(GaugeSource::Ram),
                    gauge(GaugeSource::Storage),
                ],
            ),
            r#"{"type":"show_built_panel","title":"Pi","parts":[{"kind":"gauge","source":"cpu"},{"kind":"gauge","source":"temperature"},{"kind":"gauge","source":"ram"},{"kind":"gauge","source":"storage"}]}"#,
        ),
        (
            panel(
                "Servers",
                vec![gauge(GaugeSource::Llama), gauge(GaugeSource::Bge)],
            ),
            r#"{"type":"show_built_panel","title":"Servers","parts":[{"kind":"gauge","source":"llama"},{"kind":"gauge","source":"bge"}]}"#,
        ),
    ];
    for (built, wire) in cases {
        let message = ToOrb::ShowBuiltPanel(built);
        let line = message.encode().expect("encodes");
        assert_eq!(line, format!("{wire}\n"));
        assert_eq!(ToOrb::decode(&line).expect("decodes with newline"), message);
        assert_eq!(
            ToOrb::decode(wire).expect("decodes without newline"),
            message
        );
    }
}

#[test]
fn model_text_with_quotes_newlines_and_markup_survives_the_wire_on_one_line() {
    let built = panel(
        "Say \"hi\"",
        vec![card("<b>bold</b>", "line one\nline two\u{1}")],
    );
    let message = ToOrb::ShowBuiltPanel(built);
    let line = message.encode().expect("encodes");
    assert_eq!(line.matches('\n').count(), 1, "only the terminator: {line}");
    assert_eq!(ToOrb::decode(&line).expect("decodes"), message);
}

#[test]
fn unknown_kinds_sources_and_fields_and_missing_fields_are_rejected() {
    let rejected = [
        // Not in the Catalog.
        r#"{"type":"show_built_panel","title":"T","parts":[{"kind":"chart","title":"x"}]}"#,
        r#"{"type":"show_built_panel","title":"T","parts":[{"kind":"timer","seconds":600}]}"#,
        r#"{"type":"show_built_panel","title":"T","parts":[{"kind":"Card","title":"a","text":"b"}]}"#,
        r#"{"type":"show_built_panel","title":"T","parts":[{"title":"a","text":"b"}]}"#,
        // Unknown gauge sources.
        r#"{"type":"show_built_panel","title":"T","parts":[{"kind":"gauge","source":"weather"}]}"#,
        r#"{"type":"show_built_panel","title":"T","parts":[{"kind":"gauge","source":"CPU"}]}"#,
        r#"{"type":"show_built_panel","title":"T","parts":[{"kind":"gauge"}]}"#,
        // A model never writes a gauge's number.
        r#"{"type":"show_built_panel","title":"T","parts":[{"kind":"gauge","source":"cpu","value":12}]}"#,
        // Unknown or missing fields in a part.
        r#"{"type":"show_built_panel","title":"T","parts":[{"kind":"card","title":"a","text":"b","html":"<b>"}]}"#,
        r#"{"type":"show_built_panel","title":"T","parts":[{"kind":"card","title":"a"}]}"#,
        r#"{"type":"show_built_panel","title":"T","parts":[{"kind":"list","title":"a","items":["b"],"extra":1}]}"#,
        r#"{"type":"show_built_panel","title":"T","parts":[{"kind":"list","title":"a"}]}"#,
        r#"{"type":"show_built_panel","title":"T","parts":[{"kind":"list","title":"a","items":[1]}]}"#,
        // Unknown or missing fields in the message.
        r#"{"type":"show_built_panel","title":"T","parts":[{"kind":"gauge","source":"cpu"}],"extra":1}"#,
        r#"{"type":"show_built_panel","parts":[{"kind":"gauge","source":"cpu"}]}"#,
        r#"{"type":"show_built_panel","title":"T"}"#,
        r#"{"type":"show_built_panel","title":null,"parts":[{"kind":"gauge","source":"cpu"}]}"#,
        // The limits hold on the wire too: no parts, and too many.
        r#"{"type":"show_built_panel","title":"T","parts":[]}"#,
        r#"{"type":"show_built_panel","title":"T","parts":[{"kind":"gauge","source":"cpu"},{"kind":"gauge","source":"cpu"},{"kind":"gauge","source":"cpu"},{"kind":"gauge","source":"cpu"},{"kind":"gauge","source":"cpu"}]}"#,
        // The built Panel is never shown by name.
        r#"{"type":"show_panel","panel":"built"}"#,
    ];
    for line in rejected {
        assert!(
            matches!(ToOrb::decode(line), Err(ProtocolError::Invalid(_))),
            "{line}"
        );
    }
}

#[test]
fn the_built_panel_state_is_named_built_on_the_wire() {
    assert_eq!(
        serde_json::to_string(&PanelName::Built).expect("encodes"),
        r#""built""#
    );
}

/// `count` characters of `unit`, as a `String`.
fn chars(unit: char, count: usize) -> String {
    std::iter::repeat_n(unit, count).collect()
}

/// One field at its limit and one character over, for each text limit. 🍎 is 4
/// bytes in UTF-8, so a limit counted in bytes would fail its row.
#[test]
fn every_limit_accepts_its_last_value_and_rejects_the_first_one_over() {
    let text = |count: usize| chars('a', count);
    let gauges = |count: usize| vec![gauge(GaugeSource::Cpu); count];
    let items = |count: usize| vec!["milk".to_owned(); count];
    let a_list = |title: String, items: Vec<String>| PanelPart::List { title, items };
    let a_card = |title: String, text: String| PanelPart::Card { title, text };
    // Literals, not the constants: the table is the spec (G4), so a changed
    // constant fails it.
    let at = 120;
    let over = 121;

    type Case = (
        &'static str,
        String,
        Vec<PanelPart>,
        Option<BuiltPanelError>,
    );
    let cases: Vec<Case> = vec![
        (
            "0 parts",
            text(1),
            gauges(0),
            Some(BuiltPanelError::NoParts),
        ),
        ("1 part", text(1), gauges(1), None),
        ("4 parts", text(1), gauges(4), None),
        (
            "5 parts",
            text(1),
            gauges(5),
            Some(BuiltPanelError::TooManyParts { parts: 5 }),
        ),
        (
            "0 list items",
            text(1),
            vec![a_list(text(1), items(0))],
            Some(BuiltPanelError::EmptyList { part: 1 }),
        ),
        (
            "8 list items",
            text(1),
            vec![a_list(text(1), items(8))],
            None,
        ),
        (
            "9 list items",
            text(1),
            vec![gauge(GaugeSource::Ram), a_list(text(1), items(9))],
            Some(BuiltPanelError::TooManyItems { part: 2, items: 9 }),
        ),
        (
            "card title at the limit",
            text(1),
            vec![a_card(text(at), text(1))],
            None,
        ),
        (
            "card title over",
            text(1),
            vec![a_card(text(over), text(1))],
            Some(BuiltPanelError::CardTitleTooLong {
                part: 1,
                chars: over,
            }),
        ),
        (
            "card text at the limit",
            text(1),
            vec![a_card(text(1), text(at))],
            None,
        ),
        (
            "card text over",
            text(1),
            vec![a_card(text(1), text(over))],
            Some(BuiltPanelError::CardTextTooLong {
                part: 1,
                chars: over,
            }),
        ),
        (
            "card text of 4-byte characters at the limit",
            text(1),
            vec![a_card(text(1), chars('🍎', at))],
            None,
        ),
        (
            "card text of 4-byte characters over",
            text(1),
            vec![a_card(text(1), chars('🍎', over))],
            Some(BuiltPanelError::CardTextTooLong {
                part: 1,
                chars: over,
            }),
        ),
        (
            "list title at the limit",
            text(1),
            vec![a_list(text(at), items(1))],
            None,
        ),
        (
            "list title over",
            text(1),
            vec![a_list(text(over), items(1))],
            Some(BuiltPanelError::ListTitleTooLong {
                part: 1,
                chars: over,
            }),
        ),
        (
            "list item at the limit",
            text(1),
            vec![a_list(text(1), vec!["milk".to_owned(), text(at)])],
            None,
        ),
        (
            "list item over",
            text(1),
            vec![a_list(text(1), vec!["milk".to_owned(), text(over)])],
            Some(BuiltPanelError::ListItemTooLong {
                part: 1,
                item: 2,
                chars: over,
            }),
        ),
        ("panel title at the limit", text(40), gauges(1), None),
        (
            "panel title over",
            text(41),
            gauges(1),
            Some(BuiltPanelError::TitleTooLong { chars: 41 }),
        ),
    ];
    for (name, title, parts, expected) in cases {
        let built = BuiltPanel::new(title, parts);
        match expected {
            None => assert!(built.is_ok(), "{name}: {built:?}"),
            Some(error) => assert_eq!(built, Err(error), "{name}"),
        }
    }
}

#[test]
fn each_rejection_names_the_limit_it_broke() {
    let cases = [
        (
            BuiltPanelError::TitleTooLong { chars: 41 },
            "a panel title has at most 40 characters, not 41",
        ),
        (BuiltPanelError::NoParts, "a panel has at least 1 part"),
        (
            BuiltPanelError::TooManyParts { parts: 5 },
            "a panel has at most 4 parts, not 5",
        ),
        (
            BuiltPanelError::EmptyList { part: 2 },
            "part 2: a list has at least 1 item",
        ),
        (
            BuiltPanelError::TooManyItems { part: 1, items: 9 },
            "part 1: a list has at most 8 items, not 9",
        ),
        (
            BuiltPanelError::CardTitleTooLong {
                part: 1,
                chars: 121,
            },
            "part 1: a card title has at most 120 characters, not 121",
        ),
        (
            BuiltPanelError::CardTextTooLong {
                part: 3,
                chars: 121,
            },
            "part 3: a card text has at most 120 characters, not 121",
        ),
        (
            BuiltPanelError::ListTitleTooLong {
                part: 1,
                chars: 200,
            },
            "part 1: a list title has at most 120 characters, not 200",
        ),
        (
            BuiltPanelError::ListItemTooLong {
                part: 2,
                item: 4,
                chars: 121,
            },
            "part 2, item 4: a list item has at most 120 characters, not 121",
        ),
    ];
    for (error, message) in cases {
        assert_eq!(error.to_string(), message);
    }
}

/// The widest built Panel the limits allow: the most parts, each the widest
/// part (a list of the most items), every text at its limit and written with
/// the character JSON writes widest (`\u0001`, six bytes).
fn widest_built_panel() -> BuiltPanel {
    let widest = |count: usize| chars('\u{1}', count);
    let part = PanelPart::List {
        title: widest(MAX_PART_TEXT_CHARS),
        items: vec![widest(MAX_PART_TEXT_CHARS); MAX_LIST_ITEMS],
    };
    BuiltPanel::new(widest(MAX_PANEL_TITLE_CHARS), vec![part; MAX_PANEL_PARTS])
        .expect("the widest panel is within the limits")
}

#[test]
fn the_line_limit_to_the_orb_is_the_size_of_the_widest_built_panel() {
    let line = ToOrb::ShowBuiltPanel(widest_built_panel())
        .encode()
        .expect("the widest panel encodes");
    let widest = line.trim_end_matches('\n');
    assert_eq!(widest.len(), MAX_TO_ORB_LINE_BYTES);
    assert!(ToOrb::decode(widest).is_ok());
    // One byte more (a space is valid JSON) is rejected before parsing.
    assert!(matches!(
        ToOrb::decode(&format!("{widest} ")),
        Err(ProtocolError::LineTooLong { len, limit })
            if len == MAX_TO_ORB_LINE_BYTES + 1 && limit == MAX_TO_ORB_LINE_BYTES
    ));
}

#[test]
fn a_card_is_narrower_on_the_wire_than_a_list_so_lists_set_the_limit() {
    let widest = |count: usize| chars('\u{1}', count);
    let cards = BuiltPanel::new(
        widest(MAX_PANEL_TITLE_CHARS),
        vec![
            PanelPart::Card {
                title: widest(MAX_PART_TEXT_CHARS),
                text: widest(MAX_PART_TEXT_CHARS),
            };
            MAX_PANEL_PARTS
        ],
    )
    .expect("within the limits");
    let line = ToOrb::ShowBuiltPanel(cards).encode().expect("encodes");
    assert!(line.len() < MAX_TO_ORB_LINE_BYTES);
}
