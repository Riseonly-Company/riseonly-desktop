use std::time::Duration;

pub const BLINK_INTERVAL: Duration = Duration::from_millis(500);

/// How long the caret stays solid after the last edit.
///
/// A caret that blinks through a burst of typing reads as a dropped frame, so
/// every mutation interrupts the cycle and only this much silence restarts it.
pub const PAUSE_AFTER_EDIT: Duration = Duration::from_millis(500);

/// The blink cycle, with no timer in it.
///
/// Every scheduled wake-up carries the epoch it was scheduled under. Anything
/// that disturbs the cycle bumps the epoch, so a timer that was already in
/// flight resolves into a no-op instead of racing the new one. That is the whole
/// mechanism; the state itself is testable without a window.
#[derive(Clone, Copy, Debug)]
pub struct CaretBlink {
    visible: bool,
    paused: bool,
    focused: bool,
    epoch: u64,
}

impl Default for CaretBlink {
    fn default() -> Self {
        Self {
            visible: true,
            paused: true,
            focused: false,
            epoch: 0,
        }
    }
}

impl CaretBlink {
    pub fn is_visible(&self) -> bool {
        self.focused && self.visible
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn is_current(&self, epoch: u64) -> bool {
        self.epoch == epoch
    }

    /// Show the caret, stop the cycle, and hand back the epoch a resume must
    /// carry. Called from every mutation.
    pub fn interrupt(&mut self) -> u64 {
        self.visible = true;
        self.paused = true;
        self.epoch += 1;
        self.epoch
    }

    pub fn focus(&mut self) -> u64 {
        self.focused = true;
        self.interrupt()
    }

    pub fn blur(&mut self) -> u64 {
        self.focused = false;
        self.visible = true;
        self.paused = true;
        self.epoch += 1;
        self.epoch
    }

    /// Restart the cycle if this wake-up is still the current one.
    pub fn resume(&mut self, epoch: u64) -> bool {
        if self.epoch != epoch || !self.focused {
            return false;
        }
        self.paused = false;
        true
    }

    /// Flip the caret if this wake-up is still the current one. `false` means
    /// the caller should drop its timer rather than reschedule.
    pub fn tick(&mut self, epoch: u64) -> bool {
        if self.epoch != epoch || self.paused || !self.focused {
            return false;
        }
        self.visible = !self.visible;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unfocused_caret_is_never_drawn() {
        let blink = CaretBlink::default();
        assert!(!blink.is_visible());
    }

    #[test]
    fn a_focused_caret_starts_solid_and_then_blinks() {
        let mut blink = CaretBlink::default();
        let epoch = blink.focus();
        assert!(blink.is_visible());

        assert!(blink.resume(epoch));
        assert!(blink.tick(epoch));
        assert!(!blink.is_visible());
        assert!(blink.tick(epoch));
        assert!(blink.is_visible());
    }

    #[test]
    fn typing_stops_the_blink_and_keeps_the_caret_solid() {
        let mut blink = CaretBlink::default();
        let epoch = blink.focus();
        blink.resume(epoch);
        blink.tick(epoch);
        assert!(!blink.is_visible());

        let typed = blink.interrupt();
        assert!(blink.is_visible());

        assert!(!blink.tick(typed), "paused ticks must not flip the caret");
        assert!(blink.is_visible());

        for _ in 0..30 {
            blink.interrupt();
            assert!(blink.is_visible());
        }
    }

    #[test]
    fn a_timer_from_before_an_edit_resolves_into_nothing() {
        let mut blink = CaretBlink::default();
        let stale = blink.focus();
        blink.resume(stale);
        blink.interrupt();

        assert!(!blink.tick(stale));
        assert!(!blink.resume(stale));
        assert!(blink.is_visible());
    }

    #[test]
    fn blurring_invalidates_the_cycle() {
        let mut blink = CaretBlink::default();
        let epoch = blink.focus();
        blink.resume(epoch);
        let blurred = blink.blur();

        assert!(!blink.is_visible());
        assert!(!blink.tick(blurred));
        assert!(!blink.resume(blurred));
    }
}
