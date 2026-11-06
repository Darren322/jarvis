use super::*;
use rodio::mixer::{self, MixerSource};
use std::time::Duration;

/// The stand-in output device's rate, which is also Supertonic's.
const OUTPUT_SAMPLE_RATE: u32 = 44_100;
/// Wake-noise samples at [`OUTPUT_SAMPLE_RATE`].
const OUTPUT_WAKE_NOISE_SAMPLES: usize = 30_870;

#[test]
fn wake_noise_is_pcm16_range_noise_lasting_exactly_700ms_at_the_speech_rate() {
    assert_eq!(WAKE_NOISE_DURATION, Duration::from_millis(700));
    for (hz, expected_samples) in [
        (16_000, 11_200),
        (22_050, 15_435),
        (OUTPUT_SAMPLE_RATE, OUTPUT_WAKE_NOISE_SAMPLES),
        (48_000, 33_600),
    ] {
        let rate = SampleRate::new(hz).expect("accepted rate");
        let samples = wake_noise(rate);

        assert_eq!(wake_noise_sample_count(rate), expected_samples, "{hz} Hz");
        assert_eq!(samples.len(), expected_samples, "{hz} Hz");
        assert!(samples.iter().all(|sample| {
            let pcm16 = *sample * 32_768.0;
            (-75.0..=75.0).contains(&pcm16) && pcm16.fract() == 0.0
        }));
        assert!(samples.iter().any(|sample| *sample != 0.0));
        assert!(samples.windows(2).any(|pair| pair[0] != pair[1]));
    }
}

fn output_rate() -> SampleRate {
    SampleRate::new(OUTPUT_SAMPLE_RATE).expect("the output rate is accepted")
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

    player
        .append(first.clone(), output_rate())
        .expect("queue first chunk");
    player
        .append(second.clone(), output_rate())
        .expect("queue second chunk");
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
    assert!(noise_samples > OUTPUT_WAKE_NOISE_SAMPLES * 9 / 10);
    assert!(
        noise_samples <= OUTPUT_WAKE_NOISE_SAMPLES,
        "one lead-in, not one per chunk"
    );
}

#[test]
fn the_next_answer_gets_its_own_wake_noise() {
    let (mut player, mut source) = test_player();

    player
        .append(vec![0.5; 10], output_rate())
        .expect("queue first answer");
    let first = play_out(&player, &mut source);
    player.finish();
    player
        .append(vec![0.5; 10], output_rate())
        .expect("queue second answer");
    let second = play_out(&player, &mut source);

    assert!(first.len() >= OUTPUT_WAKE_NOISE_SAMPLES + 10);
    assert!(second.len() >= OUTPUT_WAKE_NOISE_SAMPLES + 10);
}

#[test]
fn stopping_drops_everything_queued() {
    let (mut player, mut source) = test_player();
    player
        .append(vec![0.5; 100], output_rate())
        .expect("queue chunk");
    player
        .append(vec![0.5; 100], output_rate())
        .expect("queue chunk");
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
fn a_22_khz_chunk_is_resampled_to_the_output_rate_not_played_fast() {
    let (mut player, mut source) = test_player();
    let piper_rate = SampleRate::new(22_050).expect("Piper's rate is accepted");
    let chunk_samples = 22_050;

    player
        .append(vec![0.5; chunk_samples], piper_rate)
        .expect("queue 22.05 kHz chunk");
    let heard = play_out(&player, &mut source);

    // 1 s of speech at 22.05 kHz fills 1 s of a 44.1 kHz output: twice the samples.
    let speech_samples = heard.iter().filter(|sample| sample.abs() > 0.4).count();
    let expected = chunk_samples * 2;
    assert!(
        speech_samples.abs_diff(expected) <= expected / 20,
        "{speech_samples} output samples for {chunk_samples} input samples, expected about {expected}"
    );
}

#[test]
fn the_start_of_a_22_khz_answer_plays_at_full_length_after_the_wake_noise() {
    // rodio resamples in blocks sized by a buffer's total length, so a block that starts
    // inside the wake noise runs into the first speech samples and converts them at the
    // noise's rate. Piper's first syllable was heard squeezed ("…nit 734").
    let piper_rate = SampleRate::new(22_050).expect("Piper's rate is accepted");
    let opening_samples = 4_410; // 0.2 s at 22.05 kHz
    let mut chunk = vec![0.5; opening_samples];
    chunk.extend(vec![0.3; 22_050]);
    let (mut player, mut source) = test_player();

    player.append(chunk, piper_rate).expect("queue chunk");
    let heard = play_out(&player, &mut source);

    // At a 44.1 kHz output, the 0.2 s opening fills twice its own samples.
    let opening_heard = heard.iter().filter(|sample| sample.abs() > 0.45).count();
    let expected = opening_samples * 2;
    assert!(
        opening_heard.abs_diff(expected) <= expected / 50,
        "the opening filled {opening_heard} output samples, expected about {expected}"
    );
}

#[test]
fn empty_chunks_and_failed_devices_are_rejected() {
    let (mut player, _source) = test_player();
    assert!(matches!(
        player.append(Vec::new(), output_rate()),
        Err(PlaybackError::EmptySamples)
    ));
    assert!(!player.is_playing());

    player.device_failed.store(true, Ordering::Release);
    assert!(matches!(
        player.append(vec![0.5], output_rate()),
        Err(PlaybackError::DeviceFailed)
    ));
}
