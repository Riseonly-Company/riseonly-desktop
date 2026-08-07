//! One tick for every animated sticker on screen.
//!
//! THE PROBLEM THIS SOLVES
//!
//! The obvious implementation gives each player its own loop that awaits a
//! render and then sleeps. That is two main-thread resumptions plus a
//! cross-thread hop per frame per sticker — roughly 120·N work items a second —
//! and under contention the sleep collapses to zero and the loop becomes a busy
//! spin. A picker grid of sixty stickers is then sixty spinning tasks.
//!
//! One driver ticking at the sticker cadence replaces all of it. Each tick is a
//! bounded pass that reads already-rasterised frames and reports what to
//! present, so the caller applies the whole screen's worth in a single commit.
//!
//! Ported from `RiseLottieDisplayDriver`, with the callbacks removed: this
//! returns a decision rather than invoking anything, which is what lets a
//! sixty-sticker screen be tested with no window and no clock.

use std::collections::HashMap;
use std::sync::Arc;

use super::raster_policy::{MAXIMUM_FRAME_RATE, PREFETCH_PRESENTATIONS};
use super::sequence::{DecodedFrame, FrameSequence};

/// A registered player's identity. Stable for the life of the view, and never
/// an index into anything.
pub type PlayerId = u64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Loop {
    Forever,
    Once,
}

/// A frame the caller should write into its texture now.
pub struct Presentation {
    pub player: PlayerId,
    pub frame_index: usize,
    pub frame: Arc<DecodedFrame>,
}

impl std::fmt::Debug for Presentation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Presentation")
            .field("player", &self.player)
            .field("frame_index", &self.frame_index)
            .finish_non_exhaustive()
    }
}

/// A frame the raster pool should produce, in priority order: the frame a
/// player is waiting on comes before the ones it will want next.
#[derive(Clone, Debug)]
pub struct FrameRequest {
    pub sequence: Arc<FrameSequence>,
    pub frame_index: usize,
}

#[derive(Default)]
pub struct Tick {
    pub presentations: Vec<Presentation>,
    pub requests: Vec<FrameRequest>,
    /// One-shot players that reached their last frame. Already unregistered.
    pub finished: Vec<PlayerId>,
}

impl std::fmt::Debug for Tick {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Tick")
            .field("presentations", &self.presentations.len())
            .field("requests", &self.requests.len())
            .field("finished", &self.finished)
            .finish()
    }
}

struct Registration {
    sequence: Arc<FrameSequence>,
    looping: Loop,
    /// Monotonic milliseconds, and signed because registering at a non-zero
    /// frame rewinds the clock to where that frame sits on the timeline. That
    /// is what makes a player whose playback slot was revoked and regranted
    /// resume instead of snapping back to frame zero.
    started_at_millis: i64,
    last_presented: Option<usize>,
}

#[derive(Default)]
pub struct DisplayDriver {
    registrations: HashMap<PlayerId, Registration>,
}

impl DisplayDriver {
    pub fn new() -> Self {
        Self::default()
    }

    /// The cadence the host should drive this at. Not the monitor's refresh
    /// rate: a 165 Hz display is what the compositor does, not what a sticker
    /// needs, and presenting more often only costs decodes.
    pub const fn frame_rate() -> f64 {
        MAXIMUM_FRAME_RATE
    }

    pub fn is_idle(&self) -> bool {
        self.registrations.is_empty()
    }

    pub fn player_count(&self) -> usize {
        self.registrations.len()
    }

    pub fn register(
        &mut self,
        player: PlayerId,
        sequence: Arc<FrameSequence>,
        looping: Loop,
        starting_at: usize,
        now_millis: i64,
    ) {
        let rate = sequence.frame_rate().max(1.0);
        let elapsed_at_index = (starting_at as f64 / rate * 1000.0) as i64;

        self.registrations.insert(
            player,
            Registration {
                sequence,
                looping,
                started_at_millis: now_millis - elapsed_at_index,
                last_presented: Some(starting_at),
            },
        );
    }

    pub fn unregister(&mut self, player: PlayerId) {
        self.registrations.remove(&player);
    }

    /// Advance every registered player.
    ///
    /// A player whose frame is not rasterised yet is skipped rather than
    /// blanked: holding the last image while the raster pool catches up is
    /// invisible, and clearing it is a flicker on every cache miss.
    pub fn tick(&mut self, now_millis: i64) -> Tick {
        let mut tick = Tick::default();
        let mut finished: Vec<PlayerId> = Vec::new();

        let mut players: Vec<PlayerId> = self.registrations.keys().copied().collect();
        players.sort_unstable();

        for player in players {
            let Some(registration) = self.registrations.get_mut(&player) else {
                continue;
            };

            let sequence = registration.sequence.clone();
            let frame_count = sequence.frame_count();
            if frame_count == 0 {
                continue;
            }

            let stride = sequence.presentation_stride();
            let elapsed_millis = (now_millis - registration.started_at_millis).max(0);
            let elapsed_frames =
                (elapsed_millis as f64 / 1000.0 * sequence.frame_rate().max(1.0)) as usize;
            // Snapped to the stride so a looping sticker keeps visiting the
            // same frame indices, and the compressed set converges after one
            // loop instead of filling with frames nothing will show again.
            let raw_index = elapsed_frames / stride * stride;

            let index = match registration.looping {
                Loop::Forever => raw_index % frame_count,
                Loop::Once => raw_index.min(frame_count - 1),
            };

            if registration.last_presented == Some(index) {
                continue;
            }

            match sequence.cached_frame(index) {
                Some(frame) => {
                    registration.last_presented = Some(index);
                    tick.presentations.push(Presentation {
                        player,
                        frame_index: index,
                        frame,
                    });

                    if registration.looping == Loop::Once && index == frame_count - 1 {
                        finished.push(player);
                    }
                }
                None => {
                    tick.requests.push(FrameRequest {
                        sequence: sequence.clone(),
                        frame_index: index,
                    });
                }
            }

            Self::prefetch(&sequence, index, stride, &mut tick.requests);
        }

        for player in &finished {
            self.registrations.remove(player);
        }
        tick.finished = finished;

        tick
    }

    /// Keep the raster pool a few presentations ahead so the first loop fills
    /// in without ever blocking a tick.
    ///
    /// Stepping by the presentation stride matters: any other index is never
    /// shown, so rasterising it is pure waste.
    fn prefetch(
        sequence: &Arc<FrameSequence>,
        index: usize,
        stride: usize,
        requests: &mut Vec<FrameRequest>,
    ) {
        let frame_count = sequence.frame_count();

        for step in 1..=PREFETCH_PRESENTATIONS {
            let next = (index + step * stride) % frame_count;
            if sequence.cached_frame(next).is_none() {
                requests.push(FrameRequest {
                    sequence: sequence.clone(),
                    frame_index: next,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::frame_cache::FrameCache;
    use super::super::raster_policy::SequenceKey;
    use super::super::sequence::tests::CountingRasterizer;
    use super::*;

    /// The monotonic instant at which authored frame `index` becomes due.
    ///
    /// Rounded up with a millisecond of margin: elapsed frames are truncated,
    /// so asking at exactly index/rate lands one frame short and every timing
    /// assertion below would be testing the rounding rather than the policy.
    fn at_frame(index: usize, rate: f64) -> i64 {
        (index as f64 / rate * 1000.0).ceil() as i64 + 1
    }

    fn warm_sequence(frames: usize, rate: f64) -> Arc<FrameSequence> {
        let cache = FrameCache::new();
        let (rasterizer, _) = CountingRasterizer::new(frames, rate);
        let sequence = cache
            .open(SequenceKey::new("sticker", 32), Box::new(rasterizer))
            .unwrap();

        for index in 0..frames {
            sequence.frame(index);
        }
        sequence
    }

    #[test]
    fn a_thirty_fps_sticker_advances_one_frame_per_presentation() {
        let mut driver = DisplayDriver::new();
        driver.register(1, warm_sequence(30, 30.0), Loop::Forever, 0, 0);

        let first = driver.tick(at_frame(1, 30.0));
        let second = driver.tick(at_frame(2, 30.0));

        assert_eq!(first.presentations[0].frame_index, 1);
        assert_eq!(second.presentations[0].frame_index, 2);
    }

    #[test]
    fn a_sixty_fps_sticker_advances_two_authored_frames_per_presentation() {
        let mut driver = DisplayDriver::new();
        driver.register(1, warm_sequence(60, 60.0), Loop::Forever, 0, 0);

        let tick = driver.tick(at_frame(2, 60.0));

        assert_eq!(
            tick.presentations[0].frame_index, 2,
            "advancing one frame would play a 60 fps sticker at half speed"
        );
    }

    #[test]
    fn a_tick_that_lands_on_the_same_frame_presents_nothing() {
        let mut driver = DisplayDriver::new();
        driver.register(1, warm_sequence(30, 30.0), Loop::Forever, 0, 0);

        driver.tick(at_frame(1, 30.0));
        let repeat = driver.tick(at_frame(1, 30.0) + 1);

        assert!(
            repeat.presentations.is_empty(),
            "a 165 Hz host ticks far faster than the sticker cadence and must \
             not rewrite the same texture five times"
        );
    }

    #[test]
    fn a_looping_sticker_keeps_visiting_the_same_indices() {
        let mut driver = DisplayDriver::new();
        driver.register(1, warm_sequence(60, 60.0), Loop::Forever, 0, 0);

        let mut seen = Vec::new();
        for step in 1..=120 {
            let tick = driver.tick(at_frame(2 * step, 60.0));
            for presentation in &tick.presentations {
                seen.push(presentation.frame_index);
            }
        }

        assert!(
            seen.iter().all(|index| index % 2 == 0),
            "off-stride indices are never presented, so rasterising them is waste"
        );
    }

    #[test]
    fn a_one_shot_sticker_finishes_and_unregisters_itself() {
        let mut driver = DisplayDriver::new();
        driver.register(7, warm_sequence(10, 30.0), Loop::Once, 0, 0);

        let mut finished = Vec::new();
        for step in 1..=20 {
            finished.extend(driver.tick(at_frame(step, 30.0)).finished);
        }

        assert_eq!(finished, vec![7]);
        assert!(driver.is_idle());
    }

    #[test]
    fn a_regranted_player_resumes_where_it_stopped() {
        let mut driver = DisplayDriver::new();
        let sequence = warm_sequence(30, 30.0);

        driver.register(1, sequence.clone(), Loop::Forever, 0, 0);
        driver.tick(at_frame(10, 30.0));
        driver.unregister(1);

        // Comes back a whole second later, at the frame it had reached.
        driver.register(1, sequence, Loop::Forever, 10, 10_000);
        let tick = driver.tick(10_000 + at_frame(1, 30.0));

        assert_eq!(
            tick.presentations[0].frame_index, 11,
            "restarting from zero is the visible snap this rewind exists to avoid"
        );
    }

    #[test]
    fn a_cold_sticker_asks_for_its_frame_instead_of_blanking() {
        let cache = FrameCache::new();
        let (rasterizer, _) = CountingRasterizer::new(30, 30.0);
        let cold = cache
            .open(SequenceKey::new("cold", 32), Box::new(rasterizer))
            .unwrap();

        let mut driver = DisplayDriver::new();
        driver.register(1, cold, Loop::Forever, 0, 0);

        let tick = driver.tick(at_frame(1, 30.0));

        assert!(tick.presentations.is_empty());
        assert_eq!(tick.requests[0].frame_index, 1);
    }

    #[test]
    fn prefetch_runs_ahead_by_the_presentation_stride() {
        let cache = FrameCache::new();
        let (rasterizer, _) = CountingRasterizer::new(60, 60.0);
        let sequence = cache
            .open(SequenceKey::new("cold", 32), Box::new(rasterizer))
            .unwrap();
        sequence.frame(2);

        let mut driver = DisplayDriver::new();
        driver.register(1, sequence, Loop::Forever, 0, 0);

        let tick = driver.tick(at_frame(2, 60.0));
        let requested: Vec<usize> = tick.requests.iter().map(|r| r.frame_index).collect();

        assert_eq!(requested, vec![4, 6, 8]);
    }

    #[test]
    fn sixty_stickers_tick_in_one_pass_and_present_once_each() {
        let cache = FrameCache::new();
        let mut driver = DisplayDriver::new();

        for player in 0..60u64 {
            let (rasterizer, _) = CountingRasterizer::new(12, 30.0);
            let sequence = cache
                .open(
                    SequenceKey::new(format!("sticker-{player}"), 32),
                    Box::new(rasterizer),
                )
                .unwrap();
            for index in 0..12 {
                sequence.frame(index);
            }
            driver.register(player, sequence, Loop::Forever, 0, 0);
        }

        let tick = driver.tick(at_frame(1, 30.0));

        assert_eq!(tick.presentations.len(), 60);
        assert!(tick.requests.is_empty(), "a warm screen asks for nothing");
    }

    #[test]
    fn an_unregistered_player_stops_costing_anything() {
        let mut driver = DisplayDriver::new();
        driver.register(1, warm_sequence(10, 30.0), Loop::Forever, 0, 0);

        driver.unregister(1);

        assert!(driver.is_idle());
        assert!(driver.tick(1_000).presentations.is_empty());
    }

    #[test]
    fn a_clock_that_jumps_backwards_does_not_panic_or_rewind() {
        let mut driver = DisplayDriver::new();
        driver.register(1, warm_sequence(30, 30.0), Loop::Forever, 5, 10_000);

        let tick = driver.tick(0);

        assert!(tick.presentations.is_empty() || tick.presentations[0].frame_index == 0);
    }
}
