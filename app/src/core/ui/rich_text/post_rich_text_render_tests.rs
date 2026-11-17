use gpui::{Bounds, FontWeight, Hsla, Pixels, Rgba, TestAppContext, point, px};
use rise_core::foundation::ns_range::{self, NSRange};
use rise_ui::rich_text::rich_text_markup::FontDescriptorSymbolicTraits;
use rise_ui::swiftui::rich_text::{LineStyle, RichFont};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::rich_text::post_rich_text_render::{
    PostRichTextAttributedString, PostRichTextCalibration, PostRichTextColors, PostRichTextRender,
    PostRichTextRoleMention, PostRichTextStyle,
};
use crate::core::ui::rich_text::post_text_layout::PostTextLayoutBuilder;
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostTextLayouting, PostTextTokenKind,
};

fn color(r: f32, g: f32, b: f32, a: f32) -> Hsla {
    Rgba { r, g, b, a }.into()
}

fn colors() -> PostRichTextColors {
    PostRichTextColors::new(
        color(0.10, 0.10, 0.10, 1.0),
        color(0.50, 0.50, 0.50, 1.0),
        color(0.07, 0.45, 0.94, 1.0),
        color(0.90, 0.20, 0.30, 1.0),
        color(0.20, 0.25, 0.30, 1.0),
        color(0.10, 0.10, 0.10, 0.16),
    )
}

fn style() -> PostRichTextStyle {
    PostRichTextStyle::new(colors(), 0.0, 3.0, false)
}

fn bind_text_system(cx: &mut TestAppContext) {
    cx.update(|cx| EditorImageBake::install(cx));
}

fn tags(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn font(string: &PostRichTextAttributedString, location: usize) -> Option<RichFont> {
    string.font_at(location)
}

fn color_at(string: &PostRichTextAttributedString, location: usize) -> Option<Hsla> {
    string.foreground_color_at(location)
}

fn is_mention(kind: &PostTextTokenKind) -> bool {
    matches!(kind, PostTextTokenKind::Mention { .. })
}

fn max_x(rect: Bounds<Pixels>) -> f32 {
    f32::from(rect.origin.x + rect.size.width)
}

fn max_y(rect: Bounds<Pixels>) -> f32 {
    f32::from(rect.origin.y + rect.size.height)
}

fn location_of(text: &str, needle: &str) -> usize {
    ns_range::range_of(text, needle).map_or(usize::MAX, |range: NSRange| range.location)
}

#[gpui::test]
fn bold_span_uses_bold_weight_and_drops_markers(cx: &mut TestAppContext) {
    bind_text_system(cx);
    let body = PostRichTextRender::paragraph("a **bold** b", &style(), &[]).body;

    assert_eq!(body.string(), "a bold b");
    assert_eq!(font(&body, 0), Some(PostRichTextRender::font(15.0, FontWeight::NORMAL, false)));
    assert_eq!(font(&body, 2), Some(PostRichTextRender::font(15.0, FontWeight::BOLD, false)));
    assert_eq!(font(&body, 7), Some(PostRichTextRender::font(15.0, FontWeight::NORMAL, false)));
    assert_eq!(color_at(&body, 2), Some(colors().text));
}

#[gpui::test]
fn italic_span_uses_italic_font(cx: &mut TestAppContext) {
    bind_text_system(cx);
    let body = PostRichTextRender::paragraph("an _italic_ word", &style(), &[]).body;
    let is_italic = |location: usize| {
        font(&body, location).is_some_and(|font| font.symbolic_traits().contains(FontDescriptorSymbolicTraits::TRAIT_ITALIC))
    };

    assert_eq!(body.string(), "an italic word");
    assert!(is_italic(3));
    assert!(!is_italic(0));
}

#[gpui::test]
fn strikethrough_span_is_drawn_as_bar(cx: &mut TestAppContext) {
    bind_text_system(cx);
    let body = PostRichTextRender::paragraph("a ~~gone~~ b", &style(), &[]).body;

    assert_eq!(body.string(), "a gone b");
    assert_eq!(body.attributes_at(2).and_then(|attributes| attributes.strikethrough_style), Some(LineStyle::single()));
    assert_eq!(body.attributes_at(0).and_then(|attributes| attributes.strikethrough_style), None);

    let layout = PostTextLayoutBuilder::layout(&body, 320.0, None, 3.0);
    assert_eq!(layout.strikethroughs.len(), 1);
    let bar = layout.strikethroughs[0].rect;
    let line = &layout.lines[0];
    assert!(f32::from(bar.origin.y) > line.top);
    assert!(max_y(bar) < f32::from(line.baseline_origin.y));
    assert!(f32::from(bar.size.width) > 0.0);
}

#[gpui::test]
fn inline_code_is_monospaced_with_run_background(cx: &mut TestAppContext) {
    bind_text_system(cx);
    let body = PostRichTextRender::paragraph("run `ls -la` now", &style(), &[]).body;

    assert_eq!(body.string(), "run ls -la now");
    assert_eq!(font(&body, 4), Some(PostRichTextRender::monospaced_font(15.0 * 0.95, false)));
    assert_eq!(color_at(&body, 4), Some(colors().code_foreground));
    assert_eq!(body.attributes_at(4).and_then(|attributes| attributes.background_color), Some(colors().code_background));
    assert_eq!(body.attributes_at(0).and_then(|attributes| attributes.background_color), None);

    let layout = PostTextLayoutBuilder::layout(&body, 320.0, None, 3.0);
    assert_eq!(layout.backgrounds.len(), 1);
    let fill = layout.backgrounds[0].rect;
    assert!(f32::from(fill.origin.x) > 0.0);
    assert!(f32::from(fill.size.width) > 0.0);
}

#[gpui::test]
fn url_stays_plain_text_with_no_token_and_no_subtitle(cx: &mut TestAppContext) {
    bind_text_system(cx);
    let text = "see https://riseonly.net/about now";
    let paragraph = PostRichTextRender::paragraph(text, &style(), &[]);
    let url_location = location_of(paragraph.body.string(), "https://");

    assert_eq!(color_at(&paragraph.body, url_location), Some(colors().text));
    assert_eq!(color_at(&paragraph.body, 0), Some(colors().text));
    assert_eq!(paragraph.subtitle, None);

    let layouts = PostRichTextRender::paragraph_layouts(text, &style(), 320.0, &[], None);
    assert!(!layouts.body.tokens.iter().any(|token| matches!(token.kind, PostTextTokenKind::Url(_))));
    assert!(layouts.link_subtitle.is_none());
}

#[gpui::test]
fn mention_is_link_coloured_and_tappable(cx: &mut TestAppContext) {
    bind_text_system(cx);
    let text = "hi @alice!";
    let body = PostRichTextRender::paragraph(text, &style(), &[]).body;

    assert_eq!(color_at(&body, 3), Some(colors().link));
    assert_eq!(color_at(&body, 0), Some(colors().text));

    let url = RiseonlyPublicLink::PublicTag { tag: "alice".to_owned() }
        .web_url(AppEnvironment::current())
        .expect("a public tag has a web url");
    let layout = PostRichTextRender::paragraph_layouts(text, &style(), 320.0, &[], None).body;
    let token = layout.tokens.first().expect("the mention is a token");
    assert_eq!(token.kind, PostTextTokenKind::Mention { tag: "alice".to_owned(), url });
    assert_eq!(token.utf16_range, 3..9);
    let rect = token.rects[0];
    let middle_y = rect.origin.y + rect.size.height / 2.0;
    let middle_x = rect.origin.x + rect.size.width / 2.0;
    assert_eq!(layout.token(point(middle_x, middle_y), 0.0), Some(token));
    assert_eq!(layout.token(point(px(0.5), middle_y), 0.0), None);
}

#[gpui::test]
fn role_mention_is_semibold_coloured_and_not_tappable(cx: &mut TestAppContext) {
    bind_text_system(cx);
    let role_color = color(0.80, 0.10, 0.10, 1.0);
    let raw = "**hey** @team go";
    let roles = [PostRichTextRoleMention {
        raw_range: ns_range::range_of(raw, "@team").expect("literal holds the mention"),
        color: role_color,
    }];
    let body = PostRichTextRender::paragraph(raw, &style(), &roles).body;
    let location = location_of(body.string(), "@team");

    assert_eq!(body.string(), "hey @team go");
    assert_eq!(location, 4);
    assert_eq!(font(&body, location), Some(PostRichTextRender::font(15.0, FontWeight::SEMIBOLD, false)));
    assert_eq!(color_at(&body, location), Some(role_color));

    let layout = PostRichTextRender::paragraph_layouts(raw, &style(), 320.0, &roles, None).body;
    assert!(!layout.tokens.iter().any(|token| is_mention(&token.kind)));
}

#[gpui::test]
fn subtitle_never_shows(cx: &mut TestAppContext) {
    bind_text_system(cx);
    assert_eq!(PostRichTextRender::paragraph("no links **here**", &style(), &[]).subtitle, None);
    assert!(PostRichTextRender::paragraph_layouts("no links **here**", &style(), 320.0, &[], None).link_subtitle.is_none());
    assert!(PostRichTextRender::paragraph_layouts("hello @alice", &style(), 320.0, &[], None).link_subtitle.is_none());

    let linked = PostRichTextRender::paragraph_layouts("go to https://riseonly.net", &style(), 320.0, &[], None);
    assert!(linked.link_subtitle.is_none());
}

#[gpui::test]
fn line_spacing_separates_lines_and_size_is_pixel_ceiled(cx: &mut TestAppContext) {
    bind_text_system(cx);
    let body = PostRichTextRender::paragraph("first line\nsecond line\nthird", &style(), &[]).body;
    let layout = PostTextLayoutBuilder::layout(&body, 320.0, None, 3.0);

    assert_eq!(layout.line_count(), 3);
    let first = &layout.lines[0];
    let second = &layout.lines[1];
    assert!((second.top - (first.top + first.height) - 3.0).abs() < 0.001);

    let last = &layout.lines[2];
    let bottom = last.top + last.height;
    let height = f32::from(layout.size.height);
    assert!(height >= bottom - 0.001);
    assert!(height - bottom < 1.0 / 3.0 + 0.001);
    assert!(((height * 3.0).round() - height * 3.0).abs() < 0.001);
}

#[gpui::test]
fn hashtags_stay_on_one_line_with_spaced_tokens(cx: &mut TestAppContext) {
    bind_text_system(cx);
    let wide = PostRichTextRender::hashtags_layout(&tags(&["swift", "ios"]), &style(), 1_000.0);

    assert_eq!(wide.plain_text, "#swift #ios");
    assert_eq!(wide.line_count(), 1);
    assert!(!wide.is_truncated);
    let kinds: Vec<PostTextTokenKind> = wide.tokens.iter().map(|token| token.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![PostTextTokenKind::Hashtag("swift".to_owned()), PostTextTokenKind::Hashtag("ios".to_owned())]
    );
    let first_rect = wide.tokens[0].rects[0];
    let second_rect = wide.tokens[1].rects[0];
    assert!((f32::from(second_rect.origin.x) - max_x(first_rect) - 4.0).abs() < 0.05);

    let narrow = PostRichTextRender::hashtags_layout(&tags(&["swift", "ios", "uikit", "coretext"]), &style(), 90.0);
    assert_eq!(narrow.line_count(), 1);
    assert!(narrow.is_truncated);
    assert!(f32::from(narrow.size.width) <= 90.0);
}

#[gpui::test]
fn caption_stacks_blocks_and_refuses_code_fences(cx: &mut TestAppContext) {
    bind_text_system(cx);
    let caption = PostRichTextRender::caption_layout(
        "Title",
        "one\n\ntwo https://riseonly.net",
        &tags(&["tag"]),
        &style(),
        300.0,
        point(px(12.0), px(40.0)),
        None,
        false,
    )
    .expect("written text has a caption");
    let title = caption.title.as_ref().expect("title block");
    let paragraph = caption.paragraphs.first().expect("paragraph block");
    let hashtags = caption.hashtags.as_ref().expect("hashtags block");

    assert_eq!(caption.paragraphs.len(), 1);
    assert_eq!(f32::from(title.frame.origin.y), 0.0);
    assert!((f32::from(paragraph.body.frame.origin.y) - (max_y(title.frame) + 8.0)).abs() < 0.001);
    assert!(paragraph.link_subtitle.is_none());
    assert!((f32::from(hashtags.frame.origin.y) - (max_y(paragraph.body.frame) + 8.0)).abs() < 0.001);
    assert_eq!(caption.frame.origin, point(px(12.0), px(40.0)));
    assert!((f32::from(caption.frame.size.height) - max_y(hashtags.frame)).abs() < 0.001);

    let code = PostRichTextRender::caption_layout(
        "",
        "```\nlet x = 1\n```",
        &[],
        &style(),
        300.0,
        point(px(0.0), px(0.0)),
        None,
        false,
    );
    assert!(code.is_none());
}

#[gpui::test]
fn bitmap_matches_pixel_size_and_respects_budget(cx: &mut TestAppContext) {
    bind_text_system(cx);
    let layout = PostRichTextRender::title_layout("Title", &style(), 200.0);
    let bitmap = layout
        .make_bitmap(Some(2.0), PostRichTextCalibration::BITMAP_PIXEL_BUDGET, &|| false)
        .expect("a small title fits the budget");

    assert_eq!(bitmap.image.size(0).width.0, (f32::from(layout.size.width) * 2.0).ceil() as i32);
    assert_eq!(bitmap.image.size(0).height.0, (f32::from(layout.size.height) * 2.0).ceil() as i32);
    assert!(layout.make_bitmap(Some(2.0), 1, &|| false).is_none());
}
