//! The one source of everything the orb is sent, whichever door it comes through.
//!
//! The server publishes each message here once; every connected orb, on any
//! transport, reads its own copy through an [`Outbox`]. Each orb has its own
//! bounded queues, so a slow one costs the others nothing.
//!
//! Two kinds of message, two policies:
//!
//! - **Events** (state changes and whatever features send) are never dropped
//!   while there is room. An orb that falls [`EVENT_QUEUE`] events behind is cut
//!   off instead: it reconnects and is told the current state, which is better
//!   than an orb quietly showing a state that is no longer true.
//! - **Levels** are only worth the newest. An orb that falls [`LEVEL_QUEUE`]
//!   levels behind loses the oldest ones.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use orb_protocol::{OrbState, ToOrb};
use tokio::sync::broadcast::{self, error::RecvError};

/// Events one orb may have waiting before it is cut off. A power of two, which
/// is what the queue holds (the channel rounds up).
pub(super) const EVENT_QUEUE: usize = 64;
/// Levels one orb may have waiting before the oldest are dropped. A power of two.
/// At 30 levels a second this is a quarter of a second of loudness.
pub(super) const LEVEL_QUEUE: usize = 8;

/// What the hub keeps, behind one lock so that a new orb's first message and
/// its place in the event queue agree.
struct Shared {
    /// The newest state published. Told to every orb as it connects.
    state: OrbState,
    events: broadcast::Sender<ToOrb>,
    levels: broadcast::Sender<ToOrb>,
}

/// Owned by the server. Dropping it ends every [`Outbox`].
pub(super) struct Hub {
    shared: Arc<Mutex<Shared>>,
}

/// How a transport reaches the hub without keeping it alive. Cloneable and
/// `'static`, so it can go into tasks that outlive any borrow.
#[derive(Clone)]
pub(super) struct HubHandle {
    shared: Weak<Mutex<Shared>>,
}

/// An orb is gone for the hub's purposes: the server stopped, or this orb fell
/// too far behind to be kept consistent. Either way its connection should end.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Gone;

impl Hub {
    /// A hub whose current state is `state`.
    pub(super) fn new(state: OrbState) -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared {
                state,
                events: broadcast::channel(EVENT_QUEUE).0,
                levels: broadcast::channel(LEVEL_QUEUE).0,
            })),
        }
    }

    pub(super) fn handle(&self) -> HubHandle {
        HubHandle {
            shared: Arc::downgrade(&self.shared),
        }
    }

    /// Sends `message` to every connected orb. Never waits; nobody connected is
    /// normal.
    ///
    /// A level is a level; anything else is an event, and a state also becomes
    /// the state new orbs are told. Sorting by "is it a level" rather than by
    /// matching every variant lets new message kinds travel as events without
    /// this module changing.
    pub(super) fn publish(&self, message: ToOrb) {
        let mut shared = lock(&self.shared);
        if matches!(message, ToOrb::Level { .. }) {
            let _ = shared.levels.send(message);
            return;
        }
        if let ToOrb::State { state } = &message {
            shared.state = *state;
        }
        let _ = shared.events.send(message);
    }

    /// A new orb's queues. Its first message is the current state.
    pub(super) fn subscribe(&self) -> Outbox {
        Outbox::new(&lock(&self.shared))
    }
}

impl HubHandle {
    /// Like [`Hub::subscribe`], or `None` once the hub is gone.
    pub(super) fn subscribe(&self) -> Option<Outbox> {
        let shared = self.shared.upgrade()?;
        Some(Outbox::new(&lock(&shared)))
    }
}

/// The lock guards plain data and nothing under it can panic, so a poisoned
/// lock is still consistent.
fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One orb's view of the hub: what to send it next.
pub(super) struct Outbox {
    /// The state at the moment of subscribing, until it has been handed out.
    first: Option<ToOrb>,
    events: broadcast::Receiver<ToOrb>,
    levels: broadcast::Receiver<ToOrb>,
}

impl Outbox {
    fn new(shared: &Shared) -> Self {
        Self {
            first: Some(ToOrb::State {
                state: shared.state,
            }),
            events: shared.events.subscribe(),
            levels: shared.levels.subscribe(),
        }
    }

    /// The next message for this orb: the current state first, then events in
    /// order, and levels whenever no event is waiting.
    ///
    /// A state also clears the levels still queued: they were measured in the
    /// state before and must not follow the change. Safe to drop at any `.await`.
    pub(super) async fn next(&mut self) -> Result<ToOrb, Gone> {
        if let Some(first) = self.first.take() {
            return Ok(first);
        }
        loop {
            tokio::select! {
                // Events first: a level never overtakes a state change.
                biased;
                event = self.events.recv() => match event {
                    Ok(message) => {
                        if matches!(message, ToOrb::State { .. }) {
                            self.levels = self.levels.resubscribe();
                        }
                        return Ok(message);
                    }
                    Err(RecvError::Lagged(_) | RecvError::Closed) => return Err(Gone),
                },
                level = self.levels.recv() => match level {
                    Ok(message) => return Ok(message),
                    // Only the newest levels matter: skip to them.
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return Err(Gone),
                },
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/interfaces/orb/hub_tests.rs"]
mod tests;
