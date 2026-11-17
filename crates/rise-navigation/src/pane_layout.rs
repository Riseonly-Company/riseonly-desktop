use gpui::Pixels;
use rise_theme::{PanelMetrics, ShellMetrics};

/// which stack a screen lives in, a screen declares its slot never the column count
/// primary is the section own stack, secondary the chat opened beside the list
/// tertiary the right panel
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

/// how many columns the area right of the section bar is cut into
/// a narrow window is the phone, one stack where panel and open chat are pushed screens
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

    /// which stacks a drawn column may take its screen from, topmost first
    /// a section that doesnt split has an empty secondary stack so its second column is the panel
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

/// what the screen on top of the section asks of the frame, only chats splits
/// any section may give the panel a default content
/// the panel exists only while something is there to show
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PaneDemand {
    pub splits: bool,
    pub caps_centre: bool,
    pub has_aside: bool,
    pub aside_open: bool,
    pub centre_width: Option<Pixels>,
}

impl PaneDemand {
    pub fn shows_aside(&self) -> bool {
        self.has_aside && self.aside_open
    }
}

/// widths of the drawn columns left to right for one resolved layout
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PaneColumns {
    pub list: Option<Pixels>,
    pub centre: Pixels,
    pub centre_content: Pixels,
    pub aside: Option<Pixels>,
    pub slide_over: Option<Pixels>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PanePolicy {
    pub sidebar_width: Pixels,
    pub detail_min_width: Pixels,
    pub split_detail_min_width: Pixels,
    pub aside_width: Pixels,
    pub aside_min_width: Pixels,
    pub aside_max_width: Pixels,
    pub feed_max_width: Pixels,
    pub hysteresis: Pixels,
    pub aside_fills_spare: bool,
}

impl Default for PanePolicy {
    fn default() -> Self {
        Self::from_shell_metrics(ShellMetrics::default()).with_panel_metrics(PanelMetrics::default())
    }
}

impl PanePolicy {
    /// derives the policy from the theme column widths, never hard code the thresholds
    pub fn from_shell_metrics(metrics: ShellMetrics) -> Self {
        Self {
            sidebar_width: metrics.sidebar_width,
            detail_min_width: metrics.content_min_width,
            split_detail_min_width: metrics.conversation_min_width.max(metrics.content_min_width),
            aside_width: metrics.aside_width,
            aside_min_width: metrics.aside_width,
            aside_max_width: metrics.aside_width,
            feed_max_width: metrics.feed_max_width,
            hysteresis: metrics.pane_hysteresis,
            aside_fills_spare: true,
        }
    }

    /// the panel width is the persons own now, so it doesnt grow into a reading centre spare room
    pub fn with_chosen_aside(mut self, chosen: bool) -> Self {
        self.aside_fills_spare = !chosen;
        self
    }

    /// list column width the person dragged it to in this section
    pub fn with_list_width(mut self, width: Option<Pixels>) -> Self {
        if let Some(width) = width {
            self.sidebar_width = width;
        }
        self
    }

    /// range the person may drag the panel through
    pub fn with_panel_metrics(mut self, panel: PanelMetrics) -> Self {
        self.aside_min_width = panel.min_width;
        self.aside_max_width = panel.max_width;
        self
    }

    pub fn two_column_threshold(&self) -> Pixels {
        self.sidebar_width + self.detail_min_width
    }

    pub fn three_column_threshold(&self) -> Pixels {
        self.sidebar_width + self.split_detail_min_width + self.aside_min_width
    }

    /// chats section with the panel open, the widest frame the shell draws
    pub fn layout_for(&self, width: Pixels) -> PaneLayout {
        self.layout_for_demand(
            width,
            PaneDemand {
                splits: true,
                caps_centre: false,
                has_aside: true,
                aside_open: true,
                centre_width: None,
            },
        )
    }

    /// width the layout needs for a demand, none when the demand never draws it
    pub fn threshold(&self, layout: PaneLayout, demand: PaneDemand) -> Option<Pixels> {
        match (layout, demand.splits, demand.shows_aside()) {
            (PaneLayout::Stacked, _, _) => Some(Pixels::ZERO),
            (PaneLayout::TwoColumn, true, _) => Some(self.two_column_threshold()),
            (PaneLayout::TwoColumn, false, true) => {
                Some(self.detail_min_width + self.aside_min_width)
            }
            (PaneLayout::TwoColumn, false, false) => None,
            (PaneLayout::ThreeColumn, true, true) => Some(self.three_column_threshold()),
            (PaneLayout::ThreeColumn, _, _) => None,
        }
    }

    /// true when the panel gets its own column at this width once open
    /// otherwise it can only slide in over the centre
    pub fn aside_fits(&self, width: Pixels, demand: PaneDemand) -> bool {
        let open = PaneDemand { aside_open: true, ..demand };
        let layout = if demand.splits { PaneLayout::ThreeColumn } else { PaneLayout::TwoColumn };
        open.shows_aside() && self.threshold(layout, open).is_some_and(|needed| width >= needed)
    }

    pub fn layout_for_demand(&self, width: Pixels, demand: PaneDemand) -> PaneLayout {
        [PaneLayout::ThreeColumn, PaneLayout::TwoColumn]
            .into_iter()
            .find(|layout| {
                self.threshold(*layout, demand)
                    .is_some_and(|needed| width >= needed)
            })
            .unwrap_or(PaneLayout::Stacked)
    }

    /// resolves the layout with hysteresis, growing applies at once and shrinking must clear it
    pub fn layout_for_resize(&self, width: Pixels, current: PaneLayout) -> PaneLayout {
        self.layout_for_resize_demand(
            width,
            PaneDemand {
                splits: true,
                caps_centre: false,
                has_aside: true,
                aside_open: true,
                centre_width: None,
            },
            current,
        )
    }

    /// same as layout_for_resize, but a demand change (panel closed
    /// section switched) isnt a resize so it applies at once
    pub fn layout_for_resize_demand(
        &self,
        width: Pixels,
        demand: PaneDemand,
        current: PaneLayout,
    ) -> PaneLayout {
        let next = self.layout_for_demand(width, demand);
        if next == current || next.column_count() > current.column_count() {
            return next;
        }

        match self.threshold(current, demand) {
            Some(boundary) if width >= boundary - self.hysteresis => current,
            _ => next,
        }
    }

    /// cuts width (the window right of the section bar) into the columns the layout draws
    /// aside_width is the panel width the person last chose
    pub fn columns(
        &self,
        width: Pixels,
        layout: PaneLayout,
        demand: PaneDemand,
        aside_width: Pixels,
    ) -> PaneColumns {
        let width = width.max(Pixels::ZERO);
        let list = (demand.splits && layout != PaneLayout::Stacked).then_some(self.sidebar_width);
        let has_aside_column = match layout {
            PaneLayout::Stacked => false,
            PaneLayout::TwoColumn => !demand.splits && demand.shows_aside(),
            PaneLayout::ThreeColumn => demand.shows_aside(),
        };

        let after_list = (width - list.unwrap_or(Pixels::ZERO)).max(Pixels::ZERO);
        let chosen = aside_width.clamp(self.aside_min_width, self.aside_max_width);
        let aside = has_aside_column.then(|| {
            let wanted = if demand.caps_centre && self.aside_fills_spare {
                chosen.max(after_list - self.centre_cap(demand))
            } else {
                chosen
            };
            let detail_min = if demand.splits { self.split_detail_min_width } else { self.detail_min_width };
            let room = (after_list - detail_min).max(Pixels::ZERO);
            wanted.min(room)
        });

        let centre = (after_list - aside.unwrap_or(Pixels::ZERO)).max(Pixels::ZERO);
        let centre_content = if demand.caps_centre && !demand.splits && aside.is_some() {
            centre.min(self.centre_cap(demand))
        } else {
            centre
        };
        let slide_over = (demand.has_aside && !has_aside_column).then(|| chosen.min(width));

        PaneColumns {
            list,
            centre,
            centre_content,
            aside,
            slide_over,
        }
    }

    /// how wide a capped centre stays beside the panel
    fn centre_cap(&self, demand: PaneDemand) -> Pixels {
        demand.centre_width.unwrap_or(self.feed_max_width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    fn policy() -> PanePolicy {
        PanePolicy::default()
    }

    fn chats(aside_open: bool) -> PaneDemand {
        PaneDemand {
            splits: true,
            caps_centre: false,
            has_aside: true,
            aside_open,
            centre_width: None,
        }
    }

    fn feed(has_aside: bool) -> PaneDemand {
        PaneDemand {
            splits: false,
            caps_centre: true,
            has_aside,
            aside_open: true,
            centre_width: None,
        }
    }

    #[test]
    fn a_chosen_panel_width_holds_beside_a_reading_centre() {
        let p = policy();
        let width = px(1_800.0);
        let filled = p.columns(width, PaneLayout::TwoColumn, feed(true), p.aside_min_width);
        assert!(filled.aside.is_some_and(|aside| aside > p.aside_min_width));

        let chosen = p.with_chosen_aside(true);
        let columns = chosen.columns(width, PaneLayout::TwoColumn, feed(true), p.aside_min_width);
        assert_eq!(columns.aside, Some(p.aside_min_width));
        assert_eq!(columns.centre, width - p.aside_min_width);
        assert_eq!(columns.centre_content, p.feed_max_width, "the feed keeps its reading width");
    }

    #[test]
    fn the_chosen_list_width_cuts_the_list_column() {
        let p = policy().with_list_width(Some(px(300.0)));
        let width = px(1_600.0);
        let columns = p.columns(width, p.layout_for(width), chats(true), p.aside_width);
        assert_eq!(columns.list, Some(px(300.0)));
        assert_eq!(policy().with_list_width(None).sidebar_width, policy().sidebar_width);
    }

    #[test]
    fn a_narrow_window_collapses_to_the_phone_stack() {
        assert_eq!(policy().layout_for(px(600.0)), PaneLayout::Stacked);
        assert_eq!(PaneLayout::Stacked.column_count(), 1);
    }

    #[test]
    fn chats_split_into_list_and_chat_before_the_panel_fits() {
        let p = policy();
        assert_eq!(p.layout_for(p.two_column_threshold()), PaneLayout::TwoColumn);
        assert_eq!(
            p.layout_for(p.three_column_threshold() - px(1.0)),
            PaneLayout::TwoColumn
        );
        assert_eq!(p.layout_for(p.three_column_threshold()), PaneLayout::ThreeColumn);
    }

    #[test]
    fn thresholds_come_from_the_iphone_widths() {
        let p = policy();
        assert_eq!(p.two_column_threshold(), px(750.0));
        assert_eq!(p.three_column_threshold(), px(1175.0));
    }

    #[test]
    fn a_conversation_beside_the_list_and_the_panel_keeps_its_reading_width() {
        let p = policy();
        let width = p.three_column_threshold();
        let columns = p.columns(width, p.layout_for(width), chats(true), p.aside_width);
        assert!(columns.centre >= p.split_detail_min_width);
        assert_eq!(p.layout_for(width - px(1.0)), PaneLayout::TwoColumn);
    }

    #[test]
    fn a_closed_panel_never_takes_a_column() {
        let p = policy();
        assert_eq!(
            p.layout_for_demand(px(2000.0), chats(false)),
            PaneLayout::TwoColumn
        );
        assert_eq!(
            p.layout_for_demand(px(2000.0), PaneDemand { aside_open: false, ..feed(true) }),
            PaneLayout::Stacked
        );
    }

    #[test]
    fn a_screen_without_a_context_has_no_panel() {
        assert_eq!(
            policy().layout_for_demand(px(2000.0), feed(false)),
            PaneLayout::Stacked
        );
    }

    #[test]
    fn a_feed_with_a_context_shows_the_panel_beside_it() {
        let p = policy();
        let needed = p.detail_min_width + p.aside_min_width;
        assert_eq!(p.layout_for_demand(needed, feed(true)), PaneLayout::TwoColumn);
        assert_eq!(
            p.layout_for_demand(needed - px(1.0), feed(true)),
            PaneLayout::Stacked
        );
    }

    #[test]
    fn every_column_a_layout_draws_has_a_stack_to_draw_from() {
        for layout in [PaneLayout::Stacked, PaneLayout::TwoColumn, PaneLayout::ThreeColumn] {
            for column in 0..layout.column_count() {
                assert!(!layout.slots_for_column(column).is_empty());
            }
            assert!(layout.slots_for_column(layout.column_count()).is_empty());
        }
    }

    #[test]
    fn every_slot_stays_reachable_in_every_layout() {
        for layout in [PaneLayout::Stacked, PaneLayout::TwoColumn, PaneLayout::ThreeColumn] {
            for slot in PaneSlot::ALL {
                let reachable = (0..layout.column_count())
                    .any(|column| layout.slots_for_column(column).contains(&slot));
                assert!(
                    reachable,
                    "{slot:?} is unreachable in {layout:?}, so narrowing the window would lose it"
                );
            }
        }
    }

    #[test]
    fn a_narrowed_panel_becomes_a_screen_over_the_chat() {
        assert_eq!(
            PaneLayout::TwoColumn.slots_for_column(1),
            &[PaneSlot::Tertiary, PaneSlot::Secondary]
        );
        assert_eq!(
            PaneLayout::Stacked.slots_for_column(0),
            &[PaneSlot::Tertiary, PaneSlot::Secondary, PaneSlot::Primary]
        );
    }

    #[test]
    fn shrinking_across_a_boundary_holds_until_hysteresis_is_cleared() {
        let p = policy();
        let boundary = p.two_column_threshold();

        assert_eq!(
            p.layout_for_resize(boundary - px(10.0), PaneLayout::TwoColumn),
            PaneLayout::TwoColumn
        );
        assert_eq!(
            p.layout_for_resize(boundary - px(40.0), PaneLayout::TwoColumn),
            PaneLayout::Stacked
        );
    }

    #[test]
    fn growing_across_a_boundary_takes_effect_immediately() {
        let p = policy();
        assert_eq!(
            p.layout_for_resize(p.two_column_threshold(), PaneLayout::Stacked),
            PaneLayout::TwoColumn
        );
    }

    #[test]
    fn closing_the_panel_is_not_held_by_hysteresis() {
        let p = policy();
        assert_eq!(
            p.layout_for_resize_demand(px(2000.0), chats(false), PaneLayout::ThreeColumn),
            PaneLayout::TwoColumn
        );
    }

    #[test]
    fn resize_is_idempotent_when_the_layout_does_not_change() {
        let p = policy();
        let first = p.layout_for_resize(px(1200.0), PaneLayout::ThreeColumn);
        assert_eq!(first, p.layout_for_resize(px(1200.0), first));
    }

    #[test]
    fn a_feed_keeps_its_reading_width_and_the_panel_takes_all_the_rest() {
        let p = policy();
        for width in [px(1100.0), px(2400.0)] {
            let columns = p.columns(width, PaneLayout::TwoColumn, feed(true), p.aside_width);

            assert_eq!(columns.list, None);
            assert_eq!(columns.aside, Some(width - p.feed_max_width));
            assert_eq!(columns.centre, p.feed_max_width);
            assert_eq!(columns.centre_content, columns.centre, "no strip beside the feed");
            assert_eq!(columns.slide_over, None);
        }
    }

    #[test]
    fn a_reading_column_without_a_panel_grows_to_the_whole_centre() {
        let p = policy();
        let columns = p.columns(px(1400.0), PaneLayout::Stacked, feed(false), p.aside_width);

        assert_eq!(columns.centre, px(1400.0));
        assert_eq!(columns.centre_content, px(1400.0));
        assert_eq!(columns.slide_over, None);
    }

    #[test]
    fn a_conversation_beside_the_panel_keeps_the_feed_width() {
        let p = policy();
        let demand = PaneDemand { caps_centre: true, ..chats(true) };
        let columns = p.columns(px(1400.0), PaneLayout::ThreeColumn, demand, p.aside_width);

        assert_eq!(columns.list, Some(p.sidebar_width));
        assert_eq!(columns.centre, p.feed_max_width);
        assert_eq!(columns.aside, Some(px(1400.0) - p.sidebar_width - p.feed_max_width));
    }

    #[test]
    fn a_conversation_held_at_its_minimum_fills_its_column() {
        let p = policy();
        let demand = PaneDemand { caps_centre: true, ..chats(true) };
        let width = p.sidebar_width + p.split_detail_min_width + p.aside_min_width;
        let columns = p.columns(width, PaneLayout::ThreeColumn, demand, p.aside_width);

        assert!(columns.centre >= p.feed_max_width);
        assert_eq!(columns.centre_content, columns.centre, "no strip beside the chat wallpaper");
    }

    #[test]
    fn a_panel_without_a_column_slides_over_the_centre() {
        let p = policy();
        let width = px(600.0);
        let layout = p.layout_for_demand(width, feed(true));
        let columns = p.columns(width, layout, feed(true), p.aside_width);

        assert_eq!(layout, PaneLayout::Stacked);
        assert!(!p.aside_fits(width, feed(true)));
        assert_eq!(columns.aside, None);
        assert_eq!(columns.slide_over, Some(p.aside_width.clamp(p.aside_min_width, p.aside_max_width)));
    }

    #[test]
    fn a_closed_panel_that_fits_reopens_as_a_column() {
        let p = policy();
        let closed = PaneDemand { aside_open: false, ..feed(true) };
        let layout = p.layout_for_demand(px(2000.0), closed);
        let columns = p.columns(px(2000.0), layout, closed, p.aside_width);

        assert!(p.aside_fits(px(2000.0), closed));
        assert_eq!(columns.aside, None);
        assert!(columns.slide_over.is_some());
        assert!(!p.aside_fits(px(2000.0), feed(false)));
    }

    #[test]
    fn the_panel_never_squeezes_the_centre_below_an_iphone() {
        let p = policy();
        let width = p.detail_min_width + p.aside_min_width;
        let columns = p.columns(width, PaneLayout::TwoColumn, feed(true), p.aside_max_width);

        assert_eq!(columns.centre, p.detail_min_width);
        assert_eq!(columns.aside, Some(p.aside_min_width));
    }

    #[test]
    fn split_chats_give_the_list_its_width_and_the_chat_the_rest() {
        let p = policy();
        let columns = p.columns(px(1400.0), PaneLayout::ThreeColumn, chats(true), p.aside_width);

        assert_eq!(columns.list, Some(p.sidebar_width));
        assert_eq!(columns.aside, Some(p.aside_width));
        assert_eq!(columns.centre, px(1400.0) - p.sidebar_width - p.aside_width);
        assert_eq!(columns.centre_content, columns.centre, "a chat is not a feed");
    }

    #[test]
    fn a_reel_keeps_its_own_width_and_the_panel_takes_the_rest() {
        let p = policy();
        let reel = px(780.0);
        let demand = PaneDemand { centre_width: Some(reel), ..feed(true) };
        let columns = p.columns(px(2400.0), PaneLayout::TwoColumn, demand, p.aside_width);

        assert_eq!(columns.centre, reel);
        assert_eq!(columns.centre_content, reel);
        assert_eq!(columns.aside, Some(px(2400.0) - reel));
    }

    #[test]
    fn a_stacked_window_is_one_column_however_the_screen_asks() {
        let p = policy();
        let columns = p.columns(px(600.0), PaneLayout::Stacked, chats(true), p.aside_width);

        assert_eq!(columns.list, None);
        assert_eq!(columns.aside, None);
        assert_eq!(columns.centre, px(600.0));
    }
}
