use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{App, Context, ElementId, Empty, Entity, FontWeight, Hsla, ListAlignment, SharedString, Subscription, Window};
use rise_canon::{RiseLimits, RisePrivacyRule, RiseTimingCanon};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_platform::haptics::SelectionFeedbackGenerator;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatType, User};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::post::stores::post_actions::post_types::AuthorInfo;
use crate::modules::search::stores::search_actions::search_actions::SearchActionsStore;
use crate::modules::search::stores::search_actions::search_wire_types::SearchUsersListDTOExt;
use crate::modules::search::stores::search_interactions::search_interactions::SearchInteractionsStore;
use crate::modules::search::stores::search_services::search_services::SearchServicesStore;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;
use crate::modules::user::stores::user::user_actions::user_types::PrivacyExceptionAction;
use crate::modules::user::stores::user::user_actions::user_wire_types::PrivacyExceptionWireDTO;
use crate::modules::user::stores::user::user_interactions::user_interactions::{
    UserInteractionsStore, UserInteractionsTextHandler,
};
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const SEARCH_DEBOUNCE_ID: &str = "privacyExceptionsSearch";
const SEARCH_DEBOUNCE_MAX_CACHE_DATA: i64 = 10;
const PAGINATION_THRESHOLD: usize = 8;
const RESULT_SKELETON_ROWS: usize = 6;
const SECTION_SPACING: f32 = 14.0;
const RECENT_SPACING: f32 = 8.0;
const BOTTOM_SPACER: f32 = 80.0;
const TOGGLE_ANIMATION_SECONDS: f64 = 0.15;
const RECENT_SUGGESTION_LIMIT: usize = 30;

static PER_LIST_LIMIT: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("max_exceptions"));

struct PrivacyExceptionsScreenState {
    query: String,
    selected: HashMap<String, PrivacyExceptionWireDTO>,
    order: Vec<String>,
    pinned: Vec<PrivacyExceptionWireDTO>,
    did_seed: bool,
    _observations: [Subscription; 5],
}

impl PrivacyExceptionsScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let user_store = UserServicesStore::shared(cx);
        let actions = UserActionsStore::shared(cx);
        let pending_requests = SaiWsPendingRegistry::shared(cx);
        let chats = ChatServicesStore::shared(cx);
        let search = SearchServicesStore::shared(cx);
        Self {
            query: String::new(),
            selected: HashMap::new(),
            order: Vec::new(),
            pinned: Vec::new(),
            did_seed: false,
            _observations: [
                cx.observe(&user_store, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&pending_requests, |_, _, cx| cx.notify()),
                cx.observe(&chats, |_, _, cx| cx.notify()),
                cx.observe(&search, |_, _, cx| cx.notify()),
            ],
        }
    }
}

struct PrivacyExceptionsListModel {
    list: ListUi,
    ids: Vec<String>,
}

impl PrivacyExceptionsListModel {
    fn new(cx: &App) -> Self {
        Self { list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top), ids: Vec::new() }
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

struct PrivacyExceptionsPrefetch {
    query: String,
    options: SaiPaginationPrefetchOptions,
}

/// one row of the scroller, flattened so a long result list stays one virtual list
enum PrivacyExceptionsRow {
    Search,
    Hint,
    Gap { key: &'static str, height: f32 },
    User { user: PrivacyExceptionWireDTO, prefetch: Option<PrivacyExceptionsPrefetch> },
    EmptyState { icon: &'static str, text: String },
    RecentHeader,
    ResultsSkeleton,
    LoadMore { query: String },
}

impl PrivacyExceptionsRow {
    fn id(&self) -> String {
        match self {
            Self::Search => "search".to_owned(),
            Self::Hint => "hint".to_owned(),
            Self::Gap { key, .. } => format!("gap-{key}"),
            Self::User { user, .. } => format!("user-{}", user.target_user_id),
            Self::EmptyState { icon, .. } => format!("empty-{icon}"),
            Self::RecentHeader => "recent-header".to_owned(),
            Self::ResultsSkeleton => "skeleton".to_owned(),
            Self::LoadMore { query } => format!("load-more-{query}"),
        }
    }
}

#[derive(Clone)]
struct PrivacyExceptionsScreenParts {
    r#type: String,
    action: String,
    state: Entity<PrivacyExceptionsScreenState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl PrivacyExceptionsScreenParts {
    fn pop(&self, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |store, cx| store.pop(cx));
        }
    }

    fn caps_selection(cx: &mut App) -> bool {
        !AuthServicesStore::shared(cx).read(cx).profile().is_some_and(|profile| profile.more.is_premium)
    }

    fn rule(&self) -> Option<RisePrivacyRule> {
        RisePrivacyRule::ALL.into_iter().find(|rule| rule.privacy_type() == self.r#type)
    }

    fn exception_action(&self) -> PrivacyExceptionAction {
        PrivacyExceptionAction::from_raw(&self.action).unwrap_or(PrivacyExceptionAction::Allow)
    }

    fn server_list(&self, cx: &mut App) -> Vec<PrivacyExceptionWireDTO> {
        let action = self.exception_action();
        UserServicesStore::shared(cx).update(cx, |store, cx| store.privacy_exceptions(&self.r#type, action, cx))
    }

    /// nothing to save before the list is seeded, else a one frame done would flash before onAppear
    fn is_dirty(&self, cx: &mut App) -> bool {
        if !self.state.read(cx).did_seed {
            return false;
        }
        let server: HashSet<String> = self.server_list(cx).into_iter().map(|user| user.target_user_id).collect();
        let order: HashSet<String> = self.state.read(cx).order.iter().cloned().collect();
        order != server
    }

    fn done_button(&self, theme: &AppTheme) -> impl IntoElement {
        let title = tr("common_done");
        let parts = self.clone();
        LiquidGlassButton::new("privacy-exceptions-done", LiquidGlassButtonShape::Capsule)
            .horizontal_padding(14.0)
            .vertical_padding(10.0)
            .accessibility_label(Some(title.clone()))
            .on_press(move |_, cx| parts.save(cx))
            .child(MainText::new(title).px(14.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary))
    }

    fn rows(&self, cx: &mut App) -> Vec<PrivacyExceptionsRow> {
        let query = trimming_whitespaces_and_newlines(&self.state.read(cx).query).to_owned();
        let mut rows = vec![
            PrivacyExceptionsRow::Search,
            PrivacyExceptionsRow::Gap { key: "search", height: SECTION_SPACING },
            PrivacyExceptionsRow::Hint,
            PrivacyExceptionsRow::Gap { key: "hint", height: SECTION_SPACING },
        ];
        if query.is_empty() {
            self.selected_section(&mut rows, cx);
            self.recent_section(&mut rows, cx);
        } else {
            Self::results_section(&query, &mut rows, cx);
        }
        rows.push(PrivacyExceptionsRow::Gap { key: "bottom", height: SECTION_SPACING + BOTTOM_SPACER });
        rows
    }

    fn selected_section(&self, rows: &mut Vec<PrivacyExceptionsRow>, cx: &mut App) {
        let pinned = self.state.read(cx).pinned.clone();
        if pinned.is_empty() && Self::recent_direct_chat_users(&pinned, cx).is_empty() {
            rows.push(PrivacyExceptionsRow::EmptyState {
                icon: "person.crop.circle.badge.plus",
                text: tr("privacy_exceptions_empty"),
            });
            return;
        }
        for user in pinned {
            rows.push(PrivacyExceptionsRow::User { user, prefetch: None });
        }
    }

    fn recent_section(&self, rows: &mut Vec<PrivacyExceptionsRow>, cx: &mut App) {
        let pinned = self.state.read(cx).pinned.clone();
        let recents = Self::recent_direct_chat_users(&pinned, cx);
        if recents.is_empty() {
            return;
        }
        rows.push(PrivacyExceptionsRow::Gap { key: "selected", height: SECTION_SPACING });
        rows.push(PrivacyExceptionsRow::RecentHeader);
        rows.push(PrivacyExceptionsRow::Gap { key: "recent-header", height: RECENT_SPACING });
        for user in recents {
            rows.push(PrivacyExceptionsRow::User { user, prefetch: None });
        }
    }

    /// direct chats in chat list order
    /// groups and channels and saved messages and bots and top list users are out
    fn recent_direct_chat_users(pinned: &[PrivacyExceptionWireDTO], cx: &mut App) -> Vec<PrivacyExceptionWireDTO> {
        let taken: HashSet<&str> = pinned.iter().map(|user| user.target_user_id.as_str()).collect();
        let mut seen: HashSet<String> = HashSet::new();
        let mut result: Vec<PrivacyExceptionWireDTO> = Vec::new();

        let chats = ChatServicesStore::shared(cx).read(cx).current_chat_infos(cx);
        for chat in &chats {
            if chat.r#type != ChatType::Direct || chat.is_bot_chat() {
                continue;
            }
            let participant = &chat.participant;
            let id = trimming_whitespaces_and_newlines(&participant.id).to_owned();
            if id.is_empty() || participant.is_deleted || taken.contains(id.as_str()) || !seen.insert(id) {
                continue;
            }
            result.push(Self::exception_user_from_user(participant));
            if result.len() == RECENT_SUGGESTION_LIMIT {
                break;
            }
        }
        result
    }

    fn results_section(query: &str, rows: &mut Vec<PrivacyExceptionsRow>, cx: &mut App) {
        let own_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone());
        let Some(page) = SearchServicesStore::shared(cx).update(cx, |store, cx| store.search_users_page(query, cx)) else {
            rows.push(PrivacyExceptionsRow::ResultsSkeleton);
            return;
        };
        let mut seen: HashSet<String> = HashSet::new();
        let users: Vec<PrivacyExceptionWireDTO> = page
            .authors()
            .iter()
            .filter(|author| author.id != own_id)
            .map(Self::exception_user_from_author)
            .filter(|user| seen.insert(user.target_user_id.clone()))
            .collect();
        let has_more = page.is_have_more;
        if users.is_empty() && !has_more {
            rows.push(PrivacyExceptionsRow::EmptyState { icon: "magnifyingglass", text: tr("people_picker_empty") });
            return;
        }
        let cache_key = SearchServicesStore::search_users_cache_key(query);
        let relative_id = page.relative_id.map(|id| id.to_string());
        let item_count = users.len();
        for (index, user) in users.into_iter().enumerate() {
            let options =
                SaiPaginationPrefetchOptions::new(index, item_count, cache_key.clone(), relative_id.clone(), has_more)
                    .threshold(PAGINATION_THRESHOLD);
            rows.push(PrivacyExceptionsRow::User {
                user,
                prefetch: Some(PrivacyExceptionsPrefetch { query: query.to_owned(), options }),
            });
        }
        if has_more {
            rows.push(PrivacyExceptionsRow::LoadMore { query: query.to_owned() });
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

    fn list_row(&self, row: &PrivacyExceptionsRow, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        match row {
            PrivacyExceptionsRow::Search => {
                let query = Binding::entity(
                    self.state.clone(),
                    |state: &PrivacyExceptionsScreenState| state.query.clone(),
                    |state, value, _| state.query = value,
                );
                div()
                    .w_full()
                    .px(px(16.0))
                    .pt(px(10.0))
                    .child(
                        SearchFieldUi::new("privacy-exceptions-search", query)
                            .placeholder(SharedString::from(tr("story_close_friends_search")))
                            .on_change(|value, _, cx| Self::schedule_search(value, cx))
                            .uses_inline_text_field(true),
                    )
                    .into_any_element()
            }
            PrivacyExceptionsRow::Hint => {
                let key = match self.exception_action() {
                    PrivacyExceptionAction::Allow => "privacy_exceptions_hint_allow",
                    PrivacyExceptionAction::Deny => "privacy_exceptions_hint_deny",
                };
                div().w_full().px(px(20.0)).child(SecondaryText::new(tr(key)).px(12.0)).into_any_element()
            }
            PrivacyExceptionsRow::Gap { height, .. } => div().w_full().h(px(*height)).into_any_element(),
            PrivacyExceptionsRow::User { user, prefetch } => {
                let element = self.row(user, &theme, window, cx);
                match prefetch {
                    Some(prefetch) => {
                        let query = prefetch.query.clone();
                        element
                            .sai_pagination_prefetch(
                                SharedString::from(format!("privacy-exceptions-prefetch-{}", user.target_user_id)),
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
            PrivacyExceptionsRow::EmptyState { icon, text } => {
                ChatProfileEmptyState::new(*icon, text.clone()).into_any_element()
            }
            PrivacyExceptionsRow::RecentHeader => div()
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
            PrivacyExceptionsRow::ResultsSkeleton => {
                UserListSkeleton::new().rows(RESULT_SKELETON_ROWS).shows_separators(false).into_any_element()
            }
            PrivacyExceptionsRow::LoadMore { query } => {
                let load_query = query.clone();
                div()
                    .w_full()
                    .py(px(16.0))
                    .flex()
                    .justify_center()
                    .child(progress_view())
                    .on_appear(SharedString::from(format!("privacy-exceptions-load-more-{query}")), move |_, cx| {
                        SearchInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.load_more_plain_users(&load_query, cx));
                    })
                    .into_any_element()
            }
        }
    }

    /// check mark color with a 0.15s ease out
    fn check_color(id: &str, target: Hsla, window: &mut Window, cx: &mut App) -> Hsla {
        let now = RiseClock::monotonic();
        let state = window.use_keyed_state(SharedString::from(format!("privacy-exceptions-check-{id}")), cx, |_, _| {
            Animated::new(target)
        });
        state.update(cx, |animated, cx| {
            animated.set_in(target, ease_out(TOGGLE_ANIMATION_SECONDS), now, cx);
            animated.frame_value(now, window)
        })
    }

    fn row(&self, user: &PrivacyExceptionWireDTO, theme: &AppTheme, window: &mut Window, cx: &mut App) -> PlainButton {
        let id = user.target_user_id.clone();
        let is_on = self.state.read(cx).selected.contains_key(&id);
        let tag = format!("@{}", user.target_user_tag);
        let title = if user.target_user_name.is_empty() { tag.clone() } else { user.target_user_name.clone() };
        let check_target = if is_on { theme.primary._100 } else { theme.text.secondary.opacity(0.35) };
        let check_color = Self::check_color(&id, check_target, window, cx);
        let parts = self.clone();
        let toggled = user.clone();
        plain(SharedString::from(format!("privacy-exceptions-row-{id}")))
            .on_press(move |_, cx| parts.toggle(&toggled, cx))
            .w_full()
            .child(
                hstack(12.0)
                    .w_full()
                    .px(px(16.0))
                    .py(px(8.0))
                    .child(
                        UserLogoUi::new(SharedString::from(format!("privacy-exceptions-row-logo-{id}")))
                            .source(SharedString::from(user.target_user_logo.clone()))
                            .name(SharedString::from(user.target_user_name.clone()))
                            .user_id(SharedString::from(id.clone()))
                            .size(40.0)
                            .show_story_ring(false),
                    )
                    .child(
                        vstack(2.0)
                            .flex_1()
                            .min_w_0()
                            .align(HorizontalAlignment::Leading)
                            .child(
                                MainText::new(title).px(15.0).tac(TextAlignT::Left).font_weight(FontWeight::MEDIUM),
                            )
                            .when(!user.target_user_tag.is_empty(), |labels| {
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
    }

    fn seed_if_needed(&self, cx: &mut App) {
        if self.state.read(cx).did_seed {
            return;
        }
        let has_server_data = UserActionsStore::shared(cx).read(cx).get_privacy_sai.data().is_some();
        let has_optimistic = UserServicesStore::shared(cx).read(cx).optimistic_privacy().is_some();
        if !has_server_data && !has_optimistic {
            return;
        }
        let list = self.server_list(cx);
        self.state.update(cx, |state, cx| {
            state.did_seed = true;
            state.order = list.iter().map(|user| user.target_user_id.clone()).collect();
            state.selected = list.iter().map(|user| (user.target_user_id.clone(), user.clone())).collect();
            state.pinned = list;
            cx.notify();
        });
    }

    fn toggle(&self, user: &PrivacyExceptionWireDTO, cx: &mut App) {
        let id = user.target_user_id.clone();
        if id.is_empty() {
            return;
        }
        if self.state.read(cx).selected.contains_key(&id) {
            self.state.update(cx, |state, cx| {
                state.selected.remove(&id);
                state.order.retain(|entry| *entry != id);
                cx.notify();
            });
        } else {
            let count = self.state.read(cx).order.len() as i64;
            if Self::caps_selection(cx) && count >= *PER_LIST_LIMIT {
                Self::present_limit(cx);
                return;
            }
            let pinned = self.state.read(cx).pinned.clone();
            let joins_pinned = !pinned.iter().any(|entry| entry.target_user_id == id)
                && !Self::recent_direct_chat_users(&pinned, cx).iter().any(|entry| entry.target_user_id == id);
            let user = user.clone();
            self.state.update(cx, |state, cx| {
                state.selected.insert(id.clone(), user.clone());
                state.order.insert(0, id.clone());
                if joins_pinned {
                    state.pinned.insert(0, user);
                }
                cx.notify();
            });
        }
        let _ = SelectionFeedbackGenerator::new().selection_changed();
    }

    fn save(&self, cx: &mut App) {
        let Some(rule) = self.rule() else {
            return;
        };
        let state = self.state.read(cx);
        let users: Vec<PrivacyExceptionWireDTO> = state.order.iter().filter_map(|id| state.selected.get(id).cloned()).collect();
        let action = self.exception_action();
        let on_error: UserInteractionsTextHandler = Rc::new(|message: String, cx: &mut App| {
            NativeConfirmAlert::present_notice(&tr("privacy_exceptions_error"), &message, None, None, |_| {}, cx);
        });
        UserInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.save_privacy_exceptions(rule, action, &users, on_error, cx);
        });
        self.pop(cx);
    }

    fn present_limit(cx: &mut App) {
        NativeConfirmAlert::present_notice(
            &tr("privacy_exceptions_limit_title"),
            &tr_args("privacy_exceptions_limit_message", &[FormatArg::Int(*PER_LIST_LIMIT)]),
            None,
            None,
            |_| {},
            cx,
        );
    }

    fn exception_user_from_user(user: &User) -> PrivacyExceptionWireDTO {
        PrivacyExceptionWireDTO {
            target_user_id: user.id.clone(),
            target_user_logo: user.more.logo.clone(),
            target_user_name: user.name.clone(),
            target_user_tag: user.tag.clone(),
            ..PrivacyExceptionWireDTO::default()
        }
    }

    fn exception_user_from_author(author: &AuthorInfo) -> PrivacyExceptionWireDTO {
        PrivacyExceptionWireDTO {
            target_user_id: author.id.clone().unwrap_or_default(),
            target_user_logo: author.more.logo.clone(),
            target_user_name: author.name.clone(),
            target_user_tag: author.tag.clone(),
            ..PrivacyExceptionWireDTO::default()
        }
    }
}

#[derive(IntoElement)]
struct PrivacyExceptionsScreenBody {
    parts: PrivacyExceptionsScreenParts,
}

impl RenderOnce for PrivacyExceptionsScreenBody {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let parts = self.parts;
        let background = rise_ui::theme(cx).bg._100;
        let top_inset = list_shell_floating_top_inset(cx);
        let rows = Rc::new(parts.rows(cx));
        let ids: Vec<String> = rows.iter().map(PrivacyExceptionsRow::id).collect();
        let model = window.use_keyed_state("privacy-exceptions-list", cx, |_, cx| PrivacyExceptionsListModel::new(cx));
        model.update(cx, |model, _| model.sync(ids));
        model
            .read(cx)
            .list
            .element(move |index, window, cx| match rows.get(index) {
                Some(row) => {
                    let top_padding = if index == 0 { top_inset } else { 0.0 };
                    div().w_full().pt(px(top_padding)).child(parts.list_row(row, window, cx)).into_any_element()
                }
                None => Empty.into_any_element(),
            })
            .size_full()
            .bg(background)
    }
}

/// one exception list of a privacy rule, allow or deny
/// store sends the diff, shows the list as final
/// candidates from search and recent chats
#[derive(IntoElement)]
pub struct PrivacyExceptionsScreen {
    r#type: String,
    action: String,
}

impl PrivacyExceptionsScreen {
    pub fn new(r#type: impl Into<String>, action: impl Into<String>) -> Self {
        Self { r#type: r#type.into(), action: action.into() }
    }
}

impl RenderOnce for PrivacyExceptionsScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(
            SharedString::from(format!("privacy-exceptions-state-{}-{}", self.r#type, self.action)),
            cx,
            |_, cx| PrivacyExceptionsScreenState::new(cx),
        );
        let parts = PrivacyExceptionsScreenParts {
            r#type: self.r#type,
            action: self.action,
            state,
            nav_store: env::<NavigationStoreKey>(cx),
        };

        let status = UserActionsStore::shared(cx).read(cx).get_privacy_sai.status();
        let status_parts = parts.clone();
        on_change("privacy-exceptions-status", status, false, window, cx, move |_, _, _, cx| {
            status_parts.seed_if_needed(cx);
        });

        let title_key = match parts.exception_action() {
            PrivacyExceptionAction::Allow => "privacy_show_for",
            PrivacyExceptionAction::Deny => "privacy_hide_from",
        };
        let subtitle = parts.rule().and_then(|rule| rule.i18n_key()).map(|key| SharedString::from(tr(key)));
        let back_parts = parts.clone();
        let shell = SettingsWrapperUi::new(tr(title_key), move |_, cx| back_parts.pop(cx)).subtitle(subtitle);
        let shell = if parts.is_dirty(cx) { shell.right_jsx(parts.done_button(&theme)) } else { shell };

        let appear_parts = parts.clone();
        shell.content(PrivacyExceptionsScreenBody { parts }).on_appear(
            ElementId::from("privacy-exceptions-appear"),
            move |_, cx| {
                UserInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_privacy_loaded(cx));
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_initial_chats(cx));
                appear_parts.seed_if_needed(cx);
            },
        )
    }
}
