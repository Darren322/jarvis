use jarvis_orb::look::{EASE_RATE, Look, Rgb, View, ease_factor, level_response, look};
use orb_protocol::OrbState;

const OFFLINE: View = View::Offline;
const IDLE: View = View::State(OrbState::Idle);
const LISTENING: View = View::State(OrbState::Listening);
const TRANSCRIBING: View = View::State(OrbState::Transcribing);
const THINKING: View = View::State(OrbState::Thinking);
const SPEAKING: View = View::State(OrbState::Speaking);

/// Raw levels from silence to a clipped microphone, then bad input.
const LEVELS: [f32; 6] = [0.0, 0.01, 0.05, 0.1, 0.5, 1.0];
const BAD_LEVELS: [f32; 5] = [-1.0, 7.5, f32::NAN, f32::INFINITY, f32::NEG_INFINITY];

fn all_colors(look: &Look) -> [Rgb; 4] {
    [
        look.node_color,
        look.link_color,
        look.ring_color,
        look.core_color,
    ]
}

/// Every numeric field of a look, so a test can compare whole looks field by field.
fn numbers(look: &Look) -> Vec<f32> {
    let mut values: Vec<f32> = all_colors(look).into_iter().flatten().collect();
    values.extend([
        look.brightness,
        look.sphere_spin,
        look.ring_spin[0],
        look.ring_spin[1],
        look.ring_alpha,
        look.breath_depth,
        look.breath_rate,
        look.node_pulse,
        look.node_jitter,
        look.core_glow,
        look.ray_strength,
    ]);
    values
}

fn is_finite_and_in_range(look: &Look) -> bool {
    let unit_range = |v: f32| (0.0..=1.0).contains(&v);
    all_colors(look).iter().flatten().all(|c| unit_range(*c))
        && [
            look.brightness,
            look.ring_alpha,
            look.node_pulse,
            look.node_jitter,
            look.core_glow,
            look.ray_strength,
        ]
        .into_iter()
        .all(unit_range)
        && numbers(look).iter().all(|v| v.is_finite())
}

#[test]
fn every_view_at_every_level_gives_a_finite_look_within_range() {
    for view in View::ALL {
        for level in LEVELS.into_iter().chain(BAD_LEVELS) {
            let look = look(view, level);
            assert!(
                is_finite_and_in_range(&look),
                "{view:?} at {level}: {look:?}"
            );
        }
    }
}

#[test]
fn out_of_range_and_nan_levels_are_clamped_to_the_nearest_valid_level() {
    // (bad level, the valid level it must behave as)
    let cases = [
        (-1.0, 0.0),
        (f32::NEG_INFINITY, 0.0),
        (f32::NAN, 0.0),
        (7.5, 1.0),
        (f32::INFINITY, 1.0),
    ];
    for view in View::ALL {
        for (bad, valid) in cases {
            assert_eq!(look(view, bad), look(view, valid), "{view:?} at {bad}");
        }
    }
}

#[test]
fn views_cycle_through_all_six_in_order_and_wrap() {
    let mut view = OFFLINE;
    for expected in View::ALL.into_iter().skip(1).chain([OFFLINE]) {
        view = view.next();
        assert_eq!(view, expected);
    }
}

#[test]
fn offline_is_grey_and_dimmer_than_idle() {
    let offline = look(OFFLINE, 0.0);
    let idle = look(IDLE, 0.0);
    for [r, g, b] in all_colors(&offline) {
        assert!(r == g && g == b, "offline colour is not grey: {r} {g} {b}");
    }
    assert!(offline.brightness < idle.brightness);
    assert!(offline.core_glow < idle.core_glow);
    assert!(offline.ring_alpha < idle.ring_alpha);
    assert_eq!(offline.ray_strength, 0.0);
}

#[test]
fn offline_still_breathes_so_it_does_not_look_frozen() {
    let offline = look(OFFLINE, 0.0);
    assert!(offline.breath_depth > 0.0 && offline.breath_rate > 0.0);
}

#[test]
fn idle_is_blue_and_turns_slowly() {
    let idle = look(IDLE, 0.0);
    for [r, g, b] in [idle.node_color, idle.link_color, idle.ring_color] {
        assert!(b > r && b > g, "not blue-dominant: {r} {g} {b}");
    }
    assert!(idle.sphere_spin > 0.0 && idle.breath_depth > 0.0);
    assert!(idle.sphere_spin < look(THINKING, 0.0).sphere_spin);
}

#[test]
fn thinking_turns_its_rings_in_opposite_directions_around_a_gold_core() {
    let thinking = look(THINKING, 0.0);
    assert!(thinking.ring_spin[0] * thinking.ring_spin[1] < 0.0);
    let [r, g, b] = thinking.core_color;
    assert!(
        r >= 0.95 && (0.7..=0.85).contains(&g) && b <= 0.3,
        "not gold: {r} {g} {b}"
    );
    for view in View::ALL.into_iter().filter(|view| *view != THINKING) {
        assert!(
            thinking.core_glow > look(view, 0.0).core_glow,
            "thinking should glow more than {view:?}"
        );
    }
}

#[test]
fn transcribing_spins_a_ring_faster_than_any_other_view() {
    let fastest = |view: View| {
        look(view, 1.0)
            .ring_spin
            .into_iter()
            .map(f32::abs)
            .fold(0.0, f32::max)
    };
    for view in View::ALL.into_iter().filter(|view| *view != TRANSCRIBING) {
        assert!(fastest(TRANSCRIBING) > fastest(view), "{view:?}");
    }
}

#[test]
fn only_listening_and_speaking_respond_to_the_level() {
    for view in [OFFLINE, IDLE, TRANSCRIBING, THINKING] {
        let silent = look(view, 0.0);
        for level in LEVELS {
            assert_eq!(
                look(view, level),
                silent,
                "{view:?} reacted to level {level}"
            );
        }
    }
    for view in [LISTENING, SPEAKING] {
        assert_ne!(
            look(view, 0.0),
            look(view, 1.0),
            "{view:?} ignored the level"
        );
    }
}

#[test]
fn listening_and_speaking_never_fall_as_the_level_rises() {
    for view in [LISTENING, SPEAKING] {
        let mut previous = numbers(&look(view, 0.0));
        let mut rose_somewhere = false;
        for level in LEVELS.into_iter().skip(1) {
            let current = numbers(&look(view, level));
            for (before, after) in previous.iter().zip(&current) {
                assert!(after >= before, "{view:?} fell at level {level}");
                rose_somewhere |= after > before;
            }
            previous = current;
        }
        assert!(rose_somewhere);
    }
}

#[test]
fn listening_pulses_the_nodes_and_speaking_also_moves_them_and_the_rays() {
    assert_eq!(look(LISTENING, 0.0).node_pulse, 0.0);
    assert!(look(LISTENING, 0.05).node_pulse > 0.5);
    assert_eq!(look(SPEAKING, 0.0).node_jitter, 0.0);
    assert!(look(SPEAKING, 0.05).node_jitter > 0.5);
    assert!(look(SPEAKING, 0.05).ray_strength > look(SPEAKING, 0.0).ray_strength);
}

#[test]
fn ray_strength_is_highest_when_speaking_at_full_level() {
    let loudest = look(SPEAKING, 1.0).ray_strength;
    for view in View::ALL {
        for level in LEVELS {
            if view == SPEAKING && level == 1.0 {
                continue;
            }
            assert!(
                loudest > look(view, level).ray_strength,
                "{view:?} at {level} has rays as strong as speaking at full level"
            );
        }
    }
}

#[test]
fn each_view_has_its_own_look_at_silence() {
    let looks: Vec<Vec<f32>> = View::ALL
        .iter()
        .map(|view| numbers(&look(*view, 0.0)))
        .collect();
    for (i, a) in looks.iter().enumerate() {
        for b in &looks[i + 1..] {
            assert_ne!(a, b);
        }
    }
}

#[test]
fn level_response_boundaries() {
    assert_eq!(level_response(0.0), 0.0);
    assert!(level_response(1.0) >= 0.999);
    assert!(level_response(1.0) <= 1.0);
    // Out-of-range input is clamped, never extrapolated.
    assert_eq!(level_response(-0.5), 0.0);
    assert_eq!(level_response(f32::NAN), 0.0);
    assert_eq!(level_response(2.0), level_response(1.0));
}

#[test]
fn level_response_makes_quiet_speech_visible_and_rises_with_the_level() {
    // The gain curve must lift typical speech (0.01 to 0.1) well off the floor
    // while keeping room below the top, and never fall as the level rises.
    assert!(level_response(0.01) > 0.1);
    assert!(level_response(0.1) < 0.9);
    let responses: Vec<f32> = (0..=100)
        .map(|step| level_response(step as f32 / 100.0))
        .collect();
    assert!(responses.windows(2).all(|pair| pair[1] >= pair[0]));
    // Strictly rising across the range of real speech.
    assert!(level_response(0.05) > level_response(0.01));
    assert!(level_response(0.1) > level_response(0.05));
}

#[test]
fn lerp_at_zero_gives_the_start_at_one_gives_the_target_and_outside_is_clamped() {
    let from = look(OFFLINE, 0.0);
    let to = look(THINKING, 0.0);
    assert_eq!(from.lerp(&to, 0.0), from);
    assert_eq!(from.lerp(&to, 1.0), to);
    assert_eq!(from.lerp(&to, -3.0), from);
    assert_eq!(from.lerp(&to, 3.0), to);
    assert_eq!(from.lerp(&to, f32::NAN), from);
}

#[test]
fn lerp_halfway_is_between_the_two_looks_field_by_field() {
    let from = look(OFFLINE, 0.0);
    let to = look(SPEAKING, 1.0);
    let middle = numbers(&from.lerp(&to, 0.5));
    for ((start, end), mid) in numbers(&from).iter().zip(numbers(&to)).zip(middle) {
        assert!((mid - (start + end) / 2.0).abs() < 1e-6);
    }
}

#[test]
fn ease_factor_is_zero_for_no_time_and_bad_time_and_approaches_one() {
    assert_eq!(ease_factor(0.0), 0.0);
    assert_eq!(ease_factor(-1.0), 0.0);
    assert_eq!(ease_factor(f32::NAN), 0.0);
    assert!(ease_factor(0.016) > 0.0 && ease_factor(0.016) < 0.2);
    // About 95% of a change is done after a third of a second.
    assert!(ease_factor(3.0 / EASE_RATE) > 0.94);
    assert!(ease_factor(10.0) <= 1.0);
}
