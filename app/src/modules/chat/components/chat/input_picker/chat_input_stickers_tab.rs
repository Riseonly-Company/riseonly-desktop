use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, AnyView, App, AppContext, Context, ElementId, Entity, FontWeight, IntoElement, ListAlignment, ListOffset,
    ListState, Pixels, Render, RenderOnce, ScrollHandle, SharedString, StyleRefinement, Subscription, Window, black, list,
    white,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::list_ui::list_scroll_animation::ListScrollAnimation;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonShimmerExt};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::{ease_out, spring};
use rise_ui::swiftui::prelude::*;

use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit::{
    ChatInputEditableStickerCell, ChatInputPackLayout, ChatInputPackSectionPolicy, ChatInputPickerFlatChromeKey,
    ChatInputPickerMetrics, ChatInputSectionHeader, ChatInputStickerCell, ChatInputStripIconButton,
    StickerItemDeletionRequest, StickerPackLockBadge, chat_input_picker_bar_reserve,
};
use crate::modules::chat::components::chat::input_picker::sticker_hold_preview::{
    StickerHoldEntry, StickerHoldGridModifier, StickerHoldPreviewModifiers, StickerHoldPreviewPresenter,
    StickerPreviewAction, StickerPreviewAsset,
};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::stickers::components::sticker_pack_add_cell::{StickerAssetComposerStore, StickerPackAddCell};
use crate::modules::stickers::components::sticker_pack_sheet::sticker_pack_sheet_store::StickerPackSheetStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    PackKind, StickerItem, StickerPack,
};
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

const FAVOURITES_ANCHOR: &str = "stk_favourites";
const RECENTS_ANCHOR: &str = "stk_recents";
const EDITING_SPRING: (f64, f64) = (0.24, 0.86);
const STRIP_BUTTON_WIDTH: f32 = 36.0;
const STRIP_SPACING: f32 = 4.0;
const STRIP_HORIZONTAL_PADDING: f32 = 10.0;
const STRIP_OVERSCAN: usize = 4;
const STRIP_DEFAULT_VIEWPORT: f32 = 480.0;

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

fn anchor_for_pack_id(id: &str) -> String {
    format!("stk_pack_{id}")
}

fn can_manage(pack: &StickerPack) -> bool {
    pack.is_creator || pack.is_admin
}

fn can_edit_in_grid(pack: &StickerPack) -> bool {
    pack.can_edit_stickers() || pack.can_delete_stickers()
}

/// premium pack on a free account, stickers stay visible but a tap opens the paywall
/// the server would refuse with the same upgrade tier
fn is_pack_locked(pack: &StickerPack, services: &StickersServicesStore, self_is_premium: bool) -> bool {
    if self_is_premium {
        return false;
    }
    pack.is_premium || services.pack_summary(&pack.id).is_some_and(|summary| summary.is_premium)
}

fn is_item_locked(item: &StickerItem, services: &StickersServicesStore, self_is_premium: bool) -> bool {
    services.pack_summary(&item.pack_id).is_some_and(|pack| is_pack_locked(&pack, services, self_is_premium))
}

fn self_is_premium(cx: &mut App) -> bool {
    AuthServicesStore::shared(cx).read(cx).profile().is_some_and(|profile| profile.more.is_premium)
}

fn effective_user_id(cx: &mut App) -> String {
    AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default()
}

fn offer_premium(source: String, cx: &mut App) {
    let is_premium = self_is_premium(cx);
    PremiumUpsellStore::shared(cx).update(cx, |upsell, cx| {
        upsell.offer_premium_feature(tr("premium_upsell_reason_sticker_pack"), &source, is_premium, cx);
    });
}

fn load_more_packs_if_needed(visible_pack_id: &str, cx: &mut App) {
    let user_id = effective_user_id(cx);
    let services = StickersServicesStore::shared(cx);
    let cursor = {
        let services = services.read(cx);
        let saved = services.saved_pack_summaries(&user_id, PackKind::Sticker);
        saved
            .iter()
            .position(|pack| pack.id == visible_pack_id)
            .filter(|index| saved.len() - index <= 3)
            .filter(|_| services.saved_packs_has_more(&user_id, PackKind::Sticker))
            .and_then(|_| services.saved_packs_next_relative_id(&user_id, PackKind::Sticker))
    };
    if let Some(cursor) = cursor {
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_saved_packs(&user_id, PackKind::Sticker, Some(cursor), false, cx);
        });
    }
}

fn present_delete_alert(request: StickerItemDeletionRequest, cx: &mut App) {
    let user_id = effective_user_id(cx);
    NativeConfirmAlert::present_delete(
        &tr("picker_delete_sticker_title"),
        &tr("picker_delete_item_message"),
        Some(tr("delete_action")),
        None,
        false,
        None,
        None,
        move |cx| {
            StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.delete_sticker(&user_id, &request.pack.id, &request.item.id, cx);
            });
        },
        cx,
    );
}

#[derive(Clone, PartialEq)]
enum StickerGridSlot {
    Add,
    Item(StickerItem),
    Filler(usize),
}

impl StickerGridSlot {
    fn id(&self) -> String {
        match self {
            Self::Add => "add".to_owned(),
            Self::Item(item) => item.id.clone(),
            Self::Filler(index) => format!("filler_{index}"),
        }
    }
}

#[derive(Clone, PartialEq)]
struct StickerGridRow {
    id: String,
    start: usize,
    slots: Vec<StickerGridSlot>,
}

impl StickerGridRow {
    fn chunk(slots: &[StickerGridSlot], columns: usize, section: &str) -> Vec<StickerGridRow> {
        let width = columns.max(1);
        slots
            .chunks(width)
            .enumerate()
            .map(|(index, row)| StickerGridRow {
                id: format!("{section}_r{index}"),
                start: index * width,
                slots: row.to_vec(),
            })
            .collect()
    }
}

#[derive(Clone, PartialEq)]
enum SectionTrailing {
    None,
    RecentsClear,
    Lock { pack_id: String },
    Edit { pack_id: String, is_editing: bool },
}

/// a row of the grid, sections are one flat list here
/// list state scrolls by row so a jump lands on the real header at once
#[derive(Clone, PartialEq)]
enum StickersGridListRow {
    Header {
        anchor: String,
        title: String,
        trailing: SectionTrailing,
        managed_pack_id: Option<String>,
        appear_pack_id: Option<String>,
    },
    Slots {
        row: StickerGridRow,
        owner_pack: Option<StickerPack>,
        is_pack_editing: bool,
    },
    LoadingHeader {
        pack: StickerPack,
    },
    LoadingRow {
        key: String,
    },
    BottomInset,
}

impl StickersGridListRow {
    fn key(&self) -> String {
        match self {
            Self::Header { anchor, trailing, .. } => {
                let trailing = match trailing {
                    SectionTrailing::None => "n",
                    SectionTrailing::RecentsClear => "c",
                    SectionTrailing::Lock { .. } => "l",
                    SectionTrailing::Edit { is_editing: true, .. } => "s",
                    SectionTrailing::Edit { is_editing: false, .. } => "e",
                };
                format!("h:{anchor}:{trailing}")
            }
            Self::Slots { row, is_pack_editing, .. } => {
                let slots: Vec<String> = row.slots.iter().map(StickerGridSlot::id).collect();
                format!("r:{}:{is_pack_editing}:{}", row.id, slots.join(","))
            }
            Self::LoadingHeader { pack } => format!("lh:{}", pack.id),
            Self::LoadingRow { key } => format!("lr:{key}"),
            Self::BottomInset => "inset".to_owned(),
        }
    }
}

#[derive(Clone)]
enum StripEntry {
    Favourites,
    Recents,
    Pack { pack: StickerPack, cover_item: Option<StickerItem> },
}

/// view state and the rows derived from the stores
/// rows rebuild only after a change since a playing sticker redraws every frame
/// and the library can be long
struct ChatInputStickersTabState {
    pending_pack_scroll_id: Option<String>,
    editing_pack_id: Option<String>,
    editing_order: Vec<String>,
    dragged_sticker_id: Option<String>,
    frozen_recents: Option<Vec<StickerItem>>,
    user_id: String,
    width: f32,
    height: f32,
    rows_are_stale: bool,
    columns: usize,
    is_library_empty: bool,
    is_library_failed: bool,
    grid: ListState,
    grid_scroll: ListScrollAnimation,
    row_keys: Vec<String>,
    rows: Rc<Vec<StickersGridListRow>>,
    anchors: HashMap<String, usize>,
    strip: Rc<Vec<StripEntry>>,
    strip_scroll: ScrollHandle,
    hold_scroll_top: Option<ListOffset>,
    row_views: HashMap<String, Entity<StickersGridRowView>>,
    row_view_keys: Rc<Vec<Option<String>>>,
    hold_was_active: bool,
    _subscriptions: Vec<Subscription>,
}

impl ChatInputStickersTabState {
    fn new(cx: &mut Context<Self>) -> Self {
        let overdraw = ThemeStore::shared(cx).read(cx).current_theme().list.overdraw;
        let services = StickersServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let hold_presenter = StickerHoldPreviewPresenter::shared(cx);
        Self {
            pending_pack_scroll_id: None,
            editing_pack_id: None,
            editing_order: Vec::new(),
            dragged_sticker_id: None,
            frozen_recents: None,
            user_id: String::new(),
            width: 0.0,
            height: 0.0,
            rows_are_stale: true,
            columns: 0,
            is_library_empty: false,
            is_library_failed: false,
            grid: ListState::new(0, ListAlignment::Top, overdraw),
            grid_scroll: ListScrollAnimation::default(),
            row_keys: Vec::new(),
            rows: Rc::new(Vec::new()),
            anchors: HashMap::new(),
            strip: Rc::new(Vec::new()),
            strip_scroll: ScrollHandle::new(),
            hold_scroll_top: None,
            row_views: HashMap::new(),
            row_view_keys: Rc::new(Vec::new()),
            hold_was_active: hold_presenter.read(cx).is_active(),
            _subscriptions: vec![
                cx.observe(&services, |state, _, cx| {
                    state.rows_are_stale = true;
                    cx.notify();
                }),
                cx.observe(&auth, |state, _, cx| {
                    state.rows_are_stale = true;
                    cx.notify();
                }),
                cx.observe(&hold_presenter, |this, presenter, cx| {
                    let is_active = presenter.read(cx).is_active();
                    if is_active != this.hold_was_active {
                        this.hold_was_active = is_active;
                        cx.notify();
                    }
                }),
            ],
        }
    }

    fn end_editing(&mut self) {
        self.editing_pack_id = None;
        self.editing_order.clear();
        self.dragged_sticker_id = None;
        self.rows_are_stale = true;
    }

    fn scroll_to_anchor(&mut self, anchor: &str, cx: &App) {
        if let Some(&row) = self.anchors.get(anchor) {
            let target = ListOffset { item_ix: row, offset_in_item: Pixels::ZERO };
            self.grid_scroll.scroll_to(&self.grid, target, ease_out(0.22), cx);
        }
    }

    fn tap_scroll_to_anchor(state: &Entity<Self>, anchor: &str, cx: &mut App) {
        state.update(cx, |state, cx| {
            state.scroll_to_anchor(anchor, cx);
            cx.notify();
        });
    }

    /// recents stay as they were while the panel sends
    /// rows reshuffling above a deep scroll made the list jump
    /// fresh list shows when the tab opens again
    fn recents(&self, services: &StickersServicesStore) -> Vec<StickerItem> {
        self.frozen_recents.clone().unwrap_or_else(|| services.recents(&self.user_id, Some(PackKind::Sticker)))
    }

    fn ordered_items(&self, items: Vec<StickerItem>, pack: &StickerPack) -> Vec<StickerItem> {
        if self.editing_pack_id.as_deref() != Some(pack.id.as_str()) {
            return items;
        }
        let by_id: HashMap<&str, &StickerItem> = items.iter().map(|item| (item.id.as_str(), item)).collect();
        let mut ordered: Vec<StickerItem> =
            self.editing_order.iter().filter_map(|id| by_id.get(id.as_str()).map(|item| (*item).clone())).collect();
        let known: HashSet<String> = ordered.iter().map(|item| item.id.clone()).collect();
        ordered.extend(items.iter().filter(|item| !known.contains(&item.id)).cloned());
        ordered
    }

    fn reconciled_order(&self, items: &[StickerItem]) -> Vec<String> {
        let current: HashSet<&str> = items.iter().map(|item| item.id.as_str()).collect();
        let mut order: Vec<String> =
            self.editing_order.iter().filter(|id| current.contains(id.as_str())).cloned().collect();
        order.extend(items.iter().filter(|item| !self.editing_order.contains(&item.id)).map(|item| item.id.clone()));
        order
    }

    #[allow(clippy::too_many_arguments)]
    fn push_section(
        &self,
        rows: &mut Vec<StickersGridListRow>,
        anchors: &mut HashMap<String, usize>,
        title: String,
        items: Vec<StickerItem>,
        anchor: String,
        owner_pack: Option<&StickerPack>,
        recents_trailing: bool,
        services: &StickersServicesStore,
        self_is_premium: bool,
    ) {
        let is_pack_editing =
            owner_pack.is_some_and(|pack| self.editing_pack_id.as_deref() == Some(pack.id.as_str()));
        let shows_add_cell = owner_pack.is_some_and(|pack| pack.can_add_stickers() && !is_pack_editing);
        let has_items = !items.is_empty();
        let displayed = match owner_pack {
            Some(pack) => self.ordered_items(items, pack),
            None => items,
        };
        let filler_count = if owner_pack.is_none() || self.columns == 0 {
            0
        } else {
            let used = (displayed.len() + usize::from(shows_add_cell)) % self.columns;
            if used == 0 { 0 } else { self.columns - used }
        };
        let trailing = match owner_pack {
            Some(pack) if is_pack_locked(pack, services, self_is_premium) => {
                SectionTrailing::Lock { pack_id: pack.id.clone() }
            }
            Some(pack)
                if can_edit_in_grid(pack)
                    && (has_items || is_pack_editing)
                    && (self.editing_pack_id.is_none() || is_pack_editing) =>
            {
                SectionTrailing::Edit { pack_id: pack.id.clone(), is_editing: is_pack_editing }
            }
            Some(_) => SectionTrailing::None,
            None if recents_trailing => SectionTrailing::RecentsClear,
            None => SectionTrailing::None,
        };

        anchors.insert(anchor.clone(), rows.len());
        rows.push(StickersGridListRow::Header {
            anchor: anchor.clone(),
            title,
            trailing,
            managed_pack_id: owner_pack.filter(|pack| can_edit_in_grid(pack)).map(|pack| pack.id.clone()),
            appear_pack_id: owner_pack.map(|pack| pack.id.clone()),
        });

        let mut slots = Vec::with_capacity(displayed.len() + filler_count + 1);
        if shows_add_cell {
            slots.push(StickerGridSlot::Add);
        }
        slots.extend(displayed.into_iter().map(StickerGridSlot::Item));
        slots.extend((0..filler_count).map(StickerGridSlot::Filler));
        for row in StickerGridRow::chunk(&slots, self.columns, &anchor) {
            rows.push(StickersGridListRow::Slots { row, owner_pack: owner_pack.cloned(), is_pack_editing });
        }
    }

    fn rebuild_rows(&mut self, user_id: String, self_is_premium: bool, cx: &mut Context<Self>) {
        if user_id != self.user_id {
            self.user_id = user_id;
            self.frozen_recents = None;
            self.pending_pack_scroll_id = None;
            self.end_editing();
        }
        let services_entity = StickersServicesStore::shared(cx);
        let services = services_entity.read(cx);
        let user_id = self.user_id.clone();

        let favourites = services.favourites(&user_id);
        let recents = self.recents(services);
        let saved_packs = services.saved_pack_summaries(&user_id, PackKind::Sticker);
        let layout = ChatInputPackLayout::with_options(
            &saved_packs,
            |id| services.has_resolved_pack(id),
            |id| services.is_pack_unavailable(id),
            i64::try_from(saved_packs.len()).unwrap_or(i64::MAX),
        );

        self.is_library_empty = favourites.is_empty()
            && recents.is_empty()
            && layout.ordered.is_empty()
            && services.saved_packs_resolved(&user_id, PackKind::Sticker);
        self.is_library_failed = favourites.is_empty()
            && recents.is_empty()
            && layout.ordered.is_empty()
            && services.saved_packs_failed(&user_id, PackKind::Sticker);

        let mut strip = Vec::with_capacity(layout.ordered.len() + 2);
        if !favourites.is_empty() {
            strip.push(StripEntry::Favourites);
        }
        if !recents.is_empty() {
            strip.push(StripEntry::Recents);
        }
        strip.extend(layout.ordered.iter().map(|pack| {
            let first = services.pack_stickers(&pack.id).into_iter().next();
            let cover_item =
                if can_manage(pack) && !pack.is_default { first.or_else(|| pack.cover_item()) } else { pack.cover_item().or(first) };
            StripEntry::Pack { pack: pack.clone(), cover_item }
        }));

        let mut rows = Vec::new();
        let mut anchors = HashMap::new();
        if self.columns > 0 {
            if !favourites.is_empty() {
                self.push_section(
                    &mut rows,
                    &mut anchors,
                    tr("stickers_favourites_header"),
                    favourites,
                    FAVOURITES_ANCHOR.to_owned(),
                    None,
                    false,
                    services,
                    self_is_premium,
                );
            }
            if !recents.is_empty() {
                self.push_section(
                    &mut rows,
                    &mut anchors,
                    tr("stickers_recent_header"),
                    recents,
                    RECENTS_ANCHOR.to_owned(),
                    None,
                    true,
                    services,
                    self_is_premium,
                );
            }
            for pack in &layout.visible {
                if services.has_resolved_pack(&pack.id) {
                    self.push_section(
                        &mut rows,
                        &mut anchors,
                        pack.title.clone(),
                        services.pack_stickers(&pack.id),
                        anchor_for_pack_id(&pack.id),
                        Some(pack),
                        false,
                        services,
                        self_is_premium,
                    );
                } else {
                    anchors.insert(anchor_for_pack_id(&pack.id), rows.len());
                    rows.push(StickersGridListRow::LoadingHeader { pack: pack.clone() });
                    for row in 0..ChatInputPackSectionPolicy::PLACEHOLDER_ROWS {
                        rows.push(StickersGridListRow::LoadingRow { key: format!("{}_{row}", pack.id) });
                    }
                }
            }
            rows.push(StickersGridListRow::BottomInset);
        }

        let keys: Vec<String> = rows.iter().map(StickersGridListRow::key).collect();
        self.sync_rows(keys);
        let view_keys = StickersGridRowView::keys(&rows);
        let live_views: HashSet<&str> = view_keys.iter().flatten().map(String::as_str).collect();
        self.row_views.retain(|key, _| live_views.contains(key.as_str()));
        self.row_view_keys = Rc::new(view_keys);
        self.rows = Rc::new(rows);
        self.anchors = anchors;
        self.strip = Rc::new(strip);
        self.rows_are_stale = false;

        let resolved_pending = self
            .pending_pack_scroll_id
            .as_ref()
            .filter(|id| layout.plan.resolved_pack_ids.contains(id))
            .cloned();
        if let Some(id) = resolved_pending {
            self.pending_pack_scroll_id = None;
            self.scroll_to_anchor(&anchor_for_pack_id(&id), cx);
        }
    }

    fn sync_rows(&mut self, keys: Vec<String>) {
        if keys == self.row_keys {
            return;
        }
        let old = &self.row_keys;
        let prefix = old.iter().zip(keys.iter()).take_while(|(a, b)| a == b).count();
        let max_suffix = old.len().min(keys.len()) - prefix;
        let suffix =
            old.iter().rev().zip(keys.iter().rev()).take(max_suffix).take_while(|(a, b)| a == b).count();
        self.grid.splice(prefix..old.len() - suffix, keys.len() - prefix - suffix);
        self.row_keys = keys;
    }

    fn scroll_to_pack(state: &Entity<Self>, pack: &StickerPack, cx: &mut App) {
        let services = StickersServicesStore::shared(cx);
        if services.read(cx).has_resolved_pack(&pack.id) {
            Self::tap_scroll_to_anchor(state, &anchor_for_pack_id(&pack.id), cx);
            return;
        }
        state.update(cx, |state, _| state.pending_pack_scroll_id = Some(pack.id.clone()));
        StickersInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.load_pack_stickers(&pack.id, cx));
    }

    fn sticker_menu(state: &Entity<Self>, entry: &StickerHoldEntry, cx: &App) -> Vec<StickerPreviewAction> {
        let user_id = state.read(cx).user_id.clone();
        let StickerPreviewAsset::Sticker(item) = &entry.asset else {
            return Vec::new();
        };
        if user_id.is_empty() {
            return Vec::new();
        }
        let Some(services) = rise_ui::shared::try_read::<StickersServicesStore>(cx) else {
            return Vec::new();
        };
        let is_favourite = services.is_favourite(&user_id, &item.id);
        let favourite_item = item.clone();
        let mut actions = vec![
            StickerPreviewAction::new(
                "favourite",
                if is_favourite { tr("favourite_remove") } else { tr("favourite_add") },
                if is_favourite { "bookmark.slash.fill" } else { "bookmark.fill" },
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.toggle_favourite(&user_id, &favourite_item, cx);
                    });
                }),
            )
            .guide_anchor(RiseGuideAnchor::StickerMenuFavourite),
        ];
        if let Some(pack) = services.pack_summary(&item.pack_id).filter(|pack| pack.can_edit_stickers()) {
            let edit_pack = pack.clone();
            let edit_item = item.clone();
            actions.push(StickerPreviewAction::new(
                "edit",
                tr("edit_action"),
                "pencil",
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    StickerAssetComposerStore::shared(cx).update(cx, |composer, cx| {
                        composer.present_sticker(edit_pack.clone(), edit_item.clone(), cx);
                    });
                }),
            ));
            let delete_item = item.clone();
            actions.push(
                StickerPreviewAction::new(
                    "delete",
                    tr("delete_action"),
                    "trash",
                    Rc::new(move |_: &mut Window, cx: &mut App| {
                        present_delete_alert(
                            StickerItemDeletionRequest { item: delete_item.clone(), pack: pack.clone() },
                            cx,
                        );
                    }),
                )
                .destructive(true),
            );
        }
        actions
    }
}

struct EditingSlotMotion {
    position: usize,
    offset_x: Animated<f32>,
    offset_y: Animated<f32>,
}

fn editing_slot(
    id: ElementId,
    position: usize,
    columns: usize,
    cell: f32,
    content: AnyElement,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let motion = window.use_keyed_state(id, cx, |_, _| EditingSlotMotion {
        position,
        offset_x: Animated::new(0.0),
        offset_y: Animated::new(0.0),
    });
    let now = RiseClock::monotonic();
    let (x, y, is_running) = motion.update(cx, |motion, cx| {
        if motion.position != position {
            let width = columns.max(1);
            let column = |index: usize| (index % width) as f32;
            let row = |index: usize| (index / width) as f32;
            motion.offset_x.jump(motion.offset_x.value(now) + (column(motion.position) - column(position)) * cell);
            motion.offset_y.jump(motion.offset_y.value(now) + (row(motion.position) - row(position)) * cell);
            let editing_spring = spring(EDITING_SPRING.0, EDITING_SPRING.1);
            motion.offset_x.set_in(0.0, editing_spring, now, cx);
            motion.offset_y.set_in(0.0, editing_spring, now, cx);
            motion.position = position;
        }
        (
            motion.offset_x.value(now),
            motion.offset_y.value(now),
            motion.offset_x.is_running(now) || motion.offset_y.is_running(now),
        )
    });
    if is_running {
        window.request_animation_frame();
    }
    div().relative().left(px(x)).top(px(y)).child(content).into_any_element()
}

#[derive(IntoElement)]
pub struct ChatInputStickersTab {
    id: ElementId,
    on_send_sticker: Rc<dyn Fn(StickerItem, &mut Window, &mut App)>,
    is_active: bool,
    uses_flat_chrome: bool,
    on_open_pack_settings: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

/// what every row of one frame reads
struct StickersGridContext {
    id: ElementId,
    state: Entity<ChatInputStickersTabState>,
    theme: AppTheme,
    is_active: bool,
    uses_flat_chrome: bool,
    cell: f32,
    columns: usize,
    picker_bar_reserve: f32,
}

impl StickersGridContext {
    fn draws_rows_like(&self, other: &Self) -> bool {
        self.id == other.id
            && self.state == other.state
            && self.is_active == other.is_active
            && self.uses_flat_chrome == other.uses_flat_chrome
            && self.cell == other.cell
            && self.columns == other.columns
            && self.picker_bar_reserve == other.picker_bar_reserve
            && self.theme.primary._100 == other.theme.primary._100
            && self.theme.text.secondary == other.theme.text.secondary
    }

    fn recents_clear_button(&self) -> impl IntoElement {
        let state = self.state.clone();
        let color = if self.uses_flat_chrome { self.theme.text.secondary } else { white().opacity(0.38) };
        plain(child_id(&self.id, "recents-clear"))
            .on_press(move |_, cx| {
                let _ = selection_changed();
                state.update(cx, |state, cx| {
                    state.frozen_recents = None;
                    state.rows_are_stale = true;
                    cx.notify();
                });
                let user_id = effective_user_id(cx);
                StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.clear_recents(&user_id, PackKind::Sticker, cx);
                });
            })
            .mr(px(4.0))
            .child(
                div()
                    .min_h(px(24.0))
                    .flex()
                    .items_center()
                    .swiftui_font(Font::system_weight(12.0, FontWeight::MEDIUM))
                    .text_color(color)
                    .child(tr("picker_recents_clear")),
            )
    }

    fn edit_button(&self, pack_id: &str, is_editing: bool) -> impl IntoElement {
        let state = self.state.clone();
        let pack_id_owned = pack_id.to_owned();
        plain(child_id(&self.id, format!("edit/{pack_id}")))
            .on_press(move |_, cx| {
                let _ = selection_changed();
                let items = StickersServicesStore::shared(cx).read(cx).pack_stickers(&pack_id_owned);
                if is_editing {
                    let order = state.update(cx, |state, cx| {
                        let order = state.reconciled_order(&items);
                        state.end_editing();
                        cx.notify();
                        order
                    });
                    let user_id = effective_user_id(cx);
                    StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.reorder_stickers(&user_id, &pack_id_owned, order, cx);
                    });
                } else {
                    let can_reorder = StickersServicesStore::shared(cx)
                        .read(cx)
                        .pack_summary(&pack_id_owned)
                        .is_some_and(|pack| pack.can_edit_stickers());
                    tracing::warn!(target: "riseonly::X1622", pack = %pack_id_owned, can_reorder, items = items.len(), "edit mode began");
                    state.update(cx, |state, cx| {
                        state.editing_pack_id = Some(pack_id_owned.clone());
                        state.editing_order = items.iter().map(|item| item.id.clone()).collect();
                        state.dragged_sticker_id = None;
                        state.rows_are_stale = true;
                        cx.notify();
                    });
                }
            })
            .mr(px(4.0))
            .child(
                div()
                    .min_h(px(24.0))
                    .flex()
                    .items_center()
                    .swiftui_font(Font::system_weight(12.0, FontWeight::SEMIBOLD))
                    .text_color(self.theme.primary._100)
                    .child(tr(if is_editing { "save" } else { "edit_action" })),
            )
    }

    fn header(
        &self,
        anchor: &str,
        title: &str,
        trailing: &SectionTrailing,
        managed_pack_id: Option<&String>,
        appear_pack_id: Option<&String>,
    ) -> AnyElement {
        let mut header = ChatInputSectionHeader::new(child_id(&self.id, format!("header/{anchor}")), title.to_owned());
        match trailing {
            SectionTrailing::None => {}
            SectionTrailing::RecentsClear => header = header.trailing(self.recents_clear_button()),
            SectionTrailing::Lock { pack_id } => {
                let source = format!("sticker_pack:{pack_id}");
                header = header.trailing(StickerPackLockBadge::new(
                    child_id(&self.id, format!("lock/{pack_id}")),
                    move |_, cx| offer_premium(source.clone(), cx),
                ));
            }
            SectionTrailing::Edit { pack_id, is_editing } => {
                header = header.trailing(self.edit_button(pack_id, *is_editing));
            }
        }
        if let Some(pack_id) = managed_pack_id.cloned() {
            header = header.title_action(move |_, cx| {
                StickerPackSheetStore::shared(cx)
                    .update(cx, |store, cx| store.present(&pack_id, PackKind::Sticker, cx));
            });
        }
        match appear_pack_id.cloned() {
            Some(pack_id) => header
                .on_appear(child_id(&self.id, format!("header-appear/{anchor}")), move |_, cx| {
                    load_more_packs_if_needed(&pack_id, cx);
                })
                .into_any_element(),
            None => header.into_any_element(),
        }
    }

    fn grid_slot(
        &self,
        slot: &StickerGridSlot,
        position: usize,
        owner_pack: Option<&StickerPack>,
        is_pack_editing: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let cell = self.cell;
        match slot {
            StickerGridSlot::Add => match owner_pack {
                Some(pack) => {
                    let add_pack = pack.clone();
                    StickerPackAddCell::new(
                        child_id(&self.id, format!("add/{}", pack.id)),
                        PackKind::Sticker,
                        cell,
                        move |_, cx| {
                            StickerAssetComposerStore::shared(cx)
                                .update(cx, |composer, cx| composer.present(add_pack.clone(), cx));
                        },
                    )
                    .guide_anchor((pack.id == GuideSandbox::own_sticker_pack_id()).then_some(RiseGuideAnchor::StickerPackAddCell))
                    .into_any_element()
                }
                None => div().into_any_element(),
            },
            StickerGridSlot::Filler(_) => div().size(px(cell)).into_any_element(),
            StickerGridSlot::Item(item) => match owner_pack.filter(|pack| can_edit_in_grid(pack)) {
                Some(pack) => {
                    let order_reader = self.state.clone();
                    let order_writer = self.state.clone();
                    let dragged_reader = self.state.clone();
                    let dragged_writer = self.state.clone();
                    let delete_item = item.clone();
                    let delete_pack = pack.clone();
                    let editable = ChatInputEditableStickerCell::new(
                        item.clone(),
                        cell,
                        self.is_active,
                        is_pack_editing,
                        Binding::new(
                            move |cx: &App| order_reader.read(cx).editing_order.clone(),
                            move |order, _, cx| {
                                order_writer.update(cx, |state, cx| {
                                    state.editing_order = order;
                                    state.rows_are_stale = true;
                                    cx.notify();
                                });
                            },
                        ),
                        Binding::new(
                            move |cx: &App| dragged_reader.read(cx).dragged_sticker_id.clone(),
                            move |dragged, _, cx| {
                                dragged_writer.update(cx, |state, cx| {
                                    state.dragged_sticker_id = dragged;
                                    cx.notify();
                                });
                            },
                        ),
                        pack.can_delete_stickers().then(|| {
                            Rc::new(move |_: &mut Window, cx: &mut App| {
                                present_delete_alert(
                                    StickerItemDeletionRequest { item: delete_item.clone(), pack: delete_pack.clone() },
                                    cx,
                                );
                            }) as Rc<dyn Fn(&mut Window, &mut App)>
                        }),
                    )
                    .can_reorder(pack.can_edit_stickers())
                    .into_any_element();
                    if is_pack_editing {
                        editing_slot(
                            child_id(&self.id, format!("edit-slot/{}", item.id)),
                            position,
                            self.columns,
                            cell,
                            editable,
                            window,
                            cx,
                        )
                    } else {
                        editable
                    }
                }
                None => {
                    let services = StickersServicesStore::shared(cx);
                    let is_premium = self_is_premium(cx);
                    let is_locked = match owner_pack {
                        Some(pack) => is_pack_locked(pack, services.read(cx), is_premium),
                        None => is_item_locked(item, services.read(cx), is_premium),
                    };
                    let panda_anchor =
                        (item.id == GuideSandbox::panda_first_sticker_id()).then_some(RiseGuideAnchor::StickerPandaFirst);
                    div()
                        .relative()
                        .size(px(cell))
                        .child(ChatInputStickerCell::new(item.clone(), cell).is_playing(self.is_active))
                        .when(is_locked, |slot| {
                            slot.child(div().absolute().right_0().bottom_0().child(ChatInputLockedStickerMark))
                        })
                        .guide_anchor(panda_anchor)
                        .into_any_element()
                }
            },
        }
    }

    fn loading_row(&self) -> AnyElement {
        let cell = self.cell;
        let side = (cell - 8.0).max(1.0);
        let is_active = self.is_active;
        let cells = (0..self.columns).map(move |_| {
            let block = zstack(Alignment::Center).frame_size(cell, cell).child(SkeletonBlock::new(side, side, 12.0));
            if is_active { block.skeleton_shimmer(None).into_any_element() } else { block.into_any_element() }
        });
        hstack(0.0).w_full().h(px(cell)).children(cells).child(spacer_min_w(0.0)).into_any_element()
    }

    fn row(&self, row: &StickersGridListRow, window: &mut Window, cx: &mut App) -> AnyElement {
        match row {
            StickersGridListRow::Header { anchor, title, trailing, managed_pack_id, appear_pack_id } => {
                self.header(anchor, title, trailing, managed_pack_id.as_ref(), appear_pack_id.as_ref())
            }
            StickersGridListRow::Slots { row, owner_pack, is_pack_editing } => {
                let cells: Vec<AnyElement> = row
                    .slots
                    .iter()
                    .enumerate()
                    .map(|(index, slot)| {
                        self.grid_slot(slot, row.start + index, owner_pack.as_ref(), *is_pack_editing, window, cx)
                    })
                    .collect();
                div()
                    .id(child_id(&self.id, format!("row/{}", row.id)))
                    .w_full()
                    .child(hstack(0.0).w_full().h(px(self.cell)).children(cells).child(spacer_min_w(0.0)))
                    .into_any_element()
            }
            StickersGridListRow::LoadingHeader { pack } => {
                let pack_id = pack.id.clone();
                ChatInputSectionHeader::new(child_id(&self.id, format!("loading/{}", pack.id)), pack.title.clone())
                    .on_appear(child_id(&self.id, format!("loading-appear/{}", pack.id)), move |_, cx| {
                        StickersInteractionsStore::shared(cx)
                            .update(cx, |interactions, cx| interactions.load_pack_stickers(&pack_id, cx));
                        load_more_packs_if_needed(&pack_id, cx);
                    })
                    .into_any_element()
            }
            StickersGridListRow::LoadingRow { .. } => self.loading_row(),
            StickersGridListRow::BottomInset => {
                div().w_full().h(px(ChatInputPickerMetrics::GRID_BOTTOM_INSET + self.picker_bar_reserve)).into_any_element()
            }
        }
    }
}

impl ChatInputStickersTab {
    pub fn new(
        id: impl Into<ElementId>,
        on_send_sticker: impl Fn(StickerItem, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            on_send_sticker: Rc::new(on_send_sticker),
            is_active: true,
            uses_flat_chrome: false,
            on_open_pack_settings: None,
        }
    }

    pub fn is_active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }

    pub fn uses_flat_chrome(mut self, uses_flat_chrome: bool) -> Self {
        self.uses_flat_chrome = uses_flat_chrome;
        self
    }

    pub fn on_open_pack_settings(mut self, on_open_pack_settings: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_open_pack_settings = Some(Rc::new(on_open_pack_settings));
        self
    }

    fn empty_state(&self, theme: &AppTheme) -> AnyElement {
        let muted = if self.uses_flat_chrome { theme.text.secondary } else { white().opacity(0.4) };
        let mut column = vstack(14.0)
            .w_full()
            .flex_1()
            .min_h_0()
            .items_center()
            .child(spacer())
            .children(IconUi::sized("square.grid.2x2", px(30.0), muted, false))
            .child(
                div()
                    .px(px(32.0))
                    .swiftui_font(Font::system_weight(15.0, FontWeight::NORMAL))
                    .text_color(muted)
                    .text_center()
                    .child(tr("sticker_pack_saved_empty")),
            );
        if let Some(on_open_pack_settings) = self.on_open_pack_settings.clone() {
            column = column.child(
                plain(child_id(&self.id, "empty-my-packs"))
                    .on_press(move |window, cx| {
                        let _ = selection_changed();
                        on_open_pack_settings(window, cx);
                    })
                    .child(
                        div()
                            .min_h(px(36.0))
                            .flex()
                            .items_center()
                            .swiftui_font(Font::system_weight(15.0, FontWeight::SEMIBOLD))
                            .text_color(theme.primary._100)
                            .child(tr("stickers_my_packs")),
                    ),
            );
        }
        column.child(spacer()).into_any_element()
    }

    fn failed_state(&self, theme: &AppTheme, user_id: String) -> AnyElement {
        let muted = if self.uses_flat_chrome { theme.text.secondary } else { white().opacity(0.4) };
        vstack(14.0)
            .w_full()
            .flex_1()
            .min_h_0()
            .items_center()
            .child(spacer())
            .children(IconUi::sized("wifi.slash", px(30.0), muted, false))
            .child(
                div()
                    .px(px(32.0))
                    .swiftui_font(Font::system_weight(15.0, FontWeight::NORMAL))
                    .text_color(muted)
                    .text_center()
                    .child(tr("error_connection_lost")),
            )
            .child(
                plain(child_id(&self.id, "failed-retry"))
                    .on_press(move |_, cx| {
                        let user_id = user_id.clone();
                        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                            interactions.load_favourites(&user_id, cx);
                            interactions.load_recents(&user_id, Some(PackKind::Sticker), cx);
                            interactions.load_saved_packs(&user_id, PackKind::Sticker, None, true, cx);
                        });
                    })
                    .child(
                        div()
                            .min_h(px(36.0))
                            .flex()
                            .items_center()
                            .swiftui_font(Font::system_weight(15.0, FontWeight::SEMIBOLD))
                            .text_color(theme.primary._100)
                            .child(tr("common_retry")),
                    ),
            )
            .child(spacer())
            .into_any_element()
    }

    fn strip_button(
        &self,
        state: &Entity<ChatInputStickersTabState>,
        entry: &StripEntry,
    ) -> AnyElement {
        match entry {
            StripEntry::Favourites => {
                let state = state.clone();
                ChatInputStripIconButton::new(
                    child_id(&self.id, "strip/favourites"),
                    Some("star.fill"),
                    None,
                    false,
                    tr("stickers_favourites_header"),
                    move |_, cx| ChatInputStickersTabState::tap_scroll_to_anchor(&state, FAVOURITES_ANCHOR, cx),
                )
                .guide_anchor(RiseGuideAnchor::StickerFavourites)
                .into_any_element()
            }
            StripEntry::Recents => {
                let state = state.clone();
                ChatInputStripIconButton::new(
                    child_id(&self.id, "strip/recents"),
                    Some("clock"),
                    None,
                    false,
                    tr("stickers_recent_header"),
                    move |_, cx| ChatInputStickersTabState::tap_scroll_to_anchor(&state, RECENTS_ANCHOR, cx),
                )
                .into_any_element()
            }
            StripEntry::Pack { pack, cover_item } => {
                let state = state.clone();
                let tap_pack = pack.clone();
                let pack_id = pack.id.clone();
                ChatInputStripIconButton::new(
                    child_id(&self.id, format!("strip/{}", pack.id)),
                    Some("square.grid.2x2"),
                    cover_item.clone(),
                    false,
                    pack.title.clone(),
                    move |_, cx| ChatInputStickersTabState::scroll_to_pack(&state, &tap_pack, cx),
                )
                .on_appear(child_id(&self.id, format!("strip-appear/{}", pack.id)), move |_, cx| {
                    load_more_packs_if_needed(&pack_id, cx);
                })
                .into_any_element()
            }
        }
    }

    fn top_strip(&self, state: &Entity<ChatInputStickersTabState>, cx: &App) -> AnyElement {
        let tab = state.read(cx);
        let entries = tab.strip.clone();
        let handle = tab.strip_scroll.clone();
        let stride = STRIP_BUTTON_WIDTH + STRIP_SPACING;
        let viewport = match f32::from(handle.bounds().size.width) {
            width if width > 0.0 => width,
            _ => STRIP_DEFAULT_VIEWPORT,
        };
        let end = (entries.len() as f32 * stride - viewport).max(0.0);
        let scrolled = (-f32::from(handle.offset().x) - STRIP_HORIZONTAL_PADDING).clamp(0.0, end);
        let first = ((scrolled / stride) as usize).saturating_sub(STRIP_OVERSCAN).min(entries.len());
        let last = (first + (viewport / stride).ceil() as usize + STRIP_OVERSCAN * 2).min(entries.len());

        let mut strip = div()
            .id(child_id(&self.id, "strip"))
            .w_full()
            .h(px(ChatInputPickerMetrics::TOP_STRIP_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .px(px(STRIP_HORIZONTAL_PADDING))
            .gap(px(STRIP_SPACING))
            .overflow_x_scroll()
            .track_scroll(&handle);
        if first > 0 {
            strip = strip.child(div().flex_none().w(px(first as f32 * stride - STRIP_SPACING)).h(px(1.0)));
        }
        strip = strip.children(entries[first..last].iter().map(|entry| {
            div().flex_none().child(self.strip_button(state, entry))
        }));
        let remaining = entries.len() - last;
        if remaining > 0 {
            strip = strip.child(div().flex_none().w(px(remaining as f32 * stride - STRIP_SPACING)).h(px(1.0)));
        }
        strip.into_any_element()
    }

    fn grid(
        &self,
        state: &Entity<ChatInputStickersTabState>,
        theme: &AppTheme,
        list_height: f32,
        cx: &mut App,
    ) -> AnyElement {
        let tab = state.read(cx);
        let rows = tab.rows.clone();
        let width = tab.width;
        let columns = tab.columns;
        let row_view_keys = tab.row_view_keys.clone();
        let row_state = state.clone();
        let context = Rc::new(StickersGridContext {
            id: self.id.clone(),
            state: state.clone(),
            theme: theme.clone(),
            is_active: self.is_active,
            uses_flat_chrome: self.uses_flat_chrome,
            cell: (width / columns.max(1) as f32).floor(),
            columns,
            picker_bar_reserve: chat_input_picker_bar_reserve(cx),
        });
        let rows_list = list(state.read(cx).grid.clone(), move |index, window, cx| {
            let Some(row) = rows.get(index) else {
                return div().into_any_element();
            };
            match row_view_keys.get(index).and_then(Option::as_deref) {
                Some(key) => StickersGridRowView::element(&row_state, key, row, &context, cx),
                None => context.row(row, window, cx),
            }
        })
        .w(px(width))
        .h(px(list_height))
        .pt(px(ChatInputPickerMetrics::TOP_STRIP_HEIGHT));

        let on_primary_state = state.clone();
        let on_send_sticker = self.on_send_sticker.clone();
        let menu_state = state.clone();
        rows_list
            .sticker_hold_grid(
                StickerHoldGridModifier::new(
                    child_id(&self.id, "hold"),
                    Rc::new(move |entry: &StickerHoldEntry, window: &mut Window, cx: &mut App| {
                        if on_primary_state.read(cx).editing_pack_id.is_some() {
                            return;
                        }
                        let StickerPreviewAsset::Sticker(item) = &entry.asset else {
                            return;
                        };
                        let services = StickersServicesStore::shared(cx);
                        let is_premium = self_is_premium(cx);
                        if is_item_locked(item, services.read(cx), is_premium) {
                            offer_premium(format!("sticker_pack:{}", item.pack_id), cx);
                        } else {
                            on_primary_state.update(cx, |state, cx| {
                                if state.frozen_recents.is_none() {
                                    let services = StickersServicesStore::shared(cx);
                                    let recents = state.recents(services.read(cx));
                                    state.frozen_recents = Some(recents);
                                }
                            });
                            on_send_sticker(item.clone(), window, cx);
                        }
                    }),
                )
                .menu_for(Rc::new(move |entry: &StickerHoldEntry, cx: &App| {
                    ChatInputStickersTabState::sticker_menu(&menu_state, entry, cx)
                }))
                .uses_flat_chrome(self.uses_flat_chrome)
                .is_enabled(state.read(cx).editing_pack_id.is_none()),
            )
            .into_any_element()
    }
}

impl RenderOnce for ChatInputStickersTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, cx| {
            ChatInputStickersTabState::new(cx)
        });
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let user_id = effective_user_id(cx);
        let is_premium = self_is_premium(cx);
        let is_active = self.is_active;
        let hold_is_active = StickerHoldPreviewPresenter::shared(cx).read(cx).is_active();

        let retry_user_id = user_id.clone();
        let (is_library_empty, is_library_failed, height, scroll_top) = state.update(cx, |tab, cx| {
            if !is_active && tab.frozen_recents.is_some() {
                tab.frozen_recents = None;
                tab.rows_are_stale = true;
            }
            let columns = ChatInputPickerMetrics::sticker_columns(tab.width);
            if tab.width > 0.0 && tab.columns != columns {
                tab.columns = columns;
                tab.rows_are_stale = true;
            }
            if tab.rows_are_stale || user_id != tab.user_id {
                tab.rebuild_rows(user_id, is_premium, cx);
            }
            tab.grid_scroll.frame(&tab.grid, window);
            tab.grid.paint_under_top_padding(true);
            if hold_is_active {
                let top = *tab.hold_scroll_top.get_or_insert_with(|| tab.grid.logical_scroll_top());
                tab.grid.scroll_to(top);
            } else {
                tab.hold_scroll_top = None;
            }
            (tab.is_library_empty, tab.is_library_failed, tab.height, tab.grid.logical_scroll_top())
        });
        let edge_blur = ChatInputPickerMetrics::strip_edge_blur(scroll_top, &theme);

        let shows_grid = !is_library_failed && !is_library_empty;
        let body = if is_library_failed {
            self.failed_state(&theme, retry_user_id)
        } else if is_library_empty {
            self.empty_state(&theme)
        } else {
            self.grid(&state, &theme, height, cx)
        };

        let measure_state = state.clone();
        let disappear_state = state.clone();
        env_scope::<ChatInputPickerFlatChromeKey>(self.uses_flat_chrome)
            .child(
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .flex_col()
                    .children((!shows_grid).then(|| div().flex_none().h(px(ChatInputPickerMetrics::TOP_STRIP_HEIGHT))))
                    .child(body)
                    .children(edge_blur.filter(|_| shows_grid))
                    .child(div().absolute().top_0().left_0().right_0().occlude().child(self.top_strip(&state, cx)))
                    .child(measure(child_id(&self.id, "measure"), move |bounds, _window, cx| {
                        let width = f32::from(bounds.size.width);
                        let height = f32::from(bounds.size.height);
                        measure_state.update(cx, |tab, cx| {
                            if tab.width != width || tab.height != height {
                                tab.width = width;
                                tab.height = height;
                                tab.rows_are_stale = true;
                                cx.notify();
                            }
                        });
                    })),
            )
            .on_disappear(child_id(&self.id, "disappear"), move |_, cx| {
                disappear_state.update(cx, |tab, _| {
                    tab.frozen_recents = None;
                    tab.rows_are_stale = true;
                    tab.row_views.clear();
                });
            })
    }
}

/// one sticker row as its own cached view
/// a wheel step replays it moved instead of rendering every sticker again
struct StickersGridRowView {
    row: Option<StickersGridListRow>,
    context: Option<Rc<StickersGridContext>>,
    marks: StickersGridRowMarks,
}

/// what a row reads from the sticker services while it renders besides its slots
/// the lock of each plain sticker
#[derive(Clone, Default, PartialEq)]
struct StickersGridRowMarks {
    locks: Vec<bool>,
}

impl StickersGridRowMarks {
    fn of(row: &StickersGridListRow, cx: &mut App) -> Self {
        let StickersGridListRow::Slots { row, owner_pack, .. } = row else {
            return Self::default();
        };
        let is_premium = self_is_premium(cx);
        let services = StickersServicesStore::shared(cx);
        let services = services.read(cx);
        let locks = row
            .slots
            .iter()
            .map(|slot| match (slot, owner_pack.as_ref().filter(|pack| can_edit_in_grid(pack))) {
                (StickerGridSlot::Item(item), None) => match owner_pack {
                    Some(pack) => is_pack_locked(pack, services, is_premium),
                    None => is_item_locked(item, services, is_premium),
                },
                _ => false,
            })
            .collect();
        Self { locks }
    }
}

impl StickersGridRowView {
    /// view key of every row drawn through a cached row view
    /// rows of a pack in editing animate their slots and draw inline
    fn keys(rows: &[StickersGridListRow]) -> Vec<Option<String>> {
        let mut occurrences: HashMap<String, usize> = HashMap::new();
        rows.iter()
            .map(|row| match row {
                StickersGridListRow::Slots { is_pack_editing: false, .. } => {
                    let key = row.key();
                    let occurrence = occurrences.entry(key.clone()).or_insert(0);
                    *occurrence += 1;
                    Some(format!("{key}#{occurrence}"))
                }
                _ => None,
            })
            .collect()
    }

    fn set_props(&mut self, row: &StickersGridListRow, context: &Rc<StickersGridContext>, marks: StickersGridRowMarks) -> bool {
        let stale = self.row.as_ref() != Some(row)
            || self.marks != marks
            || !self.context.as_ref().is_some_and(|current| current.draws_rows_like(context));
        if stale {
            self.row = Some(row.clone());
            self.marks = marks;
        }
        self.context = Some(context.clone());
        stale
    }

    fn element(
        state: &Entity<ChatInputStickersTabState>,
        key: &str,
        row: &StickersGridListRow,
        context: &Rc<StickersGridContext>,
        cx: &mut App,
    ) -> AnyElement {
        let existing = state.read(cx).row_views.get(key).cloned();
        let view = match existing {
            Some(view) => view,
            None => {
                let view = cx.new(|_| StickersGridRowView { row: None, context: None, marks: StickersGridRowMarks::default() });
                let inserted = view.clone();
                state.update(cx, |state, _| {
                    state.row_views.insert(key.to_owned(), inserted);
                });
                view
            }
        };
        let marks = StickersGridRowMarks::of(row, cx);
        let stale = view.update(cx, |view, _| view.set_props(row, context, marks));
        if stale {
            AnyView::from(view).into_any_element()
        } else {
            AnyView::from(view)
                .cached(StyleRefinement::default().w_full().h(px(context.cell)))
                .replay_when_moved()
                .into_any_element()
        }
    }
}

impl Render for StickersGridRowView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match (&self.row, &self.context) {
            (Some(row), Some(context)) => context.row(row, window, cx),
            _ => div().into_any_element(),
        }
    }
}

/// lock on a sticker of a premium pack on a free account
#[derive(IntoElement)]
pub struct ChatInputLockedStickerMark;

impl RenderOnce for ChatInputLockedStickerMark {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div().p(px(6.0)).child(
            zstack(Alignment::Center)
                .frame_size(18.0, 18.0)
                .bg(black().opacity(0.45))
                .clip_shape(Shape::Circle)
                .children(IconUi::sized("lock.fill", px(9.0), white(), true)),
        )
    }
}
