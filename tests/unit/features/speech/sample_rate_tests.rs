use super::*;

#[test]
fn only_rates_from_16_to_48_khz_are_accepted() {
    // (hz, accepted): both range edges, one outside each, and the extremes.
    let cases = [
        (0, false),
        (MIN_SAMPLE_RATE_HZ - 1, false),
        (MIN_SAMPLE_RATE_HZ, true),
        (22_050, true),
        (44_100, true),
        (MAX_SAMPLE_RATE_HZ, true),
        (MAX_SAMPLE_RATE_HZ + 1, false),
        (u32::MAX, false),
    ];

    for (hz, accepted) in cases {
        assert_eq!(SampleRate::new(hz).is_some(), accepted, "{hz} Hz");
    }
    assert_eq!(MIN_SAMPLE_RATE_HZ, 16_000);
    assert_eq!(MAX_SAMPLE_RATE_HZ, 48_000);
}

#[test]
fn an_utterance_may_last_120_seconds_at_any_accepted_rate() {
    assert_eq!(SampleRate::new(44_100).unwrap().max_samples(), 5_292_000);
    assert_eq!(SampleRate::new(22_050).unwrap().max_samples(), 2_646_000);
    assert_eq!(SampleRate::new(48_000).unwrap().max_samples(), 5_760_000);
}

#[test]
fn a_rate_converts_to_the_same_rodio_rate() {
    let rate = SampleRate::new(22_050).unwrap();

    assert_eq!(rate.hz(), 22_050);
    assert_eq!(rate.for_rodio().get(), 22_050);
}
