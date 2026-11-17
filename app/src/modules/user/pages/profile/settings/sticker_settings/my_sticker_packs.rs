use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyView, App, AppContext, Context, Empty, Entity, ListAlignment, Render, SharedString, StyleRefinement,
    Subscription, Task, WeakEntity, Window, transparent_black,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::guide_anchor_ui::guide_scroll_host::{GuideScrollHostMount, GuideScrollHostScope};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::stickers::components::sticker_pack_create_sheet::StickerPackCreateSheet;
use crate::modules::stickers::components::sticker_pack_sheet::sticker_pack_sheet_store::StickerPackSheetStore;
use crate::modules::stickers::engine::rise_sticker_models::{RiseStickerPerformancePolicy, RiseStickerResourceID};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{PackKind, StickerItem, StickerPack};
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;
use crate::modules::user::pages::profile::settings::sticker_settings::sticker_settings::StickerPackSettingsIcon;

/// page state and the store values it draws, notifies only when one of them changes
struct MyStickerPacksState {
    kind: PackKind,
    create_sheet_presented: bool,
    create_request_id: String,
    user_id: String,
    is_creating: bool,
    create_error: Option<String>,
    total: i64,
    top_inset: Rc<Cell<f32>>,
    rows: Entity<MyStickerPacksRowsView>,
    _subscriptions: Vec<Subscription>,
}

impl MyStickerPacksState {
    fn new(kind: PackKind, cx: &mut Context<Self>) -> Self {
        let services = StickersServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let top_inset = Rc::new(Cell::new(0.0));
        let page = cx.weak_entity();
        let rows_inset = top_inset.clone();
        let rows = cx.new(|cx| MyStickerPacksRowsView::new(kind, page, rows_inset, cx));
        let mut state = Self {
            kind,
            create_sheet_presented: false,
            create_request_id: Uuid::new_v4().uuid_string(),
            user_id: String::new(),
            is_creating: false,
            create_error: None,
            total: 0,
            top_inset,
            rows,
            _subscriptions: vec![
                cx.observe(&services, |state, _, cx| state.sync(cx)),
                cx.observe(&auth, |state, _, cx| state.sync(cx)),
            ],
        };
        state.read_stores(cx);
        state
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        if self.read_stores(cx) {
            cx.notify();
        }
    }

    /// takes the page values from the stores, true if one changed
    fn read_stores(&mut self, cx: &mut App) -> bool {
        let user_id = MyStickerPacks::user_id(cx);
        let store = StickersServicesStore::shared(cx);
        let services = store.read(cx);
        let is_creating = services.is_creating_pack(self.kind);
        let create_error = services.create_pack_error(self.kind);
        let total = MyStickerPacks::total_in(services, &user_id, self.kind);
        let changed = user_id != self.user_id
            || is_creating != self.is_creating
            || create_error != self.create_error
            || total != self.total;
        self.user_id = user_id;
        self.is_creating = is_creating;
        self.create_error = create_error;
        self.total = total;
        changed
    }
}

/// the pages lazy list as its own cached view,
/// with a thousand packs only this view renders on wheel frames
struct MyStickerPacksRowsView {
    kind: PackKind,
    page: WeakEntity<MyStickerPacksState>,
    top_inset: Rc<Cell<f32>>,
    list: ListUi,
    row_keys: Vec<String>,
    snapshot: Rc<MyStickerPacksSnapshot>,
    mount: GuideScrollHostMount,
    _subscriptions: Vec<Subscription>,
}

impl MyStickerPacksRowsView {
    fn new(
        kind: PackKind,
        page: WeakEntity<MyStickerPacksState>,
        top_inset: Rc<Cell<f32>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let services = StickersServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let list = ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top);
        let mount = GuideScrollHostMount::for_list(list.state().clone());
        let mut rows = Self {
            kind,
            page,
            top_inset,
            list,
            row_keys: Vec::new(),
            snapshot: Rc::new(MyStickerPacksSnapshot::default()),
            mount,
            _subscriptions: vec![
                cx.observe(&services, |rows, _, cx| rows.sync(cx)),
                cx.observe(&auth, |rows, _, cx| rows.sync(cx)),
            ],
        };
        rows.rebuild(cx);
        rows
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        if self.snapshot.is_current(self.kind, cx) {
            return;
        }
        self.rebuild(cx);
        cx.notify();
    }

    fn rebuild(&mut self, cx: &mut App) {
        let snapshot = MyStickerPacksSnapshot::read(self.kind, cx);
        let keys: Vec<String> = snapshot.rows.iter().map(|row| snapshot.key(row)).collect();
        self.sync_list(keys);
        self.snapshot = Rc::new(snapshot);
    }

    fn sync_list(&mut self, keys: Vec<String>) {
        if keys == self.row_keys {
            return;
        }
        let start = keys.iter().zip(&self.row_keys).take_while(|(new, old)| new == old).count();
        let tail_limit = keys.len().min(self.row_keys.len()) - start;
        let end =
            keys.iter().rev().zip(self.row_keys.iter().rev()).take(tail_limit).take_while(|(new, old)| new == old).count();
        self.list.state().splice(start..self.row_keys.len() - end, keys.len() - start - end);
        self.row_keys = keys;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MyStickerPacksSection {
    Owned,
    Coauthor,
}

impl MyStickerPacksSection {
    fn key(self) -> &'static str {
        match self {
            Self::Owned => "owned",
            Self::Coauthor => "coauthor",
        }
    }
}

/// one item of the lazy list, a section card is split into rows so only visible ones get built
enum MyStickerPacksRow {
    Create,
    Title(MyStickerPacksSection),
    Pack { section: MyStickerPacksSection, index: usize, is_first: bool, is_last: bool },
    FirstLoad,
    NextPage,
    Bottom,
}

impl MyStickerPacksRow {
    fn starts_block(&self) -> bool {
        matches!(self, Self::Title(_) | Self::FirstLoad | Self::NextPage | Self::Bottom)
    }
}

#[derive(IntoElement)]
pub struct MyStickerPacks {
    kind: PackKind,
}

impl MyStickerPacks {
    pub fn new(kind: PackKind) -> Self {
        Self { kind }
    }

    fn user_id(cx: &mut App) -> String {
        let auth = AuthServicesStore::shared(cx);
        auth.read(cx).effective_user_id_for_wire().unwrap_or_default()
    }

    fn total_in(services: &StickersServicesStore, user_id: &str, kind: PackKind) -> i64 {
        let owned = services.my_packs_slice(user_id, kind).iter().filter(|pack| pack.is_creator).count();
        i64::try_from(owned).unwrap_or(i64::MAX).max(services.my_packs_total(user_id, kind))
    }

    fn reached_limit(user_id: &str, kind: PackKind, cx: &mut App) -> bool {
        let store = StickersServicesStore::shared(cx);
        Self::total_in(store.read(cx), user_id, kind) >= RiseStickerPerformancePolicy::created_packs_per_kind_limit()
    }

    fn create_row(
        kind: PackKind,
        total: i64,
        page: WeakEntity<MyStickerPacksState>,
        theme: &AppTheme,
    ) -> GroupedBtn {
        let limit = RiseStickerPerformancePolicy::created_packs_per_kind_limit();
        GroupedBtn::new(
            format!("create-{}-pack", kind.wire()),
            tr(if kind == PackKind::Emoji { "emoji_pack_create" } else { "sticker_pack_create" }),
        )
        .icon("plus")
        .icon_background(theme.primary._100)
        .trailing(format!("{}/{}", total.min(limit), limit))
        .disabled(total >= limit)
        .guide_anchor((kind == PackKind::Sticker).then_some(RiseGuideAnchor::StickerPackCreate))
        .callback(move |_, cx| {
            StickersInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.clear_create_pack_error(kind, cx));
            if let Some(page) = page.upgrade() {
                page.update(cx, |state, cx| {
                    state.create_request_id = Uuid::new_v4().uuid_string();
                    state.create_sheet_presented = true;
                    cx.notify();
                });
            }
        })
    }

    fn pack_row(pack: StickerPack, item: Option<StickerItem>, kind: PackKind) -> GroupedBtn {
        let pack_id = pack.id.clone();
        let pack_kind = pack.pack_kind;
        let subtitle = if pack.is_creator {
            tr("sticker_pack_unique_saves").replace("{count}", &RiseFormat::compact_count(pack.unique_saves))
        } else {
            tr("sticker_pack_coauthor")
        };
        GroupedBtn::new(
            format!("{}-pack-{}", if pack.is_creator { "owned" } else { "coauthor" }, pack.id),
            pack.title.clone(),
        )
        .subtitle(subtitle)
        .leading_slot(StickerPackSettingsIcon::new(pack, kind, item))
        .callback(move |_, cx| {
            StickerPackSheetStore::shared(cx).update(cx, |store, cx| store.present(&pack_id, pack_kind, cx));
        })
    }

    fn create_pack(kind: PackKind, title: String, state: &Entity<MyStickerPacksState>, cx: &mut App) {
        let user_id = Self::user_id(cx);
        if user_id.is_empty() || Self::reached_limit(&user_id, kind, cx) {
            return;
        }
        let request_id = state.read(cx).create_request_id.clone();
        let created_state = state.clone();
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.create_pack(
                &user_id,
                &title,
                kind,
                &request_id,
                move |_, cx| {
                    created_state.update(cx, |state, cx| {
                        state.create_sheet_presented = false;
                        cx.notify();
                    });
                },
                cx,
            );
        });
    }

    fn load_my_packs(kind: PackKind, cx: &mut App) {
        let user_id = Self::user_id(cx);
        if user_id.is_empty() {
            return;
        }
        StickersInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.load_my_packs(&user_id, kind, None, cx));
    }

    fn load_next_page(kind: PackKind, cx: &mut App) {
        let user_id = Self::user_id(cx);
        if user_id.is_empty() {
            return;
        }
        let services = StickersServicesStore::shared(cx);
        if !services.read(cx).my_packs_has_more(&user_id, kind) {
            return;
        }
        let Some(cursor) = services.read(cx).my_packs_next_relative_id(&user_id, kind) else {
            return;
        };
        StickersInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.load_my_packs(&user_id, kind, Some(cursor), cx));
    }
}

impl RenderOnce for MyStickerPacks {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let kind = self.kind;
        let state =
            window.use_keyed_state(SharedString::from(format!("my-sticker-packs-state-{}", kind.wire())), cx, |_, cx| {
                MyStickerPacksState::new(kind, cx)
            });
        let nav_store = env::<NavigationStoreKey>(cx);
        let page = state.read(cx);
        let user_id = page.user_id.clone();
        let is_creating = page.is_creating;
        let reached_limit = page.total >= RiseStickerPerformancePolicy::created_packs_per_kind_limit();
        let create_error = if reached_limit { Some(tr("sticker_pack_limit_reached")) } else { page.create_error.clone() };
        let presented = page.create_sheet_presented;

        let change_state = state.clone();
        on_change(
            SharedString::from(format!("my-sticker-packs-user-{}", kind.wire())),
            user_id,
            false,
            window,
            cx,
            move |_, _, _, cx| {
                change_state.update(cx, |state, cx| {
                    state.create_sheet_presented = false;
                    cx.notify();
                });
                Self::load_my_packs(kind, cx);
            },
        );
        on_change(
            SharedString::from(format!("my-sticker-packs-create-open-{}", kind.wire())),
            presented,
            false,
            window,
            cx,
            |_, open, _, cx| GuideFacts::set(RiseGuideSignal::StickerPackCreateOpen, *open, cx),
        );

        let sheet_state = state.clone();
        let is_presented = Binding::entity(
            state.clone(),
            |state: &MyStickerPacksState| state.create_sheet_presented,
            |state: &mut MyStickerPacksState, presented, _| state.create_sheet_presented = presented,
        );

        let title = tr(if kind == PackKind::Emoji { "emoji_my_packs_title" } else { "stickers_my_packs_title" });
        div()
            .relative()
            .size_full()
            .child(
                SettingsWrapperUi::new(title, move |_, cx| {
                    if let Some(nav_store) = &nav_store {
                        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                    }
                })
                .content(MyStickerPacksBody { state }),
            )
            .child(StickerPackCreateSheet::new(is_presented, kind, is_creating, create_error, move |title, _, cx| {
                Self::create_pack(kind, title, &sheet_state, cx);
            }))
            .on_appear(SharedString::from(format!("my-sticker-packs-appear-{}", kind.wire())), move |_, cx| {
                Self::load_my_packs(kind, cx);
            })
            .and_on_disappear(|_, cx| GuideFacts::set(RiseGuideSignal::StickerPackCreateOpen, false, cx))
    }
}

#[derive(IntoElement)]
struct MyStickerPacksBody {
    state: Entity<MyStickerPacksState>,
}

impl RenderOnce for MyStickerPacksBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let top_inset = list_shell_floating_top_inset(cx);
        let page = self.state.read(cx);
        let rows = page.rows.clone();
        let held_inset = page.top_inset.clone();
        if (held_inset.get() - top_inset).abs() > f32::EPSILON {
            held_inset.set(top_inset);
            rows.update(cx, |_, cx| cx.notify());
        }
        AnyView::from(rows).cached(StyleRefinement::default().size_full()).replay_when_moved()
    }
}

/// store reads of the list taken when stores change and shared with the list items
#[derive(Default)]
struct MyStickerPacksSnapshot {
    rows: Vec<MyStickerPacksRow>,
    owned: Vec<StickerPack>,
    coauthor: Vec<StickerPack>,
    owned_items: Vec<Option<StickerItem>>,
    coauthor_items: Vec<Option<StickerItem>>,
    user_id: String,
    total: i64,
    is_loading: bool,
    has_more: bool,
    next_relative_id: Option<String>,
    next_page_failed: bool,
    next_appear_id: SharedString,
}

impl MyStickerPacksSnapshot {
    fn read(kind: PackKind, cx: &mut App) -> Self {
        let user_id = MyStickerPacks::user_id(cx);
        let store = StickersServicesStore::shared(cx);
        let services = store.read(cx);
        let (owned, coauthor): (Vec<StickerPack>, Vec<StickerPack>) =
            services.my_packs_slice(&user_id, kind).iter().cloned().partition(|pack| pack.is_creator);
        let first_items = |packs: &[StickerPack]| -> Vec<Option<StickerItem>> {
            packs.iter().map(|pack| services.pack_first_sticker(&pack.id).cloned()).collect()
        };
        let owned_items = first_items(&owned);
        let coauthor_items = first_items(&coauthor);
        let packs_empty = owned.is_empty() && coauthor.is_empty();
        let total = MyStickerPacks::total_in(services, &user_id, kind);
        let is_loading = services.is_loading(RiseStickerResourceID::MyPacks(kind));
        let has_more = services.my_packs_has_more(&user_id, kind);
        let next_relative_id = services.my_packs_next_relative_id(&user_id, kind);
        let next_page_failed = services.is_failed(RiseStickerResourceID::MyPacks(kind));
        let next_appear_id = SharedString::from(format!(
            "my-sticker-packs-next-{}-{}",
            next_relative_id.as_deref().unwrap_or_default(),
            if next_page_failed { "retry" } else { "page" }
        ));

        let mut rows = Vec::with_capacity(owned.len() + coauthor.len() + 6);
        rows.push(MyStickerPacksRow::Create);
        for (section, count) in
            [(MyStickerPacksSection::Owned, owned.len()), (MyStickerPacksSection::Coauthor, coauthor.len())]
        {
            if count == 0 {
                continue;
            }
            rows.push(MyStickerPacksRow::Title(section));
            rows.extend((0..count).map(|index| MyStickerPacksRow::Pack {
                section,
                index,
                is_first: index == 0,
                is_last: index + 1 == count,
            }));
        }
        if is_loading && packs_empty {
            rows.push(MyStickerPacksRow::FirstLoad);
        }
        if has_more {
            rows.push(MyStickerPacksRow::NextPage);
        }
        rows.push(MyStickerPacksRow::Bottom);
        Self {
            rows,
            owned,
            coauthor,
            owned_items,
            coauthor_items,
            user_id,
            total,
            is_loading,
            has_more,
            next_relative_id,
            next_page_failed,
            next_appear_id,
        }
    }

    /// true if the stores still hold what this snapshot shows, compares without cloning
    fn is_current(&self, kind: PackKind, cx: &mut App) -> bool {
        let user_id = MyStickerPacks::user_id(cx);
        if user_id != self.user_id {
            return false;
        }
        let store = StickersServicesStore::shared(cx);
        let services = store.read(cx);
        if MyStickerPacks::total_in(services, &user_id, kind) != self.total
            || services.is_loading(RiseStickerResourceID::MyPacks(kind)) != self.is_loading
            || services.my_packs_has_more(&user_id, kind) != self.has_more
            || services.is_failed(RiseStickerResourceID::MyPacks(kind)) != self.next_page_failed
            || services.my_packs_next_relative_id(&user_id, kind) != self.next_relative_id
        {
            return false;
        }
        let packs = services.my_packs_slice(&user_id, kind);
        if packs.len() != self.owned.len() + self.coauthor.len() {
            return false;
        }
        let (mut owned, mut coauthor) = (self.owned.iter(), self.coauthor.iter());
        let same_packs = packs.iter().all(|pack| {
            let held = if pack.is_creator { owned.next() } else { coauthor.next() };
            held == Some(pack)
        });
        same_packs
            && self
                .owned
                .iter()
                .zip(&self.owned_items)
                .chain(self.coauthor.iter().zip(&self.coauthor_items))
                .all(|(pack, item)| services.pack_first_sticker(&pack.id) == item.as_ref())
    }

    fn packs(&self, section: MyStickerPacksSection) -> &[StickerPack] {
        match section {
            MyStickerPacksSection::Owned => &self.owned,
            MyStickerPacksSection::Coauthor => &self.coauthor,
        }
    }

    fn item(&self, section: MyStickerPacksSection, index: usize) -> Option<StickerItem> {
        let items = match section {
            MyStickerPacksSection::Owned => &self.owned_items,
            MyStickerPacksSection::Coauthor => &self.coauthor_items,
        };
        items.get(index).cloned().flatten()
    }

    fn key(&self, row: &MyStickerPacksRow) -> String {
        match row {
            MyStickerPacksRow::Create => "create".to_owned(),
            MyStickerPacksRow::Title(section) => format!("title-{}", section.key()),
            MyStickerPacksRow::Pack { section, index, is_last, .. } => format!(
                "{}-{}-{}",
                section.key(),
                self.packs(*section).get(*index).map(|pack| pack.id.as_str()).unwrap_or_default(),
                if *is_last { "last" } else { "row" }
            ),
            MyStickerPacksRow::FirstLoad => "first-load".to_owned(),
            MyStickerPacksRow::NextPage => "next-page".to_owned(),
            MyStickerPacksRow::Bottom => "bottom".to_owned(),
        }
    }
}

struct MyStickerPacksRows {
    kind: PackKind,
    page: WeakEntity<MyStickerPacksState>,
    snapshot: Rc<MyStickerPacksSnapshot>,
    top_inset: f32,
    theme: AppTheme,
}

impl MyStickerPacksRows {
    fn item(&self, index: usize) -> AnyElement {
        let snapshot = &self.snapshot;
        let Some(row) = snapshot.rows.get(index) else {
            return Empty.into_any_element();
        };
        let kind = self.kind;
        let theme = &self.theme;
        let tint = theme.primary._100;
        let top = if index == 0 {
            self.top_inset + 12.0
        } else if row.starts_block() {
            16.0
        } else {
            0.0
        };
        let body = match row {
            MyStickerPacksRow::Create => GroupedBtns::new(
                SharedString::from(format!("my-sticker-packs-create-{}", kind.wire())),
                vec![MyStickerPacks::create_row(kind, snapshot.total, self.page.clone(), theme)],
            )
            .into_any_element(),
            MyStickerPacksRow::Title(section) => {
                let title = match section {
                    MyStickerPacksSection::Owned => {
                        tr(if kind == PackKind::Emoji { "emoji_my_packs_title" } else { "stickers_my_packs_title" })
                    }
                    MyStickerPacksSection::Coauthor => tr("sticker_pack_coauthor"),
                };
                div()
                    .w_full()
                    .px(px(GroupedCardMetrics::TITLE_HORIZONTAL_PADDING))
                    .pb(px(8.0))
                    .child(MainText::new(title.to_uppercase()).px(12.0).color(theme.text.secondary))
                    .into_any_element()
            }
            MyStickerPacksRow::Pack { section, index, is_first, is_last } => {
                let Some(pack) = snapshot.packs(*section).get(*index).cloned() else {
                    return Empty.into_any_element();
                };
                let item = snapshot.item(*section, *index);
                let (is_first, is_last) = (*is_first, *is_last);
                let row = GroupedBtns::new(
                    SharedString::from(format!("my-sticker-packs-{}-{}", section.key(), kind.wire())),
                    vec![MyStickerPacks::pack_row(pack, item, kind)],
                )
                .horizontal_padding(0.0)
                .corner_radius(0.0)
                .surface_color(transparent_black());
                let card = div()
                    .w_full()
                    .bg(theme.bg._200)
                    .when(is_first, |card| card.rounded_t(px(GroupedCardMetrics::CORNER_RADIUS)))
                    .when(is_last, |card| card.rounded_b(px(GroupedCardMetrics::CORNER_RADIUS)))
                    .child(row)
                    .when(!is_last, |card| {
                        card.child(
                            div()
                                .w_full()
                                .pl(px(GroupedCardMetrics::content_leading_inset()))
                                .child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55))),
                        )
                    });
                div().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING)).child(card).into_any_element()
            }
            MyStickerPacksRow::FirstLoad => div()
                .w_full()
                .flex()
                .justify_center()
                .pt(px(8.0))
                .child(progress_view().tint(tint))
                .into_any_element(),
            MyStickerPacksRow::NextPage => div()
                .w_full()
                .flex()
                .justify_center()
                .py(px(8.0))
                .child(progress_view().tint(tint))
                .on_appear(snapshot.next_appear_id.clone(), move |_, cx| MyStickerPacks::load_next_page(kind, cx))
                .into_any_element(),
            MyStickerPacksRow::Bottom => div().w_full().h(px(80.0)).into_any_element(),
        };
        div().w_full().pt(px(top)).child(body).into_any_element()
    }
}

impl Render for MyStickerPacksRowsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.kind;
        let theme = rise_ui::theme(cx).clone();
        let data = Rc::new(MyStickerPacksRows {
            kind,
            page: self.page.clone(),
            snapshot: self.snapshot.clone(),
            top_inset: self.top_inset.get(),
            theme: theme.clone(),
        });
        let host = self.mount.host();
        let wheel_host = host.clone();
        let list = self.list.refreshable(
            SharedString::from(format!("my-sticker-packs-scroll-{}", kind.wire())),
            move |_, cx| {
                MyStickerPacks::load_my_packs(kind, cx);
                Task::ready(())
            },
            move |index, _, _| data.item(index),
        );
        div().size_full().bg(theme.bg._100).child(GuideScrollHostScope::new(
            host,
            div().relative().size_full().on_scroll_wheel(move |event, _, _| wheel_host.note_wheel(event)).child(list),
        ))
    }
}
