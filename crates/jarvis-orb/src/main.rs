//! `jarvis-orb`: draws the orb full screen. Until the socket to Jarvis exists, a
//! stand-in driver (keys, clicks and an automatic demo) chooses the view.
//!
//! Keys: Space or Right = next view, Left = previous view, Up/Down = raise or
//! lower the fake level, A = toggle the automatic demo, Escape = quit. A left
//! click (a finger tap on the Pi) also advances the view.

mod orb;

use std::io::Write;

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

const USAGE: &str = "usage: jarvis-orb [--demo] [--windowed] [--seconds S] [--fps]";

struct Options {
    demo: bool,
    windowed: bool,
    seconds: Option<f64>,
    fps: bool,
}

enum ArgError {
    Unknown(String),
    BadSeconds,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, ArgError> {
        let mut options = Self {
            demo: false,
            windowed: false,
            seconds: None,
            fps: false,
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
    };

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
        orb.draw(&current, dt);

        if let Some(meter) = &mut meter {
            meter.record(now, dt);
        }
        next_frame().await;
    }
}
