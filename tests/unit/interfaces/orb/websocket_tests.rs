use std::{net::Ipv4Addr, time::Duration};

use orb_protocol::OrbState;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpSocket,
    time::sleep,
};
use tokio_tungstenite::tungstenite::Message;

use super::*;
use crate::{
    features::{control::Tap, state::AssistantState},
    interfaces::orb::{
        orb_state,
        tests::{ALLOWED_ORIGIN, Browser, PILED_UP_LEVELS, Read, WAIT, padded_tap, serving, state},
    },
};

const TAP_TEXT: &str = r#"{"type":"tap"}"#;

#[tokio::test]
async fn the_server_listens_on_localhost_only() {
    let endpoint = WebSocketEndpoint::bind(0).unwrap();
    let address = endpoint.local_addr().unwrap();
    assert_eq!(address.ip(), Ipv4Addr::LOCALHOST);
    assert_ne!(address.port(), 0);
}

#[tokio::test]
async fn a_port_already_in_use_is_reported_with_its_number() {
    let first = WebSocketEndpoint::bind(0).unwrap();
    let port = first.local_addr().unwrap().port();
    assert!(matches!(
        WebSocketEndpoint::bind(port),
        Err(OrbError::WebSocketBind { port: reported, .. }) if reported == port
    ));
}

#[tokio::test(start_paused = true)]
async fn a_new_client_is_told_the_current_state_and_then_every_change_in_order() {
    serving(async |served| {
        served.states.publish(AssistantState::Thinking);
        let mut browser = Browser::connect(served.address).await;
        assert_eq!(browser.read().await, state(OrbState::Thinking));

        // Published back to back: none may be skipped or reordered.
        served.states.publish(AssistantState::Idle);
        served.states.publish(AssistantState::Listening);
        served.states.publish(AssistantState::Transcribing);
        served.states.publish(AssistantState::Thinking);
        for expected in [
            OrbState::Idle,
            OrbState::Listening,
            OrbState::Transcribing,
            OrbState::Thinking,
        ] {
            assert_eq!(browser.read().await, state(expected));
        }
        assert_eq!(browser.read().await, Read::Quiet);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_tap_reaches_the_tap_queue_once_and_a_second_one_waiting_is_dropped() {
    serving(async |served| {
        let mut browser = Browser::connect(served.address).await;
        assert_eq!(browser.read().await, state(OrbState::Idle));

        browser.send(TAP_TEXT).await;
        assert_eq!(timeout(WAIT, served.taps.recv()).await, Ok(Some(Tap)));
        assert!(served.taps.try_recv().is_err());

        // Three more before anyone handles one: the queue holds `TAP_QUEUE` (1).
        for _ in 0..3 {
            browser.send(TAP_TEXT).await;
        }
        assert_eq!(browser.read().await, Read::Quiet);
        assert_eq!(served.taps.try_recv(), Ok(Tap));
        assert!(served.taps.try_recv().is_err());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_bad_frame_is_ignored_and_the_connection_keeps_working() {
    let cases: [(&str, Message); 7] = [
        ("not JSON", Message::text("tap")),
        (
            "an unknown message",
            Message::text(r#"{"type":"shutdown"}"#),
        ),
        (
            "an unknown field",
            Message::text(r#"{"type":"tap","force":true}"#),
        ),
        (
            "a message meant for the orb",
            Message::text(r#"{"type":"state","state":"idle"}"#),
        ),
        ("an empty message", Message::text("")),
        ("a binary frame holding a tap", Message::binary(TAP_TEXT)),
        ("a ping", Message::Ping(Vec::new().into())),
    ];
    serving(async |served| {
        let mut browser = Browser::connect(served.address).await;
        assert_eq!(browser.read().await, state(OrbState::Idle));
        for (name, frame) in cases {
            browser.send_frame(frame).await;
            assert_eq!(browser.read().await, Read::Quiet, "{name}");
            assert!(served.taps.try_recv().is_err(), "{name}");
            // Still connected, and the next good message is read as itself.
            browser.send(TAP_TEXT).await;
            assert_eq!(
                timeout(WAIT, served.taps.recv()).await,
                Ok(Some(Tap)),
                "{name}"
            );
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_longest_message_is_accepted_and_one_byte_more_is_not() {
    // `padded_tap` ends its line; a WebSocket message has no newline.
    let message = |len| {
        let line = String::from_utf8(padded_tap(len)).unwrap();
        line.trim_end_matches('\n').to_owned()
    };
    let longest = message(MAX_LINE_BYTES);
    let too_long = message(MAX_LINE_BYTES + 1);
    assert_eq!(longest.len(), MAX_LINE_BYTES);
    assert_eq!(too_long.len(), MAX_LINE_BYTES + 1);

    serving(async |served| {
        let mut browser = Browser::connect(served.address).await;
        assert_eq!(browser.read().await, state(OrbState::Idle));

        browser.send(longest).await;
        assert_eq!(timeout(WAIT, served.taps.recv()).await, Ok(Some(Tap)));
        assert_eq!(browser.read().await, Read::Quiet);

        // Over the limit the message never reaches the tap queue and the
        // connection is closed.
        browser.send(too_long).await;
        assert_eq!(browser.read_until_closed().await, Read::Closed);
        assert!(served.taps.try_recv().is_err());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn two_clients_are_served_at_once_and_one_leaving_does_not_disturb_the_other() {
    serving(async |served| {
        let mut first = Browser::connect(served.address).await;
        let mut second = Browser::connect(served.address).await;
        assert_eq!(first.read().await, state(OrbState::Idle));
        assert_eq!(second.read().await, state(OrbState::Idle));

        served.states.publish(AssistantState::Thinking);
        assert_eq!(first.read().await, state(OrbState::Thinking));
        assert_eq!(second.read().await, state(OrbState::Thinking));

        drop(first);
        served.states.publish(AssistantState::Idle);
        assert_eq!(second.read().await, state(OrbState::Idle));
        second.send(TAP_TEXT).await;
        assert_eq!(timeout(WAIT, served.taps.recv()).await, Ok(Some(Tap)));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_client_that_reconnects_is_told_the_current_state_each_time() {
    serving(async |served| {
        for (round, assistant) in [
            AssistantState::Thinking,
            AssistantState::Speaking,
            AssistantState::Idle,
        ]
        .into_iter()
        .enumerate()
        {
            served.states.publish(assistant);
            let mut browser = Browser::connect(served.address).await;
            assert_eq!(
                browser.read().await,
                state(orb_state(assistant)),
                "round {round}"
            );
            drop(browser);
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_connection_limit_serves_its_last_client_and_refuses_the_next() {
    serving(async |served| {
        let mut browsers = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            let mut browser = Browser::connect(served.address).await;
            assert_eq!(browser.read().await, state(OrbState::Idle));
            browsers.push(browser);
        }
        assert!(Browser::try_connect(served.address).await.is_err());

        // A slot freed by a leaving client is given to the next one, once the
        // server has noticed that it left.
        browsers.pop();
        let mut replacement = None;
        for _ in 0..10 {
            if let Ok(browser) = Browser::try_connect(served.address).await {
                replacement = Some(browser);
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        let mut replacement = replacement.expect("the freed slot is given out");
        assert_eq!(replacement.read().await, state(OrbState::Idle));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_client_that_stops_reading_is_closed_and_the_others_carry_on() {
    serving(async |served| {
        // A tiny receive buffer, so that the levels back up into the server soon.
        let socket = TcpSocket::new_v4().unwrap();
        socket.set_recv_buffer_size(1024).unwrap();
        let stream = socket.connect(served.address).await.unwrap();
        let mut stuck = Browser::over(stream, served.address).await;
        let mut healthy = Browser::connect(served.address).await;
        assert_eq!(stuck.read().await, state(OrbState::Idle));
        assert_eq!(healthy.read().await, state(OrbState::Idle));

        // The stuck client reads nothing from here on.
        served.states.publish(AssistantState::Listening);
        assert_eq!(healthy.read().await, state(OrbState::Listening));
        for _ in 0..PILED_UP_LEVELS {
            assert!(matches!(healthy.read().await, Read::Message(_)));
        }
        // A change made while the stuck client is stuck still reaches the others.
        served.states.publish(AssistantState::Thinking);
        loop {
            match healthy.read().await {
                Read::Message(ToOrb::Level { .. }) => {}
                other => {
                    assert_eq!(other, state(OrbState::Thinking));
                    break;
                }
            }
        }

        // What was buffered for the stuck client ends in a close, not in more.
        let mut drained = 0;
        let end = loop {
            match stuck.read_until_closed().await {
                Read::Message(_) if drained < PILED_UP_LEVELS => drained += 1,
                other => break other,
            }
        };
        assert_eq!(end, Read::Closed);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_plain_http_request_is_refused_and_the_server_keeps_serving() {
    serving(async |served| {
        let mut stream = tokio::net::TcpStream::connect(served.address)
            .await
            .unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut reply = [0u8; 12];
        stream.read_exact(&mut reply).await.unwrap();
        // Not an upgrade: a client error, never a 200.
        assert_eq!(&reply[..10], b"HTTP/1.1 4");

        let mut browser = Browser::connect(served.address).await;
        assert_eq!(browser.read().await, state(OrbState::Idle));
    })
    .await;
}

/// The status the server answered the handshake with, if it refused it.
async fn refusal(address: std::net::SocketAddr, origin: Option<&str>) -> Option<u16> {
    match Browser::try_connect_from(address, origin).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            Some(response.status().as_u16())
        }
        Err(other) => panic!("{origin:?}: {other:?}"),
        Ok(_) => None,
    }
}

#[tokio::test(start_paused = true)]
async fn only_a_page_from_an_allowed_origin_gets_a_websocket() {
    // (name, the Origin header, whether the handshake is refused with a 403)
    let cases = [
        ("the allowed origin", Some(ALLOWED_ORIGIN), false),
        (
            "the allowed origin in capitals",
            Some("HTTP://LOCALHOST:8766"),
            false,
        ),
        ("another host", Some("http://evil.example"), true),
        ("another port", Some("http://localhost:8767"), true),
        ("another scheme", Some("https://localhost:8766"), true),
        (
            "the allowed origin with a path",
            Some("http://localhost:8766/"),
            true,
        ),
        ("opaque, as a page from disk sends", Some("null"), true),
        ("no Origin at all", None, true),
    ];
    serving(async |served| {
        for (name, origin, refused) in cases {
            let status = refusal(served.address, origin).await;
            assert_eq!(status, refused.then_some(403), "{name}");
        }
        // The refusals left nothing behind: a good client is still served.
        let mut browser = Browser::connect(served.address).await;
        assert_eq!(browser.read().await, state(OrbState::Idle));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_refused_origin_does_not_use_up_a_connection_slot() {
    serving(async |served| {
        for _ in 0..=MAX_CONNECTIONS {
            assert_eq!(
                refusal(served.address, Some("http://evil.example")).await,
                Some(403)
            );
        }
        let mut browsers = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            browsers.push(Browser::connect(served.address).await);
        }
    })
    .await;
}
