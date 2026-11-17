use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, Context, Subscription};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_theme::ThemeStore;

use crate::core::ui::custom_emoji_resource_warmup::CustomEmojiResourceWarmup;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostRowOwnPremium, PostRowPalette, PostRowPremiumBadge, PostRowSeedAuthor, PostRowStoryRingState,
};
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;
use crate::modules::stickers::engine::rise_sticker_presentation::RiseStickerPresentation;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;

#[derive(Clone, PartialEq)]
pub struct PostListObservedRow {
    pub presentation_id: String,
    pub author_id: Option<String>,
    pub is_author_deleted: bool,
    pub premium_badge: PostRowPremiumBadge,
}

/// layout bearing facts outside the seed, a change means a full re prepare
#[derive(Clone, Debug, PartialEq)]
pub struct PostListObservationSpecFacts {
    pub text_delta: f32,
    pub theme_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PostListObservationOwnPremiumFacts {
    user_id: String,
    is_premium: bool,
    name_color_rgb: Option<u32>,
    badge_emoji: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PostListObservationGroup {
    Spec,
    OwnPremium,
    Stories,
    Audio,
    HoldMenu,
    CustomEmoji,
}

impl PostListObservationGroup {
    const ALL_CASES: [PostListObservationGroup; 6] = [
        PostListObservationGroup::Spec,
        PostListObservationGroup::OwnPremium,
        PostListObservationGroup::Stories,
        PostListObservationGroup::Audio,
        PostListObservationGroup::HoldMenu,
        PostListObservationGroup::CustomEmoji,
    ];
}

/// store changes native rows depend on but seeds dont carry
pub struct PostListObservation {
    pub on_spec_changed: Option<Rc<dyn Fn(&mut App)>>,
    pub on_rows_changed: Option<Rc<dyn Fn(&HashSet<String>, &mut App)>>,
    pub on_audio_muted_changed: Option<Rc<dyn Fn(bool, &mut App)>>,
    pub on_hold_menu_active_changed: Option<Rc<dyn Fn(bool, &mut App)>>,
    pub on_premium_badges_may_have_changed: Option<Rc<dyn Fn(&HashSet<String>, &mut App)>>,

    revision: u64,
    spec_facts: PostListObservationSpecFacts,
    is_feed_audio_muted: bool,
    is_hold_menu_active: bool,
    own_premium: Option<PostRowOwnPremium>,

    rows: Rc<dyn Fn(&App) -> Vec<PostListObservedRow>>,
    own_premium_facts: Option<PostListObservationOwnPremiumFacts>,
    tracked_authors: Vec<String>,
    story_states: HashMap<String, PostRowStoryRingState>,
    requested_story_user_ids: HashSet<String>,
    tracks_custom_emoji: bool,
    custom_emoji_revision: u64,
    custom_emoji_unresolved_ids: HashSet<String>,
    subscriptions: HashMap<PostListObservationGroup, Vec<Subscription>>,
    is_started: bool,
    is_stopped: bool,
}

impl PostListObservation {
    /// rows returns the mounted rows (visible plus preload) whose authors are watched
    pub fn new(rows: Rc<dyn Fn(&App) -> Vec<PostListObservedRow>>, cx: &mut Context<Self>) -> Self {
        let premium = Self::current_own_premium_facts(cx);
        Self {
            on_spec_changed: None,
            on_rows_changed: None,
            on_audio_muted_changed: None,
            on_hold_menu_active_changed: None,
            on_premium_badges_may_have_changed: None,
            revision: 0,
            spec_facts: Self::current_spec_facts(cx),
            is_feed_audio_muted: ShortsServicesStore::shared(cx).read(cx).feed_audio_muted,
            is_hold_menu_active: HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some(),
            own_premium: premium.as_ref().map(Self::make_own_premium),
            rows,
            own_premium_facts: premium,
            tracked_authors: Vec::new(),
            story_states: HashMap::new(),
            requested_story_user_ids: HashSet::new(),
            tracks_custom_emoji: false,
            custom_emoji_revision: 0,
            custom_emoji_unresolved_ids: HashSet::new(),
            subscriptions: HashMap::new(),
            is_started: false,
            is_stopped: false,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn spec_facts(&self) -> &PostListObservationSpecFacts {
        &self.spec_facts
    }

    pub fn is_feed_audio_muted(&self) -> bool {
        self.is_feed_audio_muted
    }

    pub fn is_hold_menu_active(&self) -> bool {
        self.is_hold_menu_active
    }

    pub fn own_premium(&self) -> Option<&PostRowOwnPremium> {
        self.own_premium.as_ref()
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        if self.is_started || self.is_stopped {
            return;
        }
        self.is_started = true;
        self.refresh_values(cx);
        for group in PostListObservationGroup::ALL_CASES {
            self.arm(group, cx);
        }
        self.tracked_rows_did_change(cx);
    }

    pub fn stop(&mut self) {
        if self.is_stopped {
            return;
        }
        self.is_stopped = true;
        self.subscriptions.clear();
        self.on_spec_changed = None;
        self.on_rows_changed = None;
        self.on_audio_muted_changed = None;
        self.on_hold_menu_active_changed = None;
        self.on_premium_badges_may_have_changed = None;
    }

    /// mounted rows changed
    /// re-arms author tracking and batches story requests for unknown ones
    pub fn tracked_rows_did_change(&mut self, cx: &mut Context<Self>) {
        if !self.is_started || self.is_stopped {
            return;
        }
        let tracked = (self.rows)(&**cx);
        let mut seen = HashSet::new();
        let mut authors = Vec::new();
        for row in tracked.iter().filter(|row| !row.is_author_deleted) {
            let Some(id) = Self::normalized(row.author_id.as_deref()) else {
                continue;
            };
            if seen.insert(id.clone()) {
                authors.push(id);
            }
        }
        self.requested_story_user_ids.retain(|id| seen.contains(id));
        self.story_states.retain(|id, _| seen.contains(id));
        let authors_changed = self.tracked_authors.iter().cloned().collect::<HashSet<String>>() != seen;
        self.tracked_authors = authors;
        if authors_changed {
            self.arm(PostListObservationGroup::Stories, cx);
        }
        self.refresh_story_states(false, cx);

        let custom_rows: Vec<&PostListObservedRow> =
            tracked.iter().filter(|row| row.premium_badge.custom_emoji_id().is_some()).collect();
        let tracks_custom = !custom_rows.is_empty();
        if tracks_custom != self.tracks_custom_emoji {
            self.tracks_custom_emoji = tracks_custom;
            self.arm(PostListObservationGroup::CustomEmoji, cx);
        }
        Self::ensure_unresolved_custom_emoji(&custom_rows, cx);
    }

    /// active stories draw a ring, unseen the gradient
    /// unknown nothing
    pub fn story_ring_state(&self, user_id: &str, cx: &App) -> PostRowStoryRingState {
        let Some(id) = Self::normalized(Some(user_id)) else {
            return PostRowStoryRingState::NoStories;
        };
        rise_ui::shared::try_read::<StoryServicesStore>(cx)
            .map_or(PostRowStoryRingState::Unknown, |store| Self::story_ring_state_in(&id, store, cx))
    }

    fn arm(&mut self, group: PostListObservationGroup, cx: &mut Context<Self>) {
        self.subscriptions.remove(&group);
        if self.is_stopped {
            return;
        }
        let subscriptions = match group {
            PostListObservationGroup::Spec => {
                let store = ThemeStore::shared(cx);
                vec![cx.observe(&store, move |this, _, cx| this.deliver(group, cx))]
            }
            PostListObservationGroup::OwnPremium => {
                let store = AuthServicesStore::shared(cx);
                vec![cx.observe(&store, move |this, _, cx| this.deliver(group, cx))]
            }
            PostListObservationGroup::Stories => {
                if self.tracked_authors.is_empty() {
                    return;
                }
                let store = StoryServicesStore::shared(cx);
                vec![cx.observe(&store, move |this, _, cx| this.deliver(group, cx))]
            }
            PostListObservationGroup::Audio => {
                let store = ShortsServicesStore::shared(cx);
                vec![cx.observe(&store, move |this, _, cx| this.deliver(group, cx))]
            }
            PostListObservationGroup::HoldMenu => {
                let presenter = HoldContextMenuPresenter::shared(cx);
                vec![cx.observe(&presenter, move |this, _, cx| this.deliver(group, cx))]
            }
            PostListObservationGroup::CustomEmoji => {
                if !self.tracks_custom_emoji {
                    return;
                }
                let presentation = RiseStickerPresentation::shared(cx);
                let warmup = CustomEmojiResourceWarmup::shared(cx);
                self.custom_emoji_revision = presentation.read(cx).snapshot().revision;
                self.custom_emoji_unresolved_ids = warmup.read(cx).unresolved_custom_emoji_ids().clone();
                vec![
                    cx.observe(&presentation, move |this, _, cx| this.deliver(group, cx)),
                    cx.observe(&warmup, move |this, _, cx| this.deliver(group, cx)),
                ]
            }
        };
        self.subscriptions.insert(group, subscriptions);
    }

    fn deliver(&mut self, group: PostListObservationGroup, cx: &mut Context<Self>) {
        if !self.is_started || self.is_stopped {
            return;
        }
        match group {
            PostListObservationGroup::Spec => {
                let next = Self::current_spec_facts(cx);
                if next == self.spec_facts {
                    return;
                }
                self.spec_facts = next;
                cx.notify();
                if let Some(callback) = self.on_spec_changed.clone() {
                    self.publish(cx, move |cx| callback(cx));
                }
            }
            PostListObservationGroup::OwnPremium => {
                let next = Self::current_own_premium_facts(cx);
                if next == self.own_premium_facts {
                    return;
                }
                self.own_premium = next.as_ref().map(Self::make_own_premium);
                self.own_premium_facts = next;
                self.revision = self.revision.wrapping_add(1);
                cx.notify();
                let auth = AuthServicesStore::shared(cx);
                let auth = auth.read(cx);
                let ids: HashSet<String> = (self.rows)(&**cx)
                    .into_iter()
                    .filter(|row| {
                        Self::normalized(row.author_id.as_deref())
                            .is_some_and(|author_id| auth.is_current_user(&author_id))
                    })
                    .map(|row| row.presentation_id)
                    .collect();
                if !ids.is_empty() {
                    self.publish_rows_changed(ids.clone(), cx);
                    self.publish_premium_badges(ids, cx);
                }
            }
            PostListObservationGroup::Stories => self.refresh_story_states(true, cx),
            PostListObservationGroup::Audio => {
                let muted = ShortsServicesStore::shared(cx).read(cx).feed_audio_muted;
                if muted == self.is_feed_audio_muted {
                    return;
                }
                self.is_feed_audio_muted = muted;
                self.revision = self.revision.wrapping_add(1);
                cx.notify();
                if let Some(callback) = self.on_audio_muted_changed.clone() {
                    self.publish(cx, move |cx| callback(muted, cx));
                }
            }
            PostListObservationGroup::HoldMenu => {
                let active = HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some();
                if active == self.is_hold_menu_active {
                    return;
                }
                self.is_hold_menu_active = active;
                cx.notify();
                if let Some(callback) = self.on_hold_menu_active_changed.clone() {
                    self.publish(cx, move |cx| callback(active, cx));
                }
            }
            PostListObservationGroup::CustomEmoji => {
                let revision = RiseStickerPresentation::shared(cx).read(cx).snapshot().revision;
                let warmup = CustomEmojiResourceWarmup::shared(cx);
                let unresolved_ids = warmup.read(cx).unresolved_custom_emoji_ids();
                if revision == self.custom_emoji_revision && *unresolved_ids == self.custom_emoji_unresolved_ids {
                    return;
                }
                self.custom_emoji_revision = revision;
                self.custom_emoji_unresolved_ids = unresolved_ids.clone();
                let rows = (self.rows)(&**cx);
                let custom_rows: Vec<&PostListObservedRow> =
                    rows.iter().filter(|row| row.premium_badge.custom_emoji_id().is_some()).collect();
                if custom_rows.is_empty() {
                    return;
                }
                Self::ensure_unresolved_custom_emoji(&custom_rows, cx);
                let ids = custom_rows.iter().map(|row| row.presentation_id.clone()).collect();
                self.publish_premium_badges(ids, cx);
            }
        }
    }

    fn publish(&self, cx: &mut Context<Self>, callback: impl FnOnce(&mut App) + 'static) {
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            if this.upgrade().is_some_and(|observation| !observation.read(cx).is_stopped) {
                callback(cx);
            }
        });
    }

    fn publish_rows_changed(&self, ids: HashSet<String>, cx: &mut Context<Self>) {
        if let Some(callback) = self.on_rows_changed.clone() {
            self.publish(cx, move |cx| callback(&ids, cx));
        }
    }

    fn publish_premium_badges(&self, ids: HashSet<String>, cx: &mut Context<Self>) {
        if let Some(callback) = self.on_premium_badges_may_have_changed.clone() {
            self.publish(cx, move |cx| callback(&ids, cx));
        }
    }

    fn ensure_unresolved_custom_emoji(custom_rows: &[&PostListObservedRow], cx: &mut App) {
        for row in custom_rows.iter().filter(|row| row.premium_badge.is_unresolved_custom()) {
            let id = row.premium_badge.custom_emoji_id().unwrap_or_default();
            CustomEmojiResourceWarmup::shared(cx).update(cx, |warmup, cx| warmup.ensure_custom_emoji(&id, cx));
        }
    }

    fn refresh_values(&mut self, cx: &mut Context<Self>) {
        self.spec_facts = Self::current_spec_facts(cx);
        self.is_feed_audio_muted = ShortsServicesStore::shared(cx).read(cx).feed_audio_muted;
        self.is_hold_menu_active = HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some();
        let premium = Self::current_own_premium_facts(cx);
        self.own_premium = premium.as_ref().map(Self::make_own_premium);
        self.own_premium_facts = premium;
    }

    fn refresh_story_states(&mut self, notifies: bool, cx: &mut Context<Self>) {
        let store = StoryServicesStore::shared(cx);
        let mut changed_authors = HashSet::new();
        let mut unknown = Vec::new();
        for id in &self.tracked_authors {
            let state = Self::story_ring_state_in(id, store.read(cx), cx);
            if state == PostRowStoryRingState::Unknown && self.requested_story_user_ids.insert(id.clone()) {
                unknown.push(id.clone());
            }
            if self.story_states.get(id).is_some_and(|previous| *previous != state) {
                changed_authors.insert(id.clone());
            }
            self.story_states.insert(id.clone(), state);
        }
        if !unknown.is_empty() {
            StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.load_stories_state(&unknown, cx));
        }
        if !notifies || changed_authors.is_empty() {
            return;
        }
        self.revision = self.revision.wrapping_add(1);
        cx.notify();
        let ids: HashSet<String> = (self.rows)(&**cx)
            .into_iter()
            .filter(|row| {
                !row.is_author_deleted
                    && Self::normalized(row.author_id.as_deref()).is_some_and(|id| changed_authors.contains(&id))
            })
            .map(|row| row.presentation_id)
            .collect();
        if !ids.is_empty() {
            self.publish_rows_changed(ids, cx);
        }
    }

    fn current_spec_facts(cx: &mut App) -> PostListObservationSpecFacts {
        let theme = ThemeStore::shared(cx);
        let theme = theme.read(cx);
        PostListObservationSpecFacts {
            text_delta: theme.text_delta(),
            theme_id: PostRowPalette::new(theme.current_palette()).theme_id,
        }
    }

    /// premium name color and badge source for own posts
    fn current_own_premium_facts(cx: &mut App) -> Option<PostListObservationOwnPremiumFacts> {
        let auth = AuthServicesStore::shared(cx);
        let auth = auth.read(cx);
        let profile = auth.profile()?;
        let user_id = Self::normalized(auth.effective_user_id_for_wire().as_deref())
            .or_else(|| Self::normalized(Some(profile.id.as_str())))?;
        let more = &profile.more;
        Some(PostListObservationOwnPremiumFacts {
            user_id,
            is_premium: more.is_premium,
            name_color_rgb: if more.is_premium {
                PostRowSeedAuthor::parse_name_color(more.name_color.as_deref())
            } else {
                None
            },
            badge_emoji: more.premium_badge_emoji.clone(),
        })
    }

    fn make_own_premium(facts: &PostListObservationOwnPremiumFacts) -> PostRowOwnPremium {
        PostRowOwnPremium {
            user_id: facts.user_id.clone(),
            is_premium: facts.is_premium,
            name_color: facts.name_color_rgb.map(PostRowPalette::premium_name_color),
            badge_emoji: facts.badge_emoji.clone(),
        }
    }

    fn story_ring_state_in(user_id: &str, store: &StoryServicesStore, cx: &App) -> PostRowStoryRingState {
        if store.has_active_stories(Some(user_id), cx) {
            return if store.has_unseen_stories(Some(user_id), cx) {
                PostRowStoryRingState::Unseen
            } else {
                PostRowStoryRingState::Seen
            };
        }
        if store.has_known_stories_state(Some(user_id), cx) {
            PostRowStoryRingState::NoStories
        } else {
            PostRowStoryRingState::Unknown
        }
    }

    fn normalized(raw: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(raw?);
        (!value.is_empty()).then(|| value.to_owned())
    }
}
