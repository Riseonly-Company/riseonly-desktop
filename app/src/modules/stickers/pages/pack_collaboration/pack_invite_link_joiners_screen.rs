use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Context, ElementId, Empty, Entity, FontWeight, ListAlignment, SharedString, Subscription, Task, Window};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::music::pages::playlist_collaboration::playlist_collaboration_components::{
    PlaylistCollaborationErrorState, PlaylistCollaborationLoadingState, PlaylistCollaborationTime,
};
use crate::modules::stickers::engine::rise_pack_collaboration_presentation::RisePackCollaborationPresentation;
use crate::modules::stickers::engine::rise_pack_collaboration_repository::RisePackCoauthor;
use crate::modules::stickers::pages::pack_collaboration::pack_collaboration_components::{
    PackCollaborationRowDivider, PackCollaborationUserRow,
};
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_coauthors_interactions::PackCollaborationFailureCallback;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

const CARD_CORNER_RADIUS: f32 = 22.0;
const PREFETCH_THRESHOLD: usize = 4;

fn page_id(link_id: &str, name: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("pack-invite-link-joiners-{link_id}-{name}")))
}

/// one row of the flattened screen, the card of people is drawn piece by piece
#[derive(Clone)]
enum PackInviteLinkJoinersRow {
    Loading,
    Failure(String),
    Empty,
    Joiner { joiner: RisePackCoauthor, index: usize, item_count: usize },
    BottomSpacer,
}

impl PackInviteLinkJoinersRow {
    /// height of a row with a given key never changes
    fn key(&self) -> String {
        match self {
            Self::Loading => "loading".to_owned(),
            Self::Failure(message) => format!("failure-{message}"),
            Self::Empty => "empty".to_owned(),
            Self::Joiner { joiner, index, item_count } => {
                format!("joiner-{}-{}-{}", joiner.user_id, *index == 0, *index + 1 == *item_count)
            }
            Self::BottomSpacer => "bottom".to_owned(),
        }
    }
}

/// load failure and the list the rows live in
struct PackInviteLinkJoinersState {
    list: ListUi,
    row_keys: Vec<String>,
    load_failure: Option<String>,
    _subscriptions: [Subscription; 2],
}

impl PackInviteLinkJoinersState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = StickersServicesStore::shared(cx);
        let presentation = RisePackCollaborationPresentation::shared(cx);
        Self {
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            row_keys: Vec::new(),
            load_failure: None,
            _subscriptions: [
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&presentation, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn sync_list(&mut self, rows: &[PackInviteLinkJoinersRow]) {
        let keys: Vec<String> = rows.iter().map(PackInviteLinkJoinersRow::key).collect();
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

/// what the actions of the screen need, rows hold their own copy
#[derive(Clone)]
struct PackInviteLinkJoinersParts {
    link_id: String,
    state: Entity<PackInviteLinkJoinersState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl PackInviteLinkJoinersParts {
    fn load(&self, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.load_failure = None;
            cx.notify();
        });
        let failure_state = self.state.clone();
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_pack_link_joiners(
                &self.link_id,
                false,
                Some(Box::new(move |text: String, cx: &mut App| {
                    failure_state.update(cx, |state, cx| {
                        state.load_failure = Some(text);
                        cx.notify();
                    });
                }) as PackCollaborationFailureCallback),
                cx,
            );
        });
    }

    fn load_more(&self, cx: &mut App) {
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_pack_link_joiners(&self.link_id, true, None, cx);
        });
    }

    fn open_profile(&self, joiner: &RisePackCoauthor, cx: &mut App) {
        let Some(nav_store) = &self.nav_store else {
            return;
        };
        let route = RootRoute::UserPage {
            user_id: Some(joiner.user_id.clone()),
            tag: Some(joiner.user.as_ref().map(|user| user.tag.clone()).unwrap_or_default()),
        };
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }
}

/// people who became coauthors through one invite link, newest first with join time
#[derive(IntoElement)]
pub struct PackInviteLinkJoinersScreen {
    pack_id: String,
    link_id: String,
}

impl PackInviteLinkJoinersScreen {
    pub fn new(pack_id: impl Into<String>, link_id: impl Into<String>) -> Self {
        Self { pack_id: pack_id.into(), link_id: link_id.into() }
    }

    fn rows(loaded: Option<Vec<RisePackCoauthor>>, load_failure: Option<String>) -> Vec<PackInviteLinkJoinersRow> {
        let mut rows = Vec::new();
        match loaded {
            None => rows.push(match load_failure {
                Some(message) => PackInviteLinkJoinersRow::Failure(message),
                None => PackInviteLinkJoinersRow::Loading,
            }),
            Some(mut joiners) => {
                let mut seen = HashSet::new();
                joiners.retain(|joiner| seen.insert(joiner.user_id.clone()));
                joiners.sort_by(|left, right| right.added_at_ms.cmp(&left.added_at_ms));
                if joiners.is_empty() {
                    rows.push(PackInviteLinkJoinersRow::Empty);
                } else {
                    let item_count = joiners.len();
                    rows.extend(
                        joiners
                            .into_iter()
                            .enumerate()
                            .map(|(index, joiner)| PackInviteLinkJoinersRow::Joiner { joiner, index, item_count }),
                    );
                }
            }
        }
        rows.push(PackInviteLinkJoinersRow::BottomSpacer);
        rows
    }
}

impl RenderOnce for PackInviteLinkJoinersScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let link_id = self.link_id;
        let state = window.use_keyed_state(page_id(&link_id, &format!("{}-state", self.pack_id)), cx, |_, cx| {
            PackInviteLinkJoinersState::new(cx)
        });
        let parts = PackInviteLinkJoinersParts {
            link_id: link_id.clone(),
            state: state.clone(),
            nav_store: env::<NavigationStoreKey>(cx),
        };

        let loaded = StickersServicesStore::shared(cx).read(cx).pack_link_joiners(&link_id);
        let load_failure = state.read(cx).load_failure.clone();
        let rows = Rc::new(Self::rows(loaded, load_failure));
        state.update(cx, |state, _| state.sync_list(&rows));

        let back = parts.clone();
        let keys_refresh = parts.clone();
        let appearing = parts.clone();
        let body = PackInviteLinkJoinersBody { parts, rows };
        SettingsWrapperUi::new(tr("sticker_pack_joiners_title"), move |_, cx| {
            if let Some(nav_store) = &back.nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(PageRefreshKeys::new(page_id(&link_id, "keys"), move |_, cx| keys_refresh.load(cx), body))
        .on_appear(page_id(&link_id, "task"), move |_, cx| appearing.load(cx))
    }
}

#[derive(IntoElement)]
struct PackInviteLinkJoinersBody {
    parts: PackInviteLinkJoinersParts,
    rows: Rc<Vec<PackInviteLinkJoinersRow>>,
}

impl RenderOnce for PackInviteLinkJoinersBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let top_inset = list_shell_floating_top_inset(cx);
        let refresh = self.parts.clone();
        let screen_state = self.parts.state.clone();
        let parts = self.parts;
        let rows = self.rows;
        let background = theme.bg._100;
        let list = screen_state.read(cx).list.refreshable(
            page_id(&parts.link_id, "list"),
            move |_, cx| {
                refresh.load(cx);
                Task::ready(())
            },
            move |index, _, _| {
                let Some(row) = rows.get(index) else {
                    return Empty.into_any_element();
                };
                let top_padding = if index == 0 { top_inset + 12.0 } else { 0.0 };
                div().w_full().pt(px(top_padding)).child(joiners_row(row, &parts, &theme)).into_any_element()
            },
        );
        div().size_full().bg(background).child(list)
    }
}

fn joiners_row(row: &PackInviteLinkJoinersRow, parts: &PackInviteLinkJoinersParts, theme: &AppTheme) -> AnyElement {
    match row {
        PackInviteLinkJoinersRow::Loading => PlaylistCollaborationLoadingState::new().into_any_element(),
        PackInviteLinkJoinersRow::Failure(message) => {
            let retry = parts.clone();
            let retry: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| retry.load(cx));
            PlaylistCollaborationErrorState::new(page_id(&parts.link_id, "failure"), message.clone(), Some(retry)).into_any_element()
        }
        PackInviteLinkJoinersRow::Empty => empty_state(theme).into_any_element(),
        PackInviteLinkJoinersRow::Joiner { joiner, index, item_count } => {
            joiner_card_piece(joiner, *index, *item_count, parts, theme).into_any_element()
        }
        PackInviteLinkJoinersRow::BottomSpacer => div().h(px(80.0)).into_any_element(),
    }
}

fn empty_state(theme: &AppTheme) -> Div {
    vstack(12.0)
        .w_full()
        .pt(px(56.0))
        .children(IconUi::sized("person.2", px(44.0), theme.text.secondary, false))
        .child(MainText::new(tr("sticker_pack_joiners_empty")).px(16.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary))
}

fn joiner_card_piece(
    joiner: &RisePackCoauthor,
    index: usize,
    item_count: usize,
    parts: &PackInviteLinkJoinersParts,
    theme: &AppTheme,
) -> Div {
    let is_first = index == 0;
    let is_last = index + 1 == item_count;
    let tap_parts = parts.clone();
    let tap_joiner = joiner.clone();
    let row = PackCollaborationUserRow::new(
        page_id(&parts.link_id, &format!("row-{}", joiner.user_id)),
        joiner.user.clone(),
        joiner.user_id.clone(),
        move |_: &mut Window, cx: &mut App| tap_parts.open_profile(&tap_joiner, cx),
    )
    .trailing(
        MainText::new(PlaylistCollaborationTime::short(joiner.added_at_ms))
            .px(13.0)
            .color(theme.text.secondary)
            .number_of_lines(1),
    );

    let near_end = index + PREFETCH_THRESHOLD >= item_count;
    let more_parts = parts.clone();
    let appearing = div().w_full().child(row).on_appear(page_id(&parts.link_id, &format!("appear-{}", joiner.user_id)), move |_, cx| {
        if near_end {
            more_parts.load_more(cx);
        }
    });

    let card = div()
        .w_full()
        .bg(theme.bg._200)
        .when(is_first, |card| card.rounded_t(px(CARD_CORNER_RADIUS)))
        .when(is_last, |card| card.rounded_b(px(CARD_CORNER_RADIUS)))
        .child(appearing)
        .when(!is_last, |card| card.child(PackCollaborationRowDivider::new()));
    div().w_full().px(px(14.0)).child(card)
}
