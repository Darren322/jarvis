use rig_core::message::{Message, UserContent};

use super::{History, MAX_HISTORY_BYTES, serialized_batch_bytes};

fn user_message(text: &str) -> Message {
    Message::User {
        content: vec![UserContent::text(text)],
    }
}

#[test]
fn keeps_three_whole_successful_batches() {
    let mut history = History::default();
    let batch = |prompt: &str, answer: &str| {
        vec![
            user_message(prompt),
            Message::Assistant {
                id: None,
                content: vec![rig_core::message::AssistantContent::text(answer)],
            },
        ]
    };

    let first = batch("one", "answer one");
    let second = batch("two", "answer two");
    let third = batch("three", "answer three");
    let fourth = batch("four", "answer four");

    history.record_success_with_sources(Some(&first), &[], None);
    history.record_success_with_sources(Some(&second), &[], None);
    history.record_success_with_sources(Some(&third), &[], None);
    history.record_success_with_sources(Some(&fourth), &[], None);

    assert_eq!(history.messages(), [second, third, fourth].concat());
}

#[test]
fn history_byte_limit_keeps_exact_boundary_and_evicts_oldest_batch() {
    let mut history = History::default();
    let empty_message = [user_message("")];
    let overhead = serialized_batch_bytes(&empty_message).expect("message serializes");
    let exact_batch = [user_message(&"x".repeat(MAX_HISTORY_BYTES - overhead))];

    assert_eq!(
        serialized_batch_bytes(&exact_batch).expect("message serializes"),
        MAX_HISTORY_BYTES
    );
    assert_eq!(
        history.record_success_with_sources(Some(&exact_batch), &[], None),
        None
    );
    assert_eq!(history.messages(), exact_batch.to_vec());

    let newest = [user_message("newest")];
    assert_eq!(
        history.record_success_with_sources(Some(&newest), &[], None),
        None
    );
    assert_eq!(history.messages(), newest.to_vec());
}

#[test]
fn oversized_new_batch_clears_older_context_and_is_not_retained() {
    let mut history = History::default();
    let prior = [user_message("prior")];
    history.record_success_with_sources(Some(&prior), &[], None);

    let overhead = serialized_batch_bytes(&[user_message("")]).expect("message serializes");
    let oversized = [user_message(&"x".repeat(MAX_HISTORY_BYTES + 1 - overhead))];

    assert!(
        history
            .record_success_with_sources(Some(&oversized), &[], None)
            .is_some()
    );
    assert!(history.messages().is_empty());
}

#[test]
fn missing_messages_clears_active_history_and_warns() {
    let mut history = History::default();
    history.record_success_with_sources(Some(&[user_message("prior")]), &[], None);

    assert!(
        history
            .record_success_with_sources(None, &[], None)
            .is_some()
    );
    assert!(history.messages().is_empty());
}
