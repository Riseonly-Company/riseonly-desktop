use std::sync::{Arc, LazyLock};

use gpui::{App, FontWeight, Hsla, ObjectFit, Pixels, RenderImage, SharedString, Size, Window, hsla, img, rgb};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_theme::{AvatarPalette, RiseThemeAppearance};
use rise_ui::qr_code::rise_qr_code::{RiseQRCodeBitmap, RiseQRCodeStyle};
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::minimum_scale_factor::MinimumScaleText;
use rise_ui::swiftui::prelude::*;
use smallvec::SmallVec;

use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatWallpaperRasterFrame;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::{
    ChatWallpaperPreset, ChatWallpaperStore,
};

/// what the code card prints
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseQRCodeSubject {
    pub link: Url,
    pub tag: Option<String>,
    pub title: String,
    pub avatar_url: Option<String>,
    pub identity_seed: Option<String>,
}

impl RiseQRCodeSubject {
    pub fn new(
        link: Url,
        tag: Option<String>,
        title: String,
        avatar_url: Option<String>,
        identity_seed: Option<String>,
    ) -> Self {
        Self { link, tag, title, avatar_url, identity_seed }
    }

    /// printed under the code, the tag is shorter and recognized at a glance
    /// a community without a tag shows its name like telegram and the address only when it has none
    pub fn caption(&self) -> String {
        if let Some(tag) = &self.tag {
            return tag.clone();
        }
        let name = trimming_whitespaces_and_newlines(&self.title);
        if name.is_empty() { Self::host(&self.link) } else { name.to_owned() }
    }

    fn host(link: &Url) -> String {
        let mut text = link.absolute_string().to_owned();
        for prefix in ["https://", "http://"] {
            if text.starts_with(prefix) {
                text.replace_range(..prefix.len(), "");
            }
        }
        text
    }
}

/// code screen styling, chat wallpaper plus module gradient
/// wallpaper is reused from chat on purpose, it has a raster cache and a light variant
/// another set of backgrounds would be a second thing to maintain
#[derive(Clone, Debug, PartialEq)]
pub struct RiseQRCodeTheme {
    pub id: String,
    pub gradient: Vec<Hsla>,
}

static ALL: LazyLock<Vec<RiseQRCodeTheme>> = LazyLock::new(|| {
    vec![
        RiseQRCodeTheme::new("telegram", &[0x2E5CE6, 0x7A2BC0]),
        RiseQRCodeTheme::new("nightBlue", &[0x1E63D8, 0x2A3F9E]),
        RiseQRCodeTheme::new("violet", &[0x6A3DE8, 0xB23AA8]),
        RiseQRCodeTheme::new("ember", &[0xE0552B, 0xB02A6E]),
        RiseQRCodeTheme::new("ocean", &[0x0E8FC4, 0x1E4E9C]),
        RiseQRCodeTheme::new("forest", &[0x1E9B62, 0x4E7A2A]),
    ]
});

impl RiseQRCodeTheme {
    fn new(id: &str, gradient: &[u32]) -> Self {
        Self { id: id.to_owned(), gradient: gradient.iter().map(|&value| Self::hex(value)).collect() }
    }

    /// theme wallpaper for the lightness picked on screen
    /// most chat presets have no light variant so resolving for light gives the same dark colors
    /// so the light backdrop is derived from the theme gradient and the pattern is laid down black
    pub fn preset(&self, for_appearance: RiseThemeAppearance) -> ChatWallpaperPreset {
        if for_appearance == RiseThemeAppearance::Light { self.light_preset() } else { self.dark_preset() }
    }

    pub fn dark_preset(&self) -> ChatWallpaperPreset {
        ChatWallpaperPreset::all()
            .iter()
            .find(|preset| preset.id == self.id)
            .unwrap_or_else(ChatWallpaperPreset::default_preset)
            .clone()
    }

    fn light_preset(&self) -> ChatWallpaperPreset {
        let source = if self.gradient.is_empty() { RiseQRCodeStyle::fallback().gradient } else { self.gradient.clone() };
        let stops: Vec<String> = ChatWallpaperPreset::LIGHT_STOP_LIFTS
            .iter()
            .enumerate()
            .map(|(index, &lift)| ChatWallpaperPreset::pale_hex(source[index % source.len()], lift))
            .collect();
        let pattern: Vec<String> =
            ChatWallpaperPreset::DEFAULT_LIGHT_PATTERN_COLORS.iter().map(|&color| color.to_owned()).collect();
        ChatWallpaperPreset::new(
            format!("{}-light", self.id),
            stops.clone(),
            pattern.clone(),
            Some(stops),
            Some(pattern),
        )
    }

    pub fn style(&self) -> RiseQRCodeStyle {
        RiseQRCodeStyle { gradient: self.gradient.clone(), background: gpui::white() }
    }

    pub fn accent(&self) -> Hsla {
        self.gradient.first().copied().unwrap_or_else(|| RiseQRCodeStyle::fallback().gradient[0])
    }

    pub fn all() -> &'static [RiseQRCodeTheme] {
        &ALL
    }

    pub fn fallback() -> &'static RiseQRCodeTheme {
        &ALL[0]
    }

    pub fn theme(id: &str) -> RiseQRCodeTheme {
        Self::all().iter().find(|theme| theme.id == id).unwrap_or_else(Self::fallback).clone()
    }

    pub fn initial_id(cx: &mut App) -> String {
        let selected = ChatWallpaperStore::shared(cx).read(cx).selected_preset_id();
        if Self::all().iter().any(|theme| theme.id == selected) { selected } else { Self::fallback().id.clone() }
    }

    fn hex(value: u32) -> Hsla {
        rgb(value).into()
    }
}

pub enum RiseQRCodeCardMetrics {}

impl RiseQRCodeCardMetrics {
    pub const PADDING_RATIO: f32 = 0.125;
    pub const CORNER_RADIUS_RATIO: f32 = 0.125;
    pub const AVATAR_RATIO: f32 = 0.27;
    pub const AVATAR_CODE_GAP_RATIO: f32 = 0.03;
    pub const CANVAS_CARD_RATIO: f32 = 0.70;

    pub fn padding(card_width: f32) -> f32 {
        card_width * Self::PADDING_RATIO
    }

    pub fn code_side(card_width: f32) -> f32 {
        (card_width - Self::padding(card_width) * 2.0).max(0.0)
    }

    /// how far the avatar sits above the card, measured from the card padding and not half the avatar
    /// the part below the card top edge must fit in the padding without reaching the code
    /// half an avatar is bigger than the padding
    pub fn avatar_overlap(card_width: f32, avatar: f32) -> f32 {
        let clearance = Self::padding(card_width) - card_width * Self::AVATAR_CODE_GAP_RATIO;
        (avatar / 2.0).max(avatar - clearance)
    }
}

/// card without the avatar
/// one layer is drawn on screen and rendered into the share image so nothing inside is async
/// logo is already baked into the code raster
#[derive(IntoElement)]
pub struct RiseQRCodeCardFace {
    pub image: Option<Arc<RenderImage>>,
    pub caption: String,
    pub theme: RiseQRCodeTheme,
    pub width: f32,
}

impl RiseQRCodeCardFace {
    pub(crate) const CODE_CAPTION_SPACING: f32 = 14.0;
    pub(crate) const PLACEHOLDER_CORNER_RADIUS: f32 = 12.0;
    pub(crate) const CAPTION_MINIMUM_SCALE_FACTOR: f32 = 0.5;
    pub(crate) const CAPTION_LINE_LIMIT: usize = 2;

    pub fn new(image: Option<Arc<RenderImage>>, caption: String, theme: RiseQRCodeTheme, width: f32) -> Self {
        Self { image, caption, theme, width }
    }

    pub(crate) fn placeholder_fill() -> Hsla {
        hsla(0.0, 0.0, 0.93, 1.0)
    }

    pub(crate) fn caption_font_size(width: f32) -> f32 {
        15.0_f32.max(width * 0.068)
    }

    pub fn code_image(bitmap: &RiseQRCodeBitmap) -> Option<Arc<RenderImage>> {
        let mut bgra = bitmap.rgba.clone();
        for pixel in bgra.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let buffer = image::RgbaImage::from_raw(bitmap.width, bitmap.height, bgra)?;
        let frames: SmallVec<[image::Frame; 1]> = SmallVec::from_buf([image::Frame::new(buffer)]);
        Some(Arc::new(RenderImage::new(frames)))
    }

    fn padding(&self) -> f32 {
        RiseQRCodeCardMetrics::padding(self.width)
    }

    fn side(&self) -> f32 {
        RiseQRCodeCardMetrics::code_side(self.width)
    }
}

impl RenderOnce for RiseQRCodeCardFace {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let side = self.side();
        let padding = self.padding();
        let code: AnyElement = match self.image.clone() {
            Some(image) => img(image).size(px(side)).object_fit(ObjectFit::Fill).into_any_element(),
            None => div()
                .size(px(side))
                .continuous_corners(Self::PLACEHOLDER_CORNER_RADIUS)
                .bg(Self::placeholder_fill())
                .into_any_element(),
        };
        vstack(Self::CODE_CAPTION_SPACING)
            .child(div().size(px(side)).child(code))
            .child(
                div()
                    .w(px(side))
                    .text_center()
                    .text_color(self.theme.accent())
                    .swiftui_font(Font::system_weight(Self::caption_font_size(self.width), FontWeight::BOLD))
                    .child(
                        MinimumScaleText::new(SharedString::from(self.caption), Self::CAPTION_MINIMUM_SCALE_FACTOR)
                            .line_limit(Self::CAPTION_LINE_LIMIT),
                    ),
            )
            .p(px(padding))
            .bg(self.theme.style().background)
            .continuous_corners(self.width * RiseQRCodeCardMetrics::CORNER_RADIUS_RATIO)
            .w(px(self.width))
    }
}

/// avatar above the card with no async load, a ready image or a letter
/// the screen uses the user logo which can wait on the network but the share raster has nothing
/// to wait for and waiting would render an empty circle
#[derive(IntoElement)]
pub struct RiseQRCodeAvatar {
    pub image: Option<Arc<RenderImage>>,
    pub title: String,
    pub identity_seed: Option<String>,
    pub size: f32,
}

impl RiseQRCodeAvatar {
    pub(crate) const LETTER_FRAME_RATIO: f32 = 0.72;
    pub(crate) const LETTER_MINIMUM_SCALE_FACTOR: f32 = 0.55;

    pub fn new(image: Option<Arc<RenderImage>>, title: String, identity_seed: Option<String>, size: f32) -> Self {
        Self { image, title, identity_seed, size }
    }

    pub(crate) fn letter_font_size(size: f32) -> f32 {
        (size * 0.46).max(11.0)
    }

    pub(crate) fn fill(&self) -> Hsla {
        AvatarPalette::color_for_user(self.identity_seed.as_deref(), Some(&self.title))
    }

    pub(crate) fn letter(&self) -> String {
        AvatarPalette::display_letter(Some(&self.title))
    }
}

impl RenderOnce for RiseQRCodeAvatar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let size = self.size;
        let content: AnyElement = match self.image.clone() {
            Some(image) => img(image)
                .size(px(size))
                .object_fit(ContentMode::Fill.object_fit())
                .into_any_element(),
            None => zstack(Alignment::Center)
                .size(px(size))
                .bg(self.fill())
                .child(
                    div()
                        .size(px(size * Self::LETTER_FRAME_RATIO))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_center()
                        .text_color(AvatarPalette::letter_color_on(self.fill()))
                        .swiftui_font(Font::system_weight(Self::letter_font_size(size), FontWeight::SEMIBOLD))
                        .child(
                            MinimumScaleText::new(SharedString::from(self.letter()), Self::LETTER_MINIMUM_SCALE_FACTOR)
                                .line_limit(1),
                        ),
                )
                .into_any_element(),
        };
        div().size(px(size)).rounded_full().overflow_hidden().child(content)
    }
}

/// share sheet export layer, wallpaper avatar and card exactly as on screen with no async images
#[derive(IntoElement)]
pub struct RiseQRCodeShareCanvas {
    pub image: Option<Arc<RenderImage>>,
    pub avatar: Option<Arc<RenderImage>>,
    pub subject: RiseQRCodeSubject,
    pub theme: RiseQRCodeTheme,
    pub appearance: RiseThemeAppearance,
    pub pattern_intensity: f64,
    pub size: Size<Pixels>,
}

impl RiseQRCodeShareCanvas {
    pub fn new(
        image: Option<Arc<RenderImage>>,
        avatar: Option<Arc<RenderImage>>,
        subject: RiseQRCodeSubject,
        theme: RiseQRCodeTheme,
        appearance: RiseThemeAppearance,
        pattern_intensity: f64,
        size: Size<Pixels>,
    ) -> Self {
        Self { image, avatar, subject, theme, appearance, pattern_intensity, size }
    }

    pub(crate) fn wallpaper(&self) -> ChatWallpaperRasterFrame {
        ChatWallpaperRasterFrame::new(
            self.theme.preset(self.appearance).resolved(self.appearance),
            ChatWallpaperPreset::resolved_pattern_intensity(self.pattern_intensity, self.appearance),
            self.appearance,
            self.size,
        )
    }

    pub(crate) fn card_width(&self) -> f32 {
        f32::from(self.size.width) * RiseQRCodeCardMetrics::CANVAS_CARD_RATIO
    }

    pub(crate) fn avatar_size(&self) -> f32 {
        self.card_width() * RiseQRCodeCardMetrics::AVATAR_RATIO
    }
}

impl RenderOnce for RiseQRCodeShareCanvas {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let card_width = self.card_width();
        let avatar_size = self.avatar_size();
        let wallpaper = self.wallpaper();
        let size = self.size;

        zstack(Alignment::Center)
            .w(size.width)
            .h(size.height)
            .overflow_hidden()
            .child(wallpaper)
            .child(
                zstack(Alignment::Top)
                    .child(
                        div()
                            .pt(px(RiseQRCodeCardMetrics::avatar_overlap(card_width, avatar_size)))
                            .child(RiseQRCodeCardFace::new(
                                self.image,
                                self.subject.caption(),
                                self.theme,
                                card_width,
                            )),
                    )
                    .child(RiseQRCodeAvatar::new(
                        self.avatar,
                        self.subject.title,
                        self.subject.identity_seed,
                        avatar_size,
                    )),
            )
    }
}
