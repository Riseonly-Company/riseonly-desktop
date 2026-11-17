use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, Context, ElementId, Empty, Entity, FontWeight, Hsla, ListAlignment, SharedString, Subscription, Task, Window, white};
use rise_canon::RiseGuideAnchor;
use rise_canon::rise_service_text::music_service;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::{plural_word, tr};
use rise_platform::haptics::{NotificationKind, notification};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::music::pages::playlist_collaboration::playlist_collaboration_components::{
    PlaylistCollaborationErrorState, PlaylistCollaborationTime, PlaylistCollaborationUserLabel,
};
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_types::{MusicPlaylistJoinRequest, MusicPlaylistMember};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;

const CARD_CORNER_RADIUS: f32 = 22.0;
const LOAD_MORE_THRESHOLD: usize = 4;
const LIST_ANIMATION_RESPONSE: f64 = 0.3;
const LIST_ANIMATION_DAMPING: f64 = 0.86;

/// row motion for rows gpui keeps out of the layout tree
/// a row that left stays until faded and collapsed, one that came fades in
struct PlaylistJoinRequestRowMotion {
    presence: Animated<f32>,
    departed: Option<(MusicPlaylistJoinRequest, usize)>,
}

/// what one frame of a rows motion draws
#[derive(Clone, Copy)]
struct PlaylistJoinRequestRowFrame {
    presence: f32,
    collapsed_height: Option<f32>,
}

/// one row of the flattened screen
#[derive(Clone)]
enum PlaylistJoinRequestsRow {
    Loading,
    Error,
    Empty,
    Hint,
    Upgrade,
    SectionLabel(i64),
    Request { request: MusicPlaylistJoinRequest, index: usize, item_count: usize, is_guide_first: bool, is_departing: bool },
    BottomSpacer,
}

impl PlaylistJoinRequestsRow {
    /// height of a row with a given key doesnt change, a collapsing row aside
    fn key(&self) -> String {
        match self {
            Self::Loading => "loading".to_owned(),
            Self::Error => "error".to_owned(),
            Self::Empty => "empty".to_owned(),
            Self::Hint => "hint".to_owned(),
            Self::Upgrade => "upgrade".to_owned(),
            Self::SectionLabel(_) => "section-label".to_owned(),
            Self::Request { request, index, item_count, .. } => {
                format!("request-{}-{}", request.id, *index + 1 == *item_count)
            }
            Self::BottomSpacer => "bottom".to_owned(),
        }
    }
}

struct PlaylistJoinRequestsScreenState {
    list: ListUi,
    row_keys: Vec<String>,
    processing_ids: HashSet<String>,
    shown_requests: Vec<MusicPlaylistJoinRequest>,
    motions: HashMap<String, PlaylistJoinRequestRowMotion>,
    row_heights: Rc<RefCell<HashMap<String, f32>>>,
    _subscriptions: [Subscription; 3],
}

impl PlaylistJoinRequestsScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = MusicServicesStore::shared(cx);
        let actions = MusicActionsStore::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        Self {
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            row_keys: Vec::new(),
            processing_ids: HashSet::new(),
            shown_requests: Vec::new(),
            motions: HashMap::new(),
            row_heights: Rc::new(RefCell::new(HashMap::new())),
            _subscriptions: [
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&sandbox, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn sync_requests(&mut self, requests: &[MusicPlaylistJoinRequest], cx: &App) {
        if requests.is_empty() {
            self.shown_requests.clear();
            self.motions.clear();
            self.row_heights.borrow_mut().clear();
            return;
        }
        let now = RiseClock::monotonic();
        let animation = spring(LIST_ANIMATION_RESPONSE, LIST_ANIMATION_DAMPING);
        let current_ids: HashSet<&str> = requests.iter().map(|request| request.id.as_str()).collect();
        let was_shown: HashSet<String> = self.shown_requests.iter().map(|request| request.id.clone()).collect();

        for request in requests {
            if let Some(motion) = self.motions.get_mut(&request.id) {
                if motion.departed.take().is_some() {
                    motion.presence.set_in(1.0, animation, now, cx);
                }
            } else if !was_shown.is_empty() && !was_shown.contains(&request.id) {
                let mut presence = Animated::<f32>::new(0.0);
                presence.set_in(1.0, animation, now, cx);
                self.motions.insert(request.id.clone(), PlaylistJoinRequestRowMotion { presence, departed: None });
            }
        }
        for (index, shown) in self.shown_requests.iter().enumerate() {
            let is_leaving = !current_ids.contains(shown.id.as_str());
            let already_departed = self.motions.get(&shown.id).is_some_and(|motion| motion.departed.is_some());
            if !is_leaving || already_departed || !self.row_heights.borrow().contains_key(&shown.id) {
                continue;
            }
            let mut presence =
                self.motions.remove(&shown.id).map_or_else(|| Animated::new(1.0), |motion| motion.presence);
            presence.set_in(0.0, animation, now, cx);
            self.motions.insert(
                shown.id.clone(),
                PlaylistJoinRequestRowMotion { presence, departed: Some((shown.clone(), index)) },
            );
        }
        self.motions.retain(|_, motion| motion.presence.is_running(now));

        let mut departed: Vec<(MusicPlaylistJoinRequest, usize)> =
            self.motions.values().filter_map(|motion| motion.departed.clone()).collect();
        departed.sort_by_key(|(_, index)| *index);
        let mut shown = requests.to_vec();
        for (request, index) in departed {
            let index = index.min(shown.len());
            shown.insert(index, request);
        }
        self.row_heights.borrow_mut().retain(|id, _| shown.iter().any(|request| request.id == *id));
        self.shown_requests = shown;
    }

    fn row_frames(&self) -> HashMap<String, PlaylistJoinRequestRowFrame> {
        let now = RiseClock::monotonic();
        let heights = self.row_heights.borrow();
        self.motions
            .iter()
            .map(|(id, motion)| {
                let collapsed_height = motion.departed.as_ref().and_then(|_| heights.get(id).copied());
                let presence = motion.presence.value(now).clamp(0.0, 1.0);
                (id.clone(), PlaylistJoinRequestRowFrame { presence, collapsed_height })
            })
            .collect()
    }

    fn sync_list(&mut self, rows: &[PlaylistJoinRequestsRow]) {
        let keys: Vec<String> = rows.iter().map(PlaylistJoinRequestsRow::key).collect();
        if keys != self.row_keys {
            let start = keys.iter().zip(&self.row_keys).take_while(|(new, old)| new == old).count();
            let tail_limit = keys.len().min(self.row_keys.len()) - start;
            let end =
                keys.iter().rev().zip(self.row_keys.iter().rev()).take(tail_limit).take_while(|(new, old)| new == old).count();
            self.list.state().splice(start..self.row_keys.len() - end, keys.len() - start - end);
            self.row_keys = keys;
        }
        for (index, row) in rows.iter().enumerate() {
            if matches!(row, PlaylistJoinRequestsRow::Request { is_departing: true, .. }) {
                self.list.state().remeasure_items(index..index + 1);
            }
        }
    }
}

/// what the actions of the screen need
#[derive(Clone)]
struct PlaylistJoinRequestsScreenParts {
    playlist_id: String,
    state: Entity<PlaylistJoinRequestsScreenState>,
    nav_store: Option<Entity<NavigationStore>>,
    is_at_free_capacity: bool,
}

impl PlaylistJoinRequestsScreenParts {
    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn load(&self, force_refresh: bool, cx: &mut App) {
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_playlist_join_requests(&self.playlist_id, force_refresh, cx);
            interactions.load_playlist_settings(&self.playlist_id, force_refresh, cx);
        });
    }

    fn review(&self, request: &MusicPlaylistJoinRequest, approve: bool, cx: &mut App) {
        if approve && self.is_at_free_capacity {
            notification(NotificationKind::Warning);
            return;
        }
        let inserted = self.state.update(cx, |state, cx| {
            let inserted = state.processing_ids.insert(request.id.clone());
            cx.notify();
            inserted
        });
        if !inserted {
            return;
        }
        let success_state = self.state.clone();
        let success_id = request.id.clone();
        let failure = self.clone();
        let failure_id = request.id.clone();
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.review_playlist_join_request(
                &self.playlist_id,
                &request.id,
                approve,
                Some(Rc::new(move |_: Option<MusicPlaylistMember>, cx: &mut App| {
                    success_state.update(cx, |state, cx| {
                        state.processing_ids.remove(&success_id);
                        cx.notify();
                    });
                    notification(NotificationKind::Success);
                })),
                Some(Rc::new(move |error: Option<String>, cx: &mut App| {
                    failure.state.update(cx, |state, cx| {
                        state.processing_ids.remove(&failure_id);
                        cx.notify();
                    });
                    let Some(error) = error else {
                        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                            interactions.load_playlist_settings(&failure.playlist_id, true, cx);
                        });
                        return;
                    };
                    Notifier::send(
                        tr("music_playlist_request_error"),
                        NotifierSendOptions {
                            content: Some(error),
                            r#type: NotifierType::Error,
                            position: NotifierPosition::Bottom,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                })),
                cx,
            );
        });
    }
}

#[derive(IntoElement)]
pub struct PlaylistJoinRequestsScreen {
    playlist_id: String,
}

impl PlaylistJoinRequestsScreen {
    pub fn new(playlist_id: impl Into<String>) -> Self {
        Self { playlist_id: playlist_id.into() }
    }

    fn rows(
        requests: &[MusicPlaylistJoinRequest],
        shown_requests: &[MusicPlaylistJoinRequest],
        pending_count: Option<i64>,
        is_at_free_capacity: bool,
        is_pending: bool,
        error: Option<String>,
    ) -> Vec<PlaylistJoinRequestsRow> {
        let mut rows = Vec::new();
        if requests.is_empty() && is_pending {
            rows.push(PlaylistJoinRequestsRow::Loading);
        } else if requests.is_empty() && error.is_some_and(|error| !error.is_empty()) {
            rows.push(PlaylistJoinRequestsRow::Error);
        } else if requests.is_empty() {
            rows.push(PlaylistJoinRequestsRow::Empty);
        } else {
            rows.push(PlaylistJoinRequestsRow::Hint);
            if is_at_free_capacity {
                rows.push(PlaylistJoinRequestsRow::Upgrade);
            }
            rows.push(PlaylistJoinRequestsRow::SectionLabel((requests.len() as i64).max(pending_count.unwrap_or(0))));
            let current_ids: HashSet<&str> = requests.iter().map(|request| request.id.as_str()).collect();
            let first_id = requests.first().map(|request| request.id.as_str());
            let item_count = shown_requests.len();
            rows.extend(shown_requests.iter().enumerate().map(|(index, request)| PlaylistJoinRequestsRow::Request {
                request: request.clone(),
                index,
                item_count,
                is_guide_first: first_id == Some(request.id.as_str()),
                is_departing: !current_ids.contains(request.id.as_str()),
            }));
        }
        rows.push(PlaylistJoinRequestsRow::BottomSpacer);
        rows
    }
}

impl RenderOnce for PlaylistJoinRequestsScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let screen_id = format!("playlist-join-requests-{}", self.playlist_id);
        let state = window.use_keyed_state(SharedString::from(format!("{screen_id}-state")), cx, |_, cx| {
            PlaylistJoinRequestsScreenState::new(cx)
        });

        let services = MusicServicesStore::shared(cx);
        let (mut requests, settings, pending_count) = services.update(cx, |services, cx| {
            (
                services.playlist_join_requests(&self.playlist_id, cx),
                services.playlist_settings(&self.playlist_id, cx),
                services.playlist_pending_join_request_count(&self.playlist_id, cx),
            )
        });
        requests.retain(|request| request.status == music_service::PLAYLIST_JOIN_REQUEST_STATUS_PENDING);
        let mut seen = HashSet::new();
        requests.retain(|request| seen.insert(request.id.clone()));
        let is_at_free_capacity = settings.is_some_and(|settings| {
            settings.is_owner()
                && !settings.owner_is_premium
                && settings.active_participant_count >= settings.effective_participant_limit
        });
        let slice = MusicActionsStore::slices(cx).get_playlist_join_requests_sai.clone();
        let (is_pending, error) = (slice.is_pending(), slice.error());

        let (rows, frames, is_animating) = state.update(cx, |state, cx| {
            state.sync_requests(&requests, cx);
            let rows = PlaylistJoinRequestsScreen::rows(
                &requests,
                &state.shown_requests,
                pending_count,
                is_at_free_capacity,
                is_pending,
                error,
            );
            state.sync_list(&rows);
            (rows, state.row_frames(), !state.motions.is_empty())
        });
        if is_animating {
            window.request_animation_frame();
        }

        let parts = PlaylistJoinRequestsScreenParts {
            playlist_id: self.playlist_id,
            state,
            nav_store: env::<NavigationStoreKey>(cx),
            is_at_free_capacity,
        };
        let back = parts.clone();
        let keys_refresh = parts.clone();
        let appearing = parts.clone();
        let body = PlaylistJoinRequestsScreenBody { parts, rows: Rc::new(rows), frames: Rc::new(frames) };
        SettingsWrapperUi::new(tr("music_playlist_requests_title"), move |_, cx| {
            if let Some(nav_store) = &back.nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(PageRefreshKeys::new(
            SharedString::from(format!("{screen_id}-keys")),
            move |_, cx| keys_refresh.load(true, cx),
            body,
        ))
        .on_appear(SharedString::from(format!("{screen_id}-task")), move |_, cx| appearing.load(false, cx))
    }
}

#[derive(IntoElement)]
struct PlaylistJoinRequestsScreenBody {
    parts: PlaylistJoinRequestsScreenParts,
    rows: Rc<Vec<PlaylistJoinRequestsRow>>,
    frames: Rc<HashMap<String, PlaylistJoinRequestRowFrame>>,
}

impl RenderOnce for PlaylistJoinRequestsScreenBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let top_inset = list_shell_floating_top_inset(cx);
        let refresh = self.parts.clone();
        let screen_state = self.parts.state.clone();
        let parts = self.parts;
        let rows = self.rows;
        let frames = self.frames;
        let background = theme.bg._100;
        let list = screen_state.read(cx).list.refreshable(
            "playlist-join-requests-list",
            move |_, cx| {
                refresh.load(true, cx);
                Task::ready(())
            },
            move |index, _, cx| {
                let Some(row) = rows.get(index) else {
                    return Empty.into_any_element();
                };
                let top_padding = if index == 0 { top_inset + 12.0 } else { 0.0 };
                div()
                    .w_full()
                    .pt(px(top_padding))
                    .child(playlist_join_requests_row(row, &parts, &frames, &theme, cx))
                    .into_any_element()
            },
        );
        div().size_full().bg(background).child(list)
    }
}

fn playlist_join_requests_row(
    row: &PlaylistJoinRequestsRow,
    parts: &PlaylistJoinRequestsScreenParts,
    frames: &HashMap<String, PlaylistJoinRequestRowFrame>,
    theme: &AppTheme,
    cx: &mut App,
) -> AnyElement {
    match row {
        PlaylistJoinRequestsRow::Loading => loading_state(theme).into_any_element(),
        PlaylistJoinRequestsRow::Error => {
            let retry = parts.clone();
            let retry: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| retry.load(false, cx));
            PlaylistCollaborationErrorState::new("playlist-join-requests-error", tr("music_playlist_load_error"), Some(retry))
                .into_any_element()
        }
        PlaylistJoinRequestsRow::Empty => empty_state(theme).into_any_element(),
        PlaylistJoinRequestsRow::Hint => div()
            .w_full()
            .px(px(18.0))
            .pb(px(14.0))
            .child(MainText::new(tr("music_playlist_requests_hint")).px(13.0).color(theme.text.secondary).number_of_lines(3))
            .into_any_element(),
        PlaylistJoinRequestsRow::Upgrade => {
            let upgrading = parts.clone();
            div()
                .w_full()
                .pb(px(18.0))
                .child(
                    GroupedBtns::new(
                        "playlist-join-requests-upgrade",
                        vec![
                            GroupedBtn::new("upgrade", tr("music_playlist_upgrade"))
                                .icon("sparkles")
                                .icon_background(ChatTileColor::indigo())
                                .callback(move |_, cx| upgrading.push(RootRoute::SubscriptionSettings, cx)),
                        ],
                    )
                    .end_group_title(tr("music_playlist_preview_capacity")),
                )
                .into_any_element()
        }
        PlaylistJoinRequestsRow::SectionLabel(count) => section_label(*count, theme).into_any_element(),
        PlaylistJoinRequestsRow::Request { request, index, item_count, is_guide_first, is_departing } => {
            let frame = frames.get(&request.id).copied();
            request_card_piece(request, *index, *item_count, *is_guide_first, *is_departing, frame, parts, theme, cx)
                .into_any_element()
        }
        PlaylistJoinRequestsRow::BottomSpacer => div().h(px(80.0)).into_any_element(),
    }
}

fn section_label(count: i64, theme: &AppTheme) -> impl IntoElement {
    let one = tr("chat_join_request_one");
    let few = tr("chat_join_request_few");
    let many = tr("chat_join_request_many");
    let word = plural_word(count, &one, &few, &many);
    div().w_full().px(px(18.0)).pb(px(6.0)).child(
        MainText::new(format!("{count} {word}").to_uppercase())
            .px(12.0)
            .font_weight(FontWeight::SEMIBOLD)
            .color(theme.text.secondary),
    )
}

fn loading_state(theme: &AppTheme) -> impl IntoElement {
    div().w_full().px(px(14.0)).child(
        div().w_full().py(px(6.0)).bg(theme.bg._200).rounded_rect(CARD_CORNER_RADIUS).child(UserListSkeleton::new().rows(6)),
    )
}

fn empty_state(theme: &AppTheme) -> impl IntoElement {
    vstack(12.0)
        .w_full()
        .pt(px(56.0))
        .children(IconUi::sized("person.crop.circle.badge.checkmark", px(44.0), theme.text.secondary, false))
        .child(
            MainText::new(tr("music_playlist_requests_empty"))
                .px(16.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(theme.text.primary),
        )
        .child(
            div().px(px(30.0)).child(
                MainText::new(tr("music_playlist_requests_hint"))
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary)
                    .number_of_lines(3),
            ),
        )
}

#[allow(clippy::too_many_arguments)]
fn request_card_piece(
    request: &MusicPlaylistJoinRequest,
    index: usize,
    item_count: usize,
    is_guide_first: bool,
    is_departing: bool,
    frame: Option<PlaylistJoinRequestRowFrame>,
    parts: &PlaylistJoinRequestsScreenParts,
    theme: &AppTheme,
    cx: &mut App,
) -> impl IntoElement {
    let is_first = index == 0;
    let is_last = index + 1 == item_count;
    let (approve_parts, approve_request) = (parts.clone(), request.clone());
    let (reject_parts, reject_request) = (parts.clone(), request.clone());
    let tap_parts = parts.clone();
    let tap_route = RootRoute::UserPage { user_id: Some(request.user.id.clone()), tag: Some(request.user.tag.clone()) };
    let row = PlaylistJoinRequestRow {
        request: request.clone(),
        theme: theme.clone(),
        is_processing: parts.state.read(cx).processing_ids.contains(&request.id),
        guide_anchor: is_guide_first.then_some(RiseGuideAnchor::JoinRequestAccept),
        on_approve: Rc::new(move |_: &mut Window, cx: &mut App| approve_parts.review(&approve_request, true, cx)),
        on_reject: Rc::new(move |_: &mut Window, cx: &mut App| reject_parts.review(&reject_request, false, cx)),
        on_tap: Rc::new(move |_: &mut Window, cx: &mut App| tap_parts.push(tap_route.clone(), cx)),
    };

    let more_parts = parts.clone();
    let appears_near_end = !is_departing && index + LOAD_MORE_THRESHOLD >= item_count;
    let heights = parts.state.read(cx).row_heights.clone();
    let measured_id = request.id.clone();

    let body = div()
        .relative()
        .w_full()
        .child(row)
        .when(!is_departing, |body| {
            body.child(
                measure(
                    ElementId::Name(SharedString::from(format!("playlist-join-request-size-{}", request.id))),
                    move |bounds, _, _| {
                        heights.borrow_mut().insert(measured_id, f32::from(bounds.size.height));
                    },
                )
                .absolute()
                .inset_0(),
            )
        })
        .when(!is_last, |body| {
            body.child(div().w_full().pl(px(70.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55))))
        });

    let card = div()
        .w_full()
        .bg(theme.bg._200)
        .when(is_first, |card| card.rounded_t(px(CARD_CORNER_RADIUS)))
        .when(is_last, |card| card.rounded_b(px(CARD_CORNER_RADIUS)))
        .map(|card| match frame {
            Some(PlaylistJoinRequestRowFrame { presence, collapsed_height: Some(height) }) => {
                card.h(px(height * presence)).overflow_hidden().opacity(presence).child(body)
            }
            Some(PlaylistJoinRequestRowFrame { presence, collapsed_height: None }) => card.opacity(presence).child(body),
            None => card.child(body),
        });

    let piece = div().w_full().px(px(14.0)).child(card);
    if appears_near_end {
        piece
            .on_appear(ElementId::Name(SharedString::from(format!("playlist-join-request-more-{}", request.id))), move |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.load_more_playlist_join_requests(&more_parts.playlist_id, cx);
                });
            })
            .into_any_element()
    } else {
        piece.into_any_element()
    }
}

#[derive(IntoElement)]
struct PlaylistJoinRequestRow {
    request: MusicPlaylistJoinRequest,
    theme: AppTheme,
    is_processing: bool,
    guide_anchor: Option<RiseGuideAnchor>,
    on_approve: ButtonAction,
    on_reject: ButtonAction,
    on_tap: ButtonAction,
}

impl RenderOnce for PlaylistJoinRequestRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme.clone();
        let request_id = self.request.id.clone();
        let user = &self.request.user;
        let name_color = user
            .is_premium
            .then(|| PremiumNameColor::parse(user.name_color.as_deref()))
            .flatten()
            .unwrap_or(theme.text.primary);
        let mut name = hstack(6.0).min_w_0().max_w_full().child(
            MainText::new(PlaylistCollaborationUserLabel::display_name(user))
                .px(16.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(name_color)
                .number_of_lines(1),
        );
        if user.is_premium {
            name = name.child(
                PremiumBadge::new()
                    .user_id(Some(SharedString::from(user.id.clone())))
                    .emoji(user.premium_badge_emoji.clone().map(SharedString::from))
                    .size(14.0),
            );
        }
        let mut identity = vstack(3.0).align(HorizontalAlignment::Leading).min_w_0().child(name);
        if !user.tag.is_empty() {
            identity = identity
                .child(MainText::new(format!("@{}", user.tag)).px(13.0).color(theme.text.secondary).number_of_lines(1));
        }
        let on_tap = self.on_tap.clone();
        let header = plain(ElementId::Name(SharedString::from(format!("playlist-join-request-open-{request_id}"))))
            .w_full()
            .on_press(move |window, cx| on_tap(window, cx))
            .child(
                hstack(12.0)
                    .w_full()
                    .px(px(14.0))
                    .pt(px(12.0))
                    .pb(px(10.0))
                    .child(
                        UserLogoUi::new(SharedString::from(format!("playlist-join-request-logo-{request_id}")))
                            .source(SharedString::from(user.logo.clone()))
                            .name(SharedString::from(user.name.clone()))
                            .user_id(SharedString::from(user.id.clone()))
                            .size(46.0),
                    )
                    .child(identity)
                    .child(spacer_min_w(8.0))
                    .child(
                        div().flex_none().child(
                            MainText::new(PlaylistCollaborationTime::short(self.request.requested_at_ms))
                                .px(12.0)
                                .color(theme.text.secondary)
                                .number_of_lines(1),
                        ),
                    )
                    .children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(11.0), theme.text.secondary.opacity(0.72), false)),
            );
        let decline = self.action_button(
            format!("playlist-join-request-decline-{request_id}"),
            tr("music_playlist_request_decline"),
            theme.semantic.error_100.opacity(0.14),
            theme.semantic.error_100,
            self.on_reject.clone(),
        );
        let accept = self.action_button(
            format!("playlist-join-request-accept-{request_id}"),
            tr("music_playlist_request_accept"),
            theme.primary._100.opacity(0.86),
            white(),
            self.on_approve.clone(),
        );
        vstack(0.0)
            .w_full()
            .child(header)
            .child(div().w_full().pl(px(72.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.52))))
            .child(hstack(10.0).w_full().px(px(12.0)).py(px(10.0)).child(decline).child(accept.guide_anchor(self.guide_anchor)))
    }
}

impl PlaylistJoinRequestRow {
    fn action_button(
        &self,
        id: String,
        title: String,
        tint: Hsla,
        foreground: Hsla,
        action: ButtonAction,
    ) -> impl IntoElement + use<> {
        let label = if self.is_processing {
            progress_view().tint(foreground).into_any_element()
        } else {
            MainText::new(title)
                .px(14.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(foreground)
                .number_of_lines(1)
                .minimum_scale_factor(0.82)
                .into_any_element()
        };
        div().flex_1().min_w_0().child(
            LiquidGlassButton::new(ElementId::Name(SharedString::from(id)), LiquidGlassButtonShape::Capsule)
                .horizontal_padding(12.0)
                .vertical_padding(9.0)
                .tint(Some(tint))
                .disabled(self.is_processing)
                .on_press(move |window, cx| action(window, cx))
                .child(div().flex_1().flex().items_center().justify_center().child(label)),
        )
    }
}
