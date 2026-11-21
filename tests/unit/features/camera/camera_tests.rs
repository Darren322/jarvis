use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Stdio, time::Duration};

use orb_protocol::{FromOrb, PanelName, ToOrb};
use tempfile::TempDir;
use tokio::{
    sync::mpsc,
    task::{JoinHandle, yield_now},
    time::{Instant, sleep},
};

use super::*;
use crate::features::panel::{PANEL_TIMEOUT, channel};
use crate::interfaces::orb::OrbHandle;

/// Long enough for a shell script to start and write, on a busy machine.
const WAIT: Duration = Duration::from_secs(5);
/// How often a test looks again while it waits for something to happen.
const POLL: Duration = Duration::from_millis(10);

/// The first and second frames the fake programs send: JPEG-shaped, no more.
const FIRST_FRAME: &[u8] = b"\xFF\xD8first\xFF\xD9";
const SECOND_FRAME: &[u8] = b"\xFF\xD8second\xFF\xD9";
/// The first frame as `printf` writes it.
const PRINT_FIRST: &str = r"printf '\377\330first\377\331'";

/// A fake camera program in its own directory, which records its pid, its
/// arguments and each start.
struct FakeProgram {
    dir: TempDir,
}

impl FakeProgram {
    /// A script that records itself, then runs `body`.
    fn new(body: &str) -> Self {
        let dir = TempDir::new().expect("a temporary directory");
        let script = dir.path().join("fake-rpicam-vid");
        let here = dir.path().display();
        fs::write(
            &script,
            format!(
                "#!/bin/sh\necho $$ > '{here}/pid.tmp' && mv '{here}/pid.tmp' '{here}/pid'\n\
                 echo \"$@\" > '{here}/args'\necho start >> '{here}/starts'\n{body}\n"
            ),
        )
        .expect("the script is written");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
            .expect("the script is runnable");
        Self { dir }
    }

    fn program(&self) -> String {
        self.path("fake-rpicam-vid").display().to_string()
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// How many times the program has started.
    fn starts(&self) -> usize {
        fs::read_to_string(self.path("starts")).map_or(0, |starts| starts.lines().count())
    }

    /// The pid of its latest start, waiting for it to be written.
    async fn pid(&self) -> String {
        let path = self.path("pid");
        wait_for(|| path.exists()).await;
        fs::read_to_string(path)
            .expect("the pid is written")
            .trim()
            .to_owned()
    }
}

fn is_alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .expect("kill runs")
        .success()
}

/// Waits, in real time, until `done` holds; fails the test after `WAIT`.
async fn wait_for(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "gave up waiting");
        sleep(POLL).await;
    }
}

/// Waits until the process is gone and reaped: `kill -0` succeeds on a zombie.
async fn wait_until_gone(pid: &str) {
    wait_for(|| !is_alive(pid)).await;
}

/// The real panel state with a fake orb, and a camera following it.
struct Rig {
    panels: PanelHandle,
    feed: CameraFeed,
    failures: mpsc::Receiver<Notice>,
    from_orb: mpsc::Sender<FromOrb>,
    to_orb: mpsc::Receiver<ToOrb>,
    camera: JoinHandle<()>,
    _panel_task: JoinHandle<()>,
}

impl Rig {
    fn start(program: &str) -> Self {
        let (panels, inbox) = channel();
        let (to_orb_sender, to_orb) = mpsc::channel(16);
        let (from_orb, orb_events) = mpsc::channel(16);
        let orb = OrbHandle {
            to_orb: to_orb_sender,
            from_orb: orb_events,
        };
        let panel_task =
            tokio::spawn(async move { match inbox.run(Some(orb), PANEL_TIMEOUT).await {} });
        let (camera, feed) = Camera::new(program, panels.clone());
        let (failure_sender, failures) = mpsc::channel(FAILURE_QUEUE);
        let camera = tokio::spawn(camera.run(failure_sender));
        Self {
            panels,
            feed,
            failures,
            from_orb,
            to_orb,
            camera,
            _panel_task: panel_task,
        }
    }

    async fn show(&self, panel: PanelName) {
        self.panels
            .show(panel)
            .await
            .expect("the panel state is running");
    }

    /// The first frame a new viewer gets, within `WAIT`, once the panel state
    /// has the camera shown.
    async fn first_frame(&self) -> Frame {
        wait_for(|| self.panels.shown() == Some(PanelName::Camera)).await;
        let mut stream = self.feed.watch().expect("the camera panel is shown");
        tokio::time::timeout(WAIT, stream.next())
            .await
            .expect("a frame within the wait")
            .expect("the camera panel is still shown")
    }

    /// The panel messages sent to the orb so far.
    fn sent_to_orb(&mut self) -> Vec<ToOrb> {
        std::iter::from_fn(|| self.to_orb.try_recv().ok()).collect()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.camera.abort();
    }
}

/// Sends one frame, then waits to be killed.
fn one_frame_then_wait() -> String {
    format!("{PRINT_FIRST}\nexec sleep 30")
}

#[tokio::test]
async fn the_program_is_asked_for_mirrored_800x450_mjpeg_at_30_fps_on_stdout() {
    let fake = FakeProgram::new(&one_frame_then_wait());
    let rig = Rig::start(&fake.program());
    rig.show(PanelName::Camera).await;
    fake.pid().await;
    wait_for(|| fake.path("args").exists()).await;
    let args = fs::read_to_string(fake.path("args")).expect("the arguments are written");
    assert_eq!(
        args.trim(),
        "--width 800 --height 450 --framerate 30 --hflip --codec mjpeg --nopreview -t 0 -o -"
    );
}

#[tokio::test]
async fn nothing_films_until_the_camera_panel_is_shown() {
    let fake = FakeProgram::new(&one_frame_then_wait());
    let rig = Rig::start(&fake.program());
    rig.show(PanelName::Stats).await;
    sleep(Duration::from_millis(300)).await;
    assert_eq!(fake.starts(), 0);
    assert!(rig.feed.watch().is_none(), "no viewer while the stats show");
}

#[tokio::test]
async fn showing_the_camera_starts_the_program_and_its_frames_reach_a_viewer() {
    let fake = FakeProgram::new(&one_frame_then_wait());
    let rig = Rig::start(&fake.program());
    rig.show(PanelName::Camera).await;
    assert_eq!(rig.first_frame().await, FIRST_FRAME);
    assert_eq!(fake.starts(), 1);
}

/// What ends the camera Panel in each stop test.
#[derive(Clone, Copy, Debug)]
enum Stop {
    Hide,
    ShowStats,
    /// A tap outside the Panel, or the UI going away (the orb interface sends
    /// `PanelClosed` when a connection ends).
    OrbClosed,
    /// Jarvis shutting down: the camera's future is dropped.
    Shutdown,
}

#[tokio::test]
async fn every_way_the_panel_ends_stops_and_reaps_the_program() {
    for stop in [Stop::Hide, Stop::ShowStats, Stop::OrbClosed, Stop::Shutdown] {
        let fake = FakeProgram::new(&one_frame_then_wait());
        let rig = Rig::start(&fake.program());
        rig.show(PanelName::Camera).await;
        rig.first_frame().await;
        let pid = fake.pid().await;
        assert!(is_alive(&pid), "{stop:?}: filming");

        match stop {
            Stop::Hide => rig
                .panels
                .close()
                .await
                .expect("the panel state is running"),
            Stop::ShowStats => rig.show(PanelName::Stats).await,
            Stop::OrbClosed => rig
                .from_orb
                .send(FromOrb::PanelClosed {})
                .await
                .expect("the panel state is running"),
            Stop::Shutdown => rig.camera.abort(),
        }
        wait_until_gone(&pid).await;
        assert_eq!(fake.starts(), 1, "{stop:?}");
        match stop {
            // A later viewer never gets the last frame of this stretch.
            Stop::Hide | Stop::ShowStats | Stop::OrbClosed => {
                assert_eq!(*rig.feed.frames.borrow(), None, "{stop:?}");
            }
            // Nobody films any more, so every viewer's stream ends.
            Stop::Shutdown => assert!(rig.feed.frames.has_changed().is_err()),
        }
    }
}

#[tokio::test]
async fn showing_the_camera_again_after_hiding_it_starts_a_new_program() {
    let fake = FakeProgram::new(&one_frame_then_wait());
    let rig = Rig::start(&fake.program());
    rig.show(PanelName::Camera).await;
    rig.first_frame().await;
    let first = fake.pid().await;
    rig.panels
        .close()
        .await
        .expect("the panel state is running");
    wait_until_gone(&first).await;
    fs::remove_file(fake.path("pid")).expect("the pid file is there");

    rig.show(PanelName::Camera).await;
    assert_eq!(rig.first_frame().await, FIRST_FRAME);
    let second = fake.pid().await;
    assert_ne!(first, second);
    assert_eq!(fake.starts(), 2);
}

#[tokio::test]
async fn a_program_that_cannot_start_closes_the_panel_and_says_the_camera_is_unavailable() {
    let missing = TempDir::new().expect("a temporary directory");
    let program = missing.path().join("no-such-program");
    let mut rig = Rig::start(&program.display().to_string());
    rig.show(PanelName::Camera).await;

    let failure = tokio::time::timeout(WAIT, rig.failures.recv())
        .await
        .expect("the failure is told");
    assert_eq!(failure, Some(CameraUnavailable.notice()));
    assert_eq!(
        CameraUnavailable.notice().text(),
        "The camera isn't available."
    );
    wait_for(|| rig.panels.shown().is_none()).await;
    assert_eq!(
        rig.sent_to_orb(),
        [
            ToOrb::ShowPanel {
                panel: PanelName::Camera
            },
            ToOrb::HidePanel {}
        ],
        "the orb is told to hide the panel, so the Orb returns"
    );
}

#[tokio::test]
async fn a_program_that_exits_while_shown_closes_the_panel_once_and_is_not_retried() {
    // Like a busy or unplugged camera: a frame or none, then an error exit.
    let fake = FakeProgram::new(&format!("{PRINT_FIRST}\nexit 1"));
    let mut rig = Rig::start(&fake.program());
    rig.show(PanelName::Camera).await;

    let failure = tokio::time::timeout(WAIT, rig.failures.recv())
        .await
        .expect("the failure is told");
    assert_eq!(failure, Some(CameraUnavailable.notice()));
    wait_for(|| rig.panels.shown().is_none()).await;
    sleep(Duration::from_millis(300)).await;
    assert_eq!(fake.starts(), 1, "no retry loop");
    assert!(rig.failures.try_recv().is_err(), "told once");
    assert!(
        rig.feed.watch().is_none(),
        "no viewer once the panel closed"
    );

    // Asking again tries again, once.
    rig.show(PanelName::Camera).await;
    let failure = tokio::time::timeout(WAIT, rig.failures.recv())
        .await
        .expect("the second failure is told");
    assert_eq!(failure, Some(CameraUnavailable.notice()));
    assert_eq!(fake.starts(), 2);
}

#[tokio::test]
async fn a_failure_after_the_panel_was_replaced_leaves_the_new_panel_shown() {
    // Exits only after the stats have replaced the camera.
    let fake = FakeProgram::new("sleep 1\nexit 1");
    let mut rig = Rig::start(&fake.program());
    rig.show(PanelName::Camera).await;
    let pid = fake.pid().await;
    rig.show(PanelName::Stats).await;
    wait_until_gone(&pid).await;
    sleep(Duration::from_millis(300)).await;
    assert_eq!(rig.panels.shown(), Some(PanelName::Stats));
    assert!(
        rig.failures.try_recv().is_err(),
        "a hidden camera is no failure"
    );
}

#[test]
fn the_frame_timeout_is_ten_seconds() {
    assert_eq!(FRAME_TIMEOUT, Duration::from_secs(10));
}

/// Lets the tasks run everything they can.
async fn settle() {
    for _ in 0..8 {
        yield_now().await;
    }
}

/// A camera whose frames a test sends itself, with no program.
struct Feeding {
    panels: PanelHandle,
    camera: Camera,
    feed: CameraFeed,
    _panel_task: JoinHandle<()>,
}

impl Feeding {
    fn start() -> Self {
        let (panels, inbox) = channel();
        let panel_task = tokio::spawn(async move { match inbox.run(None, PANEL_TIMEOUT).await {} });
        let (camera, feed) = Camera::new("unused", panels.clone());
        Self {
            panels,
            camera,
            feed,
            _panel_task: panel_task,
        }
    }

    async fn show(&self, panel: PanelName) {
        self.panels
            .show(panel)
            .await
            .expect("the panel state is running");
        settle().await;
    }

    fn film(&self, frame: &'static [u8]) {
        self.camera
            .frames
            .send_replace(Some(Frame::from_static(frame)));
    }
}

#[tokio::test(start_paused = true)]
async fn a_viewer_is_refused_while_the_camera_panel_is_not_shown() {
    let feeding = Feeding::start();
    settle().await;
    assert!(feeding.feed.watch().is_none(), "no panel");
    feeding.show(PanelName::Stats).await;
    assert!(feeding.feed.watch().is_none(), "the stats");
    feeding.show(PanelName::Camera).await;
    assert!(feeding.feed.watch().is_some(), "the camera");
}

#[tokio::test(start_paused = true)]
async fn a_slow_viewer_gets_only_the_newest_frame() {
    let feeding = Feeding::start();
    feeding.show(PanelName::Camera).await;
    let mut stream = feeding.feed.watch().expect("the camera is shown");

    feeding.film(FIRST_FRAME);
    feeding.film(SECOND_FRAME);
    assert_eq!(stream.next().await.as_deref(), Some(SECOND_FRAME));

    // Nothing else is waiting: the first frame was skipped, not queued.
    let waiting = tokio::time::timeout(Duration::from_secs(1), stream.next()).await;
    assert!(waiting.is_err(), "{waiting:?}");
}

#[tokio::test(start_paused = true)]
async fn a_new_viewer_gets_the_frame_already_there_first() {
    let feeding = Feeding::start();
    feeding.show(PanelName::Camera).await;
    feeding.film(FIRST_FRAME);
    let mut stream = feeding.feed.watch().expect("the camera is shown");
    assert_eq!(stream.next().await.as_deref(), Some(FIRST_FRAME));
}

#[tokio::test(start_paused = true)]
async fn a_viewer_waits_for_the_first_frame_and_ends_when_the_panel_goes() {
    let feeding = Feeding::start();
    feeding.show(PanelName::Camera).await;
    let mut stream = feeding.feed.watch().expect("the camera is shown");
    let waiting = tokio::time::timeout(Duration::from_secs(1), stream.next()).await;
    assert!(waiting.is_err(), "no frame yet: {waiting:?}");

    feeding.show(PanelName::Stats).await;
    assert_eq!(stream.next().await, None);
}

#[tokio::test(start_paused = true)]
async fn a_viewer_ends_when_the_camera_is_gone() {
    let feeding = Feeding::start();
    feeding.show(PanelName::Camera).await;
    let mut stream = feeding.feed.watch().expect("the camera is shown");
    let Feeding { camera, .. } = feeding;
    drop(camera);
    assert_eq!(stream.next().await, None);
}

#[tokio::test(start_paused = true)]
async fn no_frame_for_the_frame_timeout_makes_the_camera_unavailable() {
    let feeding = Feeding::start();
    feeding.show(PanelName::Camera).await;
    let mut shown = feeding.panels.watch_shown();
    shown.borrow_and_update();
    // An open output that never sends a byte, like a stalled camera.
    let (_silent, output) = tokio::io::duplex(64);

    let started = Instant::now();
    let ended = feeding.camera.read_frames(output, &mut shown).await;
    assert!(matches!(ended, Ended::Unavailable), "{ended:?}");
    assert_eq!(started.elapsed(), FRAME_TIMEOUT);
}

#[tokio::test(start_paused = true)]
async fn each_frame_restarts_the_frame_timeout() {
    let feeding = Feeding::start();
    feeding.show(PanelName::Camera).await;
    let mut shown = feeding.panels.watch_shown();
    shown.borrow_and_update();
    let (mut camera_output, output) = tokio::io::duplex(64);
    let almost = FRAME_TIMEOUT - Duration::from_millis(1);
    let writer = tokio::spawn(async move {
        sleep(almost).await;
        tokio::io::AsyncWriteExt::write_all(&mut camera_output, FIRST_FRAME)
            .await
            .expect("the reader is there");
        // Kept open and silent from here on.
        camera_output
    });

    let started = Instant::now();
    let ended = feeding.camera.read_frames(output, &mut shown).await;
    assert!(matches!(ended, Ended::Unavailable), "{ended:?}");
    assert_eq!(started.elapsed(), almost + FRAME_TIMEOUT);
    assert_eq!(feeding.camera.frames.borrow().as_deref(), Some(FIRST_FRAME));
    drop(writer);
}

#[tokio::test(start_paused = true)]
async fn hiding_ends_the_reading_before_a_frame_that_is_ready() {
    let feeding = Feeding::start();
    feeding.show(PanelName::Camera).await;
    let mut shown = feeding.panels.watch_shown();
    shown.borrow_and_update();
    feeding.show(PanelName::Stats).await;
    let output: &[u8] = FIRST_FRAME;

    let ended = feeding.camera.read_frames(output, &mut shown).await;
    assert!(matches!(ended, Ended::Hidden), "{ended:?}");
    assert_eq!(*feeding.camera.frames.borrow(), None);
}

#[test]
fn only_the_camera_panel_films() {
    assert!(is_camera(Some(PanelName::Camera)));
    assert!(!is_camera(Some(PanelName::Stats)));
    assert!(!is_camera(Some(PanelName::Built)));
    assert!(!is_camera(None));
}
