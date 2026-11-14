use super::*;
use tempfile::TempDir;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    net::unix::{OwnedReadHalf, OwnedWriteHalf},
    time::Instant,
};

/// Long enough for anything the server will ever send; the clock is paused, so
/// waiting it out costs nothing.
const WAIT: Duration = Duration::from_secs(2);
const TAP_LINE: &[u8] = b"{\"type\":\"tap\"}\n";
/// Levels sent while one orb is not reading: enough to overfill its socket buffer.
const PILED_UP_LEVELS: usize = 20_000;

/// What a test holds while the server runs beside it.
struct Served<'a> {
    path: &'a Path,
    states: &'a StateFeed,
    mic: &'a watch::Sender<f32>,
    taps: &'a mut mpsc::Receiver<Tap>,
}

/// Runs `body` beside a server on a socket in a temporary directory, which is
/// removed (socket and all) when the `TempDir` drops.
async fn serving<T>(body: impl AsyncFnOnce(Served<'_>) -> T) -> T {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("orb.sock");
    let states = StateFeed::new();
    let (mic, mic_level) = watch::channel(0.0);
    let (tap_sender, mut taps) = mpsc::channel(TAP_QUEUE);
    let server = OrbServer::bind(&path).unwrap();
    let feeds = OrbFeeds {
        states: &states,
        mic_level,
        speech_level: None,
        taps: tap_sender,
    };
    let served = Served {
        path: &path,
        states: &states,
        mic: &mic,
        taps: &mut taps,
    };
    tokio::select! {
        never = server.serve(feeds) => match never {},
        result = body(served) => result,
    }
}

/// A stand-in for `jarvis-orb`.
struct Orb {
    lines: Lines<BufReader<OwnedReadHalf>>,
    write: OwnedWriteHalf,
}

/// What the orb's next read gave.
#[derive(Debug, PartialEq)]
enum Read {
    Message(ToOrb),
    /// Nothing arrived within `WAIT`.
    Quiet,
    /// The server closed the connection.
    Closed,
}

impl Orb {
    async fn connect(path: &Path) -> Self {
        let (read, write) = UnixStream::connect(path).await.unwrap().into_split();
        Self {
            lines: BufReader::new(read).lines(),
            write,
        }
    }

    async fn read(&mut self) -> Read {
        match timeout(WAIT, self.lines.next_line()).await {
            Err(_) => Read::Quiet,
            Ok(Ok(Some(line))) => Read::Message(ToOrb::decode(&line).unwrap()),
            Ok(Ok(None)) | Ok(Err(_)) => Read::Closed,
        }
    }

    async fn send(&mut self, bytes: &[u8]) {
        self.write.write_all(bytes).await.unwrap();
    }
}

fn state(state: OrbState) -> Read {
    Read::Message(ToOrb::State { state })
}

/// A valid tap padded with spaces to exactly `len` bytes before its newline.
fn padded_tap(len: usize) -> Vec<u8> {
    let mut line = b"{\"type\":\"tap\"}".to_vec();
    line.resize(len, b' ');
    line.push(b'\n');
    line
}

#[tokio::test(start_paused = true)]
async fn a_new_orb_is_told_the_current_state_and_then_every_change_in_order() {
    serving(async |served| {
        served.states.publish(AssistantState::Thinking);
        let mut orb = Orb::connect(served.path).await;
        assert_eq!(orb.read().await, state(OrbState::Thinking));

        // Published back to back: none may be skipped or reordered.
        served.states.publish(AssistantState::Idle);
        served.states.publish(AssistantState::Listening);
        served.states.publish(AssistantState::Transcribing);
        served.states.publish(AssistantState::Thinking);
        for expected in [
            OrbState::Idle,
            OrbState::Listening,
            OrbState::Transcribing,
            OrbState::Thinking,
        ] {
            assert_eq!(orb.read().await, state(expected));
        }
        assert_eq!(orb.read().await, Read::Quiet);
    })
    .await;
}

#[test]
fn each_assistant_state_has_its_orb_state() {
    let cases = [
        (AssistantState::Idle, OrbState::Idle),
        (AssistantState::Listening, OrbState::Listening),
        (AssistantState::Transcribing, OrbState::Transcribing),
        (AssistantState::Thinking, OrbState::Thinking),
        (AssistantState::Speaking, OrbState::Speaking),
    ];
    for (assistant, orb) in cases {
        assert_eq!(orb_state(assistant), orb);
    }
}

#[tokio::test(start_paused = true)]
async fn the_mic_level_is_sent_thirty_times_a_second_while_listening() {
    serving(async |served| {
        let mut orb = Orb::connect(served.path).await;
        assert_eq!(orb.read().await, state(OrbState::Idle));
        served.mic.send_replace(0.25);
        served.states.publish(AssistantState::Listening);
        assert_eq!(orb.read().await, state(OrbState::Listening));

        let mut arrivals = Vec::new();
        while arrivals.len() < 31 {
            let read = orb.read().await;
            assert_eq!(
                read,
                Read::Message(ToOrb::Level {
                    level: Level::new(0.25).unwrap(),
                    source: LevelSource::Mic,
                })
            );
            arrivals.push(Instant::now());
        }
        // Never faster than the cadence, and 30 intervals take 30 x 33 ms.
        for pair in arrivals.windows(2) {
            assert_eq!(pair[1] - pair[0], LEVEL_INTERVAL);
        }
        assert_eq!(arrivals[30] - arrivals[0], Duration::from_millis(990));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn levels_carry_the_source_of_the_state_and_stop_when_there_is_nothing_to_hear() {
    serving(async |served| {
        let mut orb = Orb::connect(served.path).await;
        assert_eq!(orb.read().await, state(OrbState::Idle));
        served.mic.send_replace(0.5);
        // (state, the source its levels carry)
        let cases = [
            (AssistantState::Listening, Some(LevelSource::Mic)),
            (AssistantState::Transcribing, None),
            (AssistantState::Speaking, Some(LevelSource::Speech)),
            (AssistantState::Thinking, None),
            (AssistantState::Idle, None),
        ];
        for (assistant, source) in cases {
            served.states.publish(assistant);
            // A level for the state before may already be on its way.
            let after_state = loop {
                match orb.read().await {
                    Read::Message(ToOrb::State { state }) => {
                        assert_eq!(state, orb_state(assistant));
                        break orb.read().await;
                    }
                    Read::Message(ToOrb::Level { .. }) => {}
                    other => panic!("{assistant:?}: {other:?}"),
                }
            };
            match source {
                Some(source) => match after_state {
                    Read::Message(ToOrb::Level { source: sent, .. }) => assert_eq!(sent, source),
                    other => panic!("{assistant:?}: {other:?}"),
                },
                None => assert_eq!(after_state, Read::Quiet, "{assistant:?}"),
            }
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_level_outside_the_range_is_clamped_before_it_is_sent() {
    serving(async |served| {
        let mut orb = Orb::connect(served.path).await;
        assert_eq!(orb.read().await, state(OrbState::Idle));
        served.mic.send_replace(7.0);
        served.states.publish(AssistantState::Listening);
        assert_eq!(orb.read().await, state(OrbState::Listening));
        assert_eq!(
            orb.read().await,
            Read::Message(ToOrb::Level {
                level: Level::new(1.0).unwrap(),
                source: LevelSource::Mic,
            })
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_tap_is_passed_on_and_a_second_one_waiting_is_dropped() {
    serving(async |served| {
        let mut orb = Orb::connect(served.path).await;
        assert_eq!(orb.read().await, state(OrbState::Idle));
        // Three taps before anyone handles one: the queue holds `TAP_QUEUE` (1).
        for _ in 0..3 {
            orb.send(TAP_LINE).await;
        }
        assert_eq!(orb.read().await, Read::Quiet);
        assert_eq!(served.taps.try_recv(), Ok(Tap));
        assert!(served.taps.try_recv().is_err());

        orb.send(TAP_LINE).await;
        assert_eq!(timeout(WAIT, served.taps.recv()).await, Ok(Some(Tap)));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_bad_line_is_ignored_and_the_connection_keeps_working() {
    let oversized = padded_tap(MAX_LINE_BYTES + 1);
    let far_oversized = vec![b'x'; 10 * MAX_LINE_BYTES];
    let cases: [(&str, &[u8]); 8] = [
        ("not JSON", b"tap\n"),
        ("an unknown message", b"{\"type\":\"shutdown\"}\n"),
        ("an unknown field", b"{\"type\":\"tap\",\"force\":true}\n"),
        (
            "a message meant for the orb",
            b"{\"type\":\"state\",\"state\":\"idle\"}\n",
        ),
        ("not UTF-8", b"\xff\xfe\n"),
        ("an empty line", b"\n"),
        ("one byte over the line limit", &oversized),
        ("far over the line limit, in one piece", &far_oversized),
    ];
    serving(async |served| {
        let mut orb = Orb::connect(served.path).await;
        assert_eq!(orb.read().await, state(OrbState::Idle));
        for (name, line) in cases {
            orb.send(line).await;
            if !line.ends_with(b"\n") {
                orb.send(b"\n").await;
            }
            assert_eq!(orb.read().await, Read::Quiet, "{name}");
            assert!(served.taps.try_recv().is_err(), "{name}");
            // Still connected, and the next good line is read as itself.
            orb.send(TAP_LINE).await;
            assert_eq!(
                timeout(WAIT, served.taps.recv()).await,
                Ok(Some(Tap)),
                "{name}"
            );
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_longest_line_is_accepted_and_one_byte_more_is_not() {
    serving(async |served| {
        let mut orb = Orb::connect(served.path).await;
        assert_eq!(orb.read().await, state(OrbState::Idle));

        orb.send(&padded_tap(MAX_LINE_BYTES)).await;
        assert_eq!(timeout(WAIT, served.taps.recv()).await, Ok(Some(Tap)));

        orb.send(&padded_tap(MAX_LINE_BYTES + 1)).await;
        assert!(timeout(WAIT, served.taps.recv()).await.is_err());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_line_split_across_writes_is_read_as_one() {
    serving(async |served| {
        let mut orb = Orb::connect(served.path).await;
        assert_eq!(orb.read().await, state(OrbState::Idle));
        let (first, second) = TAP_LINE.split_at(5);
        orb.send(first).await;
        assert!(timeout(WAIT, served.taps.recv()).await.is_err());
        orb.send(second).await;
        assert_eq!(timeout(WAIT, served.taps.recv()).await, Ok(Some(Tap)));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn two_orbs_are_served_at_once_and_one_leaving_does_not_disturb_the_other() {
    serving(async |served| {
        let mut first = Orb::connect(served.path).await;
        let mut second = Orb::connect(served.path).await;
        assert_eq!(first.read().await, state(OrbState::Idle));
        assert_eq!(second.read().await, state(OrbState::Idle));

        served.states.publish(AssistantState::Thinking);
        assert_eq!(first.read().await, state(OrbState::Thinking));
        assert_eq!(second.read().await, state(OrbState::Thinking));

        drop(first);
        served.states.publish(AssistantState::Idle);
        assert_eq!(second.read().await, state(OrbState::Idle));
        second.send(TAP_LINE).await;
        assert_eq!(timeout(WAIT, served.taps.recv()).await, Ok(Some(Tap)));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn an_orb_that_reconnects_is_served_again() {
    serving(async |served| {
        for round in 0..3 {
            let mut orb = Orb::connect(served.path).await;
            assert_eq!(orb.read().await, state(OrbState::Idle), "round {round}");
            served.states.publish(AssistantState::Thinking);
            assert_eq!(orb.read().await, state(OrbState::Thinking), "round {round}");
            drop(orb);
            // With nobody connected, publishing is silent and harmless.
            served.states.publish(AssistantState::Idle);
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_connection_limit_serves_its_last_orb_and_closes_the_next() {
    serving(async |served| {
        let mut orbs = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            let mut orb = Orb::connect(served.path).await;
            assert_eq!(orb.read().await, state(OrbState::Idle));
            orbs.push(orb);
        }
        let mut one_too_many = Orb::connect(served.path).await;
        assert_eq!(one_too_many.read().await, Read::Closed);

        // A slot freed by a leaving orb is given to the next one.
        orbs.pop();
        served.states.publish(AssistantState::Thinking);
        assert_eq!(orbs[0].read().await, state(OrbState::Thinking));
        let mut replacement = Orb::connect(served.path).await;
        assert_eq!(replacement.read().await, state(OrbState::Thinking));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn an_orb_that_stops_reading_is_closed_and_the_others_carry_on() {
    serving(async |served| {
        let mut stuck = Orb::connect(served.path).await;
        let mut healthy = Orb::connect(served.path).await;
        assert_eq!(stuck.read().await, state(OrbState::Idle));
        assert_eq!(healthy.read().await, state(OrbState::Idle));

        // Levels pile up in the stuck orb's socket until a write cannot finish
        // within `WRITE_TIMEOUT`. About 1 MB of levels overfills any socket buffer.
        served.states.publish(AssistantState::Listening);
        for _ in 0..PILED_UP_LEVELS {
            match healthy.read().await {
                Read::Message(_) => {}
                other => panic!("the healthy orb saw {other:?}"),
            }
        }
        // What was buffered for the stuck orb ends in a close, not in more levels.
        let mut drained = 0;
        let end = loop {
            match stuck.read().await {
                Read::Message(_) if drained < PILED_UP_LEVELS => drained += 1,
                other => break other,
            }
        };
        assert_eq!(end, Read::Closed);
        assert!(matches!(healthy.read().await, Read::Message(_)));
    })
    .await;
}

#[tokio::test]
async fn the_socket_is_for_its_owner_only_and_is_removed_with_the_server() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("orb.sock");
    let server = OrbServer::bind(&path).unwrap();
    let mode = fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, SOCKET_MODE);
    drop(server);
    assert!(!path.exists());
}

#[tokio::test]
async fn a_stale_socket_file_is_replaced() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("orb.sock");
    // A listener that is gone leaves its file behind, as a killed Jarvis would.
    drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
    assert!(path.exists());

    let server = OrbServer::bind(&path).unwrap();
    assert!(UnixStream::connect(&path).await.is_ok());
    drop(server);
}

#[tokio::test]
async fn a_socket_someone_is_serving_is_left_alone() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("orb.sock");
    let first = OrbServer::bind(&path).unwrap();
    assert!(matches!(OrbServer::bind(&path), Err(OrbError::InUse(_))));
    // The refused second server must not have removed the first one's file.
    assert!(path.exists());
    drop(first);
}

#[tokio::test]
async fn a_file_that_is_not_a_socket_is_never_deleted() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("notes.txt");
    fs::write(&path, "keep me").unwrap();
    assert!(matches!(
        OrbServer::bind(&path),
        Err(OrbError::NotASocket(_))
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), "keep me");
}

#[tokio::test]
async fn a_socket_in_a_missing_directory_is_reported() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("missing").join("orb.sock");
    assert!(matches!(OrbServer::bind(&path), Err(OrbError::Bind { .. })));
}
