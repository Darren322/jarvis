use rig_core::message::{AssistantContent, Message, UserContent};

use super::{
    History, MAX_EXCHANGES, MAX_HISTORY_BYTES, Recorded, TRIMMED_EXCHANGES, TRIMMED_HISTORY_BYTES,
    serialized_batch_bytes,
};

impl History {
    /// Every stored message, oldest first.
    pub(crate) fn stored_messages(&self) -> Vec<Message> {
        let (mut messages, latest) = self.split_last_exchange();
        messages.extend(latest);
        messages
    }

    fn stored_bytes(&self) -> usize {
        serialized_batch_bytes(&self.stored_messages()).expect("history serializes")
    }
}

fn user_message(text: &str) -> Message {
    Message::User {
        content: vec![UserContent::text(text)],
    }
}

fn assistant_message(text: &str) -> Message {
    Message::Assistant {
        id: None,
        content: vec![AssistantContent::text(text)],
    }
}

/// Exchange `number` as the session stores it. Every third one used a tool, so
/// it is four messages that must stay together.
fn exchange(number: usize) -> Vec<Message> {
    let prompt = user_message(&format!("question {number}"));
    let answer = assistant_message(&format!("answer {number}"));
    if number.is_multiple_of(3) {
        vec![
            prompt,
            Message::Assistant {
                id: None,
                content: vec![AssistantContent::tool_call(
                    format!("call-{number}"),
                    "system_status",
                    serde_json::json!({}),
                )],
            },
            Message::tool_result(format!("call-{number}"), "system_status", "nominal"),
            answer,
        ]
    } else {
        vec![prompt, answer]
    }
}

/// A one-message exchange that serializes, alone, to exactly `bytes`.
fn exchange_of_bytes(bytes: usize) -> Vec<Message> {
    let overhead = serialized_batch_bytes(&[user_message("")]).expect("message serializes");
    let batch = vec![user_message(&"x".repeat(bytes - overhead))];
    assert_eq!(
        serialized_batch_bytes(&batch).expect("message serializes"),
        bytes
    );
    batch
}

/// Bytes saved when `count` one-message exchanges are serialized as one list
/// instead of one list each: every `[..][..]` seam becomes a `,`.
fn joined_bytes(alone: &[usize]) -> usize {
    alone.iter().sum::<usize>() - (alone.len() - 1)
}

#[test]
fn the_history_grows_to_its_upper_bound_and_is_then_cut_back_to_the_lower_one() {
    let mut history = History::default();
    let batches: Vec<Vec<Message>> = (1..=2 * MAX_EXCHANGES).map(exchange).collect();
    let mut recorded = Vec::new();
    let mut stored_after_first_trim = None;

    for (index, batch) in batches.iter().enumerate() {
        recorded.push(history.record_success(Some(batch), &[], None));
        // The last exchange that does not trim: everything so far is still there.
        if index + 1 == MAX_EXCHANGES {
            assert_eq!(history.stored_messages(), batches[..=index].concat());
        }
        // The first that does: exactly the lower bound remains, in whole exchanges.
        if index == MAX_EXCHANGES {
            stored_after_first_trim = Some(history.stored_messages());
        }
    }

    let first_trim = MAX_EXCHANGES;
    let turns_between_trims = MAX_EXCHANGES - TRIMMED_EXCHANGES + 1;
    let second_trim = first_trim + turns_between_trims;
    for (index, recorded) in recorded.iter().enumerate() {
        let expected = if index == first_trim || index == second_trim {
            Recorded::Trimmed
        } else {
            Recorded::Kept
        };
        assert_eq!(*recorded, expected, "exchange {}", index + 1);
    }
    assert_eq!(
        stored_after_first_trim.expect("the first trim was reached"),
        batches[first_trim + 1 - TRIMMED_EXCHANGES..=first_trim].concat()
    );
    // Between trims nothing is dropped, so each request starts as the last one did.
    assert_eq!(
        history.stored_messages(),
        batches[second_trim + 1 - TRIMMED_EXCHANGES..].concat()
    );
}

#[test]
fn the_byte_cap_keeps_its_exact_boundary_and_one_byte_more_starts_a_trim() {
    struct Case {
        name: &'static str,
        extra_bytes: usize,
        expected: Recorded,
        stored_exchanges: usize,
    }
    let cases = [
        Case {
            name: "exactly the cap",
            extra_bytes: 0,
            expected: Recorded::Kept,
            stored_exchanges: 2,
        },
        Case {
            name: "one byte over",
            extra_bytes: 1,
            expected: Recorded::Trimmed,
            stored_exchanges: 1,
        },
    ];

    for case in cases {
        let mut history = History::default();
        let newest = exchange_of_bytes(200);
        // Sized so that the two together are exactly the cap, plus the case's extra.
        let older = exchange_of_bytes(MAX_HISTORY_BYTES + 1 - 200 + case.extra_bytes);
        assert_eq!(
            joined_bytes(&[MAX_HISTORY_BYTES + 1 - 200 + case.extra_bytes, 200]),
            MAX_HISTORY_BYTES + case.extra_bytes
        );

        assert_eq!(
            history.record_success(Some(&older), &[], None),
            Recorded::Kept
        );
        assert_eq!(
            history.record_success(Some(&newest), &[], None),
            case.expected,
            "{}",
            case.name
        );
        let expected_stored = if case.stored_exchanges == 2 {
            [older, newest.clone()].concat()
        } else {
            newest.clone()
        };
        assert_eq!(history.stored_messages(), expected_stored, "{}", case.name);
    }
}

#[test]
fn a_byte_trim_drops_the_oldest_exchanges_until_half_the_cap_fits() {
    struct Case {
        name: &'static str,
        extra_bytes: usize,
        stored_exchanges: usize,
    }
    let cases = [
        Case {
            name: "the two newest are exactly half the cap",
            extra_bytes: 0,
            stored_exchanges: 2,
        },
        Case {
            name: "the two newest are one byte over half the cap",
            extra_bytes: 1,
            stored_exchanges: 1,
        },
    ];

    for case in cases {
        let mut history = History::default();
        let middle_bytes = TRIMMED_HISTORY_BYTES / 2;
        let newest_bytes = TRIMMED_HISTORY_BYTES + 1 - middle_bytes + case.extra_bytes;
        let oldest = exchange_of_bytes(MAX_HISTORY_BYTES - TRIMMED_HISTORY_BYTES + 2);
        let middle = exchange_of_bytes(middle_bytes);
        let newest = exchange_of_bytes(newest_bytes);
        assert_eq!(
            joined_bytes(&[middle_bytes, newest_bytes]),
            TRIMMED_HISTORY_BYTES + case.extra_bytes
        );

        assert_eq!(
            history.record_success(Some(&oldest), &[], None),
            Recorded::Kept
        );
        assert_eq!(
            history.record_success(Some(&middle), &[], None),
            Recorded::Kept
        );
        assert_eq!(
            history.record_success(Some(&newest), &[], None),
            Recorded::Trimmed,
            "{}",
            case.name
        );

        let expected_stored = if case.stored_exchanges == 2 {
            [middle, newest].concat()
        } else {
            newest
        };
        assert_eq!(history.stored_messages(), expected_stored, "{}", case.name);
        if case.stored_exchanges > 1 {
            assert!(history.stored_bytes() <= TRIMMED_HISTORY_BYTES);
        }
    }
}

#[test]
fn a_single_exchange_is_kept_at_the_byte_cap_and_refused_one_byte_over() {
    let mut history = History::default();
    let exact = exchange_of_bytes(MAX_HISTORY_BYTES);
    assert_eq!(
        history.record_success(Some(&exact), &[], None),
        Recorded::Kept
    );
    assert_eq!(history.stored_messages(), exact);

    let oversized = exchange_of_bytes(MAX_HISTORY_BYTES + 1);
    assert!(matches!(
        history.record_success(Some(&oversized), &[], None),
        Recorded::Cleared(_)
    ));
    assert!(history.stored_messages().is_empty());
}

#[test]
fn missing_messages_clears_active_history_and_warns() {
    let mut history = History::default();
    history.record_success(Some(&[user_message("prior")]), &[], None);

    assert!(matches!(
        history.record_success(None, &[], None),
        Recorded::Cleared(_)
    ));
    assert!(history.stored_messages().is_empty());
}
