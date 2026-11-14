//! Turns touch and mouse presses into one tap per physical press, with no window
//! types. On the Pi a finger arrives only as a left click (labwc mouse emulation);
//! other machines may deliver a touch, or a touch followed by an emulated click.

/// A click this soon after a touch (seconds) is the same press, emulated.
pub const EMULATED_CLICK_WINDOW_SECONDS: f64 = 0.5;

#[derive(Debug, Default)]
pub struct TapFilter {
    /// When the last touch began, in the caller's clock.
    last_touch: Option<f64>,
}

impl TapFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one frame: whether a touch began and whether the left button was
    /// pressed this frame, with the current time in seconds. Returns true when
    /// the frame holds a new tap.
    pub fn update(&mut self, touch_began: bool, click_pressed: bool, now: f64) -> bool {
        if touch_began {
            // A click in the same frame is the same press.
            self.last_touch = Some(now);
            return true;
        }
        if click_pressed {
            let emulated = self
                .last_touch
                .is_some_and(|at| now - at <= EMULATED_CLICK_WINDOW_SECONDS);
            // A click answered by a touch window is consumed; the next is new.
            self.last_touch = None;
            return !emulated;
        }
        false
    }
}
