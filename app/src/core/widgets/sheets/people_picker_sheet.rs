use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Hsla, IntoElement, ListAlignment, ParentElement, Pixels, Render,
    RenderOnce, SharedString, Styled, Subscription, Window, canvas, div, px,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal, RiseTimingCanon};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{tr, tr_plural};
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::official_badge::OfficialBadge;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions, NotifierType};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatType;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::post::stores::post_actions::post_types::{AuthorInfo, PostCollaboratorUser};
use crate::modules::search::stores::search_actions::search_actions::SearchActionsStore;
use crate::modules::search::stores::search_actions::search_wire_types::{SearchUsersListDTO, SearchUsersListDTOExt};
use crate::modules::search::stores::search_interactions::search_interactions::SearchInteractionsStore;
use crate::modules::search::stores::search_services::search_services::SearchServicesStore;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;
use crate::modules::user::stores::user::user_actions::user_types::{
    UserListContinuation, UserListEntry, UserListKind, UserRelationshipSearchContract,
};
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PeoplePickerMode {
    #[default]
    SearchAll,
    Friends,
}

type PeoplePickerDone = Box<dyn FnOnce(Vec<PostCollaboratorUser>, &mut App)>;

/// one people picker sheet for any surface that needs a set of accounts
/// returned as a single result on done, the caller decides what it becomes
pub struct PeoplePickerStore {
    is_presented: bool,
    mounts: bool,
    host_context: PeoplePickerHostContext,
    mode: PeoplePickerMode,
    limit: i64,
    title: String,
    hint: Option<String>,
    excluded_ids: HashSet<String>,
    selected: Vec<PostCollaboratorUser>,
    selected_ids: HashSet<String>,
    query: String,

    initial_ids: HashSet<String>,
    on_done: Option<PeoplePickerDone>,
    session: u64,
    network_down: bool,
    network_observation: Option<Subscription>,
}

impl PeoplePickerStore {
    const QUERY_SCALAR_LIMIT: usize = 128;
    const UNMOUNT_FALLBACK_MILLISECONDS: u64 = 900;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            is_presented: false,
            mounts: false,
            host_context: PeoplePickerHostContext::Root,
            mode: PeoplePickerMode::SearchAll,
            limit: 0,
            title: String::new(),
            hint: None,
            excluded_ids: HashSet::new(),
            selected: Vec::new(),
            selected_ids: HashSet::new(),
            query: String::new(),
            initial_ids: HashSet::new(),
            on_done: None,
            session: 0,
            network_down: false,
            network_observation: None,
        }
    }

    pub fn is_presented(&self) -> bool {
        self.is_presented
    }

    pub fn mounts(&self) -> bool {
        self.mounts
    }

    pub fn host_context(&self) -> PeoplePickerHostContext {
        self.host_context
    }

    pub fn mode(&self) -> PeoplePickerMode {
        self.mode
    }

    pub fn limit(&self) -> i64 {
        self.limit
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    pub fn excluded_ids(&self) -> &HashSet<String> {
        &self.excluded_ids
    }

    pub fn selected(&self) -> &[PostCollaboratorUser] {
        &self.selected
    }

    pub fn selected_ids(&self) -> &HashSet<String> {
        &self.selected_ids
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn session(&self) -> u64 {
        self.session
    }

    pub fn is_dirty(&self) -> bool {
        self.selected_ids != self.initial_ids
    }

    pub fn can_select_more(&self) -> bool {
        self.limit <= 0 || (self.selected.len() as i64) < self.limit
    }

    /// reacts to is_presented changing
    fn set_is_presented(&mut self, value: bool, cx: &mut Context<Self>) {
        self.is_presented = value;
        GuideFacts::set(RiseGuideSignal::PeoplePickerOpen, value, cx);
    }

    /// reacts to selected changing
    fn set_selected(&mut self, value: Vec<PostCollaboratorUser>, cx: &mut Context<Self>) {
        self.selected = value;
        GuideFacts::set(RiseGuideSignal::PeoplePickerHasSelection, !self.selected.is_empty(), cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn present(
        &mut self,
        mode: PeoplePickerMode,
        limit: i64,
        preselected: Vec<PostCollaboratorUser>,
        excluded_ids: HashSet<String>,
        title: String,
        hint: Option<String>,
        host_context: PeoplePickerHostContext,
        on_done: impl FnOnce(Vec<PostCollaboratorUser>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        if self.mounts {
            return None;
        }
        self.session = self.session.wrapping_add(1);
        self.host_context = host_context;
        self.mode = mode;
        self.limit = limit;
        self.title = title;
        self.hint = hint;
        let selected: Vec<PostCollaboratorUser> =
            preselected.into_iter().filter(|user| !excluded_ids.contains(&user.id)).collect();
        self.excluded_ids = excluded_ids;
        self.selected_ids = selected.iter().map(|user| user.id.clone()).collect();
        self.set_selected(selected, cx);
        self.initial_ids = self.selected_ids.clone();
        self.on_done = Some(Box::new(on_done));
        self.query = String::new();
        self.mounts = true;
        self.set_is_presented(true, cx);
        self.observe_network(cx);
        cx.notify();
        Some(self.session)
    }

    fn observe_network(&mut self, cx: &mut Context<Self>) {
        if self.network_observation.is_some() {
            return;
        }
        let manager = SaiWs::manager(cx);
        self.network_down = manager.read(cx).is_network_path_down();
        self.network_observation = Some(cx.observe(&manager, |store, manager, cx| {
            let down = manager.read(cx).is_network_path_down();
            store.network_path_changed(down, cx);
        }));
    }

    fn network_path_changed(&mut self, down: bool, cx: &mut Context<Self>) {
        if down == self.network_down {
            return;
        }
        self.network_down = down;
        cx.notify();
        if down || !self.is_presented || self.mode != PeoplePickerMode::SearchAll {
            return;
        }
        let query = trimming_whitespaces_and_newlines(&self.query).to_owned();
        if query.is_empty() {
            return;
        }
        let has_page = SearchServicesStore::shared(cx)
            .update(cx, |service, cx| service.search_users_page(&query, cx))
            .is_some();
        if has_page {
            return;
        }
        SearchActionsStore::shared(cx).update(cx, |store, cx| {
            store.search_users_action(&query, None, true, true, cx);
        });
    }

    pub fn exclude(&mut self, ids: HashSet<String>, expected: u64, cx: &mut Context<Self>) {
        if expected != self.session || !self.is_presented {
            return;
        }
        let added: HashSet<String> = ids.difference(&self.excluded_ids).cloned().collect();
        if added.is_empty() {
            return;
        }
        self.excluded_ids.extend(added.iter().cloned());
        let selected: Vec<PostCollaboratorUser> =
            self.selected.iter().filter(|user| !added.contains(&user.id)).cloned().collect();
        self.set_selected(selected, cx);
        self.selected_ids.retain(|id| !added.contains(id));
        self.initial_ids.retain(|id| !added.contains(id));
        cx.notify();
    }

    pub fn set_presented(&mut self, presented: bool, cx: &mut Context<Self>) {
        if presented {
            return;
        }
        self.set_is_presented(false, cx);
        self.schedule_unmount_fallback(cx);
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.set_is_presented(false, cx);
        self.schedule_unmount_fallback(cx);
        cx.notify();
    }

    fn schedule_unmount_fallback(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(Self::UNMOUNT_FALLBACK_MILLISECONDS))
                .await;
            let _ = this.update(cx, |store, cx| {
                if store.is_presented || !store.mounts {
                    return;
                }
                store.sheet_did_dismiss(cx);
            });
        })
        .detach();
    }

    /// embedded page leaving the deck, no sheet animates out so it unmounts at once
    /// so a quick return can present again
    pub fn end_embedded(&mut self, cx: &mut Context<Self>) {
        if self.host_context != PeoplePickerHostContext::Embedded || !self.mounts {
            return;
        }
        self.set_is_presented(false, cx);
        self.sheet_did_dismiss(cx);
    }

    pub fn sheet_did_dismiss(&mut self, cx: &mut Context<Self>) {
        if self.is_presented {
            return;
        }
        self.mounts = false;
        self.on_done = None;
        self.set_selected(Vec::new(), cx);
        self.selected_ids = HashSet::new();
        self.initial_ids = HashSet::new();
        self.excluded_ids = HashSet::new();
        self.query = String::new();
        self.hint = None;
        self.title = String::new();
        self.limit = 0;
        cx.notify();
    }

    pub fn set_query(&mut self, value: &str, cx: &mut Context<Self>) {
        let limited: String = value.chars().take(Self::QUERY_SCALAR_LIMIT).collect();
        if self.query == limited {
            return;
        }
        self.query = limited;
        cx.notify();
    }

    pub fn is_selected(&self, id: &str) -> bool {
        self.selected_ids.contains(id)
    }

    pub fn toggle(&mut self, user: PostCollaboratorUser, cx: &mut Context<Self>) {
        if self.excluded_ids.contains(&user.id) {
            return;
        }
        if self.selected_ids.remove(&user.id) {
            let selected: Vec<PostCollaboratorUser> =
                self.selected.iter().filter(|candidate| candidate.id != user.id).cloned().collect();
            self.set_selected(selected, cx);
            let _ = selection_changed();
            cx.notify();
            return;
        }
        if !self.can_select_more() {
            Notifier::send(
                tr_plural("people_picker_limit_reached", self.limit),
                NotifierSendOptions {
                    r#type: NotifierType::Warning,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
            return;
        }
        self.selected_ids.insert(user.id.clone());
        let mut selected = std::mem::take(&mut self.selected);
        selected.push(user);
        self.set_selected(selected, cx);
        let _ = selection_changed();
        cx.notify();
    }

    pub fn remove(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.selected_ids.remove(id) {
            return;
        }
        let selected: Vec<PostCollaboratorUser> =
            self.selected.iter().filter(|user| user.id != id).cloned().collect();
        self.set_selected(selected, cx);
        cx.notify();
    }

    pub fn commit(&mut self, cx: &mut Context<Self>) {
        let done = self.on_done.take();
        let picked = self.selected.clone();
        self.set_is_presented(false, cx);
        self.schedule_unmount_fallback(cx);
        cx.notify();
        if let Some(done) = done {
            cx.defer(move |cx| done(picked, cx));
        }
    }
}

pub enum PeoplePickerMapping {}

impl PeoplePickerMapping {
    pub fn user(author: &AuthorInfo, is_friend: Option<bool>) -> Option<PostCollaboratorUser> {
        let raw = trimming_whitespaces_and_newlines(author.id.as_deref()?);
        if raw.is_empty() {
            return None;
        }
        let logo = trimming_whitespaces_and_newlines(&author.more.logo);
        Some(PostCollaboratorUser {
            id: raw.to_owned(),
            is_friend: is_friend.unwrap_or(author.is_friend),
            is_official: author.more.is_official,
            is_premium: author.more.is_premium,
            logo: logo.to_owned(),
            name: author.name.clone(),
            name_color: author.more.name_color.clone(),
            premium_badge_emoji: author.more.premium_badge_emoji.clone(),
            since_ms: 0,
            tag: author.tag.clone(),
        })
    }

    pub fn user_from_entry(entry: &UserListEntry) -> Option<PostCollaboratorUser> {
        Self::user(&entry.author, Some(entry.viewer_is_friend.unwrap_or(true)))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PeoplePickerHostContext {
    #[default]
    Root,
    MediaEditor,
    Embedded,
}

pub struct PeoplePickerHost {
    context: PeoplePickerHostContext,
    store: Entity<PeoplePickerStore>,
    _store_observation: Subscription,
}

impl PeoplePickerHost {
    pub fn new(context: PeoplePickerHostContext, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = PeoplePickerStore::shared(cx);
        let store_observation = cx.observe(&store, |_, _, cx| cx.notify());
        Self {
            context,
            store,
            _store_observation: store_observation,
        }
    }

    fn mounts(&self, cx: &App) -> bool {
        let store = self.store.read(cx);
        store.mounts() && store.host_context() == self.context
    }
}

impl Render for PeoplePickerHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mounts = self.mounts(cx);
        let root = div().absolute().top_0().left_0();
        if mounts {
            root.size_full().child(div().absolute().inset_0().child(PeoplePickerSheet::new()))
        } else {
            root.size_0().overflow_hidden()
        }
    }
}

#[derive(IntoElement)]
pub struct PeoplePickerSheet {
    embedded: bool,
}

impl PeoplePickerSheet {
    pub fn new() -> Self {
        Self { embedded: false }
    }

    pub fn embedded(mut self, embedded: bool) -> Self {
        self.embedded = embedded;
        self
    }
}

impl Default for PeoplePickerSheet {
    fn default() -> Self {
        Self::new()
    }
}

const SNAP_FRACTIONS: [f32; 2] = [0.62, 0.93];
const RECENT_PEOPLE_LIMIT: usize = 30;
const PAGINATION_THRESHOLD: usize = 8;
const SEARCH_DEBOUNCE_ID: &str = "peoplePickerSearch";
const SEARCH_DEBOUNCE_MAX_CACHE_DATA: i64 = 10;
const ESCAPE_KEY: &str = "escape";

/// one row of a lazy list, a person or the spinner asking for the next search page
#[derive(Clone)]
enum PeoplePickerRow {
    Person {
        user: PostCollaboratorUser,
        anchor: Option<RiseGuideAnchor>,
        prefetch: Option<PeoplePickerPrefetch>,
    },
    LoadMore {
        query: String,
        cursor: String,
    },
}

impl PeoplePickerRow {
    fn id(&self) -> String {
        match self {
            Self::Person { user, .. } => user.id.clone(),
            Self::LoadMore { .. } => LOAD_MORE_ROW_ID.to_owned(),
        }
    }
}

type LoadPage = Rc<dyn Fn(&mut Window, &mut App)>;

const LOAD_MORE_ROW_ID: &str = "people-picker-load-more";

#[derive(Clone)]
struct PeoplePickerPrefetch {
    options: SaiPaginationPrefetchOptions,
    load_page: LoadPage,
}

/// model of one lazy list
/// gpui keys rows by index so last frame row ids decide if rows were appended or its another list
struct PeoplePickerListModel {
    list: ListUi,
    ids: Vec<String>,
    anchor: Option<(u64, bool, Pixels)>,
}

impl PeoplePickerListModel {
    fn new(cx: &App) -> Self {
        Self {
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            ids: Vec::new(),
            anchor: None,
        }
    }

    fn sync(&mut self, ids: Vec<String>) {
        if ids == self.ids {
            return;
        }
        let kept = match self.ids.last() {
            Some(last) if last == LOAD_MORE_ROW_ID => self.ids.len() - 1,
            _ => self.ids.len(),
        };
        if kept > 0 && ids.starts_with(&self.ids[..kept]) {
            self.list.state().splice(kept..self.ids.len(), ids.len() - kept);
        } else if !self.ids.is_empty() && ids.len() > self.ids.len() && ids.starts_with(&self.ids) {
            self.list.appended(ids.len() - self.ids.len());
        } else {
            self.list.replaced(ids.len());
        }
        self.ids = ids;
    }
}

/// one friends page as rows read it
/// decoded once per cache revision and not every scroll frame or caret blink
struct PeoplePickerFriendsPage {
    users: Vec<PostCollaboratorUser>,
    has_more: bool,
    continuation: Option<UserListContinuation>,
}

#[derive(Default)]
struct PeoplePickerFriendsMemo {
    key: Option<(String, i64)>,
    page: Option<Rc<PeoplePickerFriendsPage>>,
}

#[derive(Clone)]
struct PeoplePickerParts {
    store: Entity<PeoplePickerStore>,
    user_service: Entity<UserServicesStore>,
    auth_service: Entity<AuthServicesStore>,
    chat_service: Entity<ChatServicesStore>,
    search_service: Entity<SearchServicesStore>,
    selected_sheet_fraction: Entity<f32>,
    search_active: Entity<bool>,
    list_model: Entity<PeoplePickerListModel>,
    friends_memo: Entity<PeoplePickerFriendsMemo>,
}

impl PeoplePickerParts {
    fn owner_id(&self, cx: &App) -> String {
        self.auth_service.read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default()
    }

    fn trimmed_query(&self, cx: &App) -> String {
        trimming_whitespaces_and_newlines(self.store.read(cx).query()).to_owned()
    }

    fn content(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let store = self.store.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg._100)
            .child(
                SheetHeaderBar::new("people-picker-header", self.store.read(cx).title().to_owned(), move |_, cx| {
                    store.update(cx, |store, cx| store.dismiss(cx));
                })
                .right_system_image(None),
            )
            .child(div().flex_1().min_h_0().child(self.picker_body(&theme, window, cx)))
            .into_any_element()
    }

    fn picker_body(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let mut column = div().size_full().flex().flex_col().bg(theme.bg._100).child(self.search_field(cx));
        if let Some(hint) = self.store.read(cx).hint() {
            column = column.child(
                div().w_full().px(px(16.0)).pt(px(8.0)).child(
                    MainText::new(hint.to_owned())
                        .px(12.0)
                        .tac(TextAlignT::Left)
                        .color(theme.text.secondary),
                ),
            );
        }
        if !self.store.read(cx).selected().is_empty() {
            column = column.child(self.selected_strip(theme, cx));
        }
        column
            .child(div().flex_1().min_h_0().child(self.results(theme, window, cx)))
            .into_any_element()
    }

    fn search_field(&self, cx: &App) -> AnyElement {
        let reader = self.store.clone();
        let writer = self.store.clone();
        let query = Binding::new(
            move |cx| reader.read(cx).query().to_owned(),
            move |value: String, _, cx| writer.update(cx, |store, cx| store.set_query(&value, cx)),
        );
        let search_parts = self.clone();
        let on_change = move |value: &str, _: &mut Window, cx: &mut App| search_parts.search(value, cx);
        let field = if *self.search_active.read(cx) {
            SearchFieldUi::new("people-picker-search", query)
                .placeholder(SharedString::from(tr("people_picker_search_placeholder")))
                .on_change(on_change)
                .uses_inline_text_field(true)
                .auto_focus(true)
                .on_focus_lost({
                    let active = self.search_active.clone();
                    move |_, cx| Self::end_search(&active, cx)
                })
        } else {
            let activate_parts = self.clone();
            SearchFieldUi::new("people-picker-search-rest", query)
                .placeholder(SharedString::from(tr("people_picker_search_placeholder")))
                .on_change(on_change)
                .on_activate(move |_, cx| activate_parts.activate_search(cx))
        };
        let escape_active = self.search_active.clone();
        let outside_active = self.search_active.clone();
        div()
            .px(px(12.0))
            .capture_key_down(move |event, _, cx| {
                if event.keystroke.key == ESCAPE_KEY && *escape_active.read(cx) {
                    Self::end_search(&escape_active, cx);
                    cx.stop_propagation();
                }
            })
            .on_mouse_down_out(move |_, _, cx| Self::end_search(&outside_active, cx))
            .child(field)
            .into_any_element()
    }

    fn end_search(search_active: &Entity<bool>, cx: &mut App) {
        search_active.update(cx, |active, cx| {
            if *active {
                *active = false;
                cx.notify();
            }
        });
    }

    fn selected_strip(&self, theme: &AppTheme, cx: &App) -> AnyElement {
        let mut chips = hstack(10.0).px(px(16.0)).py(px(2.0));
        for user in self.store.read(cx).selected() {
            chips = chips.child(self.selected_chip(user, theme));
        }
        div()
            .w_full()
            .pt(px(12.0))
            .flex()
            .flex_col()
            .items_start()
            .gap(px(6.0))
            .child(
                div().px(px(16.0)).child(
                    MainText::new(tr("people_picker_selected_section"))
                        .px(13.0)
                        .tac(TextAlignT::Left)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.secondary),
                ),
            )
            .child(div().id("people-picker-selected-strip").w_full().overflow_x_scroll().child(chips))
            .into_any_element()
    }

    fn selected_chip(&self, user: &PostCollaboratorUser, theme: &AppTheme) -> AnyElement {
        let store = self.store.clone();
        let id = user.id.clone();
        vstack(4.0)
            .items_center()
            .child(
                div()
                    .relative()
                    .child(
                        UserLogoUi::new(SharedString::from(format!("people-picker-chip-logo-{}", user.id)))
                            .source(SharedString::from(user.logo.clone()))
                            .name(SharedString::from(user.name.clone()))
                            .user_id(SharedString::from(user.id.clone()))
                            .size(46.0)
                            .show_story_ring(false)
                            .open_story_on_press(false),
                    )
                    .child(
                        div().absolute().top(px(-4.0)).right(px(-4.0)).child(
                            plain(SharedString::from(format!("people-picker-chip-remove-{}", user.id)))
                                .on_press(move |_, cx| store.update(cx, |store, cx| store.remove(&id, cx)))
                                .rounded_full()
                                .bg(theme.bg._100)
                                .child(symbol("xmark.circle.fill", 16.0, theme.text.primary)),
                        ),
                    ),
            )
            .child(
                div().w(px(58.0)).child(
                    MainText::new(display_name(user))
                        .px(11.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary)
                        .number_of_lines(1),
                ),
            )
            .into_any_element()
    }

    fn results(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        match self.store.read(cx).mode() {
            PeoplePickerMode::SearchAll => self.search_results_list(theme, window, cx),
            PeoplePickerMode::Friends => self.friends_list(theme, window, cx),
        }
    }

    fn search_results_list(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        if self.trimmed_query(cx).is_empty() {
            self.recent_people_list(theme, window, cx)
        } else {
            self.search_hits(theme, window, cx)
        }
    }

    fn recent_people_list(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let recents = self.recent_direct_chat_people(cx);
        if recents.is_empty() {
            return empty_state(theme, tr("people_picker_empty"));
        }
        let first_id = recents.first().map(|user| user.id.clone());
        let rows = recents
            .into_iter()
            .map(|user| {
                let anchor = (Some(&user.id) == first_id.as_ref()).then_some(RiseGuideAnchor::PeoplePickerFirstPerson);
                PeoplePickerRow::Person { user, anchor, prefetch: None }
            })
            .collect();
        self.lazy_list(rows, window, cx)
    }

    fn recent_direct_chat_people(&self, cx: &mut App) -> Vec<PostCollaboratorUser> {
        let sandbox = GuideSandbox::shared(cx);
        let sandbox = sandbox.read(cx);
        let mut seen: HashSet<String> = HashSet::new();
        let mut result: Vec<PostCollaboratorUser> = sandbox.map_picker_suggestions();
        result.extend(sandbox.composer_picker_suggestions(cx));
        for suggestion in &result {
            seen.insert(suggestion.id.clone());
        }
        let excluded = self.store.read(cx).excluded_ids();
        for chat in self.chat_service.read(cx).current_chat_infos(cx) {
            if chat.r#type != ChatType::Direct || chat.is_bot_chat() {
                continue;
            }
            let participant = &chat.participant;
            let id = trimming_whitespaces_and_newlines(&participant.id).to_owned();
            if id.is_empty() || participant.is_deleted || excluded.contains(&id) || !seen.insert(id.clone()) {
                continue;
            }
            let logo = trimming_whitespaces_and_newlines(&participant.more.logo).to_owned();
            result.push(PostCollaboratorUser {
                id,
                is_friend: false,
                is_official: participant.more.is_official,
                is_premium: participant.more.is_premium,
                logo,
                name: participant.name.clone(),
                name_color: participant.more.name_color.clone(),
                premium_badge_emoji: participant.more.premium_badge_emoji.clone(),
                since_ms: 0,
                tag: participant.tag.clone(),
            });
            if result.len() == RECENT_PEOPLE_LIMIT {
                break;
            }
        }
        result
    }

    fn search_hits(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let query = self.trimmed_query(cx);
        let page = self.search_service.update(cx, |service, cx| service.search_users_page(&query, cx));
        let authors = self.search_results(page.as_ref(), cx);
        let has_more = page.as_ref().is_some_and(|page| page.is_have_more);
        if authors.is_empty() && !has_more {
            if page.is_some() {
                return empty_state(theme, tr("people_picker_empty"));
            }
            return match self.search_failure_text(cx) {
                Some(text) => self.search_error_state(theme, text, query),
                None => loading_list(),
            };
        }
        let cache_key = SearchServicesStore::search_users_cache_key(&query);
        let relative_id = page.as_ref().and_then(|page| page.relative_id).map(|id| id.to_string());
        let item_count = authors.len();
        let mut rows: Vec<PeoplePickerRow> = Vec::with_capacity(item_count + 1);
        for (index, author) in authors.iter().enumerate() {
            let Some(user) = PeoplePickerMapping::user(author, None) else {
                continue;
            };
            let load_query = query.clone();
            let load_page: LoadPage = Rc::new(move |_, cx| {
                let query = load_query.clone();
                SearchInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_plain_users(&query, cx));
            });
            let options =
                SaiPaginationPrefetchOptions::new(index, item_count, cache_key.clone(), relative_id.clone(), has_more)
                    .threshold(PAGINATION_THRESHOLD);
            rows.push(PeoplePickerRow::Person {
                user,
                anchor: None,
                prefetch: Some(PeoplePickerPrefetch { options, load_page }),
            });
        }
        if has_more {
            rows.push(PeoplePickerRow::LoadMore { query, cursor: relative_id.unwrap_or_default() });
        }
        self.lazy_list(rows, window, cx)
    }

    fn search_failure_text(&self, cx: &mut App) -> Option<String> {
        if RiseTransportStatus::is_network_path_down(cx) {
            return Some(tr("error_connection_lost"));
        }
        let actions = SearchActionsStore::shared(cx);
        if actions.read(cx).search_users_sai.status() != SaiWsSliceStatus::Rejected {
            return None;
        }
        Some(
            self.search_service
                .read(cx)
                .plain_user_last_error()
                .map(str::to_owned)
                .unwrap_or_else(|| tr("error_fetching_data")),
        )
    }

    fn search_error_state(&self, theme: &AppTheme, text: String, query: String) -> AnyElement {
        div()
            .size_full()
            .px(px(24.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .items_center()
            .justify_center()
            .child(MainText::new(text).px(15.0).tac(TextAlignT::Center).color(theme.text.secondary))
            .child(
                plain("people-picker-retry")
                    .on_press(move |_, cx| {
                        SearchActionsStore::shared(cx).update(cx, |store, cx| {
                            store.search_users_action(&query, None, true, true, cx);
                        });
                    })
                    .child(
                        MainText::new(tr("common_retry"))
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.primary._100),
                    ),
            )
            .into_any_element()
    }

    fn friends_page(&self, cache_key: &str, owner: &str, query: &str, cx: &mut App) -> Rc<PeoplePickerFriendsPage> {
        let key = (cache_key.to_owned(), SaiCacheRevisionTracker::observed_revision(cache_key, cx));
        let _ = self.user_service.read(cx);
        let memo = self.friends_memo.read(cx);
        if let Some(page) = memo.page.as_ref().filter(|_| memo.key.as_ref() == Some(&key)) {
            return page.clone();
        }
        let page = self.user_service.update(cx, |user_service, cx| {
            let users = user_service
                .list_entries(UserListKind::Friends, owner, Some(query), cx)
                .iter()
                .filter_map(PeoplePickerMapping::user_from_entry)
                .collect();
            let has_more = user_service.list_has_more(UserListKind::Friends, owner, Some(query), cx);
            let continuation = user_service.list_continuation(UserListKind::Friends, owner, Some(query), cx);
            Rc::new(PeoplePickerFriendsPage { users, has_more, continuation })
        });
        let stored = page.clone();
        self.friends_memo.update(cx, |memo, _| {
            memo.key = Some(key);
            memo.page = Some(stored);
        });
        page
    }

    fn friends_list(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let owner = self.owner_id(cx);
        let query = self.store.read(cx).query().to_owned();
        let excluded = self.store.read(cx).excluded_ids().clone();
        let sandbox = GuideSandbox::shared(cx);
        let cache_key = UserServicesStore::list_cache_key(UserListKind::Friends, &owner, Some(&query));
        let page = self.friends_page(&cache_key, &owner, &query, cx);
        let has_more = page.has_more;
        let continuation = page.continuation.clone();
        let entries: Vec<&PostCollaboratorUser> =
            page.users.iter().filter(|user| !excluded.contains(&user.id)).collect();
        let invented = if query.is_empty() {
            sandbox.read(cx).composer_picker_suggestions(cx)
        } else {
            Vec::new()
        };

        if entries.is_empty() && invented.is_empty() {
            let status = self.user_service.update(cx, |user_service, cx| {
                user_service.list_render_status(UserListKind::Friends, &owner, Some(&query), cx)
            });
            return if UserRelationshipSearchContract::is_short_nonempty_query(Some(&query)) {
                empty_state(theme, tr("relationship_search_minimum_hint"))
            } else if status == SaiWsSliceStatus::Pending
                || (status == SaiWsSliceStatus::Idle && !self.trimmed_query(cx).is_empty())
            {
                loading_list()
            } else {
                empty_state(theme, tr("people_picker_empty"))
            };
        }

        let relative_id = continuation.map(|continuation| continuation.pagination_token());
        let item_count = entries.len();
        let mut rows: Vec<PeoplePickerRow> = invented
            .into_iter()
            .map(|user| PeoplePickerRow::Person {
                user,
                anchor: Some(RiseGuideAnchor::PeoplePickerFirstPerson),
                prefetch: None,
            })
            .collect();
        let load_page: LoadPage = Rc::new(move |_, cx| {
            UserInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.load_more_list(UserListKind::Friends, &owner, Some(&query), cx);
            });
        });
        for (index, user) in entries.into_iter().enumerate() {
            let options =
                SaiPaginationPrefetchOptions::new(index, item_count, cache_key.clone(), relative_id.clone(), has_more)
                    .threshold(PAGINATION_THRESHOLD);
            rows.push(PeoplePickerRow::Person {
                user: user.clone(),
                anchor: None,
                prefetch: Some(PeoplePickerPrefetch { options, load_page: load_page.clone() }),
            });
        }
        self.lazy_list(rows, window, cx)
    }

    /// scroll view with a lazy stack of rows, 6 top padding
    fn lazy_list(&self, rows: Vec<PeoplePickerRow>, _window: &mut Window, cx: &mut App) -> AnyElement {
        let ids: Vec<String> = rows.iter().map(PeoplePickerRow::id).collect();
        self.list_model.update(cx, |model, _| model.sync(ids));
        let rows = Rc::new(rows);
        let parts = self.clone();
        let element = self.list_model.read(cx).list.element(move |index, window, cx| match rows.get(index) {
            Some(row) => parts.list_row(row, window, cx),
            None => div().into_any_element(),
        });
        let store = self.store.read(cx);
        let (session, has_strip) = (store.session(), !store.selected().is_empty());
        let model = self.list_model.clone();
        let keep_rows = canvas(
            move |bounds, _, cx| {
                model.update(cx, |model, _| {
                    let top = bounds.origin.y;
                    if let Some((last_session, had_strip, last_top)) = model.anchor
                        && last_session == session
                        && had_strip != has_strip
                    {
                        model.list.state().scroll_by(top - last_top);
                    }
                    model.anchor = Some((session, has_strip, top));
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        div().relative().size_full().pt(px(6.0)).child(keep_rows).child(element).into_any_element()
    }

    fn list_row(&self, row: &PeoplePickerRow, window: &mut Window, cx: &mut App) -> AnyElement {
        match row {
            PeoplePickerRow::Person { user, anchor, prefetch } => {
                let theme = rise_ui::theme(cx).clone();
                let element = self.row(user, &theme, window, cx).guide_anchor(*anchor);
                match prefetch {
                    Some(prefetch) => {
                        let load_page = prefetch.load_page.clone();
                        element
                            .sai_pagination_prefetch(
                                SharedString::from(format!("people-picker-prefetch-{}", user.id)),
                                prefetch.options.clone(),
                                move |window, cx| load_page(window, cx),
                            )
                            .into_any_element()
                    }
                    None => element.into_any_element(),
                }
            }
            PeoplePickerRow::LoadMore { query, cursor } => {
                let query = query.clone();
                div()
                    .w_full()
                    .py(px(16.0))
                    .flex()
                    .justify_center()
                    .child(progress_view())
                    .on_appear(SharedString::from(format!("people-picker-load-more-{query}-{cursor}")), move |_, cx| {
                        SearchInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.load_more_plain_users(&query, cx));
                    })
                    .into_any_element()
            }
        }
    }

    fn row(&self, user: &PostCollaboratorUser, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let store = self.store.read(cx);
        let excluded = store.excluded_ids().contains(&user.id);
        let is_on = store.is_selected(&user.id);

        let mut names = hstack(4.0).child(
            MainText::new(display_name(user))
                .px(15.0)
                .tac(TextAlignT::Left)
                .font_weight(FontWeight::MEDIUM)
                .number_of_lines(1),
        );
        if user.is_official {
            names = names.child(OfficialBadge::new().size(px(14.0)));
        }
        if user.is_premium {
            names = names.child(
                PremiumBadge::new()
                    .user_id(Some(SharedString::from(user.id.clone())))
                    .emoji(user.premium_badge_emoji.clone().map(SharedString::from))
                    .size(14.0),
            );
        }
        let mut labels = vstack(2.0).items_start().min_w_0().child(names);
        if !user.tag.is_empty() {
            labels = labels.child(
                MainText::new(format!("@{}", user.tag))
                    .px(12.0)
                    .tac(TextAlignT::Left)
                    .color(theme.text.secondary)
                    .number_of_lines(1),
            );
        }

        let check_target = if is_on { theme.primary._100 } else { theme.text.secondary.opacity(0.35) };
        let check_color = Self::check_color(&user.id, check_target, window, cx);
        let toggled_store = self.store.clone();
        let toggled_user = user.clone();
        plain(SharedString::from(format!("people-picker-row-{}", user.id)))
            .disabled(excluded)
            .on_press(move |_, cx| {
                let user = toggled_user.clone();
                toggled_store.update(cx, |store, cx| store.toggle(user, cx));
            })
            .w_full()
            .opacity(if excluded { 0.42 } else { 1.0 })
            .child(
                hstack(12.0)
                    .w_full()
                    .px(px(16.0))
                    .py(px(8.0))
                    .child(
                        UserLogoUi::new(SharedString::from(format!("people-picker-row-logo-{}", user.id)))
                            .source(SharedString::from(user.logo.clone()))
                            .name(SharedString::from(user.name.clone()))
                            .user_id(SharedString::from(user.id.clone()))
                            .size(40.0)
                            .show_story_ring(false)
                            .open_story_on_press(false),
                    )
                    .child(labels)
                    .child(spacer_min_w(0.0))
                    .child(symbol(if is_on { "checkmark.circle.fill" } else { "circle" }, 22.0, check_color)),
            )
            .into_any_element()
    }

    /// animates the check mark color over 0.15 s ease out
    fn check_color(user_id: &str, target: Hsla, window: &mut Window, cx: &mut App) -> Hsla {
        let now = RiseClock::monotonic();
        let state = window.use_keyed_state(
            SharedString::from(format!("people-picker-check-{user_id}")),
            cx,
            |_, _| Animated::new(target),
        );
        state.update(cx, |animated, cx| {
            animated.set_in(target, ease_out(0.15), now, cx);
            animated.frame_value(now, window)
        })
    }

    /// page is read by this box own query and not the global search field
    /// so one box never overwrites the other
    fn search_results(&self, page: Option<&SearchUsersListDTO>, cx: &App) -> Vec<AuthorInfo> {
        let excluded = self.store.read(cx).excluded_ids();
        page.map(|page| page.authors())
            .unwrap_or_default()
            .into_iter()
            .filter(|author| {
                PeoplePickerMapping::user(author, None).is_some_and(|user| !excluded.contains(&user.id))
            })
            .collect()
    }

    fn activate_search(&self, cx: &mut App) {
        self.selected_sheet_fraction.update(cx, |fraction, cx| {
            *fraction = SNAP_FRACTIONS[1];
            cx.notify();
        });
        self.search_active.update(cx, |active, cx| {
            *active = true;
            cx.notify();
        });
    }

    fn search(&self, value: &str, cx: &mut App) {
        let trimmed = trimming_whitespaces_and_newlines(value).to_owned();
        match self.store.read(cx).mode() {
            PeoplePickerMode::Friends => {
                let owner = self.owner_id(cx);
                let value = value.to_owned();
                UserInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.apply_list_search(UserListKind::Friends, &owner, &value, cx);
                });
            }
            PeoplePickerMode::SearchAll => {
                if trimmed.is_empty() {
                    return;
                }
                SaiDebouncer::debounced_action_delay_milliseconds(
                    SEARCH_DEBOUNCE_ID,
                    Box::new(move |cx: &mut App| {
                        SearchActionsStore::shared(cx).update(cx, |store, cx| {
                            store.search_users_action(&trimmed, None, true, true, cx);
                        });
                    }),
                    RiseTimingCanon::debounce("global_search").client_debounce_ms,
                    SEARCH_DEBOUNCE_ID,
                    SEARCH_DEBOUNCE_MAX_CACHE_DATA,
                    None,
                    cx,
                );
            }
        }
    }

    fn footer(&self, cx: &App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let store = self.store.clone();
        div()
            .w_full()
            .px(px(16.0))
            .py(px(10.0))
            .child(
                ButtonUi::new("people-picker-done")
                    .disabled(!self.store.read(cx).is_dirty())
                    .background_color(theme.primary._100)
                    .height(48.0)
                    .loader_color(theme.content_on(theme.primary._100))
                    .on_press(move |_, cx| store.update(cx, |store, cx| store.commit(cx)))
                    .child(
                        MainText::new(tr("people_picker_done"))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.content_on(theme.primary._100)),
                    )
                    .guide_anchor(RiseGuideAnchor::PeoplePickerDone),
            )
            .into_any_element()
    }
}

impl RenderOnce for PeoplePickerSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let store = PeoplePickerStore::shared(cx);
        let parts = PeoplePickerParts {
            store: store.clone(),
            user_service: UserServicesStore::shared(cx),
            auth_service: AuthServicesStore::shared(cx),
            chat_service: ChatServicesStore::shared(cx),
            search_service: SearchServicesStore::shared(cx),
            selected_sheet_fraction: window.use_keyed_state("people-picker-fraction", cx, |_, _| SNAP_FRACTIONS[0]),
            search_active: window.use_keyed_state("people-picker-search-active", cx, |_, _| false),
            list_model: window.use_keyed_state("people-picker-list", cx, |_, cx| PeoplePickerListModel::new(cx)),
            friends_memo: window.use_keyed_state("people-picker-friends-memo", cx, |_, _| {
                PeoplePickerFriendsMemo::default()
            }),
        };

        let appear_parts = parts.clone();
        let on_appear = move |_: &mut Window, cx: &mut App| {
            let (mode, query) = {
                let store = appear_parts.store.read(cx);
                (store.mode(), store.query().to_owned())
            };
            if mode == PeoplePickerMode::Friends {
                let owner = appear_parts.owner_id(cx);
                UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.ensure_list_loaded(UserListKind::Friends, &owner, Some(&query), cx);
                });
            }
        };

        if self.embedded {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .child(div().flex_1().min_h_0().child(parts.picker_body(&theme, window, cx)))
                .child(parts.footer(cx))
                .on_appear("people-picker-appear", on_appear)
                .into_any_element();
        }

        let reader = store.clone();
        let writer = store.clone();
        let presented = Binding::new(
            move |cx| reader.read(cx).is_presented(),
            move |presented, _, cx| writer.update(cx, |store, cx| store.set_presented(presented, cx)),
        );
        let fraction = Binding::entity(
            parts.selected_sheet_fraction.clone(),
            |fraction: &f32| *fraction,
            |fraction: &mut f32, value: f32, _: &mut Context<f32>| *fraction = value,
        );
        let content_parts = parts.clone();
        let footer_parts = parts;
        let did_dismiss_store = store;
        BottomSheetUi::new(
            "people-picker-sheet",
            presented,
            move |window, cx| content_parts.content(window, cx),
            move |_, cx| footer_parts.footer(cx),
        )
        .snap_fractions(SNAP_FRACTIONS.to_vec())
        .selected_snap_fraction(Some(fraction))
        .experimental_bottom_sheet(true)
        .dismisses_keyboard_before_backdrop_dismiss(true)
        .full_page_shows_drag_indicator(true)
        .footer_shows_top_border(true)
        .content_scrollable(true)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
        .on_did_dismiss(move |_, cx| did_dismiss_store.update(cx, |store, cx| store.sheet_did_dismiss(cx)))
        .on_appear("people-picker-appear", on_appear)
        .into_any_element()
    }
}

fn display_name(user: &PostCollaboratorUser) -> String {
    if user.name.is_empty() { format!("@{}", user.tag) } else { user.name.clone() }
}

fn loading_list() -> AnyElement {
    scroll_view("people-picker-loading")
        .scroll_disabled(true)
        .shows_indicators(false)
        .child(UserListSkeleton::new().rows(7))
        .into_any_element()
}

fn empty_state(theme: &AppTheme, text: String) -> AnyElement {
    div()
        .size_full()
        .px(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .child(MainText::new(text).px(15.0).tac(TextAlignT::Center).color(theme.text.secondary))
        .into_any_element()
}

/// system symbol at a point size, an unmapped symbol keeps its box empty
fn symbol(name: &str, side: f32, color: Hsla) -> AnyElement {
    match IconUi::sized(name, px(side), color, name.ends_with(".fill")) {
        Some(icon) => icon.into_any_element(),
        None => div().size(px(side)).into_any_element(),
    }
}
