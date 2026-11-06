use super::*;
use rodio::mixer::{self, MixerSource};
use std::time::Duration;

#[test]
fn wake_noise_is_pcm16_range_noise_lasting_exactly_700ms() {
    let samples = wake_noise();

    assert_eq!(WAKE_NOISE_DURATION, Duration::from_millis(700));
    assert_eq!(WAKE_NOISE_SAMPLE_COUNT, 30_870);
    assert_eq!(samples.len(), WAKE_NOISE_SAMPLE_COUNT);
    assert!(samples.iter().all(|sample| {
        let pcm16 = *sample * 32_768.0;
        (-75.0..=75.0).contains(&pcm16) && pcm16.fract() == 0.0
    }));
    assert!(samples.iter().any(|sample| *sample != 0.0));
    assert!(samples.windows(2).any(|pair| pair[0] != pair[1]));
}

fn test_player() -> (AudioPlayer, MixerSource) {
    let (mixer, source) = mixer::mixer(
        OUTPUT_CHANNELS.try_into().expect("nonzero channel count"),
        OUTPUT_SAMPLE_RATE.try_into().expect("nonzero sample rate"),
    );
    let player = AudioPlayer::from_mixer(mixer, (), Arc::new(AtomicBool::new(false)));
    (player, source)
}

/// Plays everything queued, as the output device would, and returns the samples heard.
fn play_out(player: &AudioPlayer, source: &mut MixerSource) -> Vec<f32> {
    let mut heard = Vec::new();
    // Generous bound so a queue that never drains fails the test instead of hanging it.
    for _ in 0..1_000_000 {
        if !player.is_playing() {
            return heard;
        }
        heard.extend(source.next());
    }
    panic!("the queue did not drain");
}

#[test]
fn chunks_queue_back_to_back_after_one_wake_noise() {
    let (mut player, mut source) = test_player();
    let first = vec![0.5; 100];
    let second = vec![-0.5; 50];

    player.append(first.clone()).expect("queue first chunk");
    player.append(second.clone()).expect("queue second chunk");
    let heard = play_out(&player, &mut source);

    // rodio starts a queue with a few milliseconds of silence, so find the speech by level.
    let speech_start = heard
        .iter()
        .position(|sample| sample.abs() > 0.1)
        .expect("speech is heard");
    assert_eq!(&heard[speech_start..speech_start + 100], first.as_slice());
    assert_eq!(
        &heard[speech_start + 100..speech_start + 150],
        second.as_slice(),
        "no gap between chunks"
    );
    let lead_in = &heard[..speech_start];
    assert!(lead_in.iter().all(|sample| sample.abs() < 0.01));
    let noise_samples = lead_in.iter().filter(|sample| **sample != 0.0).count();
    // One in about 150 noise samples is exactly zero.
    assert!(noise_samples > WAKE_NOISE_SAMPLE_COUNT * 9 / 10);
    assert!(
        noise_samples <= WAKE_NOISE_SAMPLE_COUNT,
        "one lead-in, not one per chunk"
    );
}

#[test]
fn the_next_answer_gets_its_own_wake_noise() {
    let (mut player, mut source) = test_player();

    player.append(vec![0.5; 10]).expect("queue first answer");
    let first = play_out(&player, &mut source);
    player.finish();
    player.append(vec![0.5; 10]).expect("queue second answer");
    let second = play_out(&player, &mut source);

    assert!(first.len() >= WAKE_NOISE_SAMPLE_COUNT + 10);
    assert!(second.len() >= WAKE_NOISE_SAMPLE_COUNT + 10);
}

#[test]
fn stopping_drops_everything_queued() {
    let (mut player, mut source) = test_player();
    player.append(vec![0.5; 100]).expect("queue chunk");
    player.append(vec![0.5; 100]).expect("queue chunk");
    assert!(player.is_playing());

    player.stop();

    assert!(!player.is_playing());
    let heard: Vec<f32> = source.by_ref().take(1_000).collect();
    assert!(
        heard.iter().all(|sample| sample.abs() < 0.01),
        "a stopped queue plays no speech"
    );
}

#[test]
fn empty_chunks_and_failed_devices_are_rejected() {
    let (mut player, _source) = test_player();
    assert!(matches!(
        player.append(Vec::new()),
        Err(PlaybackError::EmptySamples)
    ));
    assert!(!player.is_playing());

    player.device_failed.store(true, Ordering::Release);
    assert!(matches!(
        player.append(vec![0.5]),
        Err(PlaybackError::DeviceFailed)
    ));
}
