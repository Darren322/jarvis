//! The `/camera` route: the camera Panel's picture, for the UI's `<img>`.
//!
//! It answers `multipart/x-mixed-replace` MJPEG, which Chromium shows in a plain
//! `<img>`: each part is one JPEG and replaces the one before. Only the newest
//! frame is sent, so a slow page skips frames instead of falling behind.
//!
//! An `<img>` request carries no `Origin`, so this route cannot be origin-checked
//! like the WebSocket. It relies on the `127.0.0.1` bind and on refusing every
//! request while the camera Panel is not shown (jarvis-ui ticket 11, C4): the
//! route never starts the camera, it only shows what showing the Panel started.
//! The response ends when the Panel goes.

use std::convert::Infallible;

use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{
        StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};
use futures_util::{Stream, stream};

use crate::features::camera::{CameraFeed, FrameStream};

/// The route's path; the UI builds the same URL from the WebSocket's host and port.
pub(super) const CAMERA_PATH: &str = "/camera";
/// What separates two frames in the response.
const BOUNDARY: &str = "frame";

/// One viewer of the picture, or 403 while the camera Panel is not shown. Every
/// frame is fresh, so nothing may be cached.
pub(super) async fn camera(State(feed): State<CameraFeed>) -> Response {
    let Some(frames) = feed.watch() else {
        return StatusCode::FORBIDDEN.into_response();
    };
    (
        [
            (
                CONTENT_TYPE,
                format!("multipart/x-mixed-replace; boundary={BOUNDARY}"),
            ),
            (CACHE_CONTROL, "no-store".to_owned()),
        ],
        Body::from_stream(parts(frames)),
    )
        .into_response()
}

/// Each frame as one part, until the camera Panel goes.
fn parts(frames: FrameStream) -> impl Stream<Item = Result<Bytes, Infallible>> + Send + 'static {
    stream::unfold(frames, |mut frames| async move {
        let frame = frames.next().await?;
        Some((Ok(part(&frame)), frames))
    })
}

/// One JPEG with its part header, ending in the line break before the next
/// boundary.
fn part(frame: &[u8]) -> Bytes {
    let header = format!(
        "--{BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
        frame.len()
    );
    let mut part = Vec::with_capacity(header.len() + frame.len() + 2);
    part.extend_from_slice(header.as_bytes());
    part.extend_from_slice(frame);
    part.extend_from_slice(b"\r\n");
    Bytes::from(part)
}

#[cfg(test)]
#[path = "../../../tests/unit/interfaces/orb/camera_tests.rs"]
mod tests;
