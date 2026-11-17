use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use gpui::{
    App, Bounds, BoxShadow, Context, DispatchPhase, Div, FontWeight, Hitbox, HitboxBehavior, Hsla,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, ParentElement, PathBuilder,
    Pixels, Point, Render, RenderOnce, Rgba, SharedString, Size, Styled, StyledImage, Task, Window,
    canvas, div, hsla, img, point, px,
};
use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_ui::context_menu_ui::ContextMenuItem;
use rise_ui::swiftui::context_menu::ContextMenuModifiers;
use rise_ui::swiftui::font::{Font, NATURAL_LINE_HEIGHT, SwiftUiFont};
use rise_ui::swiftui::lifecycle::on_change;
use rise_ui::swiftui::motion::CubicBezier;
use rise_ui::swiftui::rotation_effect::RotationEffectModifiers;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapLoader, CleverImageDiskPrefetch};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::media_picker::editor::editor_entity_snapping::{
    EditorEntitySideHandle, EditorEntitySnap, EditorEntitySnapFeedback,
};
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use crate::modules::media_picker::editor::editor_state::EditorMusicPlateEntity;
use crate::modules::music::shared::utils::music::music_cover_letterbox::MusicCoverLetterbox;
use crate::modules::music::shared::utils::music::music_media_url::MusicMediaURL;

fn white(level: f32, alpha: f32) -> Hsla {
    hsla(0.0, 0.0, level, alpha)
}

fn turned(vector: Point<f32>, radians: f32) -> Point<f32> {
    let (sine, cosine) = radians.sin_cos();
    point(vector.x * cosine - vector.y * sine, vector.x * sine + vector.y * cosine)
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<f32> {
    Bounds { origin: point(x, y), size: Size { width, height } }
}

fn inset_by(bounds: Bounds<f32>, dx: f32, dy: f32) -> Bounds<f32> {
    rect(
        bounds.origin.x + dx,
        bounds.origin.y + dy,
        bounds.size.width - dx * 2.0,
        bounds.size.height - dy * 2.0,
    )
}

fn contains(bounds: Bounds<f32>, location: Point<f32>) -> bool {
    location.x >= bounds.origin.x
        && location.x <= bounds.origin.x + bounds.size.width
        && location.y >= bounds.origin.y
        && location.y <= bounds.origin.y + bounds.size.height
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorMusicPlateRendererMetrics {
    pub card_size: Size<f32>,
    pub corner_radius: f32,
    pub cover_rect: Bounds<f32>,
    pub cover_corner_radius: f32,
    pub equalizer_rect: Bounds<f32>,
    pub title_font: Font,
    pub artist_font: Font,
    pub text_x: f32,
    pub text_width: f32,
    pub unit: f32,
}

/// one drawing impl for the music plate, used by the canvas view and the previews
/// all metrics derive from base_width so cards stay identical
pub enum EditorMusicPlateRenderer {}

impl EditorMusicPlateRenderer {
    pub const BASE_WIDTH_FRACTION: f32 = 0.54;

    pub fn metrics(title: &str, artist: &str, base_width: f32) -> EditorMusicPlateRendererMetrics {
        let unit = (base_width / 300.0).max(0.2);
        let pad = 14.0 * unit;
        let cover = 50.0 * unit;
        let gap = 11.0 * unit;
        let title_font = Font::system_weight(17.0 * unit, FontWeight::BOLD);
        let artist_font = Font::system_weight(13.5 * unit, FontWeight::NORMAL);

        let max_text_width = base_width - pad * 2.0 - cover - gap;
        let title_width = Self::text_width(title, &title_font);
        let artist_width = Self::text_width(artist, &artist_font);
        let text_width = max_text_width.min((64.0 * unit).max(title_width.max(artist_width)));

        let card_width = pad + cover + gap + text_width + pad;
        let card_height = cover + 22.0 * unit;
        let cover_rect = rect(pad, (card_height - cover) / 2.0, cover, cover);
        let equalizer_side = 20.0 * unit;
        let equalizer_rect = rect(
            pad + cover / 2.0 - equalizer_side / 2.0,
            card_height / 2.0 - equalizer_side / 2.0,
            equalizer_side,
            equalizer_side,
        );
        EditorMusicPlateRendererMetrics {
            card_size: Size { width: card_width, height: card_height },
            corner_radius: 15.0 * unit,
            cover_rect,
            cover_corner_radius: 9.0 * unit,
            equalizer_rect,
            title_font,
            artist_font,
            text_x: pad + cover + gap,
            text_width,
            unit,
        }
    }

    fn text_width(text: &str, font: &Font) -> f32 {
        let Some(text_system) = EditorImageBake::text_system() else {
            return 0.0;
        };
        let resolved = gpui::Font {
            family: font.family(),
            features: font.features(),
            fallbacks: None,
            weight: font.weight,
            style: font.style(),
        };
        let font_id = text_system.resolve_font(&resolved);
        text.chars().map(|ch| f32::from(text_system.layout_width(font_id, px(font.size), ch))).sum()
    }

    pub fn background_color(is_dark: bool) -> Hsla {
        if is_dark { white(0.09, 1.0) } else { white(1.0, 1.0) }
    }

    pub fn title_color(is_dark: bool) -> Hsla {
        if is_dark { white(1.0, 1.0) } else { white(0.05, 1.0) }
    }

    pub fn artist_color(is_dark: bool) -> Hsla {
        if is_dark { white(1.0, 0.62) } else { white(0.0, 0.55) }
    }

    fn line_height(font: &Font) -> f32 {
        font.size * NATURAL_LINE_HEIGHT
    }

    #[allow(clippy::too_many_arguments)]
    fn text_line(text: &str, font: &Font, color: Hsla, x: f32, y: f32, width: f32, height: f32, scale: f32) -> Div {
        div()
            .absolute()
            .left(px(x * scale))
            .top(px(y * scale))
            .w(px(width * scale))
            .h(px(height * scale))
            .overflow_hidden()
            .swiftui_font(Font::system_weight(font.size * scale, font.weight))
            .text_color(color)
            .whitespace_nowrap()
            .text_ellipsis()
            .child(SharedString::from(text.to_owned()))
    }

    /// lays out the card with origin at 0,0
    /// live view keeps the equalizer off since an animated one sits on top, a frozen card turns it on
    /// scale and rotation come from the entity transform
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        title: &str,
        artist: &str,
        _cover_url: &str,
        track_id: &str,
        is_dark: bool,
        base_width: f32,
        include_equalizer: bool,
        scale: f32,
        rotation: f32,
    ) -> Div {
        let m = Self::metrics(title, artist, base_width);
        let cover = EditorMusicPlateCoverStore::shared().image(track_id);
        let cover_rect = m.cover_rect;
        let degrees = rotation.to_degrees();

        let cover_box = div()
            .absolute()
            .left(px(cover_rect.origin.x * scale))
            .top(px(cover_rect.origin.y * scale))
            .w(px(cover_rect.size.width * scale))
            .h(px(cover_rect.size.height * scale))
            .rounded(px(m.cover_corner_radius * scale));
        let mut base = div()
            .absolute()
            .top_0()
            .left_0()
            .w(px(m.card_size.width * scale))
            .h(px(m.card_size.height * scale))
            .rounded(px(m.corner_radius * scale))
            .bg(Self::background_color(is_dark));
        match &cover {
            Some(image) => {
                base = base.child(
                    cover_box.child(
                        img(image.clone())
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                            .rounded(px(m.cover_corner_radius * scale)),
                    ),
                );
            }
            None => {
                let note_size = cover_rect.size.height * 0.44 * scale;
                base = base.child(
                    cover_box
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(white(if is_dark { 0.22 } else { 0.86 }, 1.0))
                        .swiftui_font(Font::system_weight(note_size, FontWeight::SEMIBOLD))
                        .text_color(white(if is_dark { 0.55 } else { 0.45 }, 1.0))
                        .child(SharedString::new_static("\u{266A}")),
                );
            }
        }

        let title_height = Self::line_height(&m.title_font);
        let artist_height = Self::line_height(&m.artist_font);
        let spacing = 3.0 * m.unit;
        let block_height = title_height + spacing + artist_height;
        let text_top = (m.card_size.height - block_height) / 2.0;
        base = base
            .child(Self::text_line(
                title,
                &m.title_font,
                Self::title_color(is_dark),
                m.text_x,
                text_top,
                m.text_width,
                title_height.ceil(),
                scale,
            ))
            .child(Self::text_line(
                artist,
                &m.artist_font,
                Self::artist_color(is_dark),
                m.text_x,
                text_top + title_height + spacing,
                m.text_width,
                artist_height.ceil(),
                scale,
            ));

        let mut card = div()
            .relative()
            .w(px(m.card_size.width * scale))
            .h(px(m.card_size.height * scale))
            .child(base.rotation_effect(degrees));
        if include_equalizer {
            card = card.child(Self::static_equalizer(&m, scale).rotation_effect(degrees));
        }
        card
    }

    fn static_equalizer(m: &EditorMusicPlateRendererMetrics, scale: f32) -> Div {
        let bar_rect = m.equalizer_rect;
        let bar_width = bar_rect.size.width * 0.16;
        let gap = (bar_rect.size.width - bar_width * 3.0) / 2.0;
        let mut layer = div()
            .absolute()
            .top_0()
            .left_0()
            .w(px(m.card_size.width * scale))
            .h(px(m.card_size.height * scale))
            .child(
                div()
                    .absolute()
                    .left(px(m.cover_rect.origin.x * scale))
                    .top(px(m.cover_rect.origin.y * scale))
                    .w(px(m.cover_rect.size.width * scale))
                    .h(px(m.cover_rect.size.height * scale))
                    .rounded(px(m.cover_corner_radius * scale))
                    .bg(white(0.0, 0.38)),
            );
        for (index, phase) in [0.55_f32, 0.95, 0.68].into_iter().enumerate() {
            let height = bar_rect.size.height * phase;
            layer = layer.child(
                div()
                    .absolute()
                    .left(px((bar_rect.origin.x + index as f32 * (bar_width + gap)) * scale))
                    .top(px((bar_rect.origin.y + bar_rect.size.height / 2.0 - height / 2.0) * scale))
                    .w(px(bar_width * scale))
                    .h(px(height * scale))
                    .rounded(px(bar_width / 2.0 * scale))
                    .bg(white(1.0, 1.0)),
            );
        }
        layer
    }
}

#[derive(Default)]
struct EditorMusicPlateCoverStoreState {
    images: HashMap<String, CleverImage>,
    in_flight: HashSet<String>,
}

/// cover cache, covers are fetched while the plate is on screen so later readers get them synchronously
pub struct EditorMusicPlateCoverStore {
    state: Mutex<EditorMusicPlateCoverStoreState>,
}

static COVER_STORE: LazyLock<EditorMusicPlateCoverStore> = LazyLock::new(|| EditorMusicPlateCoverStore {
    state: Mutex::new(EditorMusicPlateCoverStoreState::default()),
});

impl EditorMusicPlateCoverStore {
    const RETRY_COUNT: usize = 6;
    const RETRY_INTERVAL: Duration = Duration::from_millis(700);

    pub fn shared() -> &'static Self {
        &COVER_STORE
    }

    pub fn image(&self, track_id: &str) -> Option<CleverImage> {
        self.state.lock().images.get(track_id).cloned()
    }

    pub fn prefetch(
        &'static self,
        track_id: &str,
        cover_url: &str,
        on_loaded: Option<Box<dyn FnOnce(&mut App)>>,
        cx: &mut App,
    ) {
        if track_id.is_empty() {
            return;
        }
        let (already_loaded, already_fetching) = {
            let mut state = self.state.lock();
            let already_loaded = state.images.contains_key(track_id);
            let already_fetching = state.in_flight.contains(track_id);
            if !already_loaded && !already_fetching {
                state.in_flight.insert(track_id.to_owned());
            }
            (already_loaded, already_fetching)
        };
        if already_loaded {
            if let Some(on_loaded) = on_loaded {
                on_loaded(cx);
            }
            return;
        }
        if already_fetching {
            return;
        }

        let candidates = MusicMediaURL::cover_url_strings_for_fetch(Some(cover_url));
        if candidates.is_empty() {
            self.finish_fetch(track_id);
            return;
        }
        let track_id = track_id.to_owned();
        let schedule_candidates = candidates.clone();
        cx.spawn(async move |cx| {
            let mut scheduled = false;
            for attempt in 0..=Self::RETRY_COUNT {
                if attempt > 0 {
                    cx.background_executor().timer(Self::RETRY_INTERVAL).await;
                }
                let read_track_id = track_id.clone();
                let read_candidates = candidates.clone();
                let found = cx
                    .background_executor()
                    .spawn(async move { self.attempt_sync_read(&read_track_id, &read_candidates) })
                    .await;
                if found {
                    self.finish_fetch(&track_id);
                    if let Some(on_loaded) = on_loaded {
                        cx.update(on_loaded);
                    }
                    return;
                }
                if !scheduled {
                    scheduled = true;
                    cx.update(|cx| {
                        CleverImageDiskPrefetch::schedule(
                            &schedule_candidates,
                            None,
                            true,
                            CleverImageCachePolicy::Standard,
                            cx,
                        );
                    });
                }
            }
            self.finish_fetch(&track_id);
        })
        .detach();
    }

    fn attempt_sync_read(&self, track_id: &str, candidates: &[String]) -> bool {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        for url in candidates {
            if let Some(image) = CleverImageBitmapLoader::image_from_memory_or_disk(
                url,
                true,
                account_user_id.as_deref(),
                account_epoch.as_deref(),
                CleverImageCachePolicy::Standard,
                None,
            ) {
                let trimmed = MusicCoverLetterbox::trimmed_cache_key(&image, url);
                let square = Self::square_cropped(&trimmed);
                self.state.lock().images.insert(track_id.to_owned(), square);
                return true;
            }
        }
        false
    }

    fn square_cropped(cover: &CleverImage) -> CleverImage {
        if cover.frame_count() != 1 {
            return cover.clone();
        }
        let size = cover.size(0);
        let (Ok(width), Ok(height)) = (u32::try_from(size.width.0), u32::try_from(size.height.0)) else {
            return cover.clone();
        };
        if width == height || width < 2 || height < 2 {
            return cover.clone();
        }
        let Some(bytes) = cover.as_bytes(0) else { return cover.clone() };
        let Some(source) = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(width, height, bytes) else {
            return cover.clone();
        };
        let side = width.min(height);
        let left = (width - side) / 2;
        let top = (height - side) / 2;
        let cropped = image::ImageBuffer::from_fn(side, side, |x, y| *source.get_pixel(x + left, y + top));
        let frame = image::Frame::from_parts(cropped, 0, 0, cover.delay(0));
        std::sync::Arc::new(gpui::RenderImage::new(vec![frame]))
    }

    fn finish_fetch(&self, track_id: &str) {
        self.state.lock().in_flight.remove(track_id);
    }
}

#[derive(IntoElement)]
pub struct EditorMusicEqualizerView {
    id: SharedString,
    size: Size<f32>,
}

impl EditorMusicEqualizerView {
    const DURATION: f64 = 0.42;
    const STAGGER: f64 = 0.14;

    pub fn new(id: impl Into<SharedString>, size: Size<f32>) -> Self {
        Self { id: id.into(), size }
    }

    /// bar height from elapsed time, autoreversing and staggered by bar index
    /// before its start time a bar rests at its own height
    fn bar_height(height: f32, elapsed: f64, index: usize) -> f32 {
        let local = elapsed - index as f64 * Self::STAGGER;
        if local < 0.0 {
            return height * 0.6;
        }
        let cycle = (local / Self::DURATION).rem_euclid(2.0);
        let forward = if cycle <= 1.0 { cycle } else { 2.0 - cycle };
        let eased = CubicBezier::EASE_IN_OUT.progress(forward) as f32;
        height * 0.28 + (height * 0.95 - height * 0.28) * eased
    }
}

impl RenderOnce for EditorMusicEqualizerView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let base = window.use_keyed_state(self.id.clone(), cx, |_, _| RiseClock::monotonic());
        let elapsed = RiseClock::monotonic().saturating_sub(*base.read(cx)).as_secs_f64();
        if self.size.height > 1.0 {
            window.request_animation_frame();
        }
        let bar_width = self.size.width * 0.16;
        let gap = (self.size.width - bar_width * 3.0) / 2.0;
        let bars = (0..3).map(|index| {
            let height = Self::bar_height(self.size.height, elapsed, index);
            div()
                .absolute()
                .left(px(index as f32 * (bar_width + gap)))
                .top(px(self.size.height / 2.0 - height / 2.0))
                .w(px(bar_width))
                .h(px(height))
                .rounded(px(bar_width / 2.0))
                .bg(white(1.0, 1.0))
        });
        div().relative().w(px(self.size.width)).h(px(self.size.height)).children(bars)
    }
}

/// draws the static card with the shared renderer and puts the animated equalizer over the cover
#[derive(IntoElement)]
pub struct EditorMusicPlateCardView {
    id: SharedString,
    title: String,
    artist: String,
    cover_url: String,
    track_id: String,
    is_dark: bool,
    base_width: f32,
    scale: f32,
    rotation: f32,
}

impl EditorMusicPlateCardView {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: String::new(),
            artist: String::new(),
            cover_url: String::new(),
            track_id: String::new(),
            is_dark: false,
            base_width: 300.0,
            scale: 1.0,
            rotation: 0.0,
        }
    }

    pub fn configure(
        mut self,
        title: impl Into<String>,
        artist: impl Into<String>,
        cover_url: impl Into<String>,
        track_id: impl Into<String>,
        is_dark: bool,
        base_width: f32,
    ) -> Self {
        self.title = title.into();
        self.artist = artist.into();
        self.cover_url = cover_url.into();
        self.track_id = track_id.into();
        self.is_dark = is_dark;
        self.base_width = base_width;
        self
    }

    pub fn transform(mut self, scale: f32, rotation: f32) -> Self {
        self.scale = scale;
        self.rotation = rotation;
        self
    }

    pub fn card_size(&self) -> Size<f32> {
        EditorMusicPlateRenderer::metrics(&self.title, &self.artist, self.base_width).card_size
    }
}

impl RenderOnce for EditorMusicPlateCardView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view = window.current_view();
        let track_id = self.track_id.clone();
        let cover_url = self.cover_url.clone();
        on_change(
            SharedString::from(format!("{}-prefetch", self.id)),
            (self.track_id.clone(), self.cover_url.clone()),
            true,
            window,
            cx,
            move |_, _, _, cx| {
                EditorMusicPlateCoverStore::shared().prefetch(
                    &track_id,
                    &cover_url,
                    Some(Box::new(move |cx: &mut App| cx.notify(view))),
                    cx,
                );
            },
        );

        let scale = self.scale;
        let m = EditorMusicPlateRenderer::metrics(&self.title, &self.artist, self.base_width);
        let card = EditorMusicPlateRenderer::draw(
            &self.title,
            &self.artist,
            &self.cover_url,
            &self.track_id,
            self.is_dark,
            self.base_width,
            false,
            scale,
            self.rotation,
        );
        let cover_dim = div()
            .absolute()
            .left(px(m.cover_rect.origin.x * scale))
            .top(px(m.cover_rect.origin.y * scale))
            .w(px(m.cover_rect.size.width * scale))
            .h(px(m.cover_rect.size.height * scale))
            .rounded(px(m.cover_corner_radius * scale))
            .bg(white(0.0, 0.38));
        let equalizer = div()
            .absolute()
            .left(px(m.equalizer_rect.origin.x * scale))
            .top(px(m.equalizer_rect.origin.y * scale))
            .child(EditorMusicEqualizerView::new(
                SharedString::from(format!("{}-equalizer", self.id)),
                Size { width: m.equalizer_rect.size.width * scale, height: m.equalizer_rect.size.height * scale },
            ));
        card.child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .w(px(m.card_size.width * scale))
                .h(px(m.card_size.height * scale))
                .child(cover_dim)
                .child(equalizer)
                .rotation_effect(self.rotation.to_degrees()),
        )
    }
}

pub enum EditorMusicPlateEntityViewMetrics {}

impl EditorMusicPlateEntityViewMetrics {
    pub const SELECTION_PADDING: f32 = 8.0;
    pub const HANDLE_VISUAL_SIZE: f32 = 12.0;
    pub const HANDLE_HIT_OUTSET: f32 = 22.0;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditorMusicPlateDragKind {
    Body,
    LeftSide,
    RightSide,
}

struct EditorMusicPlatePress {
    kind: Option<EditorMusicPlateDragKind>,
    pressed_at: Point<Pixels>,
    began: bool,
}

/// music plate on the editor canvas, drag moves it
/// side handles scale and rotate
/// click flips light or dark, right click deletes
pub struct EditorMusicPlateEntityView {
    pub entity_id: Uuid,
    current: Option<EditorMusicPlateEntity>,
    canvas_size: Size<f32>,
    is_selected: bool,
    handles_visible: bool,
    allows_body_drag: bool,
    display_zoom: f32,

    base_width: f32,
    bounds_size: Size<f32>,
    center: Point<f32>,
    move_start_center: Point<f32>,
    side_start_scale: f32,
    side_start_rotation: f32,
    side_start_handle_offset: Point<f32>,
    side_start_center: Point<f32>,
    is_side_handle_dragging: bool,

    press: Option<EditorMusicPlatePress>,
    pending_tap: Option<Task<()>>,

    pub on_update: Option<Rc<dyn Fn(EditorMusicPlateEntity, &mut App)>>,
    pub on_select: Option<Rc<dyn Fn(Uuid, &mut App)>>,
    pub on_request_delete: Option<Rc<dyn Fn(Uuid, &mut App)>>,
    pub on_snap_guides_changed: Option<Rc<dyn Fn(bool, bool, &mut App)>>,
    pub on_press_changed: Option<Rc<dyn Fn(bool, &mut App)>>,

    snap_feedback: EditorEntitySnapFeedback,
}

impl EditorMusicPlateEntityView {
    const PAN_SLOP: f32 = 3.0;
    const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(300);
    const CORNER_SEGMENTS: usize = 8;

    pub fn new(entity_id: Uuid, _cx: &mut Context<Self>) -> Self {
        Self {
            entity_id,
            current: None,
            canvas_size: Size { width: 0.0, height: 0.0 },
            is_selected: false,
            handles_visible: false,
            allows_body_drag: true,
            display_zoom: 1.0,
            base_width: 300.0,
            bounds_size: Size { width: 0.0, height: 0.0 },
            center: point(0.0, 0.0),
            move_start_center: point(0.0, 0.0),
            side_start_scale: 1.0,
            side_start_rotation: 0.0,
            side_start_handle_offset: point(0.0, 0.0),
            side_start_center: point(0.0, 0.0),
            is_side_handle_dragging: false,
            press: None,
            pending_tap: None,
            on_update: None,
            on_select: None,
            on_request_delete: None,
            on_snap_guides_changed: None,
            on_press_changed: None,
            snap_feedback: EditorEntitySnapFeedback::new(),
        }
    }

    pub fn set_interaction_state(&mut self, handles_visible: bool, allows_body_drag: bool, cx: &mut Context<Self>) {
        if self.handles_visible == handles_visible && self.allows_body_drag == allows_body_drag {
            return;
        }
        self.handles_visible = handles_visible;
        self.allows_body_drag = allows_body_drag;
        if !handles_visible {
            self.is_side_handle_dragging = false;
        }
        cx.notify();
    }

    pub fn set_display_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        if self.display_zoom != zoom {
            self.display_zoom = zoom;
            cx.notify();
        }
    }

    pub fn apply(
        &mut self,
        entity: &EditorMusicPlateEntity,
        canvas_size: Size<f32>,
        is_selected: bool,
        cx: &mut Context<Self>,
    ) {
        if self.current.as_ref() == Some(entity) && self.canvas_size == canvas_size && self.is_selected == is_selected
        {
            return;
        }
        self.canvas_size = canvas_size;
        self.is_selected = is_selected;

        let short_side = canvas_size.width.min(canvas_size.height).max(1.0);
        self.base_width = EditorMusicPlateRenderer::BASE_WIDTH_FRACTION * short_side;
        self.bounds_size =
            EditorMusicPlateRenderer::metrics(&entity.title, &entity.artist, self.base_width).card_size;
        self.center = point(entity.center.x * canvas_size.width, entity.center.y * canvas_size.height);
        self.current = Some(entity.clone());
        cx.notify();
    }

    fn selection_rect(&self) -> Bounds<f32> {
        let pad = EditorMusicPlateEntityViewMetrics::SELECTION_PADDING * self.display_zoom;
        rect(-pad, -pad, self.bounds_size.width + pad * 2.0, self.bounds_size.height + pad * 2.0)
    }

    fn left_handle_rect(&self) -> Bounds<f32> {
        let selection = self.selection_rect();
        let dot = EditorMusicPlateEntityViewMetrics::HANDLE_VISUAL_SIZE * self.display_zoom;
        rect(
            selection.origin.x - dot / 2.0,
            selection.origin.y + selection.size.height / 2.0 - dot / 2.0,
            dot,
            dot,
        )
    }

    fn right_handle_rect(&self) -> Bounds<f32> {
        let selection = self.selection_rect();
        let dot = EditorMusicPlateEntityViewMetrics::HANDLE_VISUAL_SIZE * self.display_zoom;
        rect(
            selection.origin.x + selection.size.width - dot / 2.0,
            selection.origin.y + selection.size.height / 2.0 - dot / 2.0,
            dot,
            dot,
        )
    }

    fn handle_hit_rect(&self, handle: Bounds<f32>) -> Bounds<f32> {
        let outset = EditorMusicPlateEntityViewMetrics::HANDLE_HIT_OUTSET * self.display_zoom;
        inset_by(handle, -outset, -outset)
    }

    fn point_inside(&self, location: Point<f32>) -> bool {
        if self.handles_visible
            && (contains(self.handle_hit_rect(self.left_handle_rect()), location)
                || contains(self.handle_hit_rect(self.right_handle_rect()), location))
        {
            return true;
        }
        contains(self.selection_rect(), location)
    }

    pub fn contains_canvas_point(&self, location: Point<f32>) -> bool {
        let Some(entity) = self.current.as_ref() else { return false };
        let vector = point(location.x - self.center.x, location.y - self.center.y);
        let unturned = turned(vector, -entity.rotation);
        let scale = entity.scale.abs().max(f32::EPSILON);
        self.point_inside(point(
            unturned.x / scale + self.bounds_size.width / 2.0,
            unturned.y / scale + self.bounds_size.height / 2.0,
        ))
    }

    fn location_in_self(&self, position: Point<Pixels>, plate_center: Point<Pixels>, entity: &EditorMusicPlateEntity) -> Point<f32> {
        let vector = point(f32::from(position.x - plate_center.x), f32::from(position.y - plate_center.y));
        let unturned = turned(vector, -entity.rotation);
        let scale = entity.scale.abs().max(f32::EPSILON);
        point(
            unturned.x / scale + self.bounds_size.width / 2.0,
            unturned.y / scale + self.bounds_size.height / 2.0,
        )
    }

    fn fire_select(&self, cx: &mut Context<Self>) {
        if let Some(on_select) = self.on_select.clone() {
            let entity_id = self.entity_id;
            cx.defer(move |cx| on_select(entity_id, cx));
        }
    }

    fn fire_press_changed(&self, pressed: bool, cx: &mut Context<Self>) {
        if let Some(on_press_changed) = self.on_press_changed.clone() {
            cx.defer(move |cx| on_press_changed(pressed, cx));
        }
    }

    fn fire_update(&self, entity: EditorMusicPlateEntity, cx: &mut Context<Self>) {
        if let Some(on_update) = self.on_update.clone() {
            cx.defer(move |cx| on_update(entity, cx));
        }
    }

    fn fire_snap_guides(&self, snapped_x: bool, snapped_y: bool, cx: &mut Context<Self>) {
        if let Some(on_snap_guides_changed) = self.on_snap_guides_changed.clone() {
            cx.defer(move |cx| on_snap_guides_changed(snapped_x, snapped_y, cx));
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, plate_center: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(entity) = self.current.as_ref() else { return };
        let location = self.location_in_self(event.position, plate_center, entity);
        if !self.point_inside(location) {
            return;
        }
        cx.stop_propagation();
        self.pending_tap = None;
        if event.click_count >= 2 {
            self.handle_double_tap(cx);
            return;
        }
        let kind = if self.handles_visible && contains(self.right_handle_rect(), location) {
            Some(EditorMusicPlateDragKind::RightSide)
        } else if self.handles_visible && contains(self.left_handle_rect(), location) {
            Some(EditorMusicPlateDragKind::LeftSide)
        } else if self.handles_visible
            && (contains(self.handle_hit_rect(self.left_handle_rect()), location)
                || contains(self.handle_hit_rect(self.right_handle_rect()), location))
        {
            None
        } else {
            Some(EditorMusicPlateDragKind::Body)
        };
        self.press = Some(EditorMusicPlatePress { kind, pressed_at: event.position, began: false });
        self.fire_press_changed(true, cx);
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.press.is_none() {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.finish_press(false, cx);
            return;
        }
        let Some(press) = self.press.as_mut() else { return };
        let translation = point(
            f32::from(event.position.x - press.pressed_at.x),
            f32::from(event.position.y - press.pressed_at.y),
        );
        let is_first_change = !press.began;
        if is_first_change && translation.x.hypot(translation.y) < Self::PAN_SLOP {
            return;
        }
        press.began = true;
        let kind = press.kind;
        match kind {
            Some(EditorMusicPlateDragKind::Body) => {
                if is_first_change {
                    self.begin_pan(cx);
                }
                self.change_pan(translation, cx);
            }
            Some(EditorMusicPlateDragKind::LeftSide) => {
                if is_first_change {
                    self.begin_side_handle(-1.0, cx);
                }
                self.change_side_handle(translation, cx);
            }
            Some(EditorMusicPlateDragKind::RightSide) => {
                if is_first_change {
                    self.begin_side_handle(1.0, cx);
                }
                self.change_side_handle(translation, cx);
            }
            None => {}
        }
    }

    fn mouse_up(&mut self, cx: &mut Context<Self>) {
        self.finish_press(true, cx);
    }

    fn finish_press(&mut self, may_tap: bool, cx: &mut Context<Self>) {
        let Some(press) = self.press.take() else { return };
        self.fire_press_changed(false, cx);
        if press.began {
            match press.kind {
                Some(EditorMusicPlateDragKind::Body) => self.fire_snap_guides(false, false, cx),
                Some(EditorMusicPlateDragKind::LeftSide | EditorMusicPlateDragKind::RightSide) => {
                    self.end_side_handle(cx);
                }
                None => {}
            }
            return;
        }
        if may_tap {
            self.pending_tap = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Self::DOUBLE_TAP_WINDOW).await;
                let _ = this.update(cx, |this, cx| this.handle_tap(cx));
            }));
        }
    }

    fn handle_tap(&mut self, cx: &mut Context<Self>) {
        self.pending_tap = None;
        if !self.handles_visible {
            self.fire_select(cx);
            return;
        }
        self.fire_select(cx);
        let Some(entity) = self.current.as_mut() else { return };
        entity.is_dark = !entity.is_dark;
        let entity = entity.clone();
        self.fire_update(entity, cx);
        cx.notify();
    }

    fn handle_double_tap(&mut self, cx: &mut Context<Self>) {
        self.fire_select(cx);
    }

    fn begin_pan(&mut self, cx: &mut Context<Self>) {
        if !self.allows_body_drag || self.is_side_handle_dragging || self.current.is_none() {
            return;
        }
        self.move_start_center = self.center;
        self.snap_feedback.begin();
        self.fire_select(cx);
    }

    fn change_pan(&mut self, translation: Point<f32>, cx: &mut Context<Self>) {
        if !self.allows_body_drag || self.is_side_handle_dragging {
            return;
        }
        let Some(mut entity) = self.current.clone() else { return };
        let proposed = point(self.move_start_center.x + translation.x, self.move_start_center.y + translation.y);
        let (snap_point, snapped_x, snapped_y) = EditorEntitySnap::snap_center(proposed, self.canvas_size);
        self.snap_feedback.update_center(snapped_x, snapped_y);
        self.fire_snap_guides(snapped_x, snapped_y, cx);
        let kept = self.kept_inside_canvas(snap_point, &entity);
        self.center = kept;
        if self.canvas_size.width > 0.0 && self.canvas_size.height > 0.0 {
            entity.center = point(
                (kept.x / self.canvas_size.width).clamp(0.0, 1.0),
                (kept.y / self.canvas_size.height).clamp(0.0, 1.0),
            );
            self.current = Some(entity.clone());
            self.fire_update(entity, cx);
        }
        cx.notify();
    }

    /// keeps the rotated and scaled plate whole inside the canvas
    /// a box bigger than the canvas centers on that axis
    pub fn kept_inside_canvas(&self, location: Point<f32>, entity: &EditorMusicPlateEntity) -> Point<f32> {
        if self.canvas_size.width <= 0.0 || self.canvas_size.height <= 0.0 {
            return location;
        }
        let size = self.bounds_size;
        let cosine = entity.rotation.cos().abs();
        let sine = entity.rotation.sin().abs();
        let scale = entity.scale.abs();
        let half_width = (size.width * cosine + size.height * sine) * scale / 2.0;
        let half_height = (size.width * sine + size.height * cosine) * scale / 2.0;
        let clamp = |value: f32, half: f32, side: f32| {
            if half * 2.0 < side { value.max(half).min(side - half) } else { side / 2.0 }
        };
        point(
            clamp(location.x, half_width, self.canvas_size.width),
            clamp(location.y, half_height, self.canvas_size.height),
        )
    }

    fn begin_side_handle(&mut self, sign: f32, cx: &mut Context<Self>) {
        let Some(entity) = self.current.as_ref() else { return };
        self.is_side_handle_dragging = true;
        self.side_start_scale = entity.scale;
        self.side_start_rotation = entity.rotation;
        self.side_start_center = self.center;
        let selection = self.selection_rect();
        let reach = sign * selection.size.width / 2.0 * entity.scale;
        self.side_start_handle_offset = turned(point(reach, 0.0), entity.rotation);
        self.snap_feedback.begin();
        self.fire_select(cx);
    }

    fn change_side_handle(&mut self, translation: Point<f32>, cx: &mut Context<Self>) {
        let Some(mut entity) = self.current.clone() else { return };
        let (scale, rotation) = EditorEntitySideHandle::scale_and_rotation(
            self.side_start_handle_offset,
            translation,
            self.side_start_scale,
            self.side_start_rotation,
        );
        let (angle, snapped) = EditorEntitySnap::snap_rotation(rotation);
        self.snap_feedback.update_rotation(snapped);
        entity.rotation = angle;
        entity.scale = scale;
        self.center = self.side_start_center;
        self.current = Some(entity.clone());
        self.fire_update(entity, cx);
        cx.notify();
    }

    fn end_side_handle(&mut self, cx: &mut Context<Self>) {
        self.is_side_handle_dragging = false;
        let Some(mut entity) = self.current.clone() else { return };
        let kept = self.kept_inside_canvas(self.center, &entity);
        if kept == self.center || self.canvas_size.width <= 0.0 || self.canvas_size.height <= 0.0 {
            return;
        }
        self.center = kept;
        entity.center = point(
            (kept.x / self.canvas_size.width).clamp(0.0, 1.0),
            (kept.y / self.canvas_size.height).clamp(0.0, 1.0),
        );
        self.current = Some(entity.clone());
        self.fire_update(entity, cx);
        cx.notify();
    }

    fn selection_polygon(
        selection: Bounds<f32>,
        size: Size<f32>,
        entity_scale: f32,
        rotation: f32,
        zoom: f32,
        center: Point<Pixels>,
    ) -> Vec<Point<Pixels>> {
        let radius = 12.0 * zoom;
        let left = selection.origin.x;
        let top = selection.origin.y;
        let right = left + selection.size.width;
        let bottom = top + selection.size.height;
        let corners = [
            (right - radius, top + radius, -std::f32::consts::FRAC_PI_2),
            (right - radius, bottom - radius, 0.0),
            (left + radius, bottom - radius, std::f32::consts::FRAC_PI_2),
            (left + radius, top + radius, std::f32::consts::PI),
        ];
        let mut polygon = Vec::with_capacity(corners.len() * (Self::CORNER_SEGMENTS + 1));
        for (corner_x, corner_y, start) in corners {
            for step in 0..=Self::CORNER_SEGMENTS {
                let angle = start + std::f32::consts::FRAC_PI_2 * step as f32 / Self::CORNER_SEGMENTS as f32;
                let local = point(
                    (corner_x + radius * angle.cos() - size.width / 2.0) * entity_scale,
                    (corner_y + radius * angle.sin() - size.height / 2.0) * entity_scale,
                );
                let carried = turned(local, rotation);
                polygon.push(point(center.x + px(carried.x), center.y + px(carried.y)));
            }
        }
        polygon
    }
}

impl Render for EditorMusicPlateEntityView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(entity) = self.current.clone() else {
            return div();
        };
        let scale = entity.scale;
        let rotation = entity.rotation;
        let size = self.bounds_size;
        let zoom = self.display_zoom;
        let outset = (EditorMusicPlateEntityViewMetrics::SELECTION_PADDING
            + EditorMusicPlateEntityViewMetrics::HANDLE_VISUAL_SIZE / 2.0
            + EditorMusicPlateEntityViewMetrics::HANDLE_HIT_OUTSET)
            * zoom;
        let cosine = rotation.cos().abs();
        let sine = rotation.sin().abs();
        let half_width = ((size.width / 2.0 + outset) * cosine + (size.height / 2.0 + outset) * sine) * scale.abs();
        let half_height = ((size.width / 2.0 + outset) * sine + (size.height / 2.0 + outset) * cosine) * scale.abs();
        let holder_width = half_width * 2.0;
        let holder_height = half_height * 2.0;

        let id = SharedString::from(format!("editor-music-plate-{}", self.entity_id));
        let card = EditorMusicPlateCardView::new(SharedString::from(format!("{id}-card")))
            .configure(
                entity.title.clone(),
                entity.artist.clone(),
                entity.cover_url.clone(),
                entity.track_id.clone(),
                entity.is_dark,
                self.base_width,
            )
            .transform(scale, rotation);
        let card_holder = div()
            .absolute()
            .left(px(half_width - size.width * scale / 2.0))
            .top(px(half_height - size.height * scale / 2.0))
            .child(card);

        let handles = self.handles_visible.then(|| {
            let dot_at = |handle: Bounds<f32>| {
                let vector = point(
                    (handle.origin.x + handle.size.width / 2.0 - size.width / 2.0) * scale,
                    (handle.origin.y + handle.size.height / 2.0 - size.height / 2.0) * scale,
                );
                let carried = turned(vector, rotation);
                PlateHandleDotView::new(
                    half_width + carried.x,
                    half_height + carried.y,
                    EditorMusicPlateEntityViewMetrics::HANDLE_VISUAL_SIZE * zoom * scale,
                )
            };
            [dot_at(self.left_handle_rect()), dot_at(self.right_handle_rect())]
        });

        let shows_frame = self.is_selected && self.handles_visible;
        let selection = self.selection_rect();
        let view = cx.entity().downgrade();
        let surface = canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |bounds, hitbox: Hitbox, window, _| {
                let plate_center = bounds.center();
                if shows_frame {
                    let polygon = Self::selection_polygon(selection, size, scale, rotation, zoom, plate_center);
                    let stroke_scale = scale.abs() * zoom;
                    let mut builder =
                        PathBuilder::stroke(px(stroke_scale)).dash_array(&[px(5.0 * stroke_scale), px(4.0 * stroke_scale)]);
                    builder.add_polygon(&polygon, true);
                    if let Ok(path) = builder.build() {
                        window.paint_path(path, white(1.0, 0.95));
                    }
                }
                let down_view = view.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble
                        || event.button != MouseButton::Left
                        || !hitbox.is_hovered(window)
                    {
                        return;
                    }
                    let _ = down_view.update(cx, |this, cx| this.mouse_down(event, plate_center, cx));
                });
                let move_view = view.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let _ = move_view.update(cx, |this, cx| this.mouse_move(event, cx));
                });
                let up_view = view.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                        return;
                    }
                    let _ = up_view.update(cx, |this, cx| this.mouse_up(cx));
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let on_request_delete = self.on_request_delete.clone();
        let entity_id = self.entity_id;
        let delete = ContextMenuItem::new(
            1,
            tr("editor_text_delete"),
            None,
            "delete",
            move |_: &mut Window, cx: &mut App| {
                if let Some(on_request_delete) = on_request_delete.clone() {
                    cx.defer(move |cx| on_request_delete(entity_id, cx));
                }
            },
        )
        .destructive(true);

        div()
            .absolute()
            .left(px(self.center.x - half_width))
            .top(px(self.center.y - half_height))
            .w(px(holder_width))
            .h(px(holder_height))
            .child(
                div()
                    .relative()
                    .w(px(holder_width))
                    .h(px(holder_height))
                    .child(card_holder)
                    .children(handles.into_iter().flatten())
                    .child(surface)
                    .context_menu(SharedString::from(format!("{id}-menu")), vec![delete])
                    .opens_on_double_click(),
            )
    }
}

#[derive(IntoElement)]
struct PlateHandleDotView {
    center_x: f32,
    center_y: f32,
    side: f32,
}

impl PlateHandleDotView {
    fn new(center_x: f32, center_y: f32, side: f32) -> Self {
        Self { center_x, center_y, side }
    }
}

impl RenderOnce for PlateHandleDotView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let fill: Hsla = Rgba { r: 0.31, g: 0.55, b: 1.0, a: 1.0 }.into();
        div()
            .absolute()
            .left(px(self.center_x - self.side / 2.0))
            .top(px(self.center_y - self.side / 2.0))
            .w(px(self.side))
            .h(px(self.side))
            .rounded(px(self.side / 2.0))
            .bg(fill)
            .border(px(0.5))
            .border_color(white(1.0, 1.0))
            .shadow(vec![BoxShadow {
                color: white(0.0, 0.55),
                offset: point(px(0.0), px(1.0)),
                blur_radius: px(2.0),
                spread_radius: px(0.0),
                inset: false,
            }])
    }
}

/// the plate exactly as it lands on the media, for the trim overlay center preview
/// sized to the intrinsic card size
#[derive(IntoElement)]
pub struct EditorMusicPlatePreview {
    id: SharedString,
    title: String,
    artist: String,
    cover_url: String,
    track_id: String,
    is_dark: bool,
    base_width: f32,
    scale: f32,
    rotation: f32,
}

impl EditorMusicPlatePreview {
    pub fn new(
        id: impl Into<SharedString>,
        title: impl Into<String>,
        artist: impl Into<String>,
        cover_url: impl Into<String>,
        track_id: impl Into<String>,
        is_dark: bool,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            artist: artist.into(),
            cover_url: cover_url.into(),
            track_id: track_id.into(),
            is_dark,
            base_width: 280.0,
            scale: 1.0,
            rotation: 0.0,
        }
    }

    pub fn base_width(mut self, base_width: f32) -> Self {
        self.base_width = base_width;
        self
    }

    /// scale and rotation for callers that lay the plate on media
    pub fn transform(mut self, scale: f32, rotation: f32) -> Self {
        self.scale = scale;
        self.rotation = rotation;
        self
    }
}

impl RenderOnce for EditorMusicPlatePreview {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let m = EditorMusicPlateRenderer::metrics(&self.title, &self.artist, self.base_width);
        div()
            .w(px(m.card_size.width * self.scale))
            .h(px(m.card_size.height * self.scale))
            .rounded(px(m.corner_radius * self.scale))
            .shadow(vec![BoxShadow {
                color: white(0.0, 0.22),
                offset: point(px(0.0), px(6.0)),
                blur_radius: px(16.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .child(
                EditorMusicPlateCardView::new(self.id)
                    .configure(self.title, self.artist, self.cover_url, self.track_id, self.is_dark, self.base_width)
                    .transform(self.scale, self.rotation),
            )
    }
}
