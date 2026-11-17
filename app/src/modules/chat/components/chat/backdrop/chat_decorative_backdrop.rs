use std::collections::HashSet;
use std::rc::Rc;
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::Duration;

use gpui::{
    AnyView, App, AssetSource, AsyncWindowContext, Context, ElementId, Hsla, IntoElement, ObjectFit, ParentElement, ParsedSvg, Pixels, Render,
    RenderImage, RenderOnce, Rgba, SMOOTH_SVG_SCALE_FACTOR, Size, StyleRefinement, Styled, StyledImage, Subscription, SvgRenderer, Task,
    Window, black, div, img, linear_color_stop, linear_gradient, px, relative, size, white,
};
use image::{Frame, RgbaImage};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_cache::NSCache;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::CHAT_WALLPAPER_CHAT_BACKDROP_SIZE;
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_theme::theme::theme_environment::theme_store;
use rise_theme::{RiseThemeAppearance, alpha};
use rise_ui::scroll_chrome_shadow_ui::{ScrollChromeShadowStyle, ScrollChromeShadowStyleAnchor, ScrollChromeShadowUi};
use rise_ui::scroll_edge_blur_ui::ScrollEdgeBlurUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::shapes::pass_through;
use smallvec::SmallVec;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::rise_media_foreground_gate::RiseMediaForegroundGate;
use crate::core::ui::clever_image_ui::{CleverImageCorners, CleverImageUi};
use crate::core::widgets::wrappers::list_shell_ui::list_shell_safe_area_insets;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop_hooks::ChatDecorativeBackdropHooks;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_image_layer::{
    ChatWallpaperBitmapCache, ChatWallpaperImageLayer,
};
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::{
    ChatWallpaperGradientTuning, ChatWallpaperLook, ChatWallpaperPreset, ChatWallpaperRender, ChatWallpaperStore,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatWallpaperPlacement;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatBackdropLayout {
    FullBleed,
    ProfileBanner,
}

pub enum ChatBackdropMotionPolicy {}

impl ChatBackdropMotionPolicy {
    pub const TRANSITION_DURATION: f64 = 0.24;

    pub fn allows_transition(user_enabled: bool, reduce_motion: bool, low_power_mode: bool) -> bool {
        user_enabled && !reduce_motion && !low_power_mode
    }
}

/// the two places a backdrop is drawn, they mean different things
/// full bleed chat uses own wallpaper if allowed, else the account one
/// profile banner is the uploaded one or default gradient
#[derive(IntoElement)]
pub struct ChatBackdropLayers {
    pub layout: ChatBackdropLayout,
    pub chat_id: Option<String>,
    pub chat_wallpaper: Option<ChatWallpaperPlacement>,
    pub banner_url: Option<String>,
    pub bottom_corner_radius: f32,
    pub corner_radius: f32,
}

impl ChatBackdropLayers {
    pub fn new(layout: ChatBackdropLayout) -> Self {
        Self {
            layout,
            chat_id: None,
            chat_wallpaper: None,
            banner_url: None,
            bottom_corner_radius: 0.0,
            corner_radius: 0.0,
        }
    }

    pub fn corner_radius(mut self, corner_radius: f32) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    pub fn bottom_corner_radius(mut self, bottom_corner_radius: f32) -> Self {
        self.bottom_corner_radius = bottom_corner_radius;
        self
    }

    pub fn chat_id(mut self, chat_id: Option<String>) -> Self {
        self.chat_id = chat_id;
        self
    }

    pub fn chat_wallpaper(mut self, chat_wallpaper: Option<ChatWallpaperPlacement>) -> Self {
        self.chat_wallpaper = chat_wallpaper;
        self
    }

    pub fn banner_url(mut self, banner_url: Option<String>) -> Self {
        self.banner_url = banner_url;
        self
    }
}

impl RenderOnce for ChatBackdropLayers {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        match self.layout {
            ChatBackdropLayout::FullBleed => ChatDecorativeBackdrop::new()
                .ignores_safe_area(true)
                .chat_id(self.chat_id)
                .chat_wallpaper(self.chat_wallpaper)
                .into_any_element(),
            ChatBackdropLayout::ProfileBanner => {
                let top_radius = self.corner_radius;
                let foot_radius = self.bottom_corner_radius.max(self.corner_radius);
                let top = px(top_radius);
                let foot = px(foot_radius);
                let source = self
                    .banner_url
                    .as_deref()
                    .map(trimming_whitespaces_and_newlines)
                    .filter(|source| source.to_lowercase().starts_with("http"))
                    .map(str::to_owned);
                let layers = match source {
                    Some(source) => pass_through()
                        .absolute()
                        .inset_0()
                        .child(
                            pass_through()
                                .absolute()
                                .inset_0()
                                .bg(black())
                                .rounded_tl(top)
                                .rounded_tr(top)
                                .rounded_bl(foot)
                                .rounded_br(foot)
                                .overflow_hidden()
                                .child(
                                    CleverImageUi::new(source)
                                        .content_mode(ContentMode::Fill)
                                        .corners(CleverImageCorners {
                                            top_leading: top_radius,
                                            top_trailing: top_radius,
                                            bottom_leading: foot_radius,
                                            bottom_trailing: foot_radius,
                                        })
                                        .persist_to_disk(true)
                                        .shows_progress_indicator(false)
                                        .maximum_decode_pixel_dimension(1_536_i64)
                                        .shows_failure_icon(false),
                                ),
                        )
                        .child(
                            pass_through()
                                .absolute()
                                .inset_0()
                                .rounded_tl(top)
                                .rounded_tr(top)
                                .rounded_bl(foot)
                                .rounded_br(foot)
                                .bg(linear_gradient(
                                180.0,
                                linear_color_stop(alpha(black(), 0.0), 0.5),
                                linear_color_stop(alpha(black(), 0.28), 1.0),
                            )),
                        )
                        .into_any_element(),
                    None => ChatDecorativeBackdrop::new()
                        .ignores_safe_area(false)
                        .preview_preset(Some(ChatWallpaperPreset::default_preset().clone()))
                        .animates(false)
                        .corner_radius(top_radius)
                        .into_any_element(),
                };
                pass_through().relative().size_full().child(layers).into_any_element()
            }
        }
    }
}

#[derive(IntoElement)]
pub struct ChatDecorativeBackdrop {
    pub ignores_safe_area: bool,
    pub preview_preset: Option<ChatWallpaperPreset>,
    pub preview_image_url: Option<String>,
    pub preview_look: Option<ChatWallpaperLook>,
    pub chat_id: Option<String>,
    pub chat_wallpaper: Option<ChatWallpaperPlacement>,
    pub animates: bool,
    pub appearance_override: Option<RiseThemeAppearance>,
    id: Option<ElementId>,
    preview_pixel_dimension: Option<i64>,
    corner_radius: f32,
}

impl Default for ChatDecorativeBackdrop {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatDecorativeBackdrop {
    pub fn new() -> Self {
        Self {
            ignores_safe_area: true,
            preview_preset: None,
            preview_image_url: None,
            preview_look: None,
            chat_id: None,
            chat_wallpaper: None,
            animates: true,
            appearance_override: None,
            id: None,
            preview_pixel_dimension: None,
            corner_radius: 0.0,
        }
    }

    pub fn corner_radius(mut self, corner_radius: f32) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    pub fn ignores_safe_area(mut self, ignores_safe_area: bool) -> Self {
        self.ignores_safe_area = ignores_safe_area;
        self
    }

    pub fn preview_preset(mut self, preview_preset: Option<ChatWallpaperPreset>) -> Self {
        self.preview_preset = preview_preset;
        self
    }

    pub fn preview_image_url(mut self, preview_image_url: Option<String>) -> Self {
        self.preview_image_url = preview_image_url;
        self
    }

    pub fn preview_look(mut self, preview_look: Option<ChatWallpaperLook>) -> Self {
        self.preview_look = preview_look;
        self
    }

    /// why: a small preview decodes its picture at its own size and not the window one
    pub fn preview_pixel_dimension(mut self, preview_pixel_dimension: Option<i64>) -> Self {
        self.preview_pixel_dimension = preview_pixel_dimension;
        self
    }

    pub fn chat_id(mut self, chat_id: Option<String>) -> Self {
        self.chat_id = chat_id;
        self
    }

    pub fn chat_wallpaper(mut self, chat_wallpaper: Option<ChatWallpaperPlacement>) -> Self {
        self.chat_wallpaper = chat_wallpaper;
        self
    }

    pub fn animates(mut self, animates: bool) -> Self {
        self.animates = animates;
        self
    }

    pub fn appearance_override(mut self, appearance_override: Option<RiseThemeAppearance>) -> Self {
        self.appearance_override = appearance_override;
        self
    }

    /// why: gpui keys the host state, so sibling backdrops (settings tiles) pass their own key
    /// the default stays the same
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    fn host_key(&self, layer: &str) -> ElementId {
        if let Some(id) = &self.id {
            return ElementId::from(format!("{id}|{layer}"));
        }
        ElementId::from(format!(
            "chat-decorative-backdrop|{layer}|{}|{}|{}",
            self.chat_id.as_deref().unwrap_or(""),
            self.preview_preset.as_ref().map_or("", |preset| preset.id.as_str()),
            self.preview_image_url.as_deref().unwrap_or(""),
        ))
    }

    fn wallpaper_render(&self, store: &ChatWallpaperStore, cx: &App) -> Option<ChatWallpaperRender> {
        if let Some(preview_image_url) = &self.preview_image_url {
            let trimmed = trimming_whitespaces_and_newlines(preview_image_url);
            if trimmed.is_empty() {
                return None;
            }
            return Some(ChatWallpaperRender {
                image_url: trimmed.to_owned(),
                look: self.preview_look.unwrap_or_else(|| store.image_look()),
            });
        }
        if self.preview_preset.is_some() {
            return None;
        }
        store.render(self.chat_id.as_deref(), self.chat_wallpaper.as_ref(), cx)
    }

    fn gradient_core(self, cx: &mut App) -> impl IntoElement {
        let appearance = self
            .appearance_override
            .unwrap_or_else(|| theme_store(cx).read(cx).resolved_appearance());
        let (preset, pattern_intensity, motion_enabled) = if let Some(preview) = self.preview_preset.clone() {
            let tuning = ChatWallpaperGradientTuning::shared(cx);
            let tuning = tuning.read(cx);
            let pattern_intensity =
                ChatWallpaperPreset::resolved_pattern_intensity(tuning.pattern_intensity(), appearance);
            (preview.resolved(appearance), pattern_intensity, tuning.motion_enabled())
        } else {
            let store = ChatWallpaperStore::shared(cx);
            let store = store.read(cx);
            let pattern_intensity =
                ChatWallpaperPreset::resolved_pattern_intensity(store.pattern_intensity(), appearance);
            (store.current_preset().resolved(appearance), pattern_intensity, store.motion_enabled())
        };
        let (low_power_mode, reduces_motion) = {
            let energy = RiseEnergySavingStore::shared(cx);
            let energy = energy.read(cx);
            (
                energy.system_low_power_mode(),
                energy.reduces(RiseEnergySavingFeature::ChatWallpaperMotion),
            )
        };
        let allows_transition = self.animates
            && ChatBackdropMotionPolicy::allows_transition(
                motion_enabled,
                reduce_motion(cx),
                low_power_mode || reduces_motion,
            );
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let presentation_epoch_reader = ChatDecorativeBackdropHooks::resolved_chat_presentation_epoch(&*cx);
        let presentation_epoch = presentation_epoch_reader(&*cx);
        let structural_updates_blocked = ChatDecorativeBackdropHooks::resolved_structural_updates_blocked(&*cx);
        let session_id: String = [
            self.chat_id.clone().unwrap_or_default(),
            account_user_id.clone().unwrap_or_default(),
            account_epoch.clone().unwrap_or_default(),
            presentation_epoch.map(|epoch| epoch.uuid_string()).unwrap_or_default(),
        ]
        .iter()
        .map(|part| format!("{}:{}", part.len(), part))
        .collect();
        let is_session_current: Rc<dyn Fn(&App) -> bool> = Rc::new(move |cx: &App| {
            SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
                && presentation_epoch_reader(cx) == presentation_epoch
        });
        let blocks_rendering: Rc<dyn Fn(&App) -> bool> =
            Rc::new(move |cx: &App| structural_updates_blocked.as_ref().is_some_and(|blocked| (**blocked)(cx)));
        let remembers_chat_size = self.chat_id.is_some() && self.ignores_safe_area && self.preview_preset.is_none();
        ChatWallpaperRasterView {
            id: self.host_key("raster"),
            corner_radius: self.corner_radius,
            remembers_chat_size,
            preset,
            pattern_intensity,
            appearance,
            allows_transition,
            session_id,
            is_session_current,
            blocks_rendering,
        }
    }
}

impl RenderOnce for ChatDecorativeBackdrop {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let render = if self.preview_preset.is_some() && self.preview_image_url.is_none() {
            None
        } else {
            let store = ChatWallpaperStore::shared(cx);
            self.wallpaper_render(store.read(cx), cx)
        };
        let core = match render {
            Some(render) => {
                let id = self.host_key("image");
                let mut layer = ChatWallpaperImageLayer::new(render).id(id).corner_radius(self.corner_radius);
                if let Some(dimension) = self.preview_pixel_dimension {
                    layer = layer.maximum_pixel_dimension(dimension);
                }
                layer.into_any_element()
            }
            None => self.gradient_core(cx).into_any_element(),
        };
        pass_through().relative().size_full().child(core)
    }
}

#[derive(IntoElement)]
pub struct ChatEdgeShadows {
    pub top_inset: f32,
    pub bottom_inset: f32,
    pub bottom_extension: f32,
}

impl ChatEdgeShadows {
    const SHADOW_EXTENT: f32 = 66.0;
    const TOP_BLUR_FADE: f32 = 20.0;
    const PEAK_OPACITY: f64 = 0.5;
    const BOTTOM_SHADOW_SCALE: f32 = 0.6;
    const TOP_SHADOW_SCALE: f32 = 0.5;

    pub fn new(top_inset: f32, bottom_inset: f32) -> Self {
        Self {
            top_inset,
            bottom_inset,
            bottom_extension: 0.0,
        }
    }

    pub fn bottom_extension(mut self, bottom_extension: f32) -> Self {
        self.bottom_extension = bottom_extension;
        self
    }

    fn style(anchor: ScrollChromeShadowStyleAnchor, color: Hsla) -> ScrollChromeShadowStyle {
        ScrollChromeShadowStyle {
            anchor,
            peak_opacity: Self::PEAK_OPACITY,
            color,
            mid_location: 0.58,
            mid_opacity_factor: 0.68,
            top_plateau_end: 0.16,
            top_plateau_opacity_factor: 0.97,
            ..ScrollChromeShadowStyle::default()
        }
    }
}

impl RenderOnce for ChatEdgeShadows {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let top_shadow_height = (self.top_inset.max(0.0) + Self::SHADOW_EXTENT) * Self::TOP_SHADOW_SCALE;
        let bottom_shadow_height = (self.bottom_inset.max(0.0) + Self::SHADOW_EXTENT) * Self::BOTTOM_SHADOW_SCALE;
        let is_dark = theme_store(cx).read(cx).resolved_appearance() == RiseThemeAppearance::Dark;
        let color = if is_dark { black() } else { white() };
        let chrome_bottom = list_shell_safe_area_insets(cx).top.max(0.0).max(self.top_inset);
        let top_blur_height = chrome_bottom + Self::TOP_BLUR_FADE;

        pass_through()
            .relative()
            .size_full()
            .child(
                pass_through()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(top_blur_height))
                    .max_h_full()
                    .child(ScrollEdgeBlurUi::new()),
            )
            .child(
                pass_through()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(top_shadow_height))
                    .max_h_full()
                    .child(ScrollChromeShadowUi::new(Self::style(
                        ScrollChromeShadowStyleAnchor::FromTop,
                        color,
                    ))),
            )
            .child(
                pass_through()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(px(bottom_shadow_height))
                    .max_h_full()
                    .child(ScrollChromeShadowUi::new(Self::style(
                        ScrollChromeShadowStyleAnchor::FromBottom,
                        color,
                    ))),
            )
            .child(
                pass_through()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(relative(1.0))
                    .h(px(self.bottom_extension))
                    .min_h_full()
                    .bg(alpha(color, Self::PEAK_OPACITY as f32)),
            )
    }
}

#[derive(IntoElement)]
struct ChatWallpaperRasterView {
    id: ElementId,
    corner_radius: f32,
    remembers_chat_size: bool,
    preset: ChatWallpaperPreset,
    pattern_intensity: f64,
    appearance: RiseThemeAppearance,
    allows_transition: bool,
    session_id: String,
    is_session_current: Rc<dyn Fn(&App) -> bool>,
    blocks_rendering: Rc<dyn Fn(&App) -> bool>,
}

impl RenderOnce for ChatWallpaperRasterView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let host = window.use_keyed_state(self.id, cx, |_, cx| ChatWallpaperRasterHostView::new(cx));
        let corner_radius = self.corner_radius;
        host.update(cx, |host, cx| {
            host.remembers_chat_size = self.remembers_chat_size;
            if host.corner_radius != corner_radius {
                host.corner_radius = corner_radius;
                cx.notify();
            }
            host.update(
                self.preset,
                self.pattern_intensity,
                self.appearance,
                self.allows_transition,
                self.session_id,
                self.is_session_current,
                self.blocks_rendering,
                cx,
            );
        });
        pass_through()
            .size_full()
            .child(AnyView::from(host).cached(StyleRefinement::default().size_full()).replay_when_moved())
    }
}

#[derive(Clone, PartialEq)]
struct ChatWallpaperRasterHostViewConfiguration {
    preset: ChatWallpaperPreset,
    pattern_intensity: f64,
    appearance: RiseThemeAppearance,
    allows_transition: bool,
}

struct ChatWallpaperRasterHostView {
    image: Option<Arc<RenderImage>>,
    outgoing_image: Option<Arc<RenderImage>>,
    cross_dissolve: Animated<f32>,
    background_color: Hsla,
    configuration: Option<ChatWallpaperRasterHostViewConfiguration>,
    applied_identity: Option<String>,
    pending_identity: Option<String>,
    render_task: Option<Task<()>>,
    render_generation: u64,
    session_id: Option<String>,
    is_session_current: Rc<dyn Fn(&App) -> bool>,
    blocks_rendering: Rc<dyn Fn(&App) -> bool>,
    bounds_size: Option<Size<Pixels>>,
    display_scale: f32,
    remembers_chat_size: bool,
    theme_flip_pending: bool,
    corner_radius: f32,
    _foreground_observer: Subscription,
}

/// everything one cold raster needs once its wait is over
struct ChatWallpaperRasterJob {
    generation: u64,
    identity: String,
    session: Option<String>,
    configuration: ChatWallpaperRasterHostViewConfiguration,
    render_size: Size<Pixels>,
    scale: f32,
    pixel_width: i64,
    pixel_height: i64,
}

impl ChatWallpaperRasterHostView {
    const DEBOUNCE: Duration = Duration::from_millis(70);

    fn new(cx: &mut Context<Self>) -> Self {
        let gate = RiseMediaForegroundGate::shared(cx);
        let foreground_observer = cx.observe(&gate, |this, gate, cx| {
            if gate.read(cx).is_foreground() {
                this.schedule_render_if_needed(cx);
            } else {
                this.stop();
                cx.notify();
            }
        });
        Self {
            image: None,
            outgoing_image: None,
            cross_dissolve: Animated::new(1.0),
            background_color: black(),
            configuration: None,
            applied_identity: None,
            pending_identity: None,
            render_task: None,
            render_generation: 0,
            session_id: None,
            is_session_current: Rc::new(|_: &App| false),
            blocks_rendering: Rc::new(|_: &App| false),
            bounds_size: None,
            display_scale: 1.0,
            remembers_chat_size: false,
            theme_flip_pending: false,
            corner_radius: 0.0,
            _foreground_observer: foreground_observer,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        preset: ChatWallpaperPreset,
        pattern_intensity: f64,
        appearance: RiseThemeAppearance,
        allows_transition: bool,
        session_id: String,
        is_session_current: Rc<dyn Fn(&App) -> bool>,
        blocks_rendering: Rc<dyn Fn(&App) -> bool>,
        cx: &mut Context<Self>,
    ) {
        self.is_session_current = is_session_current;
        self.blocks_rendering = blocks_rendering;
        if self.session_id.as_deref() != Some(session_id.as_str()) {
            self.stop();
            self.session_id = Some(session_id);
        }
        let fallback_color = preset
            .swift_ui_background_colors()
            .first()
            .copied()
            .unwrap_or_else(black);
        let next = ChatWallpaperRasterHostViewConfiguration {
            preset,
            pattern_intensity: (pattern_intensity * 100.0).round() / 100.0,
            appearance,
            allows_transition,
        };

        if self.configuration.as_ref() == Some(&next) {
            self.schedule_render_if_needed(cx);
            return;
        }

        let previous_appearance = self.configuration.as_ref().map(|configuration| configuration.appearance);
        self.configuration = Some(next);
        if let Some(previous_appearance) = previous_appearance
            && previous_appearance != appearance
            && self.image.is_some()
        {
            self.stop();
            self.image = None;
            self.applied_identity = None;
            self.theme_flip_pending = true;
            cx.notify();
        }
        if self.background_color != fallback_color {
            self.background_color = fallback_color;
            cx.notify();
        }
        if !allows_transition {
            self.remove_all_animations();
        }
        self.schedule_render_if_needed(cx);
    }

    fn stop(&mut self) {
        self.render_generation = self.render_generation.wrapping_add(1);
        self.render_task = None;
        self.pending_identity = None;
        self.remove_all_animations();
    }

    fn remove_all_animations(&mut self) {
        self.cross_dissolve.jump(1.0);
        self.outgoing_image = None;
    }

    fn layout_subviews(
        &mut self,
        bounds_size: Size<Pixels>,
        display_scale: f32,
        window_size: Size<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.bounds_size == Some(bounds_size) && self.display_scale == display_scale {
            return;
        }
        if self.remembers_chat_size && f32::from(bounds_size.height) >= f32::from(window_size.height) - 0.5 {
            ChatWallpaperPrewarm::remember_chat_backdrop(bounds_size);
        }
        self.bounds_size = Some(bounds_size);
        self.display_scale = display_scale;
        self.schedule_render_if_needed(cx);
    }

    fn schedule_render_if_needed(&mut self, cx: &mut Context<Self>) {
        let Some(configuration) = self.configuration.clone() else {
            x456_mark(|| "schedule no_configuration".to_owned());
            return;
        };
        if !(self.is_session_current)(&**cx) || !RiseMediaForegroundGate::shared(cx).read(cx).is_foreground() {
            x456_mark(|| {
                format!(
                    "schedule gated session_current={} foreground={}",
                    (self.is_session_current)(&**cx),
                    RiseMediaForegroundGate::shared(cx).read(cx).is_foreground()
                )
            });
            return;
        }
        let Some(bounds_size) = self.bounds_size else {
            x456_mark(|| "schedule no_bounds".to_owned());
            return;
        };

        let Some(geometry) = ChatWallpaperRasterGeometry::new(bounds_size, self.display_scale) else {
            return;
        };
        let ChatWallpaperRasterGeometry { scale, pixel_width, pixel_height, render_size } = geometry;
        let base_key = ChatWallpaperRasterCache::base_key(
            &configuration.preset,
            configuration.pattern_intensity,
            configuration.appearance,
            pixel_width,
            pixel_height,
            scale,
        );
        let identity = base_key.clone();

        if self.applied_identity.as_deref() == Some(identity.as_str())
            || self.pending_identity.as_deref() == Some(identity.as_str())
        {
            return;
        }
        x456_mark(|| {
            format!(
                "schedule preset={} px={pixel_width}x{pixel_height} cached={}",
                configuration.preset.id,
                ChatWallpaperRasterCache::cached_image(&base_key).is_some()
            )
        });

        self.render_task = None;
        self.render_generation = self.render_generation.wrapping_add(1);
        let generation = self.render_generation;
        let session = self.session_id.clone();
        self.pending_identity = Some(identity.clone());
        if let Some(cached) = ChatWallpaperRasterCache::cached_image(&base_key) {
            self.remove_all_animations();
            self.image = Some(cached);
            self.applied_identity = Some(identity);
            self.pending_identity = None;
            self.render_task = None;
            self.theme_flip_pending = false;
            cx.notify();
            return;
        }
        let should_debounce = self.applied_identity.is_some() && !self.theme_flip_pending;
        let job = ChatWallpaperRasterJob {
            generation,
            identity,
            session,
            configuration,
            render_size,
            scale,
            pixel_width,
            pixel_height,
        };
        self.render_task = Some(cx.spawn(async move |this, cx| {
            if should_debounce {
                cx.background_executor().timer(Self::DEBOUNCE).await;
            }
            loop {
                let deferred = this.update(cx, |this, cx| {
                    this.is_current_job(&job, cx).then(|| !this.theme_flip_pending && this.is_render_deferred(cx))
                });
                match deferred {
                    Ok(Some(true)) => {}
                    Ok(Some(false)) => break,
                    _ => return,
                }
                cx.background_executor()
                    .timer(ChatDecorativeBackdropHooks::resolved_background_work_retry_interval())
                    .await;
            }
            let prepared = this.update(cx, |this, cx| this.prepare_render(&job, cx));
            let Ok(Some(prepared)) = prepared else {
                return;
            };
            let first = match prepared {
                ChatWallpaperRasterPrepared::Cached(image) => Some(image),
                ChatWallpaperRasterPrepared::Draw(inputs) => {
                    let key = job.identity.clone();
                    let cost = ChatWallpaperRasterCache::cost(job.pixel_width, job.pixel_height);
                    cx.background_executor()
                        .spawn(async move { ChatWallpaperRasterCache::draw(inputs, key, cost) })
                        .await
                }
            };
            let _ = this.update(cx, |this, cx| this.finish_render(job, first, cx));
        }));
    }

    fn is_current_job(&self, job: &ChatWallpaperRasterJob, cx: &App) -> bool {
        self.render_generation == job.generation
            && self.pending_identity.as_deref() == Some(job.identity.as_str())
            && self.session_id == job.session
            && (self.is_session_current)(cx)
    }

    fn prepare_render(
        &mut self,
        job: &ChatWallpaperRasterJob,
        cx: &mut Context<Self>,
    ) -> Option<ChatWallpaperRasterPrepared> {
        if !self.is_current_job(job, cx) || (!self.theme_flip_pending && self.is_render_deferred(cx)) {
            return None;
        }
        if let Some(cached) = ChatWallpaperRasterCache::cached_image(&job.identity) {
            return Some(ChatWallpaperRasterPrepared::Cached(cached));
        }
        let inputs = ChatWallpaperRasterFrame::new(
            job.configuration.preset.clone(),
            job.configuration.pattern_intensity,
            job.configuration.appearance,
            job.render_size,
        )
        .raster_inputs(job.scale, cx);
        if inputs.is_none() {
            x456_mark(|| "prepare no_inputs".to_owned());
            self.pending_identity = None;
            self.render_task = None;
        }
        inputs.map(ChatWallpaperRasterPrepared::Draw)
    }

    fn finish_render(&mut self, job: ChatWallpaperRasterJob, first: Option<Arc<RenderImage>>, cx: &mut Context<Self>) {
        if !self.is_current_job(&job, cx) {
            x456_mark(|| "finish stale_job".to_owned());
            return;
        }
        x456_mark(|| format!("finish image={}", first.is_some()));
        let Some(first) = first else {
            self.pending_identity = None;
            self.render_task = None;
            return;
        };

        let (low_power_mode, reduces_motion) = {
            let energy = RiseEnergySavingStore::shared(cx);
            let energy = energy.read(cx);
            (
                energy.system_low_power_mode(),
                energy.reduces(RiseEnergySavingFeature::ChatWallpaperMotion),
            )
        };
        let should_transition = self.configuration.as_ref().is_some_and(|configuration| configuration.allows_transition)
            && self.applied_identity.is_some()
            && ChatBackdropMotionPolicy::allows_transition(
                !reduces_motion,
                reduce_motion(cx),
                low_power_mode,
            );
        if should_transition {
            self.outgoing_image = self.image.take();
            self.cross_dissolve.jump(0.0);
            self.cross_dissolve.set(
                1.0,
                ease_in_out(ChatBackdropMotionPolicy::TRANSITION_DURATION),
                RiseClock::monotonic(),
                false,
            );
        } else {
            self.remove_all_animations();
        }
        self.image = Some(first);
        self.applied_identity = Some(job.identity);
        self.pending_identity = None;
        self.render_task = None;
        self.theme_flip_pending = false;
        cx.notify();
    }

    /// true while the app is in background or the host says to hold rendering
    /// gpui has no view ancestry to walk so the host decides
    fn is_render_deferred(&self, cx: &mut Context<Self>) -> bool {
        !RiseMediaForegroundGate::shared(cx).read(cx).is_foreground() || (self.blocks_rendering)(&**cx)
    }
}

impl Render for ChatWallpaperRasterHostView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let progress = self.cross_dissolve.frame_value(now, window).clamp(0.0, 1.0);
        if !self.cross_dissolve.is_running(now) {
            self.outgoing_image = None;
        }
        let host = cx.entity();
        let radius = px(self.corner_radius);
        pass_through()
            .relative()
            .size_full()
            .overflow_hidden()
            .rounded(radius)
            .bg(self.background_color)
            .children(self.outgoing_image.clone().map(|image| {
                img(image)
                    .absolute()
                    .inset_0()
                    .size_full()
                    .rounded(radius)
                    .object_fit(ObjectFit::Fill)
                    .opacity(1.0 - progress)
            }))
            .children(self.image.clone().map(|image| {
                img(image)
                    .absolute()
                    .inset_0()
                    .size_full()
                    .rounded(radius)
                    .object_fit(ObjectFit::Fill)
                    .opacity(progress)
            }))
            .child(measure("chat-wallpaper-raster-bounds", move |bounds, window, cx| {
                let display_scale = window.scale_factor();
                let window_size = window.viewport_size();
                host.update(cx, |host, cx| host.layout_subviews(bounds.size, display_scale, window_size, cx));
            }))
    }
}

static RASTER_IMAGES: LazyLock<NSCache<String, Arc<RenderImage>>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(usize::try_from(RiseLimits::client_value("chat_wallpaper_raster_cache_entries")).unwrap_or(0));
    cache.set_total_cost_limit(usize::try_from(RiseLimits::client_value("chat_wallpaper_raster_cache_bytes")).unwrap_or(0));
    cache
});

/// chat wallpaper drawn before the first chat opens
/// a cold bitmap would make the first chat slide in over black
/// chat list warms it once idle, nothing drawn if cached
pub enum ChatWallpaperPrewarm {}

impl ChatWallpaperPrewarm {
    const SETTLE_DELAY: Duration = Duration::from_millis(600);
    const RETRY_DELAY: Duration = Duration::from_millis(300);
    const MAXIMUM_ATTEMPTS: usize = 10;
    const PICTURE_LIMIT: usize = 4;
    const PICTURE_DELAY: Duration = Duration::from_millis(150);

    /// picture wallpapers a list leads to, decoded ahead while it stands still
    /// a forum topic with its own picture opened on black until the picture was decoded
    pub async fn pictures(chat_id: String, placements: Vec<ChatWallpaperPlacement>, cx: &mut AsyncWindowContext) {
        if placements.is_empty() {
            return;
        }
        cx.background_executor().timer(Self::PICTURE_DELAY).await;
        let renders = cx.update(|_, cx| {
            let store = ChatWallpaperStore::shared(cx);
            let store = store.read(cx);
            let mut seen = HashSet::new();
            let mut renders: Vec<ChatWallpaperRender> = Vec::new();
            for placement in &placements {
                let Some(render) = store.render(Some(&chat_id), Some(placement), cx) else { continue };
                if !seen.insert(format!("{}|{}", render.image_url, render.look.blur_percent)) {
                    continue;
                }
                renders.push(render);
                if renders.len() == Self::PICTURE_LIMIT {
                    break;
                }
            }
            renders
        });
        let Ok(renders) = renders else { return };
        for render in renders {
            if !Self::is_foreground(cx) {
                return;
            }
            let _ = ChatWallpaperBitmapCache::image(&render.image_url, render.look.blur_percent, cx).await;
        }
    }

    pub fn remember_chat_backdrop(size: Size<Pixels>) {
        let (width, height) = (f32::from(size.width), f32::from(size.height));
        if let Some(stored) = Self::stored_chat_backdrop_size()
            && (f32::from(stored.width) - width).abs() < 0.5
            && f32::from(stored.height) >= height - 0.5
        {
            return;
        }
        UserDefaults::standard().set(
            CHAT_WALLPAPER_CHAT_BACKDROP_SIZE,
            format!("{}x{}", f64::from(width), f64::from(height)),
        );
    }

    fn stored_chat_backdrop_size() -> Option<Size<Pixels>> {
        let stored = UserDefaults::standard().string(CHAT_WALLPAPER_CHAT_BACKDROP_SIZE).unwrap_or_default();
        let parts: Vec<f64> = stored.split('x').filter_map(|part| part.parse().ok()).collect();
        let [width, height] = parts[..] else { return None };
        Some(size(px(width as f32), px(height as f32)))
    }

    fn chat_backdrop_size(window_size: Size<Pixels>) -> Size<Pixels> {
        match Self::stored_chat_backdrop_size() {
            Some(stored)
                if f32::from(stored.width) <= f32::from(window_size.width) + 0.5
                    && f32::from(stored.height) >= f32::from(window_size.height) - 0.5 =>
            {
                stored
            }
            _ => window_size,
        }
    }

    pub async fn run(appearance: RiseThemeAppearance, cx: &mut AsyncWindowContext) {
        let picture = cx.update(|_, cx| {
            let store = ChatWallpaperStore::shared(cx);
            store.read(cx).render(None, None, cx)
        });
        let Ok(picture) = picture else { return };
        if let Some(picture) = picture {
            let _ = ChatWallpaperBitmapCache::image(&picture.image_url, picture.look.blur_percent, cx).await;
            return;
        }
        cx.background_executor().timer(Self::SETTLE_DELAY).await;
        for _ in 0..Self::MAXIMUM_ATTEMPTS {
            if Self::is_foreground(cx) {
                let job = cx.update(|window, cx| Self::prepare(appearance, window, cx)).ok().flatten();
                if let Some((inputs, key, cost)) = job {
                    let _ = cx
                        .background_executor()
                        .spawn(async move { ChatWallpaperRasterCache::draw(inputs, key, cost) })
                        .await;
                }
                return;
            }
            cx.background_executor().timer(Self::RETRY_DELAY).await;
        }
    }

    fn is_foreground(cx: &mut AsyncWindowContext) -> bool {
        cx.update(|_, cx| RiseMediaForegroundGate::shared(cx).read(cx).is_foreground()).unwrap_or(false)
    }

    /// inputs of the bitmap a chat will ask for, read on the main thread
    /// none when its cached already
    fn prepare(
        appearance: RiseThemeAppearance,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<(ChatWallpaperRasterInputs, String, usize)> {
        let geometry =
            ChatWallpaperRasterGeometry::new(Self::chat_backdrop_size(window.viewport_size()), window.scale_factor())?;
        let store = ChatWallpaperStore::shared(cx);
        let (preset, intensity) = {
            let store = store.read(cx);
            let preset = store.current_preset().resolved(appearance);
            let intensity =
                (ChatWallpaperPreset::resolved_pattern_intensity(store.pattern_intensity(), appearance) * 100.0).round()
                    / 100.0;
            (preset, intensity)
        };
        let base_key = ChatWallpaperRasterCache::base_key(
            &preset,
            intensity,
            appearance,
            geometry.pixel_width,
            geometry.pixel_height,
            geometry.scale,
        );
        if ChatWallpaperRasterCache::cached_image(&base_key).is_some() {
            return None;
        }
        let inputs = ChatWallpaperRasterFrame::new(preset, intensity, appearance, geometry.render_size)
            .raster_inputs(geometry.scale, cx)?;
        Some((inputs, base_key, ChatWallpaperRasterCache::cost(geometry.pixel_width, geometry.pixel_height)))
    }
}

/// bitmap a backdrop of bounds points asks for, at most 1.5x and whole pixels
#[derive(Clone, Copy)]
struct ChatWallpaperRasterGeometry {
    scale: f32,
    pixel_width: i64,
    pixel_height: i64,
    render_size: Size<Pixels>,
}

impl ChatWallpaperRasterGeometry {
    const MAXIMUM_SCALE: f32 = 1.5;

    fn new(bounds: Size<Pixels>, display_scale: f32) -> Option<Self> {
        let scale = display_scale.max(1.0).min(Self::MAXIMUM_SCALE);
        let pixel_width = (f32::from(bounds.width) * scale).ceil() as i64;
        let pixel_height = (f32::from(bounds.height) * scale).ceil() as i64;
        if pixel_width <= 1 || pixel_height <= 1 {
            return None;
        }
        let render_size = size(px(pixel_width as f32 / scale), px(pixel_height as f32 / scale));
        Some(Self { scale, pixel_width, pixel_height, render_size })
    }
}

/// what the main thread hands a cold render, the cached bitmap or the inputs to draw it
enum ChatWallpaperRasterPrepared {
    Cached(Arc<RenderImage>),
    Draw(ChatWallpaperRasterInputs),
}

enum ChatWallpaperRasterCache {}

impl ChatWallpaperRasterCache {
    fn cost(pixel_width: i64, pixel_height: i64) -> usize {
        usize::try_from(pixel_width.max(0) * pixel_height.max(0) * 4).unwrap_or(usize::MAX)
    }

    /// draws a prepared frame and caches it under key, any thread
    fn draw(inputs: ChatWallpaperRasterInputs, key: String, cost: usize) -> Option<Arc<RenderImage>> {
        if let Some(cached) = RASTER_IMAGES.object(&key) {
            return Some(cached);
        }
        let image = inputs.render()?;
        RASTER_IMAGES.set_object_cost(Arc::clone(&image), key, cost);
        Some(image)
    }

    fn cached_image(base_key: &str) -> Option<Arc<RenderImage>> {
        RASTER_IMAGES.object(&base_key.to_owned())
    }

    fn base_key(
        preset: &ChatWallpaperPreset,
        pattern_intensity: f64,
        appearance: RiseThemeAppearance,
        pixel_width: i64,
        pixel_height: i64,
        scale: f32,
    ) -> String {
        [
            preset.id.clone(),
            preset.background_colors.join(","),
            preset.pattern_colors.join(","),
            appearance.raw_value().to_owned(),
            pattern_intensity.to_bits().to_string(),
            format!("{pixel_width}x{pixel_height}"),
            f64::from(scale).to_bits().to_string(),
        ]
        .join("|")
    }

    #[allow(clippy::too_many_arguments)]
    fn image(
        preset: &ChatWallpaperPreset,
        pattern_intensity: f64,
        appearance: RiseThemeAppearance,
        size: Size<Pixels>,
        scale: f32,
        pixel_width: i64,
        pixel_height: i64,
        base_key: &str,
        cx: &App,
    ) -> Option<Arc<RenderImage>> {
        let key = base_key.to_owned();
        if let Some(cached) = RASTER_IMAGES.object(&key) {
            return Some(cached);
        }
        let inputs =
            ChatWallpaperRasterFrame::new(preset.clone(), pattern_intensity, appearance, size).raster_inputs(scale, cx)?;
        Self::draw(inputs, key, Self::cost(pixel_width, pixel_height))
    }
}

/// one wallpaper frame drawn by both the chat bitmap cache and the qr screen export
/// a second drawing would be a second home for the preset palette
#[derive(IntoElement)]
pub struct ChatWallpaperRasterFrame {
    pub preset: ChatWallpaperPreset,
    pub pattern_intensity: f64,
    pub appearance: RiseThemeAppearance,
    pub size: Size<Pixels>,
    pub corner_radius: f32,
}

const PATTERN_ASSET_PATH: &str = "images/TelegramChatWallpaperPattern.svg";
const PATTERN_VIEW_BOX: (f32, f32) = (1440.0, 2960.0);
const DARK_FRAME_SHADE: f32 = 0.08;

static PATTERN_SVG: OnceLock<Option<Arc<ParsedSvg>>> = OnceLock::new();

/// where the pattern comes from, carried to the drawing thread and parsed once per process
struct ChatWallpaperPatternSource {
    renderer: SvgRenderer,
    assets: Arc<dyn AssetSource>,
}

impl ChatWallpaperPatternSource {
    fn parsed(&self) -> Option<Arc<ParsedSvg>> {
        PATTERN_SVG
            .get_or_init(|| {
                let bytes = self.assets.load(PATTERN_ASSET_PATH).ok()??;
                self.renderer.parse_svg(&bytes).ok().map(Arc::new)
            })
            .clone()
    }
}

/// one frame resolved on the main thread (colors, size pattern source) and drawn on any
pub struct ChatWallpaperRasterInputs {
    width: usize,
    height: usize,
    base: Rgba,
    background_colors: Vec<Hsla>,
    pattern_colors: Vec<Hsla>,
    pattern_opacity: f32,
    shade: f32,
    pattern: ChatWallpaperPatternSource,
}

impl ChatWallpaperRasterFrame {
    pub fn new(
        preset: ChatWallpaperPreset,
        pattern_intensity: f64,
        appearance: RiseThemeAppearance,
        size: Size<Pixels>,
    ) -> Self {
        Self {
            preset,
            pattern_intensity,
            appearance,
            size,
            corner_radius: 0.0,
        }
    }

    pub fn corner_radius(mut self, corner_radius: f32) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    /// renders the frame to a bitmap of size times scale pixels, on the calling thread
    pub fn render_image(&self, scale: f32, cx: &App) -> Option<Arc<RenderImage>> {
        self.raster_inputs(scale, cx)?.render()
    }

    /// everything the raster render needs, read on main thread. none when the frame is under a pixel
    pub fn raster_inputs(&self, scale: f32, cx: &App) -> Option<ChatWallpaperRasterInputs> {
        let width = (f32::from(self.size.width) * scale).round();
        let height = (f32::from(self.size.height) * scale).round();
        if !(width >= 1.0 && height >= 1.0) {
            return None;
        }
        let background_colors = self.preset.swift_ui_background_colors();
        let base = background_colors.first().copied().unwrap_or_else(|| {
            if self.appearance == RiseThemeAppearance::Light {
                white()
            } else {
                black()
            }
        });
        Some(ChatWallpaperRasterInputs {
            width: width as usize,
            height: height as usize,
            base: base.to_rgb(),
            background_colors,
            pattern_colors: self.preset.swift_ui_pattern_colors(),
            pattern_opacity: self.pattern_intensity.clamp(0.0, 1.0) as f32,
            shade: if self.appearance == RiseThemeAppearance::Dark {
                DARK_FRAME_SHADE
            } else {
                0.0
            },
            pattern: ChatWallpaperPatternSource {
                renderer: cx.svg_renderer(),
                assets: Arc::clone(cx.asset_source()),
            },
        })
    }
}

impl ChatWallpaperRasterInputs {
    pub fn render(self) -> Option<Arc<RenderImage>> {
        let width = self.width;
        let height = self.height;
        let background_mesh = ChatWallpaperMesh::new(self.background_colors.clone()).rasterize(width, height);
        let pattern_mesh = ChatWallpaperMesh::new(self.pattern_colors.clone()).rasterize(width, height);
        let mask = self.pattern_mask();
        let base = self.base;
        x456_mark(|| {
            format!(
                "render {width}x{height} base={base:?} colors={} patterns={} mask={} covered={} opacity={}",
                self.background_colors.len(),
                self.pattern_colors.len(),
                mask.is_some(),
                mask.as_ref().map_or(0, |mask| mask.iter().filter(|value| **value > 0).count()),
                self.pattern_opacity
            )
        });

        let mut bytes = vec![0_u8; width * height * 4];
        for (index, pixel) in bytes.chunks_exact_mut(4).enumerate() {
            let mut out = [base.r, base.g, base.b, base.a];
            out = source_over(out, background_mesh[index]);
            let coverage = mask.as_ref().map_or(1.0, |mask| f32::from(mask[index]) / 255.0);
            let mut pattern = pattern_mesh[index];
            pattern[3] *= coverage * self.pattern_opacity;
            out = source_over(out, pattern);
            if self.shade > 0.0 {
                out = source_over(out, [0.0, 0.0, 0.0, self.shade]);
            }
            pixel[0] = to_byte(out[2]);
            pixel[1] = to_byte(out[1]);
            pixel[2] = to_byte(out[0]);
            pixel[3] = to_byte(out[3]);
        }

        let buffer = RgbaImage::from_raw(u32::try_from(width).ok()?, u32::try_from(height).ok()?, bytes)?;
        let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([Frame::new(buffer)]);
        Some(Arc::new(RenderImage::new(frames)))
    }

    fn pattern_mask(&self) -> Option<Vec<u8>> {
        let width = self.width;
        let height = self.height;
        let svg = self.pattern.parsed()?;
        let fill_scale = (width as f32 / PATTERN_VIEW_BOX.0).max(height as f32 / PATTERN_VIEW_BOX.1);
        let rendered = self
            .pattern
            .renderer
            .render_parsed(&svg, fill_scale / SMOOTH_SVG_SCALE_FACTOR)
            .ok()?;
        let rendered_size = rendered.size(0);
        let rendered_width = usize::try_from(rendered_size.width.0).ok()?;
        let rendered_height = usize::try_from(rendered_size.height.0).ok()?;
        let pixels = rendered.as_bytes(0)?;
        let offset_x = rendered_width.saturating_sub(width) / 2;
        let offset_y = rendered_height.saturating_sub(height) / 2;
        let mut mask = vec![0_u8; width * height];
        for y in 0..height {
            let source_y = y + offset_y;
            if source_y >= rendered_height {
                break;
            }
            for x in 0..width {
                let source_x = x + offset_x;
                if source_x >= rendered_width {
                    break;
                }
                mask[y * width + x] = pixels[(source_y * rendered_width + source_x) * 4 + 3];
            }
        }
        Some(mask)
    }
}

impl RenderOnce for ChatWallpaperRasterFrame {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let scale = window.scale_factor();
        let pixel_width = (f32::from(self.size.width) * scale).round() as i64;
        let pixel_height = (f32::from(self.size.height) * scale).round() as i64;
        let base_key = ChatWallpaperRasterCache::base_key(
            &self.preset,
            self.pattern_intensity,
            self.appearance,
            pixel_width,
            pixel_height,
            scale,
        );
        let image = ChatWallpaperRasterCache::image(
            &self.preset,
            self.pattern_intensity,
            self.appearance,
            self.size,
            scale,
            pixel_width,
            pixel_height,
            &base_key,
            cx,
        );
        let fallback = self
            .preset
            .swift_ui_background_colors()
            .first()
            .copied()
            .unwrap_or_else(|| if self.appearance == RiseThemeAppearance::Light { white() } else { black() });
        let radius = px(self.corner_radius);
        div()
            .w(self.size.width)
            .h(self.size.height)
            .overflow_hidden()
            .rounded(radius)
            .bg(fallback)
            .children(image.map(|image| img(image).size_full().rounded(radius).object_fit(ObjectFit::Fill)))
    }
}

/// 3x3 mesh gradient drawn on the cpu because gpui only paints linear gradients
pub struct ChatWallpaperMesh {
    pub colors: Vec<Hsla>,
}

impl ChatWallpaperMesh {
    const SAMPLE_STEP: usize = 4;

    pub fn new(colors: Vec<Hsla>) -> Self {
        Self { colors }
    }

    fn points() -> [[f32; 2]; 9] {
        [
            [0.0, 0.0],
            [0.46, 0.0],
            [1.0, 0.0],
            [0.0, 0.46],
            [0.54, 0.49],
            [1.0, 0.54],
            [0.0, 1.0],
            [0.54, 1.0],
            [1.0, 1.0],
        ]
    }

    fn mesh_colors(&self) -> [Hsla; 9] {
        let safe_colors = if self.colors.is_empty() {
            ChatWallpaperPreset::default_preset().swift_ui_background_colors()
        } else {
            self.colors.clone()
        };
        let last = safe_colors.len().saturating_sub(1);
        let color = |index: usize| safe_colors.get(index.min(last)).copied().unwrap_or_else(black);
        let (c0, c1, c2, c3) = (color(0), color(1), color(2), color(3));
        [c0, c0, c1, c0, c2, c1, c3, c2, c1]
    }

    pub fn rasterize(&self, width: usize, height: usize) -> Vec<[f32; 4]> {
        let colors = self.mesh_colors().map(|color| OklabColor::from_rgba(color.to_rgb()));
        let points = Self::points();
        let step = Self::SAMPLE_STEP;
        let columns = width.div_ceil(step) + 1;
        let rows = height.div_ceil(step) + 1;
        let mut samples = Vec::with_capacity(columns * rows);
        for row in 0..rows {
            let y = ((row * step) as f32 / (height.max(2) - 1) as f32).min(1.0);
            for column in 0..columns {
                let x = ((column * step) as f32 / (width.max(2) - 1) as f32).min(1.0);
                let (u, v) = Self::parameters(&points, x, y);
                samples.push(Self::color_at(&colors, u, v).to_rgba());
            }
        }
        let mut pixels = Vec::with_capacity(width * height);
        for y in 0..height {
            let row = y / step;
            let fy = (y % step) as f32 / step as f32;
            for x in 0..width {
                let column = x / step;
                let fx = (x % step) as f32 / step as f32;
                let top = lerp4(samples[row * columns + column], samples[row * columns + column + 1], fx);
                let bottom = lerp4(
                    samples[(row + 1) * columns + column],
                    samples[(row + 1) * columns + column + 1],
                    fx,
                );
                pixels.push(lerp4(top, bottom, fy));
            }
        }
        pixels
    }

    /// patch params (0 to 2 per axis) of a point in the unit square
    /// mesh vertices are off grid so the grid is warped back in a few steps
    fn parameters(points: &[[f32; 2]; 9], x: f32, y: f32) -> (f32, f32) {
        let mut u = x * 2.0;
        let mut v = y * 2.0;
        for _ in 0..4 {
            let column_x = |column: usize| piecewise(v, [points[column][0], points[3 + column][0], points[6 + column][0]]);
            u = inverse_piecewise(x, [column_x(0), column_x(1), column_x(2)]);
            let row_y = |row: usize| piecewise(u, [points[row * 3][1], points[row * 3 + 1][1], points[row * 3 + 2][1]]);
            v = inverse_piecewise(y, [row_y(0), row_y(1), row_y(2)]);
        }
        (u, v)
    }

    /// bicubic colour across the grid so corners blend smooth instead of creasing
    fn color_at(colors: &[OklabColor; 9], u: f32, v: f32) -> OklabColor {
        let rows = [0, 1, 2].map(|row| hermite([colors[row * 3], colors[row * 3 + 1], colors[row * 3 + 2]], u));
        hermite(rows, v)
    }
}

/// colour in oklab with straight alpha, the space the mesh blends in
#[derive(Clone, Copy)]
struct OklabColor {
    l: f32,
    a: f32,
    b: f32,
    alpha: f32,
}

impl OklabColor {
    fn from_rgba(color: Rgba) -> Self {
        let r = srgb_to_linear(color.r);
        let g = srgb_to_linear(color.g);
        let b = srgb_to_linear(color.b);
        let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
        let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
        let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
        Self {
            l: 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
            a: 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
            b: 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
            alpha: color.a,
        }
    }

    fn to_rgba(self) -> [f32; 4] {
        let l = (self.l + 0.396_337_78 * self.a + 0.215_803_76 * self.b).powi(3);
        let m = (self.l - 0.105_561_346 * self.a - 0.063_854_17 * self.b).powi(3);
        let s = (self.l - 0.089_484_18 * self.a - 1.291_485_5 * self.b).powi(3);
        let r = 4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s;
        let g = -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s;
        let b = -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s;
        [
            linear_to_srgb(r),
            linear_to_srgb(g),
            linear_to_srgb(b),
            self.alpha.clamp(0.0, 1.0),
        ]
    }

    fn mix(self, other: Self, weights: [f32; 2]) -> Self {
        Self {
            l: self.l * weights[0] + other.l * weights[1],
            a: self.a * weights[0] + other.a * weights[1],
            b: self.b * weights[0] + other.b * weights[1],
            alpha: self.alpha * weights[0] + other.alpha * weights[1],
        }
    }
}

/// cubic hermite through three knots at 0, 1 and 2 evaluated at t
fn hermite(knots: [OklabColor; 3], t: f32) -> OklabColor {
    let t = t.clamp(0.0, 2.0);
    let segment = if t >= 1.0 { 1 } else { 0 };
    let local = t - segment as f32;
    let slope = |index: usize| {
        let before = index.saturating_sub(1);
        let after = (index + 1).min(2);
        let span = (after - before) as f32;
        knots[after].mix(knots[before], [1.0 / span, -1.0 / span])
    };
    let (p0, p1) = (knots[segment], knots[segment + 1]);
    let (m0, m1) = (slope(segment), slope(segment + 1));
    let t2 = local * local;
    let t3 = t2 * local;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + local;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    p0.mix(m0, [h00, h10]).mix(p1.mix(m1, [h01, h11]), [1.0, 1.0])
}

/// value at t (0 to 2) of a polyline through three knots
fn piecewise(t: f32, knots: [f32; 3]) -> f32 {
    let t = t.clamp(0.0, 2.0);
    if t <= 1.0 {
        knots[0] + (knots[1] - knots[0]) * t
    } else {
        knots[1] + (knots[2] - knots[1]) * (t - 1.0)
    }
}

/// parameter (0 to 2) where a rising polyline through three knots hits value
fn inverse_piecewise(value: f32, knots: [f32; 3]) -> f32 {
    if value <= knots[1] {
        let span = knots[1] - knots[0];
        if span <= f32::EPSILON {
            return 0.0;
        }
        ((value - knots[0]) / span).clamp(0.0, 1.0)
    } else {
        let span = knots[2] - knots[1];
        if span <= f32::EPSILON {
            return 2.0;
        }
        1.0 + ((value - knots[1]) / span).clamp(0.0, 1.0)
    }
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

fn lerp4(from: [f32; 4], to: [f32; 4], t: f32) -> [f32; 4] {
    [
        from[0] + (to[0] - from[0]) * t,
        from[1] + (to[1] - from[1]) * t,
        from[2] + (to[2] - from[2]) * t,
        from[3] + (to[3] - from[3]) * t,
    ]
}

/// source drawn over destination, both straight rgba
fn source_over(destination: [f32; 4], source: [f32; 4]) -> [f32; 4] {
    let source_alpha = source[3].clamp(0.0, 1.0);
    let destination_alpha = destination[3].clamp(0.0, 1.0);
    let out_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    if out_alpha <= f32::EPSILON {
        return [0.0, 0.0, 0.0, 0.0];
    }
    let channel = |index: usize| {
        (source[index] * source_alpha + destination[index] * destination_alpha * (1.0 - source_alpha)) / out_alpha
    };
    [channel(0), channel(1), channel(2), out_alpha]
}

fn to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn x456_mark(stage: impl FnOnce() -> String) {
    tracing::warn!(target: "riseonly::X456", "{}", stage());
}
