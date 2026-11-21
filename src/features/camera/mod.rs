//! The camera Panel's picture (jarvis-ui ticket 11).
//!
//! [`Camera::run`] follows the shown Panel, as the stats source does: while the
//! camera Panel shows, it runs `rpicam-vid`, cuts its output into JPEG frames
//! ([`frames`]) and keeps only the newest; when the Panel is hidden or replaced,
//! or the orb closes it (a tap outside, or the UI going away), it kills and reaps
//! the program. A [`CameraFeed`] hands the frames to the orb's `/camera` route,
//! only while the camera Panel shows.
//!
//! If the program cannot start, exits, or sends no frame for [`FRAME_TIMEOUT`],
//! the Panel closes and a [`CameraUnavailable`] is reported once. Nothing is
//! retried until the camera is asked for again (C6).
//!
//! Jarvis never looks at a frame: no frame is stored, logged or sent anywhere
//! but the local `/camera` route (C7).

pub(crate) mod frames;

use std::{process::Stdio, time::Duration};

use futures_util::StreamExt;
use orb_protocol::PanelName;
use tokio::{
    io::AsyncRead,
    process::{Child, Command},
    sync::{mpsc, watch},
    time::timeout,
};
use tokio_util::{bytes::Bytes, codec::FramedRead};

use crate::features::{control::Notice, panel::PanelHandle};
use frames::JpegFrames;

/// The program that films; tests substitute a fake script.
pub(crate) const CAMERA_PROGRAM: &str = "rpicam-vid";
/// The picture's width in pixels (C1). 640x480 is the fallback if the Pi's CPU
/// cost is too high (D11).
pub(crate) const FRAME_WIDTH: u32 = 800;
/// The picture's height in pixels (C1).
pub(crate) const FRAME_HEIGHT: u32 = 450;
/// Frames a second (C1).
pub(crate) const FRAME_RATE: u32 = 30;
/// How long the program may go without sending a frame, from its start or from
/// the last frame, before the camera counts as unavailable. The camera starts
/// in about a second; a stalled one would otherwise show a still picture forever.
pub(crate) const FRAME_TIMEOUT: Duration = Duration::from_secs(10);
/// What Jarvis says when the camera cannot be shown (C6).
pub(crate) const CAMERA_UNAVAILABLE: &str = "The camera isn't available.";
/// Failures waiting to be told. One is enough: a second says the same thing.
pub(crate) const FAILURE_QUEUE: usize = 1;

/// One JPEG, as `rpicam-vid` wrote it.
pub(crate) type Frame = Bytes;

/// The camera could not start, or stopped by itself while its Panel showed. Its
/// Panel has been closed; [`CameraUnavailable::notice`] is what to tell Darren.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CameraUnavailable;

impl CameraUnavailable {
    /// The line the terminal shows and says, as an interface-neutral notice.
    pub(crate) fn notice(self) -> Notice {
        Notice::Warning(CAMERA_UNAVAILABLE.to_owned())
    }
}

/// Whether `shown` is the camera Panel. Exhaustive on purpose: a new Panel must
/// say whether it films.
pub(crate) fn is_camera(shown: Option<PanelName>) -> bool {
    match shown {
        Some(PanelName::Camera) => true,
        Some(PanelName::Stats) | None => false,
    }
}

/// Runs the camera program while the camera Panel shows.
pub(crate) struct Camera {
    program: String,
    panels: PanelHandle,
    /// The newest frame, `None` while the program is not running.
    frames: watch::Sender<Option<Frame>>,
}

/// Why one run of the program ended.
#[derive(Debug)]
enum Ended {
    /// The Panel was hidden or replaced.
    Hidden,
    /// The program could not start, stopped, or stalled.
    Unavailable,
    /// The panel state is gone: Jarvis is stopping.
    PanelsGone,
}

impl Camera {
    /// `program` is [`CAMERA_PROGRAM`] in production. The feed reads what this
    /// camera films.
    pub(crate) fn new(program: impl Into<String>, panels: PanelHandle) -> (Self, CameraFeed) {
        let (frames, frames_receiver) = watch::channel(None);
        let feed = CameraFeed {
            frames: frames_receiver,
            shown: panels.watch_shown(),
        };
        let camera = Self {
            program: program.into(),
            panels,
            frames,
        };
        (camera, feed)
    }

    /// Runs until the panel state is gone. Each failure closes the Panel and its
    /// notice is sent to `failures` (dropped if one is already waiting or nobody
    /// listens).
    ///
    /// Safe to drop at any `.await`: the program is spawned with `kill_on_drop`,
    /// so dropping this future kills it; every other stop kills and reaps it.
    pub(crate) async fn run(self, failures: mpsc::Sender<Notice>) {
        let mut shown = self.panels.watch_shown();
        loop {
            while !is_camera(*shown.borrow_and_update()) {
                if shown.changed().await.is_err() {
                    return;
                }
            }
            match self.film(&mut shown).await {
                Ended::Hidden => {}
                Ended::PanelsGone => return,
                Ended::Unavailable => {
                    // The Panel may have been replaced a moment ago; that one stays.
                    if is_camera(self.panels.shown()) {
                        let _ = self.panels.close().await;
                    }
                    let _ = failures.try_send(CameraUnavailable.notice());
                    // Wait for the close to land, so this show is not tried again.
                    while is_camera(*shown.borrow_and_update()) {
                        if shown.changed().await.is_err() {
                            return;
                        }
                    }
                }
            }
        }
    }

    /// One stretch of the camera Panel showing: the program runs until the Panel
    /// goes or the program fails, and is then killed and reaped.
    async fn film(&self, shown: &mut watch::Receiver<Option<PanelName>>) -> Ended {
        let Ok(mut child) = self.spawn() else {
            return Ended::Unavailable;
        };
        let ended = match child.stdout.take() {
            Some(output) => self.read_frames(output, shown).await,
            None => Ended::Unavailable,
        };
        // A later viewer must never see this stretch's last frame.
        self.frames.send_replace(None);
        // Already exited or not, this ends and reaps it. If it fails, dropping
        // the child still kills it.
        let _ = child.kill().await;
        ended
    }

    /// Publishes each frame in `output` until the Panel goes, `output` ends or
    /// breaks, or no frame comes for [`FRAME_TIMEOUT`].
    async fn read_frames(
        &self,
        output: impl AsyncRead + Unpin,
        shown: &mut watch::Receiver<Option<PanelName>>,
    ) -> Ended {
        let mut frames = FramedRead::new(output, JpegFrames::default());
        loop {
            tokio::select! {
                // Hiding wins over a frame that is ready at the same moment.
                biased;
                changed = shown.changed() => {
                    if changed.is_err() {
                        return Ended::PanelsGone;
                    }
                    if !is_camera(*shown.borrow_and_update()) {
                        return Ended::Hidden;
                    }
                }
                frame = timeout(FRAME_TIMEOUT, frames.next()) => match frame {
                    Ok(Some(Ok(frame))) => {
                        self.frames.send_replace(Some(frame));
                    }
                    // It exited, its output broke, or it stalled.
                    Ok(Some(Err(_)) | None) | Err(_) => return Ended::Unavailable,
                },
            }
        }
    }

    fn spawn(&self) -> std::io::Result<Child> {
        Command::new(&self.program)
            .args(arguments())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
    }
}

/// MJPEG at [`FRAME_WIDTH`]x[`FRAME_HEIGHT`] and [`FRAME_RATE`], mirrored like
/// a looking glass, without a preview window, until killed, to stdout (C1).
fn arguments() -> Vec<String> {
    [
        "--width".to_owned(),
        FRAME_WIDTH.to_string(),
        "--height".to_owned(),
        FRAME_HEIGHT.to_string(),
        "--framerate".to_owned(),
        FRAME_RATE.to_string(),
    ]
    .into_iter()
    .chain(
        [
            "--hflip",
            "--codec",
            "mjpeg",
            "--nopreview",
            "-t",
            "0",
            "-o",
            "-",
        ]
        .map(str::to_owned),
    )
    .collect()
}

/// What the `/camera` route reads: the newest frame, while the camera Panel
/// shows. Cheap to clone.
#[derive(Clone)]
pub(crate) struct CameraFeed {
    frames: watch::Receiver<Option<Frame>>,
    shown: watch::Receiver<Option<PanelName>>,
}

impl CameraFeed {
    /// The frames from now on, or `None` while the camera Panel is not shown, so
    /// that nothing but showing the Panel lets the picture out (C4).
    pub(crate) fn watch(&self) -> Option<FrameStream> {
        let mut shown = self.shown.clone();
        if !is_camera(*shown.borrow_and_update()) {
            return None;
        }
        let mut frames = self.frames.clone();
        // The frame already there, if any, goes first.
        frames.mark_changed();
        Some(FrameStream { frames, shown })
    }
}

/// One viewer's frames.
pub(crate) struct FrameStream {
    frames: watch::Receiver<Option<Frame>>,
    shown: watch::Receiver<Option<PanelName>>,
}

impl FrameStream {
    /// The newest frame not yet given out, waiting for one if needed; `None`
    /// once the camera Panel is no longer shown. A viewer that reads slowly
    /// skips the frames it missed and never builds a backlog (C3).
    ///
    /// Safe to drop at any `.await`.
    pub(crate) async fn next(&mut self) -> Option<Frame> {
        loop {
            tokio::select! {
                biased;
                changed = self.shown.changed() => {
                    if changed.is_err() || !is_camera(*self.shown.borrow_and_update()) {
                        return None;
                    }
                }
                changed = self.frames.changed() => {
                    changed.ok()?;
                    if let Some(frame) = self.frames.borrow_and_update().clone() {
                        return Some(frame);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/camera/camera_tests.rs"]
mod tests;
