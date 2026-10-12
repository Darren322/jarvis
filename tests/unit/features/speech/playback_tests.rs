use super::*;
use std::time::Duration;

#[test]
fn prepends_pcm16_range_noise_for_exactly_700ms_and_keeps_speech_tail() {
    let speech = vec![0.125, -0.25, 1.0];
    let samples = prepend_wake_noise(speech.clone());
    let prefix = &samples[..WAKE_NOISE_SAMPLE_COUNT];

    assert_eq!(WAKE_NOISE_DURATION, Duration::from_millis(700));
    assert_eq!(WAKE_NOISE_SAMPLE_COUNT, 30_870);
    assert_eq!(samples.len(), WAKE_NOISE_SAMPLE_COUNT + speech.len());
    assert!(prefix.iter().all(|sample| {
        let pcm16 = *sample * 32_768.0;
        (-75.0..=75.0).contains(&pcm16) && pcm16.fract() == 0.0
    }));
    assert!(prefix.iter().any(|sample| *sample != 0.0));
    assert!(prefix.windows(2).any(|pair| pair[0] != pair[1]));
    assert_eq!(&samples[WAKE_NOISE_SAMPLE_COUNT..], speech.as_slice());
}
