use super::*;
use tokio::{sync::watch::Receiver, time::timeout};

/// S16_LE bytes for `samples`.
fn pcm(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn loud() -> AudioChunk {
    pcm(&[i16::MIN; 512]).into()
}

#[test]
fn the_level_is_the_rms_on_a_scale_where_silence_is_0_and_full_scale_is_1() {
    let mut with_odd_byte = pcm(&[16_384; 4]);
    with_odd_byte.push(0xFF);
    // (name, chunk, expected level, tolerance)
    let cases: [(&str, Vec<u8>, f32, f32); 9] = [
        ("empty chunk", vec![], 0.0, 0.0),
        ("a lone byte is half a sample", vec![0xFF], 0.0, 0.0),
        ("silence", pcm(&[0; 1000]), 0.0, 0.0),
        ("every sample i16::MIN", pcm(&[i16::MIN; 1000]), 1.0, 0.0),
        (
            "a square wave between MIN and MAX",
            pcm(&[i16::MIN, i16::MAX].repeat(8)),
            1.0,
            1e-4,
        ),
        ("half scale", pcm(&[16_384; 4]), 0.5, 0.0),
        ("half scale, negative", pcm(&[-16_384; 4]), 0.5, 0.0),
        ("an odd trailing byte is ignored", with_odd_byte, 0.5, 0.0),
        (
            "i16::MAX is one step under full scale",
            pcm(&[i16::MAX; 8]),
            32_767.0 / 32_768.0,
            1e-6,
        ),
    ];
    for (name, bytes, expected, tolerance) in cases {
        let level = rms_level(&bytes);
        assert!((level - expected).abs() <= tolerance, "{name}: {level}");
    }
}

#[test]
fn the_level_is_the_root_of_the_mean_square_not_the_mean() {
    // The mean magnitude of one full-scale and one zero sample is 0.5, the RMS 1/sqrt(2).
    let level = rms_level(&pcm(&[i16::MIN, 0]));
    assert!(
        (level - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6,
        "{level}"
    );
}

fn start_meter() -> (
    broadcast::Sender<AudioChunk>,
    Receiver<f32>,
    tokio::task::JoinHandle<()>,
) {
    let (chunks, receiver) = broadcast::channel(8);
    let (levels, reader) = watch::channel(0.0);
    let task = tokio::spawn(run_level_meter(receiver, levels));
    (chunks, reader, task)
}

#[test]
fn the_level_interval_is_about_30_hertz() {
    let hertz = 1.0 / LEVEL_INTERVAL.as_secs_f64();
    assert!((29.0..=31.0).contains(&hertz), "{hertz}");
}

#[tokio::test(start_paused = true)]
async fn levels_are_published_about_30_times_a_second() {
    let (_chunks, mut levels, _task) = start_meter();
    let mut published = 0;
    let _ = timeout(Duration::from_secs(1), async {
        while levels.changed().await.is_ok() {
            published += 1;
        }
    })
    .await;
    // Ticks at 0, 33, ..., 990 ms.
    assert_eq!(published, 31);
}

#[tokio::test(start_paused = true)]
async fn a_chunk_is_reported_then_held_then_falls_to_silence() {
    let (chunks, mut levels, _task) = start_meter();
    chunks.send(loud()).unwrap();
    let start = Instant::now();

    let mut seen = Vec::new();
    // Ticks at 0, 33, ..., 330 ms.
    while start.elapsed() < Duration::from_millis(330) {
        levels.changed().await.unwrap();
        seen.push((start.elapsed(), *levels.borrow()));
    }

    // The chunk is held while no older than LEVEL_HOLD (250 ms): the last held tick is 231 ms.
    assert_eq!(seen.len(), 11);
    for (at, level) in seen {
        let expected = if at <= Duration::from_millis(231) {
            1.0
        } else {
            0.0
        };
        assert_eq!(level, expected, "at {at:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn the_newest_chunk_wins_when_several_arrive_within_one_tick() {
    let (chunks, mut levels, _task) = start_meter();
    chunks.send(loud()).unwrap();
    chunks.send(pcm(&[0; 512]).into()).unwrap();
    levels.changed().await.unwrap();
    assert_eq!(*levels.borrow(), 0.0);
    chunks.send(loud()).unwrap();
    levels.changed().await.unwrap();
    assert_eq!(*levels.borrow(), 1.0);
}

#[tokio::test(start_paused = true)]
async fn the_meter_ends_when_the_tap_closes_and_leaves_the_level_at_silence() {
    let (chunks, mut levels, task) = start_meter();
    chunks.send(loud()).unwrap();
    levels.changed().await.unwrap();
    assert_eq!(*levels.borrow(), 1.0);
    drop(chunks);
    task.await.unwrap();
    assert_eq!(*levels.borrow(), 0.0);
}

#[tokio::test(start_paused = true)]
async fn the_meter_ends_when_nobody_reads_the_levels() {
    let (_chunks, levels, task) = start_meter();
    drop(levels);
    task.await.unwrap();
}
