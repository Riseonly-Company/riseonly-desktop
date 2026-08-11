use smallvec::SmallVec;

use crate::pane_layout::{PaneLayout, PanePolicy, PaneSlot};
use crate::route::{RootRoute, RootTab};

type Stack = SmallVec<[RootRoute; 4]>;

// Per section; the oldest entry is dropped once it is full.
const FORWARD_LIMIT: usize = 16;

#[derive(Clone, Default)]
struct SectionStacks {
    stacks: [Stack; 3],
    // When each stack was last pushed to: recency, not slot order, wins a merged column.
    touched: [u64; 3],
    forward: SmallVec<[(PaneSlot, RootRoute); 4]>,
}

/// Per-section navigation stacks, one per pane slot, plus the column layout.
///
/// Three guarantees: the primary stack is never empty (its bottom is the section
/// itself), a screen never changes stacks (narrowing merges COLUMNS), and each
/// section keeps its own stacks across a section switch.
pub struct NavigationStore {
    policy: PanePolicy,
    layout: PaneLayout,
    tab: RootTab,
    sections: [SectionStacks; RootTab::ALL.len()],
    clock: u64,
}

impl NavigationStore {
    pub fn new(policy: PanePolicy, layout: PaneLayout) -> Self {
        let mut store = Self {
            policy,
            layout,
            tab: RootTab::Feed,
            sections: Default::default(),
            clock: 0,
        };
        store.seed_section_root();
        store
    }

    pub fn layout(&self) -> PaneLayout {
        self.layout
    }

    pub fn tab(&self) -> RootTab {
        self.tab
    }

    /// Reselecting the section you are already in returns its PRIMARY column to
    /// its root and leaves the other columns alone.
    pub fn select_tab(&mut self, tab: RootTab) {
        if self.tab == tab {
            let section = &mut self.sections[tab.index()];
            section.stacks[PaneSlot::Primary.index()].truncate(1);
            section.forward.clear();
            return;
        }

        self.tab = tab;
        self.seed_section_root();
    }

    /// Returns whether the column layout actually changed. A resize that stays
    /// inside a threshold returns `false`, so a drag does not report per-pixel.
    pub fn window_resized(&mut self, width: gpui::Pixels) -> bool {
        let next = self.policy.layout_for_resize(width, self.layout);
        if next == self.layout {
            return false;
        }

        self.layout = next;
        true
    }

    pub fn push(&mut self, route: RootRoute) {
        if let RootRoute::Tab(tab) = route {
            self.select_tab(tab);
            return;
        }

        let slot = route.slot();
        if self.stack(slot).last() == Some(&route) {
            self.touch(slot);
            return;
        }

        self.section_mut().forward.clear();
        self.stack_mut(slot).push(route);
        self.touch(slot);
    }

    /// Closes the topmost screen and remembers it, so `forward` can restore it.
    /// The bottom of the primary stack is the section and is never closable.
    pub fn back(&mut self) -> Option<RootRoute> {
        let slot = self.most_recent_occupied_slot()?;
        let route = self.stack_mut(slot).pop()?;

        let section = self.section_mut();
        if section.forward.len() == FORWARD_LIMIT {
            section.forward.remove(0);
        }
        section.forward.push((slot, route.clone()));

        Some(route)
    }

    pub fn forward(&mut self) -> Option<RootRoute> {
        let (slot, route) = self.section_mut().forward.pop()?;
        self.stack_mut(slot).push(route.clone());
        self.touch(slot);
        Some(route)
    }

    pub fn can_go_back(&self) -> bool {
        self.most_recent_occupied_slot().is_some()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.section().forward.is_empty()
    }

    pub fn top(&self, slot: PaneSlot) -> Option<&RootRoute> {
        self.stack(slot).last()
    }

    /// The screen drawn in a given column, and which stack it came from. When a
    /// column merges several stacks, the one touched last wins.
    pub fn column(&self, column: usize) -> Option<(PaneSlot, &RootRoute)> {
        let section = self.section();
        self.layout
            .slots_for_column(column)
            .iter()
            .filter(|slot| !section.stacks[slot.index()].is_empty())
            .max_by_key(|slot| section.touched[slot.index()])
            .and_then(|slot| self.stack(*slot).last().map(|route| (*slot, route)))
    }

    /// Every screen currently drawn, leftmost column first. Revalidation, cache
    /// eviction and push suppression key off this set, never a single screen.
    pub fn active_panes(&self) -> SmallVec<[(PaneSlot, &RootRoute); 3]> {
        (0..self.layout.column_count())
            .filter_map(|column| self.column(column))
            .collect()
    }

    pub fn active_routes(&self) -> SmallVec<[&RootRoute; 3]> {
        self.active_panes().into_iter().map(|(_, r)| r).collect()
    }

    /// The drawn screen the user reached for last — by recency, NOT the
    /// rightmost column.
    pub fn focused_route(&self) -> Option<&RootRoute> {
        let section = self.section();
        self.active_panes()
            .into_iter()
            .max_by_key(|(slot, _)| (section.touched[slot.index()], slot.index()))
            .map(|(_, route)| route)
    }

    /// Every route the store holds, in every section, not only the drawn ones.
    /// Keep live views for this set: a hidden column is not a closed screen.
    pub fn all_routes(&self) -> Vec<&RootRoute> {
        self.sections
            .iter()
            .flat_map(|section| section.stacks.iter())
            .flatten()
            .collect()
    }

    /// Unwinds EVERY section back to its root and drops the history. This is the
    /// account transition, not a navigation.
    pub fn reset_to_root(&mut self) {
        for section in &mut self.sections {
            section.stacks[PaneSlot::Primary.index()].truncate(1);
            section.stacks[PaneSlot::Secondary.index()].clear();
            section.stacks[PaneSlot::Tertiary.index()].clear();
            section.forward.clear();
        }
    }

    fn seed_section_root(&mut self) {
        let root = RootRoute::Tab(self.tab);
        let primary = &mut self.sections[self.tab.index()].stacks[PaneSlot::Primary.index()];
        if primary.is_empty() {
            primary.push(root);
        }
    }

    fn touch(&mut self, slot: PaneSlot) {
        self.clock += 1;
        let clock = self.clock;
        self.section_mut().touched[slot.index()] = clock;
    }

    // The stack `back` unwinds: most recently touched with something closable,
    // falling back to the rightmost.
    fn most_recent_occupied_slot(&self) -> Option<PaneSlot> {
        let section = self.section();
        PaneSlot::ALL
            .iter()
            .copied()
            .filter(|slot| {
                let floor = usize::from(*slot == PaneSlot::Primary);
                section.stacks[slot.index()].len() > floor
            })
            .max_by_key(|slot| (section.touched[slot.index()], slot.index()))
    }

    fn section(&self) -> &SectionStacks {
        &self.sections[self.tab.index()]
    }

    fn section_mut(&mut self) -> &mut SectionStacks {
        &mut self.sections[self.tab.index()]
    }

    fn stack(&self, slot: PaneSlot) -> &Stack {
        &self.section().stacks[slot.index()]
    }

    fn stack_mut(&mut self, slot: PaneSlot) -> &mut Stack {
        &mut self.sections[self.tab.index()].stacks[slot.index()]
    }
}

#[cfg(test)]
mod tests {
    // `sections` is indexed by `RootTab::index()`, so its length must track `ALL`.
    #[test]
    fn every_section_has_a_stack_of_its_own() {
        let store = NavigationStore::new(
            PanePolicy::from_shell_metrics(rise_theme::ShellMetrics::default()),
            PaneLayout::Stacked,
        );
        assert_eq!(store.sections.len(), RootTab::ALL.len());

        for tab in RootTab::ALL {
            assert!(
                tab.index() < store.sections.len(),
                "{tab:?} indexes past the end of the section array"
            );
        }
    }

    #[test]
    fn every_stack_is_listed_including_the_columns_a_narrow_window_hides() {
        let mut store = store(PaneLayout::ThreeColumn);
        store.push(RootRoute::Chat {
            chat_id: "c1".into(),
        });
        store.push(RootRoute::UserProfile {
            user_id: "u1".into(),
        });
        store.window_resized(gpui::px(400.0));

        let keys: Vec<String> = store
            .all_routes()
            .iter()
            .map(|r| r.resource_key())
            .collect();
        assert!(keys.contains(&"Chat:c1".to_owned()));
        assert!(
            keys.contains(&"UserProfile:u1".to_owned()),
            "a hidden column is not a closed screen"
        );
    }

    #[test]
    fn resetting_leaves_nothing_of_the_previous_account_behind() {
        let mut store = store(PaneLayout::ThreeColumn);
        store.push(RootRoute::Chat {
            chat_id: "c1".into(),
        });
        store.select_tab(RootTab::Search);
        store.push(RootRoute::Post {
            post_id: "p1".into(),
        });

        store.reset_to_root();

        assert!(
            store
                .all_routes()
                .iter()
                .all(|route| matches!(route, RootRoute::Tab(_))),
            "a section root is all that may survive; everything else was the previous account's"
        );
        assert!(!store.can_go_back());
        assert!(!store.can_go_forward());
        for tab in RootTab::ALL {
            store.select_tab(tab);
            assert_eq!(store.active_routes().as_slice(), &[&RootRoute::Tab(tab)]);
        }
    }

    use super::*;
    use gpui::px;

    fn chat(id: &str) -> RootRoute {
        RootRoute::Chat { chat_id: id.into() }
    }

    fn profile(id: &str) -> RootRoute {
        RootRoute::UserProfile { user_id: id.into() }
    }

    fn store(layout: PaneLayout) -> NavigationStore {
        NavigationStore::new(PanePolicy::default(), layout)
    }

    fn ids(nav: &NavigationStore) -> Vec<String> {
        nav.active_routes()
            .iter()
            .map(|route| route.resource_key())
            .collect()
    }

    #[test]
    fn the_sidebar_column_is_never_empty() {
        for layout in [
            PaneLayout::Stacked,
            PaneLayout::TwoColumn,
            PaneLayout::ThreeColumn,
        ] {
            let nav = store(layout);
            assert_eq!(
                nav.top(PaneSlot::Primary),
                Some(&RootRoute::Tab(RootTab::Feed))
            );
            assert!(!nav.active_routes().is_empty(), "{layout:?} draws nothing");
        }
    }

    #[test]
    fn a_wide_window_puts_a_conversation_beside_the_list() {
        let mut nav = store(PaneLayout::TwoColumn);
        nav.push(chat("c1"));
        assert_eq!(nav.top(PaneSlot::Secondary), Some(&chat("c1")));
        assert_eq!(ids(&nav), vec!["Feed", "Chat:c1"]);
    }

    #[test]
    fn a_stacked_window_draws_only_the_screen_on_top() {
        let mut nav = store(PaneLayout::Stacked);
        nav.push(chat("c1"));
        assert_eq!(ids(&nav), vec!["Chat:c1"]);
        assert_eq!(
            nav.top(PaneSlot::Secondary),
            Some(&chat("c1")),
            "a collapsed window hides the conversation's column, it does not move it"
        );
    }

    #[test]
    fn collapsing_and_reopening_the_window_restores_every_column() {
        let mut nav = store(PaneLayout::ThreeColumn);
        nav.push(chat("c1"));
        nav.push(profile("u1"));
        let wide = ids(&nav);
        assert_eq!(wide, vec!["Feed", "Chat:c1", "UserProfile:u1"]);

        assert!(nav.window_resized(px(400.0)));
        assert_eq!(nav.layout(), PaneLayout::Stacked);
        assert_eq!(
            ids(&nav),
            vec!["UserProfile:u1"],
            "the focused screen must survive the collapse"
        );

        assert!(nav.window_resized(px(2560.0)));
        assert_eq!(nav.layout(), PaneLayout::ThreeColumn);
        assert_eq!(ids(&nav), wide, "widening must put every screen back");
    }

    #[test]
    fn a_merged_column_shows_whatever_was_opened_last() {
        let mut nav = store(PaneLayout::TwoColumn);
        nav.push(profile("u1"));
        assert_eq!(ids(&nav), vec!["Feed", "UserProfile:u1"]);

        nav.push(chat("c1"));
        assert_eq!(
            ids(&nav),
            vec!["Feed", "Chat:c1"],
            "opening a conversation while a profile is open must show the conversation"
        );

        nav.push(profile("u2"));
        assert_eq!(ids(&nav), vec!["Feed", "UserProfile:u2"]);
    }

    #[test]
    fn the_focused_screen_is_the_one_reached_for_last_not_the_rightmost() {
        let mut nav = store(PaneLayout::ThreeColumn);
        nav.push(chat("c1"));
        nav.push(profile("u1"));
        assert_eq!(
            nav.focused_route().map(RootRoute::resource_key),
            Some("UserProfile:u1".to_owned())
        );

        nav.push(chat("c2"));
        assert_eq!(
            nav.focused_route().map(RootRoute::resource_key),
            Some("Chat:c2".to_owned()),
            "the aside is still further right, and it is not what the user is in"
        );
    }

    #[test]
    fn a_collapsed_window_shows_whatever_was_opened_last() {
        let mut nav = store(PaneLayout::Stacked);
        nav.push(profile("u1"));
        nav.push(chat("c1"));
        assert_eq!(ids(&nav), vec!["Chat:c1"]);
    }

    #[test]
    fn back_closes_the_screen_the_user_is_looking_at() {
        let mut nav = store(PaneLayout::TwoColumn);
        nav.push(profile("u1"));
        nav.push(chat("c1"));

        assert_eq!(
            nav.back(),
            Some(chat("c1")),
            "the conversation was on top, so it is what closes"
        );
        assert_eq!(ids(&nav), vec!["Feed", "UserProfile:u1"]);
        assert_eq!(nav.back(), Some(profile("u1")));
    }

    #[test]
    fn forward_puts_the_screen_back_in_front_of_what_covered_it() {
        let mut nav = store(PaneLayout::TwoColumn);
        nav.push(profile("u1"));
        nav.push(chat("c1"));
        nav.back();
        assert_eq!(ids(&nav), vec!["Feed", "UserProfile:u1"]);

        assert_eq!(nav.forward(), Some(chat("c1")));
        assert_eq!(ids(&nav), vec!["Feed", "Chat:c1"]);
    }

    #[test]
    fn a_two_column_window_draws_the_aside_over_the_conversation() {
        let mut nav = store(PaneLayout::ThreeColumn);
        nav.push(chat("c1"));
        nav.push(profile("u1"));

        nav.window_resized(px(900.0));
        assert_eq!(nav.layout(), PaneLayout::TwoColumn);
        assert_eq!(ids(&nav), vec!["Feed", "UserProfile:u1"]);

        nav.back();
        assert_eq!(
            ids(&nav),
            vec!["Feed", "Chat:c1"],
            "closing the aside uncovers the conversation that was under it"
        );
    }

    #[test]
    fn with_every_stack_occupied_no_width_leaves_a_drawn_column_empty() {
        let mut nav = store(PaneLayout::Stacked);
        nav.push(chat("c1"));
        nav.push(profile("u1"));

        for width in (400..=2560).step_by(7) {
            nav.window_resized(px(width as f32));
            let drawn = nav.active_panes().len();
            assert_eq!(
                drawn,
                nav.layout().column_count(),
                "at {width}px {:?} draws {drawn} of {} columns",
                nav.layout(),
                nav.layout().column_count()
            );
            assert!(
                nav.column(0).is_some(),
                "at {width}px the sidebar column has nothing to draw"
            );
        }
    }

    #[test]
    fn a_section_with_nothing_open_beside_it_draws_fewer_columns_than_the_layout_affords() {
        let mut nav = store(PaneLayout::ThreeColumn);
        assert_eq!(nav.active_panes().len(), 1);
        assert_eq!(nav.column(1), None, "the shell renders its own empty state");

        nav.push(chat("c1"));
        assert_eq!(nav.active_panes().len(), 2);
        assert_eq!(nav.column(2), None);
    }

    #[test]
    fn a_resize_that_does_not_cross_a_threshold_reports_no_change() {
        let mut nav = store(PaneLayout::TwoColumn);
        assert!(!nav.window_resized(px(1000.0)));
        assert_eq!(nav.layout(), PaneLayout::TwoColumn);
    }

    #[test]
    fn active_routes_is_a_set_on_desktop_not_a_single_screen() {
        let mut nav = store(PaneLayout::ThreeColumn);
        nav.push(chat("c1"));
        nav.push(RootRoute::Settings);

        let screens: Vec<_> = nav.active_routes().iter().map(|r| r.screen_id()).collect();
        assert_eq!(screens, vec!["Feed", "Chat", "Settings"]);
        assert_eq!(
            nav.focused_route().map(RootRoute::screen_id),
            Some("Settings")
        );
    }

    #[test]
    fn back_unwinds_the_rightmost_stack_first_and_stops_at_the_section() {
        let mut nav = store(PaneLayout::ThreeColumn);
        nav.push(chat("c1"));
        nav.push(RootRoute::Settings);

        assert_eq!(nav.back(), Some(RootRoute::Settings));
        assert_eq!(nav.back(), Some(chat("c1")));
        assert_eq!(nav.back(), None, "the section itself is not closable");
        assert!(!nav.can_go_back());
        assert_eq!(ids(&nav), vec!["Feed"]);
    }

    #[test]
    fn forward_restores_what_back_closed_into_the_same_column() {
        let mut nav = store(PaneLayout::ThreeColumn);
        nav.push(chat("c1"));
        nav.push(profile("u1"));

        nav.back();
        nav.back();
        assert_eq!(ids(&nav), vec!["Feed"]);
        assert!(nav.can_go_forward());

        assert_eq!(nav.forward(), Some(chat("c1")));
        assert_eq!(nav.forward(), Some(profile("u1")));
        assert_eq!(nav.forward(), None);
        assert_eq!(ids(&nav), vec!["Feed", "Chat:c1", "UserProfile:u1"]);
    }

    #[test]
    fn opening_a_new_screen_abandons_the_forward_history() {
        let mut nav = store(PaneLayout::TwoColumn);
        nav.push(chat("c1"));
        nav.back();
        assert!(nav.can_go_forward());

        nav.push(chat("c2"));
        assert!(!nav.can_go_forward());
        assert_eq!(nav.top(PaneSlot::Secondary), Some(&chat("c2")));
    }

    #[test]
    fn forward_history_is_bounded() {
        let mut nav = store(PaneLayout::TwoColumn);
        for index in 0..FORWARD_LIMIT * 2 {
            nav.push(chat(&format!("c{index}")));
        }
        for _ in 0..FORWARD_LIMIT * 2 {
            nav.back();
        }
        assert!(nav.can_go_forward());
        let mut restored = 0;
        while nav.forward().is_some() {
            restored += 1;
        }
        assert_eq!(restored, FORWARD_LIMIT);
    }

    #[test]
    fn pushing_the_screen_that_is_already_on_top_changes_nothing() {
        let mut nav = store(PaneLayout::TwoColumn);
        nav.push(chat("c1"));
        nav.push(chat("c1"));
        assert_eq!(ids(&nav), vec!["Feed", "Chat:c1"]);
        assert_eq!(nav.back(), Some(chat("c1")));
        assert_eq!(nav.back(), None);
    }

    #[test]
    fn each_section_keeps_the_screens_it_had_open() {
        let mut nav = store(PaneLayout::TwoColumn);
        nav.select_tab(RootTab::Chats);
        nav.push(chat("c1"));

        nav.select_tab(RootTab::Feed);
        assert_eq!(ids(&nav), vec!["Feed"]);

        nav.select_tab(RootTab::Chats);
        assert_eq!(
            ids(&nav),
            vec!["Chats", "Chat:c1"],
            "leaving a section and returning must not close what was open in it"
        );
    }

    #[test]
    fn reselecting_the_active_section_returns_to_its_root() {
        let mut nav = store(PaneLayout::Stacked);
        nav.select_tab(RootTab::Chats);
        nav.push(RootRoute::Onboarding);
        nav.push(chat("c1"));
        nav.select_tab(RootTab::Chats);

        assert_eq!(nav.tab(), RootTab::Chats);
        assert_eq!(
            nav.top(PaneSlot::Primary),
            Some(&RootRoute::Tab(RootTab::Chats))
        );
        assert_eq!(
            nav.top(PaneSlot::Secondary),
            Some(&chat("c1")),
            "reaching for the section a conversation lives in must not close it"
        );
        assert!(!nav.can_go_forward());
    }

    #[test]
    fn a_tab_route_selects_its_section_rather_than_stacking_on_top_of_one() {
        let mut nav = store(PaneLayout::TwoColumn);
        nav.push(RootRoute::Tab(RootTab::Search));

        assert_eq!(nav.tab(), RootTab::Search);
        assert_eq!(ids(&nav), vec!["Search"]);
    }

    #[test]
    fn the_pre_auth_flow_stacks_over_the_section_and_leaves_when_popped() {
        let mut nav = store(PaneLayout::ThreeColumn);
        nav.push(RootRoute::Onboarding);

        assert!(
            nav.active_routes()
                .iter()
                .any(|route| !route.requires_authentication()),
            "onboarding must be visible for the shell to know to hide the rail"
        );

        nav.back();
        assert!(
            nav.active_routes()
                .iter()
                .all(|route| route.requires_authentication())
        );
    }
}
