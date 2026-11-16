//! The WebSocket door, for the UI in Chromium. It listens on `127.0.0.1` only and
//! serves one route, `/`, which upgrades to a WebSocket. Each text message in
//! either direction is one `orb-protocol` JSON object, with no newline.
//!
//! A browser lets any web page open a WebSocket to localhost, and a tap starts the
//! microphone, so the handshake is refused unless the page's `Origin` is one of
//! the configured ones. A request with no `Origin`, or the opaque `null`, is
//! refused too: the UI is served from `http://localhost`.
//!
//! `axum` runs the HTTP handshake. Each connection is a task that `axum`
//! spawns, so it can outlive the serve future: it ends as soon as the hub is
//! gone (see [`Outbox::next`]) or the client leaves.

use std::{
    convert::Infallible,
    io,
    net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener},
    sync::Arc,
};

use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, header::ORIGIN},
    response::{IntoResponse, Response},
    routing::get,
};
use orb_protocol::{FromOrb, MAX_LINE_BYTES, ToOrb};
use tokio::{
    net::TcpListener,
    sync::{OwnedSemaphorePermit, Semaphore},
    time::timeout,
};

use super::{
    Inbox, MAX_CONNECTIONS, OrbError, WRITE_TIMEOUT,
    hub::{Gone, HubHandle, Outbox},
};

/// The bound port. Dropping it (or the `serve` future) stops accepting; the
/// connections already open end with the hub.
pub(super) struct WebSocketEndpoint {
    listener: TcpListener,
}

/// What every connection handler shares.
struct Shared {
    hub: HubHandle,
    inbox: Inbox,
    /// Lowercase `scheme://host[:port]`, as `OrbConfig` normalises them.
    allowed_origins: Vec<String>,
    /// One permit per connection, held until it ends.
    slots: Arc<Semaphore>,
}

impl WebSocketEndpoint {
    /// Listens on `127.0.0.1:port`. Only this machine can connect: the page that
    /// uses it is opened from disk by a browser on the same Pi.
    ///
    /// Port 0 asks the system for a free one (see [`WebSocketEndpoint::local_addr`]);
    /// the configuration never produces it. Must be called inside the Tokio
    /// runtime.
    pub(super) fn bind(port: u16) -> Result<Self, OrbError> {
        let bound = |source| OrbError::WebSocketBind { port, source };
        let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(bound)?;
        listener.set_nonblocking(true).map_err(bound)?;
        let listener = TcpListener::from_std(listener).map_err(bound)?;
        Ok(Self { listener })
    }

    /// The address actually bound.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "only tests need the port the system chose")
    )]
    pub(super) fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Serves clients until dropped. It never completes and never fails: a
    /// connection that breaks is closed and the next one is accepted.
    pub(super) async fn serve(
        self,
        hub: HubHandle,
        inbox: Inbox,
        allowed_origins: Vec<String>,
    ) -> Infallible {
        let shared = Arc::new(Shared {
            hub,
            inbox,
            allowed_origins,
            slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
        });
        let app = Router::new().route("/", get(upgrade)).with_state(shared);
        // `axum::serve` retries failed accepts itself and only returns on an error
        // that cannot be fixed by trying again; there is nothing left to serve then.
        let _ = axum::serve(self.listener, app).await;
        std::future::pending().await
    }
}

/// The handshake. A page from an origin that is not allowed gets a 403. Over the
/// connection limit, or with the server stopping, the client is told to come back
/// later.
async fn upgrade(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !is_allowed(&headers, &shared.allowed_origins) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(permit) = Arc::clone(&shared.slots).try_acquire_owned() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    // Subscribing here, before the upgrade completes, fixes the state the client
    // is told first; whatever changes after that waits in its queue.
    let Some(outbox) = shared.hub.subscribe() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let inbox = shared.inbox.clone();
    // A message over the limit makes the connection fail at the transport, with
    // nothing buffered beyond the limit.
    ws.max_message_size(MAX_LINE_BYTES)
        .max_frame_size(MAX_LINE_BYTES)
        .on_upgrade(move |socket| serve_client(socket, outbox, inbox, permit))
}

/// Whether the request names one of the `allowed` origins. Browsers write the
/// scheme and host in lowercase; anything else is lowercased before comparing.
/// No origin, an unreadable one and `null` are never allowed.
fn is_allowed(headers: &HeaderMap, allowed: &[String]) -> bool {
    let Some(origin) = headers.get(ORIGIN).and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let origin = origin.to_ascii_lowercase();
    allowed.contains(&origin)
}

/// One client, from its handshake to its last frame. Returns when the client
/// goes away, stops reading, or the hub ends.
async fn serve_client(
    mut socket: WebSocket,
    mut outbox: Outbox,
    inbox: Inbox,
    _slot: OwnedSemaphorePermit,
) {
    loop {
        let message = tokio::select! {
            // The user's tap first, then what the client is to be shown.
            biased;
            frame = socket.recv() => match frame {
                Some(Ok(Message::Text(text))) => {
                    // Not a message we know: ignored, the connection stays open.
                    if let Ok(message) = FromOrb::decode(text.as_str()) {
                        inbox.deliver(message);
                    }
                    continue;
                }
                // Only text carries the protocol. Ping and pong are answered by
                // the transport.
                Some(Ok(Message::Binary(_) | Message::Ping(_) | Message::Pong(_))) => continue,
                Some(Ok(Message::Close(_)) | Err(_)) | None => return,
            },
            next = outbox.next() => match next {
                Ok(message) => message,
                Err(Gone) => {
                    // A polite goodbye if the client is listening; the page then
                    // reconnects on its own. Not worth waiting for.
                    let _ = timeout(WRITE_TIMEOUT, socket.send(Message::Close(None))).await;
                    return;
                }
            },
        };
        if send(&mut socket, message).await.is_err() {
            return;
        }
    }
}

/// The client did not take a message: it is gone, or stuck.
struct ClientStuck;

async fn send(socket: &mut WebSocket, message: ToOrb) -> Result<(), ClientStuck> {
    // Every `ToOrb` is far under the limit, so encoding cannot fail; if it ever
    // did, closing the connection is the safe answer. `encode` ends the line with
    // a newline that a WebSocket message does not carry.
    let line = message.encode().map_err(|_| ClientStuck)?;
    let text = line.trim_end_matches('\n');
    match timeout(WRITE_TIMEOUT, socket.send(Message::text(text))).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) | Err(_) => Err(ClientStuck),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/interfaces/orb/websocket_tests.rs"]
mod tests;
