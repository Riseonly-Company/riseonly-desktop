use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Context, ElementId, Empty, Entity, FontWeight, ListAlignment, Pixels, Point, SharedString, Subscription, Task, Window};
use rise_i18n::tr;
use rise_platform::haptics::{NotificationKind, notification};
use rise_canon::rise_proto::stickers::ProtoPackCoauthor;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::music::pages::playlist_collaboration::playlist_collaboration_components::{
    PlaylistCollaborationErrorState, PlaylistCollaborationLoadingState,
};
use crate::modules::stickers::pages::pack_collaboration::pack_collaboration_components::{
    PackCollaborationRowDivider, PackCollaborationUserRow,
};
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_coauthors_interactions::PackCollaborationFailureCallback;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

pub type RisePackCoauthor = ProtoPackCoauthor;

const SECTIONS_SPACING: f32 = 18.0;
const SECTION_HEADER_SPACING: f32 = 8.0;
const CARD_CORNER_RADIUS: f32 = 22.0;
const PREFETCH_THRESHOLD: usize = 4;

/// one row of the flattened screen, gpui lists rows not nested stacks
/// so header and people and hint and leave button follow each other
#[derive(Clone)]
enum PackCoauthorsRow {
    Loading,
    Error(String),
    SectionHeader(i64),
    Coauthor { coauthor: RisePackCoauthor, index: usize, item_count: usize },
    LimitHint,
    Leave,
    BottomSpacer,
}

impl PackCoauthorsRow {
    /// height of a row with a given key never changes
    fn key(&self) -> String {
        match self {
            Self::Loading => "loading".to_owned(),
            Self::Error(message) => format!("error-{message}"),
            Self::SectionHeader(_) => "header".to_owned(),
            Self::Coauthor { coauthor, index, item_count } => {
                format!("coauthor-{}-{}", coauthor.user_id, *index + 1 == *item_count)
            }
            Self::LimitHint => "limit-hint".to_owned(),
            Self::Leave => "leave".to_owned(),
            Self::BottomSpacer => "bottom".to_owned(),
        }
    }
}

struct PackCoauthorsScreenState {
    list: ListUi,
    row_keys: Vec<String>,
    load_failure: Option<String>,
    busy_user_id: Option<String>,
    _subscriptions: [Subscription; 2],
}

impl PackCoauthorsScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = StickersServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        Self {
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            row_keys: Vec::new(),
            load_failure: None,
            busy_user_id: None,
            _subscriptions: [cx.observe(&services, |_, _, cx| cx.notify()), cx.observe(&auth, |_, _, cx| cx.notify())],
        }
    }

    fn sync_list(&mut self, rows: &[PackCoauthorsRow]) {
        let keys: Vec<String> = rows.iter().map(PackCoauthorsRow::key).collect();
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

/// what the actions of the screen need, rows and menus hold their own copy
#[derive(Clone)]
struct PackCoauthorsScreenParts {
    pack_id: String,
    state: Entity<PackCoauthorsScreenState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl PackCoauthorsScreenParts {
    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn pop(&self, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }

    fn set_busy(&self, user_id: Option<String>, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.busy_user_id = user_id;
            cx.notify();
        });
    }

    fn handle(&self, coauthor: &RisePackCoauthor, action: &str, cx: &mut App) {
        match action {
            "profile" => self.open_profile(coauthor, cx),
            "rights" => self.open_rights(coauthor, cx),
            "transfer" => self.confirm_transfer(coauthor.clone(), cx),
            "remove" => self.confirm_removal(coauthor.clone(), cx),
            _ => {}
        }
    }

    fn open(&self, coauthor: &RisePackCoauthor, can_manage: bool, cx: &mut App) {
        if can_manage && !coauthor.is_owner {
            self.open_rights(coauthor, cx);
        } else {
            self.open_profile(coauthor, cx);
        }
    }

    fn open_profile(&self, coauthor: &RisePackCoauthor, cx: &mut App) {
        let tag = coauthor.user.as_ref().map(|user| user.tag.clone()).unwrap_or_default();
        self.push(RootRoute::UserPage { user_id: Some(coauthor.user_id.clone()), tag: Some(tag) }, cx);
    }

    fn open_rights(&self, coauthor: &RisePackCoauthor, cx: &mut App) {
        self.push(
            RootRoute::StickerPackCoauthorRights { pack_id: self.pack_id.clone(), user_id: coauthor.user_id.clone() },
            cx,
        );
    }

    fn confirm_removal(&self, coauthor: RisePackCoauthor, cx: &mut App) {
        let parts = self.clone();
        NativeConfirmAlert::present_delete(
            &tr("sticker_pack_coauthor_remove"),
            &tr("sticker_pack_coauthor_remove_confirm"),
            Some(tr("sticker_pack_coauthor_remove")),
            None,
            false,
            None,
            None,
            move |cx| parts.remove(&coauthor, cx),
            cx,
        );
    }

    fn confirm_transfer(&self, coauthor: RisePackCoauthor, cx: &mut App) {
        let parts = self.clone();
        NativeConfirmAlert::present_delete(
            &tr("sticker_pack_transfer_ownership_confirm_title"),
            &tr("sticker_pack_transfer_ownership_confirm_message"),
            Some(tr("sticker_pack_transfer_ownership")),
            None,
            false,
            None,
            None,
            move |cx| parts.transfer(&coauthor, cx),
            cx,
        );
    }

    fn confirm_leave(&self, cx: &mut App) {
        let parts = self.clone();
        NativeConfirmAlert::present_delete(
            &tr("sticker_pack_leave"),
            &tr("sticker_pack_leave_confirm"),
            Some(tr("sticker_pack_leave")),
            None,
            false,
            None,
            None,
            move |cx| parts.leave(cx),
            cx,
        );
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
            interactions.load_pack_coauthors(&self.pack_id, force, false, Some(on_failure), cx);
        });
    }

    fn load_more(&self, cx: &mut App) {
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_pack_coauthors(&self.pack_id, false, true, None, cx);
        });
    }

    fn remove(&self, coauthor: &RisePackCoauthor, cx: &mut App) {
        if self.state.read(cx).busy_user_id.is_some() {
            return;
        }
        self.set_busy(Some(coauthor.user_id.clone()), cx);
        let success = self.clone();
        let failure = self.clone();
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.remove_pack_coauthor(
                &self.pack_id,
                &coauthor.user_id,
                move |cx: &mut App| {
                    success.set_busy(None, cx);
                    notification(NotificationKind::Success);
                },
                move |error: String, cx: &mut App| failure.failed(error, cx),
                cx,
            );
        });
    }

    fn transfer(&self, coauthor: &RisePackCoauthor, cx: &mut App) {
        if self.state.read(cx).busy_user_id.is_some() {
            return;
        }
        self.set_busy(Some(coauthor.user_id.clone()), cx);
        let success = self.clone();
        let failure = self.clone();
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.transfer_pack_ownership(
                &self.pack_id,
                &coauthor.user_id,
                move |cx: &mut App| {
                    success.set_busy(None, cx);
                    notification(NotificationKind::Success);
                    success.load(true, cx);
                },
                move |error: String, cx: &mut App| failure.failed(error, cx),
                cx,
            );
        });
    }

    fn leave(&self, cx: &mut App) {
        let current_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let Some(current_user_id) = current_user_id else {
            return;
        };
        if self.state.read(cx).busy_user_id.is_some() {
            return;
        }
        self.set_busy(Some(current_user_id), cx);
        let success = self.clone();
        let failure = self.clone();
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.leave_pack(
                &self.pack_id,
                move |cx: &mut App| {
                    success.set_busy(None, cx);
                    success.pop(cx);
                },
                move |error: String, cx: &mut App| failure.failed(error, cx),
                cx,
            );
        });
    }

    fn failed(&self, error: String, cx: &mut App) {
        self.set_busy(None, cx);
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
    }
}

/// owner and coauthors of a pack, owner manages rights and removal via hold menu
/// a coauthor can leave
#[derive(IntoElement)]
pub struct PackCoauthorsScreen {
    pack_id: String,
}

impl PackCoauthorsScreen {
    pub fn new(pack_id: impl Into<String>) -> Self {
        Self { pack_id: pack_id.into() }
    }

    /// owner first, then coauthors by join time
    fn sorted(mut coauthors: Vec<RisePackCoauthor>) -> Vec<RisePackCoauthor> {
        let mut seen = HashSet::new();
        coauthors.retain(|coauthor| seen.insert(coauthor.user_id.clone()));
        coauthors.sort_by(|lhs, rhs| rhs.is_owner.cmp(&lhs.is_owner).then(lhs.added_at_ms.cmp(&rhs.added_at_ms)));
        coauthors
    }

    fn rows(
        loaded: Option<Vec<RisePackCoauthor>>,
        server_count: i64,
        load_failure: Option<String>,
        can_leave: bool,
    ) -> Vec<PackCoauthorsRow> {
        let mut rows = Vec::new();
        match (loaded, load_failure) {
            (None, Some(failure)) => rows.push(PackCoauthorsRow::Error(failure)),
            (None, None) => rows.push(PackCoauthorsRow::Loading),
            (Some(coauthors), _) => {
                let coauthors = Self::sorted(coauthors);
                rows.push(PackCoauthorsRow::SectionHeader((coauthors.len() as i64).max(server_count)));
                let only_owners = coauthors.iter().all(|coauthor| coauthor.is_owner);
                let item_count = coauthors.len();
                rows.extend(
                    coauthors
                        .into_iter()
                        .enumerate()
                        .map(|(index, coauthor)| PackCoauthorsRow::Coauthor { coauthor, index, item_count }),
                );
                if only_owners {
                    rows.push(PackCoauthorsRow::LimitHint);
                }
                if can_leave {
                    rows.push(PackCoauthorsRow::Leave);
                }
            }
        }
        rows.push(PackCoauthorsRow::BottomSpacer);
        rows
    }
}

impl RenderOnce for PackCoauthorsScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let screen_id = format!("pack-coauthors-{}", self.pack_id);
        let state =
            window.use_keyed_state(SharedString::from(format!("{screen_id}-state")), cx, |_, cx| PackCoauthorsScreenState::new(cx));
        let parts = PackCoauthorsScreenParts { pack_id: self.pack_id, state: state.clone(), nav_store: env::<NavigationStoreKey>(cx) };

        let current_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let (loaded, server_count) = {
            let services = StickersServicesStore::shared(cx);
            let services = services.read(cx);
            (services.pack_coauthors(&parts.pack_id), services.pack_coauthors_count(&parts.pack_id))
        };
        let me = current_user_id.as_deref().and_then(|current| {
            loaded.as_ref().and_then(|coauthors| coauthors.iter().find(|coauthor| coauthor.user_id == current).cloned())
        });
        let can_manage = me.as_ref().is_some_and(|me| me.is_owner);
        let can_leave = me.as_ref().is_some_and(|me| !me.is_owner);
        let load_failure = state.read(cx).load_failure.clone();
        let rows = Rc::new(PackCoauthorsScreen::rows(loaded, server_count, load_failure, can_leave));
        state.update(cx, |state, _| state.sync_list(&rows));

        let back = parts.clone();
        let keys_refresh = parts.clone();
        let appearing = parts.clone();
        let body = PackCoauthorsScreenBody { parts, rows, can_manage };
        SettingsWrapperUi::new(tr("sticker_pack_coauthors_title"), move |_, cx| back.pop(cx))
            .content(PageRefreshKeys::new(
                SharedString::from(format!("{screen_id}-keys")),
                move |_, cx| keys_refresh.load(true, cx),
                body,
            ))
            .on_appear(SharedString::from(format!("{screen_id}-task")), move |_, cx| appearing.load(true, cx))
    }
}

#[derive(IntoElement)]
struct PackCoauthorsScreenBody {
    parts: PackCoauthorsScreenParts,
    rows: Rc<Vec<PackCoauthorsRow>>,
    can_manage: bool,
}

impl RenderOnce for PackCoauthorsScreenBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let top_inset = list_shell_floating_top_inset(cx);
        let refresh = self.parts.clone();
        let screen_state = self.parts.state.clone();
        let parts = self.parts;
        let rows = self.rows;
        let can_manage = self.can_manage;
        let background = theme.bg._100;
        let list = screen_state.read(cx).list.refreshable(
            "pack-coauthors-list",
            move |_, cx| {
                refresh.load(true, cx);
                Task::ready(())
            },
            move |index, _, cx| {
                let Some(row) = rows.get(index) else {
                    return Empty.into_any_element();
                };
                let top_padding = if index == 0 { top_inset + 12.0 } else { 0.0 };
                div().w_full().pt(px(top_padding)).child(pack_coauthors_row(row, &parts, can_manage, &theme, cx)).into_any_element()
            },
        );
        div().size_full().bg(background).child(list)
    }
}

fn pack_coauthors_row(
    row: &PackCoauthorsRow,
    parts: &PackCoauthorsScreenParts,
    can_manage: bool,
    theme: &AppTheme,
    cx: &mut App,
) -> AnyElement {
    match row {
        PackCoauthorsRow::Loading => PlaylistCollaborationLoadingState::new().into_any_element(),
        PackCoauthorsRow::Error(message) => {
            let retry = parts.clone();
            let retry: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| retry.load(true, cx));
            PlaylistCollaborationErrorState::new("pack-coauthors-error", message.clone(), Some(retry)).into_any_element()
        }
        PackCoauthorsRow::SectionHeader(count) => hstack(0.0)
            .w_full()
            .px(px(18.0))
            .pb(px(SECTION_HEADER_SPACING))
            .child(
                MainText::new(tr("sticker_pack_coauthors_section").to_uppercase())
                    .px(12.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.secondary),
            )
            .child(spacer())
            .child(MainText::new(count.to_string()).px(12.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.secondary))
            .into_any_element(),
        PackCoauthorsRow::Coauthor { coauthor, index, item_count } => {
            coauthor_card_piece(coauthor, *index, *item_count, parts, can_manage, theme, cx).into_any_element()
        }
        PackCoauthorsRow::LimitHint => div()
            .w_full()
            .pt(px(SECTIONS_SPACING))
            .px(px(30.0))
            .child(
                MainText::new(tr("sticker_pack_coauthor_limit_hint"))
                    .px(13.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary)
                    .number_of_lines(4),
            )
            .into_any_element(),
        PackCoauthorsRow::Leave => {
            let busy = parts.state.read(cx).busy_user_id.is_some();
            let leaving = parts.clone();
            div()
                .w_full()
                .pt(px(SECTIONS_SPACING))
                .child(GroupedBtns::new(
                    "pack-coauthors-leave",
                    vec![
                        GroupedBtn::new("leave", tr("sticker_pack_leave"))
                            .icon("rectangle.portrait.and.arrow.right")
                            .icon_background(ChatTileColor::red())
                            .destructive(true)
                            .disabled(busy)
                            .show_chevron(false)
                            .callback(move |_, cx| leaving.confirm_leave(cx)),
                    ],
                ))
                .into_any_element()
        }
        PackCoauthorsRow::BottomSpacer => div().h(px(80.0)).into_any_element(),
    }
}

fn coauthor_card_piece(
    coauthor: &RisePackCoauthor,
    index: usize,
    item_count: usize,
    parts: &PackCoauthorsScreenParts,
    can_manage: bool,
    theme: &AppTheme,
    cx: &mut App,
) -> impl IntoElement {
    let is_first = index == 0;
    let is_last = index + 1 == item_count;
    let busy = parts.state.read(cx).busy_user_id.as_deref() == Some(coauthor.user_id.as_str());
    let user_id = coauthor.user_id.clone();
    let actions = coauthor_actions(coauthor, can_manage);

    let (row_coauthor, row_theme) = (coauthor.clone(), theme.clone());
    let (action_parts, action_coauthor) = (parts.clone(), coauthor.clone());
    let (tap_parts, tap_coauthor) = (parts.clone(), coauthor.clone());
    let hold = HoldContextMenuContainer::new(
        SharedString::from(format!("pack-coauthor-hold-{user_id}")),
        HoldContextMenuAlignment::Leading,
        Rc::new(move |_: &App| actions.clone()),
        Rc::new(move |_: &mut Window, _: &mut App| {
            coauthor_row(&row_coauthor, can_manage, busy, is_first, is_last, &row_theme).into_any_element()
        }),
    )
    .show_emoji_list(false)
    .on_action(Rc::new(move |action: String, _: &mut Window, cx: &mut App| {
        action_parts.handle(&action_coauthor, &action, cx);
    }))
    .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
        tap_parts.open(&tap_coauthor, can_manage, cx);
    }));

    let card = div()
        .w_full()
        .bg(theme.bg._200)
        .when(is_first, |card| card.rounded_t(px(CARD_CORNER_RADIUS)))
        .when(is_last, |card| card.rounded_b(px(CARD_CORNER_RADIUS)))
        .child(hold)
        .when(!is_last, |card| card.child(PackCollaborationRowDivider::new()));
    let piece = div().w_full().px(px(14.0)).child(card);
    if index + PREFETCH_THRESHOLD >= item_count {
        let more_parts = parts.clone();
        piece
            .on_appear(ElementId::Name(SharedString::from(format!("pack-coauthor-more-{user_id}"))), move |_, cx| {
                more_parts.load_more(cx)
            })
            .into_any_element()
    } else {
        piece.into_any_element()
    }
}

fn coauthor_row(
    coauthor: &RisePackCoauthor,
    can_manage: bool,
    busy: bool,
    is_first: bool,
    is_last: bool,
    theme: &AppTheme,
) -> impl IntoElement {
    let subtitle = tr(if coauthor.is_owner { "sticker_pack_owner" } else { "sticker_pack_coauthor" });
    let mut row = PackCollaborationUserRow::new(
        SharedString::from(format!("pack-coauthor-row-{}", coauthor.user_id)),
        coauthor.user.clone(),
        coauthor.user_id.clone(),
        |_, _| {},
    )
    .allows_hit_testing(false)
    .subtitle(Some(subtitle));
    if can_manage && !coauthor.is_owner {
        if let Some(chevron) = IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(11.0), theme.text.secondary.opacity(0.7), false) {
            row = row.trailing(chevron);
        }
    }
    div()
        .w_full()
        .bg(theme.bg._200)
        .when(is_first, |row| row.rounded_t(px(CARD_CORNER_RADIUS)))
        .when(is_last, |row| row.rounded_b(px(CARD_CORNER_RADIUS)))
        .opacity(if busy { 0.5 } else { 1.0 })
        .child(row)
}

fn coauthor_actions(coauthor: &RisePackCoauthor, can_manage: bool) -> Vec<HoldContextMenuAction> {
    let mut result =
        vec![HoldContextMenuAction::new("profile", tr("ctx_open_profile")).system_image("person.crop.circle")];
    if !can_manage || coauthor.is_owner {
        return result;
    }
    result.push(HoldContextMenuAction::new("rights", tr("sticker_pack_rights_title")).system_image("slider.horizontal.3"));
    result.push(HoldContextMenuAction::new("transfer", tr("sticker_pack_transfer_ownership")).system_image("crown"));
    result.push(
        HoldContextMenuAction::new("remove", tr("sticker_pack_coauthor_remove"))
            .system_image("person.crop.circle.badge.minus")
            .destructive(true)
            .section_break(true),
    );
    result
}
