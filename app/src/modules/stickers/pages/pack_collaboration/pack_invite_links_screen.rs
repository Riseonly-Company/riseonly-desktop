use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Context, ElementId, Empty, Entity, FontWeight, Hsla, ListAlignment, Pixels, Point, SharedString, Subscription, Task, Window, white};
use rise_core::foundation::foundation_date::Date;
use rise_i18n::{FormatArg, tr, tr_args, tr_plural};
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle, NotificationKind, notification};
use rise_platform::pasteboard::UIPasteboard;
use rise_platform::system_share::ShareItem;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{NativeBottomSheetPresentationHost, PresentationDetent};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::sheets::share_sheet::SystemShare;
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::music::pages::playlist_collaboration::playlist_collaboration_components::{
    PlaylistCollaborationErrorState, PlaylistCollaborationLoadingState,
};
use crate::modules::stickers::engine::rise_pack_collaboration_presentation::RisePackCollaborationPresentation;
use crate::modules::stickers::engine::rise_pack_collaboration_repository::{RisePackInviteLink, RisePackInviteStats};
use crate::modules::stickers::pages::pack_collaboration::pack_invite_link_editor_sheet::PackInviteLinkEditorSheet;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_coauthors_interactions::PackCollaborationFailureCallback;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

const SUBTITLE_SEPARATOR: &str = " \u{b7} ";
const BLOCK_SPACING: f32 = 18.0;
const SECTION_TITLE_SPACING: f32 = 8.0;
const LIST_CORNER_RADIUS: f32 = 20.0;
const PREFETCH_THRESHOLD: usize = 4;

fn page_id(pack_id: &str, name: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("pack-invite-links-{pack_id}-{name}")))
}

#[derive(Clone, PartialEq)]
pub enum PackInviteLinkEditorTarget {
    Create,
    Edit(RisePackInviteLink),
}

impl PackInviteLinkEditorTarget {
    pub fn id(&self) -> String {
        match self {
            Self::Create => "create".to_owned(),
            Self::Edit(link) => link.id.clone(),
        }
    }
}

/// one row of the flattened screen,
/// each block and link of a card follow each other so the card is drawn piece by piece
#[derive(Clone)]
enum PackInviteLinksRow {
    Loading,
    Failure(String),
    Primary(RisePackInviteLink),
    Stats { stats: RisePackInviteStats, top_spacing: f32 },
    Create { top_spacing: f32 },
    SectionTitle { title: String, top_spacing: f32 },
    Link { link: RisePackInviteLink, section: &'static str, index: usize, item_count: usize, top_spacing: f32 },
    BottomSpacer,
}

impl PackInviteLinksRow {
    /// height of a row with a given key never changes
    fn key(&self) -> String {
        match self {
            Self::Loading => "loading".to_owned(),
            Self::Failure(message) => format!("failure-{message}"),
            Self::Primary(link) => format!("primary-{}-{}", link.id, link.used_count > 0),
            Self::Stats { top_spacing, .. } => format!("stats-{top_spacing}"),
            Self::Create { top_spacing } => format!("create-{top_spacing}"),
            Self::SectionTitle { title, .. } => format!("title-{title}"),
            Self::Link { link, section, index, item_count, top_spacing } => {
                format!("link-{section}-{}-{}-{top_spacing}", link.id, *index + 1 == *item_count)
            }
            Self::BottomSpacer => "bottom".to_owned(),
        }
    }
}

/// load failure and editor state plus the list the rows live in
struct PackInviteLinksScreenState {
    list: ListUi,
    row_keys: Vec<String>,
    load_failure: Option<String>,
    editor: Option<PackInviteLinkEditorTarget>,
    _subscriptions: [Subscription; 3],
}

impl PackInviteLinksScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = StickersServicesStore::shared(cx);
        let presentation = RisePackCollaborationPresentation::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        Self {
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            row_keys: Vec::new(),
            load_failure: None,
            editor: None,
            _subscriptions: [
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&presentation, |_, _, cx| cx.notify()),
                cx.observe(&hold_context_menu, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn sync_list(&mut self, rows: &[PackInviteLinksRow]) {
        let keys: Vec<String> = rows.iter().map(PackInviteLinksRow::key).collect();
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
struct PackInviteLinksParts {
    pack_id: String,
    state: Entity<PackInviteLinksScreenState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl PackInviteLinksParts {
    fn load(&self, force: bool, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.load_failure = None;
            cx.notify();
        });
        let failure_state = self.state.clone();
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_pack_invite_links(
                &self.pack_id,
                force,
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
            interactions.load_more_pack_invite_links(&self.pack_id, cx);
        });
    }

    fn open_editor(&self, target: PackInviteLinkEditorTarget, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.editor = Some(target);
            cx.notify();
        });
    }

    fn link_action(&self, link: &RisePackInviteLink, id: &str, cx: &mut App) {
        match id {
            "copy" => copy(&share_url(link), cx),
            "joiners" => {
                if let Some(nav_store) = &self.nav_store {
                    let route = RootRoute::StickerPackInviteLinkJoiners { pack_id: self.pack_id.clone(), link_id: link.id.clone() };
                    nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
                }
            }
            "edit" => self.open_editor(PackInviteLinkEditorTarget::Edit(link.clone()), cx),
            "revoke" => Self::confirm_revoke(link.clone(), cx),
            _ => {}
        }
    }

    fn confirm_revoke(link: RisePackInviteLink, cx: &mut App) {
        NativeConfirmAlert::present_delete(
            &tr("chat_links_revoke"),
            &tr("sticker_pack_invite_link_revoke_confirm"),
            Some(tr("chat_links_revoke")),
            None,
            false,
            None,
            None,
            move |cx| revoke(&link, cx),
            cx,
        );
    }
}

fn revoke(link: &RisePackInviteLink, cx: &mut App) {
    StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
        interactions.revoke_pack_invite_link(
            link,
            |_: &mut App| {
                let _ = notification(NotificationKind::Success);
            },
            |text: String, cx: &mut App| {
                Notifier::send(
                    text,
                    NotifierSendOptions {
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

fn share_url(link: &RisePackInviteLink) -> String {
    RiseonlyPublicLink::normalized_join_token(&link.link)
        .and_then(|token| RiseonlyPublicLink::StickerPackInvite { token }.web_url(AppEnvironment::current()))
        .map(|url| url.absolute_string().to_owned())
        .unwrap_or_else(|| link.link.clone())
}

fn share(link: &RisePackInviteLink, cx: &mut App) {
    SystemShare::present(vec![ShareItem::Link(share_url(link))], None, cx);
}

fn copy(value: &str, cx: &mut App) {
    UIPasteboard::general().set_string(value, cx);
    let _ = ImpactFeedbackGenerator::new(ImpactStyle::Light).impact_occurred();
    Notifier::send(
        tr("sticker_pack_invite_link_copied"),
        NotifierSendOptions { r#type: NotifierType::Success, position: NotifierPosition::Bottom, ..NotifierSendOptions::default() },
        cx,
    );
}

fn subtitle(link: &RisePackInviteLink) -> String {
    let mut parts: Vec<String> = Vec::new();
    if link.used_count > 0 {
        parts.push(tr_plural("chat_links_joined_count", link.used_count));
    } else {
        parts.push(tr("chat_links_no_joins"));
    }
    if PackInviteLinksScreen::is_active(link)
        && let Some(limit) = link.usage_limit
        && limit > 0
    {
        parts.push(tr_args("chat_links_remaining", &[FormatArg::Int((limit - link.used_count).max(0))]));
    }
    if link.creates_join_request {
        parts.push(tr("chat_links_requires_approval"));
    }
    if link.is_revoked {
        parts.push(tr("chat_links_revoked"));
    } else if !PackInviteLinksScreen::is_active(link) {
        parts.push(tr("chat_links_expired"));
    }
    parts.join(SUBTITLE_SEPARATOR)
}

fn link_actions(link: &RisePackInviteLink) -> Vec<HoldContextMenuAction> {
    let mut actions = vec![HoldContextMenuAction::new("copy", tr("chat_links_copy")).system_image("doc.on.doc")];
    if link.used_count > 0 {
        actions.push(HoldContextMenuAction::new("joiners", tr("sticker_pack_joiners_title")).system_image("person.2"));
    }
    if PackInviteLinksScreen::is_active(link) {
        actions.push(HoldContextMenuAction::new("edit", tr("sticker_pack_invite_link_edit")).system_image("pencil"));
    }
    if !link.is_revoked {
        actions.push(
            HoldContextMenuAction::new("revoke", tr("chat_links_revoke"))
                .system_image("trash")
                .destructive(true)
                .section_break(true),
        );
    }
    actions
}

fn card_button_label(icon: &'static str, title: String, theme: &AppTheme) -> Div {
    let foreground = theme.content_on(theme.primary._100);
    hstack(7.0)
        .w_full()
        .min_h(px(44.0))
        .justify_center()
        .bg(theme.primary._100)
        .capsule()
        .children(IconUi::sized(icon, px(15.0), foreground, false))
        .child(MainText::new(title).px(15.0).font_weight(FontWeight::SEMIBOLD).color(foreground))
}

fn link_row(link: &RisePackInviteLink, theme: &AppTheme) -> Div {
    let tile: Hsla = if PackInviteLinksScreen::is_active(link) { ChatTileColor::blue() } else { ChatTileColor::gray() };
    let title = match link.name.as_deref() {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => share_url(link),
    };
    hstack(12.0)
        .w_full()
        .px(px(16.0))
        .py(px(11.0))
        .child(
            div()
                .size(px(29.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_rect(8.0)
                .bg(tile)
                .children(IconUi::sized("link", px(14.0), white(), false)),
        )
        .child(
            vstack(2.0)
                .flex_1()
                .min_w_0()
                .align(HorizontalAlignment::Leading)
                .child(MainText::new(title).px(15.0).color(theme.text.primary).number_of_lines(1))
                .child(MainText::new(subtitle(link)).px(12.0).color(theme.text.secondary).number_of_lines(1)),
        )
        .child(spacer_min_w(4.0).flex_none())
}

fn primary_card(link: &RisePackInviteLink, pack_id: &str, theme: &AppTheme) -> Div {
    let url = share_url(link);
    let copy_url = url.clone();
    let share_link = link.clone();
    vstack(12.0)
        .w_full()
        .px(px(12.0))
        .child(
            div().w_full().py(px(13.0)).px(px(14.0)).bg(theme.bg._200).rounded_rect(14.0).child(
                MainText::new(url).px(15.0).tac(TextAlignT::Center).color(theme.text.primary).number_of_lines(1),
            ),
        )
        .child(
            hstack(10.0)
                .w_full()
                .child(
                    div().flex_1().min_w_0().child(
                        plain(page_id(pack_id, "copy"))
                            .w_full()
                            .on_press(move |_, cx| copy(&copy_url, cx))
                            .child(card_button_label("doc.on.doc", tr("chat_links_copy"), theme)),
                    ),
                )
                .child(
                    div().flex_1().min_w_0().child(
                        plain(page_id(pack_id, "share"))
                            .w_full()
                            .on_press(move |_, cx| share(&share_link, cx))
                            .child(card_button_label("square.and.arrow.up", tr("share_action"), theme)),
                    ),
                ),
        )
        .when(link.used_count > 0, |card| {
            card.child(MainText::new(tr_plural("chat_links_joined_count", link.used_count)).px(13.0).color(theme.primary._100))
        })
}

fn stat_cell(value: i64, key: &str, theme: &AppTheme) -> Div {
    vstack(3.0)
        .flex_1()
        .min_w_0()
        .px(px(6.0))
        .child(MainText::new(value.to_string()).px(18.0).font_weight(FontWeight::BOLD).color(theme.text.primary))
        .child(MainText::new(tr(key)).px(11.0).tac(TextAlignT::Center).color(theme.text.secondary).number_of_lines(2))
}

fn stats_card(stats: &RisePackInviteStats, theme: &AppTheme) -> Div {
    vstack(8.0)
        .w_full()
        .align(HorizontalAlignment::Leading)
        .child(div().px(px(16.0)).child(MainText::new(tr("sticker_pack_invite_stats_total")).px(12.0).color(theme.text.secondary)))
        .child(
            div().w_full().px(px(12.0)).child(
                vstack(14.0)
                    .w_full()
                    .py(px(12.0))
                    .bg(theme.bg._200)
                    .rounded_rect(20.0)
                    .child(
                        hstack(0.0)
                            .w_full()
                            .child(stat_cell(stats.total_joins, "sticker_pack_invite_stats_joins", theme))
                            .child(stat_cell(stats.joins_last24h, "sticker_pack_invite_stats_day", theme)),
                    )
                    .child(
                        hstack(0.0)
                            .w_full()
                            .child(stat_cell(stats.joins_last_week, "sticker_pack_invite_stats_week", theme))
                            .child(stat_cell(stats.active_links, "sticker_pack_invite_stats_active_links", theme)),
                    ),
            ),
        )
}

fn link_card_piece(
    link: &RisePackInviteLink,
    section: &'static str,
    index: usize,
    item_count: usize,
    parts: &PackInviteLinksParts,
    theme: &AppTheme,
) -> Div {
    let is_first = index == 0;
    let is_last = index + 1 == item_count;
    let actions = link_actions(link);
    let (row_link, row_theme) = (link.clone(), theme.clone());
    let (action_parts, action_link) = (parts.clone(), link.clone());
    let tap_url = share_url(link);
    let hold = HoldContextMenuContainer::new(
        page_id(&parts.pack_id, &format!("{section}-hold-{}", link.id)),
        HoldContextMenuAlignment::Leading,
        Rc::new(move |_: &App| actions.clone()),
        Rc::new(move |_: &mut Window, _: &mut App| link_row(&row_link, &row_theme).into_any_element()),
    )
    .show_emoji_list(false)
    .lifted_background(Some(theme.bg._200))
    .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| action_parts.link_action(&action_link, &id, cx)))
    .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| copy(&tap_url, cx)));

    let near_end = index + PREFETCH_THRESHOLD >= item_count;
    let more_parts = parts.clone();
    let appearing = div().w_full().child(hold).on_appear(page_id(&parts.pack_id, &format!("{section}-appear-{}", link.id)), move |_, cx| {
        if near_end {
            more_parts.load_more(cx);
        }
    });

    let card = div()
        .w_full()
        .bg(theme.bg._200)
        .when(is_first, |card| card.rounded_t(px(LIST_CORNER_RADIUS)))
        .when(is_last, |card| card.rounded_b(px(LIST_CORNER_RADIUS)))
        .child(appearing)
        .when(!is_last, |card| {
            card.child(div().w_full().pl(px(52.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55))))
        });
    div().w_full().px(px(12.0)).child(card)
}

fn links_row(row: &PackInviteLinksRow, parts: &PackInviteLinksParts, theme: &AppTheme) -> AnyElement {
    match row {
        PackInviteLinksRow::Loading => PlaylistCollaborationLoadingState::new().into_any_element(),
        PackInviteLinksRow::Failure(message) => {
            let retry = parts.clone();
            let retry: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| retry.load(true, cx));
            PlaylistCollaborationErrorState::new(page_id(&parts.pack_id, "failure"), message.clone(), Some(retry)).into_any_element()
        }
        PackInviteLinksRow::Primary(link) => primary_card(link, &parts.pack_id, theme).into_any_element(),
        PackInviteLinksRow::Stats { stats, top_spacing } => {
            div().w_full().pt(px(*top_spacing)).child(stats_card(stats, theme)).into_any_element()
        }
        PackInviteLinksRow::Create { top_spacing } => {
            let create = parts.clone();
            div()
                .w_full()
                .pt(px(*top_spacing))
                .child(
                    GroupedBtns::new(
                        page_id(&parts.pack_id, "create-group"),
                        vec![
                            GroupedBtn::new("create", tr("chat_links_create"))
                                .icon("plus")
                                .icon_background(ChatTileColor::blue())
                                .show_chevron(false)
                                .callback(move |_, cx| create.open_editor(PackInviteLinkEditorTarget::Create, cx)),
                        ],
                    )
                    .title(tr("chat_links_additional_section")),
                )
                .into_any_element()
        }
        PackInviteLinksRow::SectionTitle { title, top_spacing } => div()
            .w_full()
            .pt(px(*top_spacing))
            .pb(px(SECTION_TITLE_SPACING))
            .px(px(28.0))
            .child(MainText::new(title.clone()).px(12.0).color(theme.text.secondary))
            .into_any_element(),
        PackInviteLinksRow::Link { link, section, index, item_count, top_spacing } => div()
            .w_full()
            .pt(px(*top_spacing))
            .child(link_card_piece(link, section, *index, *item_count, parts, theme))
            .into_any_element(),
        PackInviteLinksRow::BottomSpacer => div().h(px(80.0)).into_any_element(),
    }
}

/// invite links of a pack with join stats,
/// primary link card then totals then extra links then revoked or exhausted ones
#[derive(IntoElement)]
pub struct PackInviteLinksScreen {
    pack_id: String,
}

impl PackInviteLinksScreen {
    pub fn new(pack_id: impl Into<String>) -> Self {
        Self { pack_id: pack_id.into() }
    }

    pub fn is_active(link: &RisePackInviteLink) -> bool {
        if link.is_revoked {
            return false;
        }
        if let Some(expires) = link.expires_at_ms
            && expires <= (Date::now().time_interval_since_1970() * 1000.0) as i64
        {
            return false;
        }
        if let Some(limit) = link.usage_limit
            && limit > 0
            && link.used_count >= limit
        {
            return false;
        }
        true
    }

    fn rows(links: Option<Vec<RisePackInviteLink>>, stats: Option<RisePackInviteStats>, load_failure: Option<String>) -> Vec<PackInviteLinksRow> {
        let mut rows = Vec::new();
        let Some(mut links) = links else {
            rows.push(match load_failure {
                Some(message) => PackInviteLinksRow::Failure(message),
                None => PackInviteLinksRow::Loading,
            });
            rows.push(PackInviteLinksRow::BottomSpacer);
            return rows;
        };
        let mut seen = HashSet::new();
        links.retain(|link| seen.insert(link.id.clone()));
        let primary = links.iter().find(|link| link.is_primary && Self::is_active(link)).cloned();
        let primary_id = primary.as_ref().map(|link| link.id.clone());
        let additional: Vec<RisePackInviteLink> =
            links.iter().filter(|link| Self::is_active(link) && Some(&link.id) != primary_id.as_ref()).cloned().collect();
        let inactive: Vec<RisePackInviteLink> = links.iter().filter(|link| !Self::is_active(link)).cloned().collect();

        let mut has_block = false;
        if let Some(primary) = primary {
            rows.push(PackInviteLinksRow::Primary(primary));
            has_block = true;
        }
        if let Some(stats) = stats {
            rows.push(PackInviteLinksRow::Stats { stats, top_spacing: if has_block { BLOCK_SPACING } else { 0.0 } });
            has_block = true;
        }
        rows.push(PackInviteLinksRow::Create { top_spacing: if has_block { BLOCK_SPACING } else { 0.0 } });
        Self::push_links(&mut rows, additional, "additional", None);
        Self::push_links(&mut rows, inactive, "inactive", Some(tr("chat_links_inactive_section")));
        rows.push(PackInviteLinksRow::BottomSpacer);
        rows
    }

    fn push_links(rows: &mut Vec<PackInviteLinksRow>, items: Vec<RisePackInviteLink>, section: &'static str, title: Option<String>) {
        if items.is_empty() {
            return;
        }
        let has_title = title.is_some();
        if let Some(title) = title {
            rows.push(PackInviteLinksRow::SectionTitle { title, top_spacing: BLOCK_SPACING });
        }
        let item_count = items.len();
        rows.extend(items.into_iter().enumerate().map(|(index, link)| {
            let top_spacing = if index == 0 && !has_title { BLOCK_SPACING } else { 0.0 };
            PackInviteLinksRow::Link { link, section, index, item_count, top_spacing }
        }));
    }
}

impl RenderOnce for PackInviteLinksScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let pack_id = self.pack_id;
        let state = window.use_keyed_state(page_id(&pack_id, "state"), cx, |_, cx| PackInviteLinksScreenState::new(cx));
        let parts = PackInviteLinksParts { pack_id: pack_id.clone(), state: state.clone(), nav_store: env::<NavigationStoreKey>(cx) };

        let (links, stats) = {
            let services = StickersServicesStore::shared(cx);
            let services = services.read(cx);
            (services.pack_invite_links(&pack_id), services.pack_invite_stats(&pack_id))
        };
        let load_failure = state.read(cx).load_failure.clone();
        let rows = Rc::new(Self::rows(links, stats, load_failure));
        state.update(cx, |state, _| state.sync_list(&rows));

        let back = parts.clone();
        let keys_refresh = parts.clone();
        let appearing = parts.clone();
        let body = PackInviteLinksBody { parts, rows };
        let shell = SettingsWrapperUi::new(tr("sticker_pack_invite_links_title"), move |_, cx| {
            if let Some(nav_store) = &back.nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(PageRefreshKeys::new(page_id(&pack_id, "keys"), move |_, cx| keys_refresh.load(true, cx), body))
        .on_appear(page_id(&pack_id, "task"), move |_, cx| appearing.load(true, cx));

        let sheet_state = state.clone();
        let sheet_pack_id = pack_id.clone();
        div().relative().size_full().child(shell).child(
            NativeBottomSheetPresentationHost::new(
                page_id(&pack_id, "editor-sheet"),
                Binding::entity(
                    state,
                    |state: &PackInviteLinksScreenState| state.editor.is_some(),
                    |state, value, _| {
                        if !value {
                            state.editor = None;
                        }
                    },
                ),
                move |_, cx| {
                    let target = sheet_state.read(cx).editor.clone().unwrap_or(PackInviteLinkEditorTarget::Create);
                    PackInviteLinkEditorSheet::new(sheet_pack_id.clone(), target)
                },
            )
            .presentation_detents([PresentationDetent::Large]),
        )
    }
}

#[derive(IntoElement)]
struct PackInviteLinksBody {
    parts: PackInviteLinksParts,
    rows: Rc<Vec<PackInviteLinksRow>>,
}

impl RenderOnce for PackInviteLinksBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let top_inset = list_shell_floating_top_inset(cx);
        let refresh = self.parts.clone();
        let screen_state = self.parts.state.clone();
        let parts = self.parts;
        let rows = self.rows;
        let background = theme.bg._100;
        let list = screen_state.read(cx).list.refreshable(
            page_id(&parts.pack_id, "list"),
            move |_, cx| {
                refresh.load(true, cx);
                Task::ready(())
            },
            move |index, _, _| {
                let Some(row) = rows.get(index) else {
                    return Empty.into_any_element();
                };
                let top_padding = if index == 0 { top_inset + 12.0 } else { 0.0 };
                div().w_full().pt(px(top_padding)).child(links_row(row, &parts, &theme)).into_any_element()
            },
        );
        div().size_full().bg(background).child(list)
    }
}
