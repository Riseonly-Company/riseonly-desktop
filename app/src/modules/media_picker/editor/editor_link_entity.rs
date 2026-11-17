use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock};

use gpui::{
    App, AssetSource, Context, DispatchPhase, ElementId, FontWeight, Hsla, ImageSource, InteractiveElement, IntoElement,
    LineFragment,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, ParentElement, PathBuilder, Pixels, Point,
    Render, RenderImage, RenderOnce, Rgba, SMOOTH_SVG_SCALE_FACTOR, SharedString, Size, Styled, StyledImage,
    SvgRenderer, TextSystem, Window, canvas, div, hsla, img, linear_color_stop, linear_gradient, point, px, size, svg,
    white,
};
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder, RgbaImage};
use rise_canon::{RiseLimits, RiseMediaLinkPreviewMode, RiseMediaLinkStyle, RiseValidation};
use rise_core::foundation::foundation_data::base64_encoded_string;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::RiseNotificationName;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle, SelectionFeedbackGenerator};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::context_menu_ui::ContextMenuItem;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::MainText;
use rise_ui::pressable_ui::PressableUi;
use rise_ui::swiftui::context_menu::ContextMenuModifiers;
use rise_ui::swiftui::font::{Font, NATURAL_LINE_HEIGHT};
use rise_ui::swiftui::rotation_effect::RotationEffectModifiers;
use rise_validate::UrlPolicy;
use serde::{Deserialize, Serialize};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_rules::{RiseLimitsCanonRules, RiseValidationCanonRules};
use crate::core::media::remote_media_disk_cache::{RemoteMediaDiskCache, RemoteMediaDiskCacheMaterializeOptions};
use crate::modules::media_picker::editor::bot_mini_app_url_policy_hooks::BotMiniAppURLPolicy;
use crate::modules::media_picker::editor::editor_entity_snapping::{EditorEntitySnap, EditorEntitySnapFeedback};
use crate::modules::media_picker::editor::editor_link_backdrop_hooks::editor_link_backdrop_hooks;
use crate::modules::media_picker::editor::editor_two_finger_transform::EditorTwoFingerTransformable;
use crate::modules::media_picker::editor::story_canvas_model::StoryCanvasGeometry;

/// a link placed on a story or post photo or riff
/// baked into the frame like a sticker so every client and moderation see the same card
/// the published frame carries a tap zone beside it
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorLinkEntity {
    pub id: Uuid,
    pub url: String,
    pub style: String,
    pub preview: String,
    pub title: String,
    pub summary: String,
    pub image_url: String,
    pub is_dark_theme: bool,
    pub base_width: f32,
    pub base_height: f32,
    pub center: Point<f32>,
    pub rotation: f32,
    pub scale: f32,
}

impl EditorLinkEntity {
    pub const CARD_WIDTH: f32 = 300.0;
    pub const DESIGN_SPAN: f32 = 360.0 * Self::CARD_WIDTH / 230.0;

    pub fn max_count() -> i64 {
        RiseLimits::service_value("story_link_areas_max")
    }

    pub fn new(url: impl Into<String>, is_dark_theme: bool, center: Point<f32>) -> Self {
        Self {
            id: Uuid::new_v4(),
            url: url.into(),
            style: RiseMediaLinkStyle::ThemeTranslucent.wire().to_owned(),
            preview: RiseMediaLinkPreviewMode::Full.wire().to_owned(),
            title: String::new(),
            summary: String::new(),
            image_url: String::new(),
            is_dark_theme,
            base_width: Self::CARD_WIDTH,
            base_height: 64.0,
            center,
            rotation: 0.0,
            scale: 1.0,
        }
    }

    pub fn style_kind(&self) -> RiseMediaLinkStyle {
        RiseMediaLinkStyle::from_wire(&self.style).unwrap_or(RiseMediaLinkStyle::ThemeTranslucent)
    }

    pub fn preview_kind(&self) -> RiseMediaLinkPreviewMode {
        RiseMediaLinkPreviewMode::from_wire(&self.preview).unwrap_or(RiseMediaLinkPreviewMode::Full)
    }

    /// next color in the canon order, a tap on the card walks the cycle
    pub fn next_style(&self) -> RiseMediaLinkStyle {
        let all = RiseMediaLinkStyle::ALL;
        let current = self.style_kind();
        let index = all.iter().position(|style| *style == current).unwrap_or(0);
        all[(index + 1) % all.len()]
    }

    /// unscaled card box on a canvas of that size
    pub fn box_size(&self, canvas: Size<f32>) -> Size<f32> {
        let unit = canvas.width.min(canvas.height).max(1.0) / Self::DESIGN_SPAN;
        size(self.base_width.max(1.0) * unit, self.base_height.max(1.0) * unit)
    }

    /// zone a viewer taps in fractions of the frame
    /// unrotated box and its turn as center_x and center_y and width and height and rotation
    pub fn area(&self, canvas: Size<f32>) -> (f64, f64, f64, f64, f64) {
        let box_size = self.box_size(canvas);
        let width = if canvas.width > 0.0 { box_size.width * self.scale / canvas.width } else { 0.0 };
        let height = if canvas.height > 0.0 { box_size.height * self.scale / canvas.height } else { 0.0 };
        (
            f64::from(self.center.x),
            f64::from(self.center.y),
            f64::from(width),
            f64::from(height),
            f64::from(self.rotation),
        )
    }

    pub fn art_key(&self) -> String {
        [
            self.url.as_str(),
            self.style.as_str(),
            self.preview.as_str(),
            if self.is_dark_theme { "d" } else { "l" },
            self.title.as_str(),
            self.summary.as_str(),
            self.image_url.as_str(),
        ]
        .join("|")
    }
}

pub const EDITOR_LINK_CUSTOMIZE: RiseNotificationName = RiseNotificationName::new("editorLinkCustomize");

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaLinkAddressOutcome {
    Valid(String),
    Rejected,
}

pub enum MediaLinkAddress {}

impl MediaLinkAddress {
    pub const ONE_LINE_MAX_CHARACTERS: usize = 34;

    fn policy() -> &'static UrlPolicy {
        static POLICY: LazyLock<UrlPolicy> = LazyLock::new(|| RiseValidation::url_policy("media_link"));
        &POLICY
    }

    /// typed text to the link that goes on the frame
    /// a bare host gets https
    /// anything not https to a public host is refused by the canon url policy media_link
    pub fn normalize(raw: &str) -> MediaLinkAddressOutcome {
        let mut text = trimming_whitespaces_and_newlines(raw).to_owned();
        if text.is_empty() || text.contains(' ') {
            return MediaLinkAddressOutcome::Rejected;
        }
        if !text.to_lowercase().contains("://") {
            text = format!("https://{text}");
        }
        let Some(url) = Url::from_string(&text) else {
            return MediaLinkAddressOutcome::Rejected;
        };
        if url.scheme().map(str::to_lowercase).as_deref() != Some("https") {
            return MediaLinkAddressOutcome::Rejected;
        }
        if !url.host().is_some_and(|host| host.contains('.')) {
            return MediaLinkAddressOutcome::Rejected;
        }
        if !BotMiniAppURLPolicy::allows_policy(&url, Self::policy()) {
            return MediaLinkAddressOutcome::Rejected;
        }
        MediaLinkAddressOutcome::Valid(url.absolute_string().to_owned())
    }

    pub fn host(url: &str) -> String {
        let Some(host) = Url::from_string(url).and_then(|parsed| parsed.host()).map(|host| host.to_lowercase()) else {
            return url.to_owned();
        };
        match host.strip_prefix("www.") {
            Some(rest) => rest.to_owned(),
            None => host,
        }
    }

    /// short form, the host and the first letters of the path cut with an ellipsis
    pub fn short(url: &str) -> String {
        let host = Self::host(url);
        let Some(parsed) = Url::from_string(url) else {
            return host;
        };
        let mut tail = parsed.path();
        if let Some(query) = parsed.query().filter(|query| !query.is_empty()) {
            tail.push('?');
            tail.push_str(query);
        }
        let tail: String = tail.chars().skip_while(|ch| *ch == '/').collect();
        if tail.is_empty() {
            return host;
        }
        let head: String = tail.chars().take(3).collect();
        let ellipsis = if tail.chars().count() > head.chars().count() { "\u{2026}" } else { "" };
        format!("{host}/{head}{ellipsis}")
    }

    /// address on one line with the scheme dropped, cut with an ellipsis
    pub fn one_line(url: &str, max_characters: usize) -> String {
        let mut text = url.to_owned();
        for prefix in ["https://", "http://"] {
            if text.to_lowercase().starts_with(prefix) {
                text = text.chars().skip(prefix.chars().count()).collect();
            }
        }
        if let Some(rest) = text.strip_prefix("www.") {
            text = rest.to_owned();
        }
        if let Some(rest) = text.strip_suffix('/') {
            text = rest.to_owned();
        }
        if text.chars().count() <= max_characters {
            return text;
        }
        let head: String = text.chars().take(max_characters.saturating_sub(1)).collect();
        format!("{head}\u{2026}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediaLinkPalette {
    pub background: Hsla,
    pub primary: Hsla,
    pub secondary: Hsla,
    pub accent: Hsla,
}

fn rgb(red: f32, green: f32, blue: f32) -> Hsla {
    Hsla::from(Rgba { r: red, g: green, b: blue, a: 1.0 })
}

impl MediaLinkPalette {
    pub fn new(style: RiseMediaLinkStyle, is_dark_theme: bool) -> Self {
        let dark_glass = Self::glass(true);
        let light_glass = Self::glass(false);
        match style {
            RiseMediaLinkStyle::ThemeTranslucent => {
                if is_dark_theme { dark_glass } else { light_glass }
            }
            RiseMediaLinkStyle::InverseTranslucent => {
                if is_dark_theme { light_glass } else { dark_glass }
            }
            RiseMediaLinkStyle::SolidLight => Self {
                background: white(),
                primary: hsla(0.0, 0.0, 0.06, 1.0),
                secondary: hsla(0.0, 0.0, 0.06, 0.62),
                accent: rgb(0.0, 0.48, 1.0),
            },
            RiseMediaLinkStyle::SolidDark => Self {
                background: rgb(0.11, 0.11, 0.12),
                primary: white(),
                secondary: white().opacity(0.68),
                accent: rgb(0.39, 0.66, 1.0),
            },
        }
    }

    fn glass(dark: bool) -> Self {
        if dark {
            Self {
                background: hsla(0.0, 0.0, 0.0, 0.46),
                primary: white(),
                secondary: white().opacity(0.74),
                accent: white(),
            }
        } else {
            Self {
                background: white().opacity(0.74),
                primary: hsla(0.0, 0.0, 0.06, 1.0),
                secondary: hsla(0.0, 0.0, 0.06, 0.62),
                accent: rgb(0.0, 0.42, 0.9),
            }
        }
    }
}

/// preview picture decoded for the card, kept as png so every render embeds it as is
#[derive(Clone)]
pub struct EditorLinkPicture {
    png: Arc<[u8]>,
}

impl EditorLinkPicture {
    fn data_uri(&self) -> String {
        format!("data:image/png;base64,{}", base64_encoded_string(&self.png))
    }
}

/// text shaping and assets and the rasterizer the card is drawn with, bound from the app once
#[derive(Clone)]
pub struct EditorLinkCardCanvas {
    text_system: Arc<TextSystem>,
    assets: Arc<dyn AssetSource>,
    svg_renderer: SvgRenderer,
}

impl EditorLinkCardCanvas {
    const APP_LOGO_ASSET: &'static str = "images/AppLogo.png";

    fn new(cx: &App) -> Self {
        Self {
            text_system: Arc::clone(cx.text_system()),
            assets: Arc::clone(cx.asset_source()),
            svg_renderer: SvgRenderer::new(Arc::clone(cx.asset_source())),
        }
    }

    fn font(weight: FontWeight) -> gpui::Font {
        let spec = Font::system_weight(0.0, weight);
        gpui::Font {
            family: spec.family(),
            features: spec.features(),
            fallbacks: None,
            weight,
            style: spec.style(),
        }
    }

    fn text_width(&self, text: &str, font: &gpui::Font, font_size: f32) -> f32 {
        let font_id = self.text_system.resolve_font(font);
        text.chars()
            .map(|ch| f32::from(self.text_system.layout_width(font_id, px(font_size), ch)))
            .sum()
    }

    /// one line with the tail cut by an ellipsis
    fn truncated(&self, text: &str, font: &gpui::Font, font_size: f32, width: f32) -> String {
        if self.text_width(text, font, font_size) <= width {
            return text.to_owned();
        }
        let mut chars: Vec<char> = text.chars().collect();
        while !chars.is_empty() {
            chars.pop();
            let candidate: String = chars.iter().collect::<String>() + "\u{2026}";
            if self.text_width(&candidate, font, font_size) <= width {
                return candidate;
            }
        }
        "\u{2026}".to_owned()
    }

    /// wrapped lines up to limit
    /// the last one cut with an ellipsis when text is left
    fn lines(&self, text: &str, font: &gpui::Font, font_size: f32, width: f32, limit: usize) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        for paragraph in text.split('\n') {
            let mut starts = vec![0_usize];
            {
                let mut wrapper = self.text_system.line_wrapper(font.clone(), px(font_size));
                let fragments = [LineFragment::text(paragraph)];
                starts.extend(wrapper.wrap_line(&fragments, px(width)).map(|boundary| boundary.ix));
            }
            for (index, start) in starts.iter().enumerate() {
                let end = starts.get(index + 1).copied().unwrap_or(paragraph.len());
                let line = paragraph.get(*start..end).unwrap_or_default();
                lines.push(line.trim_end_matches(char::is_whitespace).to_owned());
            }
        }
        if limit == 0 || lines.len() <= limit {
            return lines;
        }
        let rest = lines[limit - 1..].join(" ");
        lines.truncate(limit - 1);
        lines.push(self.truncated(&rest, font, font_size, width));
        lines
    }

    fn baseline_offset(&self, font: &gpui::Font, font_size: f32, line_height: f32) -> f32 {
        let font_id = self.text_system.resolve_font(font);
        let ascent = f32::from(self.text_system.ascent(font_id, px(font_size)));
        let descent = f32::from(self.text_system.descent(font_id, px(font_size))).abs();
        (line_height - (ascent + descent)) / 2.0 + ascent
    }

    fn asset_data_uri(&self, path: &str, mime: &str) -> Option<String> {
        let bytes = self.assets.load(path).ok().flatten()?;
        Some(format!("data:{mime};base64,{}", base64_encoded_string(&bytes)))
    }

    fn symbol_data_uri(&self, sf_symbol: &str, color: Hsla) -> Option<String> {
        let path = IconUi::asset_path(sf_symbol)?;
        let bytes = self.assets.load(&path).ok().flatten()?;
        let (hex, opacity) = svg_color(color);
        let source = String::from_utf8_lossy(&bytes)
            .replace("currentColor", &hex)
            .replacen("<svg ", &format!("<svg opacity=\"{opacity}\" "), 1);
        Some(format!("data:image/svg+xml;base64,{}", base64_encoded_string(source.as_bytes())))
    }
}

fn svg_color(color: Hsla) -> (String, f32) {
    let rgba = Rgba::from(color);
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    (
        format!("#{:02x}{:02x}{:02x}", channel(rgba.r), channel(rgba.g), channel(rgba.b)),
        rgba.a,
    )
}

fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

/// one svg document in design points, built top down
struct CardDocument<'a> {
    canvas: &'a EditorLinkCardCanvas,
    defs: String,
    body: String,
    next_id: usize,
}

impl<'a> CardDocument<'a> {
    fn new(canvas: &'a EditorLinkCardCanvas) -> Self {
        Self { canvas, defs: String::new(), body: String::new(), next_id: 0 }
    }

    fn id(&mut self, stem: &str) -> String {
        self.next_id += 1;
        format!("{stem}{}", self.next_id)
    }

    fn rounded_clip(&mut self, x: f32, y: f32, width: f32, height: f32, radius: f32) -> String {
        let id = self.id("clip");
        let _ = write!(
            self.defs,
            r#"<clipPath id="{id}"><rect x="{x}" y="{y}" width="{width}" height="{height}" rx="{radius}" ry="{radius}"/></clipPath>"#
        );
        id
    }

    fn rect(&mut self, x: f32, y: f32, width: f32, height: f32, radius: f32, color: Hsla) {
        let (hex, opacity) = svg_color(color);
        let _ = write!(
            self.body,
            r#"<rect x="{x}" y="{y}" width="{width}" height="{height}" rx="{radius}" ry="{radius}" fill="{hex}" fill-opacity="{opacity}"/>"#
        );
    }

    /// one line of text whose line box starts at top
    fn text(&mut self, x: f32, top: f32, line_height: f32, text: &str, font: &gpui::Font, font_size: f32, color: Hsla) {
        let baseline = top + self.canvas.baseline_offset(font, font_size, line_height);
        let (hex, opacity) = svg_color(color);
        let family = escaped(&font.family);
        let _ = write!(
            self.body,
            r#"<text x="{x}" y="{baseline}" xml:space="preserve" font-family="{family}" font-weight="{}" font-size="{font_size}" fill="{hex}" fill-opacity="{opacity}">{}</text>"#,
            font.weight.0,
            escaped(text)
        );
    }

    fn image(&mut self, href: &str, x: f32, y: f32, width: f32, height: f32, clip: Option<&str>, extra: &str) {
        let clip = clip.map(|id| format!(r#" clip-path="url(#{id})""#)).unwrap_or_default();
        let _ = write!(
            self.body,
            r#"<image x="{x}" y="{y}" width="{width}" height="{height}" preserveAspectRatio="xMidYMid slice"{clip}{extra} xlink:href="{href}"/>"#
        );
    }

    fn finish(self, card: Size<f32>, corner_radius: f32, background: Hsla) -> String {
        let (hex, opacity) = svg_color(background);
        let (width, height) = (card.width, card.height);
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{width}" height="{height}" viewBox="0 0 {width} {height}"><defs><clipPath id="card"><rect x="0" y="0" width="{width}" height="{height}" rx="{corner_radius}" ry="{corner_radius}"/></clipPath>{}</defs><g clip-path="url(#card)"><rect x="0" y="0" width="{width}" height="{height}" fill="{hex}" fill-opacity="{opacity}"/>{}</g></svg>"#,
            self.defs, self.body
        )
    }
}

#[derive(Clone)]
pub struct MediaLinkCardView {
    pub url: String,
    pub style: RiseMediaLinkStyle,
    pub preview: RiseMediaLinkPreviewMode,
    pub title: String,
    pub summary: String,
    pub is_dark_theme: bool,
    pub image: Option<EditorLinkPicture>,
}

impl MediaLinkCardView {
    fn palette(&self) -> MediaLinkPalette {
        MediaLinkPalette::new(self.style, self.is_dark_theme)
    }

    fn site(&self) -> String {
        MediaLinkAddress::host(&self.url)
    }

    fn shown_title(&self) -> String {
        let clean = trimming_whitespaces_and_newlines(&self.title);
        if clean.is_empty() { self.site() } else { clean.to_owned() }
    }

    /// the card as an svg document and its size in design points
    pub fn document(&self, canvas: &EditorLinkCardCanvas) -> (String, Size<f32>) {
        let mut document = CardDocument::new(canvas);
        let palette = self.palette();
        let (card, radius) = match self.preview {
            RiseMediaLinkPreviewMode::Full => (self.full_card(&mut document, palette), 18.0),
            RiseMediaLinkPreviewMode::Small => (self.small_card(&mut document, palette), 16.0),
            RiseMediaLinkPreviewMode::Short => {
                let card = self.pill(&mut document, palette, &MediaLinkAddress::short(&self.url));
                (card, card.height / 2.0)
            }
            RiseMediaLinkPreviewMode::None => {
                let text = MediaLinkAddress::one_line(&self.url, MediaLinkAddress::ONE_LINE_MAX_CHARACTERS);
                let card = self.pill(&mut document, palette, &text);
                (card, card.height / 2.0)
            }
        };
        (document.finish(card, radius, palette.background), card)
    }

    /// site row, the site name in the accent color then a spacer and the promo badge
    fn site_row(&self, document: &mut CardDocument<'_>, x: f32, top: f32, width: f32, palette: MediaLinkPalette, badge: bool) -> f32 {
        let site_font = EditorLinkCardCanvas::font(FontWeight::SEMIBOLD);
        let site_line = 13.0 * NATURAL_LINE_HEIGHT;
        let badge_size = badge.then(|| MediaLinkPromoBadge::size(document.canvas));
        let site_width = width - badge_size.map_or(0.0, |badge| badge.width + 8.0);
        let site = document.canvas.truncated(&self.site(), &site_font, 13.0, site_width.max(0.0));
        let row_height = site_line.max(badge_size.map_or(0.0, |badge| badge.height));
        document.text(x, top + (row_height - site_line) / 2.0, site_line, &site, &site_font, 13.0, palette.accent);
        if let Some(badge) = badge_size {
            MediaLinkPromoBadge::draw(
                document,
                x + width - badge.width,
                top + (row_height - badge.height) / 2.0,
                palette.secondary,
            );
        }
        row_height
    }

    fn paragraph(
        document: &mut CardDocument<'_>,
        text: &str,
        x: f32,
        top: f32,
        width: f32,
        style: (FontWeight, f32, usize),
        color: Hsla,
    ) -> f32 {
        let (weight, font_size, limit) = style;
        let font = EditorLinkCardCanvas::font(weight);
        let line_height = font_size * NATURAL_LINE_HEIGHT;
        let lines = document.canvas.lines(text, &font, font_size, width, limit);
        for (index, line) in lines.iter().enumerate() {
            document.text(x, top + line_height * index as f32, line_height, line, &font, font_size, color);
        }
        line_height * lines.len() as f32
    }

    fn full_card(&self, document: &mut CardDocument<'_>, palette: MediaLinkPalette) -> Size<f32> {
        let width = EditorLinkEntity::CARD_WIDTH;
        let mut y = 0.0;
        if let Some(image) = self.image.as_ref() {
            let height = width / 1.91;
            Self::banner(document, image, width, height);
            y = height;
        }
        let inner_width = width - 28.0;
        y += 12.0;
        y += self.site_row(document, 14.0, y, inner_width, palette, self.image.is_none());
        y += 4.0;
        y += Self::paragraph(
            document,
            &self.shown_title(),
            14.0,
            y,
            inner_width,
            (FontWeight::SEMIBOLD, 16.0, 2),
            palette.primary,
        );
        if !self.summary.is_empty() {
            y += 4.0;
            y += Self::paragraph(document, &self.summary, 14.0, y, inner_width, (FontWeight::NORMAL, 13.0, 3), palette.secondary);
        }
        y += 12.0;
        size(width, y)
    }

    /// preview picture with the app badge in its top left corner over a soft blur of the picture itself
    /// so the badge reads on any banner
    fn banner(document: &mut CardDocument<'_>, image: &EditorLinkPicture, width: f32, height: f32) {
        let href = image.data_uri();
        let banner_clip = document.rounded_clip(0.0, 0.0, width, height, 0.0);
        document.image(&href, 0.0, 0.0, width, height, Some(&banner_clip), "");

        let label = MediaLinkPromoBadgeBannerLabel::size(document.canvas);
        let inset = MediaLinkPromoBadge::BANNER_INSET;
        let capsule = document.rounded_clip(inset, inset, label.width, label.height, label.height / 2.0);
        let filter = document.id("blur");
        let _ = write!(
            document.defs,
            r#"<filter id="{filter}" filterUnits="userSpaceOnUse" x="0" y="0" width="{width}" height="{height}"><feGaussianBlur stdDeviation="{}"/></filter>"#,
            MediaLinkPromoBadge::BANNER_BLUR / 2.0
        );
        let _ = write!(document.body, r#"<g clip-path="url(#{capsule})"><g clip-path="url(#{banner_clip})">"#);
        document.image(&href, 0.0, 0.0, width, height, None, &format!(r#" filter="url(#{filter})""#));
        document.body.push_str("</g>");
        document.rect(0.0, 0.0, width, height, 0.0, hsla(0.0, 0.0, 0.0, MediaLinkPromoBadge::BANNER_DIM));
        document.body.push_str("</g>");
        MediaLinkPromoBadgeBannerLabel::draw(document, inset, inset);
    }

    fn small_card(&self, document: &mut CardDocument<'_>, palette: MediaLinkPalette) -> Size<f32> {
        let width = EditorLinkEntity::CARD_WIDTH;
        let padding = 10.0;
        let picture_side = 56.0;
        let inner_width = width - padding * 2.0;
        let text_width = inner_width - if self.image.is_some() { picture_side + 10.0 } else { 0.0 };

        let mut probe = CardDocument::new(document.canvas);
        let text_height = self.small_text(&mut probe, 0.0, 0.0, text_width, palette);
        let content_height = text_height.max(if self.image.is_some() { picture_side } else { 0.0 });
        self.small_text(document, padding, padding + (content_height - text_height) / 2.0, text_width, palette);
        if let Some(image) = self.image.as_ref() {
            let x = padding + inner_width - picture_side;
            let y = padding + (content_height - picture_side) / 2.0;
            let clip = document.rounded_clip(x, y, picture_side, picture_side, 10.0);
            document.image(&image.data_uri(), x, y, picture_side, picture_side, Some(&clip), "");
        }
        size(width, content_height + padding * 2.0)
    }

    fn small_text(&self, document: &mut CardDocument<'_>, x: f32, top: f32, width: f32, palette: MediaLinkPalette) -> f32 {
        let row = self.site_row(document, x, top, width, palette, true);
        let title = Self::paragraph(
            document,
            &self.shown_title(),
            x,
            top + row + 3.0,
            width,
            (FontWeight::SEMIBOLD, 15.0, 2),
            palette.primary,
        );
        row + 3.0 + title
    }

    fn pill(&self, document: &mut CardDocument<'_>, palette: MediaLinkPalette, text: &str) -> Size<f32> {
        let canvas = document.canvas;
        let font = EditorLinkCardCanvas::font(FontWeight::SEMIBOLD);
        let line_height = 17.0 * NATURAL_LINE_HEIGHT;
        let text_width = canvas.text_width(text, &font, 17.0);
        let icon_side = 16.0;
        let badge = MediaLinkPromoBadge::size(canvas);
        let row_height = line_height.max(icon_side).max(badge.height);
        let width = 16.0 + icon_side + 7.0 + text_width + 7.0 + 1.0 + badge.width + 16.0;
        let height = 11.0 + row_height + 11.0;

        let mut x = 16.0;
        if let Some(icon) = canvas.symbol_data_uri("link", palette.primary) {
            let _ = write!(
                document.body,
                r#"<image x="{x}" y="{}" width="{icon_side}" height="{icon_side}" xlink:href="{icon}"/>"#,
                11.0 + (row_height - icon_side) / 2.0
            );
        }
        x += icon_side + 7.0;
        document.text(x, 11.0 + (row_height - line_height) / 2.0, line_height, text, &font, 17.0, palette.primary);
        x += text_width + 7.0 + 1.0;
        MediaLinkPromoBadge::draw(document, x, 11.0 + (row_height - badge.height) / 2.0, palette.secondary);
        size(width, height)
    }
}

/// the app's mark on every link card
/// the app logo and site name, an advert that the link came from the app
/// part of the card's pixels so it never takes a tap of its own
pub enum MediaLinkPromoBadge {}

impl MediaLinkPromoBadge {
    pub const BANNER_INSET: f32 = 8.0;
    pub const BANNER_BLUR: f32 = 6.0;
    pub const BANNER_DIM: f32 = 0.28;

    /// public site host from canon for production deep links, not a literal
    pub fn host() -> String {
        AppEnvironment::Prod.public_host()
    }

    pub fn banner_title() -> String {
        let host = Self::host();
        let mut chars = host.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => host,
        }
    }

    fn size(canvas: &EditorLinkCardCanvas) -> Size<f32> {
        let font = EditorLinkCardCanvas::font(FontWeight::MEDIUM);
        let text_width = canvas.text_width(&Self::host(), &font, 11.0);
        size(16.0 + 4.0 + text_width, (11.0 * NATURAL_LINE_HEIGHT).max(16.0))
    }

    /// draws the avatar and the host in color with left top at x and y
    fn draw(document: &mut CardDocument<'_>, x: f32, y: f32, color: Hsla) {
        let badge = Self::size(document.canvas);
        let font = EditorLinkCardCanvas::font(FontWeight::MEDIUM);
        let line_height = 11.0 * NATURAL_LINE_HEIGHT;
        MediaLinkPromoBadgeAvatar { size: 16.0 }.draw(document, x, y + (badge.height - 16.0) / 2.0);
        document.text(x + 20.0, y + (badge.height - line_height) / 2.0, line_height, &Self::host(), &font, 11.0, color);
    }

}

/// the app logo in a circle
pub struct MediaLinkPromoBadgeAvatar {
    pub size: f32,
}

impl MediaLinkPromoBadgeAvatar {
    fn draw(&self, document: &mut CardDocument<'_>, x: f32, y: f32) {
        let Some(logo) = document.canvas.asset_data_uri(EditorLinkCardCanvas::APP_LOGO_ASSET, "image/png") else {
            return;
        };
        let side = self.size;
        let clip = document.rounded_clip(x, y, side, side, side / 2.0);
        document.image(&logo, x, y, side, side, Some(&clip), "");
    }
}

/// banner variant of the label without its backdrop
/// the card puts the blur under it
pub struct MediaLinkPromoBadgeBannerLabel;

impl MediaLinkPromoBadgeBannerLabel {
    fn size(canvas: &EditorLinkCardCanvas) -> Size<f32> {
        let font = EditorLinkCardCanvas::font(FontWeight::SEMIBOLD);
        let text_width = canvas.text_width(&MediaLinkPromoBadge::banner_title(), &font, 11.0);
        size(3.0 + 18.0 + 5.0 + text_width + 8.0, 24.0)
    }

    fn draw(document: &mut CardDocument<'_>, x: f32, y: f32) {
        let font = EditorLinkCardCanvas::font(FontWeight::SEMIBOLD);
        let line_height = 11.0 * NATURAL_LINE_HEIGHT;
        MediaLinkPromoBadgeAvatar { size: 18.0 }.draw(document, x + 3.0, y + (24.0 - 18.0) / 2.0);
        let title = MediaLinkPromoBadge::banner_title();
        document.text(x + 3.0 + 18.0 + 5.0, y + (24.0 - line_height) / 2.0, line_height, &title, &font, 11.0, white());
    }
}

/// cached art for a link card
#[derive(Clone)]
pub struct EditorLinkArtStoreArt {
    pub image: Option<Arc<RenderImage>>,
    pub size: Size<f32>,
    pub document: Arc<str>,
}

#[derive(Default)]
struct EditorLinkArtStoreState {
    arts: HashMap<String, EditorLinkArtStoreArt>,
    order: Vec<String>,
    pictures: HashMap<String, EditorLinkPicture>,
}

/// rendered cards keyed by everything that changes their pixels
/// canvas and bake draw the same thing, thread safe since bake reads off main
pub struct EditorLinkArtStore {
    state: Mutex<EditorLinkArtStoreState>,
    canvas: OnceLock<EditorLinkCardCanvas>,
}

static SHARED_ART_STORE: LazyLock<EditorLinkArtStore> = LazyLock::new(|| EditorLinkArtStore {
    state: Mutex::new(EditorLinkArtStoreState::default()),
    canvas: OnceLock::new(),
});

impl EditorLinkArtStore {
    const RENDER_SCALE: f32 = 3.0;
    const MAX_ENTRIES: usize = 24;
    const PICTURE_MAX_SIDE: u32 = 900;

    pub fn shared() -> &'static EditorLinkArtStore {
        &SHARED_ART_STORE
    }

    fn locked(&self) -> MutexGuard<'_, EditorLinkArtStoreState> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn art(&self, entity: &EditorLinkEntity) -> Option<EditorLinkArtStoreArt> {
        self.locked().arts.get(&entity.art_key()).cloned()
    }

    /// draws the card once its picture is on disk and reports back on the main thread
    pub fn prepare(&'static self, entity: &EditorLinkEntity, cx: &mut App, completion: Rc<dyn Fn(EditorLinkArtStoreArt, &mut App)>) {
        if let Some(hit) = self.art(entity) {
            completion(hit, cx);
            return;
        }
        let canvas = self.canvas.get_or_init(|| EditorLinkCardCanvas::new(cx)).clone();
        let mode = entity.preview_kind();
        let needs_picture = matches!(mode, RiseMediaLinkPreviewMode::Full | RiseMediaLinkPreviewMode::Small)
            && !entity.image_url.is_empty();
        if needs_picture && self.picture(&entity.image_url).is_none() {
            let interim = self.render(entity, None, &canvas);
            completion(interim, cx);
            let entity = entity.clone();
            cx.spawn(async move |cx| {
                let loaded = Self::load_picture(&entity.image_url, cx).await;
                if let Some(loaded) = loaded.as_ref() {
                    self.store_picture(loaded.clone(), &entity.image_url);
                }
                let art = self.render(&entity, loaded, &canvas);
                cx.update(|cx| completion(art, cx));
            })
            .detach();
            return;
        }
        let picture = if needs_picture { self.picture(&entity.image_url) } else { None };
        completion(self.render(entity, picture, &canvas), cx);
    }

    fn render(&self, entity: &EditorLinkEntity, image: Option<EditorLinkPicture>, canvas: &EditorLinkCardCanvas) -> EditorLinkArtStoreArt {
        let has_image = image.is_some();
        let card = MediaLinkCardView {
            url: entity.url.clone(),
            style: entity.style_kind(),
            preview: entity.preview_kind(),
            title: entity.title.clone(),
            summary: entity.summary.clone(),
            is_dark_theme: entity.is_dark_theme,
            image,
        };
        let (document, card_size) = card.document(canvas);
        let rendered = canvas
            .svg_renderer
            .render_single_frame(document.as_bytes(), Self::RENDER_SCALE / SMOOTH_SVG_SCALE_FACTOR)
            .ok();
        let Some(rendered) = rendered else {
            return EditorLinkArtStoreArt {
                image: None,
                size: size(entity.base_width, entity.base_height),
                document: Arc::from(document),
            };
        };
        let art = EditorLinkArtStoreArt { image: Some(rendered), size: card_size, document: Arc::from(document) };
        let mode = entity.preview_kind();
        let is_final = has_image
            || entity.image_url.is_empty()
            || !matches!(mode, RiseMediaLinkPreviewMode::Full | RiseMediaLinkPreviewMode::Small);
        if is_final {
            self.remember(entity.art_key(), art.clone());
        }
        art
    }

    fn remember(&self, key: String, art: EditorLinkArtStoreArt) {
        let mut state = self.locked();
        if !state.arts.contains_key(&key) {
            state.order.push(key.clone());
        }
        state.arts.insert(key, art);
        while state.order.len() > Self::MAX_ENTRIES {
            let dropped = state.order.remove(0);
            state.arts.remove(&dropped);
        }
    }

    fn picture(&self, url: &str) -> Option<EditorLinkPicture> {
        self.locked().pictures.get(url).cloned()
    }

    fn store_picture(&self, image: EditorLinkPicture, url: &str) {
        let mut state = self.locked();
        if state.pictures.len() > Self::MAX_ENTRIES {
            state.pictures.clear();
        }
        state.pictures.insert(url.to_owned(), image);
    }

    async fn load_picture(source: &str, cx: &mut gpui::AsyncApp) -> Option<EditorLinkPicture> {
        let remote = Url::from_string(source)?;
        if remote.scheme().map(str::to_lowercase).as_deref() != Some("https") {
            return None;
        }
        let options = RemoteMediaDiskCacheMaterializeOptions {
            cache_key: Some(source.to_owned()),
            suggested_filename: Some(remote.last_path_component()),
            ..Default::default()
        };
        let local = RemoteMediaDiskCache::shared().ensure_cached(&remote, options, cx).await?;
        cx.background_executor()
            .spawn(async move {
                let decoded = image::open(&local).ok()?;
                let thumbnail = decoded.thumbnail(Self::PICTURE_MAX_SIDE, Self::PICTURE_MAX_SIDE).to_rgba8();
                let (width, height) = thumbnail.dimensions();
                let mut png = Vec::new();
                PngEncoder::new(&mut png)
                    .write_image(thumbnail.as_raw(), width, height, ExtendedColorType::Rgba8)
                    .ok()?;
                Some(EditorLinkPicture { png: Arc::from(png) })
            })
            .await
    }
}

pub enum EditorLinkRenderer {}

impl EditorLinkRenderer {
    /// draws every ready link card into the bake bitmap of this size
    /// one pixel per point and straight alpha, origin top left
    pub fn draw(links: &[EditorLinkEntity], size: Size<f32>, ctx: &mut RgbaImage) {
        let store = EditorLinkArtStore::shared();
        let Some(canvas) = store.canvas.get() else {
            return;
        };
        let mut groups = String::new();
        for link in links {
            let Some(art) = store.art(link) else {
                continue;
            };
            let box_size = link.box_size(size);
            let href = format!("data:image/svg+xml;base64,{}", base64_encoded_string(art.document.as_bytes()));
            let _ = write!(
                groups,
                r#"<g transform="translate({} {}) rotate({}) scale({})"><image x="{}" y="{}" width="{}" height="{}" preserveAspectRatio="none" xlink:href="{href}"/></g>"#,
                link.center.x * size.width,
                link.center.y * size.height,
                link.rotation.to_degrees(),
                link.scale,
                -box_size.width / 2.0,
                -box_size.height / 2.0,
                box_size.width,
                box_size.height,
            );
        }
        if groups.is_empty() {
            return;
        }
        let (width, height) = (ctx.width(), ctx.height());
        let document = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{width}" height="{height}" viewBox="0 0 {} {}">{groups}</svg>"#,
            size.width, size.height
        );
        let Ok(layer) = canvas.svg_renderer.render_single_frame(document.as_bytes(), 1.0 / SMOOTH_SVG_SCALE_FACTOR) else {
            return;
        };
        let Some(pixels) = layer.as_bytes(0) else {
            return;
        };
        let layer_size = layer.size(0);
        let layer_width = u32::try_from(layer_size.width.0).unwrap_or(0).min(width);
        let layer_height = u32::try_from(layer_size.height.0).unwrap_or(0).min(height);
        let stride = usize::try_from(layer_size.width.0).unwrap_or(0) * 4;
        for y in 0..layer_height {
            for x in 0..layer_width {
                let offset = y as usize * stride + x as usize * 4;
                let Some(source) = pixels.get(offset..offset + 4) else {
                    continue;
                };
                let alpha = f32::from(source[3]) / 255.0;
                if alpha <= 0.0 {
                    continue;
                }
                let target = ctx.get_pixel_mut(x, y);
                let below = f32::from(target[3]) / 255.0;
                let out = alpha + below * (1.0 - alpha);
                for (channel, source_channel) in [(0, source[2]), (1, source[1]), (2, source[0])] {
                    let over = f32::from(source_channel) * alpha + f32::from(target[channel]) * below * (1.0 - alpha);
                    target[channel] = (over / out).round().clamp(0.0, 255.0) as u8;
                }
                target[3] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

struct EditorLinkEntityViewMetrics;

impl EditorLinkEntityViewMetrics {
    const SELECTION_PADDING: f32 = 6.0;
    const PAN_SLOP: f32 = 4.0;
}

#[derive(Clone, Copy)]
struct EditorEntityPress {
    start: Point<Pixels>,
    is_panning: bool,
}

/// dashed selection outline, rect is in window pixels
pub fn paint_dashed_selection(origin: Point<Pixels>, frame: Size<f32>, radius: f32, line: f32, window: &mut Window) {
    let radius = px(radius.max(0.0).min(frame.width / 2.0).min(frame.height / 2.0));
    let left = origin.x;
    let top = origin.y;
    let right = origin.x + px(frame.width);
    let bottom = origin.y + px(frame.height);
    let radii = point(radius, radius);
    let mut path = PathBuilder::stroke(px(line)).dash_array(&[px(5.0 * line), px(4.0 * line)]);
    path.move_to(point(left + radius, top));
    path.line_to(point(right - radius, top));
    path.arc_to(radii, px(0.0), false, true, point(right, top + radius));
    path.line_to(point(right, bottom - radius));
    path.arc_to(radii, px(0.0), false, true, point(right - radius, bottom));
    path.line_to(point(left + radius, bottom));
    path.arc_to(radii, px(0.0), false, true, point(left, bottom - radius));
    path.line_to(point(left, top + radius));
    path.arc_to(radii, px(0.0), false, true, point(left + radius, top));
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, white().opacity(0.95));
    }
}

/// link card on the editor canvas
/// click cycles colors, drag moves it
/// right click opens customize and delete
pub struct EditorLinkEntityView {
    pub entity_id: Uuid,
    art: Option<Arc<RenderImage>>,
    current: Option<EditorLinkEntity>,
    shown_key: Option<String>,
    canvas_size: Size<f32>,
    is_selected: bool,
    allows_body_drag: bool,
    move_start_center: Point<f32>,
    press: Option<EditorEntityPress>,
    two_finger_start_scale: f32,
    two_finger_start_rotation: f32,

    pub on_update: Option<Rc<dyn Fn(EditorLinkEntity, &mut App)>>,
    pub on_select: Option<Rc<dyn Fn(Uuid, &mut App)>>,
    pub on_request_delete: Option<Rc<dyn Fn(Uuid, &mut App)>>,
    pub on_request_customize: Option<Rc<dyn Fn(Uuid, &mut App)>>,
    pub on_snap_guides_changed: Option<Rc<dyn Fn(bool, bool, &mut App)>>,

    snap_feedback: EditorEntitySnapFeedback,
}

impl EditorLinkEntityView {
    pub fn new(entity_id: Uuid, _cx: &mut Context<Self>) -> Self {
        Self {
            entity_id,
            art: None,
            current: None,
            shown_key: None,
            canvas_size: size(0.0, 0.0),
            is_selected: false,
            allows_body_drag: true,
            move_start_center: point(0.0, 0.0),
            press: None,
            two_finger_start_scale: 1.0,
            two_finger_start_rotation: 0.0,
            on_update: None,
            on_select: None,
            on_request_delete: None,
            on_request_customize: None,
            on_snap_guides_changed: None,
            snap_feedback: EditorEntitySnapFeedback::new(),
        }
    }

    pub fn set_interaction_state(&mut self, _handles_visible: bool, allows_body_drag: bool, cx: &mut Context<Self>) {
        self.allows_body_drag = allows_body_drag;
        if !allows_body_drag {
            self.press = None;
        }
        cx.notify();
    }

    pub fn apply(&mut self, entity: &EditorLinkEntity, canvas_size: Size<f32>, is_selected: bool, cx: &mut Context<Self>) {
        self.canvas_size = canvas_size;
        self.current = Some(entity.clone());
        self.is_selected = is_selected;
        self.show_art_if_needed(entity, cx);
        cx.notify();
    }

    fn show_art_if_needed(&mut self, entity: &EditorLinkEntity, cx: &mut Context<Self>) {
        let key = entity.art_key();
        if self.shown_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.shown_key = Some(key.clone());
        let view = cx.weak_entity();
        EditorLinkArtStore::shared().prepare(
            entity,
            cx,
            Rc::new(move |art: EditorLinkArtStoreArt, cx: &mut App| {
                let view = view.clone();
                let key = key.clone();
                cx.defer(move |cx| {
                    let _ = view.update(cx, |this, cx| this.art_ready(&key, art, cx));
                });
            }),
        );
    }

    fn art_ready(&mut self, key: &str, art: EditorLinkArtStoreArt, cx: &mut Context<Self>) {
        if self.shown_key.as_deref() != Some(key) {
            return;
        }
        self.art = art.image.clone();
        cx.notify();
        let Some(mut entity) = self.current.clone() else {
            return;
        };
        if (entity.base_width - art.size.width).abs() <= 0.5 && (entity.base_height - art.size.height).abs() <= 0.5 {
            return;
        }
        entity.base_width = art.size.width;
        entity.base_height = art.size.height;
        let (canvas_size, is_selected) = (self.canvas_size, self.is_selected);
        self.apply(&entity, canvas_size, is_selected, cx);
        self.fire_update(entity, cx);
    }

    fn fire_select(&self, cx: &mut Context<Self>) {
        if let Some(on_select) = self.on_select.clone() {
            let id = self.entity_id;
            App::defer(cx, move |cx| on_select(id, cx));
        }
    }

    fn fire_update(&self, entity: EditorLinkEntity, cx: &mut Context<Self>) {
        if let Some(on_update) = self.on_update.clone() {
            App::defer(cx, move |cx| on_update(entity, cx));
        }
    }

    fn fire_snap_guides(&self, vertical: bool, horizontal: bool, cx: &mut Context<Self>) {
        if let Some(on_snap_guides_changed) = self.on_snap_guides_changed.clone() {
            App::defer(cx, move |cx| on_snap_guides_changed(vertical, horizontal, cx));
        }
    }

    fn center(&self) -> Point<f32> {
        self.current.as_ref().map_or(point(0.0, 0.0), |entity| {
            point(entity.center.x * self.canvas_size.width, entity.center.y * self.canvas_size.height)
        })
    }

    /// hit test, the box plus selection padding with the view scale and turn applied
    /// location is relative to the canvas
    pub fn point(&self, location: Point<f32>) -> bool {
        let Some(entity) = self.current.as_ref() else {
            return false;
        };
        let box_size = entity.box_size(self.canvas_size);
        let center = self.center();
        let (sin, cos) = entity.rotation.sin_cos();
        let scale = entity.scale.max(f32::EPSILON);
        let dx = location.x - center.x;
        let dy = location.y - center.y;
        let local_x = (dx * cos + dy * sin) / scale;
        let local_y = (-dx * sin + dy * cos) / scale;
        let pad = EditorLinkEntityViewMetrics::SELECTION_PADDING;
        local_x.abs() <= box_size.width / 2.0 + pad && local_y.abs() <= box_size.height / 2.0 + pad
    }

    fn handle_tap(&mut self, cx: &mut Context<Self>) {
        self.fire_select(cx);
        let Some(mut entity) = self.current.clone() else {
            return;
        };
        entity.style = entity.next_style().wire().to_owned();
        SelectionFeedbackGenerator::new().selection_changed();
        let (canvas_size, is_selected) = (self.canvas_size, self.is_selected);
        self.apply(&entity, canvas_size, is_selected, cx);
        self.fire_update(entity, cx);
    }

    fn handle_hold(&mut self, cx: &mut Context<Self>) {
        self.fire_select(cx);
        ImpactFeedbackGenerator::new(ImpactStyle::Medium).impact_occurred();
    }

    fn edit_menu_interaction(&self, cx: &mut Context<Self>) -> Vec<ContextMenuItem> {
        let customize_view = cx.weak_entity();
        let delete_view = cx.weak_entity();
        vec![
            ContextMenuItem::new(1, tr("media_link_customize"), None, "media_link_customize", move |_, cx| {
                let Ok((id, callback)) =
                    customize_view.read_with(cx, |view, _| (view.entity_id, view.on_request_customize.clone()))
                else {
                    return;
                };
                if let Some(callback) = callback {
                    callback(id, cx);
                }
            }),
            ContextMenuItem::new(2, tr("ctx_delete"), None, "ctx_delete", move |_, cx| {
                let Ok((id, callback)) =
                    delete_view.read_with(cx, |view, _| (view.entity_id, view.on_request_delete.clone()))
                else {
                    return;
                };
                if let Some(callback) = callback {
                    callback(id, cx);
                }
            })
            .destructive(true),
        ]
    }

    fn pan_began(&mut self, cx: &mut Context<Self>) {
        if !self.allows_body_drag || self.current.is_none() {
            return;
        }
        self.move_start_center = self.center();
        self.snap_feedback.begin();
        self.fire_select(cx);
    }

    fn pan_changed(&mut self, translation: Point<f32>, cx: &mut Context<Self>) {
        if !self.allows_body_drag || self.current.is_none() {
            return;
        }
        let proposed = point(self.move_start_center.x + translation.x, self.move_start_center.y + translation.y);
        let (snapped, snapped_x, snapped_y) = EditorEntitySnap::snap_center(proposed, self.canvas_size);
        self.snap_feedback.update_center(snapped_x, snapped_y);
        self.fire_snap_guides(snapped_x, snapped_y, cx);
        if self.canvas_size.width <= 0.0 || self.canvas_size.height <= 0.0 {
            return;
        }
        let Some(entity) = self.current.as_mut() else {
            return;
        };
        entity.center = point(snapped.x / self.canvas_size.width, snapped.y / self.canvas_size.height);
        let entity = entity.clone();
        cx.notify();
        self.fire_update(entity, cx);
    }

    fn pointer_moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(mut press) = self.press else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.pointer_released(cx);
            return;
        }
        let translation = point(
            f32::from(event.position.x - press.start.x),
            f32::from(event.position.y - press.start.y),
        );
        if !press.is_panning {
            if translation.x.hypot(translation.y) < EditorLinkEntityViewMetrics::PAN_SLOP {
                return;
            }
            press.is_panning = true;
            self.press = Some(press);
            self.pan_began(cx);
        }
        self.pan_changed(translation, cx);
    }

    fn pointer_released(&mut self, cx: &mut Context<Self>) {
        let Some(press) = self.press.take() else {
            return;
        };
        if press.is_panning {
            if self.allows_body_drag && self.current.is_some() {
                self.fire_snap_guides(false, false, cx);
            }
        } else {
            self.handle_tap(cx);
        }
    }
}

impl EditorTwoFingerTransformable for EditorLinkEntityView {
    fn entity_id(&self) -> Uuid {
        self.entity_id
    }

    fn begin_two_finger_transform(&mut self, cx: &mut Context<Self>) {
        let Some(entity) = self.current.as_ref() else {
            return;
        };
        self.two_finger_start_scale = entity.scale;
        self.two_finger_start_rotation = entity.rotation;
        self.snap_feedback.begin();
        self.fire_select(cx);
    }

    fn apply_two_finger_transform(&mut self, scale: f32, rotation: f32, cx: &mut Context<Self>) {
        let range = StoryCanvasGeometry::ENTITY_SCALE_RANGE;
        let (angle, snapped) = EditorEntitySnap::snap_rotation(self.two_finger_start_rotation + rotation);
        self.snap_feedback.update_rotation(snapped);
        let start_scale = self.two_finger_start_scale;
        let Some(entity) = self.current.as_mut() else {
            return;
        };
        entity.scale = (start_scale * scale).max(*range.start()).min(*range.end());
        entity.rotation = angle;
        let entity = entity.clone();
        cx.notify();
        self.fire_update(entity, cx);
    }

    fn end_two_finger_transform(&mut self, _cx: &mut Context<Self>) {}
}

impl Render for EditorLinkEntityView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(entity) = self.current.clone() else {
            return div().into_any_element();
        };
        let box_size = entity.box_size(self.canvas_size);
        let scale = entity.scale;
        let center = self.center();
        let frame = size(box_size.width * scale, box_size.height * scale);
        let pad = EditorLinkEntityViewMetrics::SELECTION_PADDING * scale;
        let (sin, cos) = entity.rotation.sin_cos();
        let half_w = frame.width / 2.0 + pad;
        let half_h = frame.height / 2.0 + pad;
        let reach = size(half_w * cos.abs() + half_h * sin.abs(), half_w * sin.abs() + half_h * cos.abs());
        let hit_origin = point(center.x - reach.width, center.y - reach.height);

        let is_selected = self.is_selected;
        let mut turned = div().relative().w(px(frame.width)).h(px(frame.height));
        if let Some(art) = self.art.clone() {
            turned = turned.child(img(ImageSource::Render(art)).size_full().object_fit(ObjectFit::Fill));
        }
        if is_selected {
            let selection = size(frame.width + pad * 2.0, frame.height + pad * 2.0);
            turned = turned.child(
                canvas(
                    |_, _, _| {},
                    move |bounds, (), window, _| {
                        paint_dashed_selection(bounds.origin, selection, 14.0 * scale, scale, window);
                    },
                )
                .absolute()
                .left(px(-pad))
                .top(px(-pad))
                .w(px(selection.width))
                .h(px(selection.height)),
            );
        }

        let positioned = div()
            .absolute()
            .left(px(center.x - frame.width / 2.0 - hit_origin.x))
            .top(px(center.y - frame.height / 2.0 - hit_origin.y))
            .child(turned.rotation_effect(entity.rotation.to_degrees()));

        let body = div()
            .absolute()
            .left(px(hit_origin.x))
            .top(px(hit_origin.y))
            .w(px(reach.width * 2.0))
            .h(px(reach.height * 2.0))
            .child(positioned);

        if self.allows_body_drag {
            let view = cx.weak_entity();
            let gestures = canvas(
                |_, _, _| {},
                move |bounds, (), window, _| {
                    let down_view = view.clone();
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, _, cx| {
                        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                            return;
                        }
                        if !bounds.contains(&event.position) {
                            return;
                        }
                        let local = point(
                            f32::from(event.position.x - bounds.origin.x) + hit_origin.x,
                            f32::from(event.position.y - bounds.origin.y) + hit_origin.y,
                        );
                        let inside = down_view.read_with(cx, |view, _| view.point(local)).unwrap_or(false);
                        if !inside {
                            return;
                        }
                        cx.stop_propagation();
                        let _ = down_view.update(cx, |view, cx| {
                            view.press = Some(EditorEntityPress { start: event.position, is_panning: false });
                            cx.notify();
                        });
                    });
                    let move_view = view.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        let is_pressed = move_view.read_with(cx, |view, _| view.press.is_some()).unwrap_or(false);
                        if is_pressed {
                            let _ = move_view.update(cx, |view, cx| view.pointer_moved(event, cx));
                        }
                    });
                    let release_view = view.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                            return;
                        }
                        let is_pressed = release_view.read_with(cx, |view, _| view.press.is_some()).unwrap_or(false);
                        if is_pressed {
                            let _ = release_view.update(cx, |view, cx| view.pointer_released(cx));
                        }
                    });
                },
            )
            .absolute()
            .inset_0();
            let menu_id = SharedString::from(format!("editor-link-entity-menu-{}", self.entity_id));
            let items = self.edit_menu_interaction(cx);
            return body
                .child(gestures)
                .on_mouse_down(MouseButton::Right, cx.listener(|this, _: &MouseDownEvent, _, cx| this.handle_hold(cx)))
                .context_menu(menu_id, items)
                .into_any_element();
        }
        body.into_any_element()
    }
}

/// add sheet card before it is placed
/// keeps picked color, preview kind and the snapshot fetched while typing
#[derive(Clone, Debug, PartialEq)]
pub struct EditorLinkDraft {
    pub style: RiseMediaLinkStyle,
    pub preview: RiseMediaLinkPreviewMode,
    pub url: String,
    pub fetched_url: String,
    pub title: String,
    pub summary: String,
    pub image_url: String,
}

impl Default for EditorLinkDraft {
    fn default() -> Self {
        Self {
            style: RiseMediaLinkStyle::ThemeTranslucent,
            preview: RiseMediaLinkPreviewMode::Full,
            url: String::new(),
            fetched_url: String::new(),
            title: String::new(),
            summary: String::new(),
            image_url: String::new(),
        }
    }
}

impl EditorLinkDraft {
    pub fn has_snapshot(&self) -> bool {
        !self.url.is_empty() && self.fetched_url == self.url
    }

    pub fn entity(&self, is_dark_theme: bool) -> Option<EditorLinkEntity> {
        if self.url.is_empty() {
            return None;
        }
        let mut entity = EditorLinkEntity::new(self.url.clone(), is_dark_theme, point(0.5, 0.5));
        entity.style = self.style.wire().to_owned();
        entity.preview = self.preview.wire().to_owned();
        if self.has_snapshot() {
            entity.title = self.title.clone();
            entity.summary = self.summary.clone();
            entity.image_url = self.image_url.clone();
        }
        Some(entity)
    }
}

struct EditorLinkLiveSampleState {
    sample: Option<EditorLinkArtStoreArt>,
    sample_key: String,
}

/// the card as it will look on the frame, over the default chat wallpaper of the theme
#[derive(IntoElement)]
pub struct EditorLinkLiveSample {
    pub link: EditorLinkEntity,
}

impl EditorLinkLiveSample {
    pub const HEIGHT: f32 = 150.0;
    pub const CARD_BOX: Size<f32> = Size { width: 270.0, height: 130.0 };

    pub fn new(link: EditorLinkEntity) -> Self {
        Self { link }
    }
}

impl RenderOnce for EditorLinkLiveSample {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("editor-link-live-sample", cx, |_, _| EditorLinkLiveSampleState {
            sample: None,
            sample_key: String::new(),
        });
        let key = self.link.art_key();
        if state.read(cx).sample_key != key {
            state.update(cx, |state, _| state.sample_key = key.clone());
            let weak = state.downgrade();
            EditorLinkArtStore::shared().prepare(
                &self.link,
                cx,
                Rc::new(move |art: EditorLinkArtStoreArt, cx: &mut App| {
                    let weak = weak.clone();
                    let key = key.clone();
                    cx.defer(move |cx| {
                        let _ = weak.update(cx, |state, cx| {
                            if state.sample_key == key {
                                state.sample = Some(art);
                                cx.notify();
                            }
                        });
                    });
                }),
            );
        }

        let mut sample_layer = div().absolute().inset_0().flex().items_center().justify_center();
        if let Some(sample) = state.read(cx).sample.clone()
            && let Some(image) = sample.image
        {
            let fit = (Self::CARD_BOX.width / sample.size.width.max(1.0))
                .min(Self::CARD_BOX.height / sample.size.height.max(1.0));
            sample_layer = sample_layer.child(
                img(ImageSource::Render(image))
                    .w(px(sample.size.width * fit))
                    .h(px(sample.size.height * fit))
                    .object_fit(ObjectFit::Contain),
            );
        }

        div()
            .relative()
            .w_full()
            .h(px(Self::HEIGHT))
            .rounded(px(16.0))
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .children(editor_link_backdrop_hooks().map(|hooks| (hooks.default_wallpaper_preview)())),
            )
            .child(sample_layer)
    }
}

#[derive(IntoElement)]
struct EditorLinkBackdrop;

impl RenderOnce for EditorLinkBackdrop {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div().absolute().inset_0().bg(linear_gradient(
            180.0,
            linear_color_stop(rgb(0.24, 0.26, 0.36), 0.0),
            linear_color_stop(rgb(0.10, 0.10, 0.14), 1.0),
        ))
    }
}

/// the four card colors in canon order, current one is ringed
/// no captions because canon has no names for them
#[derive(IntoElement)]
pub struct EditorLinkStyleSwatches {
    pub selected: RiseMediaLinkStyle,
    pub is_dark_theme: bool,
    pub ring_color: Hsla,
    pub on_select: Rc<dyn Fn(RiseMediaLinkStyle, &mut Window, &mut App)>,
}

impl EditorLinkStyleSwatches {
    const DIAMETER: f32 = 30.0;
    const SPACING: f32 = 16.0;
    const RING_GAP: f32 = 3.0;
    const RING_WIDTH: f32 = 2.0;

    pub fn new(
        selected: RiseMediaLinkStyle,
        is_dark_theme: bool,
        ring_color: Hsla,
        on_select: impl Fn(RiseMediaLinkStyle, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { selected, is_dark_theme, ring_color, on_select: Rc::new(on_select) }
    }

    fn swatch(&self, style: RiseMediaLinkStyle) -> gpui::Div {
        let palette = MediaLinkPalette::new(style, self.is_dark_theme);
        let mut glyph = div().absolute().inset_0().flex().items_center().justify_center();
        if let Some(path) = IconUi::asset_path("link") {
            glyph = glyph.child(svg().path(path).size(px(12.0)).text_color(palette.primary));
        }
        let disc = div()
            .absolute()
            .inset_0()
            .rounded_full()
            .overflow_hidden()
            .child(EditorLinkBackdrop)
            .child(div().absolute().inset_0().bg(palette.background))
            .child(glyph);
        let ring_outset = Self::RING_GAP + Self::RING_WIDTH;
        let ring = div()
            .absolute()
            .left(px(-ring_outset))
            .top(px(-ring_outset))
            .w(px(Self::DIAMETER + ring_outset * 2.0))
            .h(px(Self::DIAMETER + ring_outset * 2.0))
            .rounded_full()
            .border(px(Self::RING_WIDTH))
            .border_color(self.ring_color)
            .opacity(if style == self.selected { 1.0 } else { 0.0 });
        div()
            .relative()
            .w(px(Self::DIAMETER))
            .h(px(Self::DIAMETER))
            .child(disc)
            .child(ring)
    }
}

impl RenderOnce for EditorLinkStyleSwatches {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let mut row = div().flex().flex_row().gap(px(Self::SPACING));
        for style in RiseMediaLinkStyle::ALL {
            let selected = self.selected;
            let on_select = Rc::clone(&self.on_select);
            row = row.child(
                PressableUi::new(ElementId::Name(SharedString::from(format!("editor-link-swatch-{}", style.wire()))))
                    .tap_scale(0.92)
                    .on_press(move |window, cx| {
                        if style == selected {
                            return;
                        }
                        SelectionFeedbackGenerator::new().selection_changed();
                        on_select(style, window, cx);
                    })
                    .child(self.swatch(style)),
            );
        }
        div()
            .w_full()
            .flex()
            .justify_center()
            .py(px(Self::RING_GAP + Self::RING_WIDTH))
            .child(row)
    }
}

/// customize sheet with live card, its colors and the four preview kinds
#[derive(IntoElement)]
pub struct EditorLinkCustomizeContent {
    pub link: EditorLinkEntity,
    pub theme: AppTheme,
    pub on_select: Rc<dyn Fn(RiseMediaLinkPreviewMode, &mut Window, &mut App)>,
    pub on_select_style: Rc<dyn Fn(RiseMediaLinkStyle, &mut Window, &mut App)>,
}

impl EditorLinkCustomizeContent {
    pub fn new(
        link: EditorLinkEntity,
        theme: AppTheme,
        on_select: impl Fn(RiseMediaLinkPreviewMode, &mut Window, &mut App) + 'static,
        on_select_style: impl Fn(RiseMediaLinkStyle, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { link, theme, on_select: Rc::new(on_select), on_select_style: Rc::new(on_select_style) }
    }
}

impl RenderOnce for EditorLinkCustomizeContent {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = &self.theme;
        let current = self.link.preview_kind();
        let items: Vec<GroupedBtn> = RiseMediaLinkPreviewMode::ALL
            .into_iter()
            .map(|mode| {
                let on_select = Rc::clone(&self.on_select);
                let mut item = GroupedBtn::new(mode.wire(), tr(mode.i18n_key()))
                    .accessibility_selected(current == mode)
                    .show_chevron(false)
                    .callback(move |window, cx| on_select(mode, window, cx));
                if current == mode
                    && let Some(path) = IconUi::asset_path("checkmark")
                {
                    item = item.trailing_slot(svg().path(path).size(px(15.0)).text_color(theme.primary._100));
                }
                item
            })
            .collect();
        let on_select_style = Rc::clone(&self.on_select_style);

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .pt(px(6.0))
            .pb(px(12.0))
            .child(
                div().w_full().px(px(16.0)).child(
                    MainText::new(tr("media_link_customize"))
                        .px(17.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.primary),
                ),
            )
            .child(div().w_full().px(px(16.0)).child(EditorLinkLiveSample::new(self.link.clone())))
            .child(EditorLinkStyleSwatches::new(
                self.link.style_kind(),
                self.link.is_dark_theme,
                theme.primary._100,
                move |style, window, cx| on_select_style(style, window, cx),
            ))
            .child(GroupedBtns::new("editor-link-preview-modes", items))
    }
}
