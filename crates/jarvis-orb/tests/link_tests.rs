use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use jarvis_orb::link::{Display, LineSplitter, Link, Timings, socket_path};
use jarvis_orb::look::View;
use orb_protocol::{LevelSource, MAX_LINE_BYTES, OrbState, ToOrb};

// ---- pure pieces ----

#[test]
fn the_socket_path_follows_jarvis_config_rule() {
    let cwd = Path::new("/work");
    let os = |value: &str| Some(OsString::from(value));
    let cases = [
        (os("/s/a.sock"), os("/run/user/1000"), "/s/a.sock"),
        (os("rel.sock"), os("/run/user/1000"), "/work/rel.sock"),
        (None, os("/run/user/1000"), "/run/user/1000/jarvis-orb.sock"),
        (
            os(""),
            os("/run/user/1000"),
            "/run/user/1000/jarvis-orb.sock",
        ),
        (None, os("relative/dir"), "/work/data/jarvis-orb.sock"),
        (None, os(""), "/work/data/jarvis-orb.sock"),
        (None, None, "/work/data/jarvis-orb.sock"),
    ];
    for (socket, runtime, expected) in cases {
        assert_eq!(
            socket_path(socket.clone(), runtime.clone(), cwd),
            PathBuf::from(expected),
            "{socket:?} {runtime:?}"
        );
    }
}

#[test]
fn a_line_of_256_bytes_is_read_and_257_is_skipped_without_losing_the_next() {
    let mut splitter = LineSplitter::new();
    let ok = "a".repeat(MAX_LINE_BYTES);
    let long = "b".repeat(MAX_LINE_BYTES + 1);
    let input = format!("{ok}\n{long}\nnext\n");
    assert_eq!(splitter.feed(input.as_bytes()), vec![ok, "next".to_owned()]);
}

#[test]
fn lines_split_across_reads_are_joined_and_an_oversized_one_is_skipped_across_reads() {
    let mut splitter = LineSplitter::new();
    assert!(splitter.feed(b"hel").is_empty());
    assert_eq!(splitter.feed(b"lo\nwor"), vec!["hello".to_owned()]);
    let long = "x".repeat(MAX_LINE_BYTES);
    assert!(splitter.feed(long.as_bytes()).is_empty());
    assert!(splitter.feed(b"yyy").is_empty());
    assert_eq!(splitter.feed(b"zzz\nok\n"), vec!["ok".to_owned()]);
}

#[test]
fn a_line_that_is_not_utf8_is_dropped() {
    let mut splitter = LineSplitter::new();
    assert_eq!(splitter.feed(b"\xff\xfe\nfine\n"), vec!["fine".to_owned()]);
}

#[test]
fn state_and_level_messages_drive_the_display() {
    let level = |value: f32, source| ToOrb::Level {
        level: orb_protocol::Level::new(value).expect("in range"),
        source,
    };
    let state = |state| ToOrb::State { state };
    let mut display = Display::Offline;
    display.apply(state(OrbState::Listening));
    assert_eq!(display, Display::Offline, "offline ignores messages");

    let mut display = Display::CONNECTED;
    assert_eq!(display.view(), View::State(OrbState::Idle));
    display.apply(level(0.5, LevelSource::Mic));
    assert_eq!(display.level(), 0.0, "idle has no level");
    display.apply(state(OrbState::Listening));
    display.apply(level(0.5, LevelSource::Mic));
    assert_eq!(display.level(), 0.5);
    display.apply(level(0.9, LevelSource::Speech));
    assert_eq!(display.level(), 0.5, "wrong source is ignored");
    display.apply(state(OrbState::Speaking));
    assert_eq!(display.level(), 0.0, "a state change resets the level");
    display.apply(level(0.7, LevelSource::Speech));
    assert_eq!(display.level(), 0.7);
    display.apply(state(OrbState::Thinking));
    display.apply(level(0.7, LevelSource::Speech));
    assert_eq!(display.view(), View::State(OrbState::Thinking));
    assert_eq!(display.level(), 0.0);
}

// ---- the link against a fake server ----

const TEST_TIMINGS: Timings = Timings {
    reconnect: Duration::from_millis(50),
    read_poll: Duration::from_millis(5),
    write: Duration::from_millis(200),
};
const DEADLINE: Duration = Duration::from_millis(1500);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "jarvis-orb-link-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create the temporary directory");
        Self(dir)
    }

    fn socket(&self) -> PathBuf {
        self.0.join("orb.sock")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + DEADLINE;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Starts a link and accepts its connection.
fn connect(dir: &TempDir) -> (Link, UnixStream) {
    let listener = UnixListener::bind(dir.socket()).expect("bind");
    let link = Link::start_with(dir.socket(), TEST_TIMINGS);
    let (server, _) = listener.accept().expect("accept");
    wait_until("the link to be online", || link.view() != View::Offline);
    (link, server)
}

#[test]
fn state_and_level_lines_drive_the_view_and_level() {
    let dir = TempDir::new();
    let (link, mut server) = connect(&dir);
    server
        .write_all(b"{\"type\":\"state\",\"state\":\"listening\"}\n{\"type\":\"level\",\"level\":0.25,\"source\":\"mic\"}\n")
        .expect("write");
    wait_until("the level", || link.level() == 0.25);
    assert_eq!(link.view(), View::State(OrbState::Listening));
}

#[test]
fn bad_lines_are_ignored_and_the_connection_stays_up() {
    let dir = TempDir::new();
    let (link, mut server) = connect(&dir);
    let oversized = format!("{{\"type\":\"state\",\"pad\":\"{}\"}}\n", "x".repeat(300));
    server.write_all(b"not json\n").expect("write");
    server.write_all(oversized.as_bytes()).expect("write");
    server
        .write_all(b"{\"type\":\"mystery\"}\n")
        .expect("write");
    server
        .write_all(b"{\"type\":\"state\",\"state\":\"thinking\"}\n")
        .expect("write");
    wait_until("the good line after the bad ones", || {
        link.view() == View::State(OrbState::Thinking)
    });
}

#[test]
fn the_link_goes_offline_when_the_server_closes_and_reconnects_to_a_new_one() {
    let dir = TempDir::new();
    let (link, server) = connect(&dir);
    drop(server);
    wait_until("offline", || link.view() == View::Offline);
    assert_eq!(link.level(), 0.0);

    let _ = std::fs::remove_file(dir.socket());
    let listener = UnixListener::bind(dir.socket()).expect("rebind");
    let (mut server, _) = listener.accept().expect("the link reconnects");
    server
        .write_all(b"{\"type\":\"state\",\"state\":\"speaking\"}\n")
        .expect("write");
    wait_until("the new state", || {
        link.view() == View::State(OrbState::Speaking)
    });
}

fn read_line(reader: &mut BufReader<UnixStream>) -> Option<String> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line),
    }
}

#[test]
fn a_tap_while_connected_sends_exactly_one_tap_line() {
    let dir = TempDir::new();
    let (link, server) = connect(&dir);
    server
        .set_read_timeout(Some(Duration::from_millis(300)))
        .expect("timeout");
    let mut reader = BufReader::new(server);
    link.tap();
    assert_eq!(
        read_line(&mut reader).as_deref(),
        Some("{\"type\":\"tap\"}\n")
    );
    assert_eq!(read_line(&mut reader), None, "no second line");
}

#[test]
fn a_tap_while_offline_sends_nothing_even_after_reconnecting() {
    let dir = TempDir::new();
    let (link, server) = connect(&dir);
    drop(server);
    wait_until("offline", || link.view() == View::Offline);
    link.tap();

    let _ = std::fs::remove_file(dir.socket());
    let listener = UnixListener::bind(dir.socket()).expect("rebind");
    let (server, _) = listener.accept().expect("the link reconnects");
    server
        .set_read_timeout(Some(Duration::from_millis(300)))
        .expect("timeout");
    let mut reader = BufReader::new(server);
    assert_eq!(read_line(&mut reader), None, "no stale tap");
}

#[test]
fn dropping_the_link_closes_the_connection() {
    let dir = TempDir::new();
    let (link, mut server) = connect(&dir);
    server.set_read_timeout(Some(DEADLINE)).expect("timeout");
    drop(link);
    let mut buffer = [0_u8; 8];
    assert_eq!(server.read(&mut buffer).expect("a clean close"), 0);
}
