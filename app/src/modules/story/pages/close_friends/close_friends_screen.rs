use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, Context, ElementId, Entity, FontWeight, Hsla, ListAlignment, SharedString, Subscription, Window};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal, RiseTimingCanon};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_platform::haptics::SelectionFeedbackGenerator;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::{
    ChatProfileEmptyState, ChatProfileTabErrorState,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatType, User};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::post::stores::post_actions::post_types::AuthorInfo;
use crate::modules::search::stores::search_actions::search_actions::SearchActionsStore;
use crate::modules::search::stores::search_actions::search_wire_types::SearchUsersListDTOExt;
use crate::modules::search::stores::search_interactions::search_interactions::SearchInteractionsStore;
use crate::modules::search::stores::search_services::search_services::SearchServicesStore;
use crate::modules::story::stores::story::story_actions::story_actions::{
    StoryActionDoneHandler, StoryActionTextHandler,
};
use crate::modules::story::stores::story::story_actions::story_wire_types::StoryAuthorDTO;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;

const SEARCH_DEBOUNCE_ID: &str = "closeFriendsSearch";
const SEARCH_DEBOUNCE_MAX_CACHE_DATA: i64 = 10;
const PAGINATION_THRESHOLD: usize = 8;
const RESULT_SKELETON_ROWS: usize = 6;
const SECTION_SPACING: f32 = 14.0;
const RECENT_SPACING: f32 = 8.0;
const BOTTOM_SPACER: f32 = 80.0;
const TOGGLE_ANIMATION_SECONDS: f64 = 0.15;

const RECENT_SUGGESTION_LIMIT: usize = 30;

struct CloseFriendsScreenState {
    query: String,
    selected: HashMap<String, StoryAuthorDTO>,
    order: Vec<String>,
    pinned: Vec<StoryAuthorDTO>,
    did_seed: bool,
    seeded_ids: HashSet<String>,
    is_saving: bool,
    load_failed: bool,
    _observations: [Subscription; 3],
}

impl CloseFriendsScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let service = StoryServicesStore::shared(cx);
        let chats = ChatServicesStore::shared(cx);
        let search = SearchServicesStore::shared(cx);
        Self {
            query: String::new(),
            selected: HashMap::new(),
            order: Vec::new(),
            pinned: Vec::new(),
            did_seed: false,
            seeded_ids: HashSet::new(),
            is_saving: false,
            load_failed: false,
            _observations: [
                cx.observe(&service, |_, _, cx| cx.notify()),
                cx.observe(&chats, |_, _, cx| cx.notify()),
                cx.observe(&search, |_, _, cx| cx.notify()),
            ],
        }
    }
}

struct CloseFriendsListModel {
    list: ListUi,
    ids: Vec<String>,
}

impl CloseFriendsListModel {
    fn new(cx: &App) -> Self {
        Self {
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            ids: Vec::new(),
        }
    }

    fn sync(&mut self, ids: Vec<String>) {
        if ids == self.ids {
            return;
        }
        if !self.ids.is_empty() && ids.len() > self.ids.len() && ids.starts_with(&self.ids) {
            self.list.appended(ids.len() - self.ids.len());
        } else {
            self.list.replaced(ids.len());
        }
        self.ids = ids;
    }
}

struct CloseFriendsPrefetch {
    query: String,
    options: SaiPaginationPrefetchOptions,
}

/// one row of the scroller flattened, a long result list stays cheap only as one virtual list
enum CloseFriendsRow {
    Search,
    Gap { key: &'static str, height: f32 },
    LoadError,
    Author { author: StoryAuthorDTO, anchor: Option<RiseGuideAnchor>, prefetch: Option<CloseFriendsPrefetch> },
    EmptyState { icon: &'static str, text: String },
    EmptySlot { shows_empty: bool },
    RecentHeader,
    ResultsSkeleton,
    LoadMore { query: String },
}

impl CloseFriendsRow {
    fn id(&self) -> String {
        match self {
            Self::Search => "search".to_owned(),
            Self::Gap { key, .. } => format!("gap-{key}"),
            Self::LoadError => "error".to_owned(),
            Self::Author { author, .. } => format!("author-{}", author.id),
            Self::EmptyState { icon, .. } => format!("empty-{icon}"),
            Self::EmptySlot { .. } => "empty-slot".to_owned(),
            Self::RecentHeader => "recent-header".to_owned(),
            Self::ResultsSkeleton => "skeleton".to_owned(),
            Self::LoadMore { query } => format!("load-more-{query}"),
        }
    }
}

#[derive(Clone)]
struct CloseFriendsScreenParts {
    state: Entity<CloseFriendsScreenState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl CloseFriendsScreenParts {
    fn pop(&self, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |store, cx| store.pop(cx));
        }
    }

    /// nothing to save before the list is seeded
    /// empty order against a loaded list read as changed
    /// compares against what it seeded
    fn is_dirty(&self, cx: &mut App) -> bool {
        if !self.state.read(cx).did_seed {
            return false;
        }
        let saved: HashSet<String> = if GuideSandbox::shared(cx).read(cx).owns_close_friends() {
            self.state.read(cx).seeded_ids.clone()
        } else {
            let friends = StoryServicesStore::shared(cx).update(cx, |service, cx| service.close_friends(cx));
            friends.into_iter().map(|friend| friend.id).collect()
        };
        let order: HashSet<String> = self.state.read(cx).order.iter().cloned().collect();
        order != saved
    }

    fn done_button(&self, theme: &AppTheme, cx: &App) -> impl IntoElement {
        let title = tr("common_done");
        let parts = self.clone();
        LiquidGlassButton::new("close-friends-done", LiquidGlassButtonShape::Capsule)
            .horizontal_padding(14.0)
            .vertical_padding(10.0)
            .disabled(self.state.read(cx).is_saving)
            .accessibility_label(Some(title.clone()))
            .on_press(move |_, cx| parts.save(cx))
            .child(MainText::new(title).px(14.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary))
            .guide_anchor(RiseGuideAnchor::CloseFriendsDone)
    }

    fn rows(&self, cx: &mut App) -> Vec<CloseFriendsRow> {
        let state = self.state.read(cx);
        let did_seed = state.did_seed;
        let load_failed = state.load_failed;
        let query = trimming_whitespaces_and_newlines(&state.query).to_owned();
        let mut rows = vec![CloseFriendsRow::Search, CloseFriendsRow::Gap { key: "search", height: SECTION_SPACING }];
        if !did_seed && load_failed {
            rows.push(CloseFriendsRow::LoadError);
        } else if query.is_empty() {
            self.selected_section(&mut rows, cx);
            rows.push(CloseFriendsRow::Gap { key: "selected", height: SECTION_SPACING });
            self.recent_section(&mut rows, cx);
        } else {
            Self::results_section(&query, &mut rows, cx);
        }
        rows.push(CloseFriendsRow::Gap { key: "bottom", height: SECTION_SPACING + BOTTOM_SPACER });
        rows
    }

    fn selected_section(&self, rows: &mut Vec<CloseFriendsRow>, cx: &App) {
        for author in &self.state.read(cx).pinned {
            rows.push(CloseFriendsRow::Author { author: author.clone(), anchor: None, prefetch: None });
        }
    }

    fn recent_section(&self, rows: &mut Vec<CloseFriendsRow>, cx: &mut App) {
        let pinned = self.state.read(cx).pinned.clone();
        let recents = Self::recent_direct_chat_authors(&pinned, cx);
        if recents.is_empty() {
            if pinned.is_empty() {
                rows.push(CloseFriendsRow::EmptyState { icon: "star.circle", text: tr("story_close_friends_empty") });
            }
            return;
        }
        if pinned.is_empty() {
            let state = self.state.read(cx);
            rows.push(CloseFriendsRow::EmptySlot { shows_empty: state.did_seed && state.selected.is_empty() });
            rows.push(CloseFriendsRow::Gap { key: "empty-slot", height: RECENT_SPACING });
        }
        rows.push(CloseFriendsRow::RecentHeader);
        rows.push(CloseFriendsRow::Gap { key: "recent-header", height: RECENT_SPACING });
        let first_id = recents.first().map(|author| author.id.clone());
        for author in recents {
            let anchor = (Some(&author.id) == first_id.as_ref()).then_some(RiseGuideAnchor::CloseFriendsFirstSuggestion);
            rows.push(CloseFriendsRow::Author { author, anchor, prefetch: None });
        }
    }

    fn recent_direct_chat_authors(pinned: &[StoryAuthorDTO], cx: &mut App) -> Vec<StoryAuthorDTO> {
        let taken: HashSet<&str> = pinned.iter().map(|author| author.id.as_str()).collect();
        let mut seen: HashSet<String> = HashSet::new();
        let mut result: Vec<StoryAuthorDTO> = Vec::new();
        if let Some(panda) = GuideSandbox::shared(cx).read(cx).close_friends_suggestion() {
            if !taken.contains(panda.id.as_str()) {
                seen.insert(panda.id.clone());
                result.push(panda);
            }
        }

        let chats = ChatServicesStore::shared(cx).update(cx, |store, cx| store.current_chat_infos(cx));
        for chat in &chats {
            if chat.r#type != ChatType::Direct || chat.is_bot_chat() {
                continue;
            }
            let participant = &chat.participant;
            let id = trimming_whitespaces_and_newlines(&participant.id).to_owned();
            if id.is_empty() || participant.is_deleted || taken.contains(id.as_str()) || !seen.insert(id) {
                continue;
            }
            result.push(Self::story_author_from_user(participant));
            if result.len() == RECENT_SUGGESTION_LIMIT {
                break;
            }
        }
        result
    }

    fn results_section(query: &str, rows: &mut Vec<CloseFriendsRow>, cx: &mut App) {
        let own_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone());
        let Some(page) = SearchServicesStore::shared(cx).update(cx, |store, cx| store.search_users_page(query, cx)) else {
            rows.push(CloseFriendsRow::ResultsSkeleton);
            return;
        };
        let mut seen: HashSet<String> = HashSet::new();
        let authors: Vec<StoryAuthorDTO> = page
            .authors()
            .iter()
            .filter(|author| author.id != own_id)
            .map(Self::story_author_from_author)
            .filter(|author| seen.insert(author.id.clone()))
            .collect();
        let has_more = page.is_have_more;
        if authors.is_empty() && !has_more {
            rows.push(CloseFriendsRow::EmptyState { icon: "magnifyingglass", text: tr("people_picker_empty") });
            return;
        }
        let cache_key = SearchServicesStore::search_users_cache_key(query);
        let relative_id = page.relative_id.map(|id| id.to_string());
        let item_count = authors.len();
        for (index, author) in authors.into_iter().enumerate() {
            let options =
                SaiPaginationPrefetchOptions::new(index, item_count, cache_key.clone(), relative_id.clone(), has_more)
                    .threshold(PAGINATION_THRESHOLD);
            rows.push(CloseFriendsRow::Author {
                author,
                anchor: None,
                prefetch: Some(CloseFriendsPrefetch { query: query.to_owned(), options }),
            });
        }
        if has_more {
            rows.push(CloseFriendsRow::LoadMore { query: query.to_owned() });
        }
    }

    fn schedule_search(value: &str, cx: &mut App) {
        let trimmed = trimming_whitespaces_and_newlines(value).to_owned();
        if trimmed.is_empty() {
            return;
        }
        SaiDebouncer::debounced_action_delay_milliseconds(
            SEARCH_DEBOUNCE_ID,
            Box::new(move |cx: &mut App| {
                SearchActionsStore::shared(cx).update(cx, |store, cx| {
                    store.search_users_action(&trimmed, None, true, false, cx);
                });
            }),
            RiseTimingCanon::debounce("global_search").client_debounce_ms,
            SEARCH_DEBOUNCE_ID,
            SEARCH_DEBOUNCE_MAX_CACHE_DATA,
            None,
            cx,
        );
    }

    fn list_row(&self, row: &CloseFriendsRow, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        match row {
            CloseFriendsRow::Search => {
                let query = Binding::entity(
                    self.state.clone(),
                    |state: &CloseFriendsScreenState| state.query.clone(),
                    |state, value, _| state.query = value,
                );
                div()
                    .w_full()
                    .px(px(16.0))
                    .pt(px(10.0))
                    .child(
                        SearchFieldUi::new("close-friends-search", query)
                            .placeholder(SharedString::from(tr("story_close_friends_search")))
                            .on_change(|value, _, cx| Self::schedule_search(value, cx))
                            .uses_inline_text_field(true),
                    )
                    .into_any_element()
            }
            CloseFriendsRow::Gap { height, .. } => div().w_full().h(px(*height)).into_any_element(),
            CloseFriendsRow::LoadError => {
                let parts = self.clone();
                ChatProfileTabErrorState::new(tr("error_fetching_data"), move |_, cx| parts.load_list(true, cx))
                    .into_any_element()
            }
            CloseFriendsRow::Author { author, anchor, prefetch } => {
                let element = self.row(author, &theme, window, cx).guide_anchor(*anchor);
                match prefetch {
                    Some(prefetch) => {
                        let query = prefetch.query.clone();
                        element
                            .sai_pagination_prefetch(
                                SharedString::from(format!("close-friends-prefetch-{}", author.id)),
                                prefetch.options.clone(),
                                move |_, cx| {
                                    SearchInteractionsStore::shared(cx)
                                        .update(cx, |store, cx| store.load_more_plain_users(&query, cx));
                                },
                            )
                            .into_any_element()
                    }
                    None => element.into_any_element(),
                }
            }
            CloseFriendsRow::EmptyState { icon, text } => {
                ChatProfileEmptyState::new(*icon, text.clone()).into_any_element()
            }
            CloseFriendsRow::EmptySlot { shows_empty } => {
                let opacity = Self::empty_slot_opacity(*shows_empty, window, cx);
                div()
                    .w_full()
                    .py(px(16.0))
                    .opacity(opacity)
                    .child(
                        MainText::new(tr("story_close_friends_empty"))
                            .px(14.0)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary),
                    )
                    .into_any_element()
            }
            CloseFriendsRow::RecentHeader => div()
                .w_full()
                .child(
                    MainText::new(tr("story_close_friends_recent"))
                        .px(13.0)
                        .tac(TextAlignT::Left)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.secondary)
                        .m_l(20.0),
                )
                .into_any_element(),
            CloseFriendsRow::ResultsSkeleton => {
                UserListSkeleton::new().rows(RESULT_SKELETON_ROWS).shows_separators(false).into_any_element()
            }
            CloseFriendsRow::LoadMore { query } => {
                let load_query = query.clone();
                div()
                    .w_full()
                    .py(px(16.0))
                    .flex()
                    .justify_center()
                    .child(progress_view())
                    .on_appear(SharedString::from(format!("close-friends-load-more-{query}")), move |_, cx| {
                        SearchInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.load_more_plain_users(&load_query, cx));
                    })
                    .into_any_element()
            }
        }
    }

    fn empty_slot_opacity(shows_empty: bool, window: &mut Window, cx: &mut App) -> f32 {
        let target = if shows_empty { 1.0 } else { 0.0 };
        let now = RiseClock::monotonic();
        let state = window.use_keyed_state("close-friends-empty-slot", cx, |_, _| Animated::new(target));
        state.update(cx, |animated, cx| {
            animated.set_in(target, ease_out(TOGGLE_ANIMATION_SECONDS), now, cx);
            animated.frame_value(now, window)
        })
    }

    /// eases the check mark color in 0.15 s
    fn check_color(id: &str, target: Hsla, window: &mut Window, cx: &mut App) -> Hsla {
        let now = RiseClock::monotonic();
        let state = window.use_keyed_state(SharedString::from(format!("close-friends-check-{id}")), cx, |_, _| {
            Animated::new(target)
        });
        state.update(cx, |animated, cx| {
            animated.set_in(target, ease_out(TOGGLE_ANIMATION_SECONDS), now, cx);
            animated.frame_value(now, window)
        })
    }

    fn row(&self, author: &StoryAuthorDTO, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let id = author.id.clone();
        let is_on = self.state.read(cx).selected.contains_key(&id);
        let tag = format!("@{}", author.tag);
        let title = if author.name.is_empty() { tag.clone() } else { author.name.clone() };
        let check_target = if is_on { theme.primary._100 } else { theme.text.secondary.opacity(0.35) };
        let check_color = Self::check_color(&id, check_target, window, cx);
        let parts = self.clone();
        let toggled = author.clone();
        plain(SharedString::from(format!("close-friends-row-{id}")))
            .on_press(move |_, cx| parts.toggle(&toggled, cx))
            .w_full()
            .child(
                hstack(12.0)
                    .w_full()
                    .px(px(16.0))
                    .py(px(8.0))
                    .child(
                        UserLogoUi::new(SharedString::from(format!("close-friends-row-logo-{id}")))
                            .source(SharedString::from(author.logo.clone()))
                            .name(SharedString::from(author.name.clone()))
                            .user_id(SharedString::from(id.clone()))
                            .size(40.0)
                            .show_story_ring(false)
                            .open_story_on_press(false),
                    )
                    .child(
                        vstack(2.0)
                            .flex_1()
                            .min_w_0()
                            .align(HorizontalAlignment::Leading)
                            .child(
                                MainText::new(title).px(15.0).tac(TextAlignT::Left).font_weight(FontWeight::MEDIUM),
                            )
                            .when(!author.tag.is_empty(), |labels| {
                                labels.child(
                                    MainText::new(tag).px(12.0).tac(TextAlignT::Left).color(theme.text.secondary),
                                )
                            }),
                    )
                    .children(IconUi::sized(
                        if is_on { "checkmark.circle.fill" } else { "circle" },
                        px(22.0),
                        check_color,
                        false,
                    )),
            )
            .into_any_element()
    }

    fn seed_if_needed(&self, cx: &mut App) {
        if self.state.read(cx).did_seed {
            return;
        }
        let service = StoryServicesStore::shared(cx);
        let loaded = service.update(cx, |service, cx| service.close_friends_loaded(cx));
        if !loaded && !GuideSandbox::shared(cx).read(cx).owns_close_friends() {
            return;
        }
        let friends = service.update(cx, |service, cx| service.close_friends(cx));
        self.state.update(cx, |state, cx| {
            state.did_seed = true;
            state.order = friends.iter().map(|friend| friend.id.clone()).filter(|id| !id.is_empty()).collect();
            state.seeded_ids = state.order.iter().cloned().collect();
            state.pinned = friends.iter().filter(|friend| !friend.id.is_empty()).cloned().collect();
            state.selected = HashMap::new();
            for friend in &state.pinned {
                state.selected.entry(friend.id.clone()).or_insert_with(|| friend.clone());
            }
            cx.notify();
        });
    }

    /// a tick before the list is seeded had nothing to save against and was lost on back
    fn toggle(&self, author: &StoryAuthorDTO, cx: &mut App) {
        let id = author.id.clone();
        if id.is_empty() {
            return;
        }
        let state = self.state.read(cx);
        if !state.did_seed {
            if state.load_failed {
                Self::present_error(tr("error_fetching_data"), cx);
            }
            return;
        }
        if state.selected.contains_key(&id) {
            self.state.update(cx, |state, cx| {
                state.selected.remove(&id);
                state.order.retain(|entry| *entry != id);
                cx.notify();
            });
        } else {
            let limit = StoryInteractionsStore::close_friends_limit();
            if state.order.len() as i64 >= limit {
                Self::present_limit(limit, cx);
                return;
            }
            let pinned = state.pinned.clone();
            let joins_pinned = !pinned.iter().any(|entry| entry.id == id)
                && !Self::recent_direct_chat_authors(&pinned, cx).iter().any(|entry| entry.id == id);
            let author = author.clone();
            self.state.update(cx, |state, cx| {
                state.selected.insert(id.clone(), author.clone());
                state.order.insert(0, id.clone());
                if joins_pinned {
                    state.pinned.insert(0, author);
                }
                cx.notify();
            });
        }
        let _ = SelectionFeedbackGenerator::new().selection_changed();
    }

    fn load_list(&self, force: bool, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.load_failed = false;
            cx.notify();
        });
        let state = self.state.clone();
        let on_error: StoryActionDoneHandler = Rc::new(move |cx: &mut App| {
            state.update(cx, |state, cx| {
                state.load_failed = true;
                cx.notify();
            });
        });
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.load_close_friends(force, Some(on_error), cx));
    }

    /// screen closes on the server answer, a refusal shown after the pop had no screen to draw on
    fn save(&self, cx: &mut App) {
        let state = self.state.read(cx);
        if state.is_saving {
            return;
        }
        let friends: Vec<StoryAuthorDTO> = state.order.iter().filter_map(|id| state.selected.get(id).cloned()).collect();
        self.state.update(cx, |state, cx| {
            state.is_saving = true;
            cx.notify();
        });
        let success_parts = self.clone();
        let on_success: StoryActionDoneHandler = Rc::new(move |cx: &mut App| {
            if !success_parts.state.read(cx).is_saving {
                return;
            }
            success_parts.state.update(cx, |state, cx| {
                state.is_saving = false;
                cx.notify();
            });
            success_parts.pop(cx);
        });
        let error_state = self.state.clone();
        let on_error: StoryActionTextHandler = Rc::new(move |message: String, cx: &mut App| {
            let was_saving = error_state.read(cx).is_saving;
            error_state.update(cx, |state, cx| {
                state.is_saving = false;
                cx.notify();
            });
            if was_saving {
                Self::present_error(message, cx);
            }
        });
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.save_close_friends(&friends, Some(on_success), Some(on_error), cx);
        });
    }

    fn present_error(message: String, cx: &mut App) {
        NativeConfirmAlert::present_notice(&tr("story_close_friends_error"), &message, None, None, |_| {}, cx);
    }

    fn present_limit(limit: i64, cx: &mut App) {
        NativeConfirmAlert::present_notice(
            &tr("story_close_friends_limit_title"),
            &tr_args("story_close_friends_limit_message", &[FormatArg::Int(limit)]),
            None,
            None,
            |_| {},
            cx,
        );
    }

    fn story_author_from_user(user: &User) -> StoryAuthorDTO {
        StoryAuthorDTO {
            id: user.id.clone(),
            is_online: false,
            is_premium: user.more.is_premium,
            last_seen_ms: None,
            logo: user.more.logo.clone(),
            name: user.name.clone(),
            name_color: user.more.name_color.clone(),
            premium_badge_emoji: user.more.premium_badge_emoji.clone(),
            tag: user.tag.clone(),
            ..StoryAuthorDTO::default()
        }
    }

    fn story_author_from_author(author: &AuthorInfo) -> StoryAuthorDTO {
        StoryAuthorDTO {
            id: author.id.clone().unwrap_or_default(),
            is_online: false,
            is_premium: author.more.is_premium,
            last_seen_ms: None,
            logo: author.more.logo.clone(),
            name: author.name.clone(),
            name_color: author.more.name_color.clone(),
            premium_badge_emoji: author.more.premium_badge_emoji.clone(),
            tag: author.tag.clone(),
            ..StoryAuthorDTO::default()
        }
    }
}

#[derive(IntoElement)]
struct CloseFriendsScreenBody {
    parts: CloseFriendsScreenParts,
}

impl RenderOnce for CloseFriendsScreenBody {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let parts = self.parts;
        let background = rise_ui::theme(cx).bg._100;
        let rows = Rc::new(parts.rows(cx));
        let ids: Vec<String> = rows.iter().map(CloseFriendsRow::id).collect();
        let model = window.use_keyed_state("close-friends-list", cx, |_, cx| CloseFriendsListModel::new(cx));
        model.update(cx, |model, _| model.sync(ids));
        model
            .read(cx)
            .list
            .element(move |index, window, cx| match rows.get(index) {
                Some(row) => parts.list_row(row, window, cx),
                None => div().into_any_element(),
            })
            .size_full()
            .list_shell_scroll_under_chrome(cx)
            .bg(background)
    }
}

/// close friends list saved as one atomic replace so a network drop cant leave it half applied
/// membership is one sided so the picker uses global user search
#[derive(IntoElement, Default)]
pub struct CloseFriendsScreen;

impl CloseFriendsScreen {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for CloseFriendsScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("close-friends-state", cx, |_, cx| CloseFriendsScreenState::new(cx));
        let parts = CloseFriendsScreenParts { state, nav_store: env::<NavigationStoreKey>(cx) };

        let task_parts = parts.clone();
        use_task("close-friends-load", (), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| {
                task_parts.load_list(false, cx);
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_initial_chats(cx));
            });
        });

        let loaded = StoryServicesStore::shared(cx).update(cx, |service, cx| service.close_friends_loaded(cx));
        let loaded_parts = parts.clone();
        on_change("close-friends-loaded", loaded, false, window, cx, move |_, _, _, cx| {
            loaded_parts.seed_if_needed(cx);
        });
        let is_dirty = parts.is_dirty(cx);
        on_change("close-friends-dirty", is_dirty, true, window, cx, |_, dirty, _, cx| {
            GuideFacts::set(RiseGuideSignal::CloseFriendsChanged, *dirty, cx);
        });

        let back_parts = parts.clone();
        let shell = SettingsWrapperUi::new(tr("story_close_friends_title"), move |_, cx| back_parts.pop(cx));
        let shell = if is_dirty { shell.right_jsx(parts.done_button(&theme, cx)) } else { shell };

        let appear_parts = parts.clone();
        let disappear_state = parts.state.clone();
        shell
            .content(CloseFriendsScreenBody { parts })
            .on_appear(ElementId::from("close-friends-lifecycle"), move |_, cx| appear_parts.seed_if_needed(cx))
            .and_on_disappear(move |_, cx| {
                GuideFacts::set(RiseGuideSignal::CloseFriendsChanged, false, cx);
                disappear_state.update(cx, |state, cx| {
                    state.is_saving = false;
                    cx.notify();
                });
            })
    }
}
