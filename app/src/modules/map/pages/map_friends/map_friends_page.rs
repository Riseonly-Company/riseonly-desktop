use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyView, App, Context, ElementId, Empty, Entity, FontWeight, Hsla, ListAlignment, ListState, SharedString,
    StyleRefinement, Subscription, Window, list, white,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_ui::button_ui::ButtonUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::skeleton_ui::{SkeletonCircle, SkeletonLine};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_theme::AppTheme;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::rise_engine_bridge::rise_transport_status::{RiseTransportDropWatch, RiseTransportStatus};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::map::engine::rise_map_engine_models::RiseMapFriendsSnapshot;
use crate::modules::map::engine::rise_map_friends_repository::RiseMapFriendsRepositoryList;
use crate::modules::map::engine::rise_map_presentation::RiseMapPresentation;
use crate::modules::map::pages::map_friends::map_invite_link_card::MapInviteLinkCard;
use crate::modules::map::stores::map_actions::map_types::{MapFriend, MapFriendRequest, MapInvite, MapUser};
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::map::stores::map_services::map_services::MapServicesStore;

const BLOCK_SPACING: f32 = 18.0;
const SECTION_SPACING: f32 = 10.0;
const MAP_PERSON_ROW_HEIGHT: f32 = 56.0;

/// one row of the flattened list
/// gpui lists rows and not nested sections
/// so a section's title and people and load more trigger follow each other and spacing lives on the row
#[derive(Clone, PartialEq)]
enum MapFriendsPageRow {
    InviteLinkCard(Option<MapInvite>),
    Skeleton,
    Failure(Option<String>),
    SectionTitle(String),
    Incoming(MapFriendRequest),
    Outgoing { request: MapFriendRequest, is_first: bool },
    FriendsEmpty,
    Friend(MapFriend),
    LoadMoreTrigger(RiseMapFriendsRepositoryList),
    BottomSpacer,
}

impl MapFriendsPageRow {
    /// row identity for the list, the height of a row with a given key never changes
    fn key(&self) -> String {
        match self {
            Self::InviteLinkCard(invite) => {
                format!("invite-{}", invite.as_ref().map_or("", |invite| invite.token.as_str()))
            }
            Self::Skeleton => "skeleton".to_owned(),
            Self::Failure(text) => format!("failure-{}", text.as_deref().unwrap_or_default()),
            Self::SectionTitle(title) => format!("title-{title}"),
            Self::Incoming(request) => format!("incoming-{}", request.id),
            Self::Outgoing { request, .. } => format!("outgoing-{}", request.id),
            Self::FriendsEmpty => "friends-empty".to_owned(),
            Self::Friend(friend) => format!("friend-{}", friend.id()),
            Self::LoadMoreTrigger(list) => format!("more-{list:?}"),
            Self::BottomSpacer => "bottom".to_owned(),
        }
    }
}

pub struct MapFriendsPage {
    list: ListUi,
    row_keys: Vec<String>,
    list_view: Entity<MapFriendsListView>,
    shown_triggers: MapFriendsShownTriggers,
    reconnect_watch: Option<RiseTransportDropWatch>,
    _subscriptions: Vec<Subscription>,
}

/// load more triggers drawn right now
/// a trigger appears once so a page that failed offline would leave it hanging
/// the reconnect asks these lists again
type MapFriendsShownTriggers = Rc<RefCell<Vec<RiseMapFriendsRepositoryList>>>;

impl MapFriendsPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseMapPresentation::shared(cx);
        let friends = presentation.read(cx).friends.clone();
        let sandbox = GuideSandbox::shared(cx);
        let list = ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top);
        let list_state = list.state().clone();
        let shown_triggers = MapFriendsShownTriggers::default();
        let list_triggers = shown_triggers.clone();
        let mut page = Self {
            list,
            row_keys: Vec::new(),
            list_view: cx.new(|_| MapFriendsListView { list_state, props: None, shown_triggers: list_triggers }),
            shown_triggers,
            reconnect_watch: None,
            _subscriptions: vec![
                cx.observe(&friends, |_, _, cx| cx.notify()),
                cx.observe(&sandbox, |_, _, cx| cx.notify()),
            ],
        };
        page.arm_reconnect(cx);
        page
    }

    fn arm_reconnect(&mut self, cx: &mut Context<Self>) {
        let page = cx.weak_entity();
        self.reconnect_watch = Some(RiseTransportStatus::on_next_connect(
            move |cx| {
                let _ = page.update(cx, |page, cx| page.resume_after_reconnect(cx));
            },
            cx,
        ));
    }

    fn resume_after_reconnect(&mut self, cx: &mut Context<Self>) {
        self.arm_reconnect(cx);
        let lists = self.shown_triggers.borrow().clone();
        MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.resume_friends(lists, cx));
    }

    fn sync_list(&mut self, rows: &[MapFriendsPageRow]) {
        let keys: Vec<String> = rows.iter().map(MapFriendsPageRow::key).collect();
        if keys == self.row_keys {
            return;
        }
        if self.row_keys.is_empty() {
            self.list.replaced(keys.len());
        } else {
            let old = &self.row_keys;
            let prefix = old.iter().zip(&keys).take_while(|(old, new)| old == new).count();
            let max_suffix = old.len().min(keys.len()) - prefix;
            let suffix = old
                .iter()
                .rev()
                .zip(keys.iter().rev())
                .take(max_suffix)
                .take_while(|(old, new)| old == new)
                .count();
            self.list.state().splice(prefix..old.len() - suffix, keys.len() - prefix - suffix);
        }
        self.row_keys = keys;
    }

    fn rows(snapshot: &RiseMapFriendsSnapshot) -> Vec<MapFriendsPageRow> {
        let mut rows = vec![MapFriendsPageRow::InviteLinkCard(snapshot.invite.clone())];
        if snapshot.shows_initial_loader() {
            rows.push(MapFriendsPageRow::Skeleton);
        } else if snapshot.shows_blocking_error() {
            rows.push(MapFriendsPageRow::Failure(snapshot.error_text.clone()));
        } else {
            Self::lists(snapshot, &mut rows);
        }
        rows.push(MapFriendsPageRow::BottomSpacer);
        rows
    }

    fn lists(snapshot: &RiseMapFriendsSnapshot, rows: &mut Vec<MapFriendsPageRow>) {
        if !snapshot.incoming.is_empty() {
            rows.push(MapFriendsPageRow::SectionTitle(tr("map_requests_incoming").to_uppercase()));
            rows.extend(snapshot.incoming.iter().cloned().map(MapFriendsPageRow::Incoming));
            Self::load_more_trigger(snapshot.has_more_incoming, RiseMapFriendsRepositoryList::Incoming, rows);
        }

        if !snapshot.outgoing.is_empty() {
            rows.push(MapFriendsPageRow::SectionTitle(tr("map_requests_outgoing").to_uppercase()));
            rows.extend(snapshot.outgoing.iter().enumerate().map(|(index, request)| MapFriendsPageRow::Outgoing {
                request: request.clone(),
                is_first: index == 0,
            }));
            Self::load_more_trigger(snapshot.has_more_outgoing, RiseMapFriendsRepositoryList::Outgoing, rows);
        }

        rows.push(MapFriendsPageRow::SectionTitle(tr("map_friends_section").to_uppercase()));
        if snapshot.friends.is_empty() {
            rows.push(MapFriendsPageRow::FriendsEmpty);
        } else {
            rows.extend(snapshot.friends.iter().cloned().map(MapFriendsPageRow::Friend));
            Self::load_more_trigger(snapshot.has_more_friends, RiseMapFriendsRepositoryList::Friends, rows);
        }
    }

    /// sits at the end of the section and fires when it scrolls into the lazy window
    /// next page is requested while the list is still moving
    fn load_more_trigger(has_more: bool, list: RiseMapFriendsRepositoryList, rows: &mut Vec<MapFriendsPageRow>) {
        if has_more {
            rows.push(MapFriendsPageRow::LoadMoreTrigger(list));
        }
    }
}

impl Render for MapFriendsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let snapshot = MapServicesStore::friends(cx);
        let rows = Rc::new(Self::rows(&snapshot));
        self.sync_list(&rows);

        SettingsWrapperUi::new(tr("map_friends_title"), move |_, cx| {
            if let Some(nav_store) = nav_store.as_ref() {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .right_jsx(
            div()
                .id("map-friends-add-hint")
                .aria_label(tr("map_add_friend_title"))
                .tooltip_label(tr("map_add_friend_title"))
                .child(
                    PageHeaderIconButton::new("map-friends-add", "person.badge.plus", |_, cx| {
                        MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.pick_person_to_add(cx));
                    })
                    .font_size(16.0),
                )
                .guide_anchor(RiseGuideAnchor::MapFriendsAdd),
        )
        .content(MapFriendsPageBody { list_view: self.list_view.clone(), rows })
        .on_appear("map-friends-page", |_, cx| {
            MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.friends_page_appeared(cx));
        })
    }
}

#[derive(IntoElement)]
struct MapFriendsPageBody {
    list_view: Entity<MapFriendsListView>,
    rows: Rc<Vec<MapFriendsPageRow>>,
}

impl RenderOnce for MapFriendsPageBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let top_inset = list_shell_floating_top_inset(cx);
        let props = MapFriendsListProps { rows: self.rows, top_inset, theme_mark: MapFriendsThemeMark::of(&theme) };
        let stale = self.list_view.update(cx, |view, _| view.set_props(props));
        let list_view = if stale {
            AnyView::from(self.list_view).into_any_element()
        } else {
            self.list_view.cached(StyleRefinement::default().size_full()).into_any_element()
        };
        div().size_full().bg(theme.bg._100).child(list_view)
    }
}

/// theme colors the page rows draw with, a theme switch redraws every cached row
#[derive(Clone, Copy, PartialEq)]
struct MapFriendsThemeMark {
    primary: Hsla,
    secondary: Hsla,
    accent: Hsla,
    background: Hsla,
}

impl MapFriendsThemeMark {
    fn of(theme: &AppTheme) -> Self {
        Self {
            primary: theme.text.primary,
            secondary: theme.text.secondary,
            accent: theme.primary._100,
            background: theme.bg._100,
        }
    }
}

#[derive(Clone, PartialEq)]
struct MapFriendsListProps {
    rows: Rc<Vec<MapFriendsPageRow>>,
    top_inset: f32,
    theme_mark: MapFriendsThemeMark,
}

/// the list as its own cached view
/// gpui notifies the view that paints a list on every wheel frame
/// painted by the page that would rebuild the page and every ancestor each scroll frame
struct MapFriendsListView {
    list_state: ListState,
    props: Option<MapFriendsListProps>,
    shown_triggers: MapFriendsShownTriggers,
}

impl MapFriendsListView {
    fn set_props(&mut self, props: MapFriendsListProps) -> bool {
        if self.props.as_ref() == Some(&props) {
            return false;
        }
        let drawn_before = self.props.is_some();
        self.props = Some(props);
        drawn_before
    }
}

impl Render for MapFriendsListView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(props) = self.props.clone() else {
            return div().into_any_element();
        };
        let theme = rise_ui::theme(cx).clone();
        let rows = props.rows;
        let top_inset = props.top_inset;
        let theme_mark = props.theme_mark;
        let shown_triggers = self.shown_triggers.clone();
        list(self.list_state.clone(), move |index, window, cx| {
            let Some(row) = rows.get(index) else {
                return Empty.into_any_element();
            };
            map_friends_page_row(row, top_inset, &theme, theme_mark, &shown_triggers, window, cx)
        })
        .size_full()
        .into_any_element()
    }
}

/// one person row as its own cached view
/// the list renders and lays out visible rows on every wheel frame
struct MapPersonRowView {
    props: Option<MapPersonRowProps>,
}

#[derive(Clone, PartialEq)]
struct MapPersonRowProps {
    row: MapFriendsPageRow,
    theme_mark: MapFriendsThemeMark,
}

struct MapPersonRowHolder {
    row: Entity<MapPersonRowView>,
}

impl MapPersonRowView {
    fn element(row: &MapFriendsPageRow, theme_mark: MapFriendsThemeMark, window: &mut Window, cx: &mut App) -> AnyElement {
        let host = window
            .use_keyed_state(ElementId::Name(format!("map-friends-row-{}", row.key()).into()), cx, |_, cx| {
                MapPersonRowHolder { row: cx.new(|_| Self { props: None }) }
            })
            .read(cx)
            .row
            .clone();
        let props = MapPersonRowProps { row: row.clone(), theme_mark };
        let stale = host.update(cx, |host, _| host.set_props(props));
        if stale {
            AnyView::from(host).into_any_element()
        } else {
            host.cached(StyleRefinement::default().w_full().h(px(MAP_PERSON_ROW_HEIGHT))).replay_when_moved().into_any_element()
        }
    }

    fn set_props(&mut self, props: MapPersonRowProps) -> bool {
        if self.props.as_ref() == Some(&props) {
            return false;
        }
        let drawn_before = self.props.is_some();
        self.props = Some(props);
        drawn_before
    }
}

impl Render for MapPersonRowView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        match self.props.as_ref().map(|props| &props.row) {
            Some(MapFriendsPageRow::Incoming(request)) => incoming_row(request).into_any_element(),
            Some(MapFriendsPageRow::Outgoing { request, is_first }) => outgoing_row(request, *is_first).into_any_element(),
            Some(MapFriendsPageRow::Friend(friend)) => friend_row(friend).into_any_element(),
            _ => div().into_any_element(),
        }
    }
}

fn map_friends_page_row(
    row: &MapFriendsPageRow,
    top_inset: f32,
    theme: &AppTheme,
    theme_mark: MapFriendsThemeMark,
    shown_triggers: &MapFriendsShownTriggers,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (top_spacing, body): (f32, AnyElement) = match row {
        MapFriendsPageRow::InviteLinkCard(invite) => {
            (top_inset + 10.0, MapInviteLinkCard::new(invite.clone()).into_any_element())
        }
        MapFriendsPageRow::Skeleton => (BLOCK_SPACING, skeleton().into_any_element()),
        MapFriendsPageRow::Failure(text) => (BLOCK_SPACING, failure(text.as_deref(), theme).into_any_element()),
        MapFriendsPageRow::SectionTitle(title) => (
            BLOCK_SPACING,
            MainText::new(title.clone())
                .px(12.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(theme.text.secondary)
                .number_of_lines(1)
                .into_any_element(),
        ),
        MapFriendsPageRow::Outgoing { request, is_first: true } => {
            (SECTION_SPACING, outgoing_row(request, true).into_any_element())
        }
        MapFriendsPageRow::Incoming(_) | MapFriendsPageRow::Outgoing { .. } | MapFriendsPageRow::Friend(_) => {
            (SECTION_SPACING, MapPersonRowView::element(row, theme_mark, window, cx))
        }
        MapFriendsPageRow::FriendsEmpty => (
            SECTION_SPACING,
            div()
                .py(px(6.0))
                .child(MainText::new(tr("map_friends_empty")).px(14.0).color(theme.text.secondary))
                .into_any_element(),
        ),
        MapFriendsPageRow::LoadMoreTrigger(list) => {
            let list = *list;
            let appeared = shown_triggers.clone();
            let disappeared = shown_triggers.clone();
            (
                SECTION_SPACING,
                MapPersonRowSkeleton
                    .on_appear(ElementId::Name(format!("map-friends-more-{list:?}").into()), move |_, cx| {
                        if !appeared.borrow().contains(&list) {
                            appeared.borrow_mut().push(list);
                        }
                        MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.load_more(list, cx));
                    })
                    .and_on_disappear(move |_, _| disappeared.borrow_mut().retain(|shown| *shown != list))
                    .into_any_element(),
            )
        }
        MapFriendsPageRow::BottomSpacer => (BLOCK_SPACING, div().h(px(60.0)).into_any_element()),
    };
    div().w_full().px(px(16.0)).pt(px(top_spacing)).child(body).into_any_element()
}

fn incoming_row(request: &MapFriendRequest) -> MapPersonRow {
    let accept_id = request.id.clone();
    let decline_id = request.id.clone();
    MapPersonRow::new(format!("incoming-{}", request.id), request.from_user.clone(), None)
        .child(MapRowButton::new(format!("accept-{}", request.id), tr("map_request_accept"), true, move |cx| {
            MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.respond(&accept_id, true, cx));
        }))
        .child(MapRowButton::new(format!("decline-{}", request.id), tr("map_request_decline"), false, move |cx| {
            MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.respond(&decline_id, false, cx));
        }))
}

fn outgoing_row(request: &MapFriendRequest, is_first: bool) -> impl IntoElement {
    let cancel_id = request.id.clone();
    MapPersonRow::new(format!("outgoing-{}", request.id), request.to_user.clone(), Some(tr("map_request_waiting")))
        .child(MapRowButton::new(format!("cancel-{}", request.id), tr("map_request_cancel"), false, move |cx| {
            MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.cancel_request(&cancel_id, cx));
        }))
        .guide_anchor(if is_first { Some(RiseGuideAnchor::MapOutgoingFirst) } else { None })
}

fn friend_row(friend: &MapFriend) -> MapPersonRow {
    let pending_removal = friend.clone();
    MapPersonRow::new(format!("friend-{}", friend.id()), friend.user.clone(), None).child(MapRowButton::new(
        format!("remove-{}", friend.id()),
        tr("map_friend_remove_action"),
        false,
        move |cx| present_removal_confirmation(pending_removal.clone(), cx),
    ))
}

fn present_removal_confirmation(friend: MapFriend, cx: &mut App) {
    NativeConfirmAlert::present_delete(
        &tr("map_friend_remove_title"),
        &tr("map_friend_remove_message").replace("{{name}}", friend.user.display_name()),
        Some(tr("map_friend_remove_action")),
        None,
        false,
        None,
        None,
        move |cx| {
            MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.remove_friend(friend.id(), cx));
        },
        cx,
    );
}

fn skeleton() -> impl IntoElement {
    vstack(10.0)
        .align(HorizontalAlignment::Leading)
        .w_full()
        .child(SkeletonLine::new(120.0, 12.0))
        .children((0..5).map(|_| MapPersonRowSkeleton))
}

fn failure(text: Option<&str>, theme: &AppTheme) -> impl IntoElement {
    vstack(12.0)
        .align(HorizontalAlignment::Center)
        .w_full()
        .pt(px(40.0))
        .child(
            MainText::new(text.map_or_else(|| tr("map_load_failed"), str::to_owned))
                .px(15.0)
                .tac(TextAlignT::Center)
                .color(theme.text.secondary),
        )
        .child(
            ButtonUi::new("map-friends-retry")
                .height(40.0)
                .fit_content(true)
                .padding_horizontal(18.0)
                .on_press(|_, cx| {
                    MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.retry_load(cx));
                })
                .child(
                    MainText::new(tr("common_retry"))
                        .px(15.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(white())
                        .number_of_lines(1),
                ),
        )
}

#[derive(IntoElement)]
struct MapPersonRow {
    id: String,
    user: MapUser,
    subtitle: Option<String>,
    trailing: Vec<AnyElement>,
}

impl MapPersonRow {
    fn new(id: String, user: MapUser, subtitle: Option<String>) -> Self {
        Self { id, user, subtitle, trailing: Vec::new() }
    }
}

impl ParentElement for MapPersonRow {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.trailing.extend(elements);
    }
}

impl RenderOnce for MapPersonRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let secondary = theme.text.secondary;
        let user = self.user;
        let display_name = SharedString::from(user.display_name().to_owned());
        let subtitle = self.subtitle.unwrap_or_else(|| format!("@{}", user.tag));

        hstack(12.0)
            .w_full()
            .h(px(MAP_PERSON_ROW_HEIGHT))
            .items_center()
            .child(
                UserLogoUi::new(ElementId::Name(format!("map-person-logo-{}", self.id).into()))
                    .source(SharedString::from(user.logo.clone()))
                    .name(display_name.clone())
                    .user_id(SharedString::from(user.id.clone()))
                    .size(44.0),
            )
            .child(
                vstack(2.0)
                    .align(HorizontalAlignment::Leading)
                    .min_w_0()
                    .child(MainText::new(display_name).px(16.0).font_weight(FontWeight::MEDIUM).number_of_lines(1))
                    .child(MainText::new(subtitle).px(13.0).color(secondary).number_of_lines(1)),
            )
            .child(spacer_min_w(8.0))
            .children(self.trailing)
    }
}

#[derive(IntoElement)]
struct MapPersonRowSkeleton;

impl RenderOnce for MapPersonRowSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        hstack(12.0)
            .w_full()
            .h(px(MAP_PERSON_ROW_HEIGHT))
            .items_center()
            .child(SkeletonCircle::new(44.0))
            .child(
                vstack(8.0)
                    .align(HorizontalAlignment::Leading)
                    .child(SkeletonLine::new(140.0, 13.0))
                    .child(SkeletonLine::new(90.0, 11.0)),
            )
            .child(spacer_min_w(0.0))
    }
}

const ROW_ACTION_REPEAT_GUARD: Duration = Duration::from_millis(500);

thread_local! {
    static LAST_ROW_ACTION_AT: Cell<Option<Duration>> = const { Cell::new(None) };
}

fn claim_row_action() -> bool {
    let now = RiseClock::monotonic();
    LAST_ROW_ACTION_AT.with(|last| {
        if last.get().is_some_and(|at| now.saturating_sub(at) < ROW_ACTION_REPEAT_GUARD) {
            return false;
        }
        last.set(Some(now));
        true
    })
}

#[derive(IntoElement)]
struct MapRowButton {
    id: String,
    title: String,
    is_primary: bool,
    action: Rc<dyn Fn(&mut App)>,
}

impl MapRowButton {
    fn new(id: String, title: String, is_primary: bool, action: impl Fn(&mut App) + 'static) -> Self {
        Self { id, title, is_primary, action: Rc::new(action) }
    }
}

impl RenderOnce for MapRowButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let action = self.action;
        ButtonUi::new(ElementId::Name(format!("map-row-button-{}", self.id).into()))
            .background_color(if self.is_primary { theme.primary._100 } else { theme.text.secondary.opacity(0.16) })
            .height(32.0)
            .fit_content(true)
            .padding_horizontal(12.0)
            .on_press(move |_, cx| {
                if claim_row_action() {
                    action(cx);
                }
            })
            .child(
                MainText::new(self.title)
                    .px(13.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(if self.is_primary { theme.content_on(theme.primary._100) } else { theme.text.primary })
                    .number_of_lines(1),
            )
    }
}
