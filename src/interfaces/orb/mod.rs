//! The orb's side door into Jarvis: the screen that shows what Jarvis is doing.
//!
//! Jarvis is the server. It sends the assistant's state on every change and a
//! loudness about 30 times a second while listening or speaking, and it reads
//! taps. Messages are `orb-protocol` JSON. One door leads in, the [`websocket`]
//! for the UI in Chromium, on `127.0.0.1` and for allowed origins only. Every
//! client it serves gets the same messages from the same source (the [`hub`]).
//!
//! This interface shares only `features/` with the terminal: it is handed a
//! [`StateFeed`] observer, two loudness readers and the sending half of a tap
//! queue, and knows nothing about who is on the other end of them. What a tap
//! means is decided by whoever reads that queue, from the real state
//! (`features::control::tap_action`); here a tap is only passed on.
//!
//! Features that need the orb for more than state and loudness (panels) use the
//! [`OrbHandle`]: they send any `ToOrb` and read any `FromOrb` other than a tap,
//! without this module learning what the messages mean.
//!
//! The orb is optional. No orb connected, an orb that disconnects, and an orb
//! that sends rubbish are all normal and silent: orb input is untrusted, and the
//! only thing it can cause is one `Tap` in a bounded queue or one event in a
//! bounded handle queue.

mod hub;
mod websocket;

use std::{convert::Infallible, io, time::Duration};

use orb_protocol::{FromOrb, Level, LevelSource, OrbState, ToOrb};
use thiserror::Error;
use tokio::{
    sync::{mpsc, watch},
    time::MissedTickBehavior,
};

use self::{hub::Hub, websocket::WebSocketEndpoint};
use crate::{
    config::OrbConfig,
    features::{
        control::Tap,
        listening::LEVEL_INTERVAL,
        speech::LevelReceiver,
        state::{AssistantState, StateFeed, StateObserver},
    },
};

/// Orbs served at once. One screen is expected; the spare slots let
/// a restarted orb connect before its old connection is noticed as gone.
const MAX_CONNECTIONS: usize = 4;
/// How long one message may take to write before the orb counts as stuck and its
/// connection is closed. The orb reconnects by itself.
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);
/// Taps waiting to be handled. A tap is a request to act now, so a second one
/// arriving before the first is handled adds nothing and is dropped.
pub(crate) const TAP_QUEUE: usize = 1;
/// Messages from features waiting to be sent to the orbs. A feature that sends
/// faster than the hub takes them waits.
const TO_ORB_QUEUE: usize = 16;
/// Messages from the orbs (other than taps) waiting for a feature to read them.
/// When a feature is not reading, the newest are dropped: the orb must never be
/// able to make Jarvis wait.
const FROM_ORB_QUEUE: usize = 16;

#[derive(Debug, Error)]
pub(crate) enum OrbError {
    #[error("could not listen on 127.0.0.1:{port} (is another program using it?)")]
    WebSocketBind {
        port: u16,
        #[source]
        source: io::Error,
    },
}

/// What the server shows the orb and where its taps go.
pub(crate) struct OrbFeeds<'a> {
    pub(crate) states: &'a StateFeed,
    /// Microphone loudness, 0 to 1; sent while listening.
    pub(crate) mic_level: watch::Receiver<f32>,
    /// Loudness of Jarvis's voice; `None` when speech is not configured.
    pub(crate) speech_level: Option<LevelReceiver>,
    pub(crate) taps: mpsc::Sender<Tap>,
}

/// How a feature talks to the orbs, apart from state, loudness and taps.
pub(crate) struct OrbHandle {
    /// Sent to every connected orb as an event: in order, and not dropped while
    /// an orb has room. Sending waits when the queue to the hub is full.
    pub(crate) to_orb: mpsc::Sender<ToOrb>,
    /// What the orbs send that is not a tap. Dropped when full, never waited for.
    pub(crate) from_orb: mpsc::Receiver<FromOrb>,
}

/// Where what an orb sends goes.
#[derive(Clone)]
struct Inbox {
    taps: mpsc::Sender<Tap>,
    events: mpsc::Sender<FromOrb>,
}

impl Inbox {
    /// Passes one message from an orb on. Never waits. A full queue means the same
    /// thing is already waiting; a closed one means nobody acts on it. Neither is
    /// the orb's problem.
    ///
    /// Taps are told apart from the rest with `matches!`, so a new kind of message
    /// from the orb reaches the handle without this module changing.
    fn deliver(&self, message: FromOrb) {
        if matches!(message, FromOrb::Tap {}) {
            let _ = self.taps.try_send(Tap);
        } else {
            let _ = self.events.try_send(message);
        }
    }
}

/// The WebSocket door, bound. Dropping it (or the `serve` future) closes every
/// connection and frees the port.
pub(crate) struct OrbServer {
    websocket: WebSocketEndpoint,
    /// The origins the WebSocket accepts (see `OrbConfig::allowed_origins`).
    allowed_origins: Vec<String>,
    /// The other end of [`OrbHandle::to_orb`].
    messages: mpsc::Receiver<ToOrb>,
    /// The other end of [`OrbHandle::from_orb`].
    events: mpsc::Sender<FromOrb>,
}

impl OrbServer {
    /// Opens the WebSocket port. If it cannot be opened the failure is reported
    /// and Jarvis runs without an orb. Must be called inside the Tokio runtime.
    pub(crate) fn bind(config: &OrbConfig) -> Result<(Self, OrbHandle), OrbError> {
        let websocket = WebSocketEndpoint::bind(config.port.get())?;
        Ok(Self::new(websocket, config.allowed_origins.clone()))
    }

    fn new(websocket: WebSocketEndpoint, allowed_origins: Vec<String>) -> (Self, OrbHandle) {
        let (to_orb, messages) = mpsc::channel(TO_ORB_QUEUE);
        let (events, from_orb) = mpsc::channel(FROM_ORB_QUEUE);
        let server = Self {
            websocket,
            allowed_origins,
            messages,
            events: events.clone(),
        };
        (server, OrbHandle { to_orb, from_orb })
    }

    /// Serves orbs until dropped. It never completes and never fails.
    ///
    /// Everything runs inside this future (the WebSocket's connections are tasks
    /// of `axum`, but they end with the hub, which dies with this future), so
    /// dropping it at any `.await` closes them all and loses nothing.
    pub(crate) async fn serve(self, feeds: OrbFeeds<'_>) -> Infallible {
        let Self {
            websocket,
            allowed_origins,
            messages,
            events,
        } = self;
        let states = feeds.states.observe();
        let hub = Hub::new(orb_state(states.current()));
        let inbox = Inbox {
            taps: feeds.taps.clone(),
            events,
        };
        // Both never complete. `biased` makes the hub hear of a state change
        // before it accepts an orb that connects at the same moment, so that the
        // orb is told the newest state first rather than the one before.
        tokio::select! {
            biased;
            never = publish_everything(&hub, states, messages, &feeds) => match never {},
            never = websocket.serve(hub.handle(), inbox, allowed_origins) => match never {},
        }
    }
}

/// Feeds the hub: every state change, the features' messages, and a loudness on
/// each tick while there is something to hear.
async fn publish_everything(
    hub: &Hub,
    mut states: StateObserver,
    mut messages: mpsc::Receiver<ToOrb>,
    feeds: &OrbFeeds<'_>,
) -> Infallible {
    let mut ticks = tokio::time::interval(LEVEL_INTERVAL);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            // Messages in the order they were made: features, state, then loudness.
            biased;
            // `None` means every handle sender is gone; the branch then stays off.
            Some(message) = messages.recv() => hub.publish(message),
            state = states.next() => hub.publish(ToOrb::State { state: orb_state(state) }),
            _ = ticks.tick() => {
                let Some(source) = level_source(states.current()) else {
                    continue;
                };
                let level = match source {
                    LevelSource::Mic => *feeds.mic_level.borrow(),
                    LevelSource::Speech => feeds
                        .speech_level
                        .as_ref()
                        .map_or(0.0, LevelReceiver::level),
                };
                hub.publish(ToOrb::Level { level: Level::clamped(level), source });
            }
        }
    }
}

fn orb_state(state: AssistantState) -> OrbState {
    match state {
        AssistantState::Idle => OrbState::Idle,
        AssistantState::Listening => OrbState::Listening,
        AssistantState::Transcribing => OrbState::Transcribing,
        AssistantState::Thinking => OrbState::Thinking,
        AssistantState::Speaking => OrbState::Speaking,
    }
}

/// Whose loudness the orb should see in `state`, if anyone's.
fn level_source(state: AssistantState) -> Option<LevelSource> {
    match state {
        AssistantState::Listening => Some(LevelSource::Mic),
        AssistantState::Speaking => Some(LevelSource::Speech),
        AssistantState::Idle | AssistantState::Transcribing | AssistantState::Thinking => None,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/interfaces/orb/orb_tests.rs"]
mod tests;
