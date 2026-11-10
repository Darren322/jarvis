use super::*;
use rodio::buffer::SamplesBuffer;
use std::num::NonZero;
use tokio::time::Instant;

const SAMPLE_RATE_HZ: u32 = 44_100;
/// Samples in one window at [`SAMPLE_RATE_HZ`]: 44,100 / 30.
const WINDOW_SAMPLES: usize = 1_470;
const TOLERANCE: f32 = 1e-4;

fn mono_buffer(sample_rate_hz: u32, samples: Vec<f32>) -> SamplesBuffer {
    SamplesBuffer::new(
        NonZero::new(1).expect("nonzero channel count"),
        NonZero::new(sample_rate_hz).expect("nonzero sample rate"),
        samples,
    )
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= TOLERANCE,
        "level {actual}, expected {expected}"
    );
}

/// Pulls `count` samples, as the mixer would.
fn pull(tap: &mut impl Iterator<Item = f32>, count: usize) {
    for _ in 0..count {
        tap.next();
    }
}

/// True if a level was published since the last call.
fn took_update(reader: &mut LevelReceiver) -> bool {
    let changed = reader
        .level
        .has_changed()
        .expect("the level sender is alive");
    reader.level.mark_unchanged();
    changed
}

#[test]
fn the_level_is_the_rms_of_the_window_from_silence_to_full_scale() {
    let cases: [(&str, [f32; 4], f32); 8] = [
        ("silence", [0.0; 4], 0.0),
        ("full scale", [1.0; 4], 1.0),
        ("full scale, negative", [-1.0; 4], 1.0),
        ("full-scale square wave", [1.0, -1.0, 1.0, -1.0], 1.0),
        ("half scale", [0.5; 4], 0.5),
        (
            "half the samples at full scale",
            [1.0, 0.0, 1.0, 0.0],
            0.5_f32.sqrt(),
        ),
        ("louder than full scale is clipped", [3.0; 4], 1.0),
        (
            "non-finite samples count as silence",
            [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0],
            0.0,
        ),
    ];

    for (name, samples, expected) in cases {
        let mut window = LevelWindow::new(4);
        let levels: Vec<f32> = samples
            .into_iter()
            .filter_map(|sample| window.push(sample))
            .collect();

        assert_eq!(levels.len(), 1, "{name}: one window closes");
        assert_close(levels[0], expected);
    }
}

#[test]
fn a_window_is_a_thirtieth_of_a_second_of_samples() {
    let mono = NonZero::new(1).expect("nonzero channel count");
    let stereo = NonZero::new(2).expect("nonzero channel count");
    for (hz, channels, expected) in [
        (16_000, mono, 533),
        (22_050, mono, 735),
        (44_100, mono, 1_470),
        (48_000, mono, 1_600),
        (44_100, stereo, 2_940),
        // A window always closes, even for an absurdly low rate.
        (29, mono, 1),
    ] {
        let rate = NonZero::new(hz).expect("nonzero sample rate");
        assert_eq!(window_length(rate, channels), expected, "{hz} Hz");
    }
}

#[test]
fn the_first_update_comes_with_the_last_sample_of_the_first_window_not_before() {
    for hz in [22_050_u32, SAMPLE_RATE_HZ] {
        let window = usize::try_from(hz / LEVEL_UPDATES_PER_SECOND).expect("fits in usize");
        let level = PlaybackLevel::new();
        let mut reader = level.subscribe();
        let mut tap = level.tap(mono_buffer(hz, vec![1.0; window * 2]));

        pull(&mut tap, window - 1);
        assert!(!took_update(&mut reader), "{hz} Hz: {} samples", window - 1);
        assert_close(reader.level(), 0.0);

        pull(&mut tap, 1);
        assert!(took_update(&mut reader), "{hz} Hz: {window} samples");
        assert_close(reader.level(), 1.0);

        pull(&mut tap, window - 1);
        assert!(
            !took_update(&mut reader),
            "the second window is not done yet"
        );
        pull(&mut tap, 1);
        assert!(took_update(&mut reader), "the second window is done");
    }
}

#[test]
fn a_silent_clip_reads_0_and_a_full_scale_clip_reads_1() {
    for (name, amplitude, expected) in [("silent", 0.0, 0.0), ("full scale", 1.0, 1.0)] {
        let level = PlaybackLevel::new();
        let mut reader = level.subscribe();
        let mut tap = level.tap(mono_buffer(
            SAMPLE_RATE_HZ,
            vec![amplitude; WINDOW_SAMPLES * 3],
        ));

        let mut updates = 0;
        while tap.next().is_some() {
            if took_update(&mut reader) {
                updates += 1;
                assert_close(reader.level(), expected);
            }
        }

        assert_eq!(updates, 3, "{name}: an update per window, even for silence");
    }
}

#[test]
fn the_level_follows_the_samples_as_they_change() {
    let mut samples = vec![0.25; WINDOW_SAMPLES];
    samples.extend(vec![1.0; WINDOW_SAMPLES]);
    samples.extend(vec![0.0; WINDOW_SAMPLES]);
    let level = PlaybackLevel::new();
    let mut reader = level.subscribe();
    let mut tap = level.tap(mono_buffer(SAMPLE_RATE_HZ, samples));

    let mut heard = Vec::new();
    while tap.next().is_some() {
        if took_update(&mut reader) {
            heard.push(reader.level());
        }
    }

    assert_eq!(heard.len(), 3);
    for (actual, expected) in heard.into_iter().zip([0.25, 1.0, 0.0]) {
        assert_close(actual, expected);
    }
}

#[test]
fn nothing_more_is_published_once_the_source_has_ended() {
    let level = PlaybackLevel::new();
    let mut reader = level.subscribe();
    let mut tap = level.tap(mono_buffer(SAMPLE_RATE_HZ, vec![1.0; WINDOW_SAMPLES * 2]));
    pull(&mut tap, WINDOW_SAMPLES * 2);
    assert!(took_update(&mut reader));

    assert_eq!(tap.next(), None);
    assert_eq!(tap.next(), None);

    assert!(!took_update(&mut reader));
}

#[test]
fn ending_playback_sets_the_level_to_0_and_the_old_audio_publishes_nothing_more() {
    let mut level = PlaybackLevel::new();
    let mut reader = level.subscribe();
    let mut tap = level.tap(mono_buffer(SAMPLE_RATE_HZ, vec![1.0; WINDOW_SAMPLES * 4]));
    pull(&mut tap, WINDOW_SAMPLES * 2);
    assert_close(reader.level(), 1.0);
    took_update(&mut reader);

    level.end();

    assert_close(reader.level(), 0.0);
    assert!(
        took_update(&mut reader),
        "the reset reaches the reader as an update"
    );
    // The audio thread may still be inside the old source when playback ends.
    pull(&mut tap, WINDOW_SAMPLES * 2);
    assert!(!took_update(&mut reader));
    assert_close(reader.level(), 0.0);
}

#[test]
fn audio_queued_after_an_end_is_measured_again() {
    let mut level = PlaybackLevel::new();
    let mut reader = level.subscribe();
    let mut old_tap = level.tap(mono_buffer(SAMPLE_RATE_HZ, vec![1.0; WINDOW_SAMPLES * 2]));
    level.end();
    took_update(&mut reader);
    let mut new_tap = level.tap(mono_buffer(SAMPLE_RATE_HZ, vec![0.5; WINDOW_SAMPLES]));

    pull(&mut old_tap, WINDOW_SAMPLES);
    assert!(!took_update(&mut reader), "the old answer stays silent");
    pull(&mut new_tap, WINDOW_SAMPLES);

    assert!(took_update(&mut reader));
    assert_close(reader.level(), 0.5);
}

#[test]
fn a_missing_dropped_or_slow_reader_never_holds_up_the_samples() {
    // No reader was ever made.
    let level = PlaybackLevel::new();
    let mut tap = level.tap(mono_buffer(SAMPLE_RATE_HZ, vec![0.5; WINDOW_SAMPLES * 10]));
    assert_eq!(tap.by_ref().count(), WINDOW_SAMPLES * 10);

    // The only reader was dropped.
    let level = PlaybackLevel::new();
    drop(level.subscribe());
    let mut tap = level.tap(mono_buffer(SAMPLE_RATE_HZ, vec![0.5; WINDOW_SAMPLES * 10]));
    assert_eq!(tap.by_ref().count(), WINDOW_SAMPLES * 10);

    // A reader that never reads while 100 windows go by sees only the newest level.
    let level = PlaybackLevel::new();
    let reader = level.subscribe();
    let mut samples = vec![0.5; WINDOW_SAMPLES * 99];
    samples.extend(vec![0.75; WINDOW_SAMPLES]);
    let total = samples.len();
    let mut tap = level.tap(mono_buffer(SAMPLE_RATE_HZ, samples));
    assert_eq!(tap.by_ref().count(), total);
    assert_close(reader.level(), 0.75);
}

#[tokio::test(start_paused = true)]
async fn one_second_of_played_audio_gives_30_updates_33ms_apart() {
    const UPDATE_SPACING: Duration = Duration::from_nanos(1_000_000_000 / 30);
    let level = PlaybackLevel::new();
    let mut reader = level.subscribe();
    let mut tap = level.tap(mono_buffer(SAMPLE_RATE_HZ, vec![0.5; WINDOW_SAMPLES * 30]));
    drop(level);
    // A stand-in audio callback that consumes one window of samples per 33 ms.
    let device = tokio::spawn(async move {
        let mut ticks = tokio::time::interval(UPDATE_SPACING);
        for _ in 0..30 {
            ticks.tick().await;
            pull(&mut tap, WINDOW_SAMPLES);
        }
    });

    let mut update_times = Vec::new();
    while let Ok(heard) = reader.changed().await {
        assert_close(heard, 0.5);
        update_times.push(Instant::now());
    }
    device.await.expect("the stand-in device finishes");

    assert_eq!(update_times.len(), 30);
    // tokio's timer ticks in whole milliseconds, so a 33.3 ms period lands on 33 or 34 ms.
    for pair in update_times.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(
            (Duration::from_millis(33)..=Duration::from_millis(34)).contains(&gap),
            "updates {gap:?} apart"
        );
    }
    let span = update_times[29] - update_times[0];
    assert!(span < Duration::from_secs(1), "30 updates took {span:?}");
}

#[tokio::test(start_paused = true)]
async fn the_reader_learns_the_player_is_gone_after_the_last_level_is_read() {
    let level = PlaybackLevel::new();
    let mut reader = level.subscribe();
    let mut tap = level.tap(mono_buffer(SAMPLE_RATE_HZ, vec![1.0; WINDOW_SAMPLES]));
    pull(&mut tap, WINDOW_SAMPLES);
    drop(tap);
    drop(level);

    assert_close(
        reader.changed().await.expect("the last level is delivered"),
        1.0,
    );
    assert!(reader.changed().await.is_err());
}
