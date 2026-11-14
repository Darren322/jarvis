use jarvis_orb::input::{EMULATED_CLICK_WINDOW_SECONDS, TapFilter};

/// Each step is (time, touch began, click pressed); returns the taps produced.
fn taps(steps: &[(f64, bool, bool)]) -> usize {
    let mut filter = TapFilter::new();
    steps
        .iter()
        .filter(|(now, touch, click)| filter.update(*touch, *click, *now))
        .count()
}

#[test]
fn one_press_gives_one_tap_however_it_arrives() {
    let inside = EMULATED_CLICK_WINDOW_SECONDS;
    let outside = EMULATED_CLICK_WINDOW_SECONDS + 0.01;
    let cases = [
        ("nothing", vec![(0.0, false, false)], 0),
        ("click only", vec![(1.0, false, true)], 1),
        ("touch only", vec![(1.0, true, false)], 1),
        ("both in one frame", vec![(1.0, true, true)], 1),
        (
            "touch then click inside the window",
            vec![(1.0, true, false), (1.0 + inside, false, true)],
            1,
        ),
        (
            "touch then click outside the window",
            vec![(1.0, true, false), (1.0 + outside, false, true)],
            2,
        ),
        (
            "two quick deliberate clicks",
            vec![(1.0, false, true), (1.1, false, true)],
            2,
        ),
        (
            "emulated click, then another click",
            vec![(1.0, true, false), (1.1, false, true), (1.2, false, true)],
            2,
        ),
    ];
    for (name, steps, expected) in cases {
        assert_eq!(taps(&steps), expected, "{name}");
    }
}
