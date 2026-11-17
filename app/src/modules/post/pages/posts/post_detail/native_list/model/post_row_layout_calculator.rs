use std::collections::HashMap;
use std::sync::LazyLock;

use gpui::{Bounds, Font as GpuiFont, FontWeight, Pixels, Point, ShapedLine, SharedString, Size, TextRun, WindowTextSystem, bounds, point, px, size};
use parking_lot::Mutex;
use rise_i18n::tr;
use rise_ui::rich_text::rich_text_markup::{RichBlock, RichTextMarkup};
use rise_ui::swiftui::font::{Font, FontDesign};
use rise_ui::swiftui::rich_text::RichFont;

use crate::core::ui::rich_text::post_rich_text_render::{PostRichTextCalibration, PostRichTextRender, PostRichTextStyle};
use crate::core::ui::rich_text::post_text_layout::PostTextLayoutBuilder;
use crate::modules::media_picker::editor::editor_music_plate::EditorMusicPlateRenderer;
use crate::modules::post::components::collaborators::post_tag_bubbles_overlay::PostMediaCornerChrome;
use crate::modules::post::components::repost::repost_overlay::RepostOverlayMetrics;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostRowActionsLayout, PostRowBodyKind, PostRowCaptionLayout, PostRowFallbackReason, PostRowHeaderLayout, PostRowLayout,
    PostRowLayoutInputs, PostRowLayoutSpec, PostRowLikesSummaryLayout, PostRowMediaAspects, PostRowMediaBoxLayout,
    PostRowModerationBannerLayout, PostRowMusicPlateLayout, PostRowMusicStripLayout, PostRowPalette, PostRowRoute, PostRowSeed,
    PostRowSlideLayout,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_prepared_cache::{
    PostRowCaptionCache, PostRowCaptionCacheKey,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_seed_extractor::PostRowSeedExtractor;
use crate::modules::post::shared::post_media_box::PostMediaBox;
use crate::modules::post::stores::post_actions::post_types::PostModerationBanner;

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
    bounds(point(px(x), px(y)), size(px(width), px(height)))
}

fn max_x(frame: &Bounds<Pixels>) -> f32 {
    f32::from(frame.origin.x + frame.size.width)
}

fn max_y(frame: &Bounds<Pixels>) -> f32 {
    f32::from(frame.origin.y + frame.size.height)
}

/// paddings and sizes of the feed row Post bodies
pub enum PostRowMetrics {}

impl PostRowMetrics {
    pub const HORIZONTAL_PAD: f32 = 12.0;

    pub const HEADER_TOP: f32 = 10.0;
    pub const HEADER_BOTTOM: f32 = 6.0;
    pub const HEADER_SPACING: f32 = 10.0;
    pub const HEADER_SPACER_MINIMUM: f32 = 8.0;
    pub const AVATAR_SIZE: f32 = 40.0;
    pub const MORE_BUTTON_SIZE: f32 = 28.0;
    pub const NAME_FONT_SIZE: f32 = 16.0;
    pub const NAME_BADGE_SPACING: f32 = 5.0;
    pub const BADGE_SIZE: f32 = 18.0;
    pub const COLUMN_LINE_SPACING: f32 = 2.0;
    pub const SECONDARY_FONT_SIZE: f32 = 12.0;
    pub const MUSIC_GLYPH_POINT_SIZE: f32 = 11.0;

    pub const BANNER_HORIZONTAL_INSET: f32 = 10.0;
    pub const BANNER_VERTICAL_INSET: f32 = 7.0;
    pub const BANNER_SPACING: f32 = 6.0;
    pub const BANNER_BOTTOM: f32 = 6.0;
    pub const BANNER_ICON_POINT_SIZE: f32 = 12.0;
    pub const BANNER_FONT_SIZE: f32 = 12.0;

    pub const STRIP_HORIZONTAL_INSET: f32 = 10.0;
    pub const STRIP_VERTICAL_INSET: f32 = 8.0;
    pub const STRIP_SPACING: f32 = 10.0;
    pub const STRIP_BOTTOM: f32 = 4.0;
    pub const STRIP_COVER_SIZE: f32 = 40.0;
    pub const STRIP_TITLE_FONT_SIZE: f32 = 14.0;
    pub const STRIP_LINE_SPACING: f32 = 2.0;
    pub const SOUND_BUTTON_TAP_SIZE: f32 = 48.0;

    pub const SOUND_BAR_PADDING: f32 = 12.0;
    pub const PAGE_INDICATOR_BOTTOM: f32 = 10.0;
    pub const PAGE_DOT_SIZE: f32 = 6.0;
    pub const PAGE_DOT_SPACING: f32 = 5.0;
    pub const PAGE_DOTS_HORIZONTAL_INSET: f32 = 8.0;
    pub const PAGE_DOTS_VERTICAL_INSET: f32 = 5.0;
    pub const PAGE_COUNTER_HORIZONTAL_INSET: f32 = 10.0;
    pub const PAGE_COUNTER_VERTICAL_INSET: f32 = 4.0;
    pub const PAGE_COUNTER_FONT_SIZE: f32 = 12.0;
    pub const PAGE_DOT_LIMIT: usize = 10;
    pub const MUSIC_PLATE_DEFAULT_CENTER_X: f32 = 0.5;
    pub const MUSIC_PLATE_DEFAULT_CENTER_Y: f32 = 0.3;

    pub const CAPTION_TOP_WITH_MEDIA: f32 = 8.0;
    pub const CAPTION_TOP_WITHOUT_MEDIA: f32 = 5.0;
    pub const CAPTION_BOTTOM: f32 = 2.0;

    pub const ACTIONS_TOP: f32 = 7.0;
    pub const REEL_ACTIONS_TOP: f32 = 5.0;
    pub const ACTIONS_BOTTOM_BEFORE_SUMMARY: f32 = 2.0;
    pub const ACTIONS_BOTTOM: f32 = 10.0;
    pub const ACTIONS_MINIMUM_HEIGHT: f32 = 30.0;
    pub const ACTIONS_SPACING: f32 = 5.0;
    pub const ACTIONS_COUNT_FONT_SIZE: f32 = 11.0;

    pub const LIKES_HORIZONTAL_PAD: f32 = 7.0;
    pub const LIKES_BOTTOM: f32 = 10.0;
    pub const LIKES_AVATAR_SIZE: f32 = 23.0;
    pub const LIKES_AVATAR_STEP: f32 = 15.0;
    pub const LIKES_SPACING: f32 = 5.0;
    pub const LIKES_FONT_SIZE: f32 = 12.0;

    pub const ACHIEVEMENT_HEIGHT: f32 = 268.0;
    pub const COAUTHOR_BANNER_TOP: f32 = 14.0;
    pub const COAUTHOR_BANNER_BOTTOM: f32 = 2.0;
    pub const COAUTHOR_AVATAR_SIZE: f32 = 42.0;
    pub const COAUTHOR_NAME_FONT_SIZE: f32 = 15.0;
    pub const COAUTHOR_SUBTITLE_FONT_SIZE: f32 = 13.0;
    pub const COAUTHOR_BADGE_SIZE: f32 = 16.0;
    pub const COAUTHOR_LINE_SPACING: f32 = 3.0;
    pub const COAUTHOR_BUTTON_HEIGHT: f32 = 34.0;
    pub const CODE_FONT_SIZE: f32 = 13.5;
    pub const CODE_BLOCK_CHROME_ESTIMATE: f32 = 44.0;
    pub const OVERSIZE_ADVANCE_EM: f32 = 0.52;
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct LineKey {
    size: u32,
    weight: u32,
    is_bold_text_enabled: bool,
    scale: u32,
}

static LINE_HEIGHTS: LazyLock<Mutex<HashMap<LineKey, f32>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// single line text and symbol metrics, measured like the text layout builder does
pub enum PostRowTextMetrics {}

impl PostRowTextMetrics {
    pub fn display_scale(spec: &PostRowLayoutSpec) -> f32 {
        if spec.display_scale.is_finite() && spec.display_scale > 0.0 {
            spec.display_scale
        } else {
            PostRichTextCalibration::FALLBACK_DISPLAY_SCALE
        }
    }

    /// main text font, px plus text delta
    /// min 1, bold text applied
    pub fn font(px: f32, weight: FontWeight, spec: &PostRowLayoutSpec) -> RichFont {
        PostRichTextRender::font(
            PostRichTextCalibration::MINIMUM_FONT_SIZE.max(px + spec.text_delta),
            weight,
            spec.is_bold_text_enabled,
        )
    }

    /// height of one line of main text
    pub fn line_height(px: f32, weight: FontWeight, spec: &PostRowLayoutSpec) -> f32 {
        let scale = Self::display_scale(spec);
        let key = LineKey {
            size: PostRichTextCalibration::MINIMUM_FONT_SIZE.max(px + spec.text_delta).to_bits(),
            weight: weight.0.to_bits(),
            is_bold_text_enabled: spec.is_bold_text_enabled,
            scale: scale.to_bits(),
        };
        if let Some(cached) = LINE_HEIGHTS.lock().get(&key).copied() {
            return cached;
        }
        let value = Self::line_height_of("", &Self::font(px, weight, spec), scale);
        if value > 0.0 {
            LINE_HEIGHTS.lock().insert(key, value);
        }
        value
    }

    /// ascent plus descent of text in font with fallback fonts, empty text measures the font
    pub fn line_height_of(text: &str, font: &RichFont, scale: f32) -> f32 {
        Self::face_line_height(text, Self::face(font), scale)
    }

    /// width of one line without trailing whitespace
    pub fn width(text: &str, font: &RichFont, scale: f32) -> f32 {
        Self::face_width(text, Self::face(font), scale)
    }

    /// bounds of a system symbol at this size
    pub fn symbol_size(_name: &str, point_size: f32, _weight: FontWeight) -> Size<Pixels> {
        size(px(point_size), px(point_size))
    }

    pub fn pixel_ceil(value: f32, scale: f32) -> f32 {
        if !value.is_finite() || value <= 0.0 {
            return 0.0;
        }
        (value * scale - 0.001).ceil().max(0.0) / scale
    }

    pub fn pixel_round(value: f32, scale: f32) -> f32 {
        if !value.is_finite() {
            return 0.0;
        }
        (value * scale).round() / scale
    }

    fn face(font: &RichFont) -> Font {
        Font {
            size: font.size,
            weight: font.weight,
            design: font.design,
            italic: font.italic,
            monospaced_digit: false,
        }
    }

    fn shaped_line(text: &str, face: &Font) -> Option<ShapedLine> {
        let text_system = PostTextLayoutBuilder::text_system()?;
        let shaper = WindowTextSystem::new(text_system);
        let single_line = text.replace(['\n', '\r'], " ");
        let run = TextRun {
            len: single_line.len(),
            font: GpuiFont {
                family: face.family(),
                features: face.features(),
                fallbacks: None,
                weight: face.weight,
                style: face.style(),
            },
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        Some(shaper.shape_line(SharedString::from(single_line), px(face.size), &[run], None))
    }

    fn face_line_height(text: &str, face: Font, scale: f32) -> f32 {
        let sample = if text.is_empty() { "Ag" } else { text };
        let Some(line) = Self::shaped_line(sample, &face) else {
            return 0.0;
        };
        Self::pixel_ceil(f32::from(line.ascent) + f32::from(line.descent), scale)
    }

    fn face_width(text: &str, face: Font, scale: f32) -> f32 {
        let visible = text.trim_end_matches(char::is_whitespace);
        if visible.is_empty() {
            return 0.0;
        }
        let Some(line) = Self::shaped_line(visible, &face) else {
            return 0.0;
        };
        Self::pixel_ceil(f32::from(line.width).max(0.0), scale)
    }
}

/// the only thing that produces native row heights and frames
pub enum PostRowLayoutCalculator {}

impl PostRowLayoutCalculator {
    /// frames of one native row, inputs has every fact that affects a frame
    pub fn layout(inputs: &PostRowLayoutInputs, palette: &PostRowPalette) -> PostRowLayout {
        let spec = &inputs.spec;
        let width = if spec.width.is_finite() { spec.width.max(1.0) } else { 1.0 };
        let content_width = (width - PostRowMetrics::HORIZONTAL_PAD * 2.0).max(1.0);

        let header = Self::header_layout(inputs, content_width);
        let mut cursor = max_y(&header.frame) + PostRowMetrics::HEADER_BOTTOM;

        let mut banner = None;
        if let Some(kind) = inputs.moderation_banner {
            let part = Self::banner_layout(kind, spec, cursor, content_width);
            cursor = max_y(&part.frame) + PostRowMetrics::BANNER_BOTTOM;
            banner = Some(part);
        }

        let mut music_strip = None;
        let mut media_box = None;
        match inputs.body {
            PostRowBodyKind::Standard => {
                if inputs.shows_music_strip {
                    let part = Self::music_strip_layout(spec, cursor, content_width);
                    cursor = max_y(&part.frame) + PostRowMetrics::STRIP_BOTTOM;
                    music_strip = Some(part);
                } else if !inputs.slide_is_video.is_empty() {
                    let part = Self::media_box_layout(inputs, cursor, width);
                    cursor = max_y(&part.frame);
                    media_box = Some(part);
                }
            }
            PostRowBodyKind::Reel => {
                let part = Self::media_box_layout(inputs, cursor, width);
                cursor = max_y(&part.frame);
                media_box = Some(part);
            }
        }

        let mut caption = None;
        if !inputs.title.is_empty() || !inputs.content.is_empty() || !inputs.hashtags.is_empty() {
            let top = if inputs.body == PostRowBodyKind::Reel || inputs.has_visual_media {
                PostRowMetrics::CAPTION_TOP_WITH_MEDIA
            } else {
                PostRowMetrics::CAPTION_TOP_WITHOUT_MEDIA
            };
            if let Some(part) = Self::caption_layout(
                inputs,
                palette,
                point(px(PostRowMetrics::HORIZONTAL_PAD), px(cursor + top)),
                content_width,
            ) {
                cursor = max_y(&part.frame) + PostRowMetrics::CAPTION_BOTTOM;
                caption = Some(part);
            }
        }

        cursor += if inputs.body == PostRowBodyKind::Reel {
            PostRowMetrics::REEL_ACTIONS_TOP
        } else {
            PostRowMetrics::ACTIONS_TOP
        };
        let actions = Self::actions_layout(spec, cursor, content_width, inputs.shows_sparks);
        cursor = max_y(&actions.frame)
            + if inputs.shows_likes_summary {
                PostRowMetrics::ACTIONS_BOTTOM_BEFORE_SUMMARY
            } else {
                PostRowMetrics::ACTIONS_BOTTOM
            };

        let mut likes_summary = None;
        if inputs.shows_likes_summary {
            let part = Self::likes_summary_layout(inputs, cursor, width);
            cursor = max_y(&part.frame) + PostRowMetrics::LIKES_BOTTOM;
            likes_summary = Some(part);
        }

        let content_height = cursor;
        PostRowLayout {
            inputs: inputs.clone(),
            header,
            moderation_banner: banner,
            music_strip,
            media_box,
            caption,
            actions,
            likes_summary,
            separator: rect(
                0.0,
                content_height + PostRowLayout::SEPARATOR_GAP,
                width,
                PostRowLayout::SEPARATOR_THICKNESS,
            ),
            content_height,
            total_height: content_height + PostRowLayout::SEPARATOR_GAP + PostRowLayout::SEPARATOR_THICKNESS,
        }
    }

    /// display aspect of the media box against the spec screen instead of the viewport
    pub fn box_aspect(natural: Option<f32>, spec: &PostRowLayoutSpec) -> f32 {
        let value = natural
            .filter(|natural| natural.is_finite() && *natural > 0.0)
            .unwrap_or(PostMediaBox::PLACEHOLDER_ASPECT);
        let screen_width = spec.screen_width.max(1.0);
        let maximum_height = (spec.screen_height * PostMediaBox::MAXIMUM_HEIGHT_FRACTION).max(1.0);
        let minimum_height = (spec.screen_height * PostMediaBox::MINIMUM_HEIGHT_FRACTION).max(1.0);
        let tallest = screen_width / maximum_height;
        let flattest = tallest.max(screen_width / minimum_height);
        flattest.min(tallest.max(value))
    }

    /// height a hosted fallback row gets before the measurer runs, never cached
    pub fn estimated_hosted_height(
        seed: &PostRowSeed,
        reason: PostRowFallbackReason,
        aspects: &PostRowMediaAspects,
        spec: &PostRowLayoutSpec,
        palette: &PostRowPalette,
    ) -> f32 {
        let width = if spec.width.is_finite() { spec.width } else { 1.0 };
        let content_width = (width - PostRowMetrics::HORIZONTAL_PAD * 2.0).max(1.0);
        let mut content = seed.content.clone();
        let mut caption_extra = 0.0_f32;
        if reason == PostRowFallbackReason::OversizeCaption {
            caption_extra = Self::oversize_caption_estimate(&seed.content, spec, content_width);
            content = String::new();
        } else if seed.caption_has_code_block {
            let mut paragraphs: Vec<String> = Vec::new();
            for block in RichTextMarkup::blocks(&seed.content).iter() {
                match block {
                    RichBlock::Paragraph(text) => paragraphs.push(text.clone()),
                    RichBlock::Code { code, .. } => {
                        caption_extra += Self::code_block_estimate(code, spec) + PostRichTextCalibration::BLOCK_SPACING;
                    }
                }
            }
            content = paragraphs.join("\n\n");
        }

        let projected = seed.replacing_content(content);
        let route = PostRowRoute::Native(PostRowSeedExtractor::native_kind(&projected));
        let inputs = PostRowLayoutInputs::new(&projected, route, spec.clone(), aspects);
        let mut height = Self::layout(&inputs, palette).total_height;

        if caption_extra > 0.0 {
            if projected.has_written_content() {
                height += caption_extra;
            } else {
                let top = if seed.body == PostRowBodyKind::Reel || seed.has_visual_media() {
                    PostRowMetrics::CAPTION_TOP_WITH_MEDIA
                } else {
                    PostRowMetrics::CAPTION_TOP_WITHOUT_MEDIA
                };
                height += (caption_extra - PostRichTextCalibration::BLOCK_SPACING).max(0.0) + top + PostRowMetrics::CAPTION_BOTTOM;
            }
        }
        if seed.body == PostRowBodyKind::Standard && seed.shared_achievement_id.is_some() {
            height += PostRowMetrics::ACHIEVEMENT_HEIGHT;
        }
        if seed.is_pending_coauthor_invite {
            let name_line = PostRowTextMetrics::line_height(PostRowMetrics::COAUTHOR_NAME_FONT_SIZE, FontWeight::SEMIBOLD, spec)
                .max(PostRowMetrics::COAUTHOR_BADGE_SIZE);
            let subtitle = PostRowTextMetrics::line_height(PostRowMetrics::COAUTHOR_SUBTITLE_FONT_SIZE, FontWeight::NORMAL, spec);
            height += PostRowMetrics::COAUTHOR_BANNER_TOP
                + PostRowMetrics::COAUTHOR_AVATAR_SIZE
                    .max(name_line + PostRowMetrics::COAUTHOR_LINE_SPACING + subtitle)
                    .max(PostRowMetrics::COAUTHOR_BUTTON_HEIGHT)
                + PostRowMetrics::COAUTHOR_BANNER_BOTTOM;
        }
        if seed.body == PostRowBodyKind::Reel && !seed.liked_by.is_empty() && seed.counts.likes <= 0 {
            height -= Self::likes_summary_height(spec) + PostRowMetrics::LIKES_BOTTOM;
        }
        height.max(1.0)
    }

    fn header_layout(inputs: &PostRowLayoutInputs, content_width: f32) -> PostRowHeaderLayout {
        let spec = &inputs.spec;
        let scale = PostRowTextMetrics::display_scale(spec);

        let name_font = PostRowTextMetrics::font(PostRowMetrics::NAME_FONT_SIZE, FontWeight::NORMAL, spec);
        let name_height = PostRowTextMetrics::line_height_of(&inputs.author_name, &name_font, scale);
        let premium_size = inputs
            .has_premium_badge
            .then_some((inputs.premium_badge_width, inputs.premium_badge_height));
        let first_line_height = name_height
            .max(if inputs.has_official_badge { PostRowMetrics::BADGE_SIZE } else { 0.0 })
            .max(premium_size.map_or(0.0, |(_, height)| height));
        let secondary_height = PostRowTextMetrics::line_height(PostRowMetrics::SECONDARY_FONT_SIZE, FontWeight::NORMAL, spec);
        let music_glyph = PostRowTextMetrics::symbol_size("music.note", PostRowMetrics::MUSIC_GLYPH_POINT_SIZE, FontWeight::MEDIUM);
        let music_height = secondary_height.max(f32::from(music_glyph.height));

        let mut column_height = first_line_height;
        if inputs.has_meta_line {
            column_height += PostRowMetrics::COLUMN_LINE_SPACING + secondary_height;
        }
        if inputs.has_header_track_line {
            column_height += PostRowMetrics::COLUMN_LINE_SPACING + music_height;
        }
        let height = PostRowMetrics::AVATAR_SIZE.max(column_height).max(PostRowMetrics::MORE_BUTTON_SIZE);

        let column_x = PostRowMetrics::AVATAR_SIZE + PostRowMetrics::HEADER_SPACING;
        let column_maximum = (content_width
            - PostRowMetrics::AVATAR_SIZE
            - PostRowMetrics::MORE_BUTTON_SIZE
            - PostRowMetrics::HEADER_SPACING * 3.0
            - PostRowMetrics::HEADER_SPACER_MINIMUM)
            .max(0.0);
        let column_y = PostRowTextMetrics::pixel_round((height - column_height) / 2.0, scale);
        let badges_width = (if inputs.has_official_badge {
            PostRowMetrics::NAME_BADGE_SPACING + PostRowMetrics::BADGE_SIZE
        } else {
            0.0
        }) + premium_size.map_or(0.0, |(width, _)| PostRowMetrics::NAME_BADGE_SPACING + width);
        let name_width =
            PostRowTextMetrics::width(&inputs.author_name, &name_font, scale).min((column_maximum - badges_width).max(0.0));
        let name = rect(
            column_x,
            column_y + PostRowTextMetrics::pixel_round((first_line_height - name_height) / 2.0, scale),
            name_width,
            name_height,
        );

        let badge_y = column_y + PostRowTextMetrics::pixel_round((first_line_height - PostRowMetrics::BADGE_SIZE) / 2.0, scale);
        let mut badge_x = max_x(&name) + PostRowMetrics::NAME_BADGE_SPACING;
        let mut official_badge = None;
        if inputs.has_official_badge {
            official_badge = Some(rect(badge_x, badge_y, PostRowMetrics::BADGE_SIZE, PostRowMetrics::BADGE_SIZE));
            badge_x += PostRowMetrics::BADGE_SIZE + PostRowMetrics::NAME_BADGE_SPACING;
        }
        let premium_badge = premium_size.map(|(width, badge_height)| {
            rect(
                badge_x,
                column_y + PostRowTextMetrics::pixel_round((first_line_height - badge_height) / 2.0, scale),
                width,
                badge_height,
            )
        });

        let mut line_y = column_y + first_line_height;
        let mut meta_line = None;
        if inputs.has_meta_line {
            line_y += PostRowMetrics::COLUMN_LINE_SPACING;
            meta_line = Some(rect(column_x, line_y, column_maximum, secondary_height));
            line_y += secondary_height;
        }
        let mut music_line = None;
        if inputs.has_header_track_line {
            line_y += PostRowMetrics::COLUMN_LINE_SPACING;
            music_line = Some(rect(column_x, line_y, column_maximum, music_height));
        }

        PostRowHeaderLayout {
            frame: rect(PostRowMetrics::HORIZONTAL_PAD, PostRowMetrics::HEADER_TOP, content_width, height),
            avatar: rect(
                0.0,
                PostRowTextMetrics::pixel_round((height - PostRowMetrics::AVATAR_SIZE) / 2.0, scale),
                PostRowMetrics::AVATAR_SIZE,
                PostRowMetrics::AVATAR_SIZE,
            ),
            text_column: rect(column_x, column_y, column_maximum.max(name_width + badges_width), column_height),
            name,
            official_badge,
            premium_badge,
            meta_line,
            music_line,
            more_button: rect(
                content_width - PostRowMetrics::MORE_BUTTON_SIZE,
                PostRowTextMetrics::pixel_round((height - PostRowMetrics::MORE_BUTTON_SIZE) / 2.0, scale),
                PostRowMetrics::MORE_BUTTON_SIZE,
                PostRowMetrics::MORE_BUTTON_SIZE,
            ),
        }
    }

    fn banner_layout(kind: PostModerationBanner, spec: &PostRowLayoutSpec, y: f32, content_width: f32) -> PostRowModerationBannerLayout {
        let scale = PostRowTextMetrics::display_scale(spec);
        let icon = PostRowTextMetrics::symbol_size(kind.symbol_name(), PostRowMetrics::BANNER_ICON_POINT_SIZE, FontWeight::SEMIBOLD);
        let icon_width = f32::from(icon.width);
        let icon_height = f32::from(icon.height);
        let line_height = PostRowTextMetrics::line_height(PostRowMetrics::BANNER_FONT_SIZE, FontWeight::MEDIUM, spec);
        let label_x = PostRowMetrics::BANNER_HORIZONTAL_INSET + icon_width + PostRowMetrics::BANNER_SPACING;
        let label_width = (content_width - label_x - PostRowMetrics::BANNER_HORIZONTAL_INSET).max(0.0);
        let font = PostRowTextMetrics::font(PostRowMetrics::BANNER_FONT_SIZE, FontWeight::MEDIUM, spec);
        let lines = Self::banner_lines(&tr(kind.text_key()), &font, scale, label_width);
        let label_height = line_height * lines.len() as f32;
        let inner_height = label_height.max(icon_height);
        let height = PostRowMetrics::BANNER_VERTICAL_INSET * 2.0 + inner_height;
        PostRowModerationBannerLayout {
            banner: kind,
            frame: rect(PostRowMetrics::HORIZONTAL_PAD, y, content_width, height),
            icon: rect(
                PostRowMetrics::BANNER_HORIZONTAL_INSET,
                PostRowMetrics::BANNER_VERTICAL_INSET + PostRowTextMetrics::pixel_round((inner_height - icon_height) / 2.0, scale),
                icon_width,
                icon_height,
            ),
            label: rect(
                label_x,
                PostRowMetrics::BANNER_VERTICAL_INSET + PostRowTextMetrics::pixel_round((inner_height - label_height) / 2.0, scale),
                label_width,
                label_height,
            ),
            lines,
        }
    }

    fn banner_lines(text: &str, font: &RichFont, scale: f32, width: f32) -> Vec<String> {
        if width <= 0.0 || PostRowTextMetrics::width(text, font, scale) <= width {
            return vec![text.to_owned()];
        }
        let mut split = None;
        for (index, _) in text.match_indices(' ') {
            if PostRowTextMetrics::width(&text[..index], font, scale) > width {
                break;
            }
            split = Some(index);
        }
        match split {
            Some(index) => vec![text[..index].to_owned(), text[index..].trim_start().to_owned()],
            None => vec![text.to_owned()],
        }
    }

    fn music_strip_layout(spec: &PostRowLayoutSpec, y: f32, content_width: f32) -> PostRowMusicStripLayout {
        let scale = PostRowTextMetrics::display_scale(spec);
        let title_height = PostRowTextMetrics::line_height(PostRowMetrics::STRIP_TITLE_FONT_SIZE, FontWeight::MEDIUM, spec);
        let duration_height = PostRowTextMetrics::line_height(PostRowMetrics::SECONDARY_FONT_SIZE, FontWeight::NORMAL, spec);
        let column_height = title_height + PostRowMetrics::STRIP_LINE_SPACING + duration_height;
        let inner_height = PostRowMetrics::STRIP_COVER_SIZE.max(column_height).max(PostRowMetrics::SOUND_BUTTON_TAP_SIZE);
        let height = PostRowMetrics::STRIP_VERTICAL_INSET * 2.0 + inner_height;
        let column_x = PostRowMetrics::STRIP_HORIZONTAL_INSET + PostRowMetrics::STRIP_COVER_SIZE + PostRowMetrics::STRIP_SPACING;
        let column_width = (content_width
            - PostRowMetrics::STRIP_HORIZONTAL_INSET * 2.0
            - PostRowMetrics::STRIP_COVER_SIZE
            - PostRowMetrics::SOUND_BUTTON_TAP_SIZE
            - PostRowMetrics::STRIP_SPACING * 3.0)
            .max(0.0);
        let column_y = PostRowMetrics::STRIP_VERTICAL_INSET + PostRowTextMetrics::pixel_round((inner_height - column_height) / 2.0, scale);
        PostRowMusicStripLayout {
            frame: rect(PostRowMetrics::HORIZONTAL_PAD, y, content_width, height),
            cover: rect(
                PostRowMetrics::STRIP_HORIZONTAL_INSET,
                PostRowMetrics::STRIP_VERTICAL_INSET
                    + PostRowTextMetrics::pixel_round((inner_height - PostRowMetrics::STRIP_COVER_SIZE) / 2.0, scale),
                PostRowMetrics::STRIP_COVER_SIZE,
                PostRowMetrics::STRIP_COVER_SIZE,
            ),
            title_line: rect(column_x, column_y, column_width, title_height),
            duration_line: rect(
                column_x,
                column_y + title_height + PostRowMetrics::STRIP_LINE_SPACING,
                column_width,
                duration_height,
            ),
            sound_button: rect(
                content_width - PostRowMetrics::STRIP_HORIZONTAL_INSET - PostRowMetrics::SOUND_BUTTON_TAP_SIZE,
                PostRowMetrics::STRIP_VERTICAL_INSET
                    + PostRowTextMetrics::pixel_round((inner_height - PostRowMetrics::SOUND_BUTTON_TAP_SIZE) / 2.0, scale),
                PostRowMetrics::SOUND_BUTTON_TAP_SIZE,
                PostRowMetrics::SOUND_BUTTON_TAP_SIZE,
            ),
        }
    }

    fn media_box_layout(inputs: &PostRowLayoutInputs, y: f32, width: f32) -> PostRowMediaBoxLayout {
        let spec = &inputs.spec;
        let scale = PostRowTextMetrics::display_scale(spec);
        let is_standard = inputs.body == PostRowBodyKind::Standard;
        let natural_first = inputs.slide_aspects.first().copied().flatten();
        let box_aspect = Self::box_aspect(natural_first, spec);
        let height = PostRowTextMetrics::pixel_round(width / box_aspect, scale).max(1.0);

        let slides: Vec<PostRowSlideLayout> = inputs
            .slide_is_video
            .iter()
            .enumerate()
            .map(|(index, is_video)| {
                let natural = inputs.slide_aspects.get(index).copied().flatten();
                PostRowSlideLayout {
                    natural_aspect: natural,
                    fills: PostMediaBox::fills(natural, box_aspect),
                    is_video: *is_video,
                }
            })
            .collect();

        let mut page_indicator = None;
        if is_standard && slides.len() > 1 {
            let (indicator_width, indicator_height) = Self::page_indicator_size(slides.len(), spec);
            page_indicator = Some(rect(
                PostRowTextMetrics::pixel_round((width - indicator_width) / 2.0, scale),
                height - PostRowMetrics::PAGE_INDICATOR_BOTTOM - indicator_height,
                indicator_width,
                indicator_height,
            ));
        }

        let mut sound_button = None;
        if is_standard && inputs.shows_sound_bar {
            sound_button = Some(rect(
                width - PostRowMetrics::SOUND_BAR_PADDING - PostRowMetrics::SOUND_BUTTON_TAP_SIZE,
                height - PostRowMetrics::SOUND_BAR_PADDING - PostRowMetrics::SOUND_BUTTON_TAP_SIZE,
                PostRowMetrics::SOUND_BUTTON_TAP_SIZE,
                PostRowMetrics::SOUND_BUTTON_TAP_SIZE,
            ));
        }

        let mut music_plate = None;
        if is_standard
            && inputs.has_header_track_line
            && let Some(plate) = inputs.track_plate.as_ref()
        {
            let short_side = width.min(height).max(1.0);
            music_plate = Some(PostRowMusicPlateLayout {
                center: point(px(plate.center_x as f32 * width), px(plate.center_y as f32 * height)),
                base_width: EditorMusicPlateRenderer::BASE_WIDTH_FRACTION * short_side,
                scale: plate.scale as f32,
                rotation: plate.rotation as f32,
                is_dark: plate.is_dark,
            });
        }

        let corner_bottom = if inputs.has_header_track_line {
            RepostOverlayMetrics::MEDIA_BOTTOM_SAFE_ZONE
        } else {
            PostMediaCornerChrome::INSET
        };
        PostRowMediaBoxLayout {
            frame: rect(0.0, y, width, height),
            box_aspect,
            is_aspect_provisional: inputs.is_aspect_provisional,
            slides,
            page_indicator,
            sound_button,
            corner_row_origin: point(px(PostMediaCornerChrome::INSET), px(height - corner_bottom)),
            music_plate,
        }
    }

    fn page_indicator_size(count: usize, spec: &PostRowLayoutSpec) -> (f32, f32) {
        if count <= PostRowMetrics::PAGE_DOT_LIMIT {
            let dots = count as f32;
            return (
                PostRowMetrics::PAGE_DOTS_HORIZONTAL_INSET * 2.0
                    + dots * PostRowMetrics::PAGE_DOT_SIZE
                    + (dots - 1.0).max(0.0) * PostRowMetrics::PAGE_DOT_SPACING,
                PostRowMetrics::PAGE_DOTS_VERTICAL_INSET * 2.0 + PostRowMetrics::PAGE_DOT_SIZE,
            );
        }
        let scale = PostRowTextMetrics::display_scale(spec);
        let face = Font {
            size: PostRowMetrics::PAGE_COUNTER_FONT_SIZE,
            weight: PostRichTextRender::legibility_weight(FontWeight::SEMIBOLD, spec.is_bold_text_enabled),
            design: FontDesign::Rounded,
            italic: false,
            monospaced_digit: true,
        };
        let text = format!("{count}/{count}");
        (
            PostRowTextMetrics::face_width(&text, face, scale) + PostRowMetrics::PAGE_COUNTER_HORIZONTAL_INSET * 2.0,
            PostRowTextMetrics::face_line_height(&text, face, scale) + PostRowMetrics::PAGE_COUNTER_VERTICAL_INSET * 2.0,
        )
    }

    fn caption_layout(
        inputs: &PostRowLayoutInputs,
        palette: &PostRowPalette,
        origin: Point<Pixels>,
        width: f32,
    ) -> Option<PostRowCaptionLayout> {
        let spec = &inputs.spec;
        let key = PostRowCaptionCacheKey {
            title: inputs.title.clone(),
            content: inputs.content.clone(),
            hashtags: inputs.hashtags.clone(),
            width,
            text_delta: spec.text_delta,
            theme_id: spec.theme_id.clone(),
            is_bold_text_enabled: spec.is_bold_text_enabled,
            display_scale: PostRowTextMetrics::display_scale(spec),
            locale_identifier: spec.locale_identifier.clone(),
            is_expanded: inputs.is_caption_expanded,
        };
        if let Some(cached) = PostRowCaptionCache::shared().caption(&key) {
            return Some(cached.moved_to(origin));
        }
        let caption = PostRichTextRender::caption_layout(
            &inputs.title,
            &inputs.content,
            &inputs.hashtags,
            &PostRichTextStyle::from_palette(palette, spec),
            width,
            point(px(0.0), px(0.0)),
            Some(PostRichTextCalibration::CAPTION_CONTENT_LINE_LIMIT),
            inputs.is_caption_expanded,
        )?;
        let moved = caption.moved_to(origin);
        PostRowCaptionCache::shared().insert(caption, key);
        Some(moved)
    }

    fn actions_layout(spec: &PostRowLayoutSpec, y: f32, content_width: f32, shows_sparks: bool) -> PostRowActionsLayout {
        let height = PostRowMetrics::ACTIONS_MINIMUM_HEIGHT
            .max(PostRowTextMetrics::line_height(PostRowMetrics::ACTIONS_COUNT_FONT_SIZE, FontWeight::NORMAL, spec));
        let cells = if shows_sparks { 5.0_f32 } else { 4.0_f32 };
        let cell_width = ((content_width - PostRowMetrics::ACTIONS_SPACING * (cells - 1.0)) / cells).max(0.0);
        let cell = |index: usize| {
            rect(
                index as f32 * (cell_width + PostRowMetrics::ACTIONS_SPACING),
                0.0,
                cell_width,
                height,
            )
        };
        let shift = usize::from(shows_sparks);
        PostRowActionsLayout {
            frame: rect(PostRowMetrics::HORIZONTAL_PAD, y, content_width, height),
            like: cell(0),
            favorite: cell(1),
            comments: cell(2 + shift),
            repost: cell(3 + shift),
            sparks: shows_sparks.then(|| cell(2)),
        }
    }

    fn likes_summary_height(spec: &PostRowLayoutSpec) -> f32 {
        PostRowMetrics::LIKES_AVATAR_SIZE.max(PostRowTextMetrics::line_height(PostRowMetrics::LIKES_FONT_SIZE, FontWeight::NORMAL, spec))
    }

    fn likes_summary_layout(inputs: &PostRowLayoutInputs, y: f32, width: f32) -> PostRowLikesSummaryLayout {
        let spec = &inputs.spec;
        let scale = PostRowTextMetrics::display_scale(spec);
        let frame_width = (width - PostRowMetrics::LIKES_HORIZONTAL_PAD * 2.0).max(0.0);
        let height = Self::likes_summary_height(spec);
        let text_height = PostRowTextMetrics::line_height(PostRowMetrics::LIKES_FONT_SIZE, FontWeight::NORMAL, spec);
        let count = inputs.liked_by_avatar_count.clamp(0, 3);
        let avatar_y = PostRowTextMetrics::pixel_round((height - PostRowMetrics::LIKES_AVATAR_SIZE) / 2.0, scale);
        let avatars: Vec<Bounds<Pixels>> = (0..count)
            .map(|index| {
                rect(
                    index as f32 * PostRowMetrics::LIKES_AVATAR_STEP,
                    avatar_y,
                    PostRowMetrics::LIKES_AVATAR_SIZE,
                    PostRowMetrics::LIKES_AVATAR_SIZE,
                )
            })
            .collect();
        let stack_width = if count > 0 {
            PostRowMetrics::LIKES_AVATAR_SIZE + (count - 1) as f32 * PostRowMetrics::LIKES_AVATAR_STEP
        } else {
            0.0
        };
        let text_x = if count > 0 { stack_width + PostRowMetrics::LIKES_SPACING } else { 0.0 };
        PostRowLikesSummaryLayout {
            frame: rect(PostRowMetrics::LIKES_HORIZONTAL_PAD, y, frame_width, height),
            avatars,
            text: rect(
                text_x,
                PostRowTextMetrics::pixel_round((height - text_height) / 2.0, scale),
                (frame_width - text_x).max(0.0),
                text_height,
            ),
        }
    }

    fn code_block_estimate(code: &str, spec: &PostRowLayoutSpec) -> f32 {
        let scale = PostRowTextMetrics::display_scale(spec);
        let font = PostRichTextRender::monospaced_font(PostRowMetrics::CODE_FONT_SIZE, false);
        let line_height = PostRowTextMetrics::line_height_of("", &font, scale);
        let lines = 1 + code.matches('\n').count();
        lines as f32 * line_height + PostRowMetrics::CODE_BLOCK_CHROME_ESTIMATE
    }

    fn oversize_caption_estimate(content: &str, spec: &PostRowLayoutSpec, width: f32) -> f32 {
        let font_size = PostRichTextCalibration::MINIMUM_FONT_SIZE.max(PostRichTextCalibration::PARAGRAPH_FONT_SIZE + spec.text_delta);
        let line_height = PostRowTextMetrics::line_height(PostRichTextCalibration::PARAGRAPH_FONT_SIZE, FontWeight::NORMAL, spec)
            + PostRichTextCalibration::PARAGRAPH_LINE_SPACING;
        let characters_per_line = ((width / (font_size * PostRowMetrics::OVERSIZE_ADVANCE_EM).max(1.0)) as i64).max(1);
        let mut lines = 0_i64;
        let mut run = 0_i64;
        for character in content.encode_utf16() {
            if character == 0x0A {
                lines += ((run + characters_per_line - 1) / characters_per_line).max(1);
                run = 0;
            } else {
                run += 1;
            }
        }
        lines += ((run + characters_per_line - 1) / characters_per_line).max(1);
        lines as f32 * line_height
    }
}

impl PostRowSeed {
    pub fn replacing_content(&self, new_content: String) -> PostRowSeed {
        PostRowSeed {
            caption_has_code_block: false,
            caption_utf16_count: new_content.encode_utf16().count() as i64,
            content: new_content,
            ..self.clone()
        }
    }
}
