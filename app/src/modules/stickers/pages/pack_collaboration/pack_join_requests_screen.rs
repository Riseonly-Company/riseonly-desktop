use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Context, ElementId, Empty, Entity, FontWeight, Hsla, ListAlignment, SharedString, Subscription, Task, Window};
use rise_canon::rise_service_text::stickers_service;
use rise_i18n::{plural_word, tr};
use rise_platform::haptics::{NotificationKind, notification};
use rise_canon::rise_proto::stickers::ProtoPackJoinRequest;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::music::pages::playlist_collaboration::playlist_collaboration_components::{
    PlaylistCollaborationErrorState, PlaylistCollaborationLoadingState, PlaylistCollaborationTime,
};
use crate::modules::stickers::pages::pack_collaboration::pack_collaboration_components::{
    PackCollaborationRowDivider, PackCollaborationUserRow,
};
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_coauthors_interactions::PackCollaborationFailureCallback;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

pub type RisePackJoinRequest = ProtoPackJoinRequest;

const CARD_CORNER_RADIUS: f32 = 22.0;
const PREFETCH_THRESHOLD: usize = 4;

/// one row of the flattened screen,
/// hint and label and cards follow each other and the card is drawn piece by piece
#[derive(Clone)]
enum PackJoinRequestsRow {
    Loading,
    Error(String),
    Empty,
    Hint,
    SectionLabel(i64),
    Request { request: RisePackJoinRequest, index: usize, item_count: usize },
    BottomSpacer,
}

impl PackJoinRequestsRow {
    /// height of a row with a given key never changes
    fn key(&self) -> String {
        match self {
            Self::Loading => "loading".to_owned(),
            Self::Error(message) => format!("error-{message}"),
            Self::Empty => "empty".to_owned(),
            Self::Hint => "hint".to_owned(),
            Self::SectionLabel(_) => "label".to_owned(),
            Self::Request { request, index, item_count } => format!("request-{}-{}", request.id, *index + 1 == *item_count),
            Self::BottomSpacer => "bottom".to_owned(),
        }
    }
}

struct PackJoinRequestsScreenState {
    list: ListUi,
    row_keys: Vec<String>,
    processing_ids: HashSet<String>,
    load_failure: Option<String>,
    _subscriptions: [Subscription; 1],
}

impl PackJoinRequestsScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = StickersServicesStore::shared(cx);
        Self {
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            row_keys: Vec::new(),
            processing_ids: HashSet::new(),
            load_failure: None,
            _subscriptions: [cx.observe(&services, |_, _, cx| cx.notify())],
        }
    }

    fn sync_list(&mut self, rows: &[PackJoinRequestsRow]) {
        let keys: Vec<String> = rows.iter().map(PackJoinRequestsRow::key).collect();
        if keys == self.row_keys {
            return;
        }
        let start = keys.iter().zip(&self.row_keys).take_while(|(new, old)| new == old).count();
        let tail_limit = keys.len().min(self.row_keys.len()) - start;
        let end = keys.iter().rev().zip(self.row_keys.iter().rev()).take(tail_limit).take_while(|(new, old)| new == old).count();
        self.list.state().splice(start..self.row_keys.len() - end, keys.len() - start - end);
        self.row_keys = keys;
    }
}

/// what the actions of the screen need, cards hold their own copy
#[derive(Clone)]
struct PackJoinRequestsScreenParts {
    pack_id: String,
    state: Entity<PackJoinRequestsScreenState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl PackJoinRequestsScreenParts {
    fn pop(&self, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }

    fn open_profile(&self, request: &RisePackJoinRequest, cx: &mut App) {
        let Some(nav_store) = &self.nav_store else {
            return;
        };
        let tag = request.user.as_ref().map(|user| user.tag.clone()).unwrap_or_default();
        let route = RootRoute::UserPage { user_id: Some(request.user_id.clone()), tag: Some(tag) };
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }

    fn load(&self, force: bool, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.load_failure = None;
            cx.notify();
        });
        let failure_state = self.state.clone();
        let on_failure: PackCollaborationFailureCallback = Box::new(move |error: String, cx: &mut App| {
            failure_state.update(cx, |state, cx| {
                state.load_failure = Some(error);
                cx.notify();
            });
        });
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_pack_join_requests(&self.pack_id, force, false, Some(on_failure), cx);
        });
    }

    fn load_more(&self, cx: &mut App) {
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_pack_join_requests(&self.pack_id, false, true, None, cx);
        });
    }

    fn set_processing(&self, request_id: &str, processing: bool, cx: &mut App) -> bool {
        self.state.update(cx, |state, cx| {
            let changed = if processing {
                state.processing_ids.insert(request_id.to_owned())
            } else {
                state.processing_ids.remove(request_id)
            };
            cx.notify();
            changed
        })
    }

    fn review(&self, request: &RisePackJoinRequest, approve: bool, cx: &mut App) {
        if !self.set_processing(&request.id, true, cx) {
            return;
        }
        let (success, success_id) = (self.clone(), request.id.clone());
        let (failure, failure_id) = (self.clone(), request.id.clone());
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.review_pack_join_request(
                request,
                approve,
                move |cx: &mut App| {
                    success.set_processing(&success_id, false, cx);
                    notification(NotificationKind::Success);
                },
                move |error: String, cx: &mut App| {
                    failure.set_processing(&failure_id, false, cx);
                    Notifier::send(
                        tr("sticker_pack_request_error"),
                        NotifierSendOptions {
                            content: Some(error),
                            r#type: NotifierType::Error,
                            position: NotifierPosition::Bottom,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                },
                cx,
            );
        });
    }
}

/// pending requests to become a coauthor of a pack, a card per person with decline and accept
#[derive(IntoElement)]
pub struct PackJoinRequestsScreen {
    pack_id: String,
}

impl PackJoinRequestsScreen {
    pub fn new(pack_id: impl Into<String>) -> Self {
        Self { pack_id: pack_id.into() }
    }

    fn rows(loaded: Option<Vec<RisePackJoinRequest>>, server_count: i64, load_failure: Option<String>) -> Vec<PackJoinRequestsRow> {
        let mut rows = Vec::new();
        match (loaded, load_failure) {
            (None, Some(failure)) => rows.push(PackJoinRequestsRow::Error(failure)),
            (None, None) => rows.push(PackJoinRequestsRow::Loading),
            (Some(loaded), _) => {
                let mut seen = HashSet::new();
                let requests: Vec<RisePackJoinRequest> = loaded
                    .into_iter()
                    .filter(|request| request.status == stickers_service::STICKER_PACK_JOIN_REQUEST_STATUS_PENDING)
                    .filter(|request| seen.insert(request.id.clone()))
                    .collect();
                if requests.is_empty() {
                    rows.push(PackJoinRequestsRow::Empty);
                } else {
                    rows.push(PackJoinRequestsRow::Hint);
                    rows.push(PackJoinRequestsRow::SectionLabel((requests.len() as i64).max(server_count)));
                    let item_count = requests.len();
                    rows.extend(
                        requests
                            .into_iter()
                            .enumerate()
                            .map(|(index, request)| PackJoinRequestsRow::Request { request, index, item_count }),
                    );
                }
            }
        }
        rows.push(PackJoinRequestsRow::BottomSpacer);
        rows
    }
}

impl RenderOnce for PackJoinRequestsScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let screen_id = format!("pack-join-requests-{}", self.pack_id);
        let state = window.use_keyed_state(SharedString::from(format!("{screen_id}-state")), cx, |_, cx| {
            PackJoinRequestsScreenState::new(cx)
        });
        let parts =
            PackJoinRequestsScreenParts { pack_id: self.pack_id, state: state.clone(), nav_store: env::<NavigationStoreKey>(cx) };
        let (loaded, server_count) = {
            let services = StickersServicesStore::shared(cx);
            let services = services.read(cx);
            (services.pack_join_requests(&parts.pack_id), services.pack_join_requests_count(&parts.pack_id))
        };
        let load_failure = state.read(cx).load_failure.clone();
        let rows = Rc::new(PackJoinRequestsScreen::rows(loaded, server_count, load_failure));
        state.update(cx, |state, _| state.sync_list(&rows));

        let back = parts.clone();
        let keys_refresh = parts.clone();
        let appearing = parts.clone();
        let body = PackJoinRequestsScreenBody { parts, rows };
        SettingsWrapperUi::new(tr("sticker_pack_requests_title"), move |_, cx| back.pop(cx))
            .content(PageRefreshKeys::new(
                SharedString::from(format!("{screen_id}-keys")),
                move |_, cx| keys_refresh.load(true, cx),
                body,
            ))
            .on_appear(SharedString::from(format!("{screen_id}-task")), move |_, cx| appearing.load(true, cx))
    }
}

#[derive(IntoElement)]
struct PackJoinRequestsScreenBody {
    parts: PackJoinRequestsScreenParts,
    rows: Rc<Vec<PackJoinRequestsRow>>,
}

impl RenderOnce for PackJoinRequestsScreenBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let top_inset = list_shell_floating_top_inset(cx);
        let refresh = self.parts.clone();
        let screen_state = self.parts.state.clone();
        let parts = self.parts;
        let rows = self.rows;
        let background = theme.bg._100;
        let list = screen_state.read(cx).list.refreshable(
            "pack-join-requests-list",
            move |_, cx| {
                refresh.load(true, cx);
                Task::ready(())
            },
            move |index, _, cx| {
                let Some(row) = rows.get(index) else {
                    return Empty.into_any_element();
                };
                let top_padding = if index == 0 { top_inset + 12.0 } else { 0.0 };
                div().w_full().pt(px(top_padding)).child(pack_join_requests_row(row, &parts, &theme, cx)).into_any_element()
            },
        );
        div().size_full().bg(background).child(list)
    }
}

fn pack_join_requests_row(
    row: &PackJoinRequestsRow,
    parts: &PackJoinRequestsScreenParts,
    theme: &AppTheme,
    cx: &mut App,
) -> AnyElement {
    match row {
        PackJoinRequestsRow::Loading => PlaylistCollaborationLoadingState::new().into_any_element(),
        PackJoinRequestsRow::Error(message) => {
            let retry = parts.clone();
            let retry: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| retry.load(true, cx));
            PlaylistCollaborationErrorState::new("pack-join-requests-error", message.clone(), Some(retry)).into_any_element()
        }
        PackJoinRequestsRow::Empty => empty_state(theme).into_any_element(),
        PackJoinRequestsRow::Hint => div()
            .w_full()
            .px(px(18.0))
            .pb(px(14.0))
            .child(MainText::new(tr("sticker_pack_requests_hint")).px(13.0).color(theme.text.secondary).number_of_lines(3))
            .into_any_element(),
        PackJoinRequestsRow::SectionLabel(count) => section_label(*count, theme).into_any_element(),
        PackJoinRequestsRow::Request { request, index, item_count } => {
            request_card_piece(request, *index, *item_count, parts, theme, cx).into_any_element()
        }
        PackJoinRequestsRow::BottomSpacer => div().h(px(80.0)).into_any_element(),
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

fn empty_state(theme: &AppTheme) -> impl IntoElement {
    vstack(12.0)
        .w_full()
        .pt(px(56.0))
        .children(IconUi::sized("person.crop.circle.badge.checkmark", px(44.0), theme.text.secondary, false))
        .child(
            MainText::new(tr("sticker_pack_requests_empty"))
                .px(16.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(theme.text.primary),
        )
        .child(
            div().px(px(30.0)).child(
                MainText::new(tr("sticker_pack_requests_hint"))
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary)
                    .number_of_lines(3),
            ),
        )
}

fn request_card_piece(
    request: &RisePackJoinRequest,
    index: usize,
    item_count: usize,
    parts: &PackJoinRequestsScreenParts,
    theme: &AppTheme,
    cx: &mut App,
) -> impl IntoElement {
    let is_first = index == 0;
    let is_last = index + 1 == item_count;
    let (approve_parts, approve_request) = (parts.clone(), request.clone());
    let (reject_parts, reject_request) = (parts.clone(), request.clone());
    let (tap_parts, tap_request) = (parts.clone(), request.clone());
    let card_row = PackJoinRequestCard {
        request: request.clone(),
        theme: theme.clone(),
        is_processing: parts.state.read(cx).processing_ids.contains(&request.id),
        on_approve: Rc::new(move |_: &mut Window, cx: &mut App| approve_parts.review(&approve_request, true, cx)),
        on_reject: Rc::new(move |_: &mut Window, cx: &mut App| reject_parts.review(&reject_request, false, cx)),
        on_tap: Rc::new(move |_: &mut Window, cx: &mut App| tap_parts.open_profile(&tap_request, cx)),
    };
    let card = div()
        .w_full()
        .bg(theme.bg._200)
        .when(is_first, |card| card.rounded_t(px(CARD_CORNER_RADIUS)))
        .when(is_last, |card| card.rounded_b(px(CARD_CORNER_RADIUS)))
        .child(card_row)
        .when(!is_last, |card| card.child(PackCollaborationRowDivider::new()));
    let piece = div().w_full().px(px(14.0)).child(card);
    if index + PREFETCH_THRESHOLD >= item_count {
        let more_parts = parts.clone();
        piece
            .on_appear(ElementId::Name(SharedString::from(format!("pack-join-request-more-{}", request.id))), move |_, cx| {
                more_parts.load_more(cx)
            })
            .into_any_element()
    } else {
        piece.into_any_element()
    }
}

#[derive(IntoElement)]
struct PackJoinRequestCard {
    request: RisePackJoinRequest,
    theme: AppTheme,
    is_processing: bool,
    on_approve: ButtonAction,
    on_reject: ButtonAction,
    on_tap: ButtonAction,
}

impl RenderOnce for PackJoinRequestCard {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme.clone();
        let request_id = self.request.id.clone();
        let on_tap = self.on_tap.clone();
        let mut header = PackCollaborationUserRow::new(
            SharedString::from(format!("pack-join-request-row-{request_id}")),
            self.request.user.clone(),
            self.request.user_id.clone(),
            move |window, cx| on_tap(window, cx),
        )
        .trailing(MainText::new(PlaylistCollaborationTime::short(self.request.requested_at_ms)).px(12.0).color(theme.text.secondary));
        if let Some(chevron) = IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(11.0), theme.text.secondary.opacity(0.72), false) {
            header = header.trailing(chevron);
        }
        let decline = self.action_button(
            format!("pack-join-request-decline-{request_id}"),
            tr("music_playlist_request_decline"),
            theme.semantic.error_100.opacity(0.14),
            theme.semantic.error_100,
            self.on_reject.clone(),
        );
        let accept = self.action_button(
            format!("pack-join-request-accept-{request_id}"),
            tr("music_playlist_request_accept"),
            theme.primary._100.opacity(0.86),
            theme.content_on(theme.primary._100),
            self.on_approve.clone(),
        );
        vstack(0.0)
            .w_full()
            .child(header)
            .child(div().w_full().pl(px(72.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.52))))
            .child(hstack(10.0).w_full().px(px(12.0)).py(px(10.0)).child(decline).child(accept))
    }
}

impl PackJoinRequestCard {
    fn action_button(&self, id: String, title: String, tint: Hsla, foreground: Hsla, action: ButtonAction) -> impl IntoElement + use<> {
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
