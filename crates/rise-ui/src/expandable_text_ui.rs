use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, ElementId, Entity, Font, InteractiveElement, StatefulInteractiveElement, FontFeatures, FontWeight, Hsla, IntoElement, ParentElement,
    Pixels, RenderOnce, SharedString, Styled, TextRun, Window, WindowTextSystem, canvas, div, px,
};
use rise_core::foundation::foundation_character::{Character, characters, int_from_string};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use rise_core::foundation::ns_range::utf16_to_byte_range;
use rise_i18n::tr;
use rise_theme::ThemeStore;

use crate::expandable_text_ui_hooks::{ExpandableTextLinkTokenKind, expandable_text_ui_hooks};
use crate::main_text::{TextAlignT, natural_line_height, system_font};
use crate::swiftui::animated::Animated;
use crate::swiftui::binding::Binding;
use crate::swiftui::environment::{LayoutDirection, env_scope, layout_direction};
use crate::swiftui::frame::{EdgeSet, SwiftUiAlignedFrame, SwiftUiFrame};
use crate::swiftui::open_url::{OpenURLAction, OpenURLActionResult, OpenURLKey};
use crate::swiftui::rich_text::{AttributeContainer, AttributedString, LineStyle, RichFont, RichText};
use crate::swiftui::button_style::plain;
use crate::swiftui::motion::{MotionSpec, spring};
use crate::swiftui::stacks::{StackAlign, vstack};
use crate::swiftui::text::TextAlignment;

const MORE_LABEL_KEY: &str = "profile_bio_more";
const LESS_LABEL_KEY: &str = "profile_bio_less";
const HASHTAG_LINK_SCHEME: &str = "riseonly-expandable-hashtag";
const ELLIPSIS_SEPARATOR: &str = "… ";
const HASHTAG_PREFIX: &str = "#";
const HASHTAG_SEPARATOR: &str = " ";
const HASHTAG_PARAGRAPH_BREAK: &str = "\n\n";
const SUFFIX_EXTRA_WIDTH: f32 = 8.0;
const MINIMUM_TEXT_WIDTH: f32 = 24.0;
const WIDTH_TOLERANCE: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeInsets {
    pub top: f32,
    pub leading: f32,
    pub bottom: f32,
    pub trailing: f32,
}

impl EdgeInsets {
    pub const fn new(top: f32, leading: f32, bottom: f32, trailing: f32) -> Self {
        Self { top, leading, bottom, trailing }
    }
}

type HashtagTap = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// a post hashtags as the last paragraph, one blank line under the text
/// they belong to the text so the more cut hides them like any line
/// with on_tap every tag is a link
#[derive(Clone)]
pub struct ExpandableTextHashtags {
    pub tags: Vec<String>,
    pub color: Hsla,
    pub weight: Option<FontWeight>,
    pub on_tap: Option<HashtagTap>,
}

impl ExpandableTextHashtags {
    pub fn new(tags: Vec<String>, color: Hsla) -> Self {
        Self {
            tags,
            color,
            weight: None,
            on_tap: None,
        }
    }

    pub fn weight(mut self, weight: impl Into<Option<FontWeight>>) -> Self {
        self.weight = weight.into();
        self
    }

    pub fn on_tap(mut self, on_tap: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_tap = Some(Rc::new(on_tap));
        self
    }
}

/// the text and its hashtag paragraph as one string, as layout measures it and text draws it
/// tags_start is the byte offset of the first #
/// mentions and tags inside the text are marked and opened by the post text rules
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExpandableTextInlineLinks {
    pub color: Hsla,
    pub underline: bool,
}

impl ExpandableTextInlineLinks {
    pub fn new(color: Hsla) -> Self {
        Self { color, underline: false }
    }

    pub fn underline(mut self, underline: bool) -> Self {
        self.underline = underline;
        self
    }
}

thread_local! {
    static PARENT_TOGGLE_ANIMATES: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Debug, PartialEq)]
struct Content {
    head: String,
    hashtags: Vec<String>,
    string: String,
    tags_start: usize,
}

impl Content {
    fn new(text: &str, hashtags: &[String]) -> Self {
        let head = if text.is_empty() || hashtags.is_empty() {
            text.to_owned()
        } else {
            format!("{text}{HASHTAG_PARAGRAPH_BREAK}")
        };
        let tags = hashtags
            .iter()
            .map(|tag| format!("{HASHTAG_PREFIX}{tag}"))
            .collect::<Vec<_>>()
            .join(HASHTAG_SEPARATOR);
        let tags_start = head.len();
        Self {
            string: format!("{head}{tags}"),
            head,
            hashtags: hashtags.to_vec(),
            tags_start,
        }
    }

    fn measured(&self, font: &Font, hashtag_font: &Font, color: Hsla) -> Vec<TextRun> {
        let run = |len: usize, font: &Font| TextRun {
            len,
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let mut runs = vec![run(self.tags_start.min(self.string.len()), font)];
        if self.string.len() > self.tags_start {
            runs.push(run(self.string.len() - self.tags_start, hashtag_font));
        }
        runs
    }
}

/// per view state of ExpandableTextUi
struct ExpandableTextState {
    internal_expanded: bool,
    measured_width: f32,
    collapsed_length: Option<usize>,
    content: Option<Content>,
    lined: Option<Content>,
    resolved_font_size: f32,
    height: Animated<f32>,
    natural_height: f32,
    drawn_expanded: Option<bool>,
    animates_next_change: bool,
    awaiting_height: Option<f32>,
}

fn expand_animation() -> MotionSpec {
    spring(0.35, 0.86)
}

/// text clamped to line_limit lines with an inline more control at the cut
/// whole collapsed block is the expand control, state is owned by the caller if is_expanded is passed
/// on_expand_request redirects the tap (eg reel caption opening comments) and stops inline expansion
#[derive(IntoElement)]
pub struct ExpandableTextUi {
    id: ElementId,
    text: String,
    hashtags: Option<ExpandableTextHashtags>,
    inline_links: Option<ExpandableTextInlineLinks>,
    font_size: f32,
    font_weight: FontWeight,
    line_limit: usize,
    text_color: Hsla,
    accent_color: Hsla,
    line_spacing: f32,
    is_expanded: Option<Binding<bool>>,
    on_expand_request: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    collapse_control_insets: EdgeInsets,
    haptics: bool,
    alignment: TextAlignT,
    parent_handles_taps: bool,
}

impl ExpandableTextUi {
    pub fn new(
        id: impl Into<ElementId>,
        text: impl Into<String>,
        text_color: Hsla,
        accent_color: Hsla,
    ) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            hashtags: None,
            inline_links: None,
            font_size: 15.0,
            font_weight: FontWeight::NORMAL,
            line_limit: 4,
            text_color,
            accent_color,
            line_spacing: 0.0,
            is_expanded: None,
            on_expand_request: None,
            collapse_control_insets: EdgeInsets::new(8.0, 0.0, 18.0, 48.0),
            haptics: true,
            alignment: TextAlignT::Left,
            parent_handles_taps: false,
        }
    }

    pub fn hashtags(mut self, hashtags: impl Into<Option<ExpandableTextHashtags>>) -> Self {
        self.hashtags = hashtags.into();
        self
    }

    pub fn inline_links(mut self, inline_links: impl Into<Option<ExpandableTextInlineLinks>>) -> Self {
        self.inline_links = inline_links.into();
        self
    }

    pub fn parent_handles_taps(mut self, parent_handles_taps: bool) -> Self {
        self.parent_handles_taps = parent_handles_taps;
        self
    }

    /// the spring the block controls use, for a caller that toggles with parent_handles_taps
    pub fn toggle_animated(is_expanded: &Binding<bool>, window: &mut Window, cx: &mut App) {
        rise_platform::haptics::selection_changed();
        PARENT_TOGGLE_ANIMATES.with(|flag| flag.set(true));
        let value = is_expanded.get(cx);
        is_expanded.set(!value, window, cx);
    }

    pub fn font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    pub fn font_weight(mut self, font_weight: FontWeight) -> Self {
        self.font_weight = font_weight;
        self
    }

    pub fn line_limit(mut self, line_limit: usize) -> Self {
        self.line_limit = line_limit;
        self
    }

    pub fn line_spacing(mut self, line_spacing: f32) -> Self {
        self.line_spacing = line_spacing;
        self
    }

    pub fn is_expanded(mut self, is_expanded: impl Into<Option<Binding<bool>>>) -> Self {
        self.is_expanded = is_expanded.into();
        self
    }

    pub fn on_expand_request(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_expand_request = Some(Rc::new(f));
        self
    }

    pub fn collapse_control_insets(mut self, insets: EdgeInsets) -> Self {
        self.collapse_control_insets = insets;
        self
    }

    pub fn haptics(mut self, haptics: bool) -> Self {
        self.haptics = haptics;
        self
    }

    pub fn alignment(mut self, alignment: TextAlignT) -> Self {
        self.alignment = alignment;
        self
    }
}

fn set_expanded(
    value: bool,
    binding: Option<&Binding<bool>>,
    state: &Entity<ExpandableTextState>,
    window: &mut Window,
    cx: &mut App,
) {
    match binding {
        Some(binding) => binding.set(value, window, cx),
        None => state.update(cx, |state, cx| {
            state.internal_expanded = value;
            cx.notify();
        }),
    }
}

fn rich_font(size: f32, weight: FontWeight) -> RichFont {
    RichFont::system_weight(size, weight)
}

fn full_string(
    content: &Content,
    hashtags: Option<&ExpandableTextHashtags>,
    inline_links: Option<&ExpandableTextInlineLinks>,
    hashtag_font: RichFont,
) -> AttributedString {
    let mut result = inline_marked(&content.head, true, inline_links, hashtags);
    let Some(hashtags) = hashtags else {
        return result;
    };
    if content.hashtags.is_empty() {
        return result;
    }
    let mut line = AttributedString::default();
    for (index, tag) in content.hashtags.iter().enumerate() {
        if index > 0 {
            line += AttributedString::new(HASHTAG_SEPARATOR);
        }
        let link = if hashtags.on_tap.is_some() {
            Url::from_string(&format!("{HASHTAG_LINK_SCHEME}:{index}"))
        } else {
            None
        };
        line += AttributedString::with_attributes(
            format!("{HASHTAG_PREFIX}{tag}"),
            AttributeContainer {
                link,
                ..AttributeContainer::default()
            },
        );
    }
    line.update_all(|attributes| {
        attributes.font = Some(hashtag_font);
        attributes.foreground_color = Some(hashtags.color);
    });
    result += line;
    result
}

fn collapsed_string(
    content: &Content,
    length: usize,
    hashtags: Option<&ExpandableTextHashtags>,
    inline_links: Option<&ExpandableTextInlineLinks>,
    links_open: bool,
    hashtag_font: RichFont,
    accent_color: Hsla,
) -> AttributedString {
    let source = content.string.as_str();
    let visible = length.min(source.len());
    let head_end = visible.min(content.tags_start);
    let mut result = inline_marked(source.get(..head_end).unwrap_or_default(), links_open, inline_links, hashtags);
    if visible > content.tags_start
        && let Some(hashtags) = hashtags
    {
        result += AttributedString::with_attributes(
            source.get(content.tags_start..visible).unwrap_or_default(),
            AttributeContainer {
                font: Some(hashtag_font),
                foreground_color: Some(hashtags.color),
                ..AttributeContainer::default()
            },
        );
    }
    result += AttributedString::new(ELLIPSIS_SEPARATOR);
    result += AttributedString::with_attributes(
        tr(MORE_LABEL_KEY),
        AttributeContainer {
            foreground_color: Some(accent_color),
            ..AttributeContainer::default()
        },
    );
    result
}

/// head with its mentions and tags marked by inline_links, plain without it
fn inline_marked(
    head: &str,
    tappable: bool,
    inline_links: Option<&ExpandableTextInlineLinks>,
    hashtags: Option<&ExpandableTextHashtags>,
) -> AttributedString {
    let (Some(inline_links), Some(hooks)) = (inline_links, expandable_text_ui_hooks()) else {
        return AttributedString::new(head);
    };
    let mut result = AttributedString::default();
    let mut cursor = 0usize;
    for token in (hooks.caption_link_tokens)(head) {
        let Some(range) = utf16_to_byte_range(head, token.range) else {
            continue;
        };
        if range.start < cursor || range.end > head.len() {
            continue;
        }
        let mut attributes = AttributeContainer::default();
        match token.kind {
            ExpandableTextLinkTokenKind::Mention(url) => {
                attributes.foreground_color = Some(inline_links.color);
                if inline_links.underline {
                    attributes.underline_style = Some(LineStyle::single());
                }
                if tappable {
                    attributes.link = Some(url);
                }
            }
            ExpandableTextLinkTokenKind::Hashtag(tag) => {
                attributes.foreground_color = Some(hashtags.map(|h| h.color).unwrap_or(inline_links.color));
                if tappable && hashtags.is_some_and(|h| h.on_tap.is_some()) {
                    attributes.link = (hooks.hashtag_url)(&tag);
                }
            }
        }
        result += AttributedString::new(head.get(cursor..range.start).unwrap_or_default());
        result += AttributedString::with_attributes(head.get(range.clone()).unwrap_or_default(), attributes);
        cursor = range.end;
    }
    result += AttributedString::new(head.get(cursor..).unwrap_or_default());
    result
}

fn open_link(
    url: &Url,
    tags: &[String],
    on_tap: Option<&HashtagTap>,
    has_inline_links: bool,
    window: &mut Window,
    cx: &mut App,
) -> OpenURLActionResult {
    if url.scheme() == Some(HASHTAG_LINK_SCHEME) {
        return open_hashtag(url, tags, on_tap, window, cx);
    }
    let hooks = expandable_text_ui_hooks();
    if let Some(tag) = hooks.and_then(|hooks| (hooks.hashtag_from_url)(url)) {
        let Some(on_tap) = on_tap else {
            return OpenURLActionResult::Discarded;
        };
        on_tap(&tag, window, cx);
        return OpenURLActionResult::Handled;
    }
    let (true, Some(hooks)) = (has_inline_links, hooks) else {
        return OpenURLActionResult::Discarded;
    };
    (hooks.route_link)(url, window, cx);
    OpenURLActionResult::Handled
}

fn trailing_trimmed_len(text: &str) -> usize {
    text.char_indices()
        .rev()
        .find(|(_, character)| !character.is_whitespace())
        .map(|(index, character)| index + character.len_utf8())
        .unwrap_or(0)
}

fn open_hashtag(url: &Url, tags: &[String], on_tap: Option<&HashtagTap>, window: &mut Window, cx: &mut App) -> OpenURLActionResult {
    if url.scheme() != Some(HASHTAG_LINK_SCHEME) {
        return OpenURLActionResult::Discarded;
    }
    let Some(path) = UrlComponents::from_url(url, false).map(|components| components.path()) else {
        return OpenURLActionResult::Discarded;
    };
    let Some(index) = int_from_string(&path).and_then(|index| usize::try_from(index).ok()) else {
        return OpenURLActionResult::Discarded;
    };
    let (Some(tag), Some(on_tap)) = (tags.get(index), on_tap) else {
        return OpenURLActionResult::Discarded;
    };
    on_tap(tag, window, cx);
    OpenURLActionResult::Handled
}

impl RenderOnce for ExpandableTextUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let text_delta = ThemeStore::shared(cx).read(cx).text_delta();
        let resolved_font_size = (self.font_size + text_delta).max(1.0);
        let hashtag_weight = self
            .hashtags
            .as_ref()
            .and_then(|hashtags| hashtags.weight)
            .unwrap_or(self.font_weight);
        let body_font = system_font(self.font_weight, FontFeatures::default());
        let hashtag_font = system_font(hashtag_weight, FontFeatures::default());
        let content = Content::new(
            &self.text,
            self.hashtags.as_ref().map(|h| h.tags.as_slice()).unwrap_or_default(),
        );

        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| ExpandableTextState {
            internal_expanded: false,
            measured_width: 0.0,
            collapsed_length: None,
            content: None,
            lined: None,
            resolved_font_size,
            height: Animated::new(0.0),
            natural_height: 0.0,
            drawn_expanded: None,
            animates_next_change: false,
            awaiting_height: None,
        });

        let (content_changed, font_changed, measured_width) = {
            let current = state.read(cx);
            (
                current.content.as_ref().is_some_and(|previous| *previous != content),
                current.resolved_font_size != resolved_font_size,
                current.measured_width,
            )
        };
        if content_changed || font_changed || state.read(cx).content.is_none() {
            let lined = (measured_width > 0.0)
                .then(|| {
                    ExpandableTextLayout::right_to_left_lined(
                        window.text_system(),
                        &content,
                        &body_font,
                        self.text_color,
                        px(resolved_font_size),
                        measured_width,
                    )
                })
                .flatten();
            let collapsed_length = (measured_width > 0.0)
                .then(|| {
                    ExpandableTextLayout::collapsed_length(
                        window.text_system(),
                        lined.as_ref().unwrap_or(&content),
                        &body_font,
                        &hashtag_font,
                        self.text_color,
                        px(resolved_font_size),
                        measured_width,
                        self.line_limit,
                        &tr(MORE_LABEL_KEY),
                        lined.is_some(),
                    )
                })
                .flatten();
            let fits = measured_width > 0.0 && collapsed_length.is_none();
            state.update(cx, |state, _| {
                state.content = Some(content.clone());
                state.resolved_font_size = resolved_font_size;
                if measured_width > 0.0 {
                    state.collapsed_length = collapsed_length;
                }
                state.lined = lined;
                if content_changed || fits {
                    state.internal_expanded = false;
                }
            });
            if let Some(binding) = self.is_expanded.clone()
                && (content_changed || fits)
            {
                window.defer(cx, move |window, cx| binding.set(false, window, cx));
            }
        }

        let (internal_expanded, collapsed_length, lined) = {
            let current = state.read(cx);
            (current.internal_expanded, current.collapsed_length, current.lined.clone())
        };
        let paragraph_is_right_to_left = is_right_to_left_text(&content.head);
        let shown = lined.filter(|lined| lined.string.len() == content.string.len()).unwrap_or_else(|| content.clone());
        let gpui_ellipsis = shown.string == content.string;
        let expanded = match self.is_expanded.as_ref() {
            Some(binding) => binding.get(cx),
            None => internal_expanded,
        };

        let direction = layout_direction(cx);
        let text_align = if paragraph_is_right_to_left && self.alignment.text_alignment() == TextAlignment::Leading {
            TextAlignment::Leading.resolve(LayoutDirection::RightToLeft)
        } else {
            self.alignment.text_alignment().resolve(direction)
        };
        let frame_alignment = self.alignment.frame_alignment();
        let natural = natural_line_height(&body_font, resolved_font_size, cx);
        let line_height = px(natural + self.line_spacing.max(0.0));
        let hashtag_rich_font = rich_font(resolved_font_size, hashtag_weight);
        let styled_text = |text: AttributedString, id: ElementId| {
            div()
                .font(body_font.clone())
                .text_size(px(resolved_font_size))
                .line_height(line_height)
                .text_color(self.text_color)
                .text_align(text_align)
                .child(RichText::new(id, text).tint(self.hashtags.as_ref().map(|h| h.color)))
        };

        let full_text = |limit: Option<usize>| {
            let tags = content.hashtags.clone();
            let on_tap = self.hashtags.as_ref().and_then(|h| h.on_tap.clone());
            let has_inline_links = self.inline_links.is_some();
            let action = OpenURLAction::new(move |url, window, cx| {
                open_link(url, &tags, on_tap.as_ref(), has_inline_links, window, cx)
            });
            let mut text = styled_text(
                full_string(&shown, self.hashtags.as_ref(), self.inline_links.as_ref(), hashtag_rich_font),
                ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "full".into()),
            );
            if let Some(limit) = limit {
                text = text.line_clamp(limit);
                if gpui_ellipsis {
                    text = text.text_ellipsis();
                }
            }
            text = text.w_full().min_w_0();
            env_scope::<OpenURLKey>(action).child(text.frame_max_width(frame_alignment))
        };

        let measure = {
            let state = state.clone();
            let content = content.clone();
            let body_font = body_font.clone();
            let hashtag_font = hashtag_font.clone();
            let text_color = self.text_color;
            let line_limit = self.line_limit;
            let binding = self.is_expanded.clone();
            canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    let width = f32::from(bounds.size.width);
                    let previous = state.read(cx).measured_width;
                    if (previous - width).abs() <= WIDTH_TOLERANCE {
                        return;
                    }
                    let lined = ExpandableTextLayout::right_to_left_lined(
                        window.text_system(),
                        &content,
                        &body_font,
                        text_color,
                        px(resolved_font_size),
                        width,
                    );
                    let collapsed_length = ExpandableTextLayout::collapsed_length(
                        window.text_system(),
                        lined.as_ref().unwrap_or(&content),
                        &body_font,
                        &hashtag_font,
                        text_color,
                        px(resolved_font_size),
                        width,
                        line_limit,
                        &tr(MORE_LABEL_KEY),
                        lined.is_some(),
                    );
                    let previous_length = state.read(cx).collapsed_length;
                    let lines_moved = state.read(cx).lined != lined;
                    state.update(cx, |state, _| {
                        state.measured_width = width;
                        state.collapsed_length = collapsed_length;
                        state.lined = lined;
                        if collapsed_length.is_none() {
                            state.internal_expanded = false;
                        }
                    });
                    if collapsed_length.is_none()
                        && let Some(binding) = binding.clone()
                    {
                        window.defer(cx, move |window, cx| binding.set(false, window, cx));
                    }
                    if previous == 0.0 || previous_length != collapsed_length || lines_moved {
                        window.request_animation_frame();
                    }
                },
            )
            .absolute()
            .inset_0()
        };

        let body = if expanded && (collapsed_length.is_some() || measured_width == 0.0) {
            let binding = self.is_expanded.clone();
            let collapse_state = state.clone();
            let haptics = self.haptics;
            let insets = self.collapse_control_insets;
            let label = div()
                .font(body_font.clone())
                .text_size(px(resolved_font_size))
                .text_color(self.accent_color)
                .text_align(self.alignment.text_alignment().resolve(direction))
                .child(tr(LESS_LABEL_KEY))
                .padding_edges(EdgeSet::TOP, insets.top)
                .padding_edges(EdgeSet::LEADING, insets.leading)
                .padding_edges(EdgeSet::BOTTOM, insets.bottom)
                .padding_edges(EdgeSet::TRAILING, insets.trailing);
            let collapse = if self.parent_handles_taps {
                label.into_any_element()
            } else {
                plain(ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "less".into()))
                    .on_press(move |window, cx| {
                        if haptics {
                            rise_platform::haptics::selection_changed();
                        }
                        collapse_state.update(cx, |state, _| state.animates_next_change = true);
                        set_expanded(false, binding.as_ref(), &collapse_state, window, cx);
                    })
                    .child(label)
                    .into_any_element()
            };
            vstack(0.0)
                .align(frame_alignment.horizontal())
                .w_full()
                .child(full_text(None))
                .child(collapse)
                .into_any_element()
        } else if let Some(length) = collapsed_length {
            let binding = self.is_expanded.clone();
            let expand_state = state.clone();
            let haptics = self.haptics;
            let on_expand_request = self.on_expand_request.clone();
            let links_open = self.inline_links.is_some() && length >= trailing_trimmed_len(&content.head);
            let last_line_break = (!gpui_ellipsis && length <= content.tags_start)
                .then(|| shown.string.get(..length).and_then(|visible| visible.rfind('\n')))
                .flatten();
            let label = if let Some(last_line_break) = last_line_break {
                let upper = inline_marked(
                    shown.string.get(..last_line_break).unwrap_or_default(),
                    links_open,
                    self.inline_links.as_ref(),
                    self.hashtags.as_ref(),
                );
                let mut lower = inline_marked(
                    shown.string.get(last_line_break + 1..length).unwrap_or_default(),
                    links_open,
                    self.inline_links.as_ref(),
                    self.hashtags.as_ref(),
                );
                lower += AttributedString::new(ELLIPSIS_SEPARATOR);
                let upper = styled_text(
                    upper,
                    ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "collapsed".into()),
                )
                .w_full()
                .line_clamp(self.line_limit.saturating_sub(1).max(1));
                let lower = styled_text(
                    lower,
                    ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "collapsed-last".into()),
                )
                .min_w_0()
                .whitespace_nowrap()
                .overflow_hidden();
                let more = div()
                    .flex_shrink_0()
                    .font(body_font.clone())
                    .text_size(px(resolved_font_size))
                    .line_height(line_height)
                    .text_color(self.accent_color)
                    .child(tr(MORE_LABEL_KEY));
                let last_row = div().w_full().flex().flex_row().justify_end().child(more).child(lower);
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .child(upper)
                    .child(last_row)
                    .frame_max_width(frame_alignment)
                    .into_any_element()
            } else {
                let collapsed = collapsed_string(
                    &shown,
                    length,
                    self.hashtags.as_ref(),
                    self.inline_links.as_ref(),
                    links_open,
                    hashtag_rich_font,
                    self.accent_color,
                );
                let label = styled_text(
                    collapsed,
                    ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "collapsed".into()),
                );
                let mut label = if self.line_limit == 1 {
                    label.whitespace_nowrap().overflow_hidden()
                } else {
                    label.line_clamp(self.line_limit)
                };
                if gpui_ellipsis {
                    label = label.text_ellipsis();
                }
                label.frame_max_width(frame_alignment).into_any_element()
            };
            let expand_from_collapsed = move |window: &mut Window, cx: &mut App| {
                if haptics {
                    rise_platform::haptics::selection_changed();
                }
                if let Some(on_expand_request) = on_expand_request.as_ref() {
                    on_expand_request(window, cx);
                    return;
                }
                expand_state.update(cx, |state, _| state.animates_next_change = true);
                set_expanded(true, binding.as_ref(), &expand_state, window, cx);
            };
            if self.parent_handles_taps {
                label.into_any_element()
            } else if links_open {
                let tags = content.hashtags.clone();
                let on_tap = self.hashtags.as_ref().and_then(|h| h.on_tap.clone());
                let action = OpenURLAction::new(move |url, window, cx| {
                    open_link(url, &tags, on_tap.as_ref(), true, window, cx)
                });
                div()
                    .id(ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "more".into()))
                    .w_full()
                    .role(gpui::accesskit::Role::Button)
                    .on_click(move |_, window, cx| expand_from_collapsed(window, cx))
                    .child(env_scope::<OpenURLKey>(action).child(label))
                    .into_any_element()
            } else {
                plain(ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "more".into()))
                    .w_full()
                    .on_press(expand_from_collapsed)
                    .child(label)
                    .into_any_element()
            }
        } else {
            full_text(Some(self.line_limit)).into_any_element()
        };

        let now = RiseClock::monotonic();
        let parent_handles_taps = self.parent_handles_taps;
        let clip_height = state.update(cx, |state, _| {
            if state.drawn_expanded != Some(expanded) {
                let parent_toggle = parent_handles_taps && PARENT_TOGGLE_ANIMATES.with(|flag| flag.replace(false));
                if state.drawn_expanded.is_some() && (state.animates_next_change || parent_toggle) {
                    let on_screen = if state.height.is_running(now) {
                        state.height.value(now)
                    } else {
                        state.natural_height
                    };
                    state.awaiting_height = Some(on_screen);
                }
                state.drawn_expanded = Some(expanded);
                state.animates_next_change = false;
            }
            state.awaiting_height
        });
        let clip_height = match clip_height {
            Some(held) => Some(held),
            None => {
                let height = &state.read(cx).height;
                height.is_running(now).then(|| height.frame_value(now, window))
            }
        };
        let body_height = {
            let state = state.clone();
            canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    let height = f32::from(bounds.size.height);
                    let now = RiseClock::monotonic();
                    state.update(cx, |state, cx| {
                        if let Some(from) = state.awaiting_height.take() {
                            state.natural_height = height;
                            state.height.jump(from);
                            state.height.set_in(height, expand_animation(), now, cx);
                            window.request_animation_frame();
                        } else if !state.height.is_running(now) {
                            state.natural_height = height;
                            state.height.jump(height);
                        }
                    });
                },
            )
            .absolute()
            .inset_0()
        };
        let body = div()
            .w_full()
            .flex_shrink_0()
            .relative()
            .child(body_height)
            .child(body);
        let body = match clip_height {
            Some(height) => div()
                .flex()
                .flex_col()
                .w_full()
                .h(px(height.max(0.0)))
                .overflow_hidden()
                .child(body),
            None => div().flex().flex_col().w_full().child(body),
        };

        vstack(0.0)
            .align(self.alignment.frame_alignment().horizontal())
            .w_full()
            .relative()
            .child(measure)
            .child(body)
    }
}

/// one wrapped line of the measured text, in byte offsets of the whole string
struct VisualLine {
    start: usize,
    end: usize,
    paragraph_start: usize,
    paragraph: usize,
}

pub struct ExpandableTextLayout;

impl ExpandableTextLayout {
    /// mirrors the visible layout (same fonts, word wrap)
    /// tells whether text exceeds line_limit and how many bytes to keep so the more fits on the last line
    /// runs once per text, width or font change and none means it fits
    #[allow(clippy::too_many_arguments)]
    fn collapsed_length(
        text_system: &WindowTextSystem,
        content: &Content,
        font: &Font,
        hashtag_font: &Font,
        color: Hsla,
        font_size: Pixels,
        width: f32,
        line_limit: usize,
        more_label: &str,
        hand_lined: bool,
    ) -> Option<usize> {
        if line_limit == 0 || !width.is_finite() {
            return None;
        }
        let suffix: SharedString = format!("{ELLIPSIS_SEPARATOR}{more_label}").into();
        let suffix_run = TextRun {
            len: suffix.len(),
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let suffix_width =
            f32::from(text_system.shape_line(suffix.clone(), font_size, &[suffix_run.clone()], None).width())
                + SUFFIX_EXTRA_WIDTH;
        if width <= suffix_width + MINIMUM_TEXT_WIDTH {
            return None;
        }

        let text = content.string.as_str();
        if hand_lined {
            let cut = Self::hand_lined_cut(
                text_system,
                content,
                font,
                &suffix,
                &suffix_run,
                font_size,
                width - SUFFIX_EXTRA_WIDTH - WIDTH_TOLERANCE,
                line_limit,
            );
            tracing::warn!(
                target: "riseonly::X303",
                "hand lined cut: width {width}, lines {}, limit {line_limit}, cut {cut:?}, head {}, len {}",
                text.get(..content.tags_start).map_or(0, |head| head.split('\n').count()),
                content.tags_start,
                text.len()
            );
            if let Some(cut) = cut {
                return Self::trimmed_cut(text, cut);
            }
            if content.tags_start >= text.len() {
                return None;
            }
        }
        let runs = content.measured(font, hashtag_font, color);
        let paragraphs = text_system
            .shape_text(text.to_owned().into(), font_size, &runs, Some(px(width)), None)
            .ok()?;

        let mut lines: Vec<VisualLine> = Vec::new();
        let mut paragraph_start = 0;
        for (paragraph_index, paragraph) in paragraphs.iter().enumerate() {
            let layout = &paragraph.unwrapped_layout;
            let mut segment_start = 0;
            for boundary in paragraph.wrap_boundaries() {
                let Some(glyph) = layout
                    .runs
                    .get(boundary.run_ix)
                    .and_then(|run| run.glyphs.get(boundary.glyph_ix))
                else {
                    continue;
                };
                lines.push(VisualLine {
                    start: paragraph_start + segment_start,
                    end: paragraph_start + glyph.index,
                    paragraph_start,
                    paragraph: paragraph_index,
                });
                segment_start = glyph.index;
            }
            lines.push(VisualLine {
                start: paragraph_start + segment_start,
                end: paragraph_start + paragraph.len(),
                paragraph_start,
                paragraph: paragraph_index,
            });
            paragraph_start += paragraph.len() + 1;
            if lines.len() > line_limit {
                break;
            }
        }
        if lines.len() <= line_limit {
            return None;
        }
        let cut = if has_reordered_glyphs(&paragraphs) {
            Self::prefix_cut(
                text_system,
                text,
                &runs,
                &suffix,
                &suffix_run,
                font_size,
                width - SUFFIX_EXTRA_WIDTH,
                line_limit,
            )?
        } else {
            let last = &lines[line_limit - 1];
            if last.end <= last.start {
                return Self::trimmed_cut(text, last.start);
            }
            let layout = &paragraphs[last.paragraph].unwrapped_layout;
            let x_at = |byte: usize| f32::from(layout.x_for_index(byte - last.paragraph_start));
            let line_x = x_at(last.start);

            let mut cut = last.end;
            let available = width - suffix_width;
            if x_at(last.end) - line_x > available {
                let boundaries: Vec<usize> = text
                    .get(last.start..last.end)
                    .unwrap_or_default()
                    .char_indices()
                    .map(|(offset, _)| last.start + offset)
                    .chain(std::iter::once(last.end))
                    .collect();
                let (mut lo, mut hi) = (0, boundaries.len() - 1);
                while lo < hi {
                    let mid = (lo + hi).div_ceil(2);
                    if x_at(boundaries[mid]) - line_x <= available {
                        lo = mid;
                    } else {
                        hi = mid - 1;
                    }
                }
                cut = boundaries[lo];
            }
            cut
        };

        Self::trimmed_cut(text, cut)
    }

    /// cut moved off a split grapheme then before trailing spaces and newlines
    fn trimmed_cut(text: &str, cut: usize) -> Option<usize> {
        let mut cut = cut.min(text.len());
        if cut > 0 && cut < text.len()
            && let Some((start, end)) = composed_character_range(text, cut - 1)
            && end > cut
        {
            cut = start;
        }
        while cut > 0 {
            let Some((start, end)) = composed_character_range(text, cut - 1) else {
                break;
            };
            let blank = text
                .get(start..end)
                .and_then(Character::new)
                .is_some_and(|character| character.is_whitespace() || character.is_newline());
            if !blank {
                break;
            }
            cut = start;
        }
        (cut > 0).then_some(cut)
    }
}

impl ExpandableTextLayout {
    #[allow(clippy::too_many_arguments)]
    fn hand_lined_cut(
        text_system: &WindowTextSystem,
        content: &Content,
        font: &Font,
        suffix: &SharedString,
        suffix_run: &TextRun,
        font_size: Pixels,
        available: f32,
        line_limit: usize,
    ) -> Option<usize> {
        let head = content.string.get(..content.tags_start)?;
        let mut lines: Vec<(usize, usize)> = Vec::new();
        let mut start = 0;
        for line in head.split('\n') {
            lines.push((start, start + line.len()));
            start += line.len() + 1;
        }
        if lines.len() <= line_limit {
            return None;
        }
        let (line_start, line_end) = lines[line_limit - 1];
        let shaped_width = |piece: &str| -> f32 {
            if piece.is_empty() {
                return 0.0;
            }
            let run = TextRun {
                len: piece.len(),
                font: font.clone(),
                color: suffix_run.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            f32::from(text_system.shape_line(piece.to_owned().into(), font_size, &[run], None).width())
        };
        let suffix_width = f32::from(text_system.shape_line(suffix.clone(), font_size, std::slice::from_ref(suffix_run), None).width());
        let fits = |cut: usize| -> bool {
            head.get(line_start..cut)
                .is_some_and(|piece| shaped_width(piece) + suffix_width <= available)
        };
        if fits(line_end) {
            return Some(line_end);
        }
        let boundaries: Vec<usize> = head
            .get(line_start..line_end)
            .unwrap_or_default()
            .char_indices()
            .map(|(offset, _)| line_start + offset)
            .skip(1)
            .collect();
        let Some(&first) = boundaries.first() else {
            return Some(line_end);
        };
        let (mut lo, mut hi) = (0, boundaries.len() - 1);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if fits(boundaries[mid]) {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        Some(boundaries.get(lo).copied().unwrap_or(first))
    }
}

impl ExpandableTextLayout {
    #[allow(clippy::too_many_arguments)]
    fn prefix_cut(
        text_system: &WindowTextSystem,
        text: &str,
        runs: &[TextRun],
        suffix: &SharedString,
        suffix_run: &TextRun,
        font_size: Pixels,
        width: f32,
        line_limit: usize,
    ) -> Option<usize> {
        let fits = |cut: usize| -> bool {
            let Some(prefix) = text.get(..cut) else {
                return false;
            };
            let mut probe_runs = prefix_runs(runs, cut);
            probe_runs.push(suffix_run.clone());
            let probe: SharedString = format!("{prefix}{suffix}").into();
            text_system
                .shape_text(probe, font_size, &probe_runs, Some(px(width)), None)
                .map(|paragraphs| wrapped_line_count(&paragraphs) <= line_limit)
                .unwrap_or(false)
        };
        let boundaries: Vec<usize> = text.char_indices().map(|(offset, _)| offset).skip(1).collect();
        if boundaries.is_empty() || !fits(boundaries[0]) {
            return None;
        }
        let (mut lo, mut hi) = (0, boundaries.len() - 1);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if fits(boundaries[mid]) {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        Some(boundaries[lo])
    }
}

impl ExpandableTextLayout {
    /// content with a newline in place of the space before every word that starts a new line
    /// only right to left text is broken here, hashtag paragraph is left to gpui
    /// none when nothing is right to left
    fn right_to_left_lined(
        text_system: &WindowTextSystem,
        content: &Content,
        font: &Font,
        color: Hsla,
        font_size: Pixels,
        width: f32,
    ) -> Option<Content> {
        if !is_right_to_left_text(&content.head) {
            return None;
        }
        let head = content.string.get(..content.tags_start)?;
        let breaks = right_to_left_line_breaks(text_system, head, font, color, font_size, width);
        if breaks.is_empty() {
            return None;
        }
        let mut bytes = content.string.clone().into_bytes();
        for space in breaks {
            if let Some(byte) = bytes.get_mut(space) {
                *byte = b'\n';
            }
        }
        let string = String::from_utf8(bytes).ok()?;
        Some(Content {
            head: string.get(..content.tags_start)?.to_owned(),
            hashtags: content.hashtags.clone(),
            string,
            tags_start: content.tags_start,
        })
    }
}

/// byte offsets of the spaces where a right to left head starts a new line within width
/// gpui wraps rtl runs in visual order so lines come out bottom up with words cut at the edges
pub(crate) fn right_to_left_line_breaks(
    text_system: &WindowTextSystem,
    head: &str,
    font: &Font,
    color: Hsla,
    font_size: Pixels,
    width: f32,
) -> Vec<usize> {
    if !width.is_finite() || width <= MINIMUM_TEXT_WIDTH {
        return Vec::new();
    }
    let available = width - WIDTH_TOLERANCE;
    let line_width = |start: usize, end: usize| -> f32 {
        let Some(piece) = head.get(start..end) else {
            return 0.0;
        };
        if piece.is_empty() {
            return 0.0;
        }
        let run = TextRun {
            len: piece.len(),
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        f32::from(text_system.shape_line(piece.to_owned().into(), font_size, &[run], None).width())
    };
    let mut breaks: Vec<usize> = Vec::new();
    let mut paragraph_start = 0;
    for paragraph in head.split('\n') {
        let paragraph_end = paragraph_start + paragraph.len();
        let candidates: Vec<usize> = paragraph
            .match_indices(' ')
            .map(|(offset, _)| paragraph_start + offset)
            .chain(std::iter::once(paragraph_end))
            .collect();
        let mut line_start = paragraph_start;
        let mut last_fit: Option<usize> = None;
        for candidate in candidates {
            if line_width(line_start, candidate) <= available {
                last_fit = (candidate < paragraph_end).then_some(candidate);
                continue;
            }
            let Some(space) = last_fit.take() else {
                if candidate < paragraph_end {
                    breaks.push(candidate);
                    line_start = candidate + 1;
                }
                continue;
            };
            breaks.push(space);
            line_start = space + 1;
            if line_width(line_start, candidate) <= available {
                last_fit = (candidate < paragraph_end).then_some(candidate);
            }
        }
        paragraph_start = paragraph_end + 1;
    }
    breaks
}

/// text with a newline in place of every space that starts a new line within width
/// when its paragraph is right to left
/// none for left to right text or text that fits one line
pub(crate) fn right_to_left_hand_lined(
    text_system: &WindowTextSystem,
    text: &str,
    font: &Font,
    color: Hsla,
    font_size: Pixels,
    width: f32,
) -> Option<SharedString> {
    if !is_right_to_left_text(text) {
        return None;
    }
    let breaks = right_to_left_line_breaks(text_system, text, font, color, font_size, width);
    if breaks.is_empty() {
        return None;
    }
    let mut bytes = text.as_bytes().to_vec();
    for space in breaks {
        if let Some(byte) = bytes.get_mut(space) {
            *byte = b'\n';
        }
    }
    String::from_utf8(bytes).ok().map(SharedString::from)
}

/// true when the first strong letter is right to left (hebrew, arabic and neighbours)
pub(crate) fn is_right_to_left_text(text: &str) -> bool {
    first_strong_is_right_to_left(text).unwrap_or(false)
}

/// direction of the first strong letter, None when there is no letter
pub(crate) fn first_strong_is_right_to_left(text: &str) -> Option<bool> {
    text.chars().find_map(|character| {
        if is_right_to_left_scalar(character) {
            Some(true)
        } else if character.is_alphabetic() {
            Some(false)
        } else {
            None
        }
    })
}

fn is_right_to_left_scalar(character: char) -> bool {
    matches!(
        u32::from(character),
        0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF
    )
}

fn has_reordered_glyphs(paragraphs: &[gpui::WrappedLine]) -> bool {
    paragraphs.iter().any(|paragraph| {
        let mut previous = 0usize;
        paragraph
            .unwrapped_layout
            .runs
            .iter()
            .flat_map(|run| run.glyphs.iter())
            .any(|glyph| {
                let reordered = glyph.index < previous;
                previous = glyph.index;
                reordered
            })
    })
}

fn wrapped_line_count(paragraphs: &[gpui::WrappedLine]) -> usize {
    paragraphs.iter().map(|paragraph| paragraph.wrap_boundaries().len() + 1).sum()
}

fn prefix_runs(runs: &[TextRun], len: usize) -> Vec<TextRun> {
    let mut remaining = len;
    let mut out = Vec::new();
    for run in runs {
        if remaining == 0 {
            break;
        }
        let mut run = run.clone();
        run.len = run.len.min(remaining);
        remaining -= run.len;
        out.push(run);
    }
    out
}

/// extended grapheme cluster holding the byte at index, in bytes
fn composed_character_range(text: &str, index: usize) -> Option<(usize, usize)> {
    let mut start = 0;
    for character in characters(text) {
        let end = start + character.as_str().len();
        if index < end {
            return Some((start, end));
        }
        start = end;
    }
    None
}

#[cfg(test)]
#[path = "expandable_text_ui_tests.rs"]
mod tests;
