use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Context, ElementId, Empty, Entity, FontWeight, ListAlignment, Pixels, Point, SharedString, Subscription, Task, Window};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_platform::haptics::{NotificationKind, notification};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::music::pages::playlist_collaboration::playlist_collaboration_components::{
    PlaylistCollaborationErrorState, PlaylistCollaborationLoadingState, PlaylistCollaborationUserLabel,
    PlaylistMemberStatusBadge,
};
use crate::modules::music::shared::utils::music::music_pagination_prefetch::MusicPaginationPrefetchModifiers;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_types::MusicPlaylistMember;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

const SECTIONS_SPACING: f32 = 18.0;
const SECTION_HEADER_SPACING: f32 = 8.0;
const CARD_CORNER_RADIUS: f32 = 22.0;
const EMPTY_TOP_PADDING: f32 = 58.0;
const ONLY_OWNER_TOP_PADDING: f32 = 24.0;
const PREFETCH_THRESHOLD: usize = 4;

/// one row of the flattened screen
/// gpui lists rows and not nested sections so a header and its people
/// follow each other and the card is drawn piece by piece
#[derive(Clone)]
enum PlaylistMembersRow {
    Loading,
    Error,
    Empty { text_key: &'static str, top_padding: f32 },
    SectionHeader { title: String, count: i64, top_spacing: f32 },
    Member { member: MusicPlaylistMember, index: usize, item_count: usize, is_guide_first: bool },
    BottomSpacer,
}

impl PlaylistMembersRow {
    /// height of a row with a given key doesnt change
    fn key(&self) -> String {
        match self {
            Self::Loading => "loading".to_owned(),
            Self::Error => "error".to_owned(),
            Self::Empty { text_key, .. } => format!("empty-{text_key}"),
            Self::SectionHeader { title, .. } => format!("header-{title}"),
            Self::Member { member, index, item_count, .. } => {
                format!("member-{}-{}-{}", member.user_id, member.version, *index + 1 == *item_count)
            }
            Self::BottomSpacer => "bottom".to_owned(),
        }
    }
}

struct PlaylistMembersScreenState {
    list: ListUi,
    row_keys: Vec<String>,
    removing_user_id: Option<String>,
    _subscriptions: [Subscription; 3],
}

impl PlaylistMembersScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = MusicServicesStore::shared(cx);
        let actions = MusicActionsStore::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        Self {
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            row_keys: Vec::new(),
            removing_user_id: None,
            _subscriptions: [
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&sandbox, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn sync_list(&mut self, rows: &[PlaylistMembersRow]) {
        let keys: Vec<String> = rows.iter().map(PlaylistMembersRow::key).collect();
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
struct PlaylistMembersScreenParts {
    playlist_id: String,
    state: Entity<PlaylistMembersScreenState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl PlaylistMembersScreenParts {
    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn push_user_page(&self, member: &MusicPlaylistMember, cx: &mut App) {
        self.push(RootRoute::UserPage { user_id: Some(member.user.id.clone()), tag: Some(member.user.tag.clone()) }, cx);
    }

    fn push_rights(&self, member: &MusicPlaylistMember, cx: &mut App) {
        self.push(
            RootRoute::MusicPlaylistMemberRights {
                playlist_id: self.playlist_id.clone(),
                member_user_id: member.user_id.clone(),
            },
            cx,
        );
    }

    fn handle(&self, member: &MusicPlaylistMember, action: &str, cx: &mut App) {
        match action {
            "profile" => self.push_user_page(member, cx),
            "rights" => self.push_rights(member, cx),
            "remove" => self.confirm_removal(member.clone(), cx),
            _ => {}
        }
    }

    fn open(&self, member: &MusicPlaylistMember, can_manage: bool, cx: &mut App) {
        if can_manage && !member.is_owner() {
            self.push_rights(member, cx);
        } else {
            self.push_user_page(member, cx);
        }
    }

    fn confirm_removal(&self, member: MusicPlaylistMember, cx: &mut App) {
        let parts = self.clone();
        NativeConfirmAlert::present_delete(
            &tr("music_playlist_member_remove_title"),
            &tr("music_playlist_member_remove_message"),
            Some(tr("music_playlist_member_remove_confirm")),
            None,
            false,
            None,
            None,
            move |cx| parts.remove(&member, cx),
            cx,
        );
    }

    fn load(&self, force_refresh: bool, cx: &mut App) {
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_playlist_settings(&self.playlist_id, force_refresh, cx);
            interactions.load_playlist_members(&self.playlist_id, true, force_refresh, cx);
        });
    }

    fn remove(&self, member: &MusicPlaylistMember, cx: &mut App) {
        if self.state.read(cx).removing_user_id.is_some() {
            return;
        }
        self.state.update(cx, |state, cx| {
            state.removing_user_id = Some(member.user_id.clone());
            cx.notify();
        });
        let success_state = self.state.clone();
        let error_state = self.state.clone();
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.remove_playlist_member(
                &self.playlist_id,
                &member.user_id,
                Some(Rc::new(move |cx: &mut App| {
                    success_state.update(cx, |state, cx| {
                        state.removing_user_id = None;
                        cx.notify();
                    });
                    notification(NotificationKind::Success);
                    Notifier::send(
                        tr("music_playlist_member_removed"),
                        NotifierSendOptions {
                            r#type: NotifierType::Success,
                            position: NotifierPosition::Bottom,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                })),
                Some(Rc::new(move |error: String, cx: &mut App| {
                    error_state.update(cx, |state, cx| {
                        state.removing_user_id = None;
                        cx.notify();
                    });
                    Notifier::send(
                        tr("music_playlist_load_error"),
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
pub struct PlaylistMembersScreen {
    playlist_id: String,
}

impl PlaylistMembersScreen {
    pub fn new(playlist_id: impl Into<String>) -> Self {
        Self { playlist_id: playlist_id.into() }
    }

    fn rows(
        members: Vec<MusicPlaylistMember>,
        server_counts: Option<(i64, i64)>,
        is_pending: bool,
        error: Option<String>,
    ) -> Vec<PlaylistMembersRow> {
        let mut rows = Vec::new();
        if members.is_empty() && is_pending {
            rows.push(PlaylistMembersRow::Loading);
        } else if members.is_empty() && error.is_some_and(|error| !error.is_empty()) {
            rows.push(PlaylistMembersRow::Error);
        } else if members.is_empty() {
            rows.push(Self::empty_state("music_playlist_members_empty", EMPTY_TOP_PADDING));
        } else {
            Self::members_content(members, server_counts, &mut rows);
        }
        rows.push(PlaylistMembersRow::BottomSpacer);
        rows
    }

    fn empty_state(text_key: &'static str, top_padding: f32) -> PlaylistMembersRow {
        PlaylistMembersRow::Empty { text_key, top_padding }
    }

    fn members_content(
        members: Vec<MusicPlaylistMember>,
        server_counts: Option<(i64, i64)>,
        rows: &mut Vec<PlaylistMembersRow>,
    ) {
        let (active_members, suspended_members): (Vec<_>, Vec<_>) =
            members.into_iter().partition(MusicPlaylistMember::has_active_access);
        let has_only_owner = suspended_members.is_empty() && active_members.iter().all(MusicPlaylistMember::is_owner);

        let active_count = (active_members.len() as i64).max(server_counts.map_or(0, |(active, _)| active));
        Self::member_section(active_members, tr("music_playlist_members_active"), active_count, 0.0, rows);

        if !suspended_members.is_empty() {
            let suspended_count =
                (suspended_members.len() as i64).max(server_counts.map_or(0, |(active, total)| total - active));
            Self::member_section(
                suspended_members,
                tr("music_playlist_members_suspended"),
                suspended_count,
                SECTIONS_SPACING,
                rows,
            );
        }

        if has_only_owner {
            rows.push(Self::empty_state("music_playlist_invite_secure", SECTIONS_SPACING + ONLY_OWNER_TOP_PADDING));
        }
    }

    fn member_section(
        section_members: Vec<MusicPlaylistMember>,
        title: String,
        count: i64,
        top_spacing: f32,
        rows: &mut Vec<PlaylistMembersRow>,
    ) {
        rows.push(PlaylistMembersRow::SectionHeader { title, count, top_spacing });
        let first_non_owner = section_members.iter().find(|member| !member.is_owner()).map(|member| member.user_id.clone());
        let item_count = section_members.len();
        rows.extend(section_members.into_iter().enumerate().map(|(index, member)| {
            let is_guide_first = first_non_owner.as_deref() == Some(member.user_id.as_str());
            PlaylistMembersRow::Member { member, index, item_count, is_guide_first }
        }));
    }
}

impl RenderOnce for PlaylistMembersScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let screen_id = format!("playlist-members-{}", self.playlist_id);
        let state = window.use_keyed_state(SharedString::from(format!("{screen_id}-state")), cx, |_, cx| {
            PlaylistMembersScreenState::new(cx)
        });
        let parts = PlaylistMembersScreenParts {
            playlist_id: self.playlist_id,
            state: state.clone(),
            nav_store: env::<NavigationStoreKey>(cx),
        };

        let services = MusicServicesStore::shared(cx);
        let (settings, mut members, server_counts) = services.update(cx, |services, cx| {
            (
                services.playlist_settings(&parts.playlist_id, cx),
                services.playlist_members(&parts.playlist_id, true, cx),
                services.playlist_member_counts(&parts.playlist_id, true, cx),
            )
        });
        let mut seen = HashSet::new();
        members.retain(|member| seen.insert(member.user_id.clone()));
        let can_manage = settings.is_some_and(|settings| settings.is_owner());
        let slice = MusicActionsStore::slices(cx).get_playlist_members_sai.clone();
        let rows = Rc::new(PlaylistMembersScreen::rows(members, server_counts, slice.is_pending(), slice.error()));
        state.update(cx, |state, _| state.sync_list(&rows));

        let back = parts.clone();
        let keys_refresh = parts.clone();
        let appearing = parts.clone();
        let body = PlaylistMembersScreenBody { parts, rows, can_manage };
        SettingsWrapperUi::new(tr("music_playlist_members_title"), move |_, cx| {
            if let Some(nav_store) = &back.nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(PageRefreshKeys::new(
            SharedString::from(format!("{screen_id}-keys")),
            move |_, cx| keys_refresh.load(true, cx),
            body,
        ))
        .on_appear(SharedString::from(format!("{screen_id}-task")), move |_, cx| appearing.load(true, cx))
    }
}

#[derive(IntoElement)]
struct PlaylistMembersScreenBody {
    parts: PlaylistMembersScreenParts,
    rows: Rc<Vec<PlaylistMembersRow>>,
    can_manage: bool,
}

impl RenderOnce for PlaylistMembersScreenBody {
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
            "playlist-members-list",
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
                    .child(playlist_members_row(row, &parts, can_manage, &theme, cx))
                    .into_any_element()
            },
        );
        div().size_full().bg(background).child(list)
    }
}

fn playlist_members_row(
    row: &PlaylistMembersRow,
    parts: &PlaylistMembersScreenParts,
    can_manage: bool,
    theme: &AppTheme,
    cx: &mut App,
) -> AnyElement {
    match row {
        PlaylistMembersRow::Loading => PlaylistCollaborationLoadingState::new().into_any_element(),
        PlaylistMembersRow::Error => {
            let retry = parts.clone();
            let retry: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| retry.load(false, cx));
            PlaylistCollaborationErrorState::new("playlist-members-error", tr("music_playlist_load_error"), Some(retry))
                .into_any_element()
        }
        PlaylistMembersRow::Empty { text_key, top_padding } => empty_state(text_key, *top_padding, theme).into_any_element(),
        PlaylistMembersRow::SectionHeader { title, count, top_spacing } => hstack(0.0)
            .w_full()
            .px(px(16.0))
            .pt(px(*top_spacing))
            .pb(px(SECTION_HEADER_SPACING))
            .child(MainText::new(title.clone()).px(12.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.secondary))
            .child(spacer())
            .child(MainText::new(count.to_string()).px(12.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.secondary))
            .into_any_element(),
        PlaylistMembersRow::Member { member, index, item_count, is_guide_first } => {
            member_card_piece(member, *index, *item_count, *is_guide_first, parts, can_manage, theme, cx).into_any_element()
        }
        PlaylistMembersRow::BottomSpacer => div().h(px(80.0)).into_any_element(),
    }
}

fn empty_state(text_key: &str, top_padding: f32, theme: &AppTheme) -> impl IntoElement {
    vstack(12.0)
        .w_full()
        .pt(px(top_padding))
        .children(IconUi::sized("person.2", px(48.0), theme.text.secondary, false))
        .child(MainText::new(tr(text_key)).px(16.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary))
}

#[allow(clippy::too_many_arguments)]
fn member_card_piece(
    member: &MusicPlaylistMember,
    index: usize,
    item_count: usize,
    is_guide_first: bool,
    parts: &PlaylistMembersScreenParts,
    can_manage: bool,
    theme: &AppTheme,
    cx: &mut App,
) -> impl IntoElement {
    let is_first = index == 0;
    let is_last = index + 1 == item_count;
    let removing = parts.state.read(cx).removing_user_id.as_deref() == Some(member.user_id.as_str());
    let user_id = member.user_id.clone();
    let actions = member_actions(member, can_manage);

    let (row_member, row_theme) = (member.clone(), theme.clone());
    let (action_parts, action_member) = (parts.clone(), member.clone());
    let (tap_parts, tap_member) = (parts.clone(), member.clone());
    let hold = HoldContextMenuContainer::new(
        SharedString::from(format!("playlist-member-hold-{user_id}")),
        HoldContextMenuAlignment::Leading,
        Rc::new(move |_: &App| actions.clone()),
        Rc::new(move |_: &mut Window, _: &mut App| {
            member_row(&row_member, can_manage, removing, is_first, is_last, &row_theme).into_any_element()
        }),
    )
    .show_emoji_list(false)
    .on_action(Rc::new(move |action: String, _: &mut Window, cx: &mut App| {
        action_parts.handle(&action_member, &action, cx);
    }))
    .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
        tap_parts.open(&tap_member, can_manage, cx);
    }));

    let more_parts = parts.clone();
    let cache_key = MusicServicesStore::playlist_members_cache_key(&parts.playlist_id, true);
    let prefetched = hold.guide_anchor(is_guide_first.then_some(RiseGuideAnchor::PlaylistMembersFirst)).music_pagination_prefetch(
        ElementId::Name(SharedString::from(format!("playlist-member-prefetch-{user_id}"))),
        index,
        item_count,
        &cache_key,
        true,
        PREFETCH_THRESHOLD,
        move |_, cx| {
            MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.load_more_playlist_members(&more_parts.playlist_id, true, cx);
            });
        },
        cx,
    );

    let card = div()
        .w_full()
        .bg(theme.bg._200)
        .when(is_first, |card| card.rounded_t(px(CARD_CORNER_RADIUS)))
        .when(is_last, |card| card.rounded_b(px(CARD_CORNER_RADIUS)))
        .child(prefetched)
        .when(!is_last, |card| {
            card.child(div().w_full().pl(px(70.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55))))
        });
    div().w_full().px(px(12.0)).child(card)
}

fn member_row(
    member: &MusicPlaylistMember,
    can_manage: bool,
    removing: bool,
    is_first: bool,
    is_last: bool,
    theme: &AppTheme,
) -> impl IntoElement {
    let user = &member.user;
    let name_color = user
        .is_premium
        .then(|| PremiumNameColor::parse(user.name_color.as_deref()))
        .flatten()
        .unwrap_or(theme.text.primary);
    let mut name = hstack(5.0).min_w_0().max_w_full().child(
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
    let mut identity =
        vstack(4.0).align(HorizontalAlignment::Leading).flex_1().min_w_0().overflow_hidden().child(name);
    if !user.tag.is_empty() {
        identity = identity
            .child(MainText::new(format!("@{}", user.tag)).px(13.0).color(theme.text.secondary).number_of_lines(1));
    }
    let chevron = (can_manage && !member.is_owner())
        .then(|| IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(11.0), theme.text.secondary.opacity(0.7), false))
        .flatten()
        .map(|icon| icon.flex_none());
    hstack(12.0)
        .w_full()
        .px(px(14.0))
        .py(px(11.0))
        .bg(theme.bg._200)
        .when(is_first, |row| row.rounded_t(px(CARD_CORNER_RADIUS)))
        .when(is_last, |row| row.rounded_b(px(CARD_CORNER_RADIUS)))
        .opacity(if removing { 0.5 } else { 1.0 })
        .child(
            UserLogoUi::new(SharedString::from(format!("playlist-member-logo-{}", user.id)))
                .source(SharedString::from(user.logo.clone()))
                .name(SharedString::from(user.name.clone()))
                .user_id(SharedString::from(user.id.clone()))
                .size(46.0),
        )
        .child(identity)
        .child(div().flex_none().w(px(8.0)))
        .child(div().flex_none().child(PlaylistMemberStatusBadge::new(member.role.clone(), member.status.clone())))
        .children(chevron)
}

fn member_actions(member: &MusicPlaylistMember, can_manage: bool) -> Vec<HoldContextMenuAction> {
    let mut result =
        vec![HoldContextMenuAction::new("profile", tr("ctx_open_profile")).system_image("person.crop.circle")];
    if can_manage && !member.is_owner() {
        result.push(
            HoldContextMenuAction::new("rights", tr("music_playlist_member_rights_title"))
                .system_image("slider.horizontal.3"),
        );
        result.push(
            HoldContextMenuAction::new("remove", tr("music_playlist_member_remove"))
                .system_image("person.crop.circle.badge.minus")
                .destructive(true)
                .section_break(true),
        );
    }
    result
}
