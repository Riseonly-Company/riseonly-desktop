use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Bounds, BoxShadow, Context, Entity, EntityId, FontWeight, Hsla, ImageSource, IntoElement, ObjectFit,
    Pixels, Point, Render, Rgba, RenderImage, SharedString, StyledImage, WeakEntity, Window, black, canvas, hsla, img,
    px, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::photo_library::{ImageRequestId, LibraryAsset, LibraryAssetKind};
use rise_theme::theme::theme_environment::theme_store;
use rise_ui::icon_ui::IconUi;
use rise_ui::media_spoiler_overlay_view::MediaSpoilerOverlayView;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::window_screen_frame_reader::MediaCellScreenFrameRegistry;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerImageMemoryCache;
use crate::modules::media_picker::editor::editor_edit_preview_cache::EditorEditPreviewCache;
use crate::modules::media_picker::library::photo_library_fetcher::PhotoLibraryFetcher;
use crate::modules::media_picker::ui::media_backdrop_hooks::media_edge_color;

pub type MediaPickerGridCellAction = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum DisplayQuality {
    None,
    RecycledPlaceholder,
    Grid,
    Edited,
}

/// arguments for configuring a grid cell
pub struct MediaPickerGridCellConfiguration<'a> {
    pub asset: &'a LibraryAsset,
    pub fetcher: &'a Entity<PhotoLibraryFetcher>,
    pub selection_index: Option<i64>,
    pub shows_selection_control: bool,
    pub is_selectable: bool,
    pub has_edits: bool,
    pub loads_thumbnail: bool,
    pub frame_registry_id: String,
    pub on_tap_thumbnail: MediaPickerGridCellAction,
    pub on_tap_selection: MediaPickerGridCellAction,
}

pub struct MediaPickerGridCell {
    image: Option<Arc<RenderImage>>,
    spoiler_view: Entity<MediaSpoilerOverlayView>,
    spoiler_view_is_hidden: bool,
    duration_label: String,
    duration_background_is_hidden: bool,
    selection_badge: Entity<MediaPickerSelectionBadge>,
    selection_badge_is_hidden: bool,
    selection_overlay_is_hidden: bool,
    edited_indicator_is_hidden: bool,
    content_view_alpha: f32,
    background_color: Hsla,
    applied_overlay_metrics: Option<MediaPickerTileOverlayMetrics>,
    tile_side: Rc<Cell<f32>>,
    pub marks_with_check: bool,
    alpha: f32,

    thumbnail_request_id: Option<ImageRequestId>,
    thumbnail_request_generation: u64,
    thumbnail_request_asset_identifier: Option<String>,
    fetcher: Option<WeakEntity<PhotoLibraryFetcher>>,
    represented_asset_identifier: Option<String>,
    displayed_asset_identifier: Option<String>,
    display_quality: DisplayQuality,
    shows_edited_preview: bool,

    on_tap_thumbnail: Option<MediaPickerGridCellAction>,
    on_tap_selection: Option<MediaPickerGridCellAction>,

    frame_registry_id: Option<String>,
    image_view_frame: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl MediaPickerGridCell {
    pub const REUSE_IDENTIFIER: &'static str = "MediaPickerGridCell";

    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.on_release(|cell, cx| {
            cell.cancel_thumbnail_request(cx);
            if let Some(id) = cell.frame_registry_id.take() {
                MediaCellScreenFrameRegistry::remove(&id);
            }
        })
        .detach();
        Self {
            image: None,
            spoiler_view: cx.new(|_| MediaSpoilerOverlayView::new()),
            spoiler_view_is_hidden: true,
            duration_label: String::new(),
            duration_background_is_hidden: true,
            selection_badge: cx.new(|_| MediaPickerSelectionBadge::new()),
            selection_badge_is_hidden: false,
            selection_overlay_is_hidden: true,
            edited_indicator_is_hidden: true,
            content_view_alpha: 1.0,
            background_color: hsla(0.0, 0.0, 0.07, 1.0),
            applied_overlay_metrics: None,
            tile_side: Rc::new(Cell::new(0.0)),
            marks_with_check: false,
            alpha: 1.0,
            thumbnail_request_id: None,
            thumbnail_request_generation: 0,
            thumbnail_request_asset_identifier: None,
            fetcher: None,
            represented_asset_identifier: None,
            displayed_asset_identifier: None,
            display_quality: DisplayQuality::None,
            shows_edited_preview: false,
            on_tap_thumbnail: None,
            on_tap_selection: None,
            frame_registry_id: None,
            image_view_frame: Rc::new(Cell::new(None)),
        }
    }

    pub fn frame_registry_id(&self) -> Option<&str> {
        self.frame_registry_id.as_deref()
    }

    pub fn set_frame_registry_id(&mut self, frame_registry_id: Option<String>) {
        if let Some(old) = self.frame_registry_id.as_deref()
            && frame_registry_id.as_deref() != Some(old)
        {
            MediaCellScreenFrameRegistry::remove(old);
        }
        self.frame_registry_id = frame_registry_id;
    }

    pub fn refresh_screen_frame(&self) {
        self.report_screen_frame();
    }

    pub fn capture_screen_frame(&self) -> Option<Bounds<Pixels>> {
        self.report_screen_frame();
        MediaCellScreenFrameRegistry::frame(self.frame_registry_id.as_deref()?)
    }

    fn report_screen_frame(&self) {
        let (Some(id), Some(screen)) = (self.frame_registry_id.as_deref(), self.image_view_frame.get()) else {
            return;
        };
        MediaCellScreenFrameRegistry::set(id, screen, None);
    }

    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    pub fn set_alpha(&mut self, alpha: f32, cx: &mut Context<Self>) {
        if self.alpha != alpha {
            self.alpha = alpha;
            cx.notify();
        }
    }

    pub fn prepare_for_reuse(&mut self, cx: &mut Context<Self>) {
        self.set_selectable(true, cx);
        self.cancel_thumbnail_request(cx);
        self.represented_asset_identifier = None;
        if let Some(id) = self.frame_registry_id.as_deref() {
            MediaCellScreenFrameRegistry::remove(id);
        }
        self.set_frame_registry_id(None);
        self.duration_background_is_hidden = true;
        self.selection_badge.update(cx, |badge, cx| badge.set_selected(false, None, false, false, cx));
        self.selection_overlay_is_hidden = true;
        self.spoiler_view_is_hidden = true;
        self.edited_indicator_is_hidden = true;
        self.shows_edited_preview = false;
        self.on_tap_thumbnail = None;
        self.on_tap_selection = None;
        cx.notify();
    }

    /// dims content_view, never the cell
    /// alpha on the cell belongs to the hero lift
    /// a dimmed cell still takes taps, past a tier ceiling it offers premium
    pub fn set_selectable(&mut self, selectable: bool, cx: &mut Context<Self>) {
        self.content_view_alpha = if selectable { 1.0 } else { 0.32 };
        cx.notify();
    }

    /// a selected photo or video the sender hid under a spoiler shows it right in the grid
    pub fn set_spoiler(&mut self, hidden: bool, cx: &mut Context<Self>) {
        if hidden {
            if self.spoiler_view_is_hidden {
                self.spoiler_view_is_hidden = false;
                self.spoiler_view.update(cx, |spoiler, cx| spoiler.reset(cx));
            }
            let image = self.image.clone();
            let key = self.spoiler_key();
            self.spoiler_view.update(cx, |spoiler, cx| spoiler.set_source(image, &key, cx));
        } else {
            self.spoiler_view_is_hidden = true;
        }
        cx.notify();
    }

    fn spoiler_key(&self) -> String {
        format!("picker:{}:{:?}", self.displayed_asset_identifier.as_deref().unwrap_or(""), self.display_quality)
    }

    fn apply_overlay_metrics(&mut self, cx: &mut Context<Self>) {
        let side = self.tile_side.get();
        if side <= 0.0 {
            return;
        }
        let metrics = MediaPickerTileOverlayMetrics::new(side);
        if Some(metrics) == self.applied_overlay_metrics {
            return;
        }
        self.applied_overlay_metrics = Some(metrics);
        self.selection_badge.update(cx, |badge, cx| badge.set_label_font_size(metrics.badge_font_size, cx));
    }

    fn overlay_metrics(&self) -> MediaPickerTileOverlayMetrics {
        self.applied_overlay_metrics.unwrap_or(MediaPickerTileOverlayMetrics::CONSTRAINT_CONSTANTS)
    }

    pub fn set_edited(&mut self, edited: bool, cx: &mut Context<Self>) {
        self.edited_indicator_is_hidden = !edited;
        cx.notify();
    }

    pub fn configure(&mut self, configuration: MediaPickerGridCellConfiguration<'_>, cx: &mut Context<Self>) {
        let MediaPickerGridCellConfiguration {
            asset,
            fetcher,
            selection_index,
            shows_selection_control,
            is_selectable,
            has_edits,
            loads_thumbnail,
            frame_registry_id,
            on_tap_thumbnail,
            on_tap_selection,
        } = configuration;
        self.fetcher = Some(fetcher.downgrade());
        let same_asset = self.represented_asset_identifier.as_deref() == Some(asset.id.as_str());
        if !same_asset {
            self.cancel_thumbnail_request(cx);
        }
        self.represented_asset_identifier = Some(asset.id.clone());
        self.on_tap_thumbnail = Some(on_tap_thumbnail);
        self.on_tap_selection = Some(on_tap_selection);
        self.set_frame_registry_id(Some(frame_registry_id));
        self.selection_badge_is_hidden = !shows_selection_control;

        let is_video = matches!(asset.kind, LibraryAssetKind::Video);
        self.duration_background_is_hidden = !is_video;
        if is_video {
            self.duration_label = Self::format_duration(asset.duration_secs);
        }

        self.shows_edited_preview = has_edits;
        let edited = if has_edits { EditorEditPreviewCache::image(&asset.id, cx) } else { None };
        if let Some(edited) = edited {
            self.cancel_thumbnail_request(cx);
            self.set_exact_image(edited, &asset.id, DisplayQuality::Edited, cx);
        } else if self.displayed_asset_identifier.as_deref() != Some(asset.id.as_str()) {
            self.show_own_cached_thumbnail_or_blank(&asset.id);
        }

        self.set_selection_index(if shows_selection_control { selection_index } else { None }, false, cx);
        self.set_selectable(is_selectable, cx);
        self.set_edited(has_edits, cx);

        if !loads_thumbnail {
            return;
        }
        let shows_this_asset = self.displayed_asset_identifier.as_deref() == Some(asset.id.as_str());
        if has_edits && shows_this_asset && self.display_quality == DisplayQuality::Edited {
            return;
        }

        let asset_id = asset.id.clone();
        if same_asset && shows_this_asset && self.display_quality == DisplayQuality::Grid {
            return;
        }
        if self.thumbnail_request_id.is_some()
            && self.thumbnail_request_asset_identifier.as_deref() == Some(asset_id.as_str())
        {
            return;
        }

        self.cancel_thumbnail_request(cx);
        self.thumbnail_request_generation = self.thumbnail_request_generation.wrapping_add(1);
        let generation = self.thumbnail_request_generation;
        self.thumbnail_request_asset_identifier = Some(asset_id.clone());
        let this = cx.weak_entity();
        self.thumbnail_request_id = fetcher.update(cx, |fetcher, cx| {
            fetcher.request_grid_thumbnail(
                asset,
                move |delivery, cx| {
                    let _ = this.update(cx, |this, cx| {
                        if this.thumbnail_request_generation != generation
                            || this.represented_asset_identifier.as_deref() != Some(asset_id.as_str())
                        {
                            return;
                        }

                        if delivery.is_cancelled || !delivery.is_degraded {
                            this.thumbnail_request_id = None;
                            this.thumbnail_request_asset_identifier = None;
                        }
                        if delivery.is_cancelled {
                            return;
                        }
                        let Some(image) = delivery.image else {
                            return;
                        };
                        if this.shows_edited_preview && EditorEditPreviewCache::image(&asset_id, cx).is_some() {
                            return;
                        }
                        this.set_exact_image(image, &asset_id, DisplayQuality::Grid, cx);
                    });
                },
                cx,
            )
        });
    }

    pub fn suspend_thumbnail_loading(&mut self, cx: &mut App) {
        self.cancel_thumbnail_request(cx);
    }

    fn cancel_thumbnail_request(&mut self, cx: &mut App) {
        self.thumbnail_request_generation = self.thumbnail_request_generation.wrapping_add(1);
        if let Some(id) = self.thumbnail_request_id
            && let Some(fetcher) = self.fetcher.as_ref().and_then(|fetcher| fetcher.upgrade())
        {
            fetcher.update(cx, |fetcher, _| fetcher.cancel_grid_thumbnail_request(Some(id)));
        }
        self.thumbnail_request_id = None;
        self.thumbnail_request_asset_identifier = None;
    }

    fn show_own_cached_thumbnail_or_blank(&mut self, asset_id: &str) {
        if let Some(cached) = MediaViewerImageMemoryCache::memory_only(
            &format!("ph://{asset_id}"),
            SaiAccountCacheScope::active_user_id().as_deref(),
        ) {
            self.image = Some(cached);
            self.displayed_asset_identifier = Some(asset_id.to_string());
            self.display_quality = DisplayQuality::RecycledPlaceholder;
            return;
        }
        self.image = None;
        self.displayed_asset_identifier = None;
        self.display_quality = DisplayQuality::None;
    }

    pub fn set_placeholder_color(&mut self, color: Hsla, cx: &mut Context<Self>) {
        if self.background_color != color {
            self.background_color = color;
            cx.notify();
        }
    }

    fn set_exact_image(
        &mut self,
        image: Arc<RenderImage>,
        asset_id: &str,
        quality: DisplayQuality,
        cx: &mut Context<Self>,
    ) {
        self.image = Some(image.clone());
        self.displayed_asset_identifier = Some(asset_id.to_string());
        self.display_quality = quality;
        if !self.spoiler_view_is_hidden {
            let key = self.spoiler_key();
            let source = image.clone();
            self.spoiler_view.update(cx, |spoiler, cx| spoiler.set_source(Some(source), &key, cx));
        }
        MediaViewerImageMemoryCache::store(
            image,
            &format!("ph://{asset_id}"),
            SaiAccountCacheScope::active_user_id().as_deref(),
        );
        cx.notify();
    }

    pub fn set_selection_index(&mut self, index: Option<i64>, animated: bool, cx: &mut Context<Self>) {
        if let Some(index) = index {
            let check = self.marks_with_check;
            self.selection_badge
                .update(cx, |badge, cx| badge.set_selected(true, Some(index + 1), animated, check, cx));
            self.selection_overlay_is_hidden = false;
        } else {
            self.selection_badge.update(cx, |badge, cx| badge.set_selected(false, None, animated, false, cx));
            self.selection_overlay_is_hidden = true;
        }
        cx.notify();
    }

    fn format_duration(seconds: f64) -> String {
        RiseFormat::duration_time_interval(seconds)
    }

    fn image_view(&self, view_id: EntityId) -> impl IntoElement {
        let frame = self.image_view_frame.clone();
        let tile_side = self.tile_side.clone();
        div()
            .absolute()
            .inset_0()
            .overflow_hidden()
            .children(
                self.image
                    .clone()
                    .map(|image| img(ImageSource::Render(image)).size_full().object_fit(ObjectFit::Cover)),
            )
            .child(
                canvas(
                    move |bounds, _, cx| {
                        frame.set(Some(bounds));
                        let side = f32::from(bounds.size.width).min(f32::from(bounds.size.height));
                        if side != tile_side.get() {
                            tile_side.set(side);
                            cx.notify(view_id);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }

    fn duration_background(&self) -> impl IntoElement {
        let metrics = self.overlay_metrics();
        div()
            .absolute()
            .right(px(metrics.duration_inset))
            .bottom(px(metrics.duration_inset))
            .h(px(metrics.duration_height))
            .flex()
            .flex_row()
            .items_center()
            .pl(px(metrics.duration_padding))
            .pr(px(metrics.duration_padding))
            .when(metrics.shows_video_icon, |plate| plate.gap(px(metrics.video_icon_gap)))
            .bg(black().opacity(0.45))
            .rounded(px(metrics.duration_corner_radius))
            .when(metrics.shows_video_icon, |plate| {
                plate.children(IconUi::sized("play.fill", px(metrics.video_icon_side), white(), true))
            })
            .child(
                div()
                    .text_size(px(metrics.duration_font_size))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(white())
                    .child(SharedString::from(self.duration_label.clone())),
            )
    }

    fn edited_indicator(&self) -> impl IntoElement {
        let metrics = self.overlay_metrics();
        div()
            .absolute()
            .left(px(metrics.corner_inset))
            .bottom(px(metrics.corner_inset))
            .size(px(metrics.edited_side))
            .rounded(px(metrics.edited_side / 2.0))
            .overflow_hidden()
            .bg(black().opacity(0.5))
            .children(IconUi::sized("pencil.tip.crop.circle.badge.plus", px(metrics.edited_side), white(), false))
    }
}

impl Render for MediaPickerGridCell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.apply_overlay_metrics(cx);
        let metrics = self.overlay_metrics();
        let on_tap_thumbnail = self.on_tap_thumbnail.clone();
        let on_tap_selection = self.on_tap_selection.clone();
        let touch_outset = MediaPickerSelectionBadge::TOUCH_OUTSET;
        let shows_spoiler = !self.spoiler_view_is_hidden && !self.spoiler_view.read(cx).is_hidden();
        let edge = media_edge_color(theme_store(cx).read(cx).current_theme());

        let content_view = div()
            .id("media-picker-grid-cell-content")
            .absolute()
            .inset_0()
            .opacity(self.content_view_alpha)
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                if let Some(on_tap_thumbnail) = &on_tap_thumbnail {
                    on_tap_thumbnail(window, cx);
                }
            })
            .child(self.image_view(cx.entity_id()))
            .when(shows_spoiler, |content| {
                content.child(div().absolute().inset_0().child(self.spoiler_view.clone()))
            })
            .when(!self.selection_overlay_is_hidden, |content| {
                content.child(div().absolute().inset_0().bg(black().opacity(0.18)))
            })
            .when(!self.duration_background_is_hidden, |content| content.child(self.duration_background()))
            .when(!self.selection_badge_is_hidden, |content| {
                content.child(
                    div()
                        .id("media-picker-grid-cell-selection")
                        .absolute()
                        .top(px(metrics.corner_inset - touch_outset))
                        .left(px(metrics.corner_inset - touch_outset))
                        .size(px(metrics.badge_diameter + touch_outset * 2.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .block_mouse_except_scroll()
                        .on_click(move |_, window, cx| {
                            if let Some(on_tap_selection) = &on_tap_selection {
                                on_tap_selection(window, cx);
                            }
                        })
                        .child(div().size(px(metrics.badge_diameter)).child(self.selection_badge.clone())),
                )
            })
            .when(!self.edited_indicator_is_hidden, |content| content.child(self.edited_indicator()));

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .opacity(self.alpha)
            .bg(self.background_color)
            .child(content_view)
            .child(div().absolute().inset_0().border_1().border_color(edge))
    }
}

/// sizes of everything drawn over a grid tile, from the tile side
/// denser pinched grid shrinks duration plate, selection circle and edit mark with the tile
/// tiles 96pt and wider keep original sizes, text stops at 9pt
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MediaPickerTileOverlayMetrics {
    pub corner_inset: f32,
    pub badge_diameter: f32,
    pub badge_font_size: f32,
    pub duration_height: f32,
    pub duration_font_size: f32,
    pub duration_padding: f32,
    pub duration_corner_radius: f32,
    pub duration_inset: f32,
    pub shows_video_icon: bool,
    pub video_icon_side: f32,
    pub video_icon_gap: f32,
    pub edited_side: f32,
}

impl MediaPickerTileOverlayMetrics {
    pub const REFERENCE_SIDE: f32 = 96.0;
    pub const MINIMUM_FONT_SIZE: f32 = 9.0;

    const CONSTRAINT_CONSTANTS: Self = Self {
        corner_inset: 6.0,
        badge_diameter: 26.0,
        badge_font_size: 11.0,
        duration_height: 20.0,
        duration_font_size: 11.0,
        duration_padding: 6.0,
        duration_corner_radius: 4.0,
        duration_inset: 4.0,
        shows_video_icon: true,
        video_icon_side: 9.0,
        video_icon_gap: 4.0,
        edited_side: 18.0,
    };

    pub fn new(side: f32) -> Self {
        let scale = (side / Self::REFERENCE_SIDE).clamp(0.0, 1.0);
        let badge_diameter = (side * 0.3).round().clamp(16.0, 26.0);
        Self {
            corner_inset: (side * 0.06).round().clamp(3.0, 6.0),
            badge_diameter,
            badge_font_size: ((badge_diameter * 0.42 * 2.0).round() / 2.0).clamp(Self::MINIMUM_FONT_SIZE, 11.0),
            duration_height: (20.0 * scale).round().max(14.0),
            duration_font_size: ((11.0 * scale * 2.0).round() / 2.0).max(Self::MINIMUM_FONT_SIZE),
            duration_padding: (6.0 * scale).round().max(3.0),
            duration_corner_radius: ((4.0 * scale * 2.0).round() / 2.0).max(2.5),
            duration_inset: (4.0 * scale).round().max(2.0),
            shows_video_icon: scale >= 0.9,
            video_icon_side: (9.0 * scale).round().max(7.0),
            video_icon_gap: (4.0 * scale).round().max(2.0),
            edited_side: (side * 0.18).round().clamp(12.0, 18.0),
        }
    }
}

const SELECTION_CROSS_DISSOLVE_DURATION: f64 = 0.15;

pub struct MediaPickerSelectionBadge {
    is_selected: bool,
    label: String,
    label_font_size: f32,
    circle_fill: Animated<Hsla>,
    circle_stroke: Animated<Hsla>,
    label_opacity: Animated<f32>,
}

impl MediaPickerSelectionBadge {
    pub const TOUCH_OUTSET: f32 = 11.0;

    pub fn new() -> Self {
        let (fill, stroke) = Self::unselected_appearance();
        Self {
            is_selected: false,
            label: String::new(),
            label_font_size: 11.0,
            circle_fill: Animated::new(fill),
            circle_stroke: Animated::new(stroke),
            label_opacity: Animated::new(0.0),
        }
    }

    pub fn is_selected(&self) -> bool {
        self.is_selected
    }

    pub fn set_label_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        if self.label_font_size == size {
            return;
        }
        self.label_font_size = size;
        cx.notify();
    }

    pub fn set_selected(
        &mut self,
        selected: bool,
        number: Option<i64>,
        animated: bool,
        check: bool,
        cx: &mut Context<Self>,
    ) {
        self.is_selected = selected;
        let shown = match (selected, number) {
            (true, Some(number)) => Some(number),
            _ => None,
        };
        let (fill, stroke) = if shown.is_some() { Self::selected_appearance() } else { Self::unselected_appearance() };
        if let Some(number) = shown {
            self.label = if check { "\u{2713}".to_string() } else { number.to_string() };
        }
        let label_opacity = if shown.is_some() { 1.0 } else { 0.0 };
        if animated {
            let now = RiseClock::monotonic();
            let spec = ease_in_out(SELECTION_CROSS_DISSOLVE_DURATION);
            self.circle_fill.set_in(fill, spec, now, cx);
            self.circle_stroke.set_in(stroke, spec, now, cx);
            self.label_opacity.set_in(label_opacity, spec, now, cx);
        } else {
            self.circle_fill.jump(fill);
            self.circle_stroke.jump(stroke);
            self.label_opacity.jump(label_opacity);
        }
        cx.notify();
    }

    fn selected_appearance() -> (Hsla, Hsla) {
        (Rgba { r: 1.0, g: 0.25, b: 0.25, a: 1.0 }.into(), white())
    }

    fn unselected_appearance() -> (Hsla, Hsla) {
        (black().opacity(0.35), white().opacity(0.85))
    }
}

impl Default for MediaPickerSelectionBadge {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for MediaPickerSelectionBadge {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let fill = self.circle_fill.frame_value(now, window);
        let stroke = self.circle_stroke.frame_value(now, window);
        let label_opacity = self.label_opacity.frame_value(now, window);
        let circle = div()
            .absolute()
            .inset(px(1.0))
            .rounded_full()
            .bg(fill)
            .shadow(vec![BoxShadow {
                color: black().opacity(0.35),
                offset: Point::default(),
                blur_radius: px(2.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .child(centred_stroke(stroke, 1.5, Shape::Circle));
        div()
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(circle)
            .child(
                div()
                    .relative()
                    .opacity(label_opacity)
                    .text_size(px(self.label_font_size))
                    .font_weight(FontWeight::BOLD)
                    .text_color(white())
                    .child(SharedString::from(self.label.clone())),
            )
    }
}
