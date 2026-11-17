use std::time::Duration;

use gpui::{px, App, Context, EventEmitter, Pixels, WeakEntity, Window};
use rise_core::foundation::foundation_date::RiseClock;

use crate::animated_tabs_ui::animated_tabs_types::AnimatedTabsMetrics;
use crate::swiftui::animated::{Animated, MotionTime};
use crate::swiftui::environment::{env, EnvKey};
use crate::swiftui::motion::duration_from_seconds;

/// whether a pager strip is moving, under a pointer or settling
/// work that waits for rest asks here and looks again on AnimatedTabsStripDidRest
pub struct AnimatedTabsStripActivity;

impl AnimatedTabsStripActivity {
    pub const DRAG_FRAME_LIFETIME: Duration = Duration::from_millis(250);

    /// the driver entity is the strip identity, so a released strip just reads as resting
    /// no registry of strips to keep in step
    pub fn is_moving(strip: &WeakEntity<AnimatedTabsStripMotionDriver>, cx: &App) -> bool {
        strip
            .upgrade()
            .is_some_and(|driver| driver.read(cx).is_moving())
    }

    /// the strip a page is rendered in
    /// a page cant walk up to its scroll view in gpui
    /// so the pager hands its driver down through AnimatedTabsStripKey, read it in render
    pub fn enclosing(cx: &App) -> Option<WeakEntity<AnimatedTabsStripMotionDriver>> {
        env::<AnimatedTabsStripKey>(cx)
    }
}

/// the pager driver, scoped over its pages with env_scope
pub struct AnimatedTabsStripKey;

impl EnvKey for AnimatedTabsStripKey {
    type Value = Option<WeakEntity<AnimatedTabsStripMotionDriver>>;

    fn default_value() -> Self::Value {
        None
    }
}

/// a strip that was moving rests now, emitted once per rest
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimatedTabsStripDidRest;

/// page strip motion lives in this entity and not in the pager layout
/// resting page is a whole page offset and the live drag is one translation on top
/// so a pointer frame changes one number and pages arent laid out again
pub struct AnimatedTabsStripMotionDriver {
    bound: bool,

    page_width: f32,
    page_count: usize,
    resting_index: usize,

    right_to_left: bool,

    visual_index: f32,

    settle: Animated<f32>,
    settle_generation: u64,
    is_settling: bool,

    interruption_progress: f32,

    is_dragging: bool,
    last_drag_frame: MotionTime,
    published_moving: bool,
}

impl EventEmitter<AnimatedTabsStripDidRest> for AnimatedTabsStripMotionDriver {}

impl Default for AnimatedTabsStripMotionDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl AnimatedTabsStripMotionDriver {
    pub fn new() -> Self {
        Self {
            bound: false,
            page_width: 1.0,
            page_count: 1,
            resting_index: 0,
            right_to_left: false,
            visual_index: 0.0,
            settle: Animated::new(0.0),
            settle_generation: 0,
            is_settling: false,
            interruption_progress: 0.0,
            is_dragging: false,
            last_drag_frame: Duration::ZERO,
            published_moving: false,
        }
    }

    pub fn is_bound(&self) -> bool {
        self.bound
    }

    pub fn is_settling(&self) -> bool {
        self.is_settling
    }

    pub fn interruption_progress(&self) -> f32 {
        self.interruption_progress
    }

    /// true under a pointer that moved it lately, or while settling
    pub fn is_moving(&self) -> bool {
        self.is_settling
            || (self.is_dragging
                && RiseClock::monotonic().saturating_sub(self.last_drag_frame)
                    < AnimatedTabsStripActivity::DRAG_FRAME_LIFETIME)
    }

    /// called by the pager once its width is measured
    /// until then motion calls only keep the index since a pager not laid out has no strip to move
    pub fn bind(
        &mut self,
        page_width: Pixels,
        page_count: usize,
        resting_index: usize,
        right_to_left: bool,
        cx: &mut Context<Self>,
    ) {
        if self.bound {
            self.sync(page_width, page_count, resting_index, right_to_left, cx);
            return;
        }
        self.bound = true;
        self.page_width = f32::from(page_width).max(1.0);
        self.page_count = page_count.max(1);
        self.right_to_left = right_to_left;
        self.jump(resting_index, cx);
    }

    pub fn unbind(&mut self, cx: &mut Context<Self>) {
        self.is_dragging = false;
        self.cancel_settle();
        self.publish_activity(cx);
        self.interruption_progress = 0.0;
        self.bound = false;
        cx.notify();
    }

    /// geometry from the next layout pass
    /// a change of page width or tab count must not look like a settle
    /// its a resize or a folder list arriving
    pub fn sync(
        &mut self,
        page_width: Pixels,
        page_count: usize,
        resting_index: usize,
        right_to_left: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.bound {
            self.right_to_left = right_to_left;
            return;
        }

        let new_width = f32::from(page_width).max(1.0);
        let new_count = page_count.max(1);
        let geometry_changed = new_width != self.page_width
            || new_count != self.page_count
            || right_to_left != self.right_to_left;
        self.page_width = new_width;
        self.page_count = new_count;
        self.right_to_left = right_to_left;

        let clamped = self.clamp(resting_index);
        if geometry_changed {
            self.jump(clamped, cx);
        } else if clamped != self.resting_index {
            let animated = clamped.abs_diff(self.resting_index) == 1;
            self.commit(clamped, animated, cx);
        }
    }

    pub fn begin_interactive_drag(&mut self, cx: &mut Context<Self>) {
        if !self.bound {
            self.interruption_progress = 0.0;
            return;
        }
        let now = RiseClock::monotonic();
        self.is_dragging = true;
        self.last_drag_frame = now;
        self.publish_activity(cx);
        let presented = self.presented_visual_index(now);
        self.cancel_settle();
        self.visual_index = presented;
        self.interruption_progress = presented - self.resting_index as f32;
        cx.notify();
    }

    pub fn clear_interaction_residue(&mut self, cx: &mut Context<Self>) {
        self.interruption_progress = 0.0;
        self.is_dragging = false;
        self.publish_activity(cx);
    }

    pub fn set_drag_progress(&mut self, progress: f32, cx: &mut Context<Self>) {
        if !self.bound {
            return;
        }
        self.is_dragging = true;
        self.last_drag_frame = RiseClock::monotonic();
        self.publish_activity(cx);
        self.cancel_settle();
        self.visual_index = self.resting_index as f32 + progress;
        cx.notify();
    }

    /// settles onto a whole page
    /// resting offset moves instantly and the compensating translation runs to zero on the release curve
    pub fn commit(&mut self, index: usize, animated: bool, cx: &mut Context<Self>) {
        self.interruption_progress = 0.0;
        self.commit_position(index, animated, cx);
        self.is_dragging = false;
        self.publish_activity(cx);
    }

    pub fn strip_offset_x(&self, window: &Window) -> Pixels {
        let now = RiseClock::monotonic();
        let translation = if self.settle.is_running(now) {
            self.settle.frame_value(now, window)
        } else {
            self.translation(self.visual_index)
        };
        px(translation - self.resting_slot() as f32 * self.page_width)
    }

    fn commit_position(&mut self, index: usize, animated: bool, cx: &mut Context<Self>) {
        if !self.bound {
            self.resting_index = self.clamp(index);
            self.visual_index = self.resting_index as f32;
            return;
        }

        let target = self.clamp(index);
        if !animated {
            self.jump(target, cx);
            return;
        }

        let now = RiseClock::monotonic();
        let presented_translation = self.is_settling.then(|| self.settle.value(now));

        self.cancel_settle();
        let previous_resting = self.resting_index;
        self.resting_index = target;

        let resting_shift =
            self.direction_sign() * (target as f32 - previous_resting as f32) * self.page_width;
        let from_translation = presented_translation
            .map(|translation| translation + resting_shift)
            .unwrap_or_else(|| self.translation(self.visual_index));
        let to_translation = self.translation(target as f32);
        self.visual_index = target as f32;

        if (from_translation - to_translation).abs() <= 0.5 {
            cx.notify();
            return;
        }

        self.settle.jump(from_translation);
        self.settle.set(
            to_translation,
            AnimatedTabsMetrics::release_animation(),
            now,
            false,
        );

        self.is_settling = true;
        self.settle_generation = self.settle_generation.wrapping_add(1);
        let generation = self.settle_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(duration_from_seconds(AnimatedTabsMetrics::RELEASE_DURATION))
                .await;
            this.update(cx, |driver, cx| {
                if driver.settle_generation != generation {
                    return;
                }
                driver.is_settling = false;
                driver.publish_activity(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn jump(&mut self, index: usize, cx: &mut Context<Self>) {
        self.cancel_settle();
        self.interruption_progress = 0.0;
        self.resting_index = self.clamp(index);
        self.visual_index = self.resting_index as f32;
        self.publish_activity(cx);
        cx.notify();
    }

    /// tells activity readers a moving strip rests now
    /// one comparison on a drag frame, the event goes out once per rest
    fn publish_activity(&mut self, cx: &mut Context<Self>) {
        let moving = self.is_dragging || self.is_settling;
        if moving == self.published_moving {
            return;
        }
        self.published_moving = moving;
        if moving || !self.bound {
            return;
        }
        cx.emit(AnimatedTabsStripDidRest);
    }

    fn presented_visual_index(&self, now: MotionTime) -> f32 {
        if !self.is_settling {
            return self.visual_index;
        }
        let translation_x = self.settle.value(now);
        self.resting_index as f32 - self.direction_sign() * translation_x / self.page_width
    }

    fn resting_slot(&self) -> usize {
        if self.right_to_left {
            self.page_count - 1 - self.resting_index
        } else {
            self.resting_index
        }
    }

    fn translation(&self, index: f32) -> f32 {
        -self.direction_sign() * (index - self.resting_index as f32) * self.page_width
    }

    fn direction_sign(&self) -> f32 {
        if self.right_to_left {
            -1.0
        } else {
            1.0
        }
    }

    fn clamp(&self, index: usize) -> usize {
        index.min(self.page_count.saturating_sub(1))
    }

    fn cancel_settle(&mut self) {
        self.settle_generation = self.settle_generation.wrapping_add(1);
        self.is_settling = false;
        self.settle.jump(0.0);
    }
}
