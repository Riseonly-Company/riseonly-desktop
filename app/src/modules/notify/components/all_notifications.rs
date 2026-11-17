use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyView, App, Context, ElementId, Empty, Entity, FontWeight, ListAlignment, ListOffset, Pixels, Point, RenderOnce,
    StyleRefinement, Subscription, Task, WeakEntity, Window,
};
use rise_canon::rise_service_text::music_service;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change};
use rise_ui::swiftui::prelude::*;
use serde_json::{Map, Value};

use crate::app::router::main_tab_chrome::main_tab_bar_underlap;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::push_notifications::push_deep_link_store::{PushDeepLink, PushDeepLinkStore};
use crate::core::push_notifications::push_notification_presentation::PushNotificationRouteParser;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::modules::notify::components::notify_row::NotifyRow;
use crate::modules::notify::components::notify_skeleton::NotifyListSkeleton;
use crate::modules::notify::engine::rise_notifications_presentation::{
    RiseNotificationRow, RiseNotificationsLoadState, RiseNotificationsSection, RiseNotificationsSnapshot,
};
use crate::modules::notify::stores::notify_actions::notify_types::{NotificationItem, NotificationKind};
use crate::modules::notify::stores::notify_interactions::notify_interactions::NotifyInteractionsStore;
use crate::modules::notify::stores::notify_services::notify_services::NotifyServicesStore;
use crate::modules::post::pages::posts::post_detail::profile_post_list_body::splice_list_rows;

#[derive(IntoElement)]
pub struct AllNotifications {
    is_active_page: bool,
}

impl AllNotifications {
    pub fn new(is_active_page: bool) -> Self {
        Self { is_active_page }
    }
}

impl RenderOnce for AllNotifications {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        NotificationsList { tab: NotificationsTab::All, is_active_page: self.is_active_page }.on_appear(
            "all-notifications-task",
            |_, cx| {
                NotifyInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_notifications_loaded(cx));
            },
        )
    }
}

#[derive(IntoElement)]
pub struct CommentNotifications {
    is_active_page: bool,
}

impl CommentNotifications {
    pub fn new(is_active_page: bool) -> Self {
        Self { is_active_page }
    }

    fn drain_if_active(is_active_page: bool, comment_row_count: i64, cx: &mut App) {
        if !is_active_page {
            return;
        }
        NotifyInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.drain_notifications_when_filtered_empty(comment_row_count, cx));
    }
}

impl RenderOnce for CommentNotifications {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_active_page = self.is_active_page;
        let service = NotifyServicesStore::shared(cx);
        let snapshot = service.read(cx).notifications_snapshot(cx);
        let revision = snapshot.revision;
        let comment_row_count = snapshot.comment_row_count;
        on_change("comment-notifications-revision", revision, true, window, cx, move |_, _, _, cx| {
            Self::drain_if_active(is_active_page, comment_row_count, cx);
        });
        on_change("comment-notifications-active", is_active_page, false, window, cx, move |_, _, _, cx| {
            Self::drain_if_active(is_active_page, comment_row_count, cx);
        });
        NotificationsList { tab: NotificationsTab::Comments, is_active_page }.on_appear(
            "comment-notifications-task",
            |_, cx| {
                NotifyInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_notifications_loaded(cx));
            },
        )
    }
}

#[derive(Clone, Copy, PartialEq)]
enum NotificationsTab {
    All,
    Comments,
}

impl NotificationsTab {
    fn scope(self) -> &'static str {
        match self {
            Self::All => "all-notifications",
            Self::Comments => "comment-notifications",
        }
    }

    fn sections(self, snapshot: &RiseNotificationsSnapshot) -> &[RiseNotificationsSection] {
        match self {
            Self::All => &snapshot.sections,
            Self::Comments => &snapshot.comment_sections,
        }
    }

    fn row_count(self, snapshot: &RiseNotificationsSnapshot) -> i64 {
        match self {
            Self::All => snapshot.row_count,
            Self::Comments => snapshot.comment_row_count,
        }
    }

    fn empty_text(self) -> String {
        match self {
            Self::All => tr("notifications_empty"),
            Self::Comments => tr("notifications_comments_empty"),
        }
    }
}

/// layout only over rows the engine prepared
/// no decode or date math or string lookup here
#[derive(IntoElement)]
struct NotificationsList {
    tab: NotificationsTab,
    is_active_page: bool,
}

impl NotificationsList {
    /// cache first
    /// any rows mean fulfilled so a refresh never drops back to the skeleton
    fn render_status(row_count: i64, load_state: RiseNotificationsLoadState) -> SaiWsSliceStatus {
        if row_count > 0 {
            return SaiWsSliceStatus::Fulfilled;
        }
        match load_state {
            RiseNotificationsLoadState::Idle | RiseNotificationsLoadState::Loading => SaiWsSliceStatus::Pending,
            RiseNotificationsLoadState::Loaded => SaiWsSliceStatus::Fulfilled,
            RiseNotificationsLoadState::Failed => SaiWsSliceStatus::Rejected,
        }
    }
}

impl RenderOnce for NotificationsList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let skeleton_background = rise_ui::theme(cx).bg._100;
        let tab = self.tab;
        let scope = tab.scope();
        let service = NotifyServicesStore::shared(cx);
        let footer_height = main_tab_bar_underlap(cx).max(32.0);
        let rows = window.use_keyed_state(
            ElementId::Name(format!("{scope}-rows").into()),
            cx,
            move |_, cx: &mut Context<NotificationsRows>| NotificationsRows::new(scope, cx),
        );

        let shared: &App = cx;
        let snapshot = service.read(shared).notifications_snapshot(shared);
        let row_count = tab.row_count(snapshot);
        let load_state = snapshot.load_state;
        let key = NotificationsRowsKey {
            row_count,
            has_more: snapshot.has_more,
            is_loading_more: snapshot.is_loading_more,
            has_newer: snapshot.has_newer,
            is_loading_newer: snapshot.is_loading_newer,
            head_generation: snapshot.head_generation,
            revision: snapshot.revision,
            footer_height,
            is_active_page: self.is_active_page,
        };
        let stale_sections = rows
            .read(shared)
            .differs_from(&key, tab.sections(snapshot))
            .then(|| tab.sections(snapshot).to_vec());
        if let Some(sections) = stale_sections {
            rows.update(cx, |rows, cx| rows.sync(key, sections, cx));
        }

        let rows = AnyView::from(rows).cached(StyleRefinement::default().size_full());
        SaiDataRender::new(Self::render_status(row_count, load_state), move |_, _| rows)
            .id(ElementId::Name(format!("{scope}-data").into()))
            .is_empty(row_count == 0)
            .no_data_text(tab.empty_text())
            .error_text(tr("notifications_load_error"))
            .need_pending(true)
            .message_height_percent(40.0)
            .no_data_height_percent(30.0)
            .is_skeleton(false)
            .sai_data_render_loading(move |_, _| {
                div().size_full().overflow_hidden().bg(skeleton_background).child(NotifyListSkeleton::new())
            })
    }
}

#[derive(Clone, Copy, PartialEq)]
struct NotificationsRowsKey {
    row_count: i64,
    has_more: bool,
    is_loading_more: bool,
    has_newer: bool,
    is_loading_newer: bool,
    head_generation: u64,
    revision: u64,
    footer_height: f32,
    is_active_page: bool,
}

impl NotificationsRowsKey {
    fn initial(is_active_page: bool) -> Self {
        Self {
            row_count: 0,
            has_more: false,
            is_loading_more: false,
            has_newer: false,
            is_loading_newer: false,
            head_generation: 0,
            revision: 0,
            footer_height: 0.0,
            is_active_page,
        }
    }
}

/// one flat list position, gpui list has rows not sections
enum NotificationsListRow {
    Header,
    SectionHeader(String),
    Row {
        row: Rc<RiseNotificationRow>,
        view: Entity<NotificationRowView>,
        index: i64,
    },
    Footer,
}

struct NotificationsRows {
    scope: &'static str,
    list: ListUi,
    key: Option<NotificationsRowsKey>,
    sections: Vec<RiseNotificationsSection>,
    rows: Rc<Vec<NotificationsListRow>>,
    row_views: HashMap<String, Entity<NotificationRowView>>,
    rendered_rows: Vec<String>,
    footer_is_visible: bool,
    header_is_visible: bool,
    _connection: Subscription,
}

impl NotificationsRows {
    fn new(scope: &'static str, cx: &mut Context<Self>) -> Self {
        let list = ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top);
        let manager = SaiWebSocketManager::shared(cx);
        let connection = cx.observe(&manager, |_, _, cx| cx.notify());
        Self {
            scope,
            list,
            key: None,
            sections: Vec::new(),
            rows: Rc::new(Vec::new()),
            row_views: HashMap::new(),
            rendered_rows: Vec::new(),
            footer_is_visible: false,
            header_is_visible: false,
            _connection: connection,
        }
    }

    fn differs_from(&self, key: &NotificationsRowsKey, sections: &[RiseNotificationsSection]) -> bool {
        self.key.as_ref() != Some(key) || self.sections != sections
    }

    fn sync(&mut self, key: NotificationsRowsKey, sections: Vec<RiseNotificationsSection>, cx: &mut Context<Self>) {
        let prior = self.key.unwrap_or_else(|| NotificationsRowsKey::initial(key.is_active_page));
        let is_first = self.key.is_none();
        let rows_changed = self.sections != sections;
        let mut rows = Vec::new();
        let mut row_ids = Vec::new();
        if key.has_newer {
            rows.push(NotificationsListRow::Header);
            row_ids.push("list-header".to_owned());
        }
        let mut index = 0;
        let mut first_row_position = None;
        let mut row_views = HashMap::new();
        let mut redraws_kept_row = false;
        let list = cx.weak_entity();
        for section in &sections {
            rows.push(NotificationsListRow::SectionHeader(section.title.clone()));
            row_ids.push(format!("section-{}", section.id()));
            for row in &section.rows {
                first_row_position.get_or_insert(rows.len());
                let row = Rc::new(row.clone());
                let view = match self.row_views.remove(&row.id) {
                    Some(view) => {
                        redraws_kept_row |= view.update(cx, |view, cx| view.set_row(row.clone(), cx));
                        view
                    }
                    None => {
                        let (scope, row, list) = (self.scope, row.clone(), list.clone());
                        cx.new(move |_| NotificationRowView { scope, row, height: None, list })
                    }
                };
                row_views.insert(row.id.clone(), view.clone());
                row_ids.push(format!("row-{}", row.id));
                rows.push(NotificationsListRow::Row { row, view, index });
                index += 1;
            }
        }
        rows.push(NotificationsListRow::Footer);
        row_ids.push("list-footer".to_owned());

        let top = self.list.state().logical_scroll_top();
        let rests_at_top = !is_first && top.item_ix == 0 && top.offset_in_item <= px(0.0);
        splice_list_rows(self.list.state(), &mut self.rendered_rows, &row_ids);
        if rows_changed && redraws_kept_row {
            self.list.state().remeasure();
        }
        let new_head_position =
            first_row_position.filter(|_| !is_first && prior.head_generation != key.head_generation);
        if let Some(item_ix) = new_head_position {
            self.list.state().scroll_to(ListOffset { item_ix, offset_in_item: px(0.0) });
        } else if rests_at_top {
            self.list.state().scroll_to(ListOffset { item_ix: 0, offset_in_item: px(0.0) });
        }
        self.key = Some(key);
        self.sections = sections;
        self.rows = Rc::new(rows);
        self.row_views = row_views;
        if !is_first {
            cx.notify();
        }
    }

    fn scoped_id(scope: &str, suffix: &str) -> ElementId {
        ElementId::Name(format!("{scope}-{suffix}").into())
    }

    fn request_next_page(rows: &Entity<Self>, cx: &mut App) {
        let Some(key) = rows.read(cx).key else {
            return;
        };
        if !key.is_active_page || !key.has_more || key.row_count <= 0 {
            return;
        }
        NotifyInteractionsStore::shared(cx).update(cx, |store, cx| store.notifications_footer_did_appear(cx));
    }

    fn request_newer_page(rows: &Entity<Self>, cx: &mut App) {
        let Some(key) = rows.read(cx).key else {
            return;
        };
        if !key.is_active_page || !key.has_newer {
            return;
        }
        NotifyInteractionsStore::shared(cx).update(cx, |store, cx| store.notifications_header_did_appear(cx));
    }

    fn list_header(rows: &Entity<Self>, scope: &str, is_loading_newer: bool, theme: &AppTheme) -> AnyElement {
        let appeared = rows.clone();
        let disappeared = rows.clone();
        div()
            .w_full()
            .h(px(44.0))
            .flex()
            .items_center()
            .justify_center()
            .when(is_loading_newer, |slot| slot.child(Self::loader(theme)))
            .on_appear(Self::scoped_id(scope, "header-appear"), move |_, cx| {
                appeared.update(cx, |rows, _| rows.header_is_visible = true);
                Self::request_newer_page(&appeared, cx);
            })
            .on_disappear(Self::scoped_id(scope, "header-disappear"), move |_, cx| {
                disappeared.update(cx, |rows, _| rows.header_is_visible = false);
            })
            .into_any_element()
    }

    fn list_footer(rows: &Entity<Self>, scope: &str, key: NotificationsRowsKey, theme: &AppTheme) -> AnyElement {
        let appeared = rows.clone();
        let disappeared = rows.clone();
        vstack(0.0)
            .w_full()
            .when(key.has_more, |footer| {
                footer.child(
                    div()
                        .w_full()
                        .h(px(44.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(key.is_loading_more, |slot| slot.child(Self::loader(theme))),
                )
            })
            .child(div().h(px(key.footer_height)))
            .on_appear(Self::scoped_id(scope, "footer-appear"), move |_, cx| {
                appeared.update(cx, |rows, _| rows.footer_is_visible = true);
                Self::request_next_page(&appeared, cx);
            })
            .on_disappear(Self::scoped_id(scope, "footer-disappear"), move |_, cx| {
                disappeared.update(cx, |rows, _| rows.footer_is_visible = false);
            })
            .into_any_element()
    }

    fn loader(theme: &AppTheme) -> LoaderUi {
        LoaderUi::new().size(LoaderUiSize::Small).color(Some(theme.text.secondary))
    }

    fn section_header(title: &str, theme: &AppTheme) -> AnyElement {
        div()
            .w_full()
            .px(px(14.0))
            .pt(px(16.0))
            .pb(px(6.0))
            .child(
                MainText::new(title.to_owned())
                    .px(13.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.secondary),
            )
            .into_any_element()
    }
}

impl Render for NotificationsRows {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let scope = self.scope;
        let this = cx.entity();
        let key = self.key.unwrap_or_else(|| NotificationsRowsKey::initial(false));
        let is_connected = SaiWebSocketManager::shared(cx).read(cx).is_connected();

        let revision_rows = this.clone();
        on_change(Self::scoped_id(scope, "revision"), key.revision, false, window, cx, move |_, _, _, cx| {
            if revision_rows.read(cx).footer_is_visible {
                Self::request_next_page(&revision_rows, cx);
            }
        });
        let connected_rows = this.clone();
        on_change(Self::scoped_id(scope, "connected"), is_connected, false, window, cx, move |_, connected, _, cx| {
            if *connected && connected_rows.read(cx).footer_is_visible {
                Self::request_next_page(&connected_rows, cx);
            }
        });
        let active_rows = this.clone();
        on_change(Self::scoped_id(scope, "active"), key.is_active_page, false, window, cx, move |_, active, _, cx| {
            if !*active {
                return;
            }
            if active_rows.read(cx).header_is_visible {
                Self::request_newer_page(&active_rows, cx);
            }
            if active_rows.read(cx).footer_is_visible {
                Self::request_next_page(&active_rows, cx);
            }
        });

        let rows = self.rows.clone();
        let row_theme = theme.clone();
        let list = self.list.refreshable(
            Self::scoped_id(scope, "list"),
            |_, cx| {
                NotifyInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_notifications(cx));
                Task::ready(())
            },
            move |position, _, cx| {
                let Some(entry) = rows.get(position) else {
                    return Empty.into_any_element();
                };
                match entry {
                    NotificationsListRow::Header => {
                        Self::list_header(&this, scope, key.is_loading_newer, &row_theme)
                    }
                    NotificationsListRow::SectionHeader(title) => Self::section_header(title, &row_theme),
                    NotificationsListRow::Row { row, view, index } => {
                        let row_id = row.id.clone();
                        let index = *index;
                        NotificationRowView::element(view, cx)
                            .on_appear(Self::scoped_id(scope, &format!("row-appear-{row_id}")), move |_, cx| {
                                if !key.is_active_page {
                                    return;
                                }
                                NotifyInteractionsStore::shared(cx).update(cx, |store, cx| {
                                    store.notification_row_did_appear(&row_id, index, key.row_count, cx);
                                });
                            })
                            .into_any_element()
                    }
                    NotificationsListRow::Footer => Self::list_footer(&this, scope, key, &row_theme),
                }
            },
        );

        div().size_full().bg(theme.bg._100).child(list)
    }
}

/// one notification row as its own cached view
/// a wheel step replays the moved rows instead of laying out every avatar stack again
struct NotificationRowView {
    scope: &'static str,
    row: Rc<RiseNotificationRow>,
    height: Option<Pixels>,
    list: WeakEntity<NotificationsRows>,
}

impl NotificationRowView {
    fn element(view: &Entity<Self>, cx: &App) -> AnyElement {
        let height = view.read(cx).height;
        let view = AnyView::from(view.clone());
        match height {
            Some(height) => {
                view.cached(StyleRefinement::default().w_full().h(height)).replay_when_moved().into_any_element()
            }
            None => view.into_any_element(),
        }
    }

    fn set_row(&mut self, row: Rc<RiseNotificationRow>, cx: &mut Context<Self>) -> bool {
        if self.row == row {
            return false;
        }
        let redraws = !Self::draws_same(&self.row, &row);
        self.row = row;
        if redraws {
            self.height = None;
            cx.notify();
        }
        redraws
    }

    fn draws_same(old: &RiseNotificationRow, new: &RiseNotificationRow) -> bool {
        old.leading == new.leading
            && old.leading_is_heading == new.leading_is_heading
            && old.text == new.text
            && old.time == new.time
            && old.is_read == new.is_read
            && old.badge_symbol == new.badge_symbol
            && old.item.kind == new.item.kind
            && old.accessibility_hint == new.accessibility_hint
            && old.facepile.len() == new.facepile.len()
            && old.facepile.iter().zip(&new.facepile).all(|(old, new)| {
                old.actor_id == new.actor_id
                    && old.actor_name == new.actor_name
                    && old.actor_tag == new.actor_tag
                    && old.actor_avatar == new.actor_avatar
            })
    }

    /// row as stored now, may be newer than the one the cached frame was drawn from
    fn current_item(view: &WeakEntity<Self>, cx: &App) -> Option<NotificationItem> {
        view.upgrade().map(|view| view.read(cx).row.item.clone())
    }

    fn set_height(&mut self, height: Pixels, cx: &mut Context<Self>) {
        let prior = self.height.replace(height);
        if prior.is_some_and(|prior| prior != height) {
            self.list.update(cx, |_, cx| cx.notify()).ok();
        }
    }
}

impl Render for NotificationRowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let this = cx.weak_entity();
        let measure_id = NotificationsRows::scoped_id(self.scope, &format!("row-size-{}", self.row.id));
        let cell = NotificationRowCell { scope: self.scope, row: self.row.clone(), view: this.clone(), theme };
        div().relative().w_full().child(cell).child(
            measure(measure_id, move |bounds, _, cx| {
                this.update(cx, |view, cx| view.set_height(bounds.size.height, cx)).ok();
            }),
        )
    }
}

#[derive(IntoElement)]
struct NotificationRowCell {
    scope: &'static str,
    row: Rc<RiseNotificationRow>,
    view: WeakEntity<NotificationRowView>,
    theme: AppTheme,
}

impl NotificationRowCell {
    fn hold_actions(is_read: bool) -> Vec<HoldContextMenuAction> {
        let mut actions = Vec::new();
        if !is_read {
            actions.push(HoldContextMenuAction::new("mark_read", tr("ctx_mark_read")).system_image("checkmark.circle"));
        }
        actions.push(HoldContextMenuAction::new("delete", tr("ctx_delete")).system_image("trash").destructive(true));
        actions
    }

    fn handle_action(id: &str, item: &NotificationItem, cx: &mut App) {
        match id {
            "mark_read" => NotifyInteractionsStore::shared(cx).update(cx, |store, cx| store.mark_read(item, cx)),
            "delete" => {
                let item = item.clone();
                HoldContextMenuPresenter::after_dismiss(
                    move |cx: &mut App| {
                        NotifyInteractionsStore::shared(cx).update(cx, |store, cx| store.delete(&item, cx));
                    },
                    cx,
                );
            }
            _ => {}
        }
    }

    fn enqueue(link: PushDeepLink, cx: &mut App) {
        PushDeepLinkStore::shared(cx).update(cx, |store, cx| {
            store.enqueue(link, cx);
        });
    }

    fn handle_tap(item: &NotificationItem, cx: &mut App) {
        NotifyInteractionsStore::shared(cx).update(cx, |store, cx| store.mark_read(item, cx));

        if item.kind == NotificationKind::Announcement {
            let entity_id = item.entity_id();
            NotifyInteractionsStore::shared(cx).update(cx, |store, cx| store.open_announcement_entity_id(&entity_id, cx));
            return;
        }

        let user_info: Map<String, Value> =
            item.data.iter().map(|(key, value)| (key.clone(), Value::String(value.clone()))).collect();
        let route = PushNotificationRouteParser::route(&user_info)
            .filter(|route| *route != PushDeepLink::Profile { user_id: None, tag: None });
        if let Some(route) = route {
            Self::enqueue(route, cx);
            return;
        }
        if item.admin_url().is_some() {
            return;
        }

        if let Some(post_id) = item.post_id() {
            let link = match item.comment_id() {
                Some(comment_id) => PushDeepLink::PostComment { post_id, comment_id: Some(comment_id) },
                None => PushDeepLink::Post { post_id },
            };
            Self::enqueue(link, cx);
            return;
        }
        if let Some(chat_id) = item.chat_id() {
            Self::enqueue(PushDeepLink::Chat { chat_id }, cx);
            return;
        }
        if let Some(track_id) = item.track_id() {
            Self::enqueue(PushDeepLink::MusicTrack { track_id }, cx);
            return;
        }
        if let Some(playlist_id) = item.playlist_id() {
            let action = item.action();
            let link = match action.as_str() {
                music_service::PLAYLIST_PUSH_ACTION_OPEN_MUSIC => PushDeepLink::Music,
                music_service::PLAYLIST_PUSH_ACTION_JOIN_REQUESTS => PushDeepLink::PlaylistJoinRequests { playlist_id },
                music_service::PLAYLIST_PUSH_ACTION_OPEN_SESSION => PushDeepLink::PlaylistSession { playlist_id },
                _ => PushDeepLink::Playlist { playlist_id },
            };
            Self::enqueue(link, cx);
            return;
        }
        let source_user_id = item.source_user_id();
        let source_user_tag = item.source_user_tag();
        if item.kind.is_follow() || source_user_id.is_some() || source_user_tag.is_some() {
            Self::enqueue(PushDeepLink::Profile { user_id: source_user_id, tag: source_user_tag }, cx);
        }
    }
}

impl RenderOnce for NotificationRowCell {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let is_read = self.row.is_read;
        let content_row = self.row.clone();
        let action_view = self.view.clone();
        let tap_view = self.view.clone();
        vstack(0.0)
            .w_full()
            .child(
                div().w_full().child(
                    HoldContextMenuContainer::new(
                        ElementId::Name(format!("{}-hold-{}", self.scope, self.row.id).into()),
                        HoldContextMenuAlignment::Leading,
                        Rc::new(move |_: &App| Self::hold_actions(is_read)),
                        Rc::new(move |_: &mut Window, _: &mut App| {
                            NotifyRow::new((*content_row).clone()).into_any_element()
                        }),
                    )
                    .show_emoji_list(false)
                    .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                        if let Some(item) = NotificationRowView::current_item(&action_view, cx) {
                            Self::handle_action(&id, &item, cx);
                        }
                    }))
                    .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
                        if let Some(item) = NotificationRowView::current_item(&tap_view, cx) {
                            Self::handle_tap(&item, cx);
                        }
                    })),
                ),
            )
            .child(
                div()
                    .w_full()
                    .pl(px(72.0))
                    .bg(if is_read { self.theme.bg._100 } else { self.theme.bg._200 })
                    .child(div().w_full().h(px(0.5)).bg(self.theme.border._100.opacity(0.5))),
            )
    }
}
