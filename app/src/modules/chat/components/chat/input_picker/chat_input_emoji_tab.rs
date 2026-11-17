use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, AnyView, App, AppContext, Context, ElementId, Entity, FontWeight, Hsla, IntoElement, ListAlignment, ListOffset,
    ListState, Pixels, Render, RenderOnce, ScrollHandle, SharedString, StyleRefinement, Subscription, Window, black, list, white,
};
use rise_i18n::tr;
use rise_platform::haptics::{ImpactStyle, impact, selection_changed};
use rise_theme::ThemeStore;
use rise_ui::IconUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_glass::HoldContextMenuGlass;
use rise_ui::list_ui::list_scroll_animation::ListScrollAnimation;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::ui::emoji_picker_ui::emoji_picker_catalog::{EmojiPickerCatalog, EmojiPickerCatalogCell};
use crate::core::ui::emoji_picker_ui::emoji_picker_recents_store::{EmojiPickerRecentsStore, EmojiRecentSlot};
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit::{
    ChatInputEditableStickerCell, ChatInputPackLayout, ChatInputPackLoadingSection,
    ChatInputPickerFlatChromeKey, ChatInputPickerMetrics, ChatInputSectionHeader, ChatInputStickerCell, ChatInputStripIconButton,
    StickerItemDeletionRequest, StickerPackLockBadge, chat_input_picker_bar_reserve,
};
use crate::modules::chat::components::chat::input_picker::chat_input_stickers_tab::ChatInputLockedStickerMark;
use crate::modules::chat::components::chat::input_picker::sticker_hold_preview::{
    StickerHoldEntry, StickerHoldGridModifier, StickerHoldPreviewModifiers, StickerHoldPreviewPresenter, StickerPreviewAction,
    StickerPreviewAsset,
};
use crate::modules::stickers::components::sticker_pack_add_cell::{StickerAssetComposerStore, StickerPackAddCell};
use crate::modules::stickers::components::sticker_pack_sheet::sticker_pack_sheet_store::StickerPackSheetStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{PackKind, StickerItem, StickerPack};
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

type EmojiHandler = Rc<dyn Fn(String, &mut Window, &mut App)>;
type CustomEmojiHandler = Rc<dyn Fn(StickerItem, &mut Window, &mut App)>;
type BackspaceHandler = Rc<dyn Fn(&mut Window, &mut App)>;

const CUSTOM_RECENTS_ANCHOR: &str = "emoji_custom_recents";
const POPULAR_ANCHOR: &str = "emoji_popular";
const UNICODE_RECENTS_ANCHOR: &str = "emoji_unicode_recents";
const BARE_GRID_BOTTOM_INSET: f32 = 72.0;
const UNICODE_ROW_SPACING: f32 = 4.0;
const UNICODE_EMOJI_SIZE: f32 = 30.0;
const UNICODE_CELL_MIN_HEIGHT: f32 = 40.0;
const EMOJI_WARM_ROWS_BEHIND: usize = 4;
const EMOJI_WARM_ROWS_AHEAD: usize = 24;
const STRIP_BUTTON_SLOT: f32 = 40.0;
const STRIP_LEADING_PADDING: f32 = 10.0;
const STRIP_SPACING: f32 = 4.0;
const STRIP_MIN_BUTTON_WIDTH: f32 = 26.0;
const BACKSPACE_SLOT: f32 = 64.0;
const PACKS_LOAD_MORE_DISTANCE: usize = 3;

fn user_id(cx: &mut App) -> String {
    AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default()
}

fn self_is_premium(cx: &mut App) -> bool {
    AuthServicesStore::shared(cx).read(cx).profile().is_some_and(|profile| profile.more.is_premium)
}

/// premium emoji pack on a free account
/// emoji stay visible and a tap opens the paywall. same rule as stickers
fn is_locked_pack(services: &StickersServicesStore, self_is_premium: bool, pack: &StickerPack) -> bool {
    if self_is_premium {
        return false;
    }
    pack.is_premium || services.pack_summary(&pack.id).is_some_and(|summary| summary.is_premium)
}

fn is_locked_item(services: &StickersServicesStore, self_is_premium: bool, item: &StickerItem) -> bool {
    services.pack_summary(&item.pack_id).is_some_and(|pack| is_locked_pack(services, self_is_premium, &pack))
}

fn offer_premium(source: &str, self_is_premium: bool, cx: &mut App) {
    PremiumUpsellStore::shared(cx)
        .update(cx, |store, cx| store.offer_premium_feature(tr("premium_upsell_reason_emoji_pack"), source, self_is_premium, cx));
}

fn can_manage(pack: &StickerPack) -> bool {
    pack.is_creator || pack.is_admin
}

fn can_edit_in_grid(pack: &StickerPack) -> bool {
    pack.can_edit_stickers() || pack.can_delete_stickers()
}

fn anchor_for_pack_id(id: &str) -> String {
    format!("emoji_pack_{id}")
}

fn named_child(id: &ElementId, part: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), part.into())
}

fn load_more_emoji_packs_if_needed(visible_pack_id: &str, cx: &mut App) {
    let user_id = user_id(cx);
    let services = StickersServicesStore::shared(cx);
    let cursor = {
        let services = services.read(cx);
        let saved = services.saved_pack_summaries(&user_id, PackKind::Emoji);
        let Some(index) = saved.iter().position(|pack| pack.id == visible_pack_id) else {
            return;
        };
        if saved.len() - index > PACKS_LOAD_MORE_DISTANCE || !services.saved_packs_has_more(&user_id, PackKind::Emoji) {
            return;
        }
        services.saved_packs_next_relative_id(&user_id, PackKind::Emoji)
    };
    let Some(cursor) = cursor else {
        return;
    };
    StickersInteractionsStore::shared(cx)
        .update(cx, |store, cx| store.load_saved_packs(&user_id, PackKind::Emoji, Some(cursor), false, cx));
}

/// recents as at the first pick, kept until the page is left
/// a pick that reordered the live list shifted rows under the pointer
/// so the next click hit another emoji
#[derive(Clone)]
struct PinnedEmojiRecents {
    custom: Vec<StickerItem>,
    unicode: Vec<String>,
    pick_order: Vec<String>,
}

/// strip jump in flight, the anchor it heads for and the rows it was aimed over
struct EmojiTabScrollRun {
    anchor: String,
    item_ix: usize,
    item_count: usize,
}

struct ChatInputEmojiTabState {
    grid: ListState,
    grid_scroll: ListScrollAnimation,
    scroll_run: Option<EmojiTabScrollRun>,
    row_keys: Vec<String>,
    anchors: HashMap<String, usize>,
    strip: ScrollHandle,
    width: f32,
    height: f32,
    pending_pack_scroll_id: Option<String>,
    editing_pack_id: Option<String>,
    editing_order: Vec<String>,
    dragged_sticker_id: Option<String>,
    pinned_recents: Option<PinnedEmojiRecents>,
    warmed_rows: HashSet<String>,
    warmed_scale: f32,
    warmed_window: (usize, usize, usize),
    row_views: HashMap<String, Entity<EmojiTabRowView>>,
    hold_was_active: bool,
    _subscriptions: [Subscription; 4],
}

impl ChatInputEmojiTabState {
    fn new(cx: &mut Context<Self>) -> Self {
        let overdraw = ThemeStore::shared(cx).read(cx).current_theme().list.overdraw;
        let services = StickersServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let recents = EmojiPickerRecentsStore::shared(cx);
        let presenter = StickerHoldPreviewPresenter::shared(cx);
        Self {
            grid: ListState::new(0, ListAlignment::Top, overdraw),
            grid_scroll: ListScrollAnimation::default(),
            scroll_run: None,
            row_keys: Vec::new(),
            anchors: HashMap::new(),
            strip: ScrollHandle::new(),
            width: 0.0,
            height: 0.0,
            pending_pack_scroll_id: None,
            editing_pack_id: None,
            editing_order: Vec::new(),
            dragged_sticker_id: None,
            pinned_recents: None,
            warmed_rows: HashSet::new(),
            warmed_scale: 0.0,
            warmed_window: (0, 0, 0),
            row_views: HashMap::new(),
            hold_was_active: presenter.read(cx).is_active(),
            _subscriptions: [
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&auth, |_, _, cx| cx.notify()),
                cx.observe(&recents, |_, _, cx| cx.notify()),
                cx.observe(&presenter, |this, presenter, cx| {
                    let is_active = presenter.read(cx).is_active();
                    if is_active != this.hold_was_active {
                        this.hold_was_active = is_active;
                        cx.notify();
                    }
                }),
            ],
        }
    }

    fn sync_rows(&mut self, keys: Vec<String>) {
        if keys == self.row_keys {
            return;
        }
        let old = &self.row_keys;
        let prefix = old.iter().zip(keys.iter()).take_while(|(a, b)| a == b).count();
        let max_suffix = old.len().min(keys.len()) - prefix;
        let suffix = old.iter().rev().zip(keys.iter().rev()).take(max_suffix).take_while(|(a, b)| a == b).count();
        self.grid.splice(prefix..old.len() - suffix, keys.len() - prefix - suffix);
        self.row_keys = keys;
    }

    fn warm_rows_near_scroll_top(&mut self, rows: &[EmojiTabRow], window: &Window, cx: &App) {
        let scale = window.scale_factor();
        if self.warmed_scale != scale {
            self.warmed_scale = scale;
            self.warmed_rows.clear();
            self.warmed_window = (0, 0, 0);
        }
        let top = self.grid.logical_scroll_top().item_ix;
        let start = top.saturating_sub(EMOJI_WARM_ROWS_BEHIND);
        let end = top.saturating_add(EMOJI_WARM_ROWS_AHEAD).min(rows.len());
        let window_key = (start, end, rows.len());
        if self.warmed_window == window_key {
            return;
        }
        self.warmed_window = window_key;
        let mut text = String::new();
        let mut lines: Vec<SharedString> = Vec::new();
        for row in rows.get(start..end).unwrap_or_default() {
            let row_text = row.unicode_emoji_text();
            if !row_text.is_empty() && self.warmed_rows.insert(row_text.clone()) {
                text.push_str(&row_text);
                lines.extend(row.unicode_emoji_cells().into_iter().map(SharedString::from));
            }
        }
        if text.is_empty() {
            return;
        }
        let font = Font::system_size(UNICODE_EMOJI_SIZE);
        let font = gpui::Font {
            family: font.family(),
            features: font.features(),
            fallbacks: None,
            weight: font.weight,
            style: font.style(),
        };
        window.prewarm_line_layouts(lines, font.clone(), px(UNICODE_EMOJI_SIZE), cx).detach();
        window.prewarm_emoji_tiles(text.into(), font, px(UNICODE_EMOJI_SIZE), cx).detach();
    }

    fn scroll_to(&mut self, anchor: &str, cx: &mut Context<Self>) {
        if let Some(&item_ix) = self.anchors.get(anchor) {
            let target = ListOffset { item_ix, offset_in_item: Pixels::ZERO };
            self.grid_scroll.scroll_to(&self.grid, target, ease_out(0.22), cx);
            self.scroll_run = Some(EmojiTabScrollRun { anchor: anchor.to_string(), item_ix, item_count: self.grid.item_count() });
            cx.notify();
        }
    }

    fn keep_scroll_run_on_anchor(&mut self, cx: &App) {
        let Some(run) = self.scroll_run.as_mut() else {
            return;
        };
        let Some(&item_ix) = self.anchors.get(&run.anchor).filter(|_| self.grid_scroll.is_running()) else {
            self.scroll_run = None;
            return;
        };
        let item_count = self.grid.item_count();
        if item_ix == run.item_ix && item_count == run.item_count {
            return;
        }
        run.item_ix = item_ix;
        run.item_count = item_count;
        let target = ListOffset { item_ix, offset_in_item: Pixels::ZERO };
        self.grid_scroll.scroll_to(&self.grid, target, ease_out(0.22), cx);
    }

    fn end_editing(&mut self) {
        self.editing_pack_id = None;
        self.editing_order = Vec::new();
        self.dragged_sticker_id = None;
    }

    fn ordered_items(&self, items: Vec<StickerItem>, pack: &StickerPack) -> Vec<StickerItem> {
        if self.editing_pack_id.as_deref() != Some(pack.id.as_str()) {
            return items;
        }
        let by_id: HashMap<&str, &StickerItem> = items.iter().map(|item| (item.id.as_str(), item)).collect();
        let known: Vec<StickerItem> =
            self.editing_order.iter().filter_map(|id| by_id.get(id.as_str()).map(|item| (*item).clone())).collect();
        let known_ids: HashSet<&str> = known.iter().map(|item| item.id.as_str()).collect();
        let rest: Vec<StickerItem> = items.iter().filter(|item| !known_ids.contains(item.id.as_str())).cloned().collect();
        known.iter().cloned().chain(rest).collect()
    }

    fn reconciled_order(&self, items: &[StickerItem]) -> Vec<String> {
        let current_ids: HashSet<&str> = items.iter().map(|item| item.id.as_str()).collect();
        self.editing_order
            .iter()
            .filter(|id| current_ids.contains(id.as_str()))
            .cloned()
            .chain(items.iter().map(|item| item.id.clone()).filter(|id| !self.editing_order.contains(id)))
            .collect()
    }
}

#[derive(Clone, PartialEq)]
enum PackGridCell {
    Add,
    Item(StickerItem),
}

/// one list row, each grid is cut into rows of columns so the 1800 emoji stay virtualised
#[derive(Clone, PartialEq)]
enum EmojiTabRow {
    CustomRecentsHeader,
    RecentsSlots(Vec<EmojiRecentSlot>),
    PackLoading(StickerPack),
    PackHeader { pack: StickerPack, items: Vec<StickerItem> },
    PackCells { pack: StickerPack, cells: Vec<PackGridCell> },
    UnicodeHeader { title: String, is_first: bool },
    UnicodeRecentsHeader,
    CatalogCells(&'static [EmojiPickerCatalogCell]),
    RecentCells(Vec<EmojiPickerCatalogCell>),
    BottomInset(f32),
}

impl EmojiTabRow {
    fn is_sticker_row(&self) -> bool {
        match self {
            Self::PackCells { cells, .. } => cells.iter().all(|cell| matches!(cell, PackGridCell::Item(_))),
            Self::RecentsSlots(slots) => slots.iter().all(|slot| matches!(slot, EmojiRecentSlot::Custom(_))),
            _ => false,
        }
    }

    /// height of a row drawn as a cached view, known before it renders
    /// sticker row is one cell square, plain emoji row is spacing over a cell of min height
    /// none for rows that size to content
    fn cached_height(&self, cell: f32) -> Option<f32> {
        if self.is_sticker_row() {
            return Some(cell);
        }
        match self {
            Self::CatalogCells(_) | Self::RecentCells(_) => Some(UNICODE_ROW_SPACING + UNICODE_CELL_MIN_HEIGHT),
            _ => None,
        }
    }

    fn unicode_emoji_text(&self) -> String {
        match self {
            Self::CatalogCells(cells) => cells.iter().map(|cell| cell.emoji.as_str()).collect(),
            Self::RecentCells(cells) => cells.iter().map(|cell| cell.emoji.as_str()).collect(),
            Self::RecentsSlots(slots) => slots
                .iter()
                .filter_map(|slot| match slot {
                    EmojiRecentSlot::Unicode(cell) => Some(cell.emoji.as_str()),
                    _ => None,
                })
                .collect(),
            _ => String::new(),
        }
    }

    fn unicode_emoji_cells(&self) -> Vec<String> {
        match self {
            Self::CatalogCells(cells) => cells.iter().map(|cell| cell.emoji.clone()).collect(),
            Self::RecentCells(cells) => cells.iter().map(|cell| cell.emoji.clone()).collect(),
            Self::RecentsSlots(slots) => slots
                .iter()
                .filter_map(|slot| match slot {
                    EmojiRecentSlot::Unicode(cell) => Some(cell.emoji.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn key(&self) -> String {
        match self {
            Self::CustomRecentsHeader => "recents-header".to_owned(),
            Self::RecentsSlots(slots) => {
                format!("recents/{}/{}", slots.first().map(EmojiRecentSlot::id).unwrap_or_default(), slots.len())
            }
            Self::PackLoading(pack) => format!("pack-loading/{}", pack.id),
            Self::PackHeader { pack, .. } => format!("pack-header/{}", pack.id),
            Self::PackCells { pack, cells } => {
                let first = match cells.first() {
                    Some(PackGridCell::Add) => "add",
                    Some(PackGridCell::Item(item)) => item.id.as_str(),
                    None => "",
                };
                format!("pack-cells/{}/{first}/{}", pack.id, cells.len())
            }
            Self::UnicodeHeader { title, .. } => format!("unicode-header/{title}"),
            Self::UnicodeRecentsHeader => "unicode-recents-header".to_owned(),
            Self::CatalogCells(cells) => format!("catalog/{}", cells.first().map(|cell| cell.id.as_str()).unwrap_or_default()),
            Self::RecentCells(cells) => format!("recent-cells/{}", cells.first().map(|cell| cell.id.as_str()).unwrap_or_default()),
            Self::BottomInset(inset) => format!("bottom/{inset}"),
        }
    }
}

/// what a row needs for one frame, shared by the list closure and the hold grid callbacks
struct EmojiTabContext {
    id: ElementId,
    state: Entity<ChatInputEmojiTabState>,
    services: Entity<StickersServicesStore>,
    on_select_emoji: EmojiHandler,
    on_select_custom_emoji: CustomEmojiHandler,
    self_is_premium: bool,
    is_active: bool,
    system_emoji_only: bool,
    uses_flat_chrome: bool,
    columns: usize,
    cell: f32,
    primary: Hsla,
    secondary: Hsla,
}

impl EmojiTabContext {
    fn draws_rows_like(&self, other: &Self) -> bool {
        self.id == other.id
            && self.state == other.state
            && self.services == other.services
            && self.self_is_premium == other.self_is_premium
            && self.is_active == other.is_active
            && self.system_emoji_only == other.system_emoji_only
            && self.uses_flat_chrome == other.uses_flat_chrome
            && self.columns == other.columns
            && self.cell == other.cell
            && self.primary == other.primary
            && self.secondary == other.secondary
    }

    fn child_id(&self, part: impl Into<SharedString>) -> ElementId {
        named_child(&self.id, part)
    }

    fn handle_primary(&self, entry: &StickerHoldEntry, window: &mut Window, cx: &mut App) {
        if self.state.read(cx).editing_pack_id.is_some() {
            return;
        }
        match &entry.asset {
            StickerPreviewAsset::Sticker(item) => {
                if is_locked_item(self.services.read(cx), self.self_is_premium, item) {
                    offer_premium(&format!("emoji_pack:{}", item.pack_id), self.self_is_premium, cx);
                    return;
                }
                self.pin_recents_if_needed(cx);
                EmojiPickerRecentsStore::shared(cx).update(cx, |store, cx| store.register_custom(&item.id, cx));
                let user_id = user_id(cx);
                if !user_id.is_empty() {
                    StickersInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.write_recent(&user_id, &item.id, Some(PackKind::Emoji), cx));
                }
                (self.on_select_custom_emoji)(item.clone(), window, cx);
            }
            StickerPreviewAsset::UnicodeEmoji(emoji) => self.select_unicode_emoji(emoji, window, cx),
            StickerPreviewAsset::Gif(..) => {}
        }
    }

    fn select_unicode_emoji(&self, emoji: &str, window: &mut Window, cx: &mut App) {
        let Some(normalized) = EmojiPickerCatalog::normalized_selectable_emoji(Some(emoji)) else {
            return;
        };
        self.pin_recents_if_needed(cx);
        EmojiPickerRecentsStore::shared(cx).update(cx, |store, cx| store.register(&normalized, cx));
        (self.on_select_emoji)(normalized, window, cx);
    }

    fn pin_recents_if_needed(&self, cx: &mut App) {
        if self.state.read(cx).pinned_recents.is_some() {
            return;
        }
        let user_id = user_id(cx);
        let custom =
            if self.system_emoji_only { Vec::new() } else { self.services.read(cx).recents(&user_id, Some(PackKind::Emoji)) };
        let recents = EmojiPickerRecentsStore::shared(cx);
        let (unicode, pick_order) = {
            let recents = recents.read(cx);
            (recents.recents(), recents.pick_order().to_vec())
        };
        self.state.update(cx, |state, cx| {
            state.pinned_recents = Some(PinnedEmojiRecents { custom, unicode, pick_order });
            cx.notify();
        });
    }

    fn custom_emoji_menu(self: &Rc<Self>, entry: &StickerHoldEntry, cx: &App) -> Vec<StickerPreviewAction> {
        let StickerPreviewAsset::Sticker(item) = &entry.asset else {
            return Vec::new();
        };
        let Some(pack) = self.services.read(cx).pack_summary(&item.pack_id).filter(|pack| pack.can_edit_stickers()) else {
            return Vec::new();
        };
        let (edit_pack, edit_item) = (pack.clone(), item.clone());
        let (delete_context, delete_pack, delete_item) = (self.clone(), pack, item.clone());
        vec![
            StickerPreviewAction::new(
                "edit",
                tr("edit_action"),
                "pencil",
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    StickerAssetComposerStore::shared(cx)
                        .update(cx, |store, cx| store.present_sticker(edit_pack.clone(), edit_item.clone(), cx));
                }),
            ),
            StickerPreviewAction::new(
                "delete",
                tr("delete_action"),
                "trash",
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    delete_context
                        .present_delete_alert(StickerItemDeletionRequest { item: delete_item.clone(), pack: delete_pack.clone() }, cx);
                }),
            )
            .destructive(true),
        ]
    }

    fn present_delete_alert(&self, request: StickerItemDeletionRequest, cx: &mut App) {
        let state = self.state.clone();
        NativeConfirmAlert::present_delete(
            &tr("picker_delete_emoji_title"),
            &tr("picker_delete_item_message"),
            Some(tr("delete_action")),
            None,
            false,
            None,
            None,
            move |cx| {
                state.update(cx, |state, cx| {
                    state.editing_order.retain(|id| *id != request.item.id);
                    cx.notify();
                });
                let user_id = user_id(cx);
                StickersInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.delete_sticker(&user_id, &request.pack.id, &request.item.id, cx));
            },
            cx,
        );
    }

    fn clear_all_recents(&self, cx: &mut App) {
        let user_id = user_id(cx);
        if !self.services.read(cx).recents(&user_id, Some(PackKind::Emoji)).is_empty() {
            StickersInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_recents(&user_id, PackKind::Emoji, cx));
        }
        self.clear_unicode_recents(cx);
    }

    fn clear_unicode_recents(&self, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.pinned_recents = None;
            cx.notify();
        });
        EmojiPickerRecentsStore::shared(cx).update(cx, |store, cx| store.clear(cx));
    }

    fn recents_clear_button(&self, id: &str, action: impl Fn(&mut App) + 'static) -> AnyElement {
        plain(self.child_id(format!("{id}/clear")))
            .on_press(move |_, cx| {
                let _ = selection_changed();
                action(cx);
            })
            .child(
                div()
                    .min_h(px(24.))
                    .pr(px(4.))
                    .flex()
                    .items_center()
                    .swiftui_font(Font::system_weight(12., FontWeight::MEDIUM))
                    .text_color(if self.uses_flat_chrome { self.secondary } else { white().opacity(0.38) })
                    .child(SharedString::from(tr("picker_recents_clear"))),
            )
            .into_any_element()
    }

    fn edit_button(self: &Rc<Self>, pack: &StickerPack, items: &[StickerItem], cx: &App) -> Option<AnyElement> {
        let editing_pack_id = self.state.read(cx).editing_pack_id.clone();
        if !can_edit_in_grid(pack) || editing_pack_id.as_deref().is_some_and(|id| id != pack.id) {
            return None;
        }
        let is_editing = editing_pack_id.is_some();
        let state = self.state.clone();
        let pack_id = pack.id.clone();
        let items = items.to_vec();
        Some(
            plain(self.child_id(format!("pack-edit/{}", pack.id)))
                .on_press(move |_, cx| {
                    let _ = selection_changed();
                    if is_editing {
                        let order = state.read(cx).reconciled_order(&items);
                        state.update(cx, |state, cx| {
                            state.end_editing();
                            cx.notify();
                        });
                        let user_id = user_id(cx);
                        StickersInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.reorder_stickers(&user_id, &pack_id, order, cx));
                    } else {
                        let order = items.iter().map(|item| item.id.clone()).collect();
                        state.update(cx, |state, cx| {
                            state.editing_pack_id = Some(pack_id.clone());
                            state.editing_order = order;
                            state.dragged_sticker_id = None;
                            cx.notify();
                        });
                    }
                })
                .child(
                    div()
                        .min_h(px(24.))
                        .pr(px(4.))
                        .flex()
                        .items_center()
                        .swiftui_font(Font::system_weight(12., FontWeight::SEMIBOLD))
                        .text_color(self.primary)
                        .child(SharedString::from(tr(if is_editing { "save" } else { "edit_action" }))),
                )
                .into_any_element(),
        )
    }

    fn unicode_cell(self: &Rc<Self>, cell: &EmojiPickerCatalogCell) -> AnyElement {
        let label = div()
            .w_full()
            .min_h(px(UNICODE_CELL_MIN_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .swiftui_font(Font::system_size(UNICODE_EMOJI_SIZE))
            .child(SharedString::from(cell.emoji.clone()));
        if self.system_emoji_only {
            let context = self.clone();
            let emoji = cell.emoji.clone();
            return plain(self.child_id(format!("emoji/{}", cell.id)))
                .on_press(move |window, cx| context.select_unicode_emoji(&emoji, window, cx))
                .w_full()
                .child(label)
                .into_any_element();
        }
        label
            .sticker_hold_cell_frame(cell.id.clone(), StickerPreviewAsset::UnicodeEmoji(cell.emoji.clone()), None)
            .into_any_element()
    }

    fn unicode_row<'a>(self: &Rc<Self>, cells: impl Iterator<Item = &'a EmojiPickerCatalogCell>) -> AnyElement {
        div()
            .w_full()
            .px(px(8.))
            .pt(px(UNICODE_ROW_SPACING))
            .child(div().w_full().grid().grid_cols(self.columns as u16).children(cells.map(|cell| self.unicode_cell(cell))))
            .into_any_element()
    }

    fn fixed_cell(&self, content: impl IntoElement) -> AnyElement {
        div().flex_none().w(px(self.cell)).child(content).into_any_element()
    }

    /// fixed column grid, columns times cell wide and centred in the padded stack
    /// cells of a short row fill its columns from the start
    fn fixed_row(&self, cells: Vec<AnyElement>) -> AnyElement {
        div()
            .w_full()
            .px(px(4.))
            .flex()
            .justify_center()
            .child(
                hstack(0_f32)
                    .flex_none()
                    .w(px(self.cell * self.columns as f32))
                    .children(cells)
                    .child(spacer_min_w(0.)),
            )
            .into_any_element()
    }

    fn pack_cell(self: &Rc<Self>, pack: &StickerPack, cell: &PackGridCell, cx: &App) -> AnyElement {
        let item = match cell {
            PackGridCell::Add => {
                let pack = pack.clone();
                let id = self.child_id(format!("pack-add/{}", pack.id));
                return self.fixed_cell(StickerPackAddCell::new(id, PackKind::Emoji, self.cell, move |_: &mut Window, cx: &mut App| {
                    StickerAssetComposerStore::shared(cx).update(cx, |store, cx| store.present(pack.clone(), cx));
                }));
            }
            PackGridCell::Item(item) => item,
        };
        if can_edit_in_grid(pack) {
            let is_editing = self.state.read(cx).editing_pack_id.as_deref() == Some(pack.id.as_str());
            let (order_read, order_write) = (self.state.clone(), self.state.clone());
            let (dragged_read, dragged_write) = (self.state.clone(), self.state.clone());
            let delete_context = self.clone();
            let request = StickerItemDeletionRequest { item: item.clone(), pack: pack.clone() };
            return self.fixed_cell(ChatInputEditableStickerCell::new(
                item.clone(),
                self.cell,
                self.is_active,
                is_editing,
                Binding::new(
                    move |cx: &App| order_read.read(cx).editing_order.clone(),
                    move |order: Vec<String>, _: &mut Window, cx: &mut App| {
                        order_write.update(cx, |state, cx| {
                            state.editing_order = order;
                            cx.notify();
                        });
                    },
                ),
                Binding::new(
                    move |cx: &App| dragged_read.read(cx).dragged_sticker_id.clone(),
                    move |dragged: Option<String>, _: &mut Window, cx: &mut App| {
                        dragged_write.update(cx, |state, cx| {
                            state.dragged_sticker_id = dragged;
                            cx.notify();
                        });
                    },
                ),
                pack.can_delete_stickers().then(|| {
                    Rc::new(move |_: &mut Window, cx: &mut App| delete_context.present_delete_alert(request.clone(), cx))
                        as Rc<dyn Fn(&mut Window, &mut App)>
                }),
            )
            .can_reorder(pack.can_edit_stickers()));
        }
        let locked = is_locked_pack(self.services.read(cx), self.self_is_premium, pack);
        self.fixed_cell(
            div()
                .relative()
                .child(ChatInputStickerCell::new(item.clone(), self.cell).is_playing(self.is_active))
                .children(locked.then(|| div().absolute().bottom_0().right_0().child(ChatInputLockedStickerMark))),
        )
    }

    fn pack_header(self: &Rc<Self>, pack: &StickerPack, items: &[StickerItem], cx: &App) -> AnyElement {
        let mut header = ChatInputSectionHeader::new(self.child_id(format!("pack/{}", pack.id)), pack.title.clone());
        if is_locked_pack(self.services.read(cx), self.self_is_premium, pack) {
            let self_is_premium = self.self_is_premium;
            let source = format!("emoji_pack:{}", pack.id);
            header = header.trailing(StickerPackLockBadge::new(self.child_id(format!("pack-lock/{}", pack.id)), move |_, cx| {
                offer_premium(&source, self_is_premium, cx);
            }));
        } else if let Some(edit) = self.edit_button(pack, items, cx) {
            header = header.trailing(edit);
        }
        if can_manage(pack) {
            let pack_id = pack.id.clone();
            header = header.title_action(move |_, cx| {
                StickerPackSheetStore::shared(cx).update(cx, |store, cx| store.present(&pack_id, PackKind::Emoji, cx));
            });
        }
        let visible_pack_id = pack.id.clone();
        div()
            .w_full()
            .px(px(4.))
            .child(header)
            .on_appear(self.child_id(format!("pack-appear/{}", pack.id)), move |_, cx| {
                load_more_emoji_packs_if_needed(&visible_pack_id, cx);
            })
            .into_any_element()
    }

    fn render_row(self: &Rc<Self>, row: &EmojiTabRow, cx: &App) -> AnyElement {
        match row {
            EmojiTabRow::CustomRecentsHeader => {
                let context = self.clone();
                div()
                    .w_full()
                    .px(px(4.))
                    .child(
                        ChatInputSectionHeader::new(self.child_id("recents-header"), tr("emoji_recents"))
                            .trailing(self.recents_clear_button("recents", move |cx| context.clear_all_recents(cx))),
                    )
                    .into_any_element()
            }
            EmojiTabRow::RecentsSlots(slots) => self.fixed_row(
                slots
                    .iter()
                    .map(|slot| {
                        let content = match slot {
                            EmojiRecentSlot::Custom(item) => ChatInputStickerCell::new(item.clone(), self.cell)
                                .is_playing(self.is_active)
                                .into_any_element(),
                            EmojiRecentSlot::Unicode(cell) => self.unicode_cell(cell),
                        };
                        self.fixed_cell(div().id(self.child_id(format!("recent-slot/{}", slot.id()))).child(content))
                    })
                    .collect(),
            ),
            EmojiTabRow::PackLoading(pack) => {
                let (load_pack_id, visible_pack_id) = (pack.id.clone(), pack.id.clone());
                div()
                    .w_full()
                    .px(px(4.))
                    .child(ChatInputPackLoadingSection::new(
                        pack.title.clone(),
                        anchor_for_pack_id(&pack.id),
                        self.columns,
                        self.cell,
                        self.is_active,
                    ))
                    .on_appear(self.child_id(format!("pack-loading/{}", pack.id)), move |_, cx| {
                        StickersInteractionsStore::shared(cx).update(cx, |store, cx| store.load_pack_stickers(&load_pack_id, cx));
                        load_more_emoji_packs_if_needed(&visible_pack_id, cx);
                    })
                    .into_any_element()
            }
            EmojiTabRow::PackHeader { pack, items } => self.pack_header(pack, items, cx),
            EmojiTabRow::PackCells { pack, cells } => {
                self.fixed_row(cells.iter().map(|cell| self.pack_cell(pack, cell, cx)).collect())
            }
            EmojiTabRow::UnicodeHeader { title, is_first } => div()
                .w_full()
                .px(px(8.))
                .pt(px(if *is_first { 0. } else { UNICODE_ROW_SPACING }))
                .child(ChatInputSectionHeader::new(self.child_id(format!("unicode-header/{title}")), title.clone()))
                .into_any_element(),
            EmojiTabRow::UnicodeRecentsHeader => {
                let context = self.clone();
                div()
                    .w_full()
                    .px(px(8.))
                    .pt(px(UNICODE_ROW_SPACING))
                    .child(
                        ChatInputSectionHeader::new(self.child_id("unicode-recents-header"), tr("emoji_recents"))
                            .trailing(self.recents_clear_button("unicode-recents", move |cx| context.clear_unicode_recents(cx))),
                    )
                    .into_any_element()
            }
            EmojiTabRow::CatalogCells(cells) => self.unicode_row(cells.iter()),
            EmojiTabRow::RecentCells(cells) => self.unicode_row(cells.iter()),
            EmojiTabRow::BottomInset(inset) => div().w_full().h(px(*inset)).into_any_element(),
        }
    }
}

/// one sticker or emoji row as its own cached view
/// a wheel step replays it moved, and a sticker frame redraws this row and not the panel
struct EmojiTabRowView {
    row: Option<EmojiTabRow>,
    context: Option<Rc<EmojiTabContext>>,
    marks: EmojiTabRowMarks,
}

/// what a pack row reads while it renders besides its cells
/// edit mode with drag order and dragged sticker, and the premium lock. any change rerenders the row
#[derive(Clone, Default, PartialEq)]
struct EmojiTabRowMarks {
    is_editing: bool,
    editing_order: Vec<String>,
    dragged_sticker_id: Option<String>,
    is_locked: bool,
}

impl EmojiTabRowMarks {
    fn of(row: &EmojiTabRow, context: &EmojiTabContext, cx: &App) -> Self {
        let EmojiTabRow::PackCells { pack, .. } = row else {
            return Self::default();
        };
        let state = context.state.read(cx);
        let is_editing = state.editing_pack_id.as_deref() == Some(pack.id.as_str());
        Self {
            is_editing,
            editing_order: if is_editing { state.editing_order.clone() } else { Vec::new() },
            dragged_sticker_id: if is_editing { state.dragged_sticker_id.clone() } else { None },
            is_locked: is_locked_pack(context.services.read(cx), context.self_is_premium, pack),
        }
    }
}

impl EmojiTabRowView {
    fn set_props(&mut self, row: &EmojiTabRow, context: &Rc<EmojiTabContext>, marks: EmojiTabRowMarks) -> bool {
        let stale = self.row.as_ref() != Some(row)
            || self.marks != marks
            || !self.context.as_ref().is_some_and(|current| current.draws_rows_like(context));
        let drawn_before = self.row.is_some();
        if stale {
            self.row = Some(row.clone());
            self.marks = marks;
        }
        self.context = Some(context.clone());
        stale && drawn_before
    }

    fn element(
        state: &Entity<ChatInputEmojiTabState>,
        key: &str,
        row: &EmojiTabRow,
        context: &Rc<EmojiTabContext>,
        cx: &mut App,
    ) -> AnyElement {
        let existing = state.read(cx).row_views.get(key).cloned();
        let view = match existing {
            Some(view) => view,
            None => {
                let view = cx.new(|_| EmojiTabRowView { row: None, context: None, marks: EmojiTabRowMarks::default() });
                let inserted = view.clone();
                state.update(cx, |state, _| {
                    state.row_views.insert(key.to_owned(), inserted);
                });
                view
            }
        };
        let marks = EmojiTabRowMarks::of(row, context, cx);
        let stale = view.update(cx, |view, _| view.set_props(row, context, marks));
        let height = row.cached_height(context.cell);
        if stale || height.is_none() {
            AnyView::from(view).into_any_element()
        } else {
            AnyView::from(view)
                .cached(StyleRefinement::default().w_full().h(px(height.unwrap_or(context.cell))))
                .replay_when_moved()
                .into_any_element()
        }
    }
}

impl Render for EmojiTabRowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match (&self.row, &self.context) {
            (Some(row), Some(context)) => context.render_row(row, cx),
            _ => div().into_any_element(),
        }
    }
}

#[derive(Default)]
struct EmojiTabRows {
    rows: Vec<EmojiTabRow>,
    anchors: HashMap<String, usize>,
}

impl EmojiTabRows {
    fn anchor(&mut self, anchor: impl Into<String>) {
        self.anchors.insert(anchor.into(), self.rows.len());
    }

    fn unicode_sections(&mut self, columns: usize, system_emoji_only: bool, recent_cells: &[EmojiPickerCatalogCell]) {
        self.anchor(POPULAR_ANCHOR);
        self.rows.push(EmojiTabRow::UnicodeHeader { title: tr("emoji_popular_header"), is_first: true });
        self.rows.extend(EmojiPickerCatalog::reaction_cells().chunks(columns).map(EmojiTabRow::CatalogCells));
        if system_emoji_only && !recent_cells.is_empty() {
            self.anchor(UNICODE_RECENTS_ANCHOR);
            self.rows.push(EmojiTabRow::UnicodeRecentsHeader);
            self.rows.extend(recent_cells.chunks(columns).map(|cells| EmojiTabRow::RecentCells(cells.to_vec())));
        }
        for category in EmojiPickerCatalog::categories() {
            self.anchor(category.id.clone());
            self.rows.push(EmojiTabRow::UnicodeHeader { title: category.title(), is_first: false });
            self.rows.extend(category.cells.chunks(columns).map(EmojiTabRow::CatalogCells));
        }
    }

    fn pack_sections(
        &mut self,
        visible: &[StickerPack],
        services: &StickersServicesStore,
        state: &ChatInputEmojiTabState,
        columns: usize,
    ) {
        for pack in visible {
            self.anchor(anchor_for_pack_id(&pack.id));
            if !services.has_resolved_pack(&pack.id) {
                self.rows.push(EmojiTabRow::PackLoading(pack.clone()));
                continue;
            }
            let items = services.pack_stickers(&pack.id);
            let displayed = state.ordered_items(items.clone(), pack);
            let is_editing = state.editing_pack_id.as_deref() == Some(pack.id.as_str());
            let mut cells: Vec<PackGridCell> = Vec::with_capacity(displayed.len() + 1);
            if pack.can_add_stickers() && !is_editing {
                cells.push(PackGridCell::Add);
            }
            cells.extend(displayed.into_iter().map(PackGridCell::Item));
            self.rows.push(EmojiTabRow::PackHeader { pack: pack.clone(), items });
            self.rows.extend(cells.chunks(columns).map(|cells| EmojiTabRow::PackCells { pack: pack.clone(), cells: cells.to_vec() }));
        }
    }
}

#[derive(IntoElement)]
pub struct ChatInputEmojiTab {
    id: ElementId,
    on_select_emoji: EmojiHandler,
    on_select_custom_emoji: CustomEmojiHandler,
    on_backspace: BackspaceHandler,
    safe_area_bottom: f32,
    is_active: bool,
    reserves_picker_bottom_bar: bool,
    uses_flat_chrome: bool,
    defers_loading_to_parent: bool,
    system_emoji_only: bool,
    shows_backspace: bool,
}

impl ChatInputEmojiTab {
    pub fn new(
        id: impl Into<ElementId>,
        on_select_emoji: impl Fn(String, &mut Window, &mut App) + 'static,
        on_select_custom_emoji: impl Fn(StickerItem, &mut Window, &mut App) + 'static,
        on_backspace: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            on_select_emoji: Rc::new(on_select_emoji),
            on_select_custom_emoji: Rc::new(on_select_custom_emoji),
            on_backspace: Rc::new(on_backspace),
            safe_area_bottom: 0.0,
            is_active: true,
            reserves_picker_bottom_bar: true,
            uses_flat_chrome: false,
            defers_loading_to_parent: false,
            system_emoji_only: false,
            shows_backspace: true,
        }
    }

    pub fn safe_area_bottom(mut self, safe_area_bottom: f32) -> Self {
        self.safe_area_bottom = safe_area_bottom;
        self
    }

    pub fn is_active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }

    pub fn reserves_picker_bottom_bar(mut self, reserves_picker_bottom_bar: bool) -> Self {
        self.reserves_picker_bottom_bar = reserves_picker_bottom_bar;
        self
    }

    pub fn uses_flat_chrome(mut self, uses_flat_chrome: bool) -> Self {
        self.uses_flat_chrome = uses_flat_chrome;
        self
    }

    pub fn defers_loading_to_parent(mut self, defers_loading_to_parent: bool) -> Self {
        self.defers_loading_to_parent = defers_loading_to_parent;
        self
    }

    pub fn system_emoji_only(mut self, system_emoji_only: bool) -> Self {
        self.system_emoji_only = system_emoji_only;
        self
    }

    pub fn shows_backspace(mut self, shows_backspace: bool) -> Self {
        self.shows_backspace = shows_backspace;
        self
    }

    fn child_id(&self, part: impl Into<SharedString>) -> ElementId {
        named_child(&self.id, part)
    }

    fn load_all(system_emoji_only: bool, cx: &mut App) {
        if system_emoji_only {
            return;
        }
        let user_id = user_id(cx);
        if user_id.is_empty() {
            return;
        }
        StickersInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.load_recents(&user_id, Some(PackKind::Emoji), cx);
            store.load_saved_packs(&user_id, PackKind::Emoji, None, false, cx);
        });
    }

    fn scroll_to_pack(state: &Entity<ChatInputEmojiTabState>, pack_id: &str, cx: &mut App) {
        if !StickersServicesStore::shared(cx).read(cx).has_resolved_pack(pack_id) {
            state.update(cx, |state, _| state.pending_pack_scroll_id = Some(pack_id.to_owned()));
            StickersInteractionsStore::shared(cx).update(cx, |store, cx| store.load_pack_stickers(pack_id, cx));
            return;
        }
        state.update(cx, |state, cx| state.scroll_to(&anchor_for_pack_id(pack_id), cx));
    }

    fn top_strip(
        &self,
        state: &Entity<ChatInputEmojiTabState>,
        layout: &ChatInputPackLayout,
        recents_anchor: &'static str,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let services = StickersServicesStore::shared(cx);
        let mut buttons: Vec<AnyElement> = Vec::new();
        let button_width = self.strip_button_width(state.read(cx).width, layout);

        let recents_state = state.clone();
        buttons.push(
            ChatInputStripIconButton::new(self.child_id("strip-recents"), Some("clock"), None, false, tr("emoji_recents"), move |_, cx| {
                recents_state.update(cx, |state, cx| state.scroll_to(recents_anchor, cx));
            })
            .width(button_width)
            .into_any_element(),
        );

        if !self.system_emoji_only {
            for pack in &layout.ordered {
                let services = services.read(cx);
                let first = services.pack_stickers(&pack.id).into_iter().next();
                let cover = if can_manage(pack) && !pack.is_default { first.clone() } else { None }.or_else(|| pack.cover_item()).or(first);
                let tap_state = state.clone();
                let pack_id = pack.id.clone();
                buttons.push(
                    ChatInputStripIconButton::new(
                        self.child_id(format!("strip-pack/{}", pack.id)),
                        Some("face.smiling"),
                        cover,
                        false,
                        pack.title.clone(),
                        move |_, cx| Self::scroll_to_pack(&tap_state, &pack_id, cx),
                    )
                    .into_any_element(),
                );
            }
        }

        for (index, category) in EmojiPickerCatalog::categories().iter().enumerate() {
            let tap_state = state.clone();
            buttons.push(
                ChatInputStripIconButton::new(
                    self.child_id(format!("strip-category/{index}")),
                    Some(category.symbol.as_str()),
                    None,
                    false,
                    category.title(),
                    move |_, cx| tap_state.update(cx, |state, cx| state.scroll_to(&category.id, cx)),
                )
                .width(button_width)
                .into_any_element(),
            );
        }

        let strip = state.read(cx).strip.clone();
        self.load_more_from_strip(&strip, layout, window, cx);
        div()
            .id(self.child_id("strip"))
            .flex_1()
            .min_w_0()
            .h(px(ChatInputPickerMetrics::TOP_STRIP_HEIGHT))
            .flex()
            .items_center()
            .overflow_x_scroll()
            .track_scroll(&strip)
            .child(hstack(STRIP_SPACING).flex_none().px(px(STRIP_LEADING_PADDING)).children(buttons))
            .into_any_element()
    }

    fn strip_button_width(&self, tab_width: f32, layout: &ChatInputPackLayout) -> f32 {
        let has_packs = !self.system_emoji_only && !layout.ordered.is_empty();
        if has_packs || tab_width <= 0.0 {
            return ChatInputStripIconButton::WIDTH;
        }
        let count = 1 + EmojiPickerCatalog::categories().len();
        let backspace = if self.shows_backspace { BACKSPACE_SLOT } else { 0.0 };
        let gaps = STRIP_SPACING * (count - 1) as f32;
        let available = tab_width - backspace - 2.0 * STRIP_LEADING_PADDING - gaps;
        (available / count as f32).floor().clamp(STRIP_MIN_BUTTON_WIDTH, ChatInputStripIconButton::WIDTH)
    }

    fn load_more_from_strip(&self, strip: &ScrollHandle, layout: &ChatInputPackLayout, window: &mut Window, cx: &mut App) {
        let viewport = f32::from(strip.bounds().size.width);
        let Some(pack) = layout.ordered.len().checked_sub(PACKS_LOAD_MORE_DISTANCE).and_then(|index| layout.ordered.get(index)).or(layout.ordered.first()) else {
            return;
        };
        if self.system_emoji_only || viewport <= 0.0 {
            return;
        }
        let index = layout.ordered.iter().position(|ordered| ordered.id == pack.id).unwrap_or(0);
        let leading = STRIP_LEADING_PADDING + STRIP_BUTTON_SLOT * (index + 1) as f32;
        let appeared = leading < viewport - f32::from(strip.offset().x);
        let user_id = user_id(cx);
        let cursor = if appeared {
            StickersServicesStore::shared(cx).read(cx).saved_packs_next_relative_id(&user_id, PackKind::Emoji)
        } else {
            None
        };
        let visible_pack_id = pack.id.clone();
        on_change(self.child_id("strip-load-more"), cursor, true, window, cx, move |_, cursor, _, cx| {
            if cursor.is_some() {
                load_more_emoji_packs_if_needed(&visible_pack_id, cx);
            }
        });
    }

    fn backspace_key(&self, theme_text: Hsla, theme_bg: Hsla) -> impl IntoElement {
        let flat_chrome = self.uses_flat_chrome;
        let on_backspace = self.on_backspace.clone();
        let height = ChatInputPickerMetrics::SEGMENT_HEIGHT;
        let mut key = div()
            .id(self.child_id("backspace-label"))
            .tooltip_label(tr("delete_action"))
            .relative()
            .frame_size(50_f32, height)
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .overflow_hidden();
        if flat_chrome {
            key = key.bg(theme_bg);
        } else {
            key = key
                .border(px(0.6))
                .border_color(white().opacity(0.12))
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(HoldContextMenuGlass::new(self.child_id("backspace-glass")).tint(black().opacity(0.26)).corner_radius(height / 2.0).opaque(true)),
                );
        }
        let color = if flat_chrome { theme_text } else { white().opacity(0.8) };
        div().flex_none().pl(px(4.)).pr(px(10.)).child(
            plain(self.child_id("backspace"))
                .on_press(move |window, cx| {
                    let _ = impact(ImpactStyle::Light);
                    on_backspace(window, cx);
                })
                .child(key.children(IconUi::sized("delete.left.fill", px(18.), color, true))),
        )
    }
}

impl RenderOnce for ChatInputEmojiTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, cx| ChatInputEmojiTabState::new(cx));
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let services = StickersServicesStore::shared(cx);
        let recents = EmojiPickerRecentsStore::shared(cx);
        let presenter = StickerHoldPreviewPresenter::shared(cx);
        let user_id_value = user_id(cx);
        let self_is_premium_value = self_is_premium(cx);
        let (width, height) = {
            let state = state.read(cx);
            (state.width, state.height)
        };
        let columns = ChatInputPickerMetrics::emoji_columns(width).max(1);
        let cell = (width / columns as f32).floor();

        let layout = {
            let services = services.read(cx);
            let saved = services.saved_pack_summaries(&user_id_value, PackKind::Emoji);
            ChatInputPackLayout::with_options(
                &saved,
                |id| services.has_resolved_pack(id),
                |id| services.is_pack_unavailable(id),
                saved.len() as i64,
            )
        };

        let (shown_custom, shown_unicode, shown_pick_order) = match state.read(cx).pinned_recents.clone() {
            Some(pinned) => (pinned.custom, pinned.unicode, pinned.pick_order),
            None => {
                let recents = recents.read(cx);
                (
                    services.read(cx).recents(&user_id_value, Some(PackKind::Emoji)),
                    recents.recents(),
                    recents.pick_order().to_vec(),
                )
            }
        };
        let recents_anchor = if self.system_emoji_only {
            if shown_unicode.is_empty() { POPULAR_ANCHOR } else { UNICODE_RECENTS_ANCHOR }
        } else if shown_unicode.is_empty() && shown_custom.is_empty() {
            POPULAR_ANCHOR
        } else {
            CUSTOM_RECENTS_ANCHOR
        };
        let recent_cells = EmojiPickerCatalog::make_cells(&shown_unicode, UNICODE_RECENTS_ANCHOR);

        let mut rows = EmojiTabRows::default();
        if !self.system_emoji_only {
            if !shown_custom.is_empty() || !recent_cells.is_empty() {
                rows.anchor(CUSTOM_RECENTS_ANCHOR);
                rows.rows.push(EmojiTabRow::CustomRecentsHeader);
                let slots: Vec<EmojiRecentSlot> = EmojiPickerRecentsStore::ordered(
                    &shown_pick_order,
                    &shown_custom,
                    |item: &StickerItem| item.id.clone(),
                    &recent_cells,
                    |item: &StickerItem| EmojiRecentSlot::Custom(item.clone()),
                    |cell: &EmojiPickerCatalogCell| EmojiRecentSlot::Unicode(cell.clone()),
                );
                rows.rows.extend(slots.chunks(columns).map(|slots| EmojiTabRow::RecentsSlots(slots.to_vec())));
            }
            rows.pack_sections(&layout.visible, services.read(cx), state.read(cx), columns);
        }
        rows.unicode_sections(columns, self.system_emoji_only, &recent_cells);
        rows.rows.push(EmojiTabRow::BottomInset(if self.shows_backspace {
            self.safe_area_bottom + ChatInputPickerMetrics::GRID_BOTTOM_INSET
        } else if self.reserves_picker_bottom_bar {
            ChatInputPickerMetrics::GRID_BOTTOM_INSET + chat_input_picker_bar_reserve(cx)
        } else {
            BARE_GRID_BOTTOM_INSET
        }));

        let keys: Vec<String> = rows.rows.iter().map(EmojiTabRow::key).collect();
        let view_keys: Vec<Option<String>> = {
            let mut occurrences: HashMap<&str, usize> = HashMap::new();
            rows.rows
                .iter()
                .zip(keys.iter())
                .map(|(row, key)| {
                    (!self.system_emoji_only && row.cached_height(cell).is_some()).then(|| {
                        let occurrence = occurrences.entry(key.as_str()).or_insert(0);
                        *occurrence += 1;
                        format!("{key}#{occurrence}")
                    })
                })
                .collect()
        };
        let live_views: HashSet<String> = view_keys.iter().flatten().cloned().collect();
        let anchors = rows.anchors;
        let grid = state.update(cx, |state, cx| {
            state.row_views.retain(|key, _| live_views.contains(key));
            state.sync_rows(keys);
            state.anchors = anchors;
            state.keep_scroll_run_on_anchor(cx);
            state.grid_scroll.frame(&state.grid, window);
            state.warm_rows_near_scroll_top(&rows.rows, window, cx);
            state.grid.paint_under_top_padding(true);
            state.grid.clone()
        });
        let edge_blur = ChatInputPickerMetrics::strip_edge_blur(grid.logical_scroll_top(), &theme);

        let pending_state = state.clone();
        on_change(
            self.child_id("resolved"),
            layout.plan.resolved_pack_ids.clone(),
            false,
            window,
            cx,
            move |_, resolved_ids, _, cx| {
                pending_state.update(cx, |state, cx| {
                    let Some(id) = state.pending_pack_scroll_id.clone().filter(|id| resolved_ids.contains(id)) else {
                        return;
                    };
                    state.pending_pack_scroll_id = None;
                    state.scroll_to(&anchor_for_pack_id(&id), cx);
                });
            },
        );
        let active_state = state.clone();
        on_change(self.child_id("is-active"), self.is_active, false, window, cx, move |_, active, _, cx| {
            if !*active {
                active_state.update(cx, |state, cx| {
                    state.pinned_recents = None;
                    cx.notify();
                });
            }
        });
        let account_state = state.clone();
        on_change(self.child_id("user"), user_id_value.clone(), false, window, cx, move |_, _, _, cx| {
            account_state.update(cx, |state, cx| {
                state.pending_pack_scroll_id = None;
                state.pinned_recents = None;
                state.end_editing();
                cx.notify();
            });
        });

        let context = Rc::new(EmojiTabContext {
            id: self.id.clone(),
            state: state.clone(),
            services: services.clone(),
            on_select_emoji: self.on_select_emoji.clone(),
            on_select_custom_emoji: self.on_select_custom_emoji.clone(),
            self_is_premium: self_is_premium_value,
            is_active: self.is_active,
            system_emoji_only: self.system_emoji_only,
            uses_flat_chrome: self.uses_flat_chrome,
            columns,
            cell,
            primary: theme.primary._100,
            secondary: theme.text.secondary,
        });

        let is_editing = state.read(cx).editing_pack_id.is_some();
        let hold_active = presenter.read(cx).is_active();
        let strip = self.top_strip(&state, &layout, recents_anchor, window, cx);

        let row_list = Rc::new(rows.rows);
        let row_view_keys = Rc::new(view_keys);
        let list_context = context.clone();
        let row_state = state.clone();
        let grid_list = list(grid, move |index, _window, cx| {
            let Some(row) = row_list.get(index) else {
                return div().into_any_element();
            };
            match row_view_keys.get(index).and_then(Option::as_deref) {
                Some(key) => EmojiTabRowView::element(&row_state, key, row, &list_context, cx),
                None => list_context.render_row(row, cx),
            }
        })
        .w(px(width))
        .h(px(height))
        .pt(px(ChatInputPickerMetrics::TOP_STRIP_HEIGHT));

        let grid_element = if width <= 0.0 || height <= 0.0 {
            None
        } else if self.system_emoji_only {
            Some(grid_list.into_any_element())
        } else {
            let primary_context = context.clone();
            let menu_context = context.clone();
            Some(
                grid_list
                    .sticker_hold_grid(
                        StickerHoldGridModifier::new(
                            self.child_id("hold"),
                            Rc::new(move |entry: &StickerHoldEntry, window: &mut Window, cx: &mut App| {
                                primary_context.handle_primary(entry, window, cx);
                            }),
                        )
                        .menu_for(Rc::new(move |entry: &StickerHoldEntry, cx: &App| menu_context.custom_emoji_menu(entry, cx)))
                        .uses_flat_chrome(self.uses_flat_chrome)
                        .is_enabled(!is_editing),
                    )
                    .into_any_element(),
            )
        };

        let backspace = self.shows_backspace.then(|| self.backspace_key(theme.text.primary, theme.bg._100));
        let strip_row = hstack(0_f32).w_full().flex_none().child(strip).children(backspace);
        let size_state = state.clone();
        let grid_slot = div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .children(grid_element)
            .children(edge_blur)
            .child(div().absolute().top_0().left_0().right_0().occlude().child(strip_row))
            .children(hold_active.then(|| div().absolute().inset_0().on_scroll_wheel(|_, _, cx| cx.stop_propagation())))
            .child(measure(self.child_id("measure"), move |bounds, _window, cx| {
                let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                size_state.update(cx, |state, cx| {
                    if state.width != width || state.height != height {
                        state.width = width;
                        state.height = height;
                        cx.notify();
                    }
                });
            }));

        let (defers_loading, system_only) = (self.defers_loading_to_parent, self.system_emoji_only);
        let disappear_state = state.clone();
        env_scope::<ChatInputPickerFlatChromeKey>(self.uses_flat_chrome).child(
            div()
                .size_full()
                .flex()
                .flex_col()
                .capture_any_mouse_down(|_, window, _| window.prevent_default())
                .child(grid_slot)
                .on_appear(self.child_id("appear"), move |_, cx| {
                    if !defers_loading {
                        Self::load_all(system_only, cx);
                    }
                })
                .on_disappear(self.child_id("disappear"), move |_, cx| {
                    disappear_state.update(cx, |state, cx| {
                        state.pinned_recents = None;
                        state.row_views.clear();
                        cx.notify();
                    });
                }),
        )
    }
}
