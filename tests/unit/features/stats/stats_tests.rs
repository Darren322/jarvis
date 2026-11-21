use std::time::Duration;

use orb_protocol::StorageUsage;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use super::{machine::MachineError, *};

/// Nothing listens here, so a health check is refused at once.
const DEAD_SERVER: &str = "http://127.0.0.1:1";

struct Scripted(MachineReading);

impl Machine for Scripted {
    async fn read(&mut self) -> Result<MachineReading, MachineError> {
        Ok(self.0)
    }
}

fn reading() -> MachineReading {
    MachineReading {
        cpu_percent: 12.5,
        temperature_c: Some(55.0),
        ram_used_bytes: 2,
        ram_total_bytes: 8,
        storage: Some(StorageUsage {
            available_bytes: 5,
            total_bytes: 9,
        }),
    }
}

fn check(url: &str) -> HealthCheck {
    HealthCheck::new(url).expect("a localhost URL is allowed")
}

struct Running {
    visibility: watch::Sender<StatsVisibility>,
    messages: mpsc::Receiver<ToOrb>,
    task: JoinHandle<()>,
}

impl Running {
    fn start(llama: &str, bge: &str, initial: StatsVisibility) -> Self {
        let (visibility, shown) = watch::channel(initial);
        let (to_orb, messages) = mpsc::channel(16);
        let source = StatsSource::new(Scripted(reading()), check(llama), check(bge));
        let task = tokio::spawn(source.run(shown, to_orb));
        Self {
            visibility,
            messages,
            task,
        }
    }

    fn show(&self) {
        self.visibility
            .send(StatsVisibility::Shown)
            .expect("the source is running");
    }

    fn hide(&self) {
        self.visibility
            .send(StatsVisibility::Hidden)
            .expect("the source is running");
    }

    fn received(&mut self) -> Vec<ToOrb> {
        std::iter::from_fn(|| self.messages.try_recv().ok()).collect()
    }
}

fn cpu(value: f32) -> ToOrb {
    stats_message(
        MachineReading {
            cpu_percent: value,
            ..reading()
        },
        HealthStatus::Up,
        HealthStatus::Up,
    )
}

#[test]
fn cpu_percent_is_held_between_0_and_100() {
    let cases = [(-0.1, 0.0), (0.0, 0.0), (100.0, 100.0), (100.1, 100.0)];
    for (raw, expected) in cases {
        let ToOrb::Stats { cpu_percent, .. } = cpu(raw) else {
            panic!("expected Stats");
        };
        assert_eq!(cpu_percent.value(), expected, "raw {raw}");
    }
}

#[test]
fn a_missing_temperature_sensor_is_absent_never_zero() {
    for temperature_c in [None, Some(f32::NAN), Some(f32::INFINITY)] {
        let message = stats_message(
            MachineReading {
                temperature_c,
                ..reading()
            },
            HealthStatus::Up,
            HealthStatus::Up,
        );
        let ToOrb::Stats { temperature_c, .. } = message else {
            panic!("expected Stats");
        };
        assert_eq!(temperature_c, None);
        let line = message.encode().expect("fits the line limit");
        assert!(!line.contains("temperature_c"), "{line}");
    }
}

#[test]
fn a_real_temperature_of_zero_is_kept() {
    let message = stats_message(
        MachineReading {
            temperature_c: Some(0.0),
            ..reading()
        },
        HealthStatus::Up,
        HealthStatus::Up,
    );
    let ToOrb::Stats { temperature_c, .. } = message else {
        panic!("expected Stats");
    };
    assert_eq!(temperature_c, Some(0.0));
}

#[test]
fn storage_reaches_the_wire_unchanged_and_absent_storage_sends_none() {
    let storage = StorageUsage {
        available_bytes: 207_517_036_544,
        total_bytes: 251_416_899_584,
    };
    for expected in [Some(storage), None] {
        let message = stats_message(
            MachineReading {
                storage: expected,
                ..reading()
            },
            HealthStatus::Up,
            HealthStatus::Up,
        );
        let ToOrb::Stats { storage, .. } = message else {
            panic!("expected Stats");
        };
        assert_eq!(storage, expected);
    }
}

#[test]
fn ram_used_is_at_most_ram_total() {
    // (used, total, used sent)
    let cases = [(0, 0, 0), (8, 8, 8), (9, 8, 8), (7, 8, 7), (1, 0, 0)];
    for (used, total, expected) in cases {
        let message = stats_message(
            MachineReading {
                ram_used_bytes: used,
                ram_total_bytes: total,
                ..reading()
            },
            HealthStatus::Up,
            HealthStatus::Up,
        );
        let ToOrb::Stats {
            ram_used_bytes,
            ram_total_bytes,
            ..
        } = message
        else {
            panic!("expected Stats");
        };
        assert_eq!(ram_used_bytes, expected, "used {used} of {total}");
        assert_eq!(ram_total_bytes, total);
    }
}

#[test]
fn each_service_reports_its_own_health() {
    let ToOrb::Stats { llama, bge, .. } =
        stats_message(reading(), HealthStatus::Up, HealthStatus::Down)
    else {
        panic!("expected Stats");
    };
    assert_eq!((llama, bge), (ServiceHealth::Up, ServiceHealth::Down));
}

// Five ticks (1 s to 5 s) fall inside 5.5 s, and the sixth does not.
const FIVE_TICKS: Duration = Duration::from_millis(5_500);

#[tokio::test(start_paused = true)]
async fn five_seconds_shown_sends_exactly_five_stats_messages() {
    let mut running = Running::start(DEAD_SERVER, DEAD_SERVER, StatsVisibility::Shown);
    tokio::time::sleep(FIVE_TICKS).await;
    let messages = running.received();
    assert_eq!(messages.len(), 5);
    assert!(
        messages
            .iter()
            .all(|message| matches!(message, ToOrb::Stats { .. }))
    );
}

#[tokio::test(start_paused = true)]
async fn five_seconds_hidden_sends_nothing() {
    let mut running = Running::start(DEAD_SERVER, DEAD_SERVER, StatsVisibility::Hidden);
    tokio::time::sleep(FIVE_TICKS).await;
    assert!(running.received().is_empty());
}

#[tokio::test(start_paused = true)]
async fn hiding_stops_the_messages_and_showing_again_resumes_at_the_next_tick() {
    let mut running = Running::start(DEAD_SERVER, DEAD_SERVER, StatsVisibility::Shown);
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert_eq!(running.received().len(), 2);

    running.hide();
    tokio::time::sleep(FIVE_TICKS).await;
    assert!(running.received().is_empty());

    running.show();
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(running.received().is_empty(), "no tick before one interval");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(running.received().len(), 1, "the first tick after showing");
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(running.received().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_server_that_never_answers_does_not_slow_the_cadence() {
    // The delay is on the mock server's own real clock, far longer than the test.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(60)))
        .mount(&server)
        .await;
    let mut running = Running::start(&server.uri(), &server.uri(), StatsVisibility::Shown);
    tokio::time::sleep(FIVE_TICKS).await;
    let messages = running.received();
    assert_eq!(messages.len(), 5);
    for message in messages {
        let ToOrb::Stats { llama, bge, .. } = message else {
            panic!("expected Stats");
        };
        assert_eq!((llama, bge), (ServiceHealth::Down, ServiceHealth::Down));
    }
}

// Real time: the reply must arrive over a real socket before the first tick.
#[tokio::test]
async fn services_that_answer_are_reported_up() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let mut running = Running::start(&server.uri(), DEAD_SERVER, StatsVisibility::Shown);
    let message = tokio::time::timeout(2 * STATS_INTERVAL, running.messages.recv())
        .await
        .expect("a message within two intervals")
        .expect("the source is running");
    let ToOrb::Stats { llama, bge, .. } = message else {
        panic!("expected Stats");
    };
    assert_eq!((llama, bge), (ServiceHealth::Up, ServiceHealth::Down));
}

#[tokio::test(start_paused = true)]
async fn the_source_ends_when_nobody_can_change_visibility_or_hear_it() {
    let running = Running::start(DEAD_SERVER, DEAD_SERVER, StatsVisibility::Shown);
    let Running {
        visibility,
        messages,
        task,
    } = running;
    drop(messages);
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert!(task.is_finished(), "a closed orb queue ends the source");
    drop(visibility);
}

/// The real panel state, with the stats visibility following it.
struct Panels {
    panels: crate::features::panel::PanelHandle,
    visibility: watch::Receiver<StatsVisibility>,
    closed_by_orb: mpsc::Sender<orb_protocol::FromOrb>,
    _hide_messages: mpsc::Receiver<ToOrb>,
}

const PANEL_TIME: Duration = Duration::from_secs(5);

impl Panels {
    fn start() -> Self {
        let (panels, inbox) = crate::features::panel::channel();
        let (to_orb, hide_messages) = mpsc::channel(16);
        let (closed_by_orb, from_orb) = mpsc::channel(16);
        let orb = crate::interfaces::orb::OrbHandle { to_orb, from_orb };
        let (visibility_sender, visibility) = watch::channel(StatsVisibility::Hidden);
        tokio::spawn(async move { match inbox.run(Some(orb), PANEL_TIME).await {} });
        tokio::spawn(mirror_panel(panels.watch_shown(), visibility_sender));
        Self {
            panels,
            visibility,
            closed_by_orb,
            _hide_messages: hide_messages,
        }
    }

    async fn settle(&self) {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }

    async fn show_stats(&self) {
        self.panels
            .show(PanelName::Stats)
            .await
            .expect("the panel state is running");
        self.settle().await;
    }

    fn visibility(&mut self) -> StatsVisibility {
        *self.visibility.borrow_and_update()
    }
}

#[test]
fn the_dashboard_is_visible_exactly_while_the_stats_panel_is_shown() {
    assert_eq!(
        StatsVisibility::of_panel(Some(PanelName::Stats)),
        StatsVisibility::Shown
    );
    assert_eq!(StatsVisibility::of_panel(None), StatsVisibility::Hidden);
    assert_eq!(
        StatsVisibility::of_panel(Some(PanelName::Camera)),
        StatsVisibility::Hidden
    );
}

#[tokio::test(start_paused = true)]
async fn showing_the_stats_panel_shows_the_dashboard_and_closing_it_hides_it() {
    let mut rig = Panels::start();
    rig.settle().await;
    assert_eq!(rig.visibility(), StatsVisibility::Hidden);

    rig.show_stats().await;
    assert_eq!(rig.visibility(), StatsVisibility::Shown);

    rig.panels
        .close()
        .await
        .expect("the panel state is running");
    rig.settle().await;
    assert_eq!(rig.visibility(), StatsVisibility::Hidden);
}

#[tokio::test(start_paused = true)]
async fn replacing_the_stats_panel_does_not_wake_the_stats_source() {
    let mut rig = Panels::start();
    rig.show_stats().await;
    assert_eq!(rig.visibility(), StatsVisibility::Shown);

    rig.show_stats().await;
    assert!(
        !rig.visibility.has_changed().expect("the mirror is running"),
        "the answer did not change"
    );
    assert_eq!(rig.visibility(), StatsVisibility::Shown);
}

#[tokio::test(start_paused = true)]
async fn the_orb_closing_the_panel_hides_the_dashboard() {
    let mut rig = Panels::start();
    rig.show_stats().await;
    assert_eq!(rig.visibility(), StatsVisibility::Shown);

    rig.closed_by_orb
        .send(orb_protocol::FromOrb::PanelClosed {})
        .await
        .expect("the panel state is running");
    rig.settle().await;
    assert_eq!(rig.visibility(), StatsVisibility::Hidden);
}

#[tokio::test(start_paused = true)]
async fn the_dashboard_hides_when_the_panel_times_out_and_not_before() {
    let mut rig = Panels::start();
    rig.show_stats().await;

    tokio::time::advance(PANEL_TIME - Duration::from_millis(1)).await;
    rig.settle().await;
    assert_eq!(rig.visibility(), StatsVisibility::Shown);

    tokio::time::advance(Duration::from_millis(1)).await;
    rig.settle().await;
    assert_eq!(rig.visibility(), StatsVisibility::Hidden);
}

#[tokio::test(start_paused = true)]
async fn stats_flow_while_the_panel_is_shown_and_stop_when_it_closes() {
    let rig = Panels::start();
    let (to_orb, mut messages) = mpsc::channel(16);
    let (visibility, shown) = watch::channel(StatsVisibility::Hidden);
    let source = StatsSource::new(Scripted(reading()), check(DEAD_SERVER), check(DEAD_SERVER));
    tokio::spawn(source.run(shown, to_orb));
    tokio::spawn(mirror_panel(rig.panels.watch_shown(), visibility));
    rig.settle().await;
    assert!(messages.try_recv().is_err());

    rig.show_stats().await;
    tokio::time::advance(STATS_INTERVAL * 2).await;
    rig.settle().await;
    assert!(messages.try_recv().is_ok());

    rig.panels
        .close()
        .await
        .expect("the panel state is running");
    rig.settle().await;
    while messages.try_recv().is_ok() {}
    tokio::time::advance(STATS_INTERVAL * 5).await;
    rig.settle().await;
    assert!(messages.try_recv().is_err());
}
