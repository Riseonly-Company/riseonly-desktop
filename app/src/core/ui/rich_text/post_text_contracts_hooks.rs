use std::any::Any;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use gpui::{App, Bounds, Hsla, Pixels, Point, Rgba, Size, Window, px, rgb, size};
use rise_core::foundation::foundation_url::Url;
use rise_theme::theme::app_theme::ThemePaletteReadable;
use rise_theme::{ThemePalette, color};

fn hash_f32<H: Hasher>(value: f32, state: &mut H) {
    let normalized = if value == 0.0 { 0.0_f32 } else { value };
    normalized.to_bits().hash(state);
}

/// environment facts every row layout depends on, controller builds it on main
#[derive(Clone, Debug, PartialEq)]
pub struct PostRowLayoutSpec {
    pub width: f32,
    pub screen_width: f32,
    pub screen_height: f32,
    pub display_scale: f32,
    pub text_delta: f32,
    pub locale_identifier: String,
    pub theme_id: String,
    pub is_bold_text_enabled: bool,
}

impl Eq for PostRowLayoutSpec {}

impl Hash for PostRowLayoutSpec {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_f32(self.width, state);
        hash_f32(self.screen_width, state);
        hash_f32(self.screen_height, state);
        hash_f32(self.display_scale, state);
        hash_f32(self.text_delta, state);
        self.locale_identifier.hash(state);
        self.theme_id.hash(state);
        self.is_bold_text_enabled.hash(state);
    }
}

impl PostRowLayoutSpec {
    pub fn new(
        width: f32,
        screen_size: Size<Pixels>,
        display_scale: f32,
        text_delta: f32,
        locale_identifier: String,
        theme_id: String,
        is_bold_text_enabled: bool,
    ) -> Self {
        Self {
            width,
            screen_width: f32::from(screen_size.width),
            screen_height: f32::from(screen_size.height),
            display_scale,
            text_delta,
            locale_identifier,
            theme_id,
            is_bold_text_enabled,
        }
    }

    /// screen size for the media box clamp
    pub fn screen_size(&self) -> Size<Pixels> {
        size(px(self.screen_width), px(self.screen_height))
    }
}

/// what a tap on the token does
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PostTextTokenKind {
    Mention { tag: String, url: Url },
    Url(Url),
    Hashtag(String),
}

/// a mention, url or hashtag inside a prepared text layout
#[derive(Clone, Debug, PartialEq)]
pub struct PostTextToken {
    pub kind: PostTextTokenKind,
    pub utf16_range: Range<usize>,
    pub rects: Vec<Bounds<Pixels>>,
}

/// immutable text layout of one block, prepared off main
pub trait PostTextLayouting: Send + Sync {
    /// size rounded up to whole pixels, the block frame size
    fn size(&self) -> Size<Pixels>;
    fn line_count(&self) -> usize;
    /// the laid out string, for accessibility and token ranges
    fn plain_text(&self) -> &str;
    fn tokens(&self) -> &[PostTextToken];
    /// paints with the top left at origin, colors come from attributes
    fn draw(&self, origin: Point<Pixels>, window: &mut Window, cx: &mut App);
    fn as_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;

    /// first token whose rect grown by slop contains at, in layout coords
    fn token(&self, at: Point<Pixels>, slop: f32) -> Option<&PostTextToken> {
        self.tokens().iter().find(|token| token.rects.iter().any(|rect| rect.dilate(px(slop)).contains(&at)))
    }
}

/// one prepared text block, frame is local to its caption
#[derive(Clone)]
pub struct PostRowTextBlockLayout {
    pub frame: Bounds<Pixels>,
    pub text: Arc<dyn PostTextLayouting>,
}

/// one rich paragraph with its optional link subtitle
#[derive(Clone)]
pub struct PostRowParagraphLayout {
    pub body: PostRowTextBlockLayout,
    pub link_subtitle: Option<PostRowTextBlockLayout>,
}

/// caption part, block frames are local to frame
#[derive(Clone)]
pub struct PostRowCaptionLayout {
    pub frame: Bounds<Pixels>,
    pub title: Option<PostRowTextBlockLayout>,
    pub paragraphs: Vec<PostRowParagraphLayout>,
    pub hashtags: Option<PostRowTextBlockLayout>,
    pub toggle: Option<PostRowTextBlockLayout>,
}

/// resolved theme colors for rows and text builders, done once per palette
#[derive(Clone, Debug, PartialEq)]
pub struct PostRowPalette {
    pub theme_id: String,
    pub text_primary: Hsla,
    pub text_secondary: Hsla,
    pub background100: Hsla,
    pub background300: Hsla,
    pub border100: Hsla,
    pub primary100: Hsla,
    pub primary200: Hsla,
    pub primary300: Hsla,
    pub error100: Hsla,
    pub link: Hsla,
    pub official_badge: Hsla,
    pub story_ring_seen: Hsla,
}

impl PostRowPalette {
    pub fn new(palette: &ThemePalette) -> Self {
        Self {
            theme_id: [
                palette.text_100.as_str(),
                palette.secondary_100.as_str(),
                palette.bg_100.as_str(),
                palette.bg_300.as_str(),
                palette.border_100.as_str(),
                palette.primary_100.as_str(),
                palette.primary_200.as_str(),
                palette.primary_300.as_str(),
                palette.error_100.as_str(),
            ]
            .join("|"),
            text_primary: palette.readable_text_100(),
            text_secondary: palette.readable_secondary_100(),
            background100: color(&palette.bg_100),
            background300: palette.readable_surface(&palette.bg_300),
            border100: palette.readable_surface(&palette.border_100),
            primary100: color(&palette.primary_100),
            primary200: color(&palette.primary_200),
            primary300: color(&palette.primary_300),
            error100: color(&palette.error_100),
            link: Rgba { r: 0.07, g: 0.45, b: 0.94, a: 1.0 }.into(),
            official_badge: Rgba { r: 1.0, g: 0.353, b: 0.353, a: 1.0 }.into(),
            story_ring_seen: Rgba { r: 0.24, g: 0.24, b: 0.26, a: 1.0 }.into(),
        }
    }

    /// premium name color from the seed author rgb
    pub fn premium_name_color(value: u32) -> Hsla {
        rgb(value).into()
    }
}

impl PostRowCaptionLayout {
    /// same blocks with the part frame at origin, block frames stay local
    pub fn moved_to(&self, origin: Point<Pixels>) -> PostRowCaptionLayout {
        PostRowCaptionLayout {
            frame: Bounds {
                origin,
                size: self.frame.size,
            },
            title: self.title.clone(),
            paragraphs: self.paragraphs.clone(),
            hashtags: self.hashtags.clone(),
            toggle: self.toggle.clone(),
        }
    }
}
