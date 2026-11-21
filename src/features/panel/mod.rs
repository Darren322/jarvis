//! The panel on the orb screen: which one shows, and when it goes away.
//!
//! One panel shows at a time. Whatever asks for one (the `show_panel` and
//! `build_panel` tools) or closes it (the "close it" phrase) uses a
//! [`PanelHandle`]; the [`PanelInbox`] owns the state, sends `ShowPanel`,
//! `ShowBuiltPanel` and `HidePanel` through the orb handle, hears `PanelClosed`
//! from the orb and hides a panel whose time is up.
//!
//! A tap on the orb never reaches this module: it means listen
//! (`control::tap_action`) and leaves the panel as it is.

use std::{convert::Infallible, future, time::Duration};

use orb_protocol::{BuiltPanel, FromOrb, PanelName, ToOrb};
use thiserror::Error;
use tokio::{
    sync::{mpsc, watch},
    time::{Instant, sleep_until},
};

use crate::interfaces::orb::OrbHandle;

/// How long the stats panel, or a built one, stays up when nobody closes it
/// (spec proposal P2, jarvis-ui 12 G6).
/// The default of the configurable value.
pub(crate) const PANEL_TIMEOUT: Duration = Duration::from_secs(30);

/// Requests waiting for the panel state. Showing or closing waits when it is full.
const REQUEST_QUEUE: usize = 4;

/// How long `panel` stays up, if it times out at all. A built Panel times out
/// like the stats one (G6); the camera returns `None`: it stays until closed
/// (jarvis-ui D8).
pub(crate) fn timeout_for(panel: PanelName, dashboard_timeout: Duration) -> Option<Duration> {
    match panel {
        PanelName::Stats | PanelName::Built => Some(dashboard_timeout),
        PanelName::Camera => None,
    }
}

#[derive(Debug, Error)]
pub(crate) enum PanelError {
    #[error("the panel service is not running")]
    Unavailable,
    /// A built Panel opens with its parts (`show_built`), never by name.
    #[error("a built panel is shown with its parts, not by name")]
    BuiltByName,
}

#[derive(Debug)]
enum Request {
    /// Show `panel` by sending `message`, which opens it on the orb.
    Show {
        panel: PanelName,
        message: ToOrb,
    },
    Close,
}

/// How features ask for a panel. Cheap to clone.
#[derive(Clone)]
pub(crate) struct PanelHandle {
    requests: mpsc::Sender<Request>,
    shown: watch::Receiver<Option<PanelName>>,
}

impl PanelHandle {
    /// Shows `panel`, replacing the one already shown. The built Panel has no
    /// content to show by name, so asking for it is [`PanelError::BuiltByName`].
    pub(crate) async fn show(&self, panel: PanelName) -> Result<(), PanelError> {
        match panel {
            PanelName::Stats | PanelName::Camera => {
                self.send(Request::Show {
                    panel,
                    message: ToOrb::ShowPanel { panel },
                })
                .await
            }
            PanelName::Built => Err(PanelError::BuiltByName),
        }
    }

    /// Shows `built`, replacing the panel already shown, with the stats timeout.
    pub(crate) async fn show_built(&self, built: BuiltPanel) -> Result<(), PanelError> {
        self.send(Request::Show {
            panel: PanelName::Built,
            message: ToOrb::ShowBuiltPanel(built),
        })
        .await
    }

    /// Hides the panel that is shown; nothing happens when none is.
    pub(crate) async fn close(&self) -> Result<(), PanelError> {
        self.send(Request::Close).await
    }

    async fn send(&self, request: Request) -> Result<(), PanelError> {
        self.requests
            .send(request)
            .await
            .map_err(|_| PanelError::Unavailable)
    }

    /// The panel shown now, as far as the panel state has got.
    pub(crate) fn shown(&self) -> Option<PanelName> {
        *self.shown.borrow()
    }

    /// Follows the shown panel, so a feature that only cares about one kind of
    /// panel (the stats source) learns of every change without the panel state
    /// knowing it exists.
    pub(crate) fn watch_shown(&self) -> watch::Receiver<Option<PanelName>> {
        self.shown.clone()
    }
}

/// The panel state, waiting to be run beside the orb server.
pub(crate) struct PanelInbox {
    requests: mpsc::Receiver<Request>,
    shown: watch::Sender<Option<PanelName>>,
}

/// A handle for features and the inbox that serves it.
pub(crate) fn channel() -> (PanelHandle, PanelInbox) {
    let (request_sender, requests) = mpsc::channel(REQUEST_QUEUE);
    let (shown, shown_receiver) = watch::channel(None);
    (
        PanelHandle {
            requests: request_sender,
            shown: shown_receiver,
        },
        PanelInbox { requests, shown },
    )
}

impl PanelInbox {
    /// Serves requests until dropped. `orb` is `None` when no orb screen could be
    /// opened: panels are then shown to nobody, and the state still follows.
    ///
    /// Safe to drop at any `.await`: the only state is the shown panel and its
    /// deadline, and both die with it.
    pub(crate) async fn run(
        mut self,
        orb: Option<OrbHandle>,
        dashboard_timeout: Duration,
    ) -> Infallible {
        // Without an orb, sends fail at once and nothing is ever heard.
        let (to_orb, mut from_orb, _quiet) = match orb {
            Some(OrbHandle { to_orb, from_orb }) => (to_orb, from_orb, None),
            None => {
                let (to_orb, _gone) = mpsc::channel(1);
                let (quiet, from_orb) = mpsc::channel(1);
                (to_orb, from_orb, Some(quiet))
            }
        };
        let mut deadline: Option<Instant> = None;
        loop {
            let expired = async {
                match deadline {
                    Some(deadline) => sleep_until(deadline).await,
                    None => future::pending().await,
                }
            };
            tokio::select! {
                biased;
                // `None` means every handle is gone; the branch then stays off.
                Some(request) = self.requests.recv() => match request {
                    Request::Show { panel, message } => {
                        // State first: a send that waits must not leave the old deadline.
                        // A timeout too long to add to the clock never fires.
                        deadline = timeout_for(panel, dashboard_timeout)
                            .and_then(|timeout| Instant::now().checked_add(timeout));
                        self.shown.send_replace(Some(panel));
                        let _ = to_orb.send(message).await;
                    }
                    Request::Close => {
                        deadline = None;
                        if self.shown.send_replace(None).is_some() {
                            let _ = to_orb.send(ToOrb::HidePanel {}).await;
                        }
                    }
                },
                // The orb closed it itself, so there is nothing to send back.
                Some(FromOrb::PanelClosed {}) = from_orb.recv() => {
                    self.shown.send_replace(None);
                    deadline = None;
                }
                () = expired => {
                    self.shown.send_replace(None);
                    deadline = None;
                    let _ = to_orb.send(ToOrb::HidePanel {}).await;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/panel/panel_tests.rs"]
mod tests;
