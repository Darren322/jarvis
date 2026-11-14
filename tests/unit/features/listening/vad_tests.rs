use super::*;
// Through the module's re-export, the path ticket 07 will use.
use crate::features::listening::speech_ended;
use std::{pin::pin, time::Duration};
use tokio::time::timeout;

/// Real synthetic speech (macOS `say`, not a person): 2 s of 16 kHz mono S16_LE.
const SPEECH: &[u8] = include_bytes!("fixtures/speech_16k_mono_s16le.raw");
const WAIT: Duration = Duration::from_secs(1);

fn silence(milliseconds: usize) -> Vec<u8> {
    vec![0; milliseconds * 32]
}

/// Feeds `frames` and returns the index of the frame that ended it, if any.
fn first_end(frames: &[bool]) -> Option<usize> {
    let mut state = SpeechEnd::new();
    frames.iter().position(|&is_speech| state.push(is_speech))
}

fn frames(speech: u32, silent: u32) -> Vec<bool> {
    let mut all = vec![true; speech as usize];
    all.extend(vec![false; silent as usize]);
    all
}

#[test]
fn the_silence_length_ends_the_recording_at_its_first_frame_and_not_before() {
    // 1000 ms is 62.5 frames of 16 ms, rounded up to 63 (1008 ms).
    // (name, speech frames, silent frames, expected ending frame index)
    let cases = [
        ("62 silent frames", 6, 62, None),
        ("63 silent frames", 6, 63, Some(68)),
    ];
    for (name, speech, silent, expected) in cases {
        assert_eq!(first_end(&frames(speech, silent)), expected, "{name}");
    }
}

#[test]
fn the_minimum_speech_length_arms_the_stop_at_its_first_frame_and_not_before() {
    // Plenty of silence after, so only whether speech counted decides the result.
    // (name, speech frames, whether the recording ends)
    let cases = [
        ("5 speech frames (80 ms)", 5, false),
        ("6 speech frames (96 ms)", 6, true),
    ];
    for (name, speech, ends) in cases {
        let end = first_end(&frames(speech, 200));
        assert_eq!(end.is_some(), ends, "{name}");
    }
}

#[test]
fn silence_only_never_ends_the_recording() {
    assert_eq!(first_end(&frames(0, 630)), None);
}

#[test]
fn speech_then_silence_ends_the_recording() {
    assert!(first_end(&frames(50, 63)).is_some());
}

#[test]
fn speech_a_short_pause_then_speech_does_not_end_the_recording() {
    let mut all = frames(20, 62);
    all.extend(frames(20, 62));
    assert_eq!(first_end(&all), None);
}

#[test]
fn a_blip_before_real_speech_does_not_arm_the_stop() {
    // Short bursts separated by silence, each under the minimum, then a long silence.
    let mut all = Vec::new();
    for _ in 0..5 {
        all.extend(frames(5, 1));
    }
    all.extend(frames(0, 126));
    assert_eq!(first_end(&all), None);
}

fn send_all(tx: &broadcast::Sender<AudioChunk>, pcm: &[u8], chunk_bytes: usize) {
    for chunk in pcm.chunks(chunk_bytes) {
        tx.send(chunk.into()).unwrap();
    }
}

/// Runs the real future over `pcm` cut into `chunk_bytes` pieces; true if it completed.
async fn completes(pcm: &[u8], chunk_bytes: usize) -> bool {
    let (tx, rx) = broadcast::channel(1024);
    send_all(&tx, pcm, chunk_bytes);
    timeout(WAIT, speech_ended(rx)).await.is_ok()
}

#[test]
fn the_speech_fixture_is_two_seconds_of_whole_samples() {
    assert_eq!(SPEECH.len(), 64_000);
}

#[tokio::test(start_paused = true)]
async fn digital_silence_never_completes() {
    assert!(!completes(&silence(5_000), 4096).await);
}

#[tokio::test(start_paused = true)]
async fn speech_then_silence_completes() {
    let pcm = [SPEECH, &silence(1_500)].concat();
    assert!(completes(&pcm, 4096).await);
}

#[tokio::test(start_paused = true)]
async fn speech_then_less_than_the_silence_length_does_not_complete() {
    let pcm = [SPEECH, &silence(900)].concat();
    assert!(!completes(&pcm, 4096).await);
}

#[tokio::test(start_paused = true)]
async fn speech_a_short_pause_then_speech_does_not_complete() {
    let pcm = [SPEECH, &silence(500), SPEECH, &silence(500)].concat();
    assert!(!completes(&pcm, 4096).await);
}

#[tokio::test(start_paused = true)]
async fn chunk_sizes_that_do_not_divide_into_frames_give_the_same_result() {
    let pcm = [SPEECH, &silence(1_500)].concat();
    for chunk_bytes in [130, 666, 1000, 4098] {
        assert!(completes(&pcm, chunk_bytes).await, "{chunk_bytes}");
        let short = [SPEECH, &silence(900)].concat();
        assert!(!completes(&short, chunk_bytes).await, "{chunk_bytes}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_lagged_tap_does_not_end_it_and_it_keeps_listening() {
    let (tx, rx) = broadcast::channel(4);
    let stop = speech_ended(rx);
    let mut stop = pin!(stop);
    // Ten chunks into a four-chunk channel: the receiver lags.
    send_all(&tx, &silence(1_000), 3200);
    assert!(timeout(WAIT, &mut stop).await.is_err());
    send_all(&tx, SPEECH, SPEECH.len());
    send_all(&tx, &silence(1_500), 48_000);
    assert!(timeout(WAIT, &mut stop).await.is_ok());
}

#[tokio::test(start_paused = true)]
async fn a_closed_tap_never_completes_it() {
    let (tx, rx) = broadcast::channel(1024);
    send_all(&tx, SPEECH, 4096);
    send_all(&tx, &silence(500), 4096);
    drop(tx);
    assert!(
        timeout(Duration::from_secs(60), speech_ended(rx))
            .await
            .is_err()
    );
}
