use std::ops::RangeInclusive;

/// strip geometry in its own space, viewport x starts at the leading edge of the visible window
/// content x is viewport x plus scroll
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabStripLayout {
    pub viewport_width: f32,
    pub count: usize,
    pub visible_slots: f32,
}

impl TabStripLayout {
    /// peek 0 means the default
    pub fn new(viewport_width: f32, count: usize, visible_count: usize, peek: f32) -> Self {
        let shown = visible_count.max(1).min(count.max(1)) as f32;
        let visible_slots = if count > visible_count { shown + peek.max(0.0) } else { shown };
        Self {
            viewport_width: viewport_width.max(0.0),
            count,
            visible_slots,
        }
    }

    pub fn slot_width(&self) -> f32 {
        if self.count == 0 || self.visible_slots <= 0.0 {
            return 0.0;
        }
        self.viewport_width / self.visible_slots
    }

    pub fn content_width(&self) -> f32 {
        self.slot_width() * self.count as f32
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content_width() - self.viewport_width).max(0.0)
    }

    pub fn is_scrollable(&self) -> bool {
        self.max_scroll() > 0.5
    }

    pub fn center(&self, index: usize) -> f32 {
        (index as f32 + 0.5) * self.slot_width()
    }

    pub fn index(&self, at_content_x: f32) -> usize {
        let slot_width = self.slot_width();
        if self.count == 0 || slot_width <= 0.0 {
            return 0;
        }
        let raw = (at_content_x / slot_width).floor() as isize;
        raw.clamp(0, self.count as isize - 1) as usize
    }

    pub fn clamp_scroll(&self, scroll: f32) -> f32 {
        scroll.max(0.0).min(self.max_scroll())
    }

    /// scroll that puts the button in the middle of the bar, as far as the ends allow
    pub fn centered_scroll(&self, index: usize) -> f32 {
        self.clamp_scroll(self.center(index) - self.viewport_width / 2.0)
    }

    /// scrolls only when the button or its outer neighbour would be cut off
    /// buttons already on screen keep their places
    pub fn revealing_scroll(&self, index: usize, from_current: f32) -> f32 {
        let slot_width = self.slot_width();
        if slot_width <= 0.0 {
            return 0.0;
        }
        let mut scroll = self.clamp_scroll(from_current);
        let leading = index.saturating_sub(1) as f32 * slot_width;
        let trailing = (index + 2).min(self.count) as f32 * slot_width;
        if leading < scroll {
            scroll = leading;
        }
        if trailing > scroll + self.viewport_width {
            scroll = trailing - self.viewport_width;
        }
        self.clamp_scroll(scroll)
    }

    /// where the lens center may rest, first visible slot to the last one
    pub fn lens_range(&self) -> RangeInclusive<f32> {
        let half = self.slot_width() / 2.0;
        let upper = half.max(self.viewport_width.min(self.content_width()) - half);
        half..=upper
    }

    /// one lap of the looping strip, after the last button the first comes again
    pub fn period(&self) -> f32 {
        self.content_width()
    }

    /// slot under a content x when the strip loops
    /// any integer, read modulo count
    pub fn slot(&self, at_content_x: f32) -> isize {
        let slot_width = self.slot_width();
        if slot_width <= 0.0 {
            return 0;
        }
        (at_content_x / slot_width).floor() as isize
    }

    /// button a slot of the looping strip shows
    pub fn wrapped(&self, slot: isize) -> usize {
        if self.count == 0 {
            return 0;
        }
        slot.rem_euclid(self.count as isize) as usize
    }

    /// copy of button index closest to the slot
    pub fn nearest_slot(&self, index: usize, around_slot: isize) -> isize {
        let index = index as isize;
        if self.count == 0 {
            return index;
        }
        let count = self.count as isize;
        let laps = ((around_slot - index) as f64 / count as f64).round() as isize;
        index + laps * count
    }

    /// revealing_scroll without ends
    pub fn looping_revealing_scroll(&self, slot: isize, from_current: f32) -> f32 {
        let slot_width = self.slot_width();
        let mut scroll = from_current;
        let leading = (slot - 1) as f32 * slot_width;
        let trailing = (slot + 2) as f32 * slot_width;
        if leading < scroll {
            scroll = leading;
        }
        if trailing > scroll + self.viewport_width {
            scroll = trailing - self.viewport_width;
        }
        scroll
    }
}

/// how the lens pushes the strip
/// past threshold of the viewport in the travel direction each point of finger scrolls gain points
/// push eases in over ramp points so the strip doesnt jump
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LensScrollDrive {
    pub threshold: f32,
    pub gain: f32,
    pub ramp: f32,
}

impl LensScrollDrive {
    pub fn forward_line(&self, layout: &TabStripLayout) -> f32 {
        layout.viewport_width * self.threshold
    }

    pub fn backward_line(&self, layout: &TabStripLayout) -> f32 {
        layout.viewport_width * (1.0 - self.threshold)
    }

    fn pushed(&self, distance: f32) -> f32 {
        if distance <= 0.0 {
            return 0.0;
        }
        if self.ramp <= 0.0 {
            return distance * self.gain;
        }
        if distance < self.ramp {
            return self.gain * distance * distance / (2.0 * self.ramp);
        }
        self.gain * (distance - self.ramp / 2.0)
    }

    pub fn scroll_delta(&self, from_old: f32, to_new: f32, layout: &TabStripLayout) -> f32 {
        if to_new > from_old {
            let line = self.forward_line(layout);
            return self.pushed(to_new - line) - self.pushed(from_old - line);
        }
        if to_new < from_old {
            let line = self.backward_line(layout);
            return self.pushed(line - from_old) - self.pushed(line - to_new);
        }
        0.0
    }

    /// gain at which the whole strip has scrolled when the lens is a quarter slot past the last slot
    /// a finger aiming at the end overshoots about that much
    pub fn auto_gain(layout: &TabStripLayout, threshold: f32, ramp: f32) -> f32 {
        let end = *layout.lens_range().end() + layout.slot_width() * 0.25;
        let travel = end - layout.viewport_width * threshold - ramp / 2.0;
        if !layout.is_scrollable() || travel <= 1.0 {
            return 1.0;
        }
        (layout.max_scroll() / travel).max(1.0).min(4.0)
    }
}

/// one press and slide on the bar
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LensDrag {
    pub lens_x: f32,
    pub scroll: f32,
}

impl LensDrag {
    /// loops means the strip has no ends so the push is never clamped
    pub fn r#move(&mut self, dx: f32, layout: &TabStripLayout, drive: &LensScrollDrive, loops: bool) {
        let old = self.lens_x;
        self.lens_x += dx;
        if !layout.is_scrollable() {
            return;
        }
        let next = self.scroll + drive.scroll_delta(old, self.lens_x, layout);
        self.scroll = if loops { next } else { layout.clamp_scroll(next) };
    }

    pub fn displayed_x(&self, layout: &TabStripLayout) -> f32 {
        rubber_banded(self.lens_x, layout.lens_range(), layout.slot_width() * 0.4)
    }

    pub fn hovered_index(&self, layout: &TabStripLayout) -> usize {
        layout.index(self.displayed_x(layout) + self.scroll)
    }

    pub fn hovered_slot(&self, layout: &TabStripLayout) -> isize {
        layout.slot(self.displayed_x(layout) + self.scroll)
    }
}

/// scroll view style resistance past the ends of a range
pub fn rubber_banded(value: f32, range: RangeInclusive<f32>, dimension: f32) -> f32 {
    let (lower, upper) = (*range.start(), *range.end());
    if dimension <= 0.0 {
        return value.max(lower).min(upper);
    }
    let resisted = |overshoot: f32| (1.0 - 1.0 / (overshoot * 0.55 / dimension + 1.0)) * dimension;
    if value < lower {
        return lower - resisted(lower - value);
    }
    if value > upper {
        return upper + resisted(value - upper);
    }
    value
}
