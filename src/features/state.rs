use tokio::sync::{broadcast, watch};

/// What the assistant is doing right now, for any front end to show.
///
/// Interface-neutral: the terminal prints it, and a later animation can draw it.
/// It is an enum so that a display can add its own dimension (colour, emotion)
/// without parsing text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AssistantState {
    Idle,
    Listening,
    Transcribing,
    Thinking,
    Speaking,
}

/// State changes a late observer may fall behind by before it skips to the newest.
const STATE_CHANGE_QUEUE: usize = 16;

/// Publishes the assistant's state to any number of observers, such as the orb.
///
/// Every change is delivered in order, and a new observer starts from the
/// current state. Publishing never waits: an observer that falls
/// `STATE_CHANGE_QUEUE` changes behind loses the oldest ones.
pub(crate) struct StateFeed {
    current: watch::Sender<AssistantState>,
    changes: broadcast::Sender<AssistantState>,
}

impl StateFeed {
    /// Starts at `Idle`, as the assistant does.
    pub(crate) fn new() -> Self {
        Self {
            current: watch::Sender::new(AssistantState::Idle),
            changes: broadcast::channel(STATE_CHANGE_QUEUE).0,
        }
    }

    /// Records the move to `state`. A repeat of the current state is not a change.
    pub(crate) fn publish(&self, state: AssistantState) {
        let changed = self.current.send_if_modified(|current| {
            let changed = *current != state;
            *current = state;
            changed
        });
        if changed {
            // No observer is normal: the state is still kept in `current`.
            let _ = self.changes.send(state);
        }
    }

    pub(crate) fn observe(&self) -> StateObserver {
        // Subscribe first, so a change between the two lines is seen twice rather
        // than missed; `next` skips the repeat.
        let changes = self.changes.subscribe();
        StateObserver {
            current: *self.current.borrow(),
            changes,
        }
    }
}

/// One observer's view of the state.
pub(crate) struct StateObserver {
    current: AssistantState,
    changes: broadcast::Receiver<AssistantState>,
}

impl StateObserver {
    /// The newest state this observer has seen.
    pub(crate) fn current(&self) -> AssistantState {
        self.current
    }

    /// Waits for the next change and returns the new state. Never completes once
    /// the feed is gone. Safe to drop at any `.await`.
    pub(crate) async fn next(&mut self) -> AssistantState {
        loop {
            match self.changes.recv().await {
                Ok(state) if state != self.current => {
                    self.current = state;
                    return state;
                }
                // A repeat, or changes lost by falling behind: keep reading.
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => std::future::pending().await,
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/features/state_tests.rs"]
mod tests;
