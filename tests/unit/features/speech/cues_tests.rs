use super::*;
use crate::features::state::StateFeed;
use rodio::mixer::{self, MixerSource};
use std::{fs, pin::pin};

/// The stand-in output device's rate; the fixtures use it too, so rodio never resamples.
const OUTPUT_SAMPLE_RATE: u32 = 44_100;

fn mono_16_bit(rate: u32) -> hound::WavSpec {
    hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    }
}

fn write_wav<S: hound::Sample + Copy>(path: &Path, spec: hound::WavSpec, samples: &[S]) {
    let mut writer = hound::WavWriter::create(path, spec).expect("the WAV file should be created");
    for sample in samples {
        writer
            .write_sample(*sample)
            .expect("the sample should be written");
    }
    writer.finalize().expect("the WAV file should be finished");
}

fn write_good_wav(path: &Path, rate: u32, samples: &[i16]) {
    write_wav(path, mono_16_bit(rate), samples);
}

/// Samples at [`OUTPUT_SAMPLE_RATE`] of the fade-in (5 ms) and the fade-out (40 ms).
const FADE_IN_SAMPLES: usize = 220;
const FADE_OUT_SAMPLES: usize = 1_764;
/// Long enough (100 ms) to have an unfaded middle at [`OUTPUT_SAMPLE_RATE`].
const LONG_CUE_SAMPLES: usize = 4_410;

/// The part of a loaded cue that fading leaves alone; empty for nothing heard.
fn unfaded(heard: &[f32]) -> &[f32] {
    if heard.is_empty() {
        return heard;
    }
    &heard[FADE_IN_SAMPLES..heard.len() - FADE_OUT_SAMPLES]
}

/// What `decode` made of a file, without the audio.
#[derive(Debug, PartialEq)]
enum Outcome {
    Loaded,
    Open,
    Format,
    Empty,
    TooLong,
    Decode,
}

fn outcome(result: &Result<CueSound, CueError>) -> Outcome {
    match result {
        Ok(_) => Outcome::Loaded,
        Err(CueError::Open(_)) => Outcome::Open,
        Err(CueError::Format) => Outcome::Format,
        Err(CueError::Empty) => Outcome::Empty,
        Err(CueError::TooLong) => Outcome::TooLong,
        Err(CueError::Decode(_)) => Outcome::Decode,
    }
}

fn decode_outcome(path: &Path) -> Outcome {
    outcome(&decode(path, 1.0))
}

#[test]
fn a_cue_file_loads_only_when_it_is_16_bit_mono_pcm_with_audio_in_range() {
    let dir = tempfile::tempdir().expect("temporary directory should open");
    let path = dir.path().join("cue.wav");

    write_good_wav(&path, 44_100, &[100; 50]);
    let good = decode_outcome(&path);

    let missing = decode_outcome(&dir.path().join("missing.wav"));

    fs::write(&path, b"this is not a WAV file at all").expect("garbage should write");
    let garbage = decode_outcome(&path);

    write_wav(
        &path,
        hound::WavSpec {
            channels: 2,
            ..mono_16_bit(44_100)
        },
        &[100_i16; 50],
    );
    let stereo = decode_outcome(&path);

    write_wav(
        &path,
        hound::WavSpec {
            bits_per_sample: 8,
            ..mono_16_bit(44_100)
        },
        &[100_i16; 50],
    );
    let eight_bit = decode_outcome(&path);

    write_wav(
        &path,
        hound::WavSpec {
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
            ..mono_16_bit(44_100)
        },
        &[0.25_f32; 50],
    );
    let float = decode_outcome(&path);

    write_good_wav(&path, 8_000, &[100; 50]);
    let rate_out_of_range = decode_outcome(&path);

    write_good_wav(&path, 44_100, &[]);
    let no_samples = decode_outcome(&path);

    write_good_wav(&path, 44_100, &[100; 100]);
    let mut bytes = fs::read(&path).expect("the file should read");
    bytes.truncate(bytes.len() - 20);
    fs::write(&path, bytes).expect("the truncated file should write");
    let cut_short = decode_outcome(&path);

    assert_eq!(
        [
            ("good file", good),
            ("missing file", missing),
            ("garbage bytes", garbage),
            ("stereo", stereo),
            ("8-bit", eight_bit),
            ("32-bit float", float),
            ("8,000 Hz", rate_out_of_range),
            ("no samples", no_samples),
            ("data cut short", cut_short),
        ],
        [
            ("good file", Outcome::Loaded),
            ("missing file", Outcome::Open),
            ("garbage bytes", Outcome::Open),
            ("stereo", Outcome::Format),
            ("8-bit", Outcome::Format),
            ("32-bit float", Outcome::Format),
            ("8,000 Hz", Outcome::Format),
            ("no samples", Outcome::Empty),
            ("data cut short", Outcome::Decode),
        ]
    );
}

#[test]
fn a_cue_may_last_exactly_2_seconds_and_not_one_sample_more() {
    let dir = tempfile::tempdir().expect("temporary directory should open");
    let path = dir.path().join("cue.wav");

    for (hz, last_accepted) in [(16_000_u32, 32_000_usize), (44_100, 88_200)] {
        write_good_wav(&path, hz, &vec![100; last_accepted]);
        assert_eq!(
            decode_outcome(&path),
            Outcome::Loaded,
            "{last_accepted} samples at {hz} Hz"
        );

        write_good_wav(&path, hz, &vec![100; last_accepted + 1]);
        assert_eq!(
            decode_outcome(&path),
            Outcome::TooLong,
            "{} samples at {hz} Hz",
            last_accepted + 1
        );
    }
}

#[test]
fn loading_a_folder_reports_each_cue_that_did_not_load_and_keeps_the_rest() {
    let dir = tempfile::tempdir().expect("temporary directory should open");
    for cue in [
        Cue::Listening,
        Cue::Transcribing,
        Cue::Speaking,
        Cue::Idle,
        Cue::TimerEnded,
    ] {
        write_good_wav(&dir.path().join(cue.file_name()), 44_100, &[100; 50]);
    }
    // `Cue::Thinking` has no file; `Cue::Error` has a broken one.
    fs::write(dir.path().join(Cue::Error.file_name()), b"not a WAV file")
        .expect("garbage should write");

    let (sounds, failures) = CueSounds::load(dir.path(), 1.0);

    let failed: Vec<Cue> = failures.iter().map(|failure| failure.cue).collect();
    assert_eq!(failed, [Cue::Thinking, Cue::Error]);
    for cue in Cue::ALL {
        assert_eq!(
            sounds.sounds[cue.index()].is_some(),
            !failed.contains(&cue),
            "{cue:?}"
        );
    }
    assert!(!sounds.is_empty());
}

#[test]
fn an_empty_or_missing_folder_leaves_every_cue_silent() {
    let dir = tempfile::tempdir().expect("temporary directory should open");
    let missing_folder = dir.path().join("not-created");

    for folder in [dir.path(), missing_folder.as_path()] {
        let (sounds, failures) = CueSounds::load(folder, 1.0);

        let failed: Vec<Cue> = failures.iter().map(|failure| failure.cue).collect();
        assert_eq!(failed, Cue::ALL, "{folder:?}");
        assert!(sounds.is_empty(), "{folder:?}");
    }
}

#[test]
fn samples_are_scaled_to_the_range_of_minus_one_to_one_and_by_the_volume() {
    let dir = tempfile::tempdir().expect("temporary directory should open");
    let path = dir.path().join("cue.wav");
    // Each level fills the unfaded middle of a cue of its own.
    for (level, volume, expected) in [
        (16_384_i16, 1.0, 0.5),
        (16_384, 0.5, 0.25),
        (-32_768, 1.0, -1.0),
        (-32_768, 0.5, -0.5),
        (0, 1.0, 0.0),
    ] {
        write_good_wav(&path, 44_100, &vec![level; LONG_CUE_SAMPLES]);
        let sound = decode(&path, volume).expect("the file should load");

        let middle = unfaded(&sound.samples);
        assert!(!middle.is_empty());
        assert!(
            middle.iter().all(|sample| *sample == expected),
            "level {level}, volume {volume}"
        );
    }
}

/// A cue whose samples differ from every other cue's: `(index + 1) * 1,000`.
fn distinct_samples(cue: Cue) -> Vec<i16> {
    let level = i16::try_from((cue.index() + 1) * 1_000).expect("the level fits an i16");
    vec![level; LONG_CUE_SAMPLES]
}

/// What `distinct_samples` plays as, at volume 1.
fn distinct_heard(cue: Cue) -> Vec<f32> {
    distinct_samples(cue)
        .iter()
        .map(|sample| f32::from(*sample) / 32_768.0)
        .collect::<Vec<f32>>()[FADE_IN_SAMPLES..LONG_CUE_SAMPLES - FADE_OUT_SAMPLES]
        .to_vec()
}

/// The unfaded middle of what was drained, to compare with `distinct_heard`.
fn heard_middle(heard: &[f32]) -> Vec<f32> {
    unfaded(heard).to_vec()
}

/// Every cue in `cues` loaded from a file of its `distinct_samples`.
fn distinct_sounds(cues: &[Cue]) -> CueSounds {
    let dir = tempfile::tempdir().expect("temporary directory should open");
    for cue in cues {
        write_good_wav(
            &dir.path().join(cue.file_name()),
            OUTPUT_SAMPLE_RATE,
            &distinct_samples(*cue),
        );
    }
    CueSounds::load(dir.path(), 1.0).0
}

/// A player on a stand-in output device. The sender keeps the mixer on offer.
fn test_cue_player(sounds: CueSounds) -> (CuePlayer, MixerSource, watch::Sender<Option<Mixer>>) {
    let (mixer, source) = mixer::mixer(
        1.try_into().expect("nonzero channel count"),
        OUTPUT_SAMPLE_RATE.try_into().expect("nonzero sample rate"),
    );
    let (offer, offered) = watch::channel(Some(mixer));
    (CuePlayer::new(Arc::new(sounds), offered), source, offer)
}

/// Plays out everything queued, as the output device would.
fn drain(source: &mut MixerSource) -> Vec<f32> {
    source.by_ref().take(1_000_000).collect()
}

#[test]
fn playing_a_cue_returns_its_length_and_puts_its_samples_in_the_mixer() {
    let dir = tempfile::tempdir().expect("temporary directory should open");
    write_good_wav(
        &dir.path().join(Cue::Thinking.file_name()),
        OUTPUT_SAMPLE_RATE,
        &[4_096; LONG_CUE_SAMPLES],
    );
    let (sounds, failures) = CueSounds::load(dir.path(), 1.0);
    assert_eq!(failures.len(), Cue::ALL.len() - 1);
    let (player, mut source, _offer) = test_cue_player(sounds);

    let length = player.play(Cue::Thinking);

    assert_eq!(length, Duration::from_millis(100));
    let heard = drain(&mut source);
    assert_eq!(heard.len(), LONG_CUE_SAMPLES);
    assert_eq!(
        unfaded(&heard),
        vec![0.125; LONG_CUE_SAMPLES - FADE_IN_SAMPLES - FADE_OUT_SAMPLES]
    );
}

#[test]
fn playing_a_cue_with_no_sound_or_no_output_does_nothing_and_takes_no_time() {
    let (player, mut source, offer) = test_cue_player(distinct_sounds(&[Cue::Idle]));

    assert_eq!(player.play(Cue::Error), Duration::ZERO, "no sound for it");
    assert!(drain(&mut source).is_empty());

    offer.send_replace(None);
    assert_eq!(player.play(Cue::Idle), Duration::ZERO, "no output open");
    assert!(drain(&mut source).is_empty());
}

#[test]
fn a_cue_follows_the_output_that_is_open_when_it_plays() {
    let (player, mut first_source, offer) = test_cue_player(distinct_sounds(&[Cue::Idle]));
    let (second_mixer, mut second_source) = mixer::mixer(
        1.try_into().expect("nonzero channel count"),
        OUTPUT_SAMPLE_RATE.try_into().expect("nonzero sample rate"),
    );
    offer.send_replace(Some(second_mixer));

    player.play(Cue::Idle);

    assert!(drain(&mut first_source).is_empty());
    assert_eq!(
        heard_middle(&drain(&mut second_source)),
        distinct_heard(Cue::Idle)
    );
}

#[tokio::test(start_paused = true)]
async fn each_state_change_plays_its_cue_and_a_turn_that_never_spoke_ends_in_the_error_cue() {
    let sounds = distinct_sounds(&Cue::ALL);
    let (player, mut source, _offer) = test_cue_player(sounds);
    let feed = StateFeed::new();
    let mut task = pin!(run_state_cues(feed.observe(), player));

    let mut cues_heard = Vec::new();
    for state in [
        AssistantState::Listening,
        AssistantState::Transcribing,
        AssistantState::Thinking,
        AssistantState::Idle,
    ] {
        feed.publish(state);
        // The task never returns: this lets it handle the change, then gives up waiting.
        let waited = tokio::time::timeout(Duration::from_millis(10), &mut task).await;
        assert!(waited.is_err(), "the cue task never ends");
        cues_heard.push(heard_middle(&drain(&mut source)));
    }

    assert_eq!(
        cues_heard,
        [
            Vec::new(),
            distinct_heard(Cue::Transcribing),
            distinct_heard(Cue::Thinking),
            distinct_heard(Cue::Error),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_turn_that_spoke_ends_in_the_idle_cue() {
    let (player, mut source, _offer) = test_cue_player(distinct_sounds(&Cue::ALL));
    let feed = StateFeed::new();
    let mut task = pin!(run_state_cues(feed.observe(), player));

    let mut cues_heard = Vec::new();
    for state in [AssistantState::Speaking, AssistantState::Idle] {
        feed.publish(state);
        let waited = tokio::time::timeout(Duration::from_millis(10), &mut task).await;
        assert!(waited.is_err(), "the cue task never ends");
        cues_heard.push(heard_middle(&drain(&mut source)));
    }

    assert_eq!(
        cues_heard,
        [distinct_heard(Cue::Speaking), distinct_heard(Cue::Idle)]
    );
}

#[test]
fn which_cue_a_state_change_plays_depends_on_what_the_turn_did() {
    use AssistantState::{Idle, Listening, Speaking, Thinking, Transcribing};

    type Step = (AssistantState, Option<Cue>);
    let turns: [(&str, &[Step]); 8] = [
        (
            "a spoken turn that is answered aloud",
            &[
                (Listening, None),
                (Transcribing, Some(Cue::Transcribing)),
                (Thinking, Some(Cue::Thinking)),
                (Speaking, Some(Cue::Speaking)),
                (Idle, Some(Cue::Idle)),
            ],
        ),
        (
            "a typed prompt that is answered aloud",
            &[
                (Thinking, Some(Cue::Thinking)),
                (Speaking, Some(Cue::Speaking)),
                (Idle, Some(Cue::Idle)),
            ],
        ),
        (
            "an answer that is only printed",
            &[(Thinking, Some(Cue::Thinking)), (Idle, Some(Cue::Error))],
        ),
        (
            "nothing heard or the recording cancelled",
            &[(Listening, None), (Idle, Some(Cue::Error))],
        ),
        (
            "an idle that starts with nothing spoken",
            &[(Idle, Some(Cue::Error))],
        ),
        (
            "a tap that interrupts the voice goes straight to listening",
            &[
                (Thinking, Some(Cue::Thinking)),
                (Speaking, Some(Cue::Speaking)),
                (Listening, None),
            ],
        ),
        (
            "a second turn that is not spoken, after a spoken one ended in idle",
            &[
                (Thinking, Some(Cue::Thinking)),
                (Speaking, Some(Cue::Speaking)),
                (Idle, Some(Cue::Idle)),
                (Thinking, Some(Cue::Thinking)),
                (Idle, Some(Cue::Error)),
            ],
        ),
        (
            "a second turn that is not spoken, after a spoken one was interrupted",
            &[
                (Thinking, Some(Cue::Thinking)),
                (Speaking, Some(Cue::Speaking)),
                (Listening, None),
                (Transcribing, Some(Cue::Transcribing)),
                (Thinking, Some(Cue::Thinking)),
                (Idle, Some(Cue::Error)),
            ],
        ),
    ];

    for (name, steps) in turns {
        let mut sequence = CueSequence::default();
        let played: Vec<Step> = steps
            .iter()
            .map(|(state, _)| (*state, sequence.on_state(*state)))
            .collect();

        assert_eq!(played, steps, "{name}");
    }
}

/// A cue of `length` samples of a constant `0.5`, faded as `decode` fades it.
fn faded_constant(rate_hz: u32, length: usize) -> Vec<f32> {
    let rate = SampleRate::new(rate_hz).expect("accepted rate");
    let mut samples = vec![0.5; length];
    fade_edges(&mut samples, rate);
    samples
}

#[test]
fn a_cue_starts_and_ends_at_silence_and_is_at_full_level_between() {
    let dir = tempfile::tempdir().expect("temporary directory should open");
    let path = dir.path().join("cue.wav");
    write_good_wav(&path, 44_100, &vec![16_384; LONG_CUE_SAMPLES]);

    let sound = decode(&path, 1.0).expect("the file should load");

    assert_eq!(sound.samples.len(), LONG_CUE_SAMPLES);
    assert!(sound.samples[0].abs() < 0.01);
    assert!(sound.samples[LONG_CUE_SAMPLES - 1].abs() < 0.01);
    assert!(unfaded(&sound.samples).iter().all(|sample| *sample == 0.5));
}

#[test]
fn the_fades_last_exactly_5_ms_in_and_40_ms_out() {
    // (rate, fade-in samples, fade-out samples)
    for (hz, fade_in, fade_out) in [(16_000_u32, 80_usize, 640_usize), (44_100, 220, 1_764)] {
        let length = (fade_in + fade_out) * 2;
        let samples = faded_constant(hz, length);

        assert!(samples[fade_in - 1] < 0.5, "{hz} Hz: last faded-in sample");
        assert_eq!(samples[fade_in], 0.5, "{hz} Hz: first unfaded sample");
        assert_eq!(
            samples[length - fade_out - 1],
            0.5,
            "{hz} Hz: last unfaded sample"
        );
        assert!(
            samples[length - fade_out] < 0.5,
            "{hz} Hz: first faded-out sample"
        );
    }
}

#[test]
fn the_level_rises_through_the_fade_in_and_falls_through_the_fade_out() {
    let samples = faded_constant(44_100, LONG_CUE_SAMPLES);

    assert!(
        samples[..FADE_IN_SAMPLES]
            .windows(2)
            .all(|pair| pair[0] < pair[1])
    );
    assert!(
        samples[LONG_CUE_SAMPLES - FADE_OUT_SAMPLES..]
            .windows(2)
            .all(|pair| pair[0] > pair[1])
    );
}

#[test]
fn a_cue_shorter_than_both_fades_loads_and_keeps_its_length() {
    let dir = tempfile::tempdir().expect("temporary directory should open");
    let path = dir.path().join("cue.wav");

    for length in [
        1_usize,
        2,
        3,
        10,
        1_000,
        FADE_IN_SAMPLES + FADE_OUT_SAMPLES - 1,
    ] {
        write_good_wav(&path, 44_100, &vec![16_384; length]);

        let sound = decode(&path, 1.0).expect("the file should load");

        assert_eq!(sound.samples.len(), length, "{length} samples");
        assert!(sound.samples.iter().all(|sample| sample.abs() <= 0.5));
    }
}

#[test]
fn fading_does_not_change_the_length_a_cue_plays_for() {
    let dir = tempfile::tempdir().expect("temporary directory should open");
    write_good_wav(
        &dir.path().join(Cue::Idle.file_name()),
        OUTPUT_SAMPLE_RATE,
        &[8_000; LONG_CUE_SAMPLES],
    );
    let (sounds, _) = CueSounds::load(dir.path(), 1.0);
    let (player, mut source, _offer) = test_cue_player(sounds);

    let length = player.play(Cue::Idle);

    assert_eq!(length, Duration::from_millis(100));
    assert_eq!(drain(&mut source).len(), LONG_CUE_SAMPLES);
}
