use std::time::Duration;

use orb_protocol::{FromOrb, PanelName, ToOrb};
use tokio::{
    sync::mpsc::{self, error::TryRecvError},
    task::{JoinHandle, yield_now},
};

use super::{PANEL_TIMEOUT, PanelHandle, channel, timeout_for};
use crate::features::{
    control::{TapAction, tap_action},
    state::AssistantState,
};
use crate::interfaces::orb::OrbHandle;

const ONE_MILLISECOND: Duration = Duration::from_millis(1);

/// A running panel state with the orb's side of its channels.
struct Rig {
    panels: PanelHandle,
    shown_to_orb: mpsc::Receiver<ToOrb>,
    from_orb: mpsc::Sender<FromOrb>,
    task: JoinHandle<()>,
}

fn start() -> Rig {
    let (panels, inbox) = channel();
    let (to_orb, shown_to_orb) = mpsc::channel(16);
    let (from_orb, orb_events) = mpsc::channel(16);
    let orb = OrbHandle {
        to_orb,
        from_orb: orb_events,
    };
    let task = tokio::spawn(async move { match inbox.run(Some(orb), PANEL_TIMEOUT).await {} });
    Rig {
        panels,
        shown_to_orb,
        from_orb,
        task,
    }
}

/// Lets the panel task run everything it can without time passing.
async fn settle() {
    for _ in 0..8 {
        yield_now().await;
    }
}

impl Rig {
    async fn show_stats(&mut self) {
        self.show(PanelName::Stats).await;
    }

    async fn show(&mut self, panel: PanelName) {
        self.panels
            .show(panel)
            .await
            .expect("the panel task is running");
        assert_eq!(
            self.shown_to_orb.recv().await,
            Some(ToOrb::ShowPanel { panel })
        );
    }

    fn nothing_sent(&mut self) {
        assert!(matches!(
            self.shown_to_orb.try_recv(),
            Err(TryRecvError::Empty)
        ));
    }
}

#[test]
fn the_stats_panel_times_out_after_the_configured_time() {
    let timeout = Duration::from_secs(7);
    assert_eq!(timeout_for(PanelName::Stats, timeout), Some(timeout));
    assert_eq!(PANEL_TIMEOUT, Duration::from_secs(30));
}

#[test]
fn the_camera_panel_never_times_out() {
    assert_eq!(timeout_for(PanelName::Camera, Duration::from_secs(7)), None);
}

/// Longer than an hour, which is as long as ticket 11 asks the camera to stay.
const OVER_AN_HOUR: Duration = Duration::from_secs(60 * 60 + 1);

#[tokio::test(start_paused = true)]
async fn the_camera_panel_stays_shown_for_over_an_hour() {
    let mut rig = start();
    rig.show(PanelName::Camera).await;

    tokio::time::advance(OVER_AN_HOUR).await;
    settle().await;
    assert_eq!(rig.panels.shown(), Some(PanelName::Camera));
    rig.nothing_sent();
}

#[tokio::test(start_paused = true)]
async fn the_camera_replacing_the_stats_drops_the_stats_timeout() {
    let mut rig = start();
    rig.show_stats().await;
    rig.show(PanelName::Camera).await;

    tokio::time::advance(PANEL_TIMEOUT * 2).await;
    settle().await;
    assert_eq!(rig.panels.shown(), Some(PanelName::Camera));
    rig.nothing_sent();
}

#[tokio::test(start_paused = true)]
async fn the_panel_is_shown_until_the_timeout_and_then_hidden() {
    let mut rig = start();
    rig.show_stats().await;
    assert_eq!(rig.panels.shown(), Some(PanelName::Stats));

    tokio::time::advance(PANEL_TIMEOUT - ONE_MILLISECOND).await;
    settle().await;
    assert_eq!(rig.panels.shown(), Some(PanelName::Stats));
    rig.nothing_sent();

    tokio::time::advance(ONE_MILLISECOND).await;
    settle().await;
    assert_eq!(rig.shown_to_orb.try_recv(), Ok(ToOrb::HidePanel {}));
    assert_eq!(rig.panels.shown(), None);
    rig.nothing_sent();
}

#[tokio::test(start_paused = true)]
async fn a_panel_closed_message_hides_the_panel_without_a_reply() {
    let mut rig = start();
    rig.show_stats().await;

    rig.from_orb
        .send(FromOrb::PanelClosed {})
        .await
        .expect("the panel task is running");
    settle().await;

    assert_eq!(rig.panels.shown(), None);
    rig.nothing_sent();
    // The old deadline is gone: the timeout does not send a second hide.
    tokio::time::advance(PANEL_TIMEOUT * 2).await;
    settle().await;
    rig.nothing_sent();
}

#[tokio::test(start_paused = true)]
async fn showing_a_panel_again_replaces_it_and_restarts_the_timeout() {
    let mut rig = start();
    rig.show_stats().await;
    tokio::time::advance(PANEL_TIMEOUT - ONE_MILLISECOND).await;
    settle().await;

    rig.show_stats().await;
    tokio::time::advance(PANEL_TIMEOUT - ONE_MILLISECOND).await;
    settle().await;
    // The first deadline has passed, but the second show replaced it.
    assert_eq!(rig.panels.shown(), Some(PanelName::Stats));
    rig.nothing_sent();

    tokio::time::advance(ONE_MILLISECOND).await;
    settle().await;
    assert_eq!(rig.shown_to_orb.try_recv(), Ok(ToOrb::HidePanel {}));
    rig.nothing_sent();
}

#[tokio::test(start_paused = true)]
async fn closing_hides_the_shown_panel_and_does_nothing_when_none_is_shown() {
    let mut rig = start();
    rig.panels.close().await.expect("the panel task is running");
    settle().await;
    rig.nothing_sent();

    rig.show_stats().await;
    rig.panels.close().await.expect("the panel task is running");
    settle().await;
    assert_eq!(rig.shown_to_orb.try_recv(), Ok(ToOrb::HidePanel {}));
    assert_eq!(rig.panels.shown(), None);
}

#[tokio::test(start_paused = true)]
async fn a_tap_means_listen_and_leaves_the_panel_shown() {
    let mut rig = start();
    rig.show_stats().await;

    // A tap never reaches the panel: the loop acts on it from the real state.
    assert_eq!(tap_action(AssistantState::Idle), TapAction::Listen);
    settle().await;

    assert_eq!(rig.panels.shown(), Some(PanelName::Stats));
    rig.nothing_sent();
}

#[tokio::test(start_paused = true)]
async fn without_an_orb_the_state_still_follows_and_nothing_waits() {
    let (panels, inbox) = channel();
    let task = tokio::spawn(async move { match inbox.run(None, PANEL_TIMEOUT).await {} });

    panels
        .show(PanelName::Stats)
        .await
        .expect("the panel task is running");
    settle().await;
    assert_eq!(panels.shown(), Some(PanelName::Stats));

    tokio::time::advance(PANEL_TIMEOUT).await;
    settle().await;
    assert_eq!(panels.shown(), None);
    task.abort();
}

#[tokio::test]
async fn asking_a_stopped_panel_service_is_an_error() {
    let rig = start();
    rig.task.abort();
    let _ = rig.task.await;

    assert!(rig.panels.show(PanelName::Stats).await.is_err());
    assert!(rig.panels.close().await.is_err());
}

#[tokio::test(start_paused = true)]
async fn a_timeout_too_long_for_the_clock_means_the_panel_stays_until_closed() {
    let (panels, inbox) = channel();
    let task = tokio::spawn(async move { match inbox.run(None, Duration::MAX).await {} });

    panels
        .show(PanelName::Stats)
        .await
        .expect("the panel task is running");
    settle().await;
    assert_eq!(panels.shown(), Some(PanelName::Stats));
    assert!(!task.is_finished());

    tokio::time::advance(Duration::from_secs(86_400)).await;
    settle().await;
    assert_eq!(panels.shown(), Some(PanelName::Stats));
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn the_configured_timeout_decides_when_the_panel_hides() {
    let configured = Duration::from_secs(1);
    let (panels, inbox) = channel();
    let task = tokio::spawn(async move { match inbox.run(None, configured).await {} });
    panels
        .show(PanelName::Stats)
        .await
        .expect("the panel task is running");
    settle().await;

    tokio::time::advance(configured - ONE_MILLISECOND).await;
    settle().await;
    assert_eq!(panels.shown(), Some(PanelName::Stats));
    tokio::time::advance(ONE_MILLISECOND).await;
    settle().await;
    assert_eq!(panels.shown(), None);
    task.abort();
}
