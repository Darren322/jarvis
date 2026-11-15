use orb_protocol::{FromOrb, PanelName, ToOrb};
use tokio::{sync::mpsc, task::yield_now};

use rig_core::test_utils::MockCompletionModel;

use super::close_panel;
use crate::features::{
    control::{Command, Notice, parse},
    panel::{PANEL_TIMEOUT, channel},
};
use crate::interfaces::orb::OrbHandle;

async fn settle() {
    for _ in 0..8 {
        yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn closing_hides_a_shown_panel_and_says_so() {
    let (panels, inbox) = channel();
    let (to_orb, mut shown_to_orb) = mpsc::channel(16);
    let (_events, from_orb) = mpsc::channel::<FromOrb>(16);
    let task = tokio::spawn(async move {
        match inbox
            .run(Some(OrbHandle { to_orb, from_orb }), PANEL_TIMEOUT)
            .await {}
    });

    assert_eq!(
        close_panel(&panels).await,
        Notice::Info("No panel is open.".to_owned())
    );

    panels
        .show(PanelName::Stats)
        .await
        .expect("the panel task is running");
    settle().await;
    assert_eq!(
        close_panel(&panels).await,
        Notice::Info("Panel closed.".to_owned())
    );
    settle().await;
    assert_eq!(
        shown_to_orb.recv().await,
        Some(ToOrb::ShowPanel {
            panel: PanelName::Stats
        })
    );
    assert_eq!(shown_to_orb.recv().await, Some(ToOrb::HidePanel {}));
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn a_stopped_panel_service_is_a_warning_not_a_crash() {
    let (panels, inbox) = channel();
    let task = tokio::spawn(async move { match inbox.run(None, PANEL_TIMEOUT).await {} });
    panels
        .show(PanelName::Stats)
        .await
        .expect("the panel task is running");
    settle().await;
    task.abort();
    let _ = task.await;

    assert!(matches!(close_panel(&panels).await, Notice::Warning(_)));
}

#[tokio::test]
async fn a_close_phrase_makes_no_model_call() {
    // A model with no scripted turn: any request to it would panic or fail.
    let model = MockCompletionModel::from_stream_turns(Vec::<Vec<_>>::new());
    let handle = model.clone();
    let (panels, _inbox) = channel();

    for phrase in ["close it", "Close it.", "hide that", "  HIDE THAT  "] {
        assert_eq!(parse(phrase.to_owned()), Command::Close, "{phrase}");
        close_panel(&panels).await;
    }

    assert!(handle.requests().is_empty());
}
