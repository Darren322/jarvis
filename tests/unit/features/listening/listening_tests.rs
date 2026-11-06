use super::*;
use std::{
    fs,
    future::pending,
    path::{Path, PathBuf},
    time::Duration,
};

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
        let result = recorder.record(stop_after_emitted(&fixture)).await;
        match expected {
            Ok(len) => {
                let recording = result.unwrap_or_else(|e| panic!("{emitted}: {e}"));
                assert_eq!(recording.pcm.len(), len, "emitted {emitted}");
                assert_eq!(recording.sample_rate, 16_000);
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
    let recording = recorder.record(stop_after_emitted(&fixture)).await.unwrap();
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
    recorder.record(pending()).await.unwrap();
    assert_eq!(
        fs::read_to_string(args_file).unwrap().trim(),
        "-q -D test-device -f S16_LE -r 16000 -c 1 -t raw"
    );
}

#[tokio::test]
async fn a_missing_program_is_a_spawn_error() {
    let recorder = Recorder::new("dev", "/nonexistent/jarvis-arecord");
    assert!(matches!(
        recorder.record(pending()).await,
        Err(CaptureError::Spawn(_))
    ));
}

#[tokio::test]
async fn a_recorder_failing_on_its_own_reports_its_exit_status() {
    let fixture = Fixture::new("fail");
    let recorder = fixture.recorder(&emit_then(MIN_RECORDING_BYTES, "exit 3"));
    let result = recorder.record(pending()).await;
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
        _ = recorder.record(pending()) => panic!("record should still be waiting"),
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
        let result = tokio::time::timeout(Duration::from_secs(10), recorder.record(pending()))
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
