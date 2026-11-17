use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, AnyView, App, BoxShadow, Context, ElementId, Entity, Focusable, FontWeight, Hsla, ListAlignment,
    ListOffset, ListState, Pixels, Render, SharedString, StyleRefinement, Subscription, Window, black, list, point, white,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{LanguageStore, Locale, tr};
use rise_platform::haptics::{ImpactStyle, impact, selection_changed};
use rise_theme::{RiseThemeAppearance, ThemeStore};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_glass::HoldContextMenuGlass;
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::input_ui::{InputUiEvent, InputUiState};
use rise_ui::lottie_view::LottieView;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::skeleton_ui::SkeletonBlock;
use rise_ui::swiftui::button_style::{HOVER_OPACITY, button_chrome_hover};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::lib::sai::sai_auth_hooks::sai_auth_hooks;
use crate::core::ui::emoji_picker_ui::emoji_picker_catalog::{EmojiPickerCatalog, EmojiPickerCatalogCell};
use crate::core::ui::emoji_picker_ui::emoji_picker_pinned_pack::{EmojiPickerPinnedItem, EmojiPickerPinnedPack};
use crate::core::ui::emoji_picker_ui::emoji_picker_recents_store::{EmojiPickerRecentsStore, EmojiRecentSlot};
use crate::core::ui::emoji_picker_ui::emoji_picker_stickers_hooks::EmojiPickerStickersHooks;
use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit::ChatInputStickerCell;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{StickerItem, StickerPack};

type PickHandler = Rc<dyn Fn(String, &mut Window, &mut App)>;
type CustomPickHandler = Rc<dyn Fn(StickerItem, &mut Window, &mut App)>;
type PinnedPickHandler = Rc<dyn Fn(EmojiPickerPinnedItem, &mut Window, &mut App)>;
type CreatePackHandler = Rc<dyn Fn(&mut Window, &mut App)>;

const RECENTS_SECTION_ID: &str = "sec_recents";
const TOP_SECTION_ID: &str = "sec_top";
const CUSTOM_SECTION_ID: &str = "sec_custom";
const SEARCH_SECTION_ID: &str = "sec_search";
const CUSTOM_CELL_SIDE: f32 = 36.0;
const CUSTOM_SEARCH_RESULT_LIMIT: usize = 120;
const CUSTOM_SEARCH_SCAN_LIMIT: usize = 30_000;
const SEARCH_PREFETCH_PACKS_PER_STEP: usize = 6;
const COLUMN_COUNT: usize = 8;
const GRID_ROW_SPACING: f32 = 6.0;
const CELL_MIN_HEIGHT: f32 = 40.0;
const RECENTS_HEADER_MIN_HEIGHT: f32 = 32.0;
const NEW_ROWS_PER_DRAW: usize = 5;
const SKELETON_CELL_COUNT: usize = 24;
const CELL_HOVER_FILL_OPACITY: f32 = 0.08;
const CELL_HOVER_CORNER_RADIUS: f32 = 8.0;

#[derive(IntoElement)]
pub struct EmojiPickerUi {
    id: ElementId,
    pub on_pick: PickHandler,
    pub chrome: bool,
    pub shows_custom_emoji: bool,
    pub on_pick_custom: Option<CustomPickHandler>,
    pub custom_emoji_only: bool,
    pub pinned_pack: Option<EmojiPickerPinnedPack>,
    pub on_create_custom_emoji_pack: Option<CreatePackHandler>,
    pub rows_settling: bool,
}

impl EmojiPickerUi {
    pub fn new(id: impl Into<ElementId>, on_pick: impl Fn(String, &mut Window, &mut App) + 'static) -> Self {
        Self {
            id: id.into(),
            on_pick: Rc::new(on_pick),
            chrome: true,
            shows_custom_emoji: false,
            on_pick_custom: None,
            custom_emoji_only: false,
            pinned_pack: None,
            on_create_custom_emoji_pack: None,
            rows_settling: false,
        }
    }

    pub fn rows_settling(mut self, rows_settling: bool) -> Self {
        self.rows_settling = rows_settling;
        self
    }

    pub fn chrome(mut self, chrome: bool) -> Self {
        self.chrome = chrome;
        self
    }

    pub fn shows_custom_emoji(mut self, shows_custom_emoji: bool) -> Self {
        self.shows_custom_emoji = shows_custom_emoji;
        self
    }

    pub fn on_pick_custom(mut self, on_pick_custom: impl Fn(StickerItem, &mut Window, &mut App) + 'static) -> Self {
        self.on_pick_custom = Some(Rc::new(on_pick_custom));
        self
    }

    pub fn custom_emoji_only(mut self, custom_emoji_only: bool) -> Self {
        self.custom_emoji_only = custom_emoji_only;
        self
    }

    pub fn pinned_pack(mut self, pinned_pack: Option<EmojiPickerPinnedPack>) -> Self {
        self.pinned_pack = pinned_pack;
        self
    }

    pub fn on_create_custom_emoji_pack(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_create_custom_emoji_pack = Some(Rc::new(action));
        self
    }

    fn child_id(&self, name: impl Into<SharedString>) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }
}

/// view state, the search field
/// strip selection and grid scroll position live as long as the picker is drawn
struct EmojiPickerUiState {
    search: Entity<InputUiState>,
    strip_selection: String,
    grid: ListState,
    row_keys: Vec<String>,
    data_revision: u64,
    grid_memo: Option<GridMemo>,
    row_views: HashMap<String, Entity<GridRowView>>,
    row_views_foreground: Option<Hsla>,
    rows_built_this_draw: usize,
    rows_follow_up_scheduled: bool,
    row_context: Option<SharedGridContext>,
    custom_active: bool,
    search_requested_packs: HashSet<String>,
    search_requested_cursor: Option<String>,
    _subscriptions: Vec<Subscription>,
}

/// everything the grid rows are built from besides the stores behind data_revision
#[derive(PartialEq)]
struct GridMemoKey {
    revision: u64,
    query: String,
    user_id: String,
    locale: Locale,
    shows_custom_emoji: bool,
    custom_emoji_only: bool,
    picks_custom: bool,
    pinned_pack_id: Option<String>,
}

/// rows of the last build
/// an ancestor redraw redraws the picker, rebuilding every cell and key each frame is too costly
/// so rows are rebuilt only when their inputs change
struct GridMemo {
    key: GridMemoKey,
    custom: Rc<CustomEmojiSnapshot>,
    rows: Rc<Vec<GridRow>>,
    anchors: Rc<HashMap<String, usize>>,
}

impl EmojiPickerUiState {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(InputUiState::single_line);
        search.update(cx, |field, cx| {
            field.set_chromeless(true, cx);
            field.set_escape_to_layer(false);
        });
        let overdraw = px(CELL_MIN_HEIGHT + GRID_ROW_SPACING);
        let recents_store = EmojiPickerRecentsStore::shared(cx);
        let mut subscriptions = vec![
            cx.observe(&recents_store, |state, _, cx| {
                state.data_revision = state.data_revision.wrapping_add(1);
                cx.notify();
            }),
            cx.subscribe_in(&search, window, |state, field, event, window, cx| match event {
                InputUiEvent::Changed => {
                    state.data_revision = state.data_revision.wrapping_add(1);
                    state.prefetch_custom_search(cx);
                    cx.notify();
                }
                InputUiEvent::Cancelled => {
                    field.update(cx, |field, cx| field.clear(cx));
                    ScreenFocus::release_text_field(window, cx);
                }
                InputUiEvent::Submitted => {}
            }),
        ];
        if let Some(hooks) = EmojiPickerStickersHooks::shared() {
            let this = cx.entity().downgrade();
            subscriptions.push((hooks.observe)(
                Box::new(move |cx: &mut App| {
                    let _ = this.update(cx, |state, cx| {
                        state.data_revision = state.data_revision.wrapping_add(1);
                        state.prefetch_custom_search(cx);
                        cx.notify();
                    });
                }),
                cx,
            ));
        }
        Self {
            search,
            strip_selection: RECENTS_SECTION_ID.to_owned(),
            grid: ListState::new(0, ListAlignment::Top, overdraw),
            row_keys: Vec::new(),
            data_revision: 0,
            grid_memo: None,
            row_views: HashMap::new(),
            row_views_foreground: None,
            rows_built_this_draw: 0,
            rows_follow_up_scheduled: false,
            row_context: None,
            custom_active: false,
            search_requested_packs: HashSet::new(),
            search_requested_cursor: None,
            _subscriptions: subscriptions,
        }
    }

    /// search finds glyphs of every saved pack, not only pages and packs the grid has scrolled in
    /// while the query is not empty it pages in the rest of the saved library
    /// and resolves packs not loaded yet
    fn prefetch_custom_search(&mut self, cx: &mut Context<Self>) {
        if !self.custom_active {
            return;
        }
        let search_text = self.search.read(cx).text().to_owned();
        if trimming_whitespaces_and_newlines(&search_text).is_empty() {
            return;
        }
        let Some(hooks) = EmojiPickerStickersHooks::shared() else {
            return;
        };
        let user_id = CustomEmojiSnapshot::effective_user_id(cx);
        if user_id.is_empty() {
            return;
        }
        if (hooks.saved_packs_has_more)(&user_id, cx)
            && let Some(cursor) = (hooks.saved_packs_next_relative_id)(&user_id, cx)
            && self.search_requested_cursor.as_deref() != Some(cursor.as_str())
        {
            (hooks.load_saved_packs)(&user_id, Some(&cursor), false, cx);
            self.search_requested_cursor = Some(cursor);
        }
        let pending: Vec<String> = (hooks.saved_pack_summaries)(&user_id, cx)
            .into_iter()
            .map(|pack| pack.id)
            .filter(|id| !self.search_requested_packs.contains(id) && !(hooks.has_resolved_pack)(id, cx))
            .take(SEARCH_PREFETCH_PACKS_PER_STEP)
            .collect();
        tracing::warn!(
            target: "riseonly::x2154",
            "search prefetch: packs={} requested={} has_more={} cursor={:?} next={:?}",
            (hooks.saved_pack_summaries)(&user_id, cx).len(),
            self.search_requested_packs.len(),
            (hooks.saved_packs_has_more)(&user_id, cx),
            self.search_requested_cursor,
            pending,
        );
        for pack_id in pending {
            (hooks.load_pack_stickers)(&pack_id, cx);
            self.search_requested_packs.insert(pack_id);
        }
    }

    fn sync_rows(&mut self, keys: Vec<String>) {
        if keys == self.row_keys {
            return;
        }
        let old = &self.row_keys;
        let prefix = old.iter().zip(keys.iter()).take_while(|(a, b)| a == b).count();
        let max_suffix = old.len().min(keys.len()) - prefix;
        let suffix = old
            .iter()
            .rev()
            .zip(keys.iter().rev())
            .take(max_suffix)
            .take_while(|(a, b)| a == b)
            .count();
        self.grid.splice(prefix..old.len() - suffix, keys.len() - prefix - suffix);
        self.row_keys = keys;
    }
}

/// what the view reads from the sticker stores for one frame, through the hooks
struct CustomEmojiSnapshot {
    user_id: String,
    active: bool,
    packs: Vec<StickerPack>,
    visible_packs: Vec<StickerPack>,
    recents: Vec<StickerItem>,
    is_loading_packs: bool,
}

impl CustomEmojiSnapshot {
    fn effective_user_id(cx: &App) -> String {
        sai_auth_hooks().and_then(|hooks| (hooks.effective_user_id_for_wire)(cx)).unwrap_or_default()
    }

    fn read(picker: &EmojiPickerUi, cx: &App) -> Self {
        let user_id = Self::effective_user_id(cx);
        let hooks = EmojiPickerStickersHooks::shared();
        let active = picker.shows_custom_emoji
            && picker.on_pick_custom.is_some()
            && !user_id.is_empty()
            && hooks.is_some();
        let Some(hooks) = hooks.filter(|_| active) else {
            return Self {
                user_id,
                active,
                packs: Vec::new(),
                visible_packs: Vec::new(),
                recents: Vec::new(),
                is_loading_packs: hooks.is_some_and(|hooks| (hooks.is_loading)(cx)),
            };
        };
        let mut seen = HashSet::new();
        let packs: Vec<StickerPack> = (hooks.saved_pack_summaries)(&user_id, cx)
            .into_iter()
            .filter(|pack| seen.insert(pack.id.clone()))
            .collect();
        let visible_packs = packs
            .iter()
            .filter(|pack| {
                !(hooks.has_resolved_pack)(&pack.id, cx) || !(hooks.pack_stickers)(&pack.id, cx).is_empty()
            })
            .cloned()
            .collect();
        let recents = (hooks.recents)(&user_id, cx);
        Self {
            user_id,
            active,
            packs,
            visible_packs,
            recents,
            is_loading_packs: (hooks.is_loading)(cx),
        }
    }

    fn has_custom_emoji_section(&self, custom_emoji_only: bool) -> bool {
        if !self.visible_packs.is_empty() {
            return true;
        }
        if custom_emoji_only {
            return (self.packs.is_empty() && self.is_loading_packs) || !self.recents.is_empty();
        }
        false
    }
}

/// ids are scoped by section
/// else the same pack emoji in recents and in its pack collapses into one cell in the lazy grid
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct EmojiPickerUiCustomEmojiCellID {
    section_id: String,
    sticker_id: String,
}

#[derive(Clone)]
struct EmojiPickerUiCustomEmojiCell {
    id: EmojiPickerUiCustomEmojiCellID,
    item: StickerItem,
}

impl EmojiPickerUiCustomEmojiCell {
    fn new(section_id: &str, item: StickerItem) -> Self {
        Self {
            id: EmojiPickerUiCustomEmojiCellID { section_id: section_id.to_owned(), sticker_id: item.id.clone() },
            item,
        }
    }

    fn key(&self) -> String {
        format!("{}/{}", self.id.section_id, self.id.sticker_id)
    }
}

#[derive(Clone)]
enum GridCell {
    Emoji(EmojiPickerCatalogCell),
    Custom(EmojiPickerUiCustomEmojiCell),
    Pinned(EmojiPickerPinnedItem),
    Skeleton(usize),
}

impl GridCell {
    fn key(&self) -> String {
        match self {
            GridCell::Emoji(cell) => cell.id.clone(),
            GridCell::Custom(cell) => cell.key(),
            GridCell::Pinned(item) => format!("pinned/{}", item.id),
            GridCell::Skeleton(index) => format!("skeleton/{index}"),
        }
    }
}

/// one row of the lazy grid, a section header spans the width and cells come eight to a row
#[derive(Clone)]
enum GridRow {
    Anchor,
    Header { title: String, extra_top: f32 },
    RecentsHeader,
    PackHeader { title: String, pack_id: String, resolved: bool, index: usize, count: usize },
    Cells(Vec<GridCell>),
    BottomPadding,
}

impl GridRow {
    fn key(&self) -> String {
        match self {
            GridRow::Anchor => "anchor".to_owned(),
            GridRow::Header { title, extra_top } => format!("header/{title}/{extra_top}"),
            GridRow::RecentsHeader => "recents-header".to_owned(),
            GridRow::PackHeader { pack_id, resolved, index, count, .. } => {
                format!("pack-header/{pack_id}/{resolved}/{index}/{count}")
            }
            GridRow::Cells(cells) => {
                let keys: Vec<String> = cells.iter().map(GridCell::key).collect();
                format!("cells/{}", keys.join("|"))
            }
            GridRow::BottomPadding => "bottom".to_owned(),
        }
    }
}

#[derive(Default)]
struct GridRows {
    rows: Vec<GridRow>,
    anchors: HashMap<String, usize>,
}

impl GridRows {
    fn section(&mut self, header: GridRow, anchor: Option<String>, cells: Vec<GridCell>) {
        if let Some(anchor) = anchor {
            self.anchors.entry(anchor).or_insert(self.rows.len());
        }
        self.rows.push(header);
        for chunk in cells.chunks(COLUMN_COUNT) {
            self.rows.push(GridRow::Cells(chunk.to_vec()));
        }
    }

    fn cells(&mut self, cells: Vec<GridCell>) {
        for chunk in cells.chunks(COLUMN_COUNT) {
            self.rows.push(GridRow::Cells(chunk.to_vec()));
        }
    }
}

/// what the grid rows call back into, shared by every row the lazy list renders
struct GridContext {
    picker_id: ElementId,
    foreground: Hsla,
    on_pick: PickHandler,
    on_pick_custom: Option<CustomPickHandler>,
    pinned_on_pick: Option<PinnedPickHandler>,
    recents_store: Entity<EmojiPickerRecentsStore>,
    stickers_user_id: String,
    has_custom_recents: bool,
}

/// latest grid context shared with the cached rows so a click runs the handlers of the latest draw
type SharedGridContext = Rc<RefCell<Rc<GridContext>>>;

/// one row of cells as a cached view
/// a reveal spring or sheet drag redraws ancestors every frame so visible rows would render each time
/// cached, a row replays its last frame until a hover inside it or a rebuild of the rows
struct GridRowView {
    cells: Vec<GridCell>,
    top_spacing: f32,
    context: SharedGridContext,
}

impl Render for GridRowView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().w_full().px(px(8.)).pt(px(self.top_spacing)).child(cells_row(&self.cells, &self.context))
    }
}

impl GridContext {
    fn child_id(&self, name: impl Into<SharedString>) -> ElementId {
        ElementId::NamedChild(Arc::new(self.picker_id.clone()), name.into())
    }
}

impl RenderOnce for EmojiPickerUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.child_id("state"), cx, EmojiPickerUiState::new);
        let recents_store = EmojiPickerRecentsStore::shared(cx);
        let (appearance, theme_text_primary, theme_primary, theme_bg_100, theme_bg_200) = {
            let store = ThemeStore::shared(cx);
            let store = store.read(cx);
            let theme = store.current_theme();
            (
                store.resolved_appearance(),
                theme.text.primary,
                theme.primary._100,
                theme.bg._100,
                theme.bg._200,
            )
        };
        let uses_dark_chrome = self.chrome && appearance != RiseThemeAppearance::Light;
        let picker_foreground = if uses_dark_chrome { white() } else { theme_text_primary };
        let picker_control_fill = if uses_dark_chrome { white().opacity(0.07) } else { theme_bg_200 };
        let colors = PickerColors { uses_dark_chrome, foreground: picker_foreground, primary: theme_primary };

        let search_text = state.read(cx).search.read(cx).text().to_owned();
        let query = trimming_whitespaces_and_newlines(&search_text).to_lowercase();
        let (custom, rows, anchors) = self.memoized_grid(&state, &query, &recents_store, cx);

        let core = if self.shows_custom_emoji_empty_state(&custom, cx) {
            self.custom_emoji_empty_state(picker_foreground)
        } else {
            let strip = self.category_strip(&state, &custom, &recents_store, anchors, colors, cx);
            let search_bar = self.search_bar(&state, &search_text, picker_control_fill, colors, cx);
            let grid = self.emoji_grid(&state, rows, &custom, &recents_store, picker_foreground, cx);
            vstack(0.).size_full().child(strip).child(search_bar).child(grid).into_any_element()
        };

        let custom_active = custom.active;
        let user_id = custom.user_id.clone();
        let custom_emoji_only = self.custom_emoji_only;
        let pinned_id = self.pinned_pack.as_ref().map(|pack| pack.id.clone());
        let appear_state = state.clone();
        let body = if self.chrome {
            let corner_radius = HoldContextMenuMetrics::PICKER_CORNER_RADIUS;
            let background = if uses_dark_chrome {
                div()
                    .absolute()
                    .inset_0()
                    .bg(theme_bg_100)
                    .child(HoldContextMenuGlass::new(self.child_id("glass")).corner_radius(corner_radius))
                    .into_any_element()
            } else {
                div().absolute().inset_0().bg(theme_bg_100).into_any_element()
            };
            div()
                .relative()
                .size_full()
                .shadow(vec![BoxShadow {
                    color: black().opacity(0.28),
                    offset: point(px(0.), px(12.)),
                    blur_radius: px(22.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .child(
                    div()
                        .relative()
                        .size_full()
                        .clip_shape(Shape::continuous(corner_radius))
                        .child(background)
                        .child(core),
                )
                .child(centred_stroke(
                    picker_foreground.opacity(HoldContextMenuMetrics::PANEL_STROKE_OPACITY as f32),
                    1.,
                    Shape::continuous(corner_radius),
                ))
                .into_any_element()
        } else {
            core
        };

        div()
            .id(self.id.clone())
            .size_full()
            .child(body)
            .on_appear(self.child_id("appear"), move |_window, cx| {
                appear_state.update(cx, |state, cx| {
                    if !custom_emoji_only || state.strip_selection != RECENTS_SECTION_ID {
                        return;
                    }
                    if custom_active {
                        state.strip_selection = CUSTOM_SECTION_ID.to_owned();
                        cx.notify();
                    } else if let Some(pinned_id) = pinned_id {
                        state.strip_selection = pinned_id;
                        cx.notify();
                    }
                });
                load_custom_emoji_if_needed(custom_active, &user_id, cx);
            })
    }
}

struct StripIcon {
    id: String,
    symbol: String,
    label: String,
    target: String,
}

impl StripIcon {
    fn new(id: &str, symbol: &str, label: String, target: &str) -> Self {
        Self { id: id.to_owned(), symbol: symbol.to_owned(), label, target: target.to_owned() }
    }
}

#[derive(Clone, Copy)]
struct PickerColors {
    uses_dark_chrome: bool,
    foreground: Hsla,
    primary: Hsla,
}

fn load_custom_emoji_if_needed(custom_emoji_active: bool, user_id: &str, cx: &mut App) {
    if !custom_emoji_active {
        return;
    }
    let Some(hooks) = EmojiPickerStickersHooks::shared() else {
        return;
    };
    (hooks.load_recents)(user_id, cx);
    let force = !(hooks.saved_packs_resolved)(user_id, cx);
    (hooks.load_saved_packs)(user_id, None, force, cx);
}

fn load_more_custom_packs_if_needed(index: usize, count: usize, user_id: &str, cx: &mut App) {
    let Some(hooks) = EmojiPickerStickersHooks::shared() else {
        return;
    };
    if count - index > 3 || !(hooks.saved_packs_has_more)(user_id, cx) {
        return;
    }
    let Some(cursor) = (hooks.saved_packs_next_relative_id)(user_id, cx) else {
        return;
    };
    (hooks.load_saved_packs)(user_id, Some(&cursor), false, cx);
}

fn symbol_icon(symbol: &str, side: f32, color: Hsla) -> AnyElement {
    let is_filled = symbol.ends_with(".fill");
    match IconUi::sized(symbol, px(side), color, is_filled) {
        Some(icon) => icon.into_any_element(),
        None => div().size(px(side)).into_any_element(),
    }
}

impl EmojiPickerUi {

    fn category_strip(
        &self,
        state: &Entity<EmojiPickerUiState>,
        custom: &CustomEmojiSnapshot,
        recents_store: &Entity<EmojiPickerRecentsStore>,
        anchors: Rc<HashMap<String, usize>>,
        colors: PickerColors,
        cx: &App,
    ) -> AnyElement {
        let mut icons: Vec<StripIcon> = Vec::new();
        if !self.custom_emoji_only {
            let target = if recents_store.read(cx).recents().is_empty() && custom.recents.is_empty() {
                TOP_SECTION_ID
            } else {
                RECENTS_SECTION_ID
            };
            icons.push(StripIcon::new(RECENTS_SECTION_ID, "clock", tr("emoji_recents"), target));
        }
        if custom.active && custom.has_custom_emoji_section(self.custom_emoji_only) {
            icons.push(StripIcon::new(
                CUSTOM_SECTION_ID,
                "face.smiling",
                tr("chat_reactions_custom_emoji_section"),
                CUSTOM_SECTION_ID,
            ));
        }
        if !self.custom_emoji_only {
            for category in EmojiPickerCatalog::categories() {
                icons.push(StripIcon::new(&category.id, &category.symbol, category.title(), &category.id));
            }
        }
        if let Some(pinned_pack) = &self.pinned_pack {
            icons.push(StripIcon::new(
                &pinned_pack.id,
                &pinned_pack.symbol,
                pinned_pack.title.clone(),
                &pinned_pack.id,
            ));
        }
        let selection = state.read(cx).strip_selection.clone();
        let mut strip = hstack(4.).px(px(10.));
        for icon in icons {
            strip = strip.child(self.strip_icon(state, icon, &selection, &anchors, colors));
        }
        div()
            .id(self.child_id("strip"))
            .w_full()
            .flex_none()
            .overflow_x_scroll()
            .pt(px(10.))
            .pb(px(4.))
            .child(strip)
            .into_any_element()
    }

    fn strip_icon(
        &self,
        state: &Entity<EmojiPickerUiState>,
        icon: StripIcon,
        selection: &str,
        anchors: &Rc<HashMap<String, usize>>,
        colors: PickerColors,
    ) -> AnyElement {
        let is_active = selection == icon.id;
        let icon_color = if !colors.uses_dark_chrome && is_active {
            colors.primary
        } else {
            colors.foreground.opacity(if is_active { 0.95 } else { 0.45 })
        };
        let fill = if !colors.uses_dark_chrome && is_active {
            colors.primary.opacity(0.14)
        } else {
            colors.foreground.opacity(if is_active { 0.12 } else { 0. })
        };
        let state = state.clone();
        let target_row = anchors.get(&icon.target).copied();
        let label: SharedString = icon.label.into();
        let selection = icon.id.clone();
        let action: ButtonAction = Rc::new(move |window: &mut Window, cx: &mut App| {
            let _ = selection_changed();
            state.update(cx, |state, cx| {
                state.strip_selection = selection.clone();
                state.search.update(cx, |field, cx| field.clear(cx));
                if let Some(row) = target_row {
                    state.grid.scroll_to(ListOffset { item_ix: row, offset_in_item: Pixels::ZERO });
                }
                cx.notify();
            });
            let focus = state.read(cx).search.read(cx).focus_handle(cx);
            if focus.is_focused(window) {
                ScreenFocus::release_text_field(window, cx);
            }
        });
        let button = div()
            .id(self.child_id(format!("strip/{}", icon.id)))
            .w(px(34.))
            .h(px(30.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .capsule()
            .bg(fill)
            .child(symbol_icon(&icon.symbol, 16., icon_color));
        button_chrome(button, false, Some(action))
            .tooltip_label(label)
            .into_any_element()
    }


    fn search_bar(
        &self,
        state: &Entity<EmojiPickerUiState>,
        search_text: &str,
        control_fill: Hsla,
        colors: PickerColors,
        cx: &mut App,
    ) -> AnyElement {
        let foreground = colors.foreground;
        let tint = if colors.uses_dark_chrome { foreground.opacity(0.8) } else { colors.primary };
        let placeholder: SharedString = tr("search_text").into();
        let field = state.read(cx).search.clone();
        field.update(cx, |field, cx| {
            if field.placeholder() != &placeholder {
                field.set_placeholder(placeholder, cx);
            }
            if field.text_color() != Some(foreground) {
                field.set_text_color(Some(foreground), cx);
            }
            let placeholder_color = foreground.opacity(0.40);
            if field.placeholder_color() != Some(placeholder_color) {
                field.set_placeholder_color(Some(placeholder_color), cx);
            }
            if field.caret_color() != Some(tint) {
                field.set_caret_color(Some(tint), cx);
            }
            if field.font_size() != Some(px(15.)) {
                field.set_font_size(Some(px(15.)), cx);
            }
        });

        let trailing = if search_text.is_empty() {
            self.quick_filter_chips(&field, foreground)
        } else {
            let clear_field = field.clone();
            plain(self.child_id("search/clear"))
                .on_press(move |_window, cx| clear_field.update(cx, |field, cx| field.clear(cx)))
                .child(symbol_icon("xmark.circle.fill", 15., foreground.opacity(0.40)))
                .into_any_element()
        };

        div()
            .flex_none()
            .w_full()
            .px(px(10.))
            .pt(px(4.))
            .pb(px(8.))
            .child(
                hstack(7.)
                    .px(px(11.))
                    .py(px(7.))
                    .capsule()
                    .bg(control_fill)
                    .child(symbol_icon("magnifyingglass", 14., foreground.opacity(0.40)))
                    .child(div().flex_1().min_w_0().child(field))
                    .child(trailing),
            )
            .into_any_element()
    }

    fn quick_filter_chips(&self, field: &Entity<InputUiState>, foreground: Hsla) -> AnyElement {
        let mut chips = hstack(10.).pr(px(2.));
        for filter in EmojiPickerCatalog::quick_filters() {
            let field = field.clone();
            let query = filter.query.clone();
            chips = chips.child(
                plain(self.child_id(format!("search/filter/{}", filter.id)))
                    .on_press(move |_window, cx| {
                        let _ = selection_changed();
                        let query = query.clone();
                        field.update(cx, |field, cx| field.set_text(query, cx));
                    })
                    .child(symbol_icon(&filter.symbol, 14., foreground.opacity(0.40))),
            );
        }
        chips.into_any_element()
    }


    fn memoized_grid(
        &self,
        state: &Entity<EmojiPickerUiState>,
        query: &str,
        recents_store: &Entity<EmojiPickerRecentsStore>,
        cx: &mut App,
    ) -> (Rc<CustomEmojiSnapshot>, Rc<Vec<GridRow>>, Rc<HashMap<String, usize>>) {
        let key = GridMemoKey {
            revision: state.read(cx).data_revision,
            query: query.to_owned(),
            user_id: CustomEmojiSnapshot::effective_user_id(cx),
            locale: LanguageStore::current_locale(),
            shows_custom_emoji: self.shows_custom_emoji,
            custom_emoji_only: self.custom_emoji_only,
            picks_custom: self.on_pick_custom.is_some(),
            pinned_pack_id: self.pinned_pack.as_ref().map(|pack| pack.id.clone()),
        };
        if let Some(memo) = state.read(cx).grid_memo.as_ref().filter(|memo| memo.key == key) {
            return (memo.custom.clone(), memo.rows.clone(), memo.anchors.clone());
        }
        let custom = Rc::new(CustomEmojiSnapshot::read(self, cx));
        let grid_rows = self.grid_rows(query, &custom, recents_store, cx);
        let keys = grid_rows.rows.iter().map(GridRow::key).collect();
        let rows = Rc::new(grid_rows.rows);
        let anchors = Rc::new(grid_rows.anchors);
        state.update(cx, |state, _| {
            state.custom_active = custom.active;
            state.sync_rows(keys);
            let live_keys: HashSet<&str> = state.row_keys.iter().map(String::as_str).collect();
            state.row_views.retain(|key, _| live_keys.contains(key.as_str()));
            state.grid_memo =
                Some(GridMemo { key, custom: custom.clone(), rows: rows.clone(), anchors: anchors.clone() });
        });
        (custom, rows, anchors)
    }

    fn grid_rows(
        &self,
        query: &str,
        custom: &CustomEmojiSnapshot,
        recents_store: &Entity<EmojiPickerRecentsStore>,
        cx: &App,
    ) -> GridRows {
        let mut rows = GridRows::default();
        if self.custom_emoji_only {
            self.custom_only_grid_content(&mut rows, query, custom, cx);
        } else if query.is_empty() {
            rows.section(
                GridRow::Anchor,
                Some(TOP_SECTION_ID.to_owned()),
                EmojiPickerCatalog::reaction_cells().iter().cloned().map(GridCell::Emoji).collect(),
            );

            let recents = recents_store.read(cx);
            let plain_recents = recents.recents();
            if !plain_recents.is_empty() || !custom.recents.is_empty() {
                let slots = EmojiPickerRecentsStore::ordered(
                    recents.pick_order(),
                    &custom.recents,
                    |item: &StickerItem| item.id.clone(),
                    &EmojiPickerCatalog::make_cells(&plain_recents, RECENTS_SECTION_ID),
                    |item: &StickerItem| EmojiRecentSlot::Custom(item.clone()),
                    |cell: &EmojiPickerCatalogCell| EmojiRecentSlot::Unicode(cell.clone()),
                );
                let cells = slots
                    .into_iter()
                    .map(|slot| match slot {
                        EmojiRecentSlot::Custom(item) => {
                            GridCell::Custom(EmojiPickerUiCustomEmojiCell::new(RECENTS_SECTION_ID, item))
                        }
                        EmojiRecentSlot::Unicode(cell) => GridCell::Emoji(cell),
                    })
                    .collect();
                rows.section(GridRow::RecentsHeader, Some(RECENTS_SECTION_ID.to_owned()), cells);
            }

            if custom.active {
                self.custom_emoji_sections(&mut rows, false, custom, cx);
            }

            for category in EmojiPickerCatalog::categories() {
                rows.section(
                    GridRow::Header { title: category.title(), extra_top: 0. },
                    Some(category.id.clone()),
                    category.cells.iter().cloned().map(GridCell::Emoji).collect(),
                );
            }

            self.pinned_pack_section(&mut rows);
        } else {
            let results = EmojiPickerCatalog::search(query);
            if results.is_empty() {
                rows.section(GridRow::Header { title: tr("emoji_no_results"), extra_top: 18. }, None, Vec::new());
            } else {
                rows.cells(
                    EmojiPickerCatalog::make_cells(&results, SEARCH_SECTION_ID)
                        .into_iter()
                        .map(GridCell::Emoji)
                        .collect(),
                );
            }
        }
        rows.rows.push(GridRow::BottomPadding);
        rows
    }

    fn custom_only_grid_content(&self, rows: &mut GridRows, query: &str, custom: &CustomEmojiSnapshot, cx: &App) {
        if query.is_empty() {
            if custom.active {
                if custom.packs.is_empty() && custom.is_loading_packs {
                    rows.section(
                        GridRow::Anchor,
                        Some(CUSTOM_SECTION_ID.to_owned()),
                        (0..SKELETON_CELL_COUNT).map(GridCell::Skeleton).collect(),
                    );
                } else {
                    self.custom_emoji_sections(rows, true, custom, cx);
                }
            }
            self.pinned_pack_section(rows);
        } else {
            let results = if custom.active { self.custom_emoji_search_results(query, custom, cx) } else { Vec::new() };
            if results.is_empty() {
                rows.section(GridRow::Header { title: tr("emoji_no_results"), extra_top: 18. }, None, Vec::new());
            } else {
                rows.cells(
                    results
                        .into_iter()
                        .map(|item| GridCell::Custom(EmojiPickerUiCustomEmojiCell::new(SEARCH_SECTION_ID, item)))
                        .collect(),
                );
            }
        }
    }

    fn pinned_pack_section(&self, rows: &mut GridRows) {
        let Some(pinned_pack) = &self.pinned_pack else {
            return;
        };
        if pinned_pack.items.is_empty() {
            return;
        }
        rows.section(
            GridRow::Header { title: pinned_pack.title.clone(), extra_top: 0. },
            Some(pinned_pack.id.clone()),
            pinned_pack.items.iter().cloned().map(GridCell::Pinned).collect(),
        );
    }

    fn custom_emoji_search_results(&self, query: &str, custom: &CustomEmojiSnapshot, cx: &App) -> Vec<StickerItem> {
        let aliases: HashSet<String> = EmojiPickerCatalog::search(query).into_iter().collect();
        if aliases.is_empty() {
            return Vec::new();
        }
        let Some(hooks) = EmojiPickerStickersHooks::shared() else {
            return Vec::new();
        };

        let mut out: Vec<StickerItem> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut scanned = 0;

        for pack in &custom.packs {
            for item in (hooks.pack_stickers)(&pack.id, cx) {
                scanned += 1;
                if scanned > CUSTOM_SEARCH_SCAN_LIMIT {
                    return out;
                }
                if !item.associated_emojis.iter().any(|emoji| aliases.contains(emoji)) || !seen.insert(item.id.clone()) {
                    continue;
                }
                out.push(item);
                if out.len() >= CUSTOM_SEARCH_RESULT_LIMIT {
                    return out;
                }
            }
        }

        tracing::warn!(
            target: "riseonly::x2154",
            "search results: query={query} aliases={} packs={} resolved={} scanned={scanned} out={}",
            aliases.len(),
            custom.packs.len(),
            custom.packs.iter().filter(|pack| (hooks.has_resolved_pack)(&pack.id, cx)).count(),
            out.len(),
        );
        out
    }

    fn custom_emoji_sections(&self, rows: &mut GridRows, including_recents: bool, custom: &CustomEmojiSnapshot, cx: &App) {
        let custom_recents: &[StickerItem] = if including_recents { &custom.recents } else { &[] };
        if !custom_recents.is_empty() {
            rows.section(
                GridRow::Header { title: tr("emoji_recents"), extra_top: 0. },
                Some(CUSTOM_SECTION_ID.to_owned()),
                custom_recents
                    .iter()
                    .cloned()
                    .map(|item| GridCell::Custom(EmojiPickerUiCustomEmojiCell::new(RECENTS_SECTION_ID, item)))
                    .collect(),
            );
        }
        let Some(hooks) = EmojiPickerStickersHooks::shared() else {
            return;
        };
        let packs = &custom.visible_packs;
        for (index, pack) in packs.iter().enumerate() {
            let section_id = format!("custom_pack_{}", pack.id);
            let header_id = if custom_recents.is_empty() && index == 0 {
                CUSTOM_SECTION_ID.to_owned()
            } else {
                section_id.clone()
            };
            let resolved = (hooks.has_resolved_pack)(&pack.id, cx);
            let cells = if resolved {
                (hooks.pack_stickers)(&pack.id, cx)
                    .into_iter()
                    .map(|item| GridCell::Custom(EmojiPickerUiCustomEmojiCell::new(&section_id, item)))
                    .collect()
            } else {
                Vec::new()
            };
            rows.section(
                GridRow::PackHeader {
                    title: pack.title.clone(),
                    pack_id: pack.id.clone(),
                    resolved,
                    index,
                    count: packs.len(),
                },
                Some(header_id),
                cells,
            );
        }
    }

    fn emoji_grid(
        &self,
        state: &Entity<EmojiPickerUiState>,
        rows: Rc<Vec<GridRow>>,
        custom: &CustomEmojiSnapshot,
        recents_store: &Entity<EmojiPickerRecentsStore>,
        foreground: Hsla,
        cx: &mut App,
    ) -> AnyElement {
        let grid = state.read(cx).grid.clone();
        let context = Rc::new(GridContext {
            picker_id: self.id.clone(),
            foreground,
            on_pick: self.on_pick.clone(),
            on_pick_custom: self.on_pick_custom.clone(),
            pinned_on_pick: self.pinned_pack.as_ref().map(|pack| pack.on_pick.clone()),
            recents_store: recents_store.clone(),
            stickers_user_id: custom.user_id.clone(),
            has_custom_recents: !custom.recents.is_empty(),
        });
        let rows_settling = self.rows_settling;
        let live = state.update(cx, |state, _| {
            if rows_settling || state.row_views_foreground != Some(foreground) {
                state.row_views.clear();
                state.row_views_foreground = Some(foreground);
            }
            state.rows_built_this_draw = 0;
            match &state.row_context {
                Some(live) => {
                    *live.borrow_mut() = context;
                    live.clone()
                }
                None => {
                    let live: SharedGridContext = Rc::new(RefCell::new(context));
                    state.row_context = Some(live.clone());
                    live
                }
            }
        });
        let row_state = state.clone();
        let row_count = rows.len();
        list(grid, move |index, window, cx| {
            let Some(row) = rows.get(index) else {
                return div().into_any_element();
            };
            let top_spacing = if index == 0 || index + 1 == row_count { 0. } else { GRID_ROW_SPACING };
            if let GridRow::Cells(cells) = row
                && !rows_settling
                && let Some(view) = row_state.update(cx, |state, cx| {
                    let key = state.row_keys.get(index)?;
                    if let Some(view) = state.row_views.get(key)
                        && view.read(cx).top_spacing == top_spacing
                    {
                        return Some(view.clone());
                    }
                    if state.rows_built_this_draw >= NEW_ROWS_PER_DRAW {
                        return None;
                    }
                    state.rows_built_this_draw += 1;
                    let view = cx.new(|_| GridRowView { cells: cells.clone(), top_spacing, context: live.clone() });
                    state.row_views.insert(key.clone(), view.clone());
                    Some(view)
                })
            {
                return AnyView::from(view)
                    .cached(StyleRefinement::default().w_full().h(px(CELL_MIN_HEIGHT + top_spacing)))
                    .replay_when_moved()
                    .into_any_element();
            }
            if matches!(row, GridRow::Cells(_)) && !rows_settling {
                let scheduled =
                    row_state.update(cx, |state, _| std::mem::replace(&mut state.rows_follow_up_scheduled, true));
                if !scheduled {
                    let follow_up = row_state.clone();
                    window.on_next_frame(move |_, cx| {
                        follow_up.update(cx, |state, cx| {
                            state.rows_follow_up_scheduled = false;
                            cx.notify();
                        });
                    });
                }
                return div().w_full().h(px(CELL_MIN_HEIGHT + top_spacing)).into_any_element();
            }
            div()
                .w_full()
                .px(px(8.))
                .pt(px(top_spacing))
                .child(render_row(row, &live))
                .into_any_element()
        })
        .flex_1()
        .w_full()
        .into_any_element()
    }

    fn shows_custom_emoji_empty_state(&self, custom: &CustomEmojiSnapshot, cx: &App) -> bool {
        if !self.custom_emoji_only || !custom.active || self.pinned_pack.is_some() {
            return false;
        }
        let Some(hooks) = EmojiPickerStickersHooks::shared() else {
            return false;
        };
        (hooks.saved_packs_resolved)(&custom.user_id, cx)
            && !custom.is_loading_packs
            && custom.visible_packs.is_empty()
            && custom.recents.is_empty()
    }

    fn custom_emoji_empty_state(&self, foreground: Hsla) -> AnyElement {
        let mut content = vstack(10.)
            .child(symbol_icon("face.smiling", 40., foreground.opacity(0.4)))
            .child(
                MainText::new(tr("emoji_pack_saved_empty"))
                    .px(16.)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(foreground)
                    .number_of_lines(2),
            )
            .child(
                MainText::new(tr("emoji_picker_custom_empty_hint"))
                    .px(14.)
                    .tac(TextAlignT::Center)
                    .color(foreground.opacity(0.55))
                    .number_of_lines(4),
            );
        if let Some(on_create) = self.on_create_custom_emoji_pack.clone() {
            content = content.child(
                div().pt(px(6.)).child(
                    ButtonUi::new(self.child_id("create-pack"))
                        .fit_content(true)
                        .padding_horizontal(20.)
                        .on_press(move |window, cx| on_create(window, cx))
                        .child(
                            MainText::new(tr("emoji_pack_create"))
                                .px(15.)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(white()),
                        ),
                ),
            );
        }
        content.px(px(24.)).frame_max(Alignment::Center).into_any_element()
    }
}

fn render_row(row: &GridRow, live: &SharedGridContext) -> AnyElement {
    let context = &live.borrow().clone();
    match row {
        GridRow::Anchor => div().w_full().h(px(1.)).into_any_element(),
        GridRow::Header { title, extra_top } => {
            div().pt(px(*extra_top)).child(section_header(title, context.foreground)).into_any_element()
        }
        GridRow::RecentsHeader => recents_header(context),
        GridRow::PackHeader { title, pack_id, resolved, index, count } => {
            let pack_id = pack_id.clone();
            let resolved = *resolved;
            let index = *index;
            let count = *count;
            let user_id = context.stickers_user_id.clone();
            div()
                .w_full()
                .child(section_header(title, context.foreground))
                .on_appear(context.child_id(format!("pack-header/{pack_id}/{resolved}")), move |_window, cx| {
                    if !resolved && let Some(hooks) = EmojiPickerStickersHooks::shared() {
                        (hooks.load_pack_stickers)(&pack_id, cx);
                    }
                    load_more_custom_packs_if_needed(index, count, &user_id, cx);
                })
                .into_any_element()
        }
        GridRow::Cells(cells) => cells_row(cells, live),
        GridRow::BottomPadding => div().w_full().h(px(12.)).into_any_element(),
    }
}

fn cells_row(cells: &[GridCell], live: &SharedGridContext) -> AnyElement {
    let mut row = hstack(0.).w_full();
    for cell in cells {
        row = row.child(render_cell(cell, live));
    }
    for _ in cells.len()..COLUMN_COUNT {
        row = row.child(div().flex_1());
    }
    row.into_any_element()
}

fn render_cell(cell: &GridCell, live: &SharedGridContext) -> AnyElement {
    let context = &live.borrow().clone();
    let (id, label, action): (String, AnyElement, Option<ButtonAction>) = match cell {
        GridCell::Emoji(cell) => {
            let emoji = cell.emoji.clone();
            let live = live.clone();
            let label = div()
                .swiftui_font(Font::system_size(30.))
                .child(SharedString::from(cell.emoji.clone()))
                .into_any_element();
            let action: ButtonAction = Rc::new(move |window: &mut Window, cx: &mut App| {
                let _ = impact(ImpactStyle::Medium);
                let emoji = emoji.clone();
                let context = live.borrow().clone();
                context.recents_store.update(cx, |store, cx| store.register(&emoji, cx));
                (context.on_pick)(emoji, window, cx);
            });
            (format!("cell/{}", cell.id), label, Some(action))
        }
        GridCell::Custom(cell) => {
            let item = &cell.item;
            let label = ChatInputStickerCell::new(item.clone(), CUSTOM_CELL_SIDE)
                .is_playing(true)
                .allows_hold_preview(false)
                .into_any_element();
            let picked = item.clone();
            let live = live.clone();
            let action: ButtonAction = Rc::new(move |window: &mut Window, cx: &mut App| {
                let context = live.borrow().clone();
                pick_custom(&context, picked.clone(), window, cx)
            });
            (format!("custom/{}", cell.key()), label, Some(action))
        }
        GridCell::Pinned(item) => {
            let label = LottieView::new(item.lottie_name.clone()).size(CUSTOM_CELL_SIDE - 4.).into_any_element();
            let action = context.pinned_on_pick.is_some().then(|| {
                let picked = item.clone();
                let live = live.clone();
                let action: ButtonAction = Rc::new(move |window: &mut Window, cx: &mut App| {
                    let Some(on_pick) = live.borrow().pinned_on_pick.clone() else {
                        return;
                    };
                    let _ = impact(ImpactStyle::Medium);
                    on_pick(picked.clone(), window, cx);
                });
                action
            });
            (format!("pinned/{}", item.id), label, action)
        }
        GridCell::Skeleton(index) => {
            let label = SkeletonBlock::new(CUSTOM_CELL_SIDE - 4., CUSTOM_CELL_SIDE - 4., 9.).into_any_element();
            (format!("skeleton/{index}"), label, None)
        }
    };
    let hover_fill = context.foreground.opacity(CELL_HOVER_FILL_OPACITY);
    let has_action = action.is_some();
    let cell = div()
        .id(context.child_id(id))
        .flex_1()
        .min_h(px(CELL_MIN_HEIGHT))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(CELL_HOVER_CORNER_RADIUS))
        .child(label);
    if !has_action {
        return cell.into_any_element();
    }
    button_chrome_hover(cell, false, action, move |style: StyleRefinement| {
        style.opacity(HOVER_OPACITY).bg(hover_fill)
    })
    .into_any_element()
}

fn pick_custom(context: &GridContext, item: StickerItem, window: &mut Window, cx: &mut App) {
    let Some(on_pick_custom) = context.on_pick_custom.clone() else {
        return;
    };
    let _ = impact(ImpactStyle::Medium);
    let sticker_id = item.id.clone();
    context.recents_store.update(cx, |store, cx| store.register_custom(&sticker_id, cx));
    let user_id = &context.stickers_user_id;
    if !user_id.is_empty()
        && let Some(hooks) = EmojiPickerStickersHooks::shared()
    {
        (hooks.write_recent)(user_id, &sticker_id, cx);
    }
    on_pick_custom(item, window, cx);
}

fn section_header(title: &str, foreground: Hsla) -> AnyElement {
    div()
        .w_full()
        .flex()
        .justify_center()
        .pt(px(10.))
        .pb(px(2.))
        .swiftui_font(Font::system_weight(12.5, FontWeight::SEMIBOLD))
        .text_color(foreground.opacity(0.48))
        .child(SharedString::from(title.to_uppercase()))
        .into_any_element()
}

fn recents_header(context: &Rc<GridContext>) -> AnyElement {
    let foreground = context.foreground;
    let clear_context = context.clone();
    let clear = plain(context.child_id("recents/clear"))
        .on_press(move |_window, cx| {
            let _ = selection_changed();
            if clear_context.has_custom_recents
                && let Some(hooks) = EmojiPickerStickersHooks::shared()
            {
                (hooks.clear_recents)(&clear_context.stickers_user_id, cx);
            }
            clear_context.recents_store.update(cx, |store, cx| store.clear(cx));
        })
        .min_h(px(24.))
        .flex()
        .items_center()
        .swiftui_font(Font::system_weight(12., FontWeight::MEDIUM))
        .text_color(foreground.opacity(0.48))
        .child(SharedString::from(tr("picker_recents_clear")));
    div()
        .relative()
        .w_full()
        .min_h(px(RECENTS_HEADER_MIN_HEIGHT))
        .flex()
        .flex_col()
        .justify_center()
        .child(section_header(&tr("emoji_recents"), foreground))
        .child(
            hstack(None::<f32>)
                .absolute()
                .top_0()
                .right_0()
                .pt(px(8.))
                .pr(px(4.))
                .child(clear),
        )
        .into_any_element()
}
