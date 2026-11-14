//! `jarvis-orb`: draws the orb full screen. Until the socket to Jarvis exists, a
//! stand-in driver (keys, clicks and an automatic demo) chooses the view.
//!
//! Keys: Space or Right = next view, Left = previous view, Up/Down = raise or
//! lower the fake level, A = toggle the automatic demo, Escape = quit. A left
//! click (a finger tap on the Pi) also advances the view.
//!
//! `--snapshot <dir>` draws each of the six views at a fixed level and animation
//! step, saves `<dir>/<n>-<view>.png` for each, and exits, so the picture can be
//! reviewed without the Pi. It works together with `--windowed`.

mod orb;

use std::io::Write;
use std::path::{Path, PathBuf};

use jarvis_orb::look::{self, Look, View, ease_factor};
use macroquad::miniquad::conf::{LinuxBackend, Platform};
use macroquad::prelude::*;
use orb::Orb;
use orb_protocol::OrbState;

const WINDOW_TITLE: &str = "jarvis-orb";
const SCREEN_WIDTH: i32 = 800;
const SCREEN_HEIGHT: i32 = 480;
/// How long the automatic demo stays on each view.
const DEMO_VIEW_SECONDS: f64 = 5.0;
/// The fake level starts at a typical speech loudness.
const START_LEVEL: f32 = 0.05;
/// How fast Up/Down change the fake level, per second.
const LEVEL_KEY_RATE: f32 = 0.25;
/// The synthetic speech level peaks near this loudness.
const DEMO_LEVEL_PEAK: f32 = 0.08;
/// Syllables per second in the synthetic speech.
const DEMO_SYLLABLE_RATE: f64 = 4.0;
/// Phrases per second in the synthetic speech (the slow envelope).
const DEMO_PHRASE_RATE: f64 = 0.45;
const FPS_REPORT_SECONDS: f64 = 1.0;
/// Snapshots advance the animation by this fixed step per frame, not real time.
const SNAPSHOT_STEP_SECONDS: f32 = 1.0 / 60.0;
/// Frames drawn per view before it is saved: 0.67 s, twice the time the easing
/// needs to settle (about a third of a second).
const SNAPSHOT_SETTLE_FRAMES: u32 = 40;
/// The raw level every snapshot is drawn at, loud enough to show the level-driven views.
const SNAPSHOT_LEVEL: f32 = 0.08;
/// File-name word for each view, in `View::ALL` order.
const VIEW_NAMES: [&str; 6] = [
    "offline",
    "idle",
    "listening",
    "transcribing",
    "thinking",
    "speaking",
];

const USAGE: &str =
    "usage: jarvis-orb [--demo] [--windowed] [--seconds S] [--fps] [--snapshot DIR]";

struct Options {
    demo: bool,
    windowed: bool,
    seconds: Option<f64>,
    fps: bool,
    snapshot: Option<PathBuf>,
}

enum ArgError {
    Unknown(String),
    BadSeconds,
    BadSnapshot,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, ArgError> {
        let mut options = Self {
            demo: false,
            windowed: false,
            seconds: None,
            fps: false,
            snapshot: None,
        };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--demo" => options.demo = true,
                "--windowed" => options.windowed = true,
                "--fps" => options.fps = true,
                "--seconds" => {
                    let seconds = args
                        .next()
                        .and_then(|value| value.parse::<f64>().ok())
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .ok_or(ArgError::BadSeconds)?;
                    options.seconds = Some(seconds);
                }
                "--snapshot" => {
                    options.snapshot = Some(
                        args.next()
                            .map(PathBuf::from)
                            .ok_or(ArgError::BadSnapshot)?,
                    );
                }
                _ => return Err(ArgError::Unknown(arg)),
            }
        }
        Ok(options)
    }
}

fn window_conf() -> Conf {
    let windowed = Options::parse(std::env::args().skip(1)).is_ok_and(|options| options.windowed);
    // Wayland only on the Pi (miniquad defaults to X11). A desktop test window may
    // fall back to X11.
    let linux_backend = if windowed {
        LinuxBackend::WaylandWithX11Fallback
    } else {
        LinuxBackend::WaylandOnly
    };
    Conf {
        window_title: WINDOW_TITLE.to_owned(),
        window_width: SCREEN_WIDTH,
        window_height: SCREEN_HEIGHT,
        fullscreen: !windowed,
        platform: Platform {
            linux_backend,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// The stand-in for the socket: decides the view and the level.
struct Driver {
    view: View,
    manual_level: f32,
    demo: bool,
    /// Seconds into the current demo view.
    demo_elapsed: f64,
}

impl Driver {
    fn new(demo: bool) -> Self {
        Self {
            view: if demo {
                View::Offline
            } else {
                View::State(OrbState::Idle)
            },
            manual_level: START_LEVEL,
            demo,
            demo_elapsed: 0.0,
        }
    }

    fn previous_view(&mut self) {
        // Five steps forward is one step back in a cycle of six.
        for _ in 1..View::ALL.len() {
            self.view = self.view.next();
        }
    }

    /// Applies this frame's input and returns the level to show.
    fn update(&mut self, dt: f32, clock: f64) -> f32 {
        if is_key_pressed(KeyCode::A) {
            self.demo = !self.demo;
            self.demo_elapsed = 0.0;
        }
        if is_key_pressed(KeyCode::Space)
            || is_key_pressed(KeyCode::Right)
            || is_mouse_button_pressed(MouseButton::Left)
        {
            self.view = self.view.next();
            self.demo_elapsed = 0.0;
        }
        if is_key_pressed(KeyCode::Left) {
            self.previous_view();
            self.demo_elapsed = 0.0;
        }
        if is_key_down(KeyCode::Up) {
            self.manual_level += LEVEL_KEY_RATE * dt;
        }
        if is_key_down(KeyCode::Down) {
            self.manual_level -= LEVEL_KEY_RATE * dt;
        }
        self.manual_level = self.manual_level.clamp(0.0, 1.0);

        if self.demo {
            self.demo_elapsed += f64::from(dt);
            if self.demo_elapsed >= DEMO_VIEW_SECONDS {
                self.demo_elapsed = 0.0;
                self.view = self.view.next();
            }
            synthetic_speech_level(clock)
        } else {
            self.manual_level
        }
    }
}

/// A fake raw level shaped like speech: quick syllables inside slower phrases
/// with a pause between them.
fn synthetic_speech_level(clock: f64) -> f32 {
    let tau = std::f64::consts::TAU;
    let syllable = 0.5 + 0.5 * (clock * tau * DEMO_SYLLABLE_RATE).sin();
    let phrase = (0.5 + 0.5 * (clock * tau * DEMO_PHRASE_RATE).sin()).powi(2);
    // Both factors are within 0..1, so narrowing to f32 is exact enough.
    (syllable * phrase) as f32 * DEMO_LEVEL_PEAK
}

/// Frame-rate figures for the `--fps` report.
struct FpsMeter {
    window_start: f64,
    frames: u32,
    shortest: f32,
    longest: f32,
}

impl FpsMeter {
    fn new(now: f64) -> Self {
        Self {
            window_start: now,
            frames: 0,
            shortest: f32::MAX,
            longest: 0.0,
        }
    }

    fn record(&mut self, now: f64, frame_seconds: f32) {
        self.frames += 1;
        self.shortest = self.shortest.min(frame_seconds);
        self.longest = self.longest.max(frame_seconds);
        let elapsed = now - self.window_start;
        if elapsed >= FPS_REPORT_SECONDS {
            // Frame counts per second are small, so the conversion is exact.
            let average = f64::from(self.frames) / elapsed;
            let mut out = std::io::stdout();
            // A closed stdout must not stop the orb.
            let _ = writeln!(
                out,
                "avg_fps={average:.1} min_frame_ms={:.2} max_frame_ms={:.2}",
                self.shortest * 1000.0,
                self.longest * 1000.0
            );
            let _ = out.flush();
            *self = Self::new(now);
        }
    }
}

/// Draws every view in turn into an off-screen target of the reference screen
/// size, and saves one PNG per view into `dir`. Drawing off screen keeps the
/// pictures the same size on every desktop, whatever the window manager does to
/// the window.
fn save_snapshots(dir: &Path, orb: &mut Orb) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    // 800 and 480 are small whole numbers, exact in `f32`.
    let area = vec2(SCREEN_WIDTH as f32, SCREEN_HEIGHT as f32);
    // Both are positive constants, so the conversions are exact.
    let target = render_target(SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32);
    let camera = Camera2D {
        render_target: Some(target.clone()),
        ..Camera2D::from_display_rect(Rect::new(0.0, 0.0, area.x, area.y))
    };
    let mut current = look::look(View::ALL[0], SNAPSHOT_LEVEL);
    for (index, (view, name)) in View::ALL.into_iter().zip(VIEW_NAMES).enumerate() {
        let path = dir.join(format!("{}-{name}.png", index + 1));
        // `export_png` panics on a write error, so prove the file can be created first.
        std::fs::File::create(&path)?;
        let path = path.to_str().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "path is not valid UTF-8")
        })?;
        let aim = look::look(view, SNAPSHOT_LEVEL);
        for _ in 0..=SNAPSHOT_SETTLE_FRAMES {
            current = current.lerp(&aim, ease_factor(SNAPSHOT_STEP_SECONDS));
            set_camera(&camera);
            clear_background(BLACK);
            orb.draw(&current, SNAPSHOT_STEP_SECONDS, area);
            set_default_camera();
        }
        let mut image = target.texture.get_texture_data();
        // Blending lowers the target's alpha channel. The colours are already
        // blended over black, so make the picture opaque before saving.
        for pixel in image.bytes.as_chunks_mut::<4>().0 {
            pixel[3] = u8::MAX;
        }
        image.export_png(path);
    }
    Ok(())
}

#[macroquad::main(window_conf)]
async fn main() {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(ArgError::Unknown(arg)) => {
            eprintln!("unknown argument {arg}\n{USAGE}");
            std::process::exit(2);
        }
        Err(ArgError::BadSeconds) => {
            eprintln!("--seconds needs a number of seconds\n{USAGE}");
            std::process::exit(2);
        }
        Err(ArgError::BadSnapshot) => {
            eprintln!("--snapshot needs a directory\n{USAGE}");
            std::process::exit(2);
        }
    };

    if let Some(dir) = &options.snapshot {
        if let Err(error) = save_snapshots(dir, &mut Orb::new()) {
            eprintln!("snapshot failed in {}: {error}", dir.display());
            std::process::exit(1);
        }
        return;
    }

    if !options.windowed {
        show_mouse(false);
    }
    let mut driver = Driver::new(options.demo);
    let mut orb = Orb::new();
    let mut current = look::look(driver.view, 0.0);
    let mut meter = options.fps.then(|| FpsMeter::new(get_time()));

    loop {
        let dt = get_frame_time();
        let now = get_time();
        if is_key_pressed(KeyCode::Escape) || options.seconds.is_some_and(|limit| now >= limit) {
            break;
        }

        let level = driver.update(dt, now);
        let target: Look = look::look(driver.view, level);
        current = current.lerp(&target, ease_factor(dt));

        clear_background(BLACK);
        orb.draw(&current, dt, vec2(screen_width(), screen_height()));

        if let Some(meter) = &mut meter {
            meter.record(now, dt);
        }
        next_frame().await;
    }
}
