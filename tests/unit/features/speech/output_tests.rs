use super::*;
use std::io::Write;

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

#[test]
fn decodes_bounded_mono_pcm16_wav() {
    let directory = tempfile::tempdir().expect("create fixture directory");
    let path = directory.path().join("answer.wav");
    write_test_wav(&path, EXPECTED_SAMPLE_RATE);

    let decoded = SpeechOutput::decode_wav(&path).expect("decode valid WAV");

    assert_eq!(decoded.samples.len(), 3);
    assert_eq!(decoded.samples[0], -1.0);
    assert_eq!(decoded.samples[1], 0.0);
    assert!(decoded.samples[2] > 0.999 && decoded.samples[2] < 1.0);
    assert!(decoded.duration < Duration::from_millis(1));
    assert!(decoded.playback_deadline() > Duration::from_millis(2_700));
    assert!(decoded.playback_deadline() < Duration::from_millis(2_701));
    let empty = ValidatedWav {
        samples: Vec::new(),
        duration: Duration::ZERO,
    };
    assert_eq!(empty.playback_deadline(), Duration::from_millis(2_700));
    let maximum = ValidatedWav {
        samples: Vec::new(),
        duration: Duration::from_secs(120),
    };
    assert_eq!(MAX_PLAYBACK_DURATION, Duration::from_millis(122_700));
    assert_eq!(maximum.playback_deadline(), Duration::from_millis(122_700));
}

#[test]
fn rejects_wrong_format_and_wav_size_bounds() {
    let directory = tempfile::tempdir().expect("create fixture directory");
    let path = directory.path().join("answer.wav");
    write_test_wav(&path, 48_000);

    assert!(matches!(
        SpeechOutput::decode_wav(&path),
        Err(SpeechError::InvalidWavFormat)
    ));

    let oversized_path = directory.path().join("oversized.wav");
    std::fs::File::create(&oversized_path)
        .expect("create oversized file")
        .set_len(MAX_WAV_BYTES + 1)
        .expect("extend sparse oversized file");
    assert!(matches!(
        SpeechOutput::decode_wav(&oversized_path),
        Err(SpeechError::WavTooLarge)
    ));

    let too_many_samples_path = directory.path().join("too-many-samples.wav");
    let data_bytes = u32::try_from((MAX_WAV_SAMPLES + 1) * 2).expect("bounded WAV size");
    let riff_bytes = 36_u32 + data_bytes;
    let mut wav =
        std::fs::File::create(&too_many_samples_path).expect("create sample-count fixture");
    wav.write_all(b"RIFF").expect("write RIFF tag");
    wav.write_all(&riff_bytes.to_le_bytes())
        .expect("write RIFF size");
    wav.write_all(b"WAVEfmt ").expect("write format tags");
    wav.write_all(&16_u32.to_le_bytes())
        .expect("write format chunk size");
    wav.write_all(&1_u16.to_le_bytes())
        .expect("write PCM format");
    wav.write_all(&EXPECTED_CHANNELS.to_le_bytes())
        .expect("write channel count");
    wav.write_all(&EXPECTED_SAMPLE_RATE.to_le_bytes())
        .expect("write sample rate");
    wav.write_all(&(EXPECTED_SAMPLE_RATE * 2).to_le_bytes())
        .expect("write byte rate");
    wav.write_all(&2_u16.to_le_bytes())
        .expect("write block alignment");
    wav.write_all(&EXPECTED_BITS_PER_SAMPLE.to_le_bytes())
        .expect("write bit depth");
    wav.write_all(b"data").expect("write data tag");
    wav.write_all(&data_bytes.to_le_bytes())
        .expect("write data size");
    wav.set_len(44 + u64::from(data_bytes))
        .expect("extend sparse sample-count fixture");
    drop(wav);
    assert!(matches!(
        SpeechOutput::decode_wav(&too_many_samples_path),
        Err(SpeechError::WavTooLong)
    ));
}

#[tokio::test]
async fn oversized_text_skips_speech_without_disabling_session() {
    let mut speech = SpeechOutput::new(TtsConfig {
        python: PathBuf::from("/unused/python"),
        worker_script: PathBuf::from("/unused/worker.py"),
        model_dir: PathBuf::from("/unused/model"),
        threads: 2,
        audio_device: None,
    });

    let oversized_text = "x".repeat(MAX_SPEECH_TEXT_BYTES + 1);
    let (playback_started, playback_started_rx) = tokio::sync::oneshot::channel();
    assert!(matches!(
        speech.speak(&oversized_text, playback_started).await,
        Err(SpeechError::TextTooLarge)
    ));
    assert!(playback_started_rx.await.is_err());
    assert!(!speech.is_disabled());
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
            duration: Duration::from_secs(1),
        })
    });
    let mut speech = SpeechOutput::new(TtsConfig {
        python: PathBuf::from("/unused/python"),
        worker_script: PathBuf::from("/unused/worker.py"),
        model_dir: PathBuf::from("/unused/model"),
        threads: 2,
        audio_device: None,
    });
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
    SpeechOutput::new(TtsConfig {
        python: PathBuf::from("/unused/python"),
        worker_script: PathBuf::from("/unused/worker.py"),
        model_dir: PathBuf::from("/unused/model"),
        threads: 2,
        audio_device: None,
    })
}

/// Starts a real child process in place of the Python worker. The owner is live and
/// never becomes ready, so `fail_operation` has to terminate it.
fn launch_stand_in_worker() -> SupertonicWorker {
    SupertonicWorker::launch(
        Path::new("/bin/cat"),
        Path::new("/dev/null"),
        Path::new("/unused/model"),
        Path::new("/unused/voice.bin"),
        2,
    )
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
    let (playback_started, _playback_started_rx) = tokio::sync::oneshot::channel();

    assert!(matches!(
        speech.speak("fourth answer", playback_started).await,
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
