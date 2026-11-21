use orb_protocol::{FromOrb, PanelName, ToOrb};
use tokio::{sync::mpsc, task::yield_now};

use rig_core::test_utils::MockCompletionModel;

use super::{close_panel, show_panel};
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

#[tokio::test(start_paused = true)]
async fn a_show_phrase_shows_the_stats_without_a_model_call() {
    // A model with no scripted turn: any request to it would panic or fail.
    let model = MockCompletionModel::from_stream_turns(Vec::<Vec<_>>::new());
    let handle = model.clone();
    let (panels, inbox) = channel();
    let (to_orb, mut shown_to_orb) = mpsc::channel(16);
    let (_events, from_orb) = mpsc::channel::<FromOrb>(16);
    let task = tokio::spawn(async move {
        match inbox
            .run(Some(OrbHandle { to_orb, from_orb }), PANEL_TIMEOUT)
            .await {}
    });

    let Command::Show(panel) = parse("Bring up the stats.".to_owned()) else {
        panic!("a show phrase parses to Show");
    };
    assert_eq!(
        show_panel(&panels, panel).await,
        Notice::Info("Showing the stats.".to_owned())
    );
    settle().await;

    assert_eq!(
        shown_to_orb.recv().await,
        Some(ToOrb::ShowPanel {
            panel: PanelName::Stats
        })
    );
    assert!(handle.requests().is_empty());
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn a_show_phrase_shows_the_camera_without_a_model_call() {
    // A model with no scripted turn: any request to it would panic or fail.
    let model = MockCompletionModel::from_stream_turns(Vec::<Vec<_>>::new());
    let handle = model.clone();
    let (panels, inbox) = channel();
    let (to_orb, mut shown_to_orb) = mpsc::channel(16);
    let (_events, from_orb) = mpsc::channel::<FromOrb>(16);
    let task = tokio::spawn(async move {
        match inbox
            .run(Some(OrbHandle { to_orb, from_orb }), PANEL_TIMEOUT)
            .await {}
    });

    let Command::Show(panel) = parse("Show me the camera.".to_owned()) else {
        panic!("a show phrase parses to Show");
    };
    assert_eq!(
        show_panel(&panels, panel).await,
        Notice::Info("Showing the camera.".to_owned())
    );
    settle().await;

    assert_eq!(
        shown_to_orb.recv().await,
        Some(ToOrb::ShowPanel {
            panel: PanelName::Camera
        })
    );
    assert!(handle.requests().is_empty());
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn showing_with_the_panel_service_stopped_is_a_warning_not_a_crash() {
    let (panels, inbox) = channel();
    drop(inbox);

    assert!(matches!(
        show_panel(&panels, PanelName::Stats).await,
        Notice::Warning(_)
    ));
}

#[tokio::test(start_paused = true)]
async fn close_it_closes_a_built_panel() {
    let (panels, inbox) = channel();
    let (to_orb, mut shown_to_orb) = mpsc::channel(16);
    let (_events, from_orb) = mpsc::channel::<FromOrb>(16);
    let task = tokio::spawn(async move {
        match inbox
            .run(Some(OrbHandle { to_orb, from_orb }), PANEL_TIMEOUT)
            .await {}
    });
    let built = orb_protocol::BuiltPanel::new(
        "Shopping".to_owned(),
        vec![orb_protocol::PanelPart::List {
            title: "To buy".to_owned(),
            items: vec!["milk".to_owned()],
        }],
    )
    .expect("within the limits");
    panels
        .show_built(built.clone())
        .await
        .expect("the panel task is running");
    settle().await;

    assert_eq!(parse("close it".to_owned()), Command::Close);
    assert_eq!(
        close_panel(&panels).await,
        Notice::Info("Panel closed.".to_owned())
    );
    settle().await;
    assert_eq!(
        shown_to_orb.recv().await,
        Some(ToOrb::ShowBuiltPanel(built))
    );
    assert_eq!(shown_to_orb.recv().await, Some(ToOrb::HidePanel {}));
    assert_eq!(panels.shown(), None);
    task.abort();
}
