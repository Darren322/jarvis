use std::{fs, net::SocketAddr, os::unix::fs::PermissionsExt, time::Duration};

use orb_protocol::PanelName;
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc,
    time::{Instant, sleep, timeout},
};

use super::*;
use crate::features::{
    camera::{Camera, FAILURE_QUEUE},
    panel::{PANEL_TIMEOUT, PanelHandle, channel},
};
use crate::interfaces::orb::tests::{serving, serving_with};

/// Long enough for a shell script to start and write, on a busy machine.
const WAIT: Duration = Duration::from_secs(5);
const FRAME: &[u8] = b"\xFF\xD8a picture\xFF\xD9";

/// Asks for the picture and returns the connection, the reply still unread.
async fn request_camera(address: SocketAddr) -> TcpStream {
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream
        .write_all(format!("GET {CAMERA_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
        .await
        .unwrap();
    stream
}

/// Reads until what was read holds `wanted`, and returns all of it.
async fn read_until(stream: &mut TcpStream, read: &mut Vec<u8>, wanted: &[u8]) {
    let deadline = Instant::now() + WAIT;
    let mut chunk = [0u8; 4096];
    while !read.windows(wanted.len()).any(|window| window == wanted) {
        let count = timeout(deadline - Instant::now(), stream.read(&mut chunk))
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "never read {wanted:?} in {:?}",
                    String::from_utf8_lossy(read)
                )
            })
            .unwrap();
        assert_ne!(count, 0, "closed before {wanted:?}");
        read.extend_from_slice(&chunk[..count]);
    }
}

/// The real panel state (with no orb) and a camera running `body` as its program.
struct Filming {
    panels: PanelHandle,
    _dir: TempDir,
    _tasks: [tokio::task::JoinHandle<()>; 2],
}

impl Filming {
    fn start(body: &str) -> (Self, crate::features::camera::CameraFeed) {
        let dir = TempDir::new().unwrap();
        let program = dir.path().join("fake-rpicam-vid");
        fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        let (panels, inbox) = channel();
        let panel_task = tokio::spawn(async move { match inbox.run(None, PANEL_TIMEOUT).await {} });
        let (camera, feed) = Camera::new(program.display().to_string(), panels.clone());
        let (failures, _) = mpsc::channel(FAILURE_QUEUE);
        let camera_task = tokio::spawn(camera.run(failures));
        let filming = Self {
            panels,
            _dir: dir,
            _tasks: [panel_task, camera_task],
        };
        (filming, feed)
    }

    async fn show(&self, panel: PanelName) {
        self.panels.show(panel).await.unwrap();
        let deadline = Instant::now() + WAIT;
        while self.panels.shown() != Some(panel) {
            assert!(
                Instant::now() < deadline,
                "the panel state never showed {panel:?}"
            );
            sleep(Duration::from_millis(10)).await;
        }
    }
}

impl Drop for Filming {
    fn drop(&mut self) {
        for task in &self._tasks {
            task.abort();
        }
    }
}

#[tokio::test]
async fn the_picture_is_refused_while_the_camera_panel_is_not_shown() {
    serving(async |served| {
        let mut stream = request_camera(served.address).await;
        let mut read = Vec::new();
        read_until(&mut stream, &mut read, b"\r\n").await;
        assert!(
            read.starts_with(b"HTTP/1.1 403"),
            "{:?}",
            String::from_utf8_lossy(&read)
        );
    })
    .await;
}

#[tokio::test]
async fn while_the_camera_panel_shows_its_frames_stream_as_mjpeg_until_it_goes() {
    let (filming, feed) = Filming::start("printf '\\377\\330a picture\\377\\331'\nexec sleep 30");
    filming.show(PanelName::Camera).await;
    serving_with(feed, async |served| {
        let mut stream = request_camera(served.address).await;
        let mut read = Vec::new();
        read_until(&mut stream, &mut read, &part(FRAME)).await;
        let head = String::from_utf8_lossy(&read).to_ascii_lowercase();
        assert!(head.starts_with("http/1.1 200"), "{head}");
        assert!(
            head.contains("content-type: multipart/x-mixed-replace; boundary=frame\r\n"),
            "{head}"
        );
        assert!(head.contains("cache-control: no-store\r\n"), "{head}");

        filming.show(PanelName::Stats).await;
        // The chunked body's last chunk: the response is over.
        read_until(&mut stream, &mut read, b"\r\n0\r\n\r\n").await;
    })
    .await;
}

#[test]
fn a_part_is_its_boundary_headers_and_jpeg() {
    assert_eq!(
        part(FRAME).as_ref(),
        b"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: 13\r\n\r\n\xFF\xD8a picture\xFF\xD9\r\n"
    );
}
