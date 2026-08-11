#![allow(dead_code)]

use std::collections::VecDeque;

use rise_navigation::{PaneSlot, RootRoute};

pub const MAX_LEASES: usize = 64;

pub const MAX_CLAIM_RECORDS: usize = 128;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ScreenIdentity {
    screen_id: &'static str,
    resource_key: String,
}

impl ScreenIdentity {
    pub fn for_route(route: &RootRoute) -> Self {
        Self {
            screen_id: route.screen_id(),
            resource_key: route.resource_key(),
        }
    }

    pub fn new(screen_id: &'static str, resource_key: impl Into<String>) -> Self {
        Self {
            screen_id,
            resource_key: resource_key.into(),
        }
    }

    pub fn screen_id(&self) -> &'static str {
        self.screen_id
    }

    pub fn resource_key(&self) -> &str {
        &self.resource_key
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CacheKey(String);

impl CacheKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&ScreenIdentity> for CacheKey {
    fn from(identity: &ScreenIdentity) -> Self {
        Self(identity.resource_key.clone())
    }
}

impl From<String> for CacheKey {
    fn from(key: String) -> Self {
        Self(key)
    }
}

impl From<&str> for CacheKey {
    fn from(key: &str) -> Self {
        Self(key.to_owned())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum LeaseColumn {
    Primary,
    Secondary,
    Tertiary,
    Overlay,
}

impl LeaseColumn {
    pub const ALL: [LeaseColumn; 4] = [
        LeaseColumn::Primary,
        LeaseColumn::Secondary,
        LeaseColumn::Tertiary,
        LeaseColumn::Overlay,
    ];

    pub fn index(self) -> usize {
        match self {
            Self::Primary => 0,
            Self::Secondary => 1,
            Self::Tertiary => 2,
            Self::Overlay => 3,
        }
    }
}

impl From<PaneSlot> for LeaseColumn {
    fn from(slot: PaneSlot) -> Self {
        match slot {
            PaneSlot::Primary => Self::Primary,
            PaneSlot::Secondary => Self::Secondary,
            PaneSlot::Tertiary => Self::Tertiary,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum RefreshReason {
    #[default]
    ColdStart,
    Foreground,
    Reconnect,
    EventGap,
    Manual,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct RefreshCycle(u64);

impl RefreshCycle {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VisibleResourceLease {
    owner: ScreenIdentity,
    key: CacheKey,
    column: LeaseColumn,
    depth: u32,
    appeared_at: u64,
    seen_revision: Option<u64>,
}

impl VisibleResourceLease {
    pub fn owner(&self) -> &ScreenIdentity {
        &self.owner
    }

    pub fn key(&self) -> &CacheKey {
        &self.key
    }

    pub fn column(&self) -> LeaseColumn {
        self.column
    }

    pub fn depth(&self) -> u32 {
        self.depth
    }

    pub fn appeared_at(&self) -> u64 {
        self.appeared_at
    }

    pub fn seen_revision(&self) -> Option<u64> {
        self.seen_revision
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RefreshGrant {
    cycle: RefreshCycle,
    reason: RefreshReason,
    key: CacheKey,
    seen_revision: Option<u64>,
}

impl RefreshGrant {
    pub fn cycle(&self) -> RefreshCycle {
        self.cycle
    }

    pub fn reason(&self) -> RefreshReason {
        self.reason
    }

    pub fn key(&self) -> &CacheKey {
        &self.key
    }

    pub fn seen_revision(&self) -> Option<u64> {
        self.seen_revision
    }
}

#[derive(Default)]
pub struct ActiveScreens {
    leases: Vec<VisibleResourceLease>,
    claimed: VecDeque<CacheKey>,
    cycle: RefreshCycle,
    reason: RefreshReason,
    next_tick: u64,
}

impl ActiveScreens {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cycle(&self) -> RefreshCycle {
        self.cycle
    }

    pub fn reason(&self) -> RefreshReason {
        self.reason
    }

    pub fn len(&self) -> usize {
        self.leases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.leases.is_empty()
    }

    pub fn register(
        &mut self,
        owner: ScreenIdentity,
        key: CacheKey,
        column: LeaseColumn,
    ) -> &VisibleResourceLease {
        let index = self.insert_or_update(owner, key, column);
        &self.leases[index]
    }

    // Never clears the claim record: a resize must not refetch inside the same cycle.
    pub fn release(&mut self, owner: &ScreenIdentity) {
        self.leases.retain(|lease| lease.owner != *owner);
    }

    pub fn sync_visible(&mut self, visible: &[(ScreenIdentity, LeaseColumn)]) {
        self.leases
            .retain(|lease| visible.iter().any(|(owner, _)| *owner == lease.owner));

        for (owner, column) in visible {
            if self.index_of(owner).is_none() {
                self.insert_or_update(owner.clone(), CacheKey::from(owner), *column);
            }
        }

        let mut next_depth = [0u32; LeaseColumn::ALL.len()];
        for (owner, column) in visible {
            let depth = &mut next_depth[column.index()];
            *depth += 1;
            if let Some(index) = self.index_of(owner) {
                self.leases[index].column = *column;
                self.leases[index].depth = *depth;
            }
        }
    }

    pub fn active_screens(&self) -> impl Iterator<Item = &ScreenIdentity> {
        self.leases.iter().map(|lease| &lease.owner)
    }

    pub fn leases(&self) -> impl Iterator<Item = &VisibleResourceLease> {
        self.leases.iter()
    }

    pub fn lease_for(&self, owner: &ScreenIdentity) -> Option<&VisibleResourceLease> {
        self.index_of(owner).map(|index| &self.leases[index])
    }

    pub fn focused(&self) -> Option<&ScreenIdentity> {
        let index = self.topmost_in(LeaseColumn::Overlay).or_else(|| {
            [
                LeaseColumn::Tertiary,
                LeaseColumn::Secondary,
                LeaseColumn::Primary,
            ]
            .into_iter()
            .find_map(|column| self.topmost_in(column))
        })?;
        Some(&self.leases[index].owner)
    }

    pub fn is_eligible(&self, owner: &ScreenIdentity) -> bool {
        let Some(index) = self.index_of(owner) else {
            return false;
        };
        if let Some(overlay) = self.topmost_in(LeaseColumn::Overlay) {
            return overlay == index;
        }
        self.topmost_in(self.leases[index].column) == Some(index)
    }

    pub fn begin_cycle(&mut self, reason: RefreshReason) -> RefreshCycle {
        self.cycle = RefreshCycle(self.cycle.0 + 1);
        self.reason = reason;
        self.claimed.clear();
        self.cycle
    }

    pub fn claim_refresh(
        &mut self,
        owner: &ScreenIdentity,
        key: &CacheKey,
    ) -> Option<RefreshGrant> {
        if !self.is_eligible(owner) || self.claimed.contains(key) {
            return None;
        }

        while self.claimed.len() >= MAX_CLAIM_RECORDS {
            self.claimed.pop_front();
        }
        self.claimed.push_back(key.clone());

        Some(RefreshGrant {
            cycle: self.cycle,
            reason: self.reason,
            key: key.clone(),
            seen_revision: self.seen_revision(key),
        })
    }

    pub fn is_visible(&self, screen: &ScreenIdentity) -> bool {
        self.index_of(screen).is_some()
    }

    pub fn suppresses_notification(&self, key: &CacheKey) -> bool {
        self.holds(key)
    }

    pub fn evictable(&self, cached: &[CacheKey]) -> Vec<CacheKey> {
        cached
            .iter()
            .filter(|key| !self.holds(key))
            .cloned()
            .collect()
    }

    pub fn note_revision(&mut self, key: &CacheKey, revision: u64) {
        for lease in self.leases.iter_mut().filter(|lease| lease.key == *key) {
            lease.seen_revision = Some(match lease.seen_revision {
                Some(seen) => seen.max(revision),
                None => revision,
            });
        }
    }

    fn insert_or_update(
        &mut self,
        owner: ScreenIdentity,
        key: CacheKey,
        column: LeaseColumn,
    ) -> usize {
        if let Some(index) = self.index_of(&owner) {
            let depth = if self.leases[index].column == column {
                self.leases[index].depth
            } else {
                self.next_depth(column)
            };
            let lease = &mut self.leases[index];
            lease.key = key;
            lease.column = column;
            lease.depth = depth;
            return index;
        }

        while self.leases.len() >= MAX_LEASES {
            let Some(oldest) = self.oldest_lease() else {
                break;
            };
            self.leases.remove(oldest);
        }

        let depth = self.next_depth(column);
        let appeared_at = self.next_tick;
        self.next_tick += 1;
        self.leases.push(VisibleResourceLease {
            owner,
            key,
            column,
            depth,
            appeared_at,
            seen_revision: None,
        });
        self.leases.len() - 1
    }

    fn oldest_lease(&self) -> Option<usize> {
        self.leases
            .iter()
            .enumerate()
            .min_by_key(|(_, lease)| lease.appeared_at)
            .map(|(index, _)| index)
    }

    fn next_depth(&self, column: LeaseColumn) -> u32 {
        self.leases
            .iter()
            .filter(|lease| lease.column == column)
            .map(|lease| lease.depth)
            .max()
            .unwrap_or(0)
            + 1
    }

    fn index_of(&self, owner: &ScreenIdentity) -> Option<usize> {
        self.leases.iter().position(|lease| lease.owner == *owner)
    }

    fn topmost_in(&self, column: LeaseColumn) -> Option<usize> {
        self.leases
            .iter()
            .enumerate()
            .filter(|(_, lease)| lease.column == column)
            .max_by_key(|(_, lease)| (lease.depth, lease.appeared_at))
            .map(|(index, _)| index)
    }

    fn holds(&self, key: &CacheKey) -> bool {
        self.leases.iter().any(|lease| lease.key == *key)
    }

    fn seen_revision(&self, key: &CacheKey) -> Option<u64> {
        self.leases
            .iter()
            .filter(|lease| lease.key == *key)
            .filter_map(|lease| lease.seen_revision)
            .max()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(id: &'static str) -> ScreenIdentity {
        ScreenIdentity::new(id, id)
    }

    fn key(key: &str) -> CacheKey {
        CacheKey::from(key)
    }

    fn own_key(owner: &ScreenIdentity) -> CacheKey {
        CacheKey::from(owner)
    }

    fn three_columns() -> (ActiveScreens, Vec<(ScreenIdentity, LeaseColumn)>) {
        let visible = vec![
            (screen("Feed"), LeaseColumn::Primary),
            (screen("Chat:c1"), LeaseColumn::Secondary),
            (screen("UserProfile:u1"), LeaseColumn::Tertiary),
        ];
        let mut screens = ActiveScreens::new();
        screens.sync_visible(&visible);
        (screens, visible)
    }

    #[test]
    fn an_identity_built_from_a_route_carries_the_routes_own_key() {
        let route = RootRoute::Chat {
            chat_id: "c1".into(),
        };
        let identity = ScreenIdentity::for_route(&route);
        assert_eq!(identity.screen_id(), route.screen_id());
        assert_eq!(identity.resource_key(), route.resource_key());
        assert_eq!(CacheKey::from(&identity).as_str(), route.resource_key());
    }

    #[test]
    fn a_lease_column_mirrors_the_pane_slot_it_came_from() {
        assert_eq!(LeaseColumn::from(PaneSlot::Primary), LeaseColumn::Primary);
        assert_eq!(
            LeaseColumn::from(PaneSlot::Secondary),
            LeaseColumn::Secondary
        );
        assert_eq!(LeaseColumn::from(PaneSlot::Tertiary), LeaseColumn::Tertiary);
        assert!(LeaseColumn::Overlay > LeaseColumn::Tertiary);
    }

    #[test]
    fn eligibility_gives_every_column_its_own_grant_when_no_overlay_is_open() {
        let (mut screens, visible) = three_columns();
        for (owner, _) in &visible {
            assert!(
                screens.is_eligible(owner),
                "{owner:?} is topmost in its own column and must be able to refresh"
            );
        }
        let grants = visible
            .iter()
            .filter(|(owner, _)| screens.claim_refresh(owner, &own_key(owner)).is_some())
            .count();
        assert_eq!(grants, 3);
    }

    #[test]
    fn eligibility_leaves_only_the_topmost_overlay_while_a_sheet_is_open() {
        let feed = screen("Feed");
        let comments = screen("Comments:p1");
        let replies = screen("CommentReplies:r1");
        let mut screens = ActiveScreens::new();
        screens.sync_visible(&[
            (feed.clone(), LeaseColumn::Primary),
            (comments.clone(), LeaseColumn::Overlay),
            (replies.clone(), LeaseColumn::Overlay),
        ]);

        assert!(screens.is_eligible(&replies));
        assert!(!screens.is_eligible(&comments));
        assert!(
            !screens.is_eligible(&feed),
            "a feed underneath an open sheet must not fetch alongside it"
        );
        assert!(screens.claim_refresh(&feed, &own_key(&feed)).is_none());
    }

    #[test]
    fn eligibility_skips_a_screen_covered_inside_its_own_column() {
        let feed = screen("Feed");
        let post = screen("Post:p1");
        let mut screens = ActiveScreens::new();
        screens.sync_visible(&[
            (feed.clone(), LeaseColumn::Primary),
            (post.clone(), LeaseColumn::Primary),
        ]);

        assert!(screens.is_eligible(&post));
        assert!(!screens.is_eligible(&feed));
    }

    #[test]
    fn a_key_is_claimed_once_per_cycle_however_many_panes_ask() {
        let conversation = ScreenIdentity::new("Chat", "Chat:c1");
        let info = ScreenIdentity::new("ChatInfo", "Chat:c1");
        let mut screens = ActiveScreens::new();
        screens.sync_visible(&[
            (conversation.clone(), LeaseColumn::Secondary),
            (info.clone(), LeaseColumn::Tertiary),
        ]);

        let shared = key("Chat:c1");
        assert!(screens.claim_refresh(&conversation, &shared).is_some());
        assert!(
            screens.claim_refresh(&info, &shared).is_none(),
            "two panes on one conversation are one resource, not two"
        );
    }

    #[test]
    fn a_key_claimed_once_per_cycle_ignores_the_order_the_panes_ask_in() {
        let conversation = ScreenIdentity::new("Chat", "Chat:c1");
        let info = ScreenIdentity::new("ChatInfo", "Chat:c1");
        let mut screens = ActiveScreens::new();
        screens.sync_visible(&[
            (conversation.clone(), LeaseColumn::Secondary),
            (info.clone(), LeaseColumn::Tertiary),
        ]);

        let shared = key("Chat:c1");
        assert!(screens.claim_refresh(&info, &shared).is_some());
        assert!(screens.claim_refresh(&conversation, &shared).is_none());
    }

    #[test]
    fn a_rearrangement_between_two_asks_does_not_open_a_second_claim() {
        let conversation = ScreenIdentity::new("Chat", "Chat:c1");
        let info = ScreenIdentity::new("ChatInfo", "Chat:c1");
        let mut screens = ActiveScreens::new();
        screens.sync_visible(&[
            (conversation.clone(), LeaseColumn::Secondary),
            (info.clone(), LeaseColumn::Tertiary),
        ]);
        let shared = key("Chat:c1");
        assert!(screens.claim_refresh(&conversation, &shared).is_some());

        screens.sync_visible(&[
            (info.clone(), LeaseColumn::Secondary),
            (conversation.clone(), LeaseColumn::Tertiary),
        ]);

        assert!(screens.claim_refresh(&info, &shared).is_none());
        assert!(screens.claim_refresh(&conversation, &shared).is_none());
    }

    #[test]
    fn a_visibility_change_never_starts_a_cycle() {
        let (mut screens, visible) = three_columns();
        let cycle = screens.cycle();

        screens.release(&visible[0].0);
        screens.register(screen("Settings"), key("Settings"), LeaseColumn::Tertiary);
        screens.sync_visible(&visible);

        assert_eq!(screens.cycle(), cycle);
    }

    #[test]
    fn a_new_cycle_lets_every_key_be_claimed_again() {
        let (mut screens, visible) = three_columns();
        let feed = &visible[0].0;
        assert!(screens.claim_refresh(feed, &own_key(feed)).is_some());
        assert!(screens.claim_refresh(feed, &own_key(feed)).is_none());

        let cycle = screens.begin_cycle(RefreshReason::Reconnect);

        let grant = screens
            .claim_refresh(feed, &own_key(feed))
            .expect("a new cycle re-arms every key");
        assert_eq!(grant.cycle(), cycle);
        assert_eq!(grant.reason(), RefreshReason::Reconnect);
    }

    #[test]
    fn the_resize_invariant_holds_across_a_collapse_and_a_reopen() {
        let (mut screens, wide) = three_columns();
        screens.begin_cycle(RefreshReason::Foreground);
        let first = wide
            .iter()
            .filter(|(owner, _)| screens.claim_refresh(owner, &own_key(owner)).is_some())
            .count();
        assert_eq!(first, 3);

        screens.sync_visible(&[(wide[0].0.clone(), LeaseColumn::Primary)]);
        assert_eq!(screens.len(), 1);
        screens.sync_visible(&wide);
        assert_eq!(screens.len(), 3);

        for (owner, _) in &wide {
            assert!(
                screens.claim_refresh(owner, &own_key(owner)).is_none(),
                "{owner:?} refetched after a window resize inside the same cycle"
            );
        }
    }

    #[test]
    fn releasing_a_lease_does_not_forget_that_its_key_was_refreshed() {
        let feed = screen("Feed");
        let mut screens = ActiveScreens::new();
        screens.register(feed.clone(), own_key(&feed), LeaseColumn::Primary);
        assert!(screens.claim_refresh(&feed, &own_key(&feed)).is_some());

        screens.release(&feed);
        screens.register(feed.clone(), own_key(&feed), LeaseColumn::Primary);

        assert!(screens.claim_refresh(&feed, &own_key(&feed)).is_none());
    }

    #[test]
    fn changing_column_keeps_the_lease_instead_of_recreating_it() {
        let chat = screen("Chat:c1");
        let mut screens = ActiveScreens::new();
        screens.register(chat.clone(), own_key(&chat), LeaseColumn::Secondary);
        screens.note_revision(&own_key(&chat), 7);
        let appeared_at = screens.lease_for(&chat).expect("registered").appeared_at();

        screens.sync_visible(&[(chat.clone(), LeaseColumn::Primary)]);

        let lease = screens.lease_for(&chat).expect("still visible");
        assert_eq!(lease.column(), LeaseColumn::Primary);
        assert_eq!(lease.appeared_at(), appeared_at);
        assert_eq!(lease.seen_revision(), Some(7));
    }

    #[test]
    fn the_lease_list_evicts_the_oldest_when_it_overflows() {
        let mut screens = ActiveScreens::new();
        let first = ScreenIdentity::new("Search", "Search:0");
        screens.register(first.clone(), own_key(&first), LeaseColumn::Primary);
        for index in 1..MAX_LEASES {
            let owner = ScreenIdentity::new("Search", format!("Search:{index}"));
            screens.register(owner.clone(), own_key(&owner), LeaseColumn::Primary);
        }
        assert_eq!(screens.len(), MAX_LEASES);

        let newest = ScreenIdentity::new("Search", format!("Search:{MAX_LEASES}"));
        screens.register(newest.clone(), own_key(&newest), LeaseColumn::Primary);

        assert_eq!(screens.len(), MAX_LEASES);
        assert!(
            !screens.is_visible(&first),
            "the oldest lease must go first"
        );
        assert!(screens.is_visible(&newest));
    }

    #[test]
    fn an_overflowing_claim_record_evicts_the_oldest_and_permits_one_extra_refresh() {
        let search = screen("Search");
        let mut screens = ActiveScreens::new();
        screens.register(search.clone(), own_key(&search), LeaseColumn::Primary);

        let first = key("Search:q0");
        assert!(screens.claim_refresh(&search, &first).is_some());
        for index in 1..MAX_CLAIM_RECORDS {
            let query = CacheKey::from(format!("Search:q{index}"));
            assert!(screens.claim_refresh(&search, &query).is_some());
        }
        assert!(screens.claim_refresh(&search, &first).is_none());

        let overflow = CacheKey::from(format!("Search:q{MAX_CLAIM_RECORDS}"));
        assert!(screens.claim_refresh(&search, &overflow).is_some());

        assert!(
            screens.claim_refresh(&search, &first).is_some(),
            "the evicted record is the documented cost of a bounded registry"
        );
    }

    #[test]
    fn suppression_lasts_exactly_as_long_as_the_lease() {
        let chat = screen("Chat:c1");
        let other = key("Chat:c2");
        let mut screens = ActiveScreens::new();
        screens.register(chat.clone(), own_key(&chat), LeaseColumn::Secondary);

        assert!(screens.suppresses_notification(&own_key(&chat)));
        assert!(!screens.suppresses_notification(&other));

        screens.release(&chat);
        assert!(!screens.suppresses_notification(&own_key(&chat)));
    }

    #[test]
    fn suppression_survives_being_covered_by_an_overlay() {
        let chat = screen("Chat:c1");
        let palette = screen("CommandPalette");
        let mut screens = ActiveScreens::new();
        screens.sync_visible(&[
            (chat.clone(), LeaseColumn::Secondary),
            (palette, LeaseColumn::Overlay),
        ]);

        assert!(!screens.is_eligible(&chat));
        assert!(
            screens.suppresses_notification(&own_key(&chat)),
            "the conversation is still on screen even when it may not refresh"
        );
    }

    #[test]
    fn evictable_names_every_cached_key_with_no_live_lease() {
        let (screens, visible) = three_columns();
        let cached = vec![
            own_key(&visible[0].0),
            key("Chat:c9"),
            own_key(&visible[2].0),
            key("Post:p3"),
        ];

        assert_eq!(
            screens.evictable(&cached),
            vec![key("Chat:c9"), key("Post:p3")]
        );
    }

    #[test]
    fn focused_prefers_the_topmost_overlay() {
        let (mut screens, visible) = three_columns();
        let palette = screen("CommandPalette");
        let mut with_overlay = visible.clone();
        with_overlay.push((palette.clone(), LeaseColumn::Overlay));
        screens.sync_visible(&with_overlay);

        assert_eq!(screens.focused(), Some(&palette));
    }

    #[test]
    fn focused_is_the_rightmost_occupied_column_without_an_overlay() {
        let (mut screens, visible) = three_columns();
        assert_eq!(screens.focused(), Some(&visible[2].0));

        screens.sync_visible(&visible[..2]);
        assert_eq!(screens.focused(), Some(&visible[1].0));

        screens.sync_visible(&visible[..1]);
        assert_eq!(screens.focused(), Some(&visible[0].0));

        screens.sync_visible(&[]);
        assert_eq!(screens.focused(), None);
    }

    #[test]
    fn note_revision_reaches_the_next_grant() {
        let feed = screen("Feed");
        let mut screens = ActiveScreens::new();
        screens.register(feed.clone(), own_key(&feed), LeaseColumn::Primary);
        screens.note_revision(&own_key(&feed), 11);
        screens.note_revision(&own_key(&feed), 4);

        let grant = screens
            .claim_refresh(&feed, &own_key(&feed))
            .expect("the only screen is eligible");
        assert_eq!(grant.seen_revision(), Some(11));
        assert_eq!(grant.key(), &own_key(&feed));
    }

    #[test]
    fn a_screen_with_no_lease_cannot_claim_anything() {
        let mut screens = ActiveScreens::new();
        let ghost = screen("Feed");
        assert!(!screens.is_visible(&ghost));
        assert!(screens.claim_refresh(&ghost, &own_key(&ghost)).is_none());
        assert!(screens.is_empty());
    }

    #[test]
    fn active_screens_is_a_set_not_a_single_screen() {
        let (screens, visible) = three_columns();
        let ids: Vec<_> = screens
            .active_screens()
            .map(ScreenIdentity::screen_id)
            .collect();
        assert_eq!(ids.len(), visible.len());
        assert_eq!(screens.leases().count(), visible.len());
    }
}
