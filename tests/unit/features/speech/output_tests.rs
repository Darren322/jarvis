use super::*;
use crate::config::TtsEngine;
use rodio::mixer::{self, MixerSource};
use std::{
    future::Future,
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};

fn write_test_wav(path: &Path, sample_rate: u32) {
    let spec = hound::WavSpec {
        channels: EXPECTED_CHANNELS,
        sample_rate,
        bits_per_sample: EXPECTED_BITS_PER_SAMPLE,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).expect("create WAV fixture");
    for sample in [-32_768_i16, 0, 32_767] {
        writer.write_sample(sample).expect("write WAV fixture");
    }
    writer.finalize().expect("finish WAV fixture");
}

/// Writes a valid mono PCM16 WAV of `samples` silent samples without writing the audio:
/// the data chunk is a sparse, zero-filled extension of the file.
fn write_sparse_wav(path: &Path, sample_rate: u32, samples: usize) {
    let data_bytes = u32::try_from(samples * 2).expect("bounded WAV size");
    let mut wav = std::fs::File::create(path).expect("create sparse WAV fixture");
    wav.write_all(b"RIFF").expect("write RIFF tag");
    wav.write_all(&(36_u32 + data_bytes).to_le_bytes())
        .expect("write RIFF size");
    wav.write_all(b"WAVEfmt ").expect("write format tags");
    wav.write_all(&16_u32.to_le_bytes())
        .expect("write format chunk size");
    wav.write_all(&1_u16.to_le_bytes())
        .expect("write PCM format");
    wav.write_all(&EXPECTED_CHANNELS.to_le_bytes())
        .expect("write channel count");
    wav.write_all(&sample_rate.to_le_bytes())
        .expect("write sample rate");
    wav.write_all(&(sample_rate * 2).to_le_bytes())
        .expect("write byte rate");
    wav.write_all(&2_u16.to_le_bytes())
        .expect("write block alignment");
    wav.write_all(&EXPECTED_BITS_PER_SAMPLE.to_le_bytes())
        .expect("write bit depth");
    wav.write_all(b"data").expect("write data tag");
    wav.write_all(&data_bytes.to_le_bytes())
        .expect("write data size");
    wav.set_len(44 + u64::from(data_bytes))
        .expect("extend sparse sample fixture");
}

fn rate(hz: u32) -> SampleRate {
    SampleRate::new(hz).expect("test rate is in range")
}

fn supertonic_config(python: &str, worker_script: PathBuf) -> TtsConfig {
    TtsConfig {
        python: PathBuf::from(python),
        worker_script,
        engine: TtsEngine::Supertonic {
            model_dir: PathBuf::from("/unused/model"),
        },
        threads: 2,
        audio_device: None,
    }
}

const PIPER_VOICE: &str = "en_GB-northern_english_male-medium";

fn piper_config(python: &str, worker_script: PathBuf) -> TtsConfig {
    TtsConfig {
        python: PathBuf::from(python),
        worker_script,
        engine: TtsEngine::Piper {
            model: PathBuf::from(format!("/unused/{PIPER_VOICE}.onnx")),
            voice: PIPER_VOICE.to_owned(),
        },
        threads: 2,
        audio_device: None,
    }
}

#[test]
fn decodes_bounded_mono_pcm16_wav() {
    let directory = tempfile::tempdir().expect("create fixture directory");
    let path = directory.path().join("answer.wav");
    write_test_wav(&path, 44_100);

    let decoded = SpeechOutput::decode_wav(&path, rate(44_100)).expect("decode valid WAV");

    assert_eq!(decoded.samples.len(), 3);
    assert_eq!(decoded.samples[0], -1.0);
    assert_eq!(decoded.samples[1], 0.0);
    assert!(decoded.samples[2] > 0.999 && decoded.samples[2] < 1.0);
    assert!(decoded.duration < Duration::from_millis(1));
}

#[tokio::test(start_paused = true)]
async fn playback_is_overdue_two_seconds_after_the_queued_audio_should_have_ended() {
    let mut clock = PlaybackClock::new();
    clock.queue(WAKE_NOISE_DURATION);
    clock.queue(Duration::from_secs(10));
    let queued = WAKE_NOISE_DURATION + Duration::from_secs(10);

    tokio::time::advance(queued + PLAYBACK_GRACE - Duration::from_millis(1)).await;
    assert!(!clock.is_overdue(), "last moment still within the grace");
    tokio::time::advance(Duration::from_millis(1)).await;
    assert!(clock.is_overdue(), "first moment past the grace");
}

#[tokio::test(start_paused = true)]
async fn each_queued_chunk_extends_the_playback_deadline_so_long_answers_are_not_cut_off() {
    let mut clock = PlaybackClock::new();
    let chunk = Duration::from_secs(20);
    let chunks = 10;

    for _ in 0..chunks {
        clock.queue(chunk);
    }

    // Far beyond the old 122.7 s cap on a single utterance.
    tokio::time::advance(chunk * chunks + PLAYBACK_GRACE - Duration::from_millis(1)).await;
    assert!(!clock.is_overdue());
    tokio::time::advance(Duration::from_millis(1)).await;
    assert!(clock.is_overdue());
}

#[tokio::test(start_paused = true)]
async fn a_chunk_queued_after_a_pause_counts_from_when_it_was_queued() {
    let mut clock = PlaybackClock::new();
    clock.queue(Duration::from_secs(1));
    tokio::time::advance(Duration::from_secs(30)).await;

    clock.queue(Duration::from_secs(5));

    tokio::time::advance(Duration::from_secs(5) + PLAYBACK_GRACE - Duration::from_millis(1)).await;
    assert!(!clock.is_overdue());
    tokio::time::advance(Duration::from_millis(1)).await;
    assert!(clock.is_overdue());
}

#[test]
fn rejects_wrong_format_and_wav_size_bounds() {
    let directory = tempfile::tempdir().expect("create fixture directory");
    let path = directory.path().join("answer.wav");
    write_test_wav(&path, 48_000);

    assert!(matches!(
        SpeechOutput::decode_wav(&path, rate(44_100)),
        Err(SpeechError::InvalidWavFormat)
    ));

    let oversized_path = directory.path().join("oversized.wav");
    std::fs::File::create(&oversized_path)
        .expect("create oversized file")
        .set_len(MAX_WAV_BYTES + 1)
        .expect("extend sparse oversized file");
    assert!(matches!(
        SpeechOutput::decode_wav(&oversized_path, rate(44_100)),
        Err(SpeechError::WavTooLarge)
    ));
}

#[test]
fn a_wav_must_match_the_rate_the_worker_declared() {
    let directory = tempfile::tempdir().expect("create fixture directory");
    let path = directory.path().join("answer.wav");
    // (rate in the WAV header, rate the worker declared, accepted)
    let cases = [
        (22_050, 22_050, true),
        (44_100, 44_100, true),
        (44_100, 22_050, false),
        (22_050, 44_100, false),
        (22_051, 22_050, false),
        (22_049, 22_050, false),
    ];

    for (wav_hz, declared_hz, accepted) in cases {
        write_test_wav(&path, wav_hz);
        let result = SpeechOutput::decode_wav(&path, rate(declared_hz));
        if accepted {
            let decoded = result.expect("matching rates decode");
            assert_eq!(decoded.sample_rate, rate(declared_hz));
            assert_eq!(decoded.samples.len(), 3);
        } else {
            assert!(
                matches!(result, Err(SpeechError::InvalidWavFormat)),
                "a {wav_hz} Hz WAV declared as {declared_hz} Hz must be rejected"
            );
        }
    }
}

#[test]
fn a_22_khz_wav_lasts_as_long_as_its_own_rate_says() {
    let directory = tempfile::tempdir().expect("create fixture directory");
    let path = directory.path().join("answer.wav");
    write_sparse_wav(&path, 22_050, 22_050);

    let decoded = SpeechOutput::decode_wav(&path, rate(22_050)).expect("decode 22.05 kHz WAV");

    // One second of samples at 22.05 kHz, not half a second as at 44.1 kHz.
    assert_eq!(decoded.duration, Duration::from_secs(1));
}

#[test]
fn a_wav_may_hold_120_seconds_at_its_rate_and_not_one_sample_more() {
    let directory = tempfile::tempdir().expect("create fixture directory");
    let path = directory.path().join("answer.wav");

    for hz in [22_050_u32, 44_100] {
        let limit = rate(hz).max_samples();
        assert_eq!(limit, usize::try_from(hz * 120).expect("small limit"));

        write_sparse_wav(&path, hz, limit);
        let decoded = SpeechOutput::decode_wav(&path, rate(hz))
            .unwrap_or_else(|error| panic!("{limit} samples at {hz} Hz should decode: {error}"));
        assert_eq!(decoded.samples.len(), limit);

        write_sparse_wav(&path, hz, limit + 1);
        assert!(
            matches!(
                SpeechOutput::decode_wav(&path, rate(hz)),
                Err(SpeechError::WavTooLong)
            ),
            "{} samples at {hz} Hz are too long",
            limit + 1
        );
    }
}

#[tokio::test(start_paused = true)]
async fn cancel_decode_retains_directory_until_helper_finishes() {
    let operation_dir = Builder::new()
        .prefix("jarvis-speech-test-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .expect("create operation directory");
    let operation_path = operation_dir.path().to_path_buf();
    let (release_helper, wait_for_release) = tokio::sync::oneshot::channel();
    let decode_task = tokio::spawn(async move {
        let _ = wait_for_release.await;
        Ok(ValidatedWav {
            samples: vec![0.0],
            sample_rate: rate(44_100),
            duration: Duration::from_secs(1),
        })
    });
    let mut speech = idle_speech();
    speech.active = Some(ActiveSpeech {
        stage: SpeechStage::Decoding,
        operation_dir: Some(operation_dir),
        decode_task: Some(decode_task),
    });

    let mut cancellation = Box::pin(speech.cancel());
    tokio::select! {
        biased;
        result = &mut cancellation => panic!("cancel unexpectedly completed: {result:?}"),
        _ = tokio::task::yield_now() => {}
    }
    tokio::time::advance(CLEANUP_TIMEOUT).await;
    let timeout_error = cancellation
        .await
        .expect_err("helper should remain pending");

    assert!(matches!(
        timeout_error,
        SpeechError::CleanupFailed {
            resource: "WAV decoder",
            cause: CleanupFailure::BlockingTaskTimeout,
            ..
        }
    ));
    assert!(speech.is_disabled());
    assert!(operation_path.exists());
    assert!(speech.active.is_some());
    assert!(
        speech
            .active
            .as_ref()
            .and_then(|active| active.decode_task.as_ref())
            .is_some()
    );

    let _ = release_helper.send(());
    speech
        .cancel()
        .await
        .expect("settled decoder should clean up");

    assert!(speech.active.is_none());
    assert!(!operation_path.exists());
}

fn idle_speech() -> SpeechOutput {
    SpeechOutput::new(supertonic_config(
        "/unused/python",
        PathBuf::from("/unused/worker.py"),
    ))
}

/// Starts a real child process in place of the Python worker. The owner is live and
/// never becomes ready, so `fail_operation` has to terminate it.
fn launch_stand_in_worker() -> TtsWorker {
    TtsWorker::launch(&supertonic_config("/bin/cat", PathBuf::from("/dev/null")))
        .expect("launch stand-in worker")
}

async fn fail_one_answer(speech: &mut SpeechOutput) -> SpeechError {
    speech.worker = Some(launch_stand_in_worker());
    speech.active = Some(ActiveSpeech {
        stage: SpeechStage::Synthesizing,
        operation_dir: Some(tempfile::tempdir().expect("create operation directory")),
        decode_task: None,
    });
    speech
        .fail_operation(SpeechError::Synthesis(WorkerError::SynthesisTimeout))
        .await
}

#[tokio::test]
async fn the_next_answer_is_accepted_after_one_failed_answer() {
    let mut speech = idle_speech();

    let error = fail_one_answer(&mut speech).await;

    assert!(matches!(
        error,
        SpeechError::Synthesis(WorkerError::SynthesisTimeout)
    ));
    assert!(speech.worker.is_none(), "failed worker must be discarded");
    assert!(speech.active.is_none());
    assert_eq!(
        speech.health,
        SpeechHealth::Recovering {
            consecutive_failures: 1
        }
    );
    assert!(!speech.is_disabled());
    assert!(speech.ensure_accepting().is_ok());
}

#[tokio::test]
async fn speech_is_disabled_on_the_third_consecutive_failure() {
    let cases = [
        (
            2,
            SpeechHealth::Recovering {
                consecutive_failures: 2,
            },
        ),
        (
            3,
            SpeechHealth::Disabled(DisabledReason::ConsecutiveFailures),
        ),
    ];
    assert_eq!(MAX_CONSECUTIVE_SPEECH_FAILURES, 3);

    for (failures, expected) in cases {
        let mut speech = idle_speech();
        for _ in 0..failures {
            fail_one_answer(&mut speech).await;
        }

        assert_eq!(speech.health, expected, "after {failures} failures");
        assert_eq!(speech.disabled_reason().is_some(), failures == 3);
    }
}

#[tokio::test]
async fn a_disabled_session_refuses_the_fourth_answer_without_touching_audio() {
    let mut speech = idle_speech();
    for _ in 0..=MAX_CONSECUTIVE_SPEECH_FAILURES {
        if speech.is_disabled() {
            break;
        }
        fail_one_answer(&mut speech).await;
    }
    let (_chunks_sender, chunks) = mpsc::channel(1);
    let (playback_started, _playback_started_rx) = oneshot::channel();

    assert!(matches!(
        speech.speak(chunks, playback_started).await,
        Err(SpeechError::Disabled(DisabledReason::ConsecutiveFailures))
    ));
    speech.health.record_failure();
    assert_eq!(
        speech.disabled_reason(),
        Some(DisabledReason::ConsecutiveFailures),
        "a failure after disabling keeps the original reason"
    );
}

#[tokio::test]
async fn a_success_between_failures_resets_the_failure_count() {
    let mut speech = idle_speech();
    for _ in 0..MAX_CONSECUTIVE_SPEECH_FAILURES - 1 {
        fail_one_answer(&mut speech).await;
    }

    speech.health.record_success();
    assert_eq!(speech.health, SpeechHealth::Ready);

    for _ in 0..MAX_CONSECUTIVE_SPEECH_FAILURES - 1 {
        fail_one_answer(&mut speech).await;
    }
    assert!(!speech.is_disabled());
    assert_eq!(
        speech.health,
        SpeechHealth::Recovering {
            consecutive_failures: MAX_CONSECUTIVE_SPEECH_FAILURES - 1
        }
    );
}

#[test]
fn a_device_or_playback_failure_is_recoverable_and_the_next_answer_is_accepted() {
    let mut speech = idle_speech();

    speech.count_failure_and_reset_player();

    assert!(speech.player.is_none(), "the device is re-opened next time");
    assert_eq!(
        speech.health,
        SpeechHealth::Recovering {
            consecutive_failures: 1
        }
    );
    assert!(speech.ensure_accepting().is_ok());
}

#[tokio::test]
async fn cancelling_synthesis_is_not_a_failure_and_has_no_limit() {
    let mut speech = idle_speech();

    for _ in 0..5 {
        speech.worker = Some(launch_stand_in_worker());
        speech.active = Some(ActiveSpeech {
            stage: SpeechStage::Synthesizing,
            operation_dir: Some(tempfile::tempdir().expect("create operation directory")),
            decode_task: None,
        });

        speech.cancel().await.expect("cancel settles cleanly");

        assert!(speech.worker.is_none(), "cancelled worker is discarded");
        assert!(speech.active.is_none());
    }

    assert_eq!(speech.health, SpeechHealth::Ready);
    assert!(speech.ensure_accepting().is_ok());
}

#[tokio::test]
async fn a_failed_directory_cleanup_disables_speech_as_unrecoverable() {
    let mut speech = idle_speech();
    let operation_dir = tempfile::tempdir().expect("create operation directory");
    let locked = operation_dir.path().join("locked");
    std::fs::create_dir(&locked).expect("create locked directory");
    std::fs::write(locked.join("file"), b"x").expect("write file");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000))
        .expect("lock directory");
    speech.active = Some(ActiveSpeech {
        stage: SpeechStage::Synthesizing,
        operation_dir: Some(operation_dir),
        decode_task: None,
    });

    let error = speech
        .fail_operation(SpeechError::Synthesis(WorkerError::SynthesisTimeout))
        .await;

    assert!(matches!(error, SpeechError::OperationAndCleanup { .. }));
    assert_eq!(
        speech.health,
        SpeechHealth::Disabled(DisabledReason::Unrecoverable)
    );
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700))
        .expect("unlock directory");
    std::fs::remove_dir_all(locked.parent().expect("locked has a parent"))
        .expect("remove fixture directory");
}

#[test]
fn a_disabled_session_ignores_later_successes() {
    let mut health = SpeechHealth::Disabled(DisabledReason::Unrecoverable);

    health.record_success();

    assert_eq!(
        health,
        SpeechHealth::Disabled(DisabledReason::Unrecoverable)
    );
}

#[test]
fn a_synthesis_timeout_is_reachable_through_the_error_source_chain() {
    let error = SpeechError::Synthesis(WorkerError::SynthesisTimeout);

    assert_eq!(error.to_string(), "speech synthesis failed");
    let cause = std::error::Error::source(&error).expect("synthesis error keeps its cause");
    assert_eq!(cause.to_string(), "tts synthesis timed out");
}

// ---- Streaming answers, through a real child process standing in for the worker ----

/// Samples in the fixture WAV every fake synthesis returns (0.1 s).
const FIXTURE_SAMPLES: usize = 4_410;
/// Level of the fixture samples; the wake noise never exceeds 75 / 32,768.
const FIXTURE_LEVEL: f32 = 0.5;
const TEST_DEADLINE: Duration = Duration::from_secs(20);

/// Counts what the output device would have played: lead-in noise and speech samples.
struct Pump {
    stop: Arc<AtomicBool>,
    noise: Arc<AtomicUsize>,
    speech: Arc<AtomicUsize>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Pump {
    /// Drains the mixer about 100 times faster than real time, so a played-out answer
    /// finishes in milliseconds.
    fn start(mut source: MixerSource) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let noise = Arc::new(AtomicUsize::new(0));
        let speech = Arc::new(AtomicUsize::new(0));
        let (thread_stop, thread_noise, thread_speech) =
            (Arc::clone(&stop), Arc::clone(&noise), Arc::clone(&speech));
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                for _ in 0..4_410 {
                    match source.next() {
                        Some(sample) if sample.abs() >= FIXTURE_LEVEL / 2.0 => {
                            thread_speech.fetch_add(1, Ordering::AcqRel);
                        }
                        Some(sample) if sample != 0.0 => {
                            thread_noise.fetch_add(1, Ordering::AcqRel);
                        }
                        _ => {}
                    }
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
        Self {
            stop,
            noise,
            speech,
            thread: Some(thread),
        }
    }

    fn noise_samples(&self) -> usize {
        self.noise.load(Ordering::Acquire)
    }

    fn speech_samples(&self) -> usize {
        self.speech.load(Ordering::Acquire)
    }
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Harness {
    speech: SpeechOutput,
    directory: tempfile::TempDir,
    source: Option<MixerSource>,
}

/// A speech output whose worker is a shell script and whose audio device is a mixer
/// nobody drains until a test starts a [`Pump`].
///
/// `fail_second_request_once` makes the first worker answer its second request with an
/// error; a replacement worker behaves.
fn harness(fail_second_request_once: bool) -> Harness {
    harness_for(FixtureVoice::Supertonic, fail_second_request_once)
}

/// Which engine the fake worker claims to be, and the rate it synthesizes at.
#[derive(Clone, Copy)]
enum FixtureVoice {
    Supertonic,
    Piper,
}

impl FixtureVoice {
    fn sample_rate(self) -> u32 {
        match self {
            Self::Supertonic => 44_100,
            Self::Piper => 22_050,
        }
    }

    /// The ready frame, with `%s` where the shell fills in its own pid.
    fn ready_frame(self) -> String {
        let (engine, precision, voice) = match self {
            Self::Supertonic => ("supertonic-3", "int8", "M5"),
            Self::Piper => ("piper", "fp32", PIPER_VOICE),
        };
        format!(
            r#"{{"type":"ready","protocol":1,"pid":%s,"engine":"{engine}","precision":"{precision}","voice":"{voice}","language":"en","provider":"cpu","threads":2,"sample_rate":{},"num_speakers":1}}"#,
            self.sample_rate()
        )
    }

    fn config(self, worker_script: PathBuf) -> TtsConfig {
        match self {
            Self::Supertonic => supertonic_config("/bin/sh", worker_script),
            Self::Piper => piper_config("/bin/sh", worker_script),
        }
    }
}

fn harness_for(voice: FixtureVoice, fail_second_request_once: bool) -> Harness {
    let directory = tempfile::tempdir().expect("create fixture directory");
    let wav = directory.path().join("fixture.wav");
    let spec = hound::WavSpec {
        channels: EXPECTED_CHANNELS,
        sample_rate: voice.sample_rate(),
        bits_per_sample: EXPECTED_BITS_PER_SAMPLE,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&wav, spec).expect("create WAV fixture");
    for _ in 0..FIXTURE_SAMPLES {
        writer
            .write_sample((FIXTURE_LEVEL * 32_768.0) as i16)
            .expect("write WAV fixture");
    }
    writer.finalize().expect("finish WAV fixture");

    let log = directory.path().join("requests.log");
    let failed_marker = directory.path().join("failed-once");
    let failure_rule = if fail_second_request_once {
        format!(
            r#"if [ "$n" -eq 2 ] && [ ! -e '{marker}' ]; then : > '{marker}'; printf '{{"type":"error","protocol":1,"id":"%s","error":"injected"}}\n' "$id"; continue; fi"#,
            marker = failed_marker.display()
        )
    } else {
        String::new()
    };
    let script = format!(
        r#"printf '{ready_frame}\n' "$$"
n=0
while IFS= read -r line; do
  n=$((n+1))
  printf '%s\n' "$line" >> '{log}'
  id=$(printf '%s' "$line" | sed 's/.*"id":"\([^"]*\)".*/\1/')
  out=$(printf '%s' "$line" | sed 's/.*"output_path":"\([^"]*\)".*/\1/')
  {failure_rule}
  cp '{wav}' "$out"
  printf '{{"type":"completed","protocol":1,"id":"%s","sample_rate":{rate},"num_samples":{FIXTURE_SAMPLES},"wav_bytes":{wav_bytes}}}\n' "$id"
done
"#,
        ready_frame = voice.ready_frame(),
        rate = voice.sample_rate(),
        log = log.display(),
        wav = wav.display(),
        wav_bytes = 44 + FIXTURE_SAMPLES * 2,
    );
    let worker_script = directory.path().join("worker.sh");
    std::fs::write(&worker_script, script).expect("write fake worker");

    let mut speech = SpeechOutput::new(voice.config(worker_script));
    // The output device runs at 44.1 kHz whatever the voice's rate.
    let (mixer, source) = mixer::mixer(
        EXPECTED_CHANNELS.try_into().expect("nonzero channel count"),
        44_100_u32.try_into().expect("nonzero sample rate"),
    );
    speech.player = Some(AudioPlayer::from_mixer(
        mixer,
        (),
        Arc::new(AtomicBool::new(false)),
    ));
    Harness {
        speech,
        directory,
        source: Some(source),
    }
}

impl Harness {
    fn start_pump(&mut self) -> Pump {
        Pump::start(self.source.take().expect("pump started once"))
    }

    /// The text of every request the fake worker has received, in order.
    fn requests(&self) -> Vec<String> {
        let log = self.directory.path().join("requests.log");
        let Ok(log) = std::fs::read_to_string(log) else {
            return Vec::new();
        };
        log.lines()
            .map(|line| {
                let request: serde_json::Value =
                    serde_json::from_str(line).expect("worker log line is JSON");
                request["text"]
                    .as_str()
                    .expect("request has text")
                    .to_owned()
            })
            .collect()
    }
}

async fn within_deadline<T>(future: impl Future<Output = T>) -> T {
    timeout(TEST_DEADLINE, future)
        .await
        .expect("the speech pipeline stalled")
}

#[tokio::test]
async fn a_chunk_is_synthesized_and_queued_while_the_text_stream_is_still_open() {
    let mut harness = harness(false);
    let pump = harness.start_pump();
    let (chunks_sender, chunks) = mpsc::channel(8);
    let (playback_started, started) = oneshot::channel();
    chunks_sender
        .send("The first chunk.".to_owned())
        .await
        .expect("queue first chunk");
    let mut speaking = Box::pin(harness.speech.speak(chunks, playback_started));

    // The sender stays open, so the answer cannot finish: the only way playback can
    // start is by synthesizing the first chunk before the text is complete.
    tokio::select! {
        result = &mut speaking => panic!("answer ended with the text still open: {result:?}"),
        signal = within_deadline(started) => signal.expect("playback should start"),
    }

    chunks_sender
        .send("The second chunk.".to_owned())
        .await
        .expect("queue second chunk");
    drop(chunks_sender);
    within_deadline(speaking)
        .await
        .expect("the answer should play out");
    assert_eq!(
        harness.requests(),
        ["The first chunk.", "The second chunk."]
    );
    assert_eq!(pump.speech_samples(), 2 * FIXTURE_SAMPLES);
    assert_eq!(harness.speech.health, SpeechHealth::Ready);
    assert!(harness.speech.active.is_none());
}

#[tokio::test]
async fn the_lead_in_plays_once_per_answer_not_once_per_chunk() {
    let mut harness = harness(false);
    let pump = harness.start_pump();
    let (chunks_sender, chunks) = mpsc::channel(8);
    let (playback_started, _started) = oneshot::channel();
    for chunk in ["One chunk.", "Two chunks.", "Three chunks."] {
        chunks_sender
            .send(chunk.to_owned())
            .await
            .expect("queue chunk");
    }
    drop(chunks_sender);

    within_deadline(harness.speech.speak(chunks, playback_started))
        .await
        .expect("the answer should play out");

    assert_eq!(pump.speech_samples(), 3 * FIXTURE_SAMPLES);
    let lead_in_samples = 44_100 * 700 / 1_000;
    // One in about 150 noise samples is exactly zero and is not counted.
    assert!(pump.noise_samples() > lead_in_samples * 9 / 10);
    assert!(pump.noise_samples() <= lead_in_samples);
}

#[tokio::test]
async fn a_piper_answer_at_22_khz_plays_through_the_44_khz_output_for_its_full_length() {
    let mut harness = harness_for(FixtureVoice::Piper, false);
    let pump = harness.start_pump();
    let (chunks_sender, chunks) = mpsc::channel(8);
    let (playback_started, _started) = oneshot::channel();
    for chunk in ["One chunk.", "Two chunks."] {
        chunks_sender
            .send(chunk.to_owned())
            .await
            .expect("queue chunk");
    }
    drop(chunks_sender);

    within_deadline(harness.speech.speak(chunks, playback_started))
        .await
        .expect("the answer should play out");

    // Each chunk is FIXTURE_SAMPLES at 22.05 kHz, so the 44.1 kHz output plays twice as
    // many samples. Played unconverted it would last half as long, at double pitch.
    let expected = 2 * 2 * FIXTURE_SAMPLES;
    assert!(
        pump.speech_samples().abs_diff(expected) <= expected / 20,
        "{} speech samples heard, expected about {expected}",
        pump.speech_samples()
    );
    assert_eq!(harness.speech.health, SpeechHealth::Ready);
    assert_eq!(
        harness.requests(),
        ["One chunk.", "Two chunks."],
        "the Piper worker was accepted as ready"
    );
}

#[tokio::test]
async fn the_playback_signal_fires_on_the_first_spoken_chunk_and_skips_chunks_with_no_words() {
    let mut harness = harness(false);
    let _pump = harness.start_pump();
    let (chunks_sender, chunks) = mpsc::channel(8);
    let (playback_started, mut started) = oneshot::channel();
    // A lone "..." has nothing to say; "1." and "—" differ in whether they contain a
    // letter or digit.
    for chunk in ["...", "—", "1.", "Real words."] {
        chunks_sender
            .send(chunk.to_owned())
            .await
            .expect("queue chunk");
    }
    let mut speaking = Box::pin(harness.speech.speak(chunks, playback_started));

    // Wait for the first chunk that is actually spoken, with the text still open.
    tokio::select! {
        result = &mut speaking => panic!("answer ended with the text still open: {result:?}"),
        signal = within_deadline(&mut started) => signal.expect("playback should start"),
    }
    drop(chunks_sender);
    within_deadline(speaking)
        .await
        .expect("the answer should play out");

    assert_eq!(harness.requests(), ["1.", "Real words."]);
}

#[tokio::test]
async fn an_answer_with_no_spoken_chunks_ends_cleanly_without_a_playback_signal() {
    let mut harness = harness(false);
    let (chunks_sender, chunks) = mpsc::channel(8);
    let (playback_started, started) = oneshot::channel();
    chunks_sender
        .send("...".to_owned())
        .await
        .expect("queue chunk");
    drop(chunks_sender);

    within_deadline(harness.speech.speak(chunks, playback_started))
        .await
        .expect("an answer with nothing to say is not a failure");

    assert!(started.await.is_err(), "no chunk was played");
    assert!(harness.requests().is_empty());
    assert_eq!(harness.speech.health, SpeechHealth::Ready);
    assert!(harness.speech.active.is_none());
}

#[tokio::test]
async fn a_chunk_may_be_4096_bytes_but_not_4097() {
    assert_eq!(MAX_SPEECH_TEXT_BYTES, 4_096);
    // (bytes in the chunk, whether it is accepted)
    for (bytes, accepted) in [
        (MAX_SPEECH_TEXT_BYTES, true),
        (MAX_SPEECH_TEXT_BYTES + 1, false),
    ] {
        let mut harness = harness(false);
        let _pump = harness.start_pump();
        let (chunks_sender, chunks) = mpsc::channel(8);
        let (playback_started, _started) = oneshot::channel();
        chunks_sender
            .send("a".repeat(bytes))
            .await
            .expect("queue chunk");
        drop(chunks_sender);

        let result = within_deadline(harness.speech.speak(chunks, playback_started)).await;

        if accepted {
            result.expect("the largest chunk is spoken");
            assert_eq!(harness.requests().len(), 1);
        } else {
            assert!(matches!(result, Err(SpeechError::TextTooLarge)), "{bytes}");
            assert!(harness.requests().is_empty());
        }
        // Too much text is not a speech failure and never disables or counts.
        assert_eq!(harness.speech.health, SpeechHealth::Ready, "{bytes}");
        assert!(harness.speech.active.is_none(), "{bytes}");
        assert!(!harness.speech.is_playing(), "{bytes}");
    }
}

#[tokio::test]
async fn stopping_mid_answer_drops_queued_chunks_and_the_next_answer_is_spoken() {
    let mut harness = harness(false);
    let (chunks_sender, chunks) = mpsc::channel(8);
    let (playback_started, started) = oneshot::channel();
    for chunk in ["First chunk.", "Second chunk.", "Third chunk."] {
        chunks_sender
            .send(chunk.to_owned())
            .await
            .expect("queue chunk");
    }
    {
        // Nothing drains the mixer, so the first chunk never finishes playing.
        let mut speaking = Box::pin(harness.speech.speak(chunks, playback_started));
        tokio::select! {
            result = &mut speaking => panic!("answer ended while playing: {result:?}"),
            signal = within_deadline(started) => signal.expect("playback should start"),
        }
        // The user stops: the REPL drops the future, then calls `cancel`.
    }
    assert!(
        harness.speech.is_playing(),
        "audio is queued before the stop"
    );

    harness
        .speech
        .cancel()
        .await
        .expect("cancel settles cleanly");

    assert!(!harness.speech.is_playing(), "queued audio is dropped");
    assert!(harness.speech.active.is_none());
    assert_eq!(
        harness.speech.health,
        SpeechHealth::Ready,
        "a cancel is not a failure"
    );
    drop(chunks_sender);

    // The next answer starts fresh and plays only its own chunk.
    let pump = harness.start_pump();
    let (chunks_sender, chunks) = mpsc::channel(8);
    let (playback_started, _started) = oneshot::channel();
    chunks_sender
        .send("Fresh answer.".to_owned())
        .await
        .expect("queue chunk");
    drop(chunks_sender);
    within_deadline(harness.speech.speak(chunks, playback_started))
        .await
        .expect("the next answer is spoken");

    assert_eq!(
        harness.requests().last().map(String::as_str),
        Some("Fresh answer.")
    );
    assert_eq!(
        pump.speech_samples(),
        FIXTURE_SAMPLES,
        "chunks of the stopped answer never play"
    );
}

#[tokio::test]
async fn a_failure_on_the_second_chunk_ends_the_answer_counts_once_and_the_next_answer_is_accepted()
{
    let mut harness = harness(true);
    let _pump = harness.start_pump();
    let (chunks_sender, chunks) = mpsc::channel(8);
    let (playback_started, _started) = oneshot::channel();
    for chunk in ["First chunk.", "Second chunk.", "Third chunk."] {
        chunks_sender
            .send(chunk.to_owned())
            .await
            .expect("queue chunk");
    }

    let result = within_deadline(harness.speech.speak(chunks, playback_started)).await;

    assert!(
        matches!(
            result,
            Err(SpeechError::Synthesis(WorkerError::WorkerReported))
        ),
        "{result:?}"
    );
    assert_eq!(
        harness.requests(),
        ["First chunk.", "Second chunk."],
        "the third chunk is not attempted after the failure"
    );
    assert_eq!(
        harness.speech.health,
        SpeechHealth::Recovering {
            consecutive_failures: 1
        },
        "one failed answer is one failure, not one per remaining chunk"
    );
    assert!(harness.speech.worker.is_none(), "failed worker is replaced");
    assert!(harness.speech.active.is_none());
    assert!(
        !harness.speech.is_playing(),
        "the rest of the answer is dropped"
    );
    assert!(harness.speech.ensure_accepting().is_ok());

    // The next answer gets a fresh worker and is spoken in full.
    drop(chunks_sender);
    let (chunks_sender, chunks) = mpsc::channel(8);
    let (playback_started, _started) = oneshot::channel();
    for chunk in ["Next answer one.", "Next answer two."] {
        chunks_sender
            .send(chunk.to_owned())
            .await
            .expect("queue chunk");
    }
    drop(chunks_sender);
    within_deadline(harness.speech.speak(chunks, playback_started))
        .await
        .expect("the next answer is spoken");
    assert_eq!(harness.speech.health, SpeechHealth::Ready);
}
