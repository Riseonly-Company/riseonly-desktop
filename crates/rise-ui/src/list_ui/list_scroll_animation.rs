use gpui::{App, ListOffset, ListState, Pixels, Window, px};
use rise_core::foundation::foundation_date::RiseClock;

use crate::swiftui::animated::{Animated, MotionTime};
use crate::swiftui::motion::MotionSpec;

const SAME_OFFSET_TOLERANCE: Pixels = px(0.5);

struct ListScrollMotion {
    position: Animated<f32>,
    target: ListOffset,
    item_count: usize,
    last_set: ListOffset,
}

/// one animated scroll of a list at a time, held next to the ListState
/// frame runs in every render of the view that draws the list before the list lays out
#[derive(Default)]
pub struct ListScrollAnimation {
    motion: Option<ListScrollMotion>,
    row_height: Pixels,
}

impl ListScrollAnimation {
    /// starts moving the list to target from where it is now
    /// a running motion is replaced and reduce motion lands at once, the caller notifies the view
    pub fn scroll_to(&mut self, list: &ListState, target: ListOffset, spec: MotionSpec, cx: &App) {
        self.start(list, target, spec, RiseClock::monotonic(), cx.reduce_motion());
    }

    /// advances the list for this frame, asks for the next one while the motion runs
    pub fn frame(&mut self, list: &ListState, window: &Window) {
        if self.tick(list, RiseClock::monotonic()) {
            window.request_animation_frame();
        }
    }

    /// true while a motion is in progress
    pub fn is_running(&self) -> bool {
        self.motion.is_some()
    }

    pub(crate) fn start(&mut self, list: &ListState, target: ListOffset, spec: MotionSpec, now: MotionTime, reduce_motion: bool) {
        let current = list.logical_scroll_top();
        let from = self.position_of(list, current);
        let to = self.position_of(list, target);
        list.scroll_to(current);

        let mut position = Animated::new(from);
        position.set(to, spec, now, reduce_motion);
        if !position.is_running(now) {
            self.motion = None;
            list.scroll_to(target);
            return;
        }
        self.motion = Some(ListScrollMotion { position, target, item_count: list.item_count(), last_set: current });
    }

    /// returns whether another frame is needed
    pub(crate) fn tick(&mut self, list: &ListState, now: MotionTime) -> bool {
        let Some(motion) = self.motion.as_mut() else {
            return false;
        };
        if list.item_count() != motion.item_count || !is_same_offset(list.logical_scroll_top(), motion.last_set) {
            self.motion = None;
            return false;
        }
        if !motion.position.is_running(now) {
            list.scroll_to(motion.target);
            self.motion = None;
            return false;
        }

        let position = motion.position.value(now).max(0.0);
        let item_ix = position.floor() as usize;
        let height = known_row_height(list, item_ix, &mut self.row_height);
        let offset = ListOffset { item_ix, offset_in_item: height * position.fract() };
        list.scroll_to(offset);
        motion.last_set = offset;
        true
    }

    fn position_of(&mut self, list: &ListState, offset: ListOffset) -> f32 {
        let height = known_row_height(list, offset.item_ix, &mut self.row_height);
        let share = if height > Pixels::ZERO { offset.offset_in_item / height } else { 0.0 };
        offset.item_ix as f32 + share
    }
}

/// leaves the list with item_ix at the top, bounds_for_item says nothing for a row above it
fn known_row_height(list: &ListState, item_ix: usize, row_height: &mut Pixels) -> Pixels {
    list.scroll_to(ListOffset { item_ix, offset_in_item: Pixels::ZERO });
    if let Some(bounds) = list.bounds_for_item(item_ix)
        && bounds.size.height > Pixels::ZERO
    {
        *row_height = bounds.size.height;
    }
    *row_height
}

fn is_same_offset(a: ListOffset, b: ListOffset) -> bool {
    a.item_ix == b.item_ix && (a.offset_in_item - b.offset_in_item).abs() < SAME_OFFSET_TOLERANCE
}
