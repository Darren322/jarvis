use super::*;

#[test]
fn speech_cue_is_quiet_one_second_prefix_and_preserves_speech_samples() {
    let speech = vec![-0.75, -0.25, 0.0, 0.5, 1.0];

    let samples = prepend_speech_cue(speech.clone());
    let cue_sample_count = OUTPUT_SAMPLE_RATE as usize;
    let cue = &samples[..cue_sample_count];

    assert_eq!(cue.len(), cue_sample_count);
    assert_eq!(&samples[cue_sample_count..], speech.as_slice());
    assert!(cue.iter().all(|sample| sample.is_finite()));

    let peak = cue
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0_f32, f32::max);
    let expected_peak = 1_600.0 / 32_768.0;
    assert!(peak > 0.0);
    assert!(peak > expected_peak * 0.999);
    assert!(peak <= expected_peak);
}
