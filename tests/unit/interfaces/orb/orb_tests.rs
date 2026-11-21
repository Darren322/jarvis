use std::{net::SocketAddr, time::Duration};

use futures_util::{SinkExt, StreamExt};
use tokio::time::{Instant, timeout};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, client_async, connect_async,
    tungstenite::{self, Message, client::IntoClientRequest, http::HeaderValue},
};

use super::*;

/// Long enough for anything the server will ever send; the clock is paused, so
/// waiting it out costs nothing.
pub(super) const WAIT: Duration = Duration::from_secs(2);
/// Times the browser waits `WAIT` for a frame before it counts as not coming.
const READ_ATTEMPTS: usize = 3;
/// Levels sent while one client is not reading: enough to overfill its socket buffer.
pub(super) const PILED_UP_LEVELS: usize = 20_000;

/// The one origin the test server allows, as the UI's would be.
pub(super) const ALLOWED_ORIGIN: &str = "http://localhost:8766";

/// What a test holds while the server runs beside it.
pub(super) struct Served<'a> {
    pub(super) address: SocketAddr,
    pub(super) states: &'a StateFeed,
    pub(super) mic: &'a watch::Sender<f32>,
    pub(super) taps: &'a mut mpsc::Receiver<Tap>,
    pub(super) handle: &'a mut OrbHandle,
}

/// A camera feed whose camera Panel is never shown, so `/camera` refuses every
/// request.
fn no_camera() -> crate::features::camera::CameraFeed {
    let (panels, _no_panel_state) = crate::features::panel::channel();
    crate::features::camera::Camera::new("unused", panels).1
}

/// Runs `body` beside a server on a free localhost port, with [`no_camera`].
pub(super) async fn serving<T>(body: impl AsyncFnOnce(Served<'_>) -> T) -> T {
    serving_with(no_camera(), body).await
}

/// [`serving`], with `/camera` reading `camera`.
pub(super) async fn serving_with<T>(
    camera: crate::features::camera::CameraFeed,
    body: impl AsyncFnOnce(Served<'_>) -> T,
) -> T {
    let states = StateFeed::new();
    let (mic, mic_level) = watch::channel(0.0);
    let (tap_sender, mut taps) = mpsc::channel(TAP_QUEUE);
    let websocket = WebSocketEndpoint::bind(0).unwrap();
    let address = websocket.local_addr().unwrap();
    let (server, mut handle) = OrbServer::new(websocket, vec![ALLOWED_ORIGIN.to_owned()]);
    let feeds = OrbFeeds {
        states: &states,
        mic_level,
        speech_level: None,
        taps: tap_sender,
        camera,
    };
    let served = Served {
        address,
        states: &states,
        mic: &mic,
        taps: &mut taps,
        handle: &mut handle,
    };
    tokio::select! {
        never = server.serve(feeds) => match never {},
        result = body(served) => result,
    }
}

/// What a client's next read gave.
#[derive(Debug, PartialEq)]
pub(super) enum Read {
    Message(ToOrb),
    /// Nothing arrived within `WAIT`.
    Quiet,
    /// The server closed the connection.
    Closed,
}

pub(super) fn state(state: OrbState) -> Read {
    Read::Message(ToOrb::State { state })
}

/// A valid tap padded with spaces to exactly `len` bytes before its newline.
pub(super) fn padded_tap(len: usize) -> Vec<u8> {
    let mut line = b"{\"type\":\"tap\"}".to_vec();
    line.resize(len, b' ');
    line.push(b'\n');
    line
}

#[test]
fn each_assistant_state_has_its_orb_state() {
    let cases = [
        (AssistantState::Idle, OrbState::Idle),
        (AssistantState::Listening, OrbState::Listening),
        (AssistantState::Transcribing, OrbState::Transcribing),
        (AssistantState::Thinking, OrbState::Thinking),
        (AssistantState::Speaking, OrbState::Speaking),
    ];
    for (assistant, orb) in cases {
        assert_eq!(orb_state(assistant), orb);
    }
}

/// Read straight from the hub's queue rather than over a connection: the paused
/// clock cannot tell whether a frame is still travelling over loopback, so the
/// arrival times of a socket read are not the times the levels were sent.
#[tokio::test(start_paused = true)]
async fn the_mic_level_is_sent_thirty_times_a_second_while_listening() {
    let states = StateFeed::new();
    let (_mic, mic_level) = watch::channel(0.25);
    let (tap_sender, _taps) = mpsc::channel(TAP_QUEUE);
    let (_to_orb, messages) = mpsc::channel(TO_ORB_QUEUE);
    let feeds = OrbFeeds {
        states: &states,
        mic_level,
        speech_level: None,
        taps: tap_sender,
        camera: no_camera(),
    };
    let hub = Hub::new(OrbState::Idle);
    let mut outbox = hub.handle().subscribe().unwrap();
    let observer = states.observe();

    let arrivals = tokio::select! {
        never = publish_everything(&hub, observer, messages, &feeds) => match never {},
        arrivals = async {
            assert_eq!(outbox.next().await, Ok(ToOrb::State { state: OrbState::Idle }));
            states.publish(AssistantState::Listening);
            assert_eq!(outbox.next().await, Ok(ToOrb::State { state: OrbState::Listening }));
            let mut arrivals = Vec::new();
            while arrivals.len() < 31 {
                assert_eq!(
                    outbox.next().await,
                    Ok(ToOrb::Level {
                        level: Level::new(0.25).unwrap(),
                        source: LevelSource::Mic,
                    })
                );
                arrivals.push(Instant::now());
            }
            arrivals
        } => arrivals,
    };
    // Never faster than the cadence, and 30 intervals take 30 x 33 ms.
    for pair in arrivals.windows(2) {
        assert_eq!(pair[1] - pair[0], LEVEL_INTERVAL);
    }
    assert_eq!(arrivals[30] - arrivals[0], Duration::from_millis(990));
}

#[tokio::test(start_paused = true)]
async fn levels_carry_the_source_of_the_state_and_stop_when_there_is_nothing_to_hear() {
    serving(async |served| {
        let mut orb = Browser::connect(served.address).await;
        assert_eq!(orb.read().await, state(OrbState::Idle));
        served.mic.send_replace(0.5);
        // (state, the source its levels carry)
        let cases = [
            (AssistantState::Listening, Some(LevelSource::Mic)),
            (AssistantState::Transcribing, None),
            (AssistantState::Speaking, Some(LevelSource::Speech)),
            (AssistantState::Thinking, None),
            (AssistantState::Idle, None),
        ];
        for (assistant, source) in cases {
            served.states.publish(assistant);
            // A level for the state before may already be on its way.
            let after_state = loop {
                match orb.read().await {
                    Read::Message(ToOrb::State { state }) => {
                        assert_eq!(state, orb_state(assistant));
                        break orb.read().await;
                    }
                    Read::Message(ToOrb::Level { .. }) => {}
                    other => panic!("{assistant:?}: {other:?}"),
                }
            };
            match source {
                Some(source) => match after_state {
                    Read::Message(ToOrb::Level { source: sent, .. }) => assert_eq!(sent, source),
                    other => panic!("{assistant:?}: {other:?}"),
                },
                None => assert_eq!(after_state, Read::Quiet, "{assistant:?}"),
            }
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_level_outside_the_range_is_clamped_before_it_is_sent() {
    serving(async |served| {
        let mut orb = Browser::connect(served.address).await;
        assert_eq!(orb.read().await, state(OrbState::Idle));
        served.mic.send_replace(7.0);
        served.states.publish(AssistantState::Listening);
        assert_eq!(orb.read().await, state(OrbState::Listening));
        assert_eq!(
            orb.read().await,
            Read::Message(ToOrb::Level {
                level: Level::new(1.0).unwrap(),
                source: LevelSource::Mic,
            })
        );
    })
    .await;
}

fn request(address: SocketAddr, origin: Option<&str>) -> tungstenite::handshake::client::Request {
    let mut request = format!("ws://{address}/").into_client_request().unwrap();
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert("Origin", HeaderValue::from_str(origin).unwrap());
    }
    request
}

/// A stand-in for the UI in Chromium.
pub(super) struct Browser {
    socket: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
}

impl Browser {
    pub(super) async fn connect(address: SocketAddr) -> Self {
        Self::try_connect(address).await.unwrap()
    }

    /// Connects from the allowed origin, as the UI does.
    pub(super) async fn try_connect(address: SocketAddr) -> Result<Self, tungstenite::Error> {
        Self::try_connect_from(address, Some(ALLOWED_ORIGIN)).await
    }

    /// Connects with the given `Origin` header, or none.
    pub(super) async fn try_connect_from(
        address: SocketAddr,
        origin: Option<&str>,
    ) -> Result<Self, tungstenite::Error> {
        let (socket, _) = connect_async(request(address, origin)).await?;
        Ok(Self { socket })
    }

    /// Connects over a socket the test has set up.
    pub(super) async fn over(stream: tokio::net::TcpStream, address: SocketAddr) -> Self {
        let (socket, _) = client_async(
            request(address, Some(ALLOWED_ORIGIN)),
            MaybeTlsStream::Plain(stream),
        )
        .await
        .unwrap();
        Self { socket }
    }

    /// The next frame, or `None` when nothing came in `READ_ATTEMPTS` waits.
    ///
    /// With the clock paused, the runtime may jump ahead before the system has
    /// handed a frame that is already on its way over the loopback connection to
    /// the client. Asking again then gets it; asking again when nothing is coming
    /// only costs virtual time.
    async fn next_frame(&mut self) -> Option<Option<Result<Message, tungstenite::Error>>> {
        for _ in 0..READ_ATTEMPTS {
            if let Ok(frame) = timeout(WAIT, self.socket.next()).await {
                return Some(frame);
            }
        }
        None
    }

    /// Every message must be one JSON object with no newline.
    pub(super) async fn read(&mut self) -> Read {
        loop {
            match self.next_frame().await {
                None => return Read::Quiet,
                Some(Some(Ok(Message::Text(text)))) => {
                    assert!(!text.contains('\n'), "{text:?} carries a newline");
                    return Read::Message(ToOrb::decode(text.as_str()).unwrap());
                }
                Some(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => {}
                Some(Some(Ok(other))) => panic!("unexpected frame {other:?}"),
                Some(Some(Err(_)) | None) => return Read::Closed,
            }
        }
    }

    /// Like `read`, but a close frame, which the server sends when it stops, also
    /// counts as closed.
    pub(super) async fn read_until_closed(&mut self) -> Read {
        loop {
            match self.next_frame().await {
                None => return Read::Quiet,
                Some(Some(Ok(Message::Close(_))) | Some(Err(_)) | None) => return Read::Closed,
                Some(Some(Ok(Message::Text(text)))) => {
                    return Read::Message(ToOrb::decode(text.as_str()).unwrap());
                }
                Some(Some(Ok(_))) => {}
            }
        }
    }

    pub(super) async fn send(&mut self, text: impl Into<tungstenite::Utf8Bytes>) {
        self.send_frame(Message::Text(text.into())).await;
    }

    pub(super) async fn send_frame(&mut self, frame: Message) {
        self.socket.send(frame).await.unwrap();
    }
}

fn level(value: f32, source: LevelSource) -> ToOrb {
    ToOrb::Level {
        level: Level::new(value).unwrap(),
        source,
    }
}

#[tokio::test(start_paused = true)]
async fn a_state_change_reaches_every_websocket_client_alike() {
    serving(async |served| {
        let mut first = Browser::connect(served.address).await;
        let mut second = Browser::connect(served.address).await;
        assert_eq!(first.read().await, state(OrbState::Idle));
        assert_eq!(second.read().await, state(OrbState::Idle));

        for assistant in [AssistantState::Thinking, AssistantState::Speaking] {
            served.states.publish(assistant);
            assert_eq!(first.read().await, state(orb_state(assistant)));
            assert_eq!(second.read().await, state(orb_state(assistant)));
        }
        // Speech is not configured here, so the level is silence, from the same source.
        let expected = Read::Message(level(0.0, LevelSource::Speech));
        assert_eq!(first.read().await, expected);
        assert_eq!(second.read().await, expected);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_tap_from_any_websocket_client_goes_to_the_same_tap_queue() {
    serving(async |served| {
        let mut first = Browser::connect(served.address).await;
        let mut second = Browser::connect(served.address).await;
        assert_eq!(first.read().await, state(OrbState::Idle));
        assert_eq!(second.read().await, state(OrbState::Idle));

        first.send(r#"{"type":"tap"}"#).await;
        assert_eq!(timeout(WAIT, served.taps.recv()).await, Ok(Some(Tap)));
        second.send(r#"{"type":"tap"}"#).await;
        assert_eq!(timeout(WAIT, served.taps.recv()).await, Ok(Some(Tap)));
        // A tap is not something a feature has to hear about as well.
        assert!(served.handle.from_orb.try_recv().is_err());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_message_from_a_feature_reaches_every_client_and_is_remembered() {
    serving(async |served| {
        let mut first = Browser::connect(served.address).await;
        let mut second = Browser::connect(served.address).await;
        assert_eq!(first.read().await, state(OrbState::Idle));
        assert_eq!(second.read().await, state(OrbState::Idle));

        let message = ToOrb::State {
            state: OrbState::Thinking,
        };
        served.handle.to_orb.send(message.clone()).await.unwrap();
        assert_eq!(first.read().await, Read::Message(message.clone()));
        assert_eq!(second.read().await, Read::Message(message));

        // An orb that connects later is told the state the feature set.
        let mut late = Browser::connect(served.address).await;
        assert_eq!(late.read().await, state(OrbState::Thinking));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn stopping_the_server_closes_the_websocket_clients_and_frees_the_port() {
    let mut browser = None;
    let address = serving(async |served| {
        let mut connected = Browser::connect(served.address).await;
        assert_eq!(connected.read().await, state(OrbState::Idle));
        browser = Some(connected);
        served.address
    })
    .await;
    let mut browser = browser.unwrap();
    assert_eq!(browser.read_until_closed().await, Read::Closed);
    // Nobody is listening on the port any more.
    assert!(Browser::try_connect(address).await.is_err());
}

#[tokio::test]
async fn a_port_already_in_use_is_reported_once_and_serves_once_it_is_free() {
    let occupant = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = occupant.local_addr().unwrap().port();
    let config = OrbConfig {
        port: std::num::NonZeroU16::new(port).unwrap(),
        allowed_origins: vec![ALLOWED_ORIGIN.to_owned()],
    };

    let Err(error) = OrbServer::bind(&config) else {
        panic!("the port is taken");
    };
    assert!(matches!(error, OrbError::WebSocketBind { port: bound, .. } if bound == port));
    // The other program still has its port.
    assert!(occupant.local_addr().is_ok());

    // Once the port is free the same configuration serves.
    drop(occupant);
    let (server, _handle) = OrbServer::bind(&config).unwrap();
    drop(server);
}

#[tokio::test(start_paused = true)]
async fn a_websocket_client_that_leaves_closes_the_panel() {
    serving(async |served| {
        let mut browser = Browser::connect(served.address).await;
        assert_eq!(browser.read().await, state(OrbState::Idle));
        assert!(served.handle.from_orb.try_recv().is_err(), "connected");

        drop(browser);
        assert_eq!(
            timeout(WAIT, served.handle.from_orb.recv()).await,
            Ok(Some(FromOrb::PanelClosed {}))
        );
        assert!(served.taps.try_recv().is_err());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_panel_closed_from_a_websocket_client_reaches_the_handle_and_not_the_tap_queue() {
    serving(async |served| {
        let mut browser = Browser::connect(served.address).await;
        assert_eq!(browser.read().await, state(OrbState::Idle));

        browser.send(r#"{"type":"panel_closed"}"#).await;
        assert_eq!(
            timeout(WAIT, served.handle.from_orb.recv()).await,
            Ok(Some(FromOrb::PanelClosed {}))
        );
        assert!(served.taps.try_recv().is_err());
    })
    .await;
}
