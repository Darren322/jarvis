use super::*;
use std::{
    fs,
    future::pending,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::oneshot;

/// For tests that do not look at the "recording started" signal.
fn unobserved() -> oneshot::Sender<()> {
    oneshot::channel().0
}

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("jarvis-listening-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// Writes an executable shell script and returns a recorder running it.
    fn recorder(&self, body: &str) -> Recorder {
        use std::os::unix::fs::PermissionsExt;
        let path = self.0.join("fake-arecord");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        Recorder::new("test-device", path.to_str().unwrap())
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Emits `bytes`, then drops a marker file next to the script, then runs `tail`.
fn emit_then(bytes: usize, tail: &str) -> String {
    format!("head -c {bytes} /dev/zero\ntouch \"$0.emitted\"\n{tail}")
}

/// Resolves shortly after the fake recorder has emitted its bytes, so a stop
/// cannot beat a slow script start-up and the reader has time to drain stdout.
async fn stop_after_emitted(fixture: &Fixture) {
    let marker = fixture.path("fake-arecord.emitted");
    for _ in 0..500 {
        if marker.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
}

fn process_is_alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

async fn read_pid(path: &Path) -> String {
    for _ in 0..100 {
        if let Ok(pid) = fs::read_to_string(path)
            && pid.ends_with('\n')
        {
            return pid.trim().to_owned();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("fake recorder never wrote its pid");
}

#[tokio::test]
async fn recording_lengths_are_accepted_or_clamped_at_the_limits() {
    let cases: [(usize, Result<usize, usize>); 6] = [
        (MIN_RECORDING_BYTES - 2, Err(MIN_RECORDING_BYTES - 2)),
        (MIN_RECORDING_BYTES - 1, Err(MIN_RECORDING_BYTES - 2)),
        (MIN_RECORDING_BYTES, Ok(MIN_RECORDING_BYTES)),
        (MAX_RECORDING_BYTES, Ok(MAX_RECORDING_BYTES)),
        (MAX_RECORDING_BYTES + 1, Ok(MAX_RECORDING_BYTES)),
        (MAX_RECORDING_BYTES + 2, Ok(MAX_RECORDING_BYTES)),
    ];
    for (emitted, expected) in cases {
        let fixture = Fixture::new("limits");
        // The script sleeps after emitting, so only the cap can end it.
        let recorder = fixture.recorder(&emit_then(emitted, "exec sleep 30"));
        // The cap cases end before `stop`; under-cap cases need it to finish.
        let result = recorder
            .record(stop_after_emitted(&fixture), unobserved())
            .await;
        match expected {
            Ok(len) => {
                let recording = result.unwrap_or_else(|e| panic!("{emitted}: {e}"));
                assert_eq!(recording.pcm.len(), len, "emitted {emitted}");
            }
            Err(bytes) => assert!(
                matches!(result, Err(CaptureError::TooShort { bytes: b }) if b == bytes),
                "emitted {emitted}: {result:?}"
            ),
        }
    }
}

#[tokio::test]
async fn limits_match_the_spec_in_bytes() {
    assert_eq!(MAX_RECORDING_BYTES, 480_000);
    assert_eq!(MIN_RECORDING_BYTES, 9_600);
}

#[tokio::test]
async fn stopping_early_returns_what_was_read_and_ends_the_recorder() {
    let fixture = Fixture::new("stop");
    let pid_file = fixture.path("pid");
    let recorder = fixture.recorder(&format!(
        "echo $$ > {}\n{}",
        pid_file.display(),
        emit_then(20_000, "exec sleep 30")
    ));
    let recording = recorder
        .record(stop_after_emitted(&fixture), unobserved())
        .await
        .unwrap();
    assert_eq!(recording.pcm.len(), 20_000);
    assert!(!process_is_alive(&read_pid(&pid_file).await));
}

#[tokio::test]
async fn the_recorder_receives_the_device_and_raw_s16_16khz_mono_arguments() {
    let fixture = Fixture::new("args");
    let args_file = fixture.path("args");
    let recorder = fixture.recorder(&format!(
        "echo \"$@\" > {}\n{}",
        args_file.display(),
        emit_then(MIN_RECORDING_BYTES, "")
    ));
    recorder.record(pending(), unobserved()).await.unwrap();
    assert_eq!(
        fs::read_to_string(args_file).unwrap().trim(),
        "-q -D test-device -f S16_LE -r 16000 -c 1 -t raw"
    );
}

#[tokio::test]
async fn a_missing_program_is_a_spawn_error() {
    let recorder = Recorder::new("dev", "/nonexistent/jarvis-arecord");
    assert!(matches!(
        recorder.record(pending(), unobserved()).await,
        Err(CaptureError::Spawn(_))
    ));
}

#[tokio::test]
async fn a_recorder_failing_on_its_own_reports_its_exit_status() {
    let fixture = Fixture::new("fail");
    let recorder = fixture.recorder(&emit_then(MIN_RECORDING_BYTES, "exit 3"));
    let result = recorder.record(pending(), unobserved()).await;
    assert!(
        matches!(&result, Err(CaptureError::Failed(status)) if status.code() == Some(3)),
        "{result:?}"
    );
}

#[tokio::test]
async fn dropping_a_recording_in_progress_kills_the_recorder() {
    let fixture = Fixture::new("drop");
    let pid_file = fixture.path("pid");
    let recorder = fixture.recorder(&format!("echo $$ > {}\nexec sleep 30", pid_file.display()));
    // Drop the recording future only once the child is known to be running.
    let pid = tokio::select! {
        _ = recorder.record(pending(), unobserved()) => panic!("record should still be waiting"),
        pid = read_pid(&pid_file) => pid,
    };
    for _ in 0..100 {
        if !process_is_alive(&pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("recorder {pid} survived the dropped future");
}

#[tokio::test]
async fn the_wall_clock_deadline_ends_a_stalled_recording_with_what_was_read() {
    // `max_wait` is private to `listening`; as a child module the tests may shorten it.
    // A paused clock cannot be used: it auto-advances while the killed child is reaped.
    let cases: [(usize, Result<usize, usize>); 2] =
        [(0, Err(0)), (MIN_RECORDING_BYTES, Ok(MIN_RECORDING_BYTES))];
    for (emitted, expected) in cases {
        let fixture = Fixture::new("stall");
        let recorder = Recorder {
            max_wait: Duration::from_secs(1),
            ..fixture.recorder(&emit_then(emitted, "exec sleep 30"))
        };
        // The outer timeout fails the test, instead of hanging, if the deadline is missing.
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            recorder.record(pending(), unobserved()),
        )
        .await
        .expect("record must give up at its deadline");
        match expected {
            Ok(len) => assert_eq!(result.unwrap().pcm.len(), len),
            Err(bytes) => assert!(
                matches!(result, Err(CaptureError::TooShort { bytes: b }) if b == bytes),
                "{result:?}"
            ),
        }
    }
}

#[test]
fn the_wall_clock_deadline_is_the_cap_plus_one_second_of_grace() {
    assert_eq!(MAX_RECORDING_WAIT, Duration::from_secs(16));
}

#[tokio::test]
async fn the_first_audio_announces_that_recording_started() {
    let fixture = Fixture::new("started");
    let recorder = fixture.recorder(&emit_then(MIN_RECORDING_BYTES, "exec sleep 30"));
    let (sender, started) = oneshot::channel();

    // Stopping only once the announcement arrived proves it is sent while still recording.
    let stop = async {
        started.await.expect("the first audio should be announced");
    };
    let result = tokio::time::timeout(Duration::from_secs(10), recorder.record(stop, sender))
        .await
        .expect("record must stop once the first audio was announced");

    // Stopped after one read chunk, so too short, but not empty.
    assert!(
        matches!(result, Err(CaptureError::TooShort { bytes }) if bytes > 0),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_recorder_that_sends_no_audio_never_announces_recording_started() {
    let fixture = Fixture::new("silent");
    let recorder = fixture.recorder("exec sleep 30");
    let (sender, mut started) = oneshot::channel();

    let result = recorder.record(std::future::ready(()), sender).await;

    assert!(matches!(result, Err(CaptureError::TooShort { bytes: 0 })));
    assert_eq!(
        started.try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    );
}

use tokio::sync::{broadcast, watch};

/// Everything an observer has received so far, in order, and whether it lagged.
fn drain(tap: &mut broadcast::Receiver<AudioChunk>) -> (Vec<u8>, bool) {
    use broadcast::error::TryRecvError;
    let (mut bytes, mut lagged) = (Vec::new(), false);
    loop {
        match tap.try_recv() {
            Ok(chunk) => bytes.extend_from_slice(&chunk),
            Err(TryRecvError::Lagged(_)) => lagged = true,
            Err(TryRecvError::Empty | TryRecvError::Closed) => return (bytes, lagged),
        }
    }
}

/// A recorder that emits `bytes` of non-repeating data, then exits.
fn emitting_random(fixture: &Fixture, bytes: usize) -> Recorder {
    fixture.recorder(&format!("head -c {bytes} /dev/urandom"))
}

#[tokio::test]
async fn every_observer_receives_every_chunk_in_order() {
    let fixture = Fixture::new("tap-order");
    let recorder = emitting_random(&fixture, 40_000);
    let (mut first, mut second) = (recorder.tap(), recorder.tap());

    let recording = recorder.record(pending(), unobserved()).await.unwrap();

    for tap in [&mut first, &mut second] {
        let (heard, lagged) = drain(tap);
        assert!(!lagged);
        assert_eq!(heard, recording.pcm);
    }
}

#[tokio::test]
async fn a_tap_subscribed_between_recordings_sees_the_next_recording() {
    let fixture = Fixture::new("tap-later");
    let recorder = emitting_random(&fixture, 20_000);
    recorder.record(pending(), unobserved()).await.unwrap();
    let mut late = recorder.tap();
    assert!(
        drain(&mut late).0.is_empty(),
        "earlier audio is not replayed"
    );

    let recording = recorder.record(pending(), unobserved()).await.unwrap();

    assert_eq!(drain(&mut late).0, recording.pcm);
}

#[tokio::test]
async fn an_observer_that_never_reads_neither_blocks_nor_changes_the_recording() {
    // More chunks than the tap holds: every read is at most READ_CHUNK_BYTES.
    let bytes = (TAP_CAPACITY_CHUNKS + 1) * READ_CHUNK_BYTES;
    assert!(bytes <= MAX_RECORDING_BYTES);
    let script = format!("yes abcdefg | head -c {bytes}");

    let baseline = Fixture::new("tap-baseline");
    let expected = baseline
        .recorder(&script)
        .record(pending(), unobserved())
        .await
        .unwrap();

    let fixture = Fixture::new("tap-full");
    let recorder = fixture.recorder(&script);
    let mut slow = recorder.tap();
    let recording = tokio::time::timeout(
        Duration::from_secs(10),
        recorder.record(pending(), unobserved()),
    )
    .await
    .expect("a full observer must not block the recording")
    .unwrap();

    assert_eq!(recording.pcm, expected.pcm);
    // The slow observer lost the oldest chunks, then still gets the newest ones.
    let (heard, lagged) = drain(&mut slow);
    assert!(lagged);
    assert!(!heard.is_empty() && heard.len() < bytes);
    assert!(expected.pcm.ends_with(&heard));
}

#[tokio::test]
async fn tapped_chunks_are_whole_samples_however_the_reads_split() {
    // Odd-sized writes with a pause after each, so the reader usually sees each one
    // alone: a 4097-byte write is read as 4096 + 1 bytes, a 3-byte write as 3. The
    // pauses make an odd split very likely but cannot force one; the assertions
    // hold however the reads split. 12 297 bytes is odd, so a lone byte ends the stream.
    let fixture = Fixture::new("tap-odd");
    let recorder = fixture.recorder(
        "w() { head -c \"$1\" /dev/urandom; sleep 0.05; }\n\
         w 4097; w 3; w 4097; w 3; w 4097",
    );
    let mut tap = recorder.tap();

    let recording = recorder.record(pending(), unobserved()).await.unwrap();

    let mut chunks = Vec::new();
    while let Ok(chunk) = tap.try_recv() {
        chunks.push(chunk);
    }
    assert!(!chunks.is_empty());
    assert!(
        chunks.iter().all(|chunk| chunk.len() % 2 == 0),
        "odd chunk lengths: {:?}",
        chunks.iter().map(|chunk| chunk.len()).collect::<Vec<_>>()
    );
    // Only the final lone byte is missing, and `Recording` drops it too.
    assert_eq!(recording.pcm.len(), 12_296);
    assert_eq!(chunks.concat(), recording.pcm);
}

#[tokio::test]
async fn a_dropped_observer_does_not_end_or_change_the_recording() {
    let fixture = Fixture::new("tap-closed");
    let recorder = emitting_random(&fixture, 20_000);
    let kept = recorder.tap();
    drop(recorder.tap());
    drop(kept);

    let recording = recorder.record(pending(), unobserved()).await.unwrap();

    assert_eq!(recording.pcm.len(), 20_000);
}

#[tokio::test]
async fn the_mic_level_follows_the_audio_a_recording_taps() {
    // 0x7F bytes make samples of 0x7F7F, which is just under full scale.
    let fixture = Fixture::new("tap-level");
    let recorder = fixture.recorder("head -c 20000 /dev/zero | tr '\\000' '\\177'");
    let (levels, mut reader) = watch::channel(0.0);
    let meter = tokio::spawn(run_level_meter(recorder.tap(), levels));

    recorder.record(pending(), unobserved()).await.unwrap();

    let loudest = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            reader.changed().await.unwrap();
            let level = *reader.borrow();
            if level > 0.0 {
                return level;
            }
        }
    })
    .await
    .expect("the meter should report the tapped audio");
    assert!((0.99..=1.0).contains(&loudest), "{loudest}");
    meter.abort();
}
