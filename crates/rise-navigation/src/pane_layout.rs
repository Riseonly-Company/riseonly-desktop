use gpui::Pixels;
use rise_theme::ShellMetrics;

/// Which column a screen asks to live in.
///
/// A screen declares its slot; it never decides how many columns exist. That
/// keeps every module ignorant of window width, which is what lets the shell
/// collapse to a phone-shaped stack without touching a single feature.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaneSlot {
    Primary,
    Secondary,
    Tertiary,
}

impl PaneSlot {
    pub const ALL: [PaneSlot; 3] = [PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary];

    pub fn index(self) -> usize {
        match self {
            Self::Primary => 0,
            Self::Secondary => 1,
            Self::Tertiary => 2,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaneLayout {
    Stacked,
    TwoColumn,
    ThreeColumn,
}

impl PaneLayout {
    pub fn visible_slots(self) -> &'static [PaneSlot] {
        match self {
            Self::Stacked => &[PaneSlot::Primary],
            Self::TwoColumn => &[PaneSlot::Primary, PaneSlot::Secondary],
            Self::ThreeColumn => &[PaneSlot::Primary, PaneSlot::Secondary, PaneSlot::Tertiary],
        }
    }

    pub fn shows(self, slot: PaneSlot) -> bool {
        self.visible_slots().contains(&slot)
    }

    pub fn column_count(self) -> usize {
        self.visible_slots().len()
    }

    /// Which stacks a drawn column may take its screen from, topmost first.
    ///
    /// Narrowing the window must never move a screen into another slot's stack.
    /// If it did, widening the window again could not put it back, and the user
    /// would watch a conversation they opened in the second column reappear in
    /// the sidebar. So the stacks stay put and the COLUMNS merge instead: a
    /// two-column window draws the aside's screen in the content column when the
    /// aside has one, and a collapsed window draws whichever stack is furthest
    /// right. Every projection here is reversible, which is what makes a resize
    /// from 400px to 2560px and back lossless.
    pub fn slots_for_column(self, column: usize) -> &'static [PaneSlot] {
        match (self, column) {
            (Self::ThreeColumn, 0) => &[PaneSlot::Primary],
            (Self::ThreeColumn, 1) => &[PaneSlot::Secondary],
            (Self::ThreeColumn, 2) => &[PaneSlot::Tertiary],
            (Self::TwoColumn, 0) => &[PaneSlot::Primary],
            (Self::TwoColumn, 1) => &[PaneSlot::Tertiary, PaneSlot::Secondary],
            (Self::Stacked, 0) => &[PaneSlot::Tertiary, PaneSlot::Secondary, PaneSlot::Primary],
            _ => &[],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PanePolicy {
    pub sidebar_width: Pixels,
    pub detail_min_width: Pixels,
    pub aside_width: Pixels,
    pub hysteresis: Pixels,
}

impl Default for PanePolicy {
    fn default() -> Self {
        Self::from_shell_metrics(ShellMetrics::default())
    }
}

impl PanePolicy {
    /// The column widths are design tokens and live in `rise-theme`, so density
    /// reaches them. Repeating the numbers here would let a comfortable-density
    /// window compute its thresholds from compact-density columns.
    pub fn from_shell_metrics(metrics: ShellMetrics) -> Self {
        Self {
            sidebar_width: metrics.sidebar_width,
            detail_min_width: metrics.content_min_width,
            aside_width: metrics.aside_width,
            hysteresis: metrics.pane_hysteresis,
        }
    }

    pub fn two_column_threshold(&self) -> Pixels {
        self.sidebar_width + self.detail_min_width
    }

    pub fn three_column_threshold(&self) -> Pixels {
        self.two_column_threshold() + self.aside_width
    }

    pub fn layout_for(&self, width: Pixels) -> PaneLayout {
        if width >= self.three_column_threshold() {
            PaneLayout::ThreeColumn
        } else if width >= self.two_column_threshold() {
            PaneLayout::TwoColumn
        } else {
            PaneLayout::Stacked
        }
    }

    /// Resolves the layout while resisting a flip that a few pixels of drag
    /// would otherwise cause. A window resized across a boundary should settle,
    /// not strobe between one and two columns.
    pub fn layout_for_resize(&self, width: Pixels, current: PaneLayout) -> PaneLayout {
        let next = self.layout_for(width);
        if next == current {
            return current;
        }

        let boundary = match (current, next) {
            (PaneLayout::TwoColumn, PaneLayout::Stacked) => self.two_column_threshold(),
            (PaneLayout::ThreeColumn, PaneLayout::TwoColumn) => self.three_column_threshold(),
            _ => return next,
        };

        if width >= boundary - self.hysteresis {
            current
        } else {
            next
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    fn policy() -> PanePolicy {
        PanePolicy::default()
    }

    #[test]
    fn a_narrow_window_collapses_to_a_single_stack() {
        assert_eq!(policy().layout_for(px(600.0)), PaneLayout::Stacked);
        assert_eq!(PaneLayout::Stacked.column_count(), 1);
    }

    #[test]
    fn a_medium_window_shows_list_and_content() {
        let p = policy();
        assert_eq!(
            p.layout_for(p.two_column_threshold()),
            PaneLayout::TwoColumn
        );
        assert_eq!(p.layout_for(px(1000.0)), PaneLayout::TwoColumn);
    }

    #[test]
    fn a_wide_window_shows_the_aside_as_well() {
        let p = policy();
        assert_eq!(
            p.layout_for(p.three_column_threshold()),
            PaneLayout::ThreeColumn
        );
        assert_eq!(p.layout_for(px(1600.0)), PaneLayout::ThreeColumn);
    }

    #[test]
    fn thresholds_are_ordered_and_derived_from_column_widths() {
        let p = policy();
        assert!(p.two_column_threshold() < p.three_column_threshold());
        assert_eq!(p.two_column_threshold(), px(780.0));
        assert_eq!(p.three_column_threshold(), px(1080.0));
    }

    #[test]
    fn every_column_a_layout_draws_has_a_stack_to_draw_from() {
        for layout in [
            PaneLayout::Stacked,
            PaneLayout::TwoColumn,
            PaneLayout::ThreeColumn,
        ] {
            for column in 0..layout.column_count() {
                assert!(
                    !layout.slots_for_column(column).is_empty(),
                    "{layout:?} column {column} has nowhere to take a screen from, so it would \
                     render empty"
                );
            }
            assert!(
                layout.slots_for_column(layout.column_count()).is_empty(),
                "{layout:?} claims a column it does not draw"
            );
        }
    }

    #[test]
    fn every_slot_stays_reachable_in_every_layout() {
        for layout in [
            PaneLayout::Stacked,
            PaneLayout::TwoColumn,
            PaneLayout::ThreeColumn,
        ] {
            for slot in PaneSlot::ALL {
                let reachable = (0..layout.column_count())
                    .any(|column| layout.slots_for_column(column).contains(&slot));
                assert!(
                    reachable,
                    "{slot:?} is unreachable in {layout:?}, so narrowing the window would lose \
                     whatever the user had open there"
                );
            }
        }
    }

    #[test]
    fn a_merged_column_prefers_the_screen_that_was_on_top() {
        assert_eq!(
            PaneLayout::TwoColumn.slots_for_column(1),
            &[PaneSlot::Tertiary, PaneSlot::Secondary],
            "the aside sits above the conversation, so it wins the merged column"
        );
        assert_eq!(
            PaneLayout::Stacked.slots_for_column(0),
            &[PaneSlot::Tertiary, PaneSlot::Secondary, PaneSlot::Primary]
        );
    }

    #[test]
    fn only_the_primary_slot_renders_when_stacked() {
        assert!(PaneLayout::Stacked.shows(PaneSlot::Primary));
        assert!(!PaneLayout::Stacked.shows(PaneSlot::Secondary));
        assert!(!PaneLayout::Stacked.shows(PaneSlot::Tertiary));
    }

    #[test]
    fn shrinking_across_a_boundary_holds_until_hysteresis_is_cleared() {
        let p = policy();
        let boundary = p.two_column_threshold();

        let just_inside = boundary - px(10.0);
        assert_eq!(
            p.layout_for_resize(just_inside, PaneLayout::TwoColumn),
            PaneLayout::TwoColumn,
            "a few pixels of drag must not flip the layout"
        );

        let clearly_past = boundary - px(40.0);
        assert_eq!(
            p.layout_for_resize(clearly_past, PaneLayout::TwoColumn),
            PaneLayout::Stacked
        );
    }

    #[test]
    fn growing_across_a_boundary_takes_effect_immediately() {
        let p = policy();
        assert_eq!(
            p.layout_for_resize(p.two_column_threshold(), PaneLayout::Stacked),
            PaneLayout::TwoColumn,
            "gaining room should never be delayed"
        );
    }

    #[test]
    fn resize_is_idempotent_when_the_layout_does_not_change() {
        let p = policy();
        let width = px(1200.0);
        let first = p.layout_for_resize(width, PaneLayout::ThreeColumn);
        let second = p.layout_for_resize(width, first);
        assert_eq!(first, second);
    }
}
