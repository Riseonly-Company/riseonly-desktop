use std::time::Duration;

use gpui::Animation;

/// One full sweep of the highlight across a placeholder group.
///
/// The reference sweeps at roughly this cadence. It is deliberately slow: gpui
/// has no damage regions, so a permanently animating element repaints its whole
/// window every frame it advances, and the cost is battery and compositor load
/// that never appears in a frame trace.
pub const SHIMMER_PERIOD: Duration = Duration::from_millis(1200);

/// The looping driver for a skeleton group.
///
/// One per container, never one per shape. `gpui::AnimationExt::with_animation`
/// honours `App::reduce_motion` on its own — with it set the element renders in
/// its start state and no frames are scheduled at all — which is why the
/// reduce-motion policy is not repeated at the call site.
pub fn shimmer_animation() -> Animation {
    Animation::new(SHIMMER_PERIOD).repeat()
}

/// Where the highlight band sits, as a fraction of the container's width.
///
/// `leading` and `trailing` are the band's two edges. Both may fall outside
/// `0.0..=1.0`, which is what makes the band enter and leave rather than
/// appearing and disappearing at the edges.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ShimmerBand {
    pub leading: f32,
    pub trailing: f32,
}

impl ShimmerBand {
    pub fn width(&self) -> f32 {
        self.trailing - self.leading
    }

    pub fn centre(&self) -> f32 {
        (self.leading + self.trailing) / 2.0
    }
}

/// The band's position for one point in the sweep.
///
/// `progress` is the 0..=1 delta `with_animation` supplies. The travel is wider
/// than the container on both sides so the band is fully off-screen at the ends
/// of the loop; otherwise the highlight pops into existence at the left edge on
/// every repeat.
pub fn shimmer_band(progress: f32) -> ShimmerBand {
    const BAND_WIDTH: f32 = 0.45;

    let travel = 1.0 + BAND_WIDTH * 2.0;
    let leading = progress.clamp(0.0, 1.0) * travel - BAND_WIDTH * 2.0;

    ShimmerBand {
        leading,
        trailing: leading + BAND_WIDTH,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_band_starts_and_ends_entirely_outside_the_container() {
        let start = shimmer_band(0.0);
        assert!(
            start.trailing <= 0.0,
            "the highlight pops in at the left edge on every repeat: {start:?}"
        );

        let end = shimmer_band(1.0);
        assert!(
            end.leading >= 1.0,
            "the highlight is still visible when the loop restarts: {end:?}"
        );
    }

    #[test]
    fn the_band_crosses_the_middle_exactly_once_and_moves_forward() {
        let mut previous = f32::NEG_INFINITY;
        let mut crossings = 0;
        let mut was_before = true;

        for step in 0..=100 {
            let band = shimmer_band(step as f32 / 100.0);
            assert!(band.leading > previous, "the band moved backwards");
            previous = band.leading;

            let is_before = band.centre() < 0.5;
            if was_before && !is_before {
                crossings += 1;
            }
            was_before = is_before;
        }

        assert_eq!(crossings, 1);
    }

    #[test]
    fn the_band_keeps_its_width_throughout_the_sweep() {
        let first = shimmer_band(0.0).width();
        for step in 0..=100 {
            let width = shimmer_band(step as f32 / 100.0).width();
            assert!((width - first).abs() < 1e-5);
        }
    }

    #[test]
    fn a_progress_outside_the_unit_range_cannot_send_the_band_anywhere() {
        assert_eq!(shimmer_band(-5.0), shimmer_band(0.0));
        assert_eq!(shimmer_band(5.0), shimmer_band(1.0));
    }

    #[test]
    fn the_sweep_is_slow_enough_not_to_repaint_the_window_at_frame_rate() {
        assert!(SHIMMER_PERIOD >= Duration::from_millis(1000));
    }
}
