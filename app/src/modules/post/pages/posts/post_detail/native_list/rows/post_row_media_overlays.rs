use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use gpui::{
    App, BoxShadow, ClickEvent, ElementId, FontWeight, MouseButton, MouseDownEvent, Pixels, Point,
    RenderImage, SharedString, Size, Window, black, hsla, point, white,
};
use image::imageops::{FilterType, resize};
use image::{Frame, RgbaImage};
use rise_core::foundation::ns_cache::NSCache;
use rise_i18n::tr;
use rise_platform::context_click::context_click::is_context_click_here;
use rise_theme::alpha;
use rise_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::swiftui::blur::gaussian_blur_render_image;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use smallvec::{SmallVec, smallvec};

use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageDiskCache};
use crate::core::ui::rich_text::post_rich_text_render::PostRichTextRender;
use crate::modules::media_picker::editor::editor_music_plate::{EditorMusicPlatePreview, EditorMusicPlateRenderer};
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostListCachePurge, PostRowMusicPlateLayout, PostRowSeedTrack,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_layout_calculator::PostRowMetrics;

/// media box overlay sizes
pub enum PostRowMediaMetrics {}

impl PostRowMediaMetrics {
    pub const OVERLAY_INSET: f32 = 12.0;

    pub const SOUND_BUTTON_CIRCLE: f32 = 34.0;
    pub const SOUND_BUTTON_TAP: f32 = 48.0;
    pub const SOUND_BUTTON_GLYPH_POINT_SIZE: f32 = 14.0;
    pub const SOUND_BUTTON_FILL_ALPHA: f32 = 0.45;

    pub const PLAY_GLYPH_CIRCLE: f32 = 54.0;
    pub const PLAY_GLYPH_POINT_SIZE: f32 = 22.0;
    pub const PLAY_GLYPH_FILL_ALPHA: f32 = 0.45;
    pub const PLAY_GLYPH_SHADOW_ALPHA: f32 = 0.28;
    pub const PLAY_GLYPH_SHADOW_RADIUS: f32 = 6.0;
    pub const PLAY_GLYPH_SHADOW_Y: f32 = 2.0;

    pub const CHIP_SIZE: f32 = 28.0;
    pub const CHIP_GLYPH_POINT_SIZE: f32 = 14.0;
    pub const CHIP_SHADOW_ALPHA: f32 = 0.28;
    pub const CHIP_SHADOW_RADIUS: f32 = 6.0;
    pub const CHIP_SHADOW_Y: f32 = 2.0;
    pub const CHIP_LONG_PRESS_DURATION: f64 = 0.4;
    pub const CHIP_STROKE_TOP_ALPHA: f32 = 0.14;
    pub const CHIP_STROKE_BOTTOM_ALPHA: f32 = 0.06;
    pub const CORNER_ROW_SPACING: f32 = 10.0;

    pub const PAGE_DOT_CURRENT_ALPHA: f32 = 0.95;
    pub const PAGE_DOT_OTHER_ALPHA: f32 = 0.45;
    pub const PAGE_DOTS_CAPSULE_ALPHA: f32 = 0.28;
    pub const PAGE_COUNTER_CAPSULE_ALPHA: f32 = 0.45;

    pub const BLUR_DECODE_PIXEL_DIMENSION: i64 = 96;
    pub const BLUR_RADIUS: f32 = 18.0;
    pub const BLUR_OVERSCAN: f32 = 1.14;
    pub const BLUR_DIM: f32 = 0.22;

    pub const PLATE_SHADOW_ALPHA: f32 = 0.22;
    pub const PLATE_SHADOW_RADIUS: f32 = 16.0;
    pub const PLATE_SHADOW_Y: f32 = 6.0;

    pub const LOADING_INDICATOR_SCALE: f32 = 0.85;
    pub const FAILURE_GLYPH_ALPHA: f32 = 0.6;
    pub const TAGS_FADE: f64 = 0.22;
    pub const TAGS_UNMOUNT_DELAY: f64 = 0.3;
}

/// layer shadow radius is a gaussian sigma scale so the css blur radius is twice it
fn layer_shadow(alpha_value: f32, radius: f32, y: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: hsla(0.0, 0.0, 0.0, alpha_value),
        offset: point(px(0.0), px(y)),
        blur_radius: px(radius * 2.0),
        spread_radius: px(0.0),
        inset: false,
    }]
}

#[derive(IntoElement, Default)]
pub struct PostRowPassthroughView {
    children: Vec<AnyElement>,
}

impl PostRowPassthroughView {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ParentElement for PostRowPassthroughView {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for PostRowPassthroughView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        pass_through().absolute().inset_0().children(self.children)
    }
}

/// feed sound button, 34 pt circle in a 48 pt tap frame
/// router toggles the store and the glyph follows feed_audio_muted on the next configure
#[derive(IntoElement)]
pub struct PostRowSoundButton {
    id: ElementId,
    muted_state: Option<bool>,
    on_tap: Option<Rc<dyn Fn(&mut App)>>,
}

impl PostRowSoundButton {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), muted_state: Some(true), on_tap: None }
    }

    pub fn on_tap(mut self, on_tap: impl Fn(&mut App) + 'static) -> Self {
        self.on_tap = Some(Rc::new(on_tap));
        self
    }

    pub fn is_muted(&self) -> bool {
        self.muted_state.unwrap_or(true)
    }

    pub fn set_muted(mut self, muted: bool) -> Self {
        self.muted_state = Some(muted);
        self
    }
}

impl RenderOnce for PostRowSoundButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let is_muted = self.is_muted();
        let glyph = if is_muted { "speaker.slash.fill" } else { "speaker.wave.2.fill" };
        let label = SharedString::from(tr(if is_muted { "feed_sound_unmute" } else { "feed_sound_mute" }));
        let on_tap = self.on_tap;
        let action: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| {
            if let Some(on_tap) = on_tap.as_ref() {
                on_tap(cx);
            }
        });
        button_chrome(div().id(self.id), false, Some(action))
            .block_mouse_except_scroll()
            .size(px(PostRowMediaMetrics::SOUND_BUTTON_TAP))
            .flex()
            .items_center()
            .justify_center()
            .active(|style| style.opacity(0.7))
            .aria_label(label.clone())
            .tooltip_label(label)
            .child(
                pass_through()
                    .size(px(PostRowMediaMetrics::SOUND_BUTTON_CIRCLE))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(black().opacity(PostRowMediaMetrics::SOUND_BUTTON_FILL_ALPHA))
                    .children(IconUi::sized(glyph, px(PostRowMediaMetrics::SOUND_BUTTON_GLYPH_POINT_SIZE), white(), !is_muted)),
            )
    }
}

/// play glyph drawn while the video page leaf is not mounted
#[derive(IntoElement, Default)]
pub struct PostRowPlayGlyphView;

impl PostRowPlayGlyphView {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for PostRowPlayGlyphView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        pass_through().absolute().inset_0().flex().items_center().justify_center().child(
            pass_through()
                .size(px(PostRowMediaMetrics::PLAY_GLYPH_CIRCLE))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(black().opacity(PostRowMediaMetrics::PLAY_GLYPH_FILL_ALPHA))
                .shadow(layer_shadow(
                    PostRowMediaMetrics::PLAY_GLYPH_SHADOW_ALPHA,
                    PostRowMediaMetrics::PLAY_GLYPH_SHADOW_RADIUS,
                    PostRowMediaMetrics::PLAY_GLYPH_SHADOW_Y,
                ))
                .children(IconUi::sized("play.fill", px(PostRowMediaMetrics::PLAY_GLYPH_POINT_SIZE), white(), true)),
        )
    }
}

/// page dots up to PAGE_DOT_LIMIT then an n/N counter
/// the capsule hugs the current text inside the frame the parent gives it
#[derive(IntoElement, Default)]
pub struct PostRowPageIndicatorView {
    count: i64,
    current: i64,
    is_bold_text_enabled: bool,
}

impl PostRowPageIndicatorView {
    pub fn new() -> Self {
        Self::default()
    }

    fn uses_dots(&self) -> bool {
        self.count <= PostRowMetrics::PAGE_DOT_LIMIT as i64
    }

    pub fn configure(mut self, count: i64, current: i64, is_bold_text_enabled: bool) -> Self {
        self.count = count.max(0);
        self.is_bold_text_enabled = is_bold_text_enabled;
        self.set_current(current)
    }

    pub fn set_current(mut self, index: i64) -> Self {
        self.current = if self.count > 0 { index.max(0).min(self.count - 1) } else { 0 };
        self
    }
}

impl RenderOnce for PostRowPageIndicatorView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        if self.uses_dots() {
            let current = self.current;
            pass_through()
                .flex()
                .flex_row()
                .gap(px(PostRowMetrics::PAGE_DOT_SPACING))
                .px(px(PostRowMetrics::PAGE_DOTS_HORIZONTAL_INSET))
                .py(px(PostRowMetrics::PAGE_DOTS_VERTICAL_INSET))
                .children((0..self.count).map(move |index| {
                    let dot_alpha = if index == current {
                        PostRowMediaMetrics::PAGE_DOT_CURRENT_ALPHA
                    } else {
                        PostRowMediaMetrics::PAGE_DOT_OTHER_ALPHA
                    };
                    pass_through().size(px(PostRowMetrics::PAGE_DOT_SIZE)).rounded_full().bg(alpha(white(), dot_alpha))
                }))
                .rounded_full()
                .bg(alpha(black(), PostRowMediaMetrics::PAGE_DOTS_CAPSULE_ALPHA))
        } else {
            let weight = PostRichTextRender::legibility_weight(FontWeight::SEMIBOLD, self.is_bold_text_enabled);
            let font = Font::system(PostRowMetrics::PAGE_COUNTER_FONT_SIZE, weight, FontDesign::Rounded).monospaced_digit();
            pass_through().size_full().flex().justify_center().child(
                pass_through()
                    .h_full()
                    .max_w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(PostRowMetrics::PAGE_COUNTER_HORIZONTAL_INSET))
                    .overflow_hidden()
                    .rounded_full()
                    .bg(alpha(black(), PostRowMediaMetrics::PAGE_COUNTER_CAPSULE_ALPHA))
                    .child(
                        div()
                            .swiftui_font(font)
                            .text_color(white())
                            .whitespace_nowrap()
                            .child(format!("{}/{}", self.current + 1, self.count)),
                    ),
            )
        }
    }
}

/// tags chip, glass circle with a person glyph
/// tap and hold go to closures
#[derive(IntoElement)]
pub struct PostRowTagsChipView {
    id: ElementId,
    symbol: Option<SharedString>,
    accessibility_label: Option<SharedString>,
    on_tap: Option<Rc<dyn Fn(&mut App)>>,
    on_long_press: Option<Rc<dyn Fn(&mut App)>>,
}

impl PostRowTagsChipView {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), symbol: None, accessibility_label: None, on_tap: None, on_long_press: None }
    }

    pub fn set_symbol(mut self, name: impl Into<SharedString>) -> Self {
        self.symbol = Some(name.into());
        self
    }

    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }

    pub fn on_tap(mut self, on_tap: impl Fn(&mut App) + 'static) -> Self {
        self.on_tap = Some(Rc::new(on_tap));
        self
    }

    pub fn on_long_press(mut self, on_long_press: impl Fn(&mut App) + 'static) -> Self {
        self.on_long_press = Some(Rc::new(on_long_press));
        self
    }
}

impl RenderOnce for PostRowTagsChipView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let glyph = match self.symbol.as_ref() {
            Some(symbol) => IconUi::sized(symbol, px(16.0), white(), symbol.ends_with(".fill")),
            None => IconUi::sized("person.crop.circle", px(PostRowMediaMetrics::CHIP_GLYPH_POINT_SIZE), white(), false),
        };
        let face = div()
            .size(px(PostRowMediaMetrics::CHIP_SIZE))
            .flex()
            .items_center()
            .justify_center()
            .children(glyph)
            .liquid_chrome_circle(None);
        let on_tap = self.on_tap;
        let right_press = self.on_long_press.clone();
        let control_press = self.on_long_press;
        let label = self.accessibility_label.unwrap_or_else(|| SharedString::from(tr("post_tagged_people_button_hint")));
        button_chrome(div().id(self.id), false, None)
            .block_mouse_except_scroll()
            .size(px(PostRowMediaMetrics::CHIP_SIZE))
            .rounded_full()
            .shadow(layer_shadow(
                PostRowMediaMetrics::CHIP_SHADOW_ALPHA,
                PostRowMediaMetrics::CHIP_SHADOW_RADIUS,
                PostRowMediaMetrics::CHIP_SHADOW_Y,
            ))
            .aria_label(label.clone())
            .tooltip_label(label)
            .on_click(move |event: &ClickEvent, _, cx| {
                if is_context_click_here(MouseButton::Left, &event.modifiers()) {
                    return;
                }
                if let Some(on_tap) = on_tap.as_ref() {
                    on_tap(cx);
                }
            })
            .on_mouse_down(MouseButton::Right, move |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                if let Some(on_long_press) = right_press.as_ref() {
                    on_long_press(cx);
                }
            })
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                if !is_context_click_here(MouseButton::Left, &event.modifiers) {
                    return;
                }
                cx.stop_propagation();
                if let Some(on_long_press) = control_press.as_ref() {
                    on_long_press(cx);
                }
            })
            .child(face)
    }
}

/// music plate over the whole box
/// card with the stored center, scale and rotation
/// not interactive and clipped to the box
#[derive(IntoElement)]
pub struct PostRowMusicPlateView {
    id: SharedString,
    plate: Option<(PostRowSeedTrack, PostRowMusicPlateLayout)>,
    is_visible_in_viewport: bool,
}

impl PostRowMusicPlateView {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self { id: id.into(), plate: None, is_visible_in_viewport: true }
    }

    pub fn configure(mut self, track: &PostRowSeedTrack, plate: &PostRowMusicPlateLayout) -> Self {
        self.plate = Some((track.clone(), plate.clone()));
        self
    }

    /// outside the viewport the plate equalizer shows nothing and dropping it skips its render
    pub fn set_visible_in_viewport(mut self, is_visible: bool) -> Self {
        self.is_visible_in_viewport = is_visible;
        self
    }

    pub fn contains_plate(track: &PostRowSeedTrack, plate: &PostRowMusicPlateLayout, at: Point<Pixels>) -> bool {
        if plate.scale <= 0.0 {
            return false;
        }
        let card = EditorMusicPlateRenderer::metrics(&track.title, &track.artist, plate.base_width).card_size;
        let dx = f32::from(at.x - plate.center.x);
        let dy = f32::from(at.y - plate.center.y);
        let (sin, cos) = plate.rotation.sin_cos();
        let local_x = (dx * cos + dy * sin) / plate.scale;
        let local_y = (dy * cos - dx * sin) / plate.scale;
        local_x.abs() <= card.width / 2.0 && local_y.abs() <= card.height / 2.0
    }

    pub fn reset(mut self) -> Self {
        self.plate = None;
        self
    }
}

impl RenderOnce for PostRowMusicPlateView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let plate = self.plate.filter(|_| self.is_visible_in_viewport).map(|(track, plate)| {
            let card = EditorMusicPlateRenderer::metrics(&track.title, &track.artist, plate.base_width).card_size;
            pass_through()
                .absolute()
                .left(plate.center.x - px(card.width * plate.scale / 2.0))
                .top(plate.center.y - px(card.height * plate.scale / 2.0))
                .child(
                    EditorMusicPlatePreview::new(
                        self.id,
                        track.title,
                        track.artist,
                        track.cover_url,
                        track.id,
                        plate.is_dark,
                    )
                    .base_width(plate.base_width)
                    .transform(plate.scale, plate.rotation),
                )
        });
        pass_through().absolute().inset_0().overflow_hidden().children(plate)
    }
}

/// blur fill of a fit page, rendered once off main from a small decode
pub enum PostRowBackdropRenderer {}

impl PostRowBackdropRenderer {
    pub fn render(image: &RenderImage, display_size: Size<f32>, maximum_pixel_dimension: i64) -> Option<CleverImage> {
        if display_size.width <= 0.0 || display_size.height <= 0.0 {
            return None;
        }
        let small = Self::downscaled(image, maximum_pixel_dimension)?;
        let small_size = small.size(0);
        let width = small_size.width.0 as f32;
        let height = small_size.height.0 as f32;
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let points_per_pixel = (display_size.width / width).max(display_size.height / height);
        let sigma = PostRowMediaMetrics::BLUR_RADIUS / points_per_pixel.max(0.001);
        Some(Arc::new(gaussian_blur_render_image(&small, sigma, true)))
    }

    fn downscaled(image: &RenderImage, maximum_pixel_dimension: i64) -> Option<RenderImage> {
        let size = image.size(0);
        let source_width = u32::try_from(size.width.0).ok()?;
        let source_height = u32::try_from(size.height.0).ok()?;
        let longest = source_width.max(source_height);
        if longest == 0 || maximum_pixel_dimension <= 0 {
            return None;
        }
        let scale = (maximum_pixel_dimension as f32 / longest as f32).min(1.0);
        let width = ((source_width as f32 * scale).round() as u32).max(1);
        let height = ((source_height as f32 * scale).round() as u32).max(1);
        let source = RgbaImage::from_raw(source_width, source_height, image.as_bytes(0)?.to_vec())?;
        let scaled = resize(&source, width, height, FilterType::Triangle);
        let frames: SmallVec<[Frame; 1]> = smallvec![Frame::new(scaled)];
        Some(RenderImage::new(frames))
    }
}

#[derive(Clone)]
pub struct PostRowBackdropImage {
    pub image: CleverImage,
}

/// rendered blur fills keyed by account, source and box size
pub struct PostRowBackdropCache {
    cache: NSCache<String, CleverImage>,
}

static POST_ROW_BACKDROP_CACHE: LazyLock<PostRowBackdropCache> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(96);
    cache.set_total_cost_limit(8 * 1_024 * 1_024);
    PostRowBackdropCache { cache }
});

impl PostRowBackdropCache {
    pub fn shared() -> &'static Self {
        &POST_ROW_BACKDROP_CACHE
    }

    pub fn key(url: &str, account_user_id: Option<&str>, account_epoch: Option<&str>, display_size: Size<f32>) -> String {
        [
            account_user_id.unwrap_or("anonymous").to_owned(),
            account_epoch.unwrap_or("none").to_owned(),
            CleverImageDiskCache::normalized_cache_key(url),
            format!("{}x{}", display_size.width.round() as i64, display_size.height.round() as i64),
        ]
        .join("|")
    }

    pub fn image(&self, for_key: &str) -> Option<CleverImage> {
        self.cache.object(&for_key.to_owned())
    }

    pub fn insert(&self, image: CleverImage, for_key: impl Into<String>) {
        let cost = image.as_bytes(0).map_or(0, <[u8]>::len);
        self.cache.set_object_cost(image, for_key.into(), cost);
    }

    pub fn remove_all(&self) {
        self.cache.remove_all_objects();
    }
}

thread_local! {
    static POST_ROW_MEDIA_CACHES_REGISTERED: Cell<bool> = const { Cell::new(false) };
}

/// registers the media package caches with the purge
pub enum PostRowMediaCaches {}

impl PostRowMediaCaches {
    pub fn register_purge_if_needed(cx: &mut App) {
        if POST_ROW_MEDIA_CACHES_REGISTERED.replace(true) {
            return;
        }
        let _ = PostListCachePurge::register(|_| PostRowBackdropCache::shared().remove_all(), cx);
    }
}
