use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, AnyView, AnyWindowHandle, App, AppContext, Context, ElementId, Entity, FontWeight, ListAlignment,
    ListState, Render, SharedString, StyleRefinement, Subscription, TextAlign, Window, black, linear_color_stop,
    linear_gradient, list, white,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::haptics::{NotificationKind, notification, selection_changed};
use rise_theme::{AppTheme, Appearance, RiseThemeAppearance, ThemeStore};
use rise_ui::error_text_ui::system_red;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_screen::{
    BottomSheetNavigationState, BottomSheetScreenItem, bottom_sheet_navigation,
};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonShimmerExt};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_pages_ui::BottomSheetPagesUi;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::modules::story::components::story_thumbnail_ui::StoryThumbnailUi;
use crate::modules::story::stores::story::story_actions::story_actions::StoryActionsStore;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::StoryProfileScope;
use crate::modules::story::stores::story::story_actions::story_wire_types::{StoryDTO, StoryDTOExt};
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;

pub(super) type HighlightSheetAction = Rc<dyn Fn(&mut Window, &mut App)>;

pub(super) fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

/// sheet callbacks from a store have no window, closing waits for the end of the effect cycle
pub(super) fn with_window(
    window: AnyWindowHandle,
    action: impl FnOnce(&mut Window, &mut App) + 'static,
    cx: &mut App,
) {
    cx.defer(move |cx| {
        let _ = window.update(cx, move |_, window, cx| action(window, cx));
    });
}

pub(super) fn resolved_appearance(cx: &mut App) -> Appearance {
    match ThemeStore::shared(cx).read(cx).resolved_appearance() {
        RiseThemeAppearance::Light => Appearance::Light,
        RiseThemeAppearance::Dark => Appearance::Dark,
    }
}

/// creating a highlight, pick stories then name it
/// page deck not a nav stack, nesting wrote into the outer stack path
/// going back crashed, trust me
#[derive(IntoElement)]
pub struct HighlightCreateSheet {
    id: ElementId,
    is_presented: Binding<bool>,
    user_id: String,
}

struct HighlightCreateSheetState {
    selected: Vec<i64>,
    title: String,
    cover: Option<(i64, Option<Rc<StoryDTO>>)>,
}

impl HighlightCreateSheet {
    const SELECT_PAGE_ID: &'static str = "select";
    const CREATE_PAGE_ID: &'static str = "create";

    pub fn new(id: impl Into<ElementId>, is_presented: Binding<bool>, user_id: impl Into<String>) -> Self {
        Self { id: id.into(), is_presented, user_id: user_id.into() }
    }

    fn pages(&self, state: &Entity<HighlightCreateSheetState>, cx: &App) -> Vec<BottomSheetScreenItem> {
        let selected_count = state.read(cx).selected.len();
        let read_selected = state.clone();
        let write_selected = state.clone();
        let selected = Binding::new(
            move |cx: &App| read_selected.read(cx).selected.clone(),
            move |value: Vec<i64>, _: &mut Window, cx: &mut App| {
                write_selected.update(cx, |state, cx| {
                    state.selected = value;
                    cx.notify();
                });
            },
        );
        let read_title = state.clone();
        let write_title = state.clone();
        let title = Binding::new(
            move |cx: &App| read_title.read(cx).title.clone(),
            move |value: String, _: &mut Window, cx: &mut App| {
                write_title.update(cx, |state, cx| {
                    state.title = value;
                    cx.notify();
                });
            },
        );

        let select_id = child_id(&self.id, "select-page");
        let select_user_id = self.user_id.clone();
        let create_id = child_id(&self.id, "create-page");
        let create_user_id = self.user_id.clone();
        let create_state = state.clone();
        let is_presented = self.is_presented.clone();
        vec![
            BottomSheetScreenItem::new(
                Self::SELECT_PAGE_ID,
                format!("{} {}", tr("highlights_selected"), selected_count),
                HighlightGridMetrics::SHEET_FRACTION,
                move |_: &mut Window, _: &mut App| {
                    HighlightStorySelectionView::new(
                        select_id.clone(),
                        select_user_id.clone(),
                        selected.clone(),
                        |navigation, window, cx| {
                            if let Some(navigation) = navigation {
                                BottomSheetNavigationState::navigate(&navigation, Self::CREATE_PAGE_ID, window, cx);
                            }
                        },
                    )
                },
            ),
            BottomSheetScreenItem::new(
                Self::CREATE_PAGE_ID,
                tr("highlights_create_title"),
                HighlightGridMetrics::SHEET_FRACTION,
                move |_: &mut Window, cx: &mut App| {
                    let selected_story_ids = create_state.read(cx).selected.clone();
                    let cover_story = Self::cover_story(&create_state, &create_user_id, cx);
                    let close = is_presented.clone();
                    HighlightCreateView {
                        id: create_id.clone(),
                        title: title.clone(),
                        selected_story_ids,
                        cover_story,
                        on_created: Rc::new(move |window: &mut Window, cx: &mut App| close.set(false, window, cx)),
                    }
                },
            ),
        ]
    }

    fn cover_story(state: &Entity<HighlightCreateSheetState>, user_id: &str, cx: &mut App) -> Option<Rc<StoryDTO>> {
        let first = *state.read(cx).selected.first()?;
        let cached = state.read(cx).cover.as_ref().filter(|(id, _)| *id == first).map(|(_, cover)| cover.clone());
        if let Some(cover) = cached {
            return cover;
        }
        let stories =
            StoryServicesStore::shared(cx).update(cx, |service, cx| service.highlight_picker_stories(user_id, cx));
        let cover = stories.into_iter().find(|story| story.id == first).map(Rc::new);
        let stored = cover.clone();
        state.update(cx, |state, _| state.cover = Some((first, stored)));
        cover
    }
}

impl RenderOnce for HighlightCreateSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._100;
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, _| HighlightCreateSheetState {
            selected: Vec::new(),
            title: String::new(),
            cover: None,
        });
        let pages = self.pages(&state, cx);
        let refused = self.is_presented.clone();
        BottomSheetPagesUi::new(child_id(&self.id, "pages"), self.is_presented.clone(), pages, Self::SELECT_PAGE_ID)
            .sheet_background(Some(background))
            .on_present(move |_, cx| {
                state.update(cx, |state, cx| {
                    state.selected = Vec::new();
                    state.title = String::new();
                    state.cover = None;
                    cx.notify();
                });
            })
            .on_presentation_refused(move |window, cx| refused.set(false, window, cx))
    }
}

pub enum HighlightGridMetrics {}

impl HighlightGridMetrics {
    pub const COLUMNS: usize = 3;
    pub const CELL_SPACING: f32 = 2.0;
    pub const HORIZONTAL_PADDING: f32 = 2.0;
    pub const CORNER_RADIUS: f32 = 6.0;
    pub const SHEET_FRACTION: f32 = 1.0;
    pub const ACTION_BAR_HEIGHT: f32 = 72.0;
    pub const CELL_ASPECT: f32 = 9.0 / 16.0;
    pub const PAGINATION_THRESHOLD: usize = 9;

    pub fn cell_width(width: f32) -> f32 {
        let available =
            width - Self::HORIZONTAL_PADDING * 2.0 - Self::CELL_SPACING * (Self::COLUMNS as f32 - 1.0);
        (available / Self::COLUMNS as f32).floor().max(40.0)
    }

    pub fn cell_height(cell_width: f32) -> f32 {
        (cell_width / Self::CELL_ASPECT).round()
    }

    pub fn decode_dimension(width: f32, height: f32) -> i64 {
        (width.max(height) * 3.0).ceil() as i64
    }
}

/// the one action of a deck page, floats over the scrolling content
#[derive(IntoElement)]
pub struct HighlightSheetActionPill {
    id: ElementId,
    title: String,
    system_image: &'static str,
    disabled: bool,
    pending: bool,
    action: HighlightSheetAction,
}

impl HighlightSheetActionPill {
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<String>,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            system_image: "checkmark",
            disabled: false,
            pending: false,
            action: Rc::new(action),
        }
    }

    pub fn system_image(mut self, system_image: &'static str) -> Self {
        self.system_image = system_image;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn pending(mut self, pending: bool) -> Self {
        self.pending = pending;
        self
    }
}

impl RenderOnce for HighlightSheetActionPill {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let foreground = if self.disabled { theme.text.secondary } else { theme.content_on(theme.primary._100) };
        let tint = (!self.disabled).then_some(theme.primary._100);
        let action = self.action;
        let leading = if self.pending {
            progress_view().tint(theme.content_on(theme.primary._100)).into_any_element()
        } else {
            div().children(IconUi::sized(self.system_image, px(14.0), foreground, false)).into_any_element()
        };
        div().h(px(HighlightGridMetrics::ACTION_BAR_HEIGHT)).flex().items_center().child(
            LiquidGlassButton::new(self.id, LiquidGlassButtonShape::Capsule)
                .horizontal_padding(22.0)
                .vertical_padding(12.0)
                .tint(tint)
                .disabled(self.disabled || self.pending)
                .accessibility_label(Some(self.title.clone()))
                .on_press(move |window, cx| action(window, cx))
                .child(
                    hstack(8.0).child(leading).child(
                        MainText::new(self.title)
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(foreground)
                            .number_of_lines(1),
                    ),
                ),
        )
    }
}

/// round cover both highlight pages open with
pub(super) fn highlight_cover(story: Option<&StoryDTO>, side: f32, decode_dimension: i64, theme: &AppTheme) -> Div {
    let cover = div()
        .relative()
        .flex_shrink_0()
        .w(px(side))
        .h(px(side))
        .rounded_full()
        .bg(theme.bg._200)
        .flex()
        .items_center()
        .justify_center();
    let cover = match story.filter(|story| !story.media_source().is_empty()) {
        Some(story) => cover.child(
            StoryThumbnailUi::from_story(story, side, side, decode_dimension)
                .corner_radius(side / 2.0)
                .shows_progress_indicator(true),
        ),
        None => cover.children(IconUi::sized("photo", px(28.0), theme.text.secondary, false)),
    };
    cover.child(centred_stroke(theme.border._200.opacity(0.7), 2.0, Shape::Circle))
}

/// centered name field of both highlight pages
pub(super) fn highlight_title_field(
    id: ElementId,
    rule: RiseFieldRule,
    title: Binding<String>,
    theme: &AppTheme,
) -> ShapedTextField {
    ShapedTextField::new(id, "", rule.binding(title))
        .prompt(Some(tr("highlights_name_placeholder")))
        .placeholder_color(theme.text.secondary.opacity(0.7))
        .font_size(22.0)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.text.primary)
        .text_align(TextAlign::Center)
}

/// day_label is set only on the first story of a day
/// wire_index is the position in the wire array, newest first
/// grid reads it reversed
#[derive(Clone, PartialEq)]
struct HighlightStoryTile {
    id: i64,
    source: String,
    is_video: bool,
    day_label: Option<String>,
    wire_index: usize,
}

#[derive(Clone, PartialEq)]
struct HighlightStoriesSignature {
    count: usize,
    newest_id: i64,
    oldest_id: i64,
    newest_source: String,
}

impl HighlightStoriesSignature {
    fn new(stories: &[StoryDTO]) -> Self {
        Self {
            count: stories.len(),
            newest_id: stories.first().map_or(0, |story| story.id),
            oldest_id: stories.last().map_or(0, |story| story.id),
            newest_source: stories.first().map_or_else(String::new, |story| story.file_id.clone()),
        }
    }
}

type HighlightStorySelectionNext = Rc<dyn Fn(Option<Entity<BottomSheetNavigationState>>, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct HighlightStorySelectionView {
    id: ElementId,
    user_id: String,
    selected: Binding<Vec<i64>>,
    on_next: HighlightStorySelectionNext,
}

/// paging info read once per render since every lookup decodes a cached payload
#[derive(Clone)]
struct HighlightStorySelectionViewPaging {
    has_more: bool,
    cursor: Option<i64>,
    cache_key: String,
}

impl HighlightStorySelectionViewPaging {
    fn relative_id(&self) -> Option<String> {
        self.cursor.map(|cursor| cursor.to_string())
    }
}

struct HighlightStorySelectionViewState {
    tiles: Rc<Vec<HighlightStoryTile>>,
    rows: ListState,
    row_count: usize,
    width: f32,
    height: f32,
    row_views: HashMap<usize, Entity<HighlightStoryGridRowView>>,
    read: Option<Rc<HighlightStoryPickerRead>>,
    _service_observation: Subscription,
}

/// story services as one picker render reads them
struct HighlightStoryPickerRead {
    user_id: String,
    revisions: [i64; 2],
    paging: HighlightStorySelectionViewPaging,
    stories: Rc<Vec<StoryDTO>>,
    is_loaded: bool,
}

impl HighlightStorySelectionViewState {
    const ROW_OVERDRAW: f32 = 120.0;

    fn set_tiles(&mut self, tiles: Vec<HighlightStoryTile>) {
        let columns = HighlightGridMetrics::COLUMNS;
        let row_count = (tiles.len() + columns - 1) / columns;
        if self.row_count == 0 {
            self.rows.reset(row_count + 2);
        } else if row_count > self.row_count {
            self.rows.splice(1..1, row_count - self.row_count);
        } else if row_count < self.row_count {
            self.rows.splice(1..1 + self.row_count - row_count, 0);
        }
        self.row_count = row_count;
        self.tiles = Rc::new(tiles);
    }
}

/// what one grid row needs, shared by every row of a frame
#[derive(Clone)]
struct HighlightStoryGridRowContext {
    user_id: String,
    tiles: Rc<Vec<HighlightStoryTile>>,
    selected: Binding<Vec<i64>>,
    selected_ids: Rc<HashSet<i64>>,
    paging: HighlightStorySelectionViewPaging,
    relative_id: Option<String>,
    cell_width: f32,
    cell_height: f32,
    decode_dimension: i64,
}

impl HighlightStoryGridRowContext {
    fn grid_row(&self, row: usize) -> Div {
        let columns = HighlightGridMetrics::COLUMNS;
        let first = row * columns;
        let last = (first + columns).min(self.tiles.len());
        let columns_width =
            self.cell_width * columns as f32 + HighlightGridMetrics::CELL_SPACING * (columns as f32 - 1.0);
        let cells = self.tiles[first.min(last)..last].iter().map(|tile| self.cell(tile).into_any_element());
        div().w_full().flex().justify_center().child(
            div()
                .w(px(columns_width))
                .h(px(self.cell_height + HighlightGridMetrics::CELL_SPACING))
                .flex()
                .flex_row()
                .items_start()
                .gap(px(HighlightGridMetrics::CELL_SPACING))
                .children(cells.collect::<Vec<AnyElement>>()),
        )
    }

    fn row_marks(&self, row: usize) -> HighlightStoryRowMarks {
        let columns = HighlightGridMetrics::COLUMNS;
        let first = row * columns;
        let last = (first + columns).min(self.tiles.len());
        let tiles = self.tiles[first.min(last)..last].to_vec();
        let selected = tiles.iter().map(|tile| self.selected_ids.contains(&tile.id)).collect();
        HighlightStoryRowMarks {
            tiles,
            selected,
            first_id: self.tiles.first().map(|tile| tile.id),
            tile_count: self.tiles.len(),
            cache_key: self.paging.cache_key.clone(),
            relative_id: self.relative_id.clone(),
            has_more: self.paging.has_more,
            cell_width: self.cell_width,
            cell_height: self.cell_height,
            decode_dimension: self.decode_dimension,
        }
    }

    fn cell(&self, tile: &HighlightStoryTile) -> AnyElement {
        let tile_id = tile.id;
        let selected = self.selected.clone();
        let is_first = self.tiles.first().is_some_and(|first| first.id == tile.id);
        let user_id = self.user_id.clone();
        let cell = HighlightStoryCell {
            tile: tile.clone(),
            is_selected: self.selected_ids.contains(&tile.id),
            width: self.cell_width,
            height: self.cell_height,
            decode_dimension: self.decode_dimension,
            on_tap: Rc::new(move |window: &mut Window, cx: &mut App| {
                HighlightStorySelectionView::toggle(&selected, tile_id, window, cx);
            }),
        }
        .guide_anchor(is_first.then_some(RiseGuideAnchor::HighlightPickerFirstStory));
        let prefetch_id = ElementId::from(("highlight-story-prefetch", tile.id as u64));
        if !Self::is_near_older_edge(tile.wire_index, self.tiles.len()) {
            return cell.on_appear(prefetch_id, |_, _| {}).into_any_element();
        }
        cell.sai_pagination_prefetch(
            prefetch_id,
            SaiPaginationPrefetchOptions::new(
                tile.wire_index,
                self.tiles.len(),
                self.paging.cache_key.clone(),
                self.relative_id.clone(),
                self.paging.has_more,
            )
            .threshold(HighlightGridMetrics::PAGINATION_THRESHOLD),
            move |_: &mut Window, cx: &mut App| {
                StoryInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_more_highlight_picker_stories(&user_id, cx));
            },
        )
        .into_any_element()
    }

    fn is_near_older_edge(wire_index: usize, tile_count: usize) -> bool {
        tile_count.saturating_sub(wire_index) <= HighlightGridMetrics::PAGINATION_THRESHOLD.max(1)
    }
}

impl HighlightStorySelectionView {
    pub fn new(
        id: impl Into<ElementId>,
        user_id: impl Into<String>,
        selected: Binding<Vec<i64>>,
        on_next: impl Fn(Option<Entity<BottomSheetNavigationState>>, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), user_id: user_id.into(), selected, on_next: Rc::new(on_next) }
    }

    fn paging(service: &Entity<StoryServicesStore>, user_id: &str, cx: &mut App) -> HighlightStorySelectionViewPaging {
        service.update(cx, |service, cx| {
            let scope = service.highlight_picker_paging_scope(user_id, cx);
            HighlightStorySelectionViewPaging {
                has_more: scope.is_some(),
                cursor: scope.and_then(|scope| service.profile_stories_next_cursor(user_id, scope, cx)),
                cache_key: StoryServicesStore::profile_stories_cache_key(
                    user_id,
                    scope.unwrap_or(StoryProfileScope::Profile),
                ),
            }
        })
    }

    fn picker_read(
        state: &Entity<HighlightStorySelectionViewState>,
        service: &Entity<StoryServicesStore>,
        user_id: &str,
        cx: &mut App,
    ) -> Rc<HighlightStoryPickerRead> {
        let revisions = Self::cache_revisions(user_id, cx);
        if let Some(read) =
            state.read(cx).read.as_ref().filter(|read| read.user_id == user_id && read.revisions == revisions)
        {
            return read.clone();
        }
        let paging = Self::paging(service, user_id, cx);
        let edge = paging.cursor.unwrap_or(0);
        let (stories, is_loaded) = service.update(cx, |service, cx| {
            (
                service.highlight_picker_stories_from_edge(user_id, edge, cx),
                service.highlight_picker_stories_loaded(user_id, cx),
            )
        });
        let read = Rc::new(HighlightStoryPickerRead {
            user_id: user_id.to_owned(),
            revisions,
            paging,
            stories: Rc::new(stories),
            is_loaded,
        });
        let stored = read.clone();
        state.update(cx, |state, _| state.read = Some(stored));
        read
    }

    /// reading the revisions subscribes the view to both page caches so a landed page redraws the picker
    fn cache_revisions(user_id: &str, cx: &mut App) -> [i64; 2] {
        [StoryProfileScope::Profile, StoryProfileScope::Archive].map(|scope| {
            SaiCacheRevisionTracker::observed_revision(
                &StoryServicesStore::profile_stories_cache_key(user_id, scope),
                cx,
            )
        })
    }

    fn content(
        state: &Entity<HighlightStorySelectionViewState>,
        context: HighlightStoryGridRowContext,
        is_loaded: bool,
        theme: &AppTheme,
        cx: &App,
    ) -> AnyElement {
        if context.tiles.is_empty() {
            if is_loaded {
                return Self::empty_state(theme).into_any_element();
            }
            return Self::skeleton(context.cell_width, context.cell_height).into_any_element();
        }
        Self::grid(state, context, cx).into_any_element()
    }

    fn grid(
        state: &Entity<HighlightStorySelectionViewState>,
        context: HighlightStoryGridRowContext,
        cx: &App,
    ) -> AnyElement {
        let columns = HighlightGridMetrics::COLUMNS;
        let rows = (context.tiles.len() + columns - 1) / columns;
        let spacing = HighlightGridMetrics::CELL_SPACING;
        let bottom_inset = HighlightGridMetrics::ACTION_BAR_HEIGHT + 8.0;
        let content_height =
            rows as f32 * context.cell_height + rows.saturating_sub(1) as f32 * spacing + 6.0 + bottom_inset;
        let state_entity = state;
        let state = state.read(cx);
        if content_height > state.height {
            let row_count = state.row_count;
            let rows_state = state_entity.clone();
            let context = Rc::new(context);
            return list(state.rows.clone(), move |index, _, cx| {
                if index == 0 {
                    return div().h(px(6.0)).into_any_element();
                }
                if index > row_count {
                    return div().h(px((bottom_inset - spacing).max(0.0))).into_any_element();
                }
                HighlightStoryGridRowView::element(&rows_state, index - 1, &context, cx)
            })
            .size_full()
            .into_any_element();
        }
        div()
            .size_full()
            .pt(px(6.0))
            .flex()
            .flex_col()
            .children((0..rows).map(|row| context.grid_row(row).into_any_element()))
            .into_any_element()
    }

    fn skeleton(cell_width: f32, cell_height: f32) -> impl IntoElement {
        let columns = HighlightGridMetrics::COLUMNS;
        let columns_width =
            cell_width * columns as f32 + HighlightGridMetrics::CELL_SPACING * (columns as f32 - 1.0);
        pass_through()
            .size_full()
            .flex()
            .flex_col()
            .justify_end()
            .items_center()
            .pb(px(HighlightGridMetrics::ACTION_BAR_HEIGHT + 8.0))
            .child(
                div()
                    .w(px(columns_width))
                    .flex()
                    .flex_wrap()
                    .gap(px(HighlightGridMetrics::CELL_SPACING))
                    .children((0..12).map(|_| {
                        SkeletonBlock::new(cell_width, cell_height, HighlightGridMetrics::CORNER_RADIUS)
                    })),
            )
            .skeleton_shimmer(None)
    }

    fn empty_state(theme: &AppTheme) -> impl IntoElement {
        vstack(12.0)
            .size_full()
            .justify_center()
            .px(px(32.0))
            .children(IconUi::sized("photo.stack", px(34.0), theme.text.secondary, false))
            .child(MainText::new(tr("story_no_stories")).px(15.0).tac(TextAlignT::Center).color(theme.text.secondary))
    }

    fn load_stories(user_id: &str, cx: &mut App) {
        if user_id.is_empty() {
            return;
        }
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.load_highlight_picker_stories(user_id, cx));
    }

    fn build_tiles(stories: &[StoryDTO]) -> Vec<HighlightStoryTile> {
        let calendar = Calendar::current();
        let now = Date::now();
        let locale = RiseFormat::app_locale();
        let mut tiles = Vec::with_capacity(stories.len());
        let mut previous_day: Option<Date> = None;

        for (index, story) in stories.iter().enumerate().rev() {
            let mut day_label = None;
            if story.created_at_ms > 0 {
                let date = Date::from_time_interval_since_1970(story.created_at_ms as f64 / 1000.0);
                let day = calendar.start_of_day(&date);
                if Some(day) != previous_day {
                    previous_day = Some(day);
                    day_label = Some(RiseFormat::compact_day_label(&date, &now, &calendar, &locale));
                }
            }
            tiles.push(HighlightStoryTile {
                id: story.id,
                source: story.media_source(),
                is_video: story.media_type.to_lowercase() == "video",
                day_label,
                wire_index: index,
            });
        }
        tiles
    }

    fn toggle(selected: &Binding<Vec<i64>>, id: i64, window: &mut Window, cx: &mut App) {
        let _ = selection_changed();
        let mut ids = selected.get(cx);
        if let Some(index) = ids.iter().position(|selected_id| *selected_id == id) {
            ids.remove(index);
        } else {
            ids.push(id);
        }
        selected.set(ids, window, cx);
    }
}

impl RenderOnce for HighlightStorySelectionView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let service = StoryServicesStore::shared(cx);
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, cx: &mut Context<HighlightStorySelectionViewState>| {
            let service = StoryServicesStore::shared(cx);
            HighlightStorySelectionViewState {
                tiles: Rc::new(Vec::new()),
                rows: ListState::new(2, ListAlignment::Bottom, px(HighlightStorySelectionViewState::ROW_OVERDRAW)),
                row_count: 0,
                width: 0.0,
                height: 0.0,
                row_views: HashMap::new(),
                read: None,
                _service_observation: cx.observe(&service, |state, _, cx| {
                    state.read = None;
                    cx.notify();
                }),
            }
        });

        let read = Self::picker_read(&state, &service, &self.user_id, cx);
        let paging = read.paging.clone();
        let is_loaded = read.is_loaded;
        let signature = HighlightStoriesSignature::new(&read.stories);
        let tiles_state = state.clone();
        let tiles_stories = read.stories.clone();
        on_change(child_id(&self.id, "signature"), signature, true, window, cx, move |_, _, _, cx| {
            let tiles = Self::build_tiles(&tiles_stories);
            tiles_state.update(cx, |state, cx| {
                state.set_tiles(tiles);
                cx.notify();
            });
        });

        let selected_ids = self.selected.get(cx);
        on_change(child_id(&self.id, "selected"), !selected_ids.is_empty(), true, window, cx, |_, picked, _, cx| {
            GuideFacts::set(RiseGuideSignal::HighlightStoriesPicked, *picked, cx);
        });

        let (tiles, width) = {
            let state = state.read(cx);
            (state.tiles.clone(), state.width)
        };
        let cell_width = HighlightGridMetrics::cell_width(width);
        let cell_height = HighlightGridMetrics::cell_height(cell_width);
        let relative_id = paging.relative_id();
        let context = HighlightStoryGridRowContext {
            user_id: self.user_id.clone(),
            tiles: tiles.clone(),
            selected: self.selected.clone(),
            selected_ids: Rc::new(selected_ids.iter().copied().collect()),
            paging: paging.clone(),
            relative_id: relative_id.clone(),
            cell_width,
            cell_height,
            decode_dimension: HighlightGridMetrics::decode_dimension(cell_width, cell_height),
        };

        let navigation = bottom_sheet_navigation(cx);
        let on_next = self.on_next.clone();
        let measured = state.clone();
        let appear_user_id = self.user_id.clone();
        let drain_user_id = self.user_id.clone();
        let is_empty_selection = selected_ids.is_empty();
        div()
            .relative()
            .size_full()
            .bg(theme.bg._100)
            .child(measure(child_id(&self.id, "size"), move |bounds, _, cx| {
                measured.update(cx, |state, cx| {
                    state.width = f32::from(bounds.size.width);
                    state.height = f32::from(bounds.size.height);
                    cx.notify();
                });
            }))
            .when(width > 0.0, |root| root.child(Self::content(&state, context, is_loaded, &theme, cx)))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .pt(px(8.0))
                    .flex()
                    .justify_center()
                    .bg(linear_gradient(
                        180.0,
                        linear_color_stop(theme.bg._100.opacity(0.0), 0.0),
                        linear_color_stop(theme.bg._100, 0.45),
                    ))
                    .occlude()
                    .child(
                    HighlightSheetActionPill::new(
                        child_id(&self.id, "next"),
                        tr("next"),
                        move |window, cx| on_next(navigation.clone(), window, cx),
                    )
                    .system_image("arrow.right")
                    .disabled(is_empty_selection)
                    .guide_anchor(RiseGuideAnchor::HighlightPickerNext),
                ),
            )
            .on_appear(child_id(&self.id, "appear"), move |_, cx| {
                Self::load_stories(&appear_user_id, cx);
                GuideFacts::set(RiseGuideSignal::HighlightPickerOpen, true, cx);
            })
            .on_disappear(child_id(&self.id, "disappear"), |_, cx| {
                GuideFacts::set(RiseGuideSignal::HighlightPickerOpen, false, cx);
                GuideFacts::set(RiseGuideSignal::HighlightStoriesPicked, false, cx);
            })
            .sai_pagination_drain_filtered_empty(
                child_id(&self.id, "drain"),
                SaiPaginationPrefetchOptions::drain_filtered_empty(
                    tiles.len(),
                    paging.cache_key,
                    relative_id,
                    paging.has_more,
                ),
                move |_, cx| {
                    StoryInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.load_more_highlight_picker_stories(&drain_user_id, cx));
                },
            )
    }
}

#[derive(Clone, PartialEq)]
struct HighlightStoryRowMarks {
    tiles: Vec<HighlightStoryTile>,
    selected: Vec<bool>,
    first_id: Option<i64>,
    tile_count: usize,
    cache_key: String,
    relative_id: Option<String>,
    has_more: bool,
    cell_width: f32,
    cell_height: f32,
    decode_dimension: i64,
}

struct HighlightStoryGridRowView {
    row: usize,
    context: Option<Rc<HighlightStoryGridRowContext>>,
    marks: Option<HighlightStoryRowMarks>,
}

impl HighlightStoryGridRowView {
    fn element(
        state: &Entity<HighlightStorySelectionViewState>,
        row: usize,
        context: &Rc<HighlightStoryGridRowContext>,
        cx: &mut App,
    ) -> AnyElement {
        let existing = state.read(cx).row_views.get(&row).cloned();
        let view = match existing {
            Some(view) => view,
            None => {
                let view = cx.new(|_| HighlightStoryGridRowView { row, context: None, marks: None });
                let inserted = view.clone();
                state.update(cx, |state, _| {
                    state.row_views.insert(row, inserted);
                });
                view
            }
        };
        let marks = context.row_marks(row);
        let stale = view.update(cx, |view, _| {
            let stale = view.marks.is_some() && view.marks.as_ref() != Some(&marks);
            if view.marks.as_ref() != Some(&marks) {
                view.marks = Some(marks);
            }
            view.context = Some(context.clone());
            stale
        });
        if stale {
            return AnyView::from(view).into_any_element();
        }
        AnyView::from(view)
            .cached(
                StyleRefinement::default()
                    .w_full()
                    .h(px(context.cell_height + HighlightGridMetrics::CELL_SPACING)),
            )
            .replay_when_moved()
            .into_any_element()
    }
}

impl Render for HighlightStoryGridRowView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        match &self.context {
            Some(context) => context.grid_row(self.row).into_any_element(),
            None => div().into_any_element(),
        }
    }
}

#[derive(IntoElement)]
struct HighlightStoryCell {
    tile: HighlightStoryTile,
    is_selected: bool,
    width: f32,
    height: f32,
    decode_dimension: i64,
    on_tap: HighlightSheetAction,
}

impl HighlightStoryCell {
    fn day_badge(text: &str) -> Div {
        div().absolute().top_0().left_0().right_0().p(px(6.0)).flex().flex_row().child(
            div()
                .flex_none()
                .max_w_full()
                .px(px(7.0))
                .py(px(4.0))
                .bg(black().opacity(0.55))
                .continuous_corners(8.0)
                .child(
                    MainText::new(text.to_owned())
                        .px(12.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(white())
                        .number_of_lines(1),
                ),
        )
    }

    fn selection_indicator(&self, theme: &AppTheme) -> Div {
        let indicator = div()
            .absolute()
            .top(px(7.0))
            .right(px(7.0))
            .w(px(24.0))
            .h(px(24.0))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center();
        if self.is_selected {
            return indicator
                .bg(theme.primary._100)
                .children(IconUi::sized("checkmark", px(12.0), theme.content_on(theme.primary._100), false));
        }
        indicator
            .child(div().absolute().top(px(1.0)).left_0().w(px(24.0)).h(px(24.0)).child(centred_stroke(
                black().opacity(0.25),
                1.5,
                Shape::Circle,
            )))
            .child(centred_stroke(white().opacity(0.9), 1.5, Shape::Circle))
    }
}

impl RenderOnce for HighlightStoryCell {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let indicator = self.selection_indicator(&theme);
        let on_tap = self.on_tap;
        let tile = self.tile;
        div()
            .id(("highlight-story-cell", tile.id as u64))
            .relative()
            .flex_shrink_0()
            .w(px(self.width))
            .h(px(self.height))
            .bg(theme.bg._200)
            .clip_shape(Shape::continuous(HighlightGridMetrics::CORNER_RADIUS))
            .cursor_pointer()
            .on_click(move |_, window, cx| on_tap(window, cx))
            .when(!tile.source.is_empty(), |cell| {
                cell.child(
                    StoryThumbnailUi::new(
                        tile.id,
                        tile.source.clone(),
                        tile.is_video,
                        self.width,
                        self.height,
                        self.decode_dimension,
                    )
                    .corner_radius(HighlightGridMetrics::CORNER_RADIUS),
                )
            })
            .children(
                tile.day_label
                    .as_deref()
                    .filter(|day_label| !day_label.is_empty())
                    .map(Self::day_badge),
            )
            .child(indicator)
    }
}

#[derive(IntoElement)]
struct HighlightCreateView {
    id: ElementId,
    title: Binding<String>,
    selected_story_ids: Vec<i64>,
    cover_story: Option<Rc<StoryDTO>>,
    on_created: HighlightSheetAction,
}

struct HighlightCreateViewState {
    error_text: Option<String>,
    creating: bool,
}

impl HighlightCreateView {
    /// highlight title rule, 1 to 100 chars per canon
    fn title_rule() -> RiseFieldRule {
        RiseFieldRule::new("story_create_highlight", "/title")
    }

    fn create(
        title: &Binding<String>,
        selected_story_ids: &[i64],
        on_created: &HighlightSheetAction,
        state: &Entity<HighlightCreateViewState>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if state.read(cx).creating {
            return;
        }
        state.update(cx, |state, cx| {
            state.error_text = None;
            state.creating = true;
            cx.notify();
        });
        let title = title.get(cx);
        let handle = window.window_handle();
        let on_created = on_created.clone();
        let done = state.clone();
        let on_success: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            done.update(cx, |state, cx| {
                state.creating = false;
                cx.notify();
            });
            let _ = notification(NotificationKind::Success);
            let on_created = on_created.clone();
            with_window(handle, move |window, cx| on_created(window, cx), cx);
        });
        let failed = state.clone();
        let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(move |message: String, cx: &mut App| {
            let _ = notification(NotificationKind::Error);
            failed.update(cx, |state, cx| {
                state.creating = false;
                state.error_text = Some(message);
                cx.notify();
            });
        });
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.create_highlight(&title, selected_story_ids, Some(on_success), Some(on_error), cx);
        });
    }
}

impl RenderOnce for HighlightCreateView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window
            .use_keyed_state(child_id(&self.id, "state"), cx, |_, _| HighlightCreateViewState {
                error_text: None,
                creating: false,
            });
        let error_text = state.read(cx).error_text.clone();
        let is_pending =
            state.read(cx).creating || StoryActionsStore::shared(cx).read(cx).create_highlight_sai.is_pending();
        let title_value = self.title.get(cx);
        let is_empty = trimming_whitespaces_and_newlines(&title_value).is_empty();

        on_change(child_id(&self.id, "title-filled"), !is_empty, true, window, cx, |_, filled, _, cx| {
            GuideFacts::set(RiseGuideSignal::HighlightNameFilled, *filled, cx);
        });

        let selected_story_ids = Rc::new(self.selected_story_ids);
        let submit = {
            let title = self.title.clone();
            let ids = selected_story_ids.clone();
            let on_created = self.on_created.clone();
            let state = state.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                Self::create(&title, &ids, &on_created, &state, window, cx);
            })
        };
        let on_submit = submit.clone();
        let on_cancel = self.on_created.clone();

        scroll_view(child_id(&self.id, "scroll"))
            .shows_indicators(false)
            .size_full()
            .bg(theme.bg._100)
            .child(
                vstack(14.0)
                    .w_full()
                    .pb(px(24.0))
                    .child(div().pt(px(20.0)).child(highlight_cover(self.cover_story.as_deref(), 132.0, 396, &theme)))
                    .child(
                        hstack(0.0)
                            .w_full()
                            .pt(px(6.0))
                            .px(px(24.0))
                            .child(
                                highlight_title_field(
                                    child_id(&self.id, "title"),
                                    Self::title_rule(),
                                    self.title.clone(),
                                    &theme,
                                )
                                .auto_focus(true)
                                .on_submit(move |window, cx| on_submit(window, cx))
                                .on_cancel(move |window, cx| on_cancel(window, cx))
                                .guide_anchor(RiseGuideAnchor::HighlightNameField),
                            ),
                    )
                    .children(error_text.map(|error_text| {
                        div().px(px(24.0)).child(
                            MainText::new(error_text)
                                .px(13.0)
                                .tac(TextAlignT::Center)
                                .color(system_red(cx)),
                        )
                    }))
                    .child(
                        div().pt(px(8.0)).child(
                            HighlightSheetActionPill::new(child_id(&self.id, "done"), tr("done"), move |window, cx| {
                                submit(window, cx);
                            })
                            .disabled(is_empty)
                            .pending(is_pending)
                            .guide_anchor(RiseGuideAnchor::HighlightCreateDone),
                        ),
                    ),
            )
            .on_appear(child_id(&self.id, "appear"), |_, cx| {
                GuideFacts::set(RiseGuideSignal::HighlightNaming, true, cx);
            })
            .on_disappear(child_id(&self.id, "disappear"), |_, cx| {
                GuideFacts::set(RiseGuideSignal::HighlightNaming, false, cx);
                GuideFacts::set(RiseGuideSignal::HighlightNameFilled, false, cx);
            })
    }
}
