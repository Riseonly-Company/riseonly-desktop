use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use gpui::{
    AnyElement, AnyWindowHandle, App, AsyncApp, Bounds, Element, ElementId, GlobalElementId, InspectorElementId,
    LayoutId, Pixels, RenderImage, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, Size, Styled, TextRun, Transformation, Window, div, img, px, radians, transparent_black, white,
};
use image::{Delay, Frame, RgbaImage};
use rise_canon::rise_key::gift_colors;
use rise_core::foundation::ns_cache::NSCache;
use rise_i18n::{FormatArg, format_printf, tr};
use rise_theme::theme::app_theme::{alpha, color};
use rise_ui::IconUi;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::font::{Font, SwiftUiFont};
use rise_ui::swiftui::frame::SwiftUiFrame;
use rise_ui::swiftui::lifecycle::use_task;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::minimum_scale_factor::MinimumScaleText;
use rise_ui::swiftui::radial_gradient::RadialGradient;
use rise_ui::swiftui::rotation_effect::RotationEffectModifiers;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, pass_through};
use rise_ui::swiftui::stacks::{Alignment, hstack, zstack};
use smallvec::SmallVec;
use rise_ui::swiftui::transition::UnitPoint;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapLoader, CleverImageWorkLane};
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::gifts::components::gift_animation_view::GiftAnimationView;
use crate::modules::gifts::components::gift_cosmetic_preview::{GiftCosmeticContent, GiftCosmeticPreview, GiftEquippedBadge};
use crate::modules::gifts::components::gift_pattern_raster::{GiftPatternRaster, GiftPatternRasterRequest};
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftAttributeDTO, RiseGiftInstanceDTO, RiseGiftInstanceExt, RiseGiftMediaDTO, RiseGiftRarityClass,
    RiseGiftUserDTO,
};
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::GiftRecipient;

/// colours of a card, the rolled backdrop if any or the app own card
/// a gift with no backdrop roll shows the default
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GiftBackdropColors {
    pub center: Hsla,
    pub edge: Hsla,
    pub ribbon: Hsla,
    pub ribbon_ink: Hsla,
    pub pattern: Hsla,
    pub is_rolled: bool,
}

static DEFAULT_RIBBON_INK: LazyLock<Hsla> =
    LazyLock::new(|| GiftBackdropColors::ink(GiftRarityStyle::tint(RiseGiftRarityClass::Rare)));

impl GiftBackdropColors {
    pub fn resolve(backdrop: Option<&RiseGiftAttributeDTO>, rarity: RiseGiftRarityClass) -> GiftBackdropColors {
        if let Some(colors) = backdrop.map(|backdrop| &backdrop.colors)
            && let (Some(center), Some(edge)) = (colors.get(gift_colors::CENTER), colors.get(gift_colors::EDGE))
        {
            let ribbon = colors
                .get(gift_colors::RIBBON)
                .map(|hex| color(hex))
                .unwrap_or_else(|| GiftRarityStyle::tint(rarity));
            return GiftBackdropColors {
                center: color(center),
                edge: color(edge),
                ribbon,
                ribbon_ink: Self::ink(ribbon),
                pattern: colors
                    .get(gift_colors::PATTERN)
                    .map(|hex| color(hex))
                    .unwrap_or_else(|| alpha(gpui::black(), 0.35)),
                is_rolled: true,
            };
        }
        let tint = GiftRarityStyle::tint(rarity);
        GiftBackdropColors {
            center: alpha(tint, 0.34),
            edge: alpha(tint, 0.08),
            ribbon: GiftRarityStyle::tint(RiseGiftRarityClass::Rare),
            ribbon_ink: *DEFAULT_RIBBON_INK,
            pattern: alpha(white(), 0.18),
            is_rolled: false,
        }
    }

    /// white while it keeps a 3:1 contrast with the fill, else dark
    pub fn ink(fill: Hsla) -> Hsla {
        let rgba = gpui::Rgba::from(fill);
        fn linear(channel: f32) -> f64 {
            let value = f64::from(channel);
            if value <= 0.03928 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
        }
        let luminance = 0.2126 * linear(rgba.r) + 0.7152 * linear(rgba.g) + 0.0722 * linear(rgba.b);
        if 1.05 / (luminance + 0.05) >= 3.0 { white() } else { alpha(gpui::black(), 0.85) }
    }
}

pub enum GiftRarityStyle {}

impl GiftRarityStyle {
    pub fn tint(rarity: RiseGiftRarityClass) -> Hsla {
        match rarity {
            RiseGiftRarityClass::Common => color("#8A94A6"),
            RiseGiftRarityClass::Uncommon => color("#34C07A"),
            RiseGiftRarityClass::Rare => color("#3F8EF7"),
            RiseGiftRarityClass::Epic => color("#A45CFF"),
            RiseGiftRarityClass::Legendary => color("#F5A623"),
        }
    }

    pub fn title_key(rarity: RiseGiftRarityClass) -> String {
        format!("gift_rarity_{}", rarity.slug())
    }
}

/// serial within the edition as the ribbon and the card title print it
pub enum GiftSerialFormat {}

impl GiftSerialFormat {
    /// serial stays exact since collectors read the number, the edition is a ribbon count
    pub fn ribbon(serial: i64, supply_total: Option<i64>) -> String {
        let Some(total) = supply_total.filter(|total| *total > 0) else {
            return format_printf(&tr("gift_serial"), &[FormatArg::Text(serial.to_string().into())]);
        };
        format_printf(
            &tr("gift_serial_of"),
            &[FormatArg::Text(serial.to_string().into()), FormatArg::Text(Self::ribbon_count(total).into())],
        )
    }

    /// count on a ribbon where every char costs width, eg 950 or 9.7K or 10K or 1.2M
    /// latin K and M after the number in every language and the language decimal separator,
    /// floored so it never reads as more than it is
    pub fn ribbon_count(value: i64) -> String {
        let count = value.max(0);
        if count < 1_000 {
            return count.to_string();
        }
        let (divisor, suffix): (i64, &str) = if count >= 1_000_000 { (1_000_000, "M") } else { (1_000, "K") };
        let tenths = count * 10 / divisor;
        let whole = tenths / 10;
        let fraction = tenths % 10;
        if !(whole < 10 && fraction > 0) {
            return format!("{whole}{suffix}");
        }
        format!("{whole}{}{fraction}{suffix}", RiseFormat::decimal_separator())
    }

    pub fn percent_ppm(ppm: i64) -> String {
        Self::percent_share(ppm as f64 / 1_000_000.0)
    }

    pub fn percent_share(share: f64) -> String {
        let value = share * 100.0;
        let template = if value >= 10.0 {
            "%.0f%%"
        } else if value >= 1.0 {
            "%.1f%%"
        } else if value >= 0.01 {
            "%.2f%%"
        } else {
            "%.3f%%"
        };
        format_printf(template, &[FormatArg::Float(value)])
    }
}

/// a gift on its card, bottom to top
/// gradient then pattern rings then frame then model then effect then the N of M ribbon
/// model and effect share one camera so they line up
#[derive(IntoElement)]
pub struct GiftCardView {
    id: ElementId,
    media: Option<RiseGiftMediaDTO>,
    backdrop: Option<RiseGiftAttributeDTO>,
    pattern: Option<RiseGiftAttributeDTO>,
    frame: Option<RiseGiftAttributeDTO>,
    effect: Option<RiseGiftAttributeDTO>,
    rarity: RiseGiftRarityClass,
    ribbon: Option<String>,
    animated: bool,
    large: bool,
    corner_radius: f32,
    cosmetic: Option<GiftCosmeticContent>,
    size_hint: Option<Size<f32>>,
}

impl GiftCardView {
    pub fn new(id: impl Into<ElementId>, media: Option<RiseGiftMediaDTO>) -> Self {
        Self {
            id: id.into(),
            media,
            backdrop: None,
            pattern: None,
            frame: None,
            effect: None,
            rarity: RiseGiftRarityClass::Common,
            ribbon: None,
            animated: true,
            large: true,
            corner_radius: 22.0,
            cosmetic: None,
            size_hint: None,
        }
    }

    /// an instance with every layer it rolled
    /// a cosmetic is worn by person, a nameplate without the person row over it when bare_nameplate
    pub fn instance(
        id: impl Into<ElementId>,
        instance: &RiseGiftInstanceDTO,
        person: Option<GiftRecipient>,
        bare_nameplate: bool,
    ) -> Self {
        let cosmetic = instance.is_cosmetic().then(|| GiftCosmeticContent {
            kind: instance.kind(),
            model: instance.model().and_then(|model| model.media.clone()),
            effect: instance.effect().and_then(|effect| effect.media.clone()),
            person,
            bare_nameplate,
        });
        Self {
            backdrop: instance.backdrop().cloned(),
            pattern: instance.pattern().cloned(),
            frame: instance.frame().cloned(),
            effect: instance.effect().cloned(),
            rarity: instance.rarity_class(),
            cosmetic,
            ..Self::new(id, instance.model().and_then(|model| model.media.clone()))
        }
    }

    pub fn backdrop(mut self, backdrop: Option<RiseGiftAttributeDTO>) -> Self {
        self.backdrop = backdrop;
        self
    }

    pub fn pattern(mut self, pattern: Option<RiseGiftAttributeDTO>) -> Self {
        self.pattern = pattern;
        self
    }

    pub fn frame(mut self, frame: Option<RiseGiftAttributeDTO>) -> Self {
        self.frame = frame;
        self
    }

    pub fn effect(mut self, effect: Option<RiseGiftAttributeDTO>) -> Self {
        self.effect = effect;
        self
    }

    pub fn rarity(mut self, rarity: RiseGiftRarityClass) -> Self {
        self.rarity = rarity;
        self
    }

    pub fn ribbon(mut self, ribbon: Option<String>) -> Self {
        self.ribbon = ribbon;
        self
    }

    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }

    pub fn large(mut self, large: bool) -> Self {
        self.large = large;
        self
    }

    pub fn corner_radius(mut self, corner_radius: f32) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    pub fn cosmetic(mut self, cosmetic: Option<GiftCosmeticContent>) -> Self {
        self.cosmetic = cosmetic;
        self
    }

    /// size the parent gives the card, its first render draws at it
    pub fn size_hint(mut self, size_hint: Option<Size<f32>>) -> Self {
        self.size_hint = size_hint;
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }
}

/// size laid out last frame for a component, re-renders when it moves
pub(crate) fn geometry_size(id: ElementId, window: &mut Window, cx: &mut App) -> (Size<f32>, impl IntoElement + use<>) {
    geometry_size_seeded(id, None, window, cx)
}

pub(crate) fn same_within_snap(a: Size<f32>, b: Size<f32>) -> bool {
    (a.width - b.width).abs() <= 1.0 && (a.height - b.height).abs() <= 1.0
}

#[derive(Default)]
struct GeometrySizeState {
    size: Size<f32>,
    seed: Option<Size<f32>>,
    measured: Option<Size<f32>>,
}

/// same as geometry_size but the first render already has the size the parent knows
/// so a fixed grid cell draws its card at once and not a frame later
pub(crate) fn geometry_size_seeded(
    id: ElementId,
    seed: Option<Size<f32>>,
    window: &mut Window,
    cx: &mut App,
) -> (Size<f32>, impl IntoElement + use<>) {
    let state = window.use_keyed_state(id.clone(), cx, move |_, _| GeometrySizeState {
        size: seed.unwrap_or_default(),
        seed,
        measured: None,
    });
    let size = state.read(cx).size;
    let reader = measure(ElementId::NamedChild(Arc::new(id), "measure".into()), move |bounds, _, cx| {
        let measured = Size { width: f32::from(bounds.size.width), height: f32::from(bounds.size.height) };
        state.update(cx, |state, cx| {
            if state.measured == Some(measured) {
                return;
            }
            state.measured = Some(measured);
            let next = match state.seed {
                Some(seed) if same_within_snap(seed, measured) => seed,
                _ => measured,
            };
            if state.size != next {
                state.size = next;
                cx.notify();
            }
        });
    })
    .absolute()
    .inset_0();
    (size, reader)
}

impl RenderOnce for GiftCardView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = GiftBackdropColors::resolve(self.backdrop.as_ref(), self.rarity);
        let (size, reader) = geometry_size_seeded(self.child_id("geometry"), self.size_hint, window, cx);
        let side = size.width.min(size.height);
        let inset = side * if self.large { 0.05 } else { 0.02 };
        let inner_hint = self
            .size_hint
            .map(|_| Size { width: (size.width - 2.0 * inset).max(0.0), height: (size.height - 2.0 * inset).max(0.0) });
        let layer = || div().absolute().inset_0();

        let mut stack = div().relative().size_full().child(reader).child(
            layer().child(
                RadialGradient::new([colors.center, colors.edge], UnitPoint::new(0.5, 0.42), 0.0, side * 0.75).view(),
            ),
        );
        if let Some(symbol) = self.pattern.as_ref().map(|pattern| pattern.image_url.clone())
            && !symbol.is_empty()
        {
            let strength = if self.cosmetic.is_none() { 1.0 } else { GiftPatternLayer::COSMETIC_STRENGTH };
            stack = stack.child(layer().child(
                GiftPatternLayer::new(self.child_id("pattern"), symbol, colors.pattern)
                    .strength(strength)
                    .size_hint(self.size_hint.map(|_| size)),
            ));
        }
        if let Some(cosmetic) = self.cosmetic.clone() {
            stack = stack.child(
                layer().p(px(side * 0.07)).child(
                    GiftCosmeticPreview::new(self.child_id("cosmetic"), cosmetic.kind, cosmetic.person, cosmetic.model)
                        .effect(cosmetic.effect)
                        .animated(self.animated)
                        .bare_nameplate(cosmetic.bare_nameplate),
                ),
            );
        } else {
            if let Some(picture) = self.frame.as_ref().map(|frame| frame.image_url.clone())
                && !picture.is_empty()
            {
                stack = stack.child(
                    layer().child(
                        CleverImageUi::new(picture)
                            .id(self.child_id("frame"))
                            .content_mode(ContentMode::Fit)
                            .corner_radius(0.0)
                            .persist_to_disk(true)
                            .shows_progress_indicator(false)
                            .maximum_decode_pixel_dimension(if self.large { 1_024 } else { 512 })
                            .shows_failure_icon(false)
                            .placeholder_color(transparent_black()),
                    ),
                );
            }
            stack = stack.child(layer().p(px(inset)).child(
                GiftAnimationView::new(self.child_id("model"), self.media.clone())
                    .large(self.large)
                    .animated(self.animated)
                    .size_hint(inner_hint),
            ));
            if let Some(effect) = self.effect.as_ref().and_then(|effect| effect.media.clone()) {
                stack = stack.child(
                    layer().p(px(inset)).child(
                        pass_through().child(
                            GiftAnimationView::new(self.child_id("effect"), Some(effect))
                                .large(self.large)
                                .animated(self.animated)
                                .size_hint(inner_hint),
                        ),
                    ),
                );
            }
        }

        let mut card = zstack(Alignment::TopTrailing).size_full().child(stack);
        if let Some(ribbon) = self.ribbon.clone() {
            let large = self.large;
            card = card.child(
                div()
                    .p(px(if large { 12.0 } else { 6.0 }))
                    .child(
                        div()
                            .swiftui_font(Font::system_weight(if large { 13.0 } else { 10.5 }, FontWeight::SEMIBOLD).monospaced_digit())
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(colors.ribbon_ink)
                            .px(px(if large { 9.0 } else { 6.0 }))
                            .py(px(if large { 4.0 } else { 2.0 }))
                            .capsule()
                            .bg(colors.ribbon)
                            .child(SharedString::from(ribbon)),
                    ),
            );
        }
        card.clip_shape(Shape::continuous(self.corner_radius))
    }
}

/// pattern symbol in three rings around the model, tinted with the backdrop pattern colour
/// strength scales every ring opacity, one bitmap drawn ahead and shared from cache
#[derive(IntoElement)]
pub struct GiftPatternLayer {
    id: ElementId,
    symbol_url: String,
    color: Hsla,
    strength: f64,
    size_hint: Option<Size<f32>>,
}

#[derive(Default)]
struct GiftPatternLayerDrawn {
    drawn: Option<CleverImage>,
    drawn_key: Option<String>,
    swap_window: Option<AnyWindowHandle>,
    placeholder: Option<Arc<RenderImage>>,
}

impl GiftPatternLayer {
    pub const COSMETIC_STRENGTH: f64 = 0.4;

    pub fn new(id: impl Into<ElementId>, symbol_url: impl Into<String>, color: Hsla) -> Self {
        Self { id: id.into(), symbol_url: symbol_url.into(), color, strength: 1.0, size_hint: None }
    }

    pub fn strength(mut self, strength: f64) -> Self {
        self.strength = strength;
        self
    }

    /// layer size when the card knows it so the pattern is drawn for it from the first render
    pub fn size_hint(mut self, size_hint: Option<Size<f32>>) -> Self {
        self.size_hint = size_hint;
        self
    }
}

impl RenderOnce for GiftPatternLayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let display_scale = window.scale_factor();
        let (size, reader) = geometry_size_seeded(
            ElementId::NamedChild(Arc::new(self.id.clone()), "geometry".into()),
            self.size_hint,
            window,
            cx,
        );
        let request = GiftPatternRasterRequest::new(&self.symbol_url, self.color, self.strength, size, display_scale);
        let drawn_state = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), "drawn".into()),
            cx,
            |_, _| GiftPatternLayerDrawn::default(),
        );
        let image = request.as_ref().and_then(|request| {
            GiftPatternRaster::cached(request).or_else(|| {
                let drawn = drawn_state.read(cx);
                (drawn.drawn_key.as_deref() == Some(request.cache_key.as_str()))
                    .then(|| drawn.drawn.clone())
                    .flatten()
            })
        });
        let draw_request = request.clone().filter(|_| image.is_none());
        let swap_key = (drawn_state.entity_id().as_u64() as u32).wrapping_add(1);
        let placeholder = match (&image, &request) {
            (None, Some(request)) => pattern_placeholder(request.pixel_width, request.pixel_height),
            _ => None,
        };
        let window_handle = window.window_handle();
        drawn_state.update(cx, |state, _| {
            state.swap_window = Some(window_handle);
            state.placeholder = placeholder.clone();
        });
        let task_state = drawn_state.clone();
        let task_request = draw_request.clone();
        use_task(
            ElementId::NamedChild(Arc::new(self.id.clone()), "draw".into()),
            draw_request,
            window,
            cx,
            async move |cx| {
                let Some(request) = task_request else {
                    return;
                };
                let cache_key = request.cache_key.clone();
                let Some(image) = GiftPatternRaster::image(request, cx).await else {
                    return;
                };
                task_state.update(cx, |state, cx| {
                    if state.drawn.as_ref().is_some_and(|drawn| Arc::ptr_eq(drawn, &image)) {
                        return;
                    }
                    let swapped = match (state.swap_window, state.placeholder.as_ref()) {
                        (Some(handle), Some(placeholder)) => handle
                            .update(cx, |_, window, _| window.swap_image(swap_key, placeholder, &image, 0))
                            .unwrap_or(false),
                        _ => false,
                    };
                    state.drawn = Some(image);
                    state.drawn_key = Some(cache_key);
                    if !swapped {
                        cx.notify();
                    }
                });
            },
        );
        let mut layer = div().relative().size_full().child(reader);
        if let Some(picture) = image.or(placeholder) {
            layer = layer.child(GiftPatternSwapKeyed {
                key: swap_key,
                child: img(picture).absolute().inset_0().size_full().into_any_element(),
            });
        }
        pass_through().size_full().child(layer)
    }
}

thread_local! {
    static PATTERN_PLACEHOLDERS: RefCell<HashMap<(i64, i64), Arc<RenderImage>>> = RefCell::new(HashMap::new());
}

fn pattern_placeholder(pixel_width: i64, pixel_height: i64) -> Option<Arc<RenderImage>> {
    let key = (pixel_width, pixel_height);
    if let Some(image) = PATTERN_PLACEHOLDERS.with(|images| images.borrow().get(&key).cloned()) {
        return Some(image);
    }
    let width = u32::try_from(pixel_width).ok().filter(|width| *width > 0)?;
    let height = u32::try_from(pixel_height).ok().filter(|height| *height > 0)?;
    let bitmap = RgbaImage::new(width, height);
    let frame = Frame::from_parts(bitmap, 0, 0, Delay::from_numer_denom_ms(0, 1));
    let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([frame]);
    let image = Arc::new(RenderImage::new(frames));
    PATTERN_PLACEHOLDERS.with(|images| images.borrow_mut().insert(key, image.clone()));
    Some(image)
}

struct GiftPatternSwapKeyed {
    key: u32,
    child: AnyElement,
}

impl IntoElement for GiftPatternSwapKeyed {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GiftPatternSwapKeyed {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_image_swap_key(self.key, |window| self.child.paint(window, cx));
    }
}

/// pattern symbols by url kept in memory since a tab repeats one symbol on many cards
pub struct GiftSymbolImages {
    cache: NSCache<String, CleverImage>,
}

static GIFT_SYMBOL_IMAGES: LazyLock<GiftSymbolImages> = LazyLock::new(|| GiftSymbolImages { cache: NSCache::new() });

impl GiftSymbolImages {
    pub fn shared() -> &'static GiftSymbolImages {
        &GIFT_SYMBOL_IMAGES
    }

    pub async fn image(&self, url: &str, cx: &AsyncApp) -> Option<CleverImage> {
        if let Some(cached) = self.cache.object(&url.to_owned()) {
            return Some(cached);
        }
        let image = CleverImageBitmapLoader::load(
            url,
            false,
            true,
            SaiAccountCacheScope::active_user_id(),
            SaiAccountCacheScope::active_epoch(),
            None,
            CleverImageCachePolicy::MemoryOnly,
            CleverImageWorkLane::Visible,
            cx,
        )
        .await?;
        self.cache.set_object(image.clone(), url.to_owned());
        Some(image)
    }
}

#[derive(IntoElement)]
pub struct GiftRarityPill {
    rarity: RiseGiftRarityClass,
    chance_ppm: Option<i64>,
}

impl GiftRarityPill {
    pub fn new(rarity: RiseGiftRarityClass) -> Self {
        Self { rarity, chance_ppm: None }
    }

    pub fn chance_ppm(mut self, chance_ppm: Option<i64>) -> Self {
        self.chance_ppm = chance_ppm;
        self
    }
}

impl RenderOnce for GiftRarityPill {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tint = GiftRarityStyle::tint(self.rarity);
        let mut row = hstack(4.0).child(
            div()
                .swiftui_font(Font::system_weight(12.0, FontWeight::SEMIBOLD))
                .child(SharedString::from(tr(&GiftRarityStyle::title_key(self.rarity)))),
        );
        if let Some(chance_ppm) = self.chance_ppm
            && chance_ppm > 0
        {
            row = row.child(
                div()
                    .swiftui_font(Font::system_weight(12.0, FontWeight::MEDIUM).monospaced_digit())
                    .opacity(0.8)
                    .child(SharedString::from(GiftSerialFormat::percent_ppm(chance_ppm))),
            );
        }
        row.text_color(tint).px(px(9.0)).py(px(4.0)).capsule().bg(alpha(tint, 0.14))
    }
}

pub enum GiftsGridMetrics {}

impl GiftsGridMetrics {
    pub const COLUMNS: i64 = 3;
    pub const SPACING: f32 = 8.0;
    pub const HORIZONTAL_PADDING: f32 = 12.0;
    pub const MAX_CELL_SIDE: f32 = 200.0;

    /// columns for a container width, three at phone and panel widths and more once tiles would
    /// grow past max cell side
    pub fn columns(container_width: f32) -> i64 {
        let inner = container_width - Self::HORIZONTAL_PADDING * 2.0;
        let fitted = ((inner + Self::SPACING) / (Self::MAX_CELL_SIDE + Self::SPACING)).ceil();
        if fitted.is_finite() { (fitted as i64).max(Self::COLUMNS) } else { Self::COLUMNS }
    }

    pub fn cell_side(container_width: f32) -> f32 {
        let columns = Self::columns(container_width);
        let gutters = Self::SPACING * (columns - 1) as f32 + Self::HORIZONTAL_PADDING * 2.0;
        ((container_width - gutters) / columns as f32).floor().max(1.0)
    }

    pub fn grid_height(item_count: i64, container_width: f32) -> f32 {
        let columns = Self::columns(container_width);
        let rows = (item_count as f64 / columns as f64).ceil() as i64;
        if rows <= 0 {
            return 0.0;
        }
        let side = Self::cell_side(container_width);
        rows as f32 * side + (rows - 1) as f32 * Self::SPACING
    }
}

/// one gift in a profile grid, plays every rolled layer once in view
/// edition band top right, giver top left unless anonymous
/// pin bottom right, worn check for owner
#[derive(IntoElement)]
pub struct GiftGridCell {
    instance: RiseGiftInstanceDTO,
    side: f32,
    shows_owner_marks: bool,
    person: Option<GiftRecipient>,
}

impl GiftGridCell {
    const CORNER_RADIUS: f32 = 16.0;

    pub fn new(instance: RiseGiftInstanceDTO, side: f32, shows_owner_marks: bool) -> Self {
        Self { instance, side, shows_owner_marks, person: None }
    }

    pub fn person(mut self, person: Option<GiftRecipient>) -> Self {
        self.person = person;
        self
    }

    fn visible_giver(&self) -> Option<&RiseGiftUserDTO> {
        if self.instance.is_grant() || self.instance.anonymous {
            return None;
        }
        let sender = self.instance.sender.as_ref()?;
        (sender.id != self.instance.owner_user_id).then_some(sender)
    }
}

impl RenderOnce for GiftGridCell {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let instance = &self.instance;
        let colors = GiftBackdropColors::resolve(instance.backdrop(), instance.rarity_class());
        let shape = Shape::continuous(Self::CORNER_RADIUS);
        let shows_worn = self.shows_owner_marks && instance.is_cosmetic() && instance.is_equipped();
        let id = ElementId::Name(format!("gift-grid-cell-{}", instance.id).into());
        let child_id = |name: &'static str| ElementId::NamedChild(Arc::new(id.clone()), name.into());
        let overlay = || div().absolute().inset_0();

        let mut cell = div()
            .id(id.clone())
            .relative()
            .frame_size(self.side, self.side)
            .cursor_pointer()
            .hover(|style| style.opacity(0.92))
            .child(
                GiftCardView::instance(child_id("card"), instance, self.person.clone(), true)
                    .large(false)
                    .corner_radius(Self::CORNER_RADIUS)
                    .size_hint(Some(Size { width: self.side, height: self.side })),
            );
        if instance.is_numbered() {
            cell = cell.child(overlay().child(GiftCornerRibbon::new(
                child_id("ribbon"),
                GiftSerialFormat::ribbon(instance.serial, Some(instance.supply_total)),
                colors.ribbon,
                colors.ribbon_ink,
            )
            .size_hint(Some(Size { width: self.side, height: self.side }))));
        }
        if let Some(giver) = self.visible_giver() {
            cell = cell.child(
                zstack(Alignment::TopLeading).absolute().inset_0().child(
                    div().p(px(6.0)).child(
                        UserLogoUi::new(child_id("giver"))
                            .source(Some(SharedString::from(giver.avatar.clone())))
                            .name(Some(SharedString::from(giver.name.clone())))
                            .user_id(Some(SharedString::from(giver.id.clone())))
                            .size(22.0)
                            .show_story_ring(false),
                    ),
                ),
            );
        }
        if instance.is_pinned() || shows_worn {
            let mut marks = hstack(4.0);
            if instance.is_pinned() {
                let mut pin = div().size(px(22.0)).flex().items_center().justify_center().circle().bg(alpha(gpui::black(), 0.42));
                if let Some(icon) = IconUi::sized("pin.fill", px(10.0), white(), true) {
                    pin = pin.child(icon.with_transformation(Transformation::rotate(radians(45f32.to_radians()))));
                }
                marks = marks.child(pin);
            }
            if shows_worn {
                marks = marks.child(GiftEquippedBadge::new().compact(true));
            }
            cell = cell.child(zstack(Alignment::BottomTrailing).absolute().inset_0().child(div().p(px(6.0)).child(marks)));
        }
        if self.shows_owner_marks && !instance.is_displayed() {
            let mut dimmed = div().size_full().flex().items_center().justify_center();
            dimmed = shape.round(dimmed).bg(alpha(gpui::black(), 0.38));
            if let Some(icon) = IconUi::sized("eye.slash.fill", px(18.0), alpha(white(), 0.9), true) {
                dimmed = dimmed.child(icon);
            }
            cell = cell.child(overlay().child(dimmed));
        }
        cell.clip_shape(shape)
    }
}

enum GiftCornerRibbonFont {}

impl GiftCornerRibbonFont {
    const SIZE: f32 = 14.0;

    fn measure() -> Font {
        Font::system_weight(Self::SIZE, FontWeight::BOLD).monospaced_digit()
    }
}

pub enum GiftCornerRibbonFill {
    Color(Hsla),
    Gradient(LinearGradient),
}

impl From<Hsla> for GiftCornerRibbonFill {
    fn from(color: Hsla) -> Self {
        Self::Color(color)
    }
}

impl From<LinearGradient> for GiftCornerRibbonFill {
    fn from(gradient: LinearGradient) -> Self {
        Self::Gradient(gradient)
    }
}

/// band across the top right corner of a card like the shop marks a numbered or discounted item
/// the card clips it and the text runs along the visible middle
#[derive(IntoElement)]
pub struct GiftCornerRibbon {
    id: ElementId,
    text: String,
    fill: GiftCornerRibbonFill,
    ink: Hsla,
    inset: f32,
    band_height: f32,
    size_hint: Option<Size<f32>>,
}

impl GiftCornerRibbon {
    pub fn new(id: impl Into<ElementId>, text: impl Into<String>, fill: impl Into<GiftCornerRibbonFill>, ink: Hsla) -> Self {
        Self { id: id.into(), text: text.into(), fill: fill.into(), ink, inset: 25.0, band_height: 21.0, size_hint: None }
    }

    /// card size when the parent knows it so the band is placed from the first render
    pub fn size_hint(mut self, size_hint: Option<Size<f32>>) -> Self {
        self.size_hint = size_hint;
        self
    }

    pub fn inset(mut self, inset: f32) -> Self {
        self.inset = inset;
        self
    }

    pub fn band_height(mut self, band_height: f32) -> Self {
        self.band_height = band_height;
        self
    }

    fn natural_width(text: &str, window: &Window) -> f32 {
        let font = GiftCornerRibbonFont::measure();
        let run = TextRun {
            len: text.len(),
            font: gpui::Font {
                family: font.family(),
                features: font.features(),
                fallbacks: None,
                weight: font.weight,
                style: font.style(),
            },
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        f32::from(
            window
                .text_system()
                .shape_line(SharedString::from(text.to_owned()), px(font.size), &[run], None)
                .width(),
        )
    }
}

impl RenderOnce for GiftCornerRibbon {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let cut_ends = self.band_height * 0.45;
        let natural = Self::natural_width(&self.text, window);
        let needed = (natural * 0.75 + cut_ends) / (2.0 * 2f32.sqrt());
        let band_inset = self.inset.max(needed.ceil()).min(self.inset * 1.5);
        let visible_length = 2.0 * band_inset * 2f32.sqrt() - cut_ends;
        let (size, reader) = geometry_size_seeded(
            ElementId::NamedChild(Arc::new(self.id.clone()), "geometry".into()),
            self.size_hint,
            window,
            cx,
        );
        let band_width = visible_length + self.band_height * 4.0;
        let band = match self.fill {
            GiftCornerRibbonFill::Color(fill) => div().size_full().bg(fill).into_any_element(),
            GiftCornerRibbonFill::Gradient(gradient) => gradient.view().into_any_element(),
        };
        let label = div()
            .swiftui_font(GiftCornerRibbonFont::measure())
            .text_color(self.ink)
            .frame_size(visible_length, self.band_height)
            .flex()
            .flex_col()
            .justify_center()
            .text_center()
            .child(MinimumScaleText::new(self.text.clone(), 0.6).line_limit(1));
        let ribbon = zstack(Alignment::Center)
            .frame_size(band_width, self.band_height)
            .child(div().frame_size(band_width, self.band_height).child(band))
            .child(label)
            .rotation_effect(45.0);
        let center_x = size.width - band_inset;
        let center_y = band_inset;
        pass_through().size_full().child(
            div().relative().size_full().child(reader).child(
                div()
                    .absolute()
                    .left(px(center_x - band_width / 2.0))
                    .top(px(center_y - self.band_height / 2.0))
                    .child(ribbon),
            ),
        )
    }
}
