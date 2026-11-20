use std::any::TypeId;
use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, CursorStyle, DispatchPhase, Element, ElementId, FocusHandle, GlobalElementId, Hitbox,
    HitboxBehavior, InspectorElementId, IntoElement, KeyContext, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Point, RenderOnce, SharedString, Size, Style, Styled, StyledText, TextLayout, Window,
    div, fill, point, px, size,
};
use rise_core::foundation::foundation_character::characters;
use rise_core::foundation::ns_range::{NSRange, byte_to_utf16_range, length_utf16, utf16_to_byte_range};
use rise_platform::pasteboard::UIPasteboard;
use rise_ui::input_ui::input_actions;
use rise_ui::rich_text::document::rise_document_projection::RiseDocumentProjection;
use rise_ui::rich_text::rich_text_markup::RichTextMarkup;
use rise_ui::swiftui::font::{Font, NATURAL_LINE_HEIGHT, SwiftUiFont};
use rise_ui::swiftui::rich_text::{AttributeContainer, AttributedString, RichFont, RichText};
use rise_ui::swiftui::text_selection::{
    TextDragAxis, TextSelection, mark_selectable_text_press, mark_text_glyph_press, text_drag_axis, text_press_swiped,
};

use crate::core::ui::custom_emoji_glyph::CustomEmojiGlyphMetrics;
use crate::core::ui::formatted_text_ui::{FormattedText, FormattedTextOptions, FormattedTextSelection, FormattedTextStyle};
use crate::modules::chat::components::chat::bubble::chat_mention_tap_layer::{
    ChatMentionTap, ChatMentionTapLayerModifiers, ChatMentionTapsOptions,
};
use crate::modules::chat::components::chat::bubble::inline_custom_emoji_run::InlineCustomEmojiRun;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::shared::utils::message_preview::MessagePreview;
use crate::modules::chat::stores::message::message_actions::message_types::MessageCustomEmojiEntity;

#[derive(IntoElement)]
pub struct InlineCustomEmojiText {
    id: ElementId,
    pub text: String,
    pub style: FormattedTextStyle,
    pub entities: Vec<MessageCustomEmojiEntity>,
    pub options: FormattedTextOptions,
    pub trailing_meta: Option<AnyElement>,
    pub on_mention_tap: Option<ChatMentionTap>,
    pub on_link_tap: Option<ChatMentionTap>,
    pub opens_pack_on_tap: bool,
    pub text_selection: Option<FormattedTextSelection>,
}

/// run kinds of the inline emoji text, each has its range in the displayed text
/// selection crosses words and glyphs
#[derive(Clone, Debug, PartialEq)]
pub enum InlineCustomEmojiTextRun {
    Text(AttributedString, NSRange),
    Emoji(MessageCustomEmojiEntity, NSRange),
    OrphanPlaceholder(NSRange),
    LineBreak(NSRange),
}

impl InlineCustomEmojiText {
    pub fn new(
        id: impl Into<ElementId>,
        text: impl Into<String>,
        style: FormattedTextStyle,
        entities: Vec<MessageCustomEmojiEntity>,
    ) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            style,
            entities,
            options: FormattedTextOptions::default(),
            trailing_meta: None,
            on_mention_tap: None,
            on_link_tap: None,
            opens_pack_on_tap: true,
            text_selection: None,
        }
    }

    pub fn options(mut self, options: FormattedTextOptions) -> Self {
        self.options = options;
        self
    }

    pub fn trailing_meta(mut self, trailing_meta: Option<AnyElement>) -> Self {
        self.trailing_meta = trailing_meta;
        self
    }

    pub fn opens_pack_on_tap(mut self, opens_pack_on_tap: bool) -> Self {
        self.opens_pack_on_tap = opens_pack_on_tap;
        self
    }

    pub fn on_mention_tap(mut self, on_mention_tap: Option<ChatMentionTap>) -> Self {
        self.on_mention_tap = on_mention_tap;
        self
    }

    pub fn on_link_tap(mut self, on_link_tap: Option<ChatMentionTap>) -> Self {
        self.on_link_tap = on_link_tap;
        self
    }

    pub fn text_selection(mut self, text_selection: Option<FormattedTextSelection>) -> Self {
        self.text_selection = text_selection;
        self
    }

    fn indexed_id(&self, name: &'static str, index: usize) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::from(format!("{name}-{index}")))
    }

    fn text_run(&self, index: usize, chunk: AttributedString, range: NSRange) -> (AnyElement, Option<TextLayout>) {
        let id = self.indexed_id("text", index);
        let (body, layout) = if self.text_selection.is_some() {
            let characters: SharedString = chunk.characters().to_owned().into();
            let highlights = chunk.highlights(None);
            let overrides = RichText::new(id.clone(), chunk).font_family_overrides();
            let mut styled = StyledText::new(characters).with_highlights(highlights);
            if !overrides.is_empty() {
                styled = styled.with_font_family_overrides(overrides);
            }
            let layout = styled.layout().clone();
            (styled.into_any_element(), Some(layout))
        } else {
            (RichText::new(id, chunk).into_any_element(), None)
        };
        let content = div()
            .whitespace_nowrap()
            .swiftui_font(Font::system_size(self.style.font_size))
            .line_height(px(self.style.font_size * NATURAL_LINE_HEIGHT))
            .text_color(self.style.base_color)
            .child(body);
        let element = match self.on_mention_tap.clone() {
            Some(on_mention_tap) => content
                .chat_mention_taps(
                    self.indexed_id("taps", index),
                    self.text.clone(),
                    self.style.clone(),
                    ChatMentionTapsOptions {
                        options: self.options.clone(),
                        display_range: Some(range),
                        on_link: self.on_link_tap.clone(),
                        ..ChatMentionTapsOptions::default()
                    },
                    on_mention_tap,
                )
                .into_any_element(),
            None => content.into_any_element(),
        };
        (element, layout)
    }

    fn flow_child(&self, index: usize, run: InlineCustomEmojiTextRun, display: &str) -> InlineRunFlowChild {
        match run {
            InlineCustomEmojiTextRun::Text(chunk, range) => {
                let (element, layout) = self.text_run(index, chunk, range);
                InlineRunFlowChild::new(element, InlineRunRole::Run, display, range, layout)
            }
            InlineCustomEmojiTextRun::Emoji(entity, range) => {
                let element = InlineCustomEmojiRun::new(
                    self.indexed_id("emoji", index),
                    entity.custom_emoji_id,
                    entity.file_url,
                    entity.asset_type,
                )
                .size(CustomEmojiGlyphMetrics::ink_height(self.style.font_size))
                .canvas_width(entity.width)
                .canvas_height(entity.height)
                .maximum_aspect(CustomEmojiGlyphMetrics::TEXT_MAXIMUM_ASPECT)
                .pack_id(entity.pack_id)
                .opens_pack_on_tap(self.opens_pack_on_tap)
                .into_any_element();
                InlineRunFlowChild::new(element, InlineRunRole::Run, display, range, None)
            }
            InlineCustomEmojiTextRun::OrphanPlaceholder(range) => {
                let element = div()
                    .whitespace_nowrap()
                    .swiftui_font(Font::system_size(self.style.font_size))
                    .line_height(px(self.style.font_size * NATURAL_LINE_HEIGHT))
                    .child(SharedString::new_static(MessagePreview::CUSTOM_EMOJI_PREVIEW_GLYPH))
                    .into_any_element();
                InlineRunFlowChild::new(element, InlineRunRole::Run, display, range, None)
            }
            InlineCustomEmojiTextRun::LineBreak(range) => {
                let element = div().w(px(0.0)).h(px(self.style.font_size * NATURAL_LINE_HEIGHT)).into_any_element();
                InlineRunFlowChild::new(element, InlineRunRole::LineBreak, display, range, None)
            }
        }
    }

    pub fn build_runs(
        text: &str,
        style: &FormattedTextStyle,
        options: &FormattedTextOptions,
        entities: &[MessageCustomEmojiEntity],
    ) -> Vec<InlineCustomEmojiTextRun> {
        let formatted = FormattedText::format(text, style, options, &[]);
        let display = formatted.display_string.as_str();
        let removed = if options.no_markdown { Vec::new() } else { RichTextMarkup::parse_inline(text).removed_ranges };
        let raw_total = length_utf16(text) as i64;
        let total = length_utf16(display);
        let mut placed: Vec<(NSRange, &MessageCustomEmojiEntity)> = entities
            .iter()
            .filter_map(|entity| {
                if entity.offset < 0
                    || entity.length <= 0
                    || entity.offset > raw_total
                    || entity.length > raw_total - entity.offset
                {
                    return None;
                }
                let raw = NSRange { location: entity.offset as usize, length: entity.length as usize };
                let range = RichTextMarkup::display_range(raw, &removed)?;
                (range.location + range.length <= total).then_some((range, entity))
            })
            .collect();
        placed.sort_by_key(|(range, _)| range.location);

        let mut runs = Vec::new();
        let mut cursor = 0;
        for (range, entity) in placed {
            if range.location < cursor {
                continue;
            }
            if range.location > cursor {
                Self::append_text_chunks(
                    NSRange { location: cursor, length: range.location - cursor },
                    &formatted.attributed_body,
                    display,
                    style,
                    &mut runs,
                );
            }
            runs.push(InlineCustomEmojiTextRun::Emoji(entity.clone(), range));
            cursor = range.location + range.length;
        }
        if cursor < total {
            Self::append_text_chunks(
                NSRange { location: cursor, length: total - cursor },
                &formatted.attributed_body,
                display,
                style,
                &mut runs,
            );
        }
        runs
    }

    /// newline is its own break not part of a word
    /// a text ending in one draws an extra empty line and never starts the next flow line
    fn append_text_chunks(
        range: NSRange,
        body: &AttributedString,
        display: &str,
        style: &FormattedTextStyle,
        runs: &mut Vec<InlineCustomEmojiTextRun>,
    ) {
        if range.length == 0 {
            return;
        }
        let Some(segment) = utf16_to_byte_range(display, range) else {
            return;
        };
        let mut chunk_start = segment.start;
        let mut index = segment.start;
        for character in characters(&display[segment.clone()]) {
            let next = index + character.as_str().len();
            if character.is_newline() {
                Self::append_text(chunk_start..index, body, display, style, runs);
                runs.push(InlineCustomEmojiTextRun::LineBreak(byte_to_utf16_range(display, index..next)));
                chunk_start = next;
            } else if character.as_str() == RiseDocumentProjection::CUSTOM_EMOJI_PLACEHOLDER {
                Self::append_text(chunk_start..index, body, display, style, runs);
                runs.push(InlineCustomEmojiTextRun::OrphanPlaceholder(byte_to_utf16_range(display, index..next)));
                chunk_start = next;
            } else if character.as_str() == " " {
                Self::append_text(chunk_start..next, body, display, style, runs);
                chunk_start = next;
            }
            index = next;
        }
        Self::append_text(chunk_start..segment.end, body, display, style, runs);
    }

    fn append_text(
        range: Range<usize>,
        body: &AttributedString,
        display: &str,
        style: &FormattedTextStyle,
        runs: &mut Vec<InlineCustomEmojiTextRun>,
    ) {
        if range.is_empty() {
            return;
        }
        let ns_range = byte_to_utf16_range(display, range.clone());
        if body.characters().get(range.clone()) == display.get(range.clone()) {
            runs.push(InlineCustomEmojiTextRun::Text(Self::attributed_slice(body, range), ns_range));
            return;
        }
        let Some(characters) = display.get(range) else {
            return;
        };
        let plain = AttributedString::with_attributes(
            characters,
            AttributeContainer {
                font: Some(RichFont::system(style.font_size)),
                foreground_color: Some(style.base_color),
                ..AttributeContainer::default()
            },
        );
        runs.push(InlineCustomEmojiTextRun::Text(plain, ns_range));
    }

    fn attributed_slice(body: &AttributedString, range: Range<usize>) -> AttributedString {
        let characters = body.characters();
        let mut slice = AttributedString::new("");
        for (run, attributes) in body.runs() {
            let start = run.start.max(range.start);
            let end = run.end.min(range.end);
            if start >= end {
                continue;
            }
            if let Some(part) = characters.get(start..end) {
                slice.append(AttributedString::with_attributes(part, attributes.clone()));
            }
        }
        slice
    }
}

impl RenderOnce for InlineCustomEmojiText {
    fn render(mut self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let runs = Self::build_runs(&self.text, &self.style, &self.options, &self.entities);
        let display = FormattedText::format(&self.text, &self.style, &self.options, &[]).display_string.clone();
        let mut children: Vec<InlineRunFlowChild> =
            runs.into_iter().enumerate().map(|(index, run)| self.flow_child(index, run, &display)).collect();
        if let Some(trailing_meta) = self.trailing_meta.take() {
            children.push(InlineRunFlowChild {
                element: trailing_meta,
                role: InlineRunRole::Meta,
                display_range: 0..0,
                layout: None,
            });
        }
        InlineRunFlow {
            id: self.id,
            spacing: 0.0,
            line_spacing: self.style.line_spacing,
            meta_spacing: MessageSizes::text_meta_flow_spacing(),
            children,
            selection: self.text_selection.map(|configuration| InlineRunFlowSelection {
                display: display.into(),
                configuration,
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InlineRunRole {
    Run,
    LineBreak,
    Meta,
}

struct InlineRunFlowChild {
    element: AnyElement,
    role: InlineRunRole,
    display_range: Range<usize>,
    layout: Option<TextLayout>,
}

impl InlineRunFlowChild {
    fn new(element: AnyElement, role: InlineRunRole, display: &str, range: NSRange, layout: Option<TextLayout>) -> Self {
        Self { element, role, display_range: utf16_to_byte_range(display, range).unwrap_or(0..0), layout }
    }
}

struct InlineRunFlowSelection {
    display: SharedString,
    configuration: FormattedTextSelection,
}

struct InlineRunFlow {
    id: ElementId,
    spacing: f32,
    line_spacing: f32,
    meta_spacing: f32,
    children: Vec<InlineRunFlowChild>,
    selection: Option<InlineRunFlowSelection>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct InlineRunFlowLine {
    indices: Vec<usize>,
    width: f32,
    height: f32,
}

/// where the meta goes in the inline run flow
#[derive(Clone, Copy, Debug, PartialEq)]
struct InlineRunFlowMetaPlacement {
    index: usize,
    min_y: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct InlineRunFlowPlan {
    lines: Vec<InlineRunFlowLine>,
    width: f32,
    height: f32,
    meta: Option<InlineRunFlowMetaPlacement>,
}

/// cache of intrinsic sizes the layout pass of the frame keeps
struct InlineRunFlowCache {
    roles: Rc<Vec<InlineRunRole>>,
    sizes: Rc<Vec<Size<f32>>>,
}

/// lives in element state between frames
#[derive(Clone)]
struct InlineRunFlowSelectionState {
    selection: Rc<RefCell<TextSelection>>,
    focus: FocusHandle,
    drag: Rc<Cell<(Option<Point<Pixels>>, Option<TextDragAxis>)>>,
}

/// where one run landed this frame, for pointer and highlight
#[derive(Clone)]
struct InlineRunFlowPlacedRun {
    bounds: Bounds<Pixels>,
    line_top: Pixels,
    line_bottom: Pixels,
    display_range: Range<usize>,
    layout: Option<TextLayout>,
}

struct InlineRunFlowPrepaint {
    selection: Option<(Hitbox, InlineRunFlowSelectionState)>,
    placed: Rc<Vec<InlineRunFlowPlacedRun>>,
}

impl InlineRunFlow {
    const UNCONSTRAINED_WIDTH: f32 = 100_000.0;

    const IDEAL_SPACE: Size<AvailableSpace> =
        Size { width: AvailableSpace::MaxContent, height: AvailableSpace::MaxContent };

    fn size_that_fits(
        proposal: Option<f32>,
        spacing: f32,
        line_spacing: f32,
        meta_spacing: f32,
        roles: &[InlineRunRole],
        sizes: &[Size<f32>],
    ) -> Size<f32> {
        if roles.is_empty() {
            return size(0.0, 0.0);
        }
        let column_width = proposal.filter(|width| width.is_finite() && *width < Self::UNCONSTRAINED_WIDTH).map(|width| width.max(1.0));
        let plan = Self::plan(roles, sizes, column_width.unwrap_or(f32::MAX), spacing, line_spacing, meta_spacing);
        if let (Some(_), Some(column_width)) = (plan.meta, column_width) {
            return size(column_width, plan.height);
        }
        size(column_width.map_or(plan.width, |column_width| plan.width.min(column_width)), plan.height)
    }

    fn plan(
        roles: &[InlineRunRole],
        sizes: &[Size<f32>],
        max_width: f32,
        spacing: f32,
        line_spacing: f32,
        meta_spacing: f32,
    ) -> InlineRunFlowPlan {
        let mut lines = vec![InlineRunFlowLine::default()];
        let mut meta_index = None;
        let mut breaks_line = false;
        for (index, role) in roles.iter().enumerate() {
            if *role == InlineRunRole::Meta {
                meta_index = Some(index);
                continue;
            }
            let run_size = sizes[index];
            let needs_new_line = lines.last().is_some_and(|current| {
                breaks_line || (!current.indices.is_empty() && current.width + spacing + run_size.width > max_width)
            });
            if needs_new_line {
                lines.push(InlineRunFlowLine::default());
            }
            breaks_line = *role == InlineRunRole::LineBreak;
            if let Some(line) = lines.last_mut() {
                if !line.indices.is_empty() {
                    line.width += spacing;
                }
                line.width += run_size.width;
                line.height = line.height.max(run_size.height);
                line.indices.push(index);
            }
        }

        let text_width = lines.iter().fold(0.0_f32, |width, line| width.max(line.width));
        let text_height =
            lines.iter().map(|line| line.height).sum::<f32>() + (lines.len() - 1) as f32 * line_spacing;
        let Some(meta_index) = meta_index else {
            return InlineRunFlowPlan { lines, width: text_width, height: text_height, meta: None };
        };

        let meta = sizes[meta_index];
        let (last_width, last_height, last_is_empty) =
            lines.last().map_or((0.0, 0.0, true), |last| (last.width, last.height, last.indices.is_empty()));
        let inline_width = last_width + if last_is_empty { 0.0 } else { meta_spacing } + meta.width;
        if inline_width <= max_width {
            let line_height = last_height.max(meta.height);
            if let Some(last) = lines.last_mut() {
                last.height = line_height;
            }
            let height = text_height - last_height + line_height;
            return InlineRunFlowPlan {
                lines,
                width: text_width.max(inline_width.ceil()),
                height,
                meta: Some(InlineRunFlowMetaPlacement {
                    index: meta_index,
                    min_y: (height - meta.height - 1.0).max(0.0),
                }),
            };
        }
        InlineRunFlowPlan {
            lines,
            width: text_width.max(meta.width),
            height: text_height + 2.0 + meta.height,
            meta: Some(InlineRunFlowMetaPlacement { index: meta_index, min_y: text_height + 2.0 }),
        }
    }

    /// byte of the displayed text under the pointer
    /// nearest line by height then nearest run then text layout inside it
    fn index_at(placed: &[InlineRunFlowPlacedRun], position: Point<Pixels>) -> usize {
        let line_distance = |run: &InlineRunFlowPlacedRun| {
            if position.y < run.line_top {
                f32::from(run.line_top - position.y)
            } else if position.y > run.line_bottom {
                f32::from(position.y - run.line_bottom)
            } else {
                0.0
            }
        };
        let run_distance = |run: &InlineRunFlowPlacedRun| {
            if position.x < run.bounds.left() {
                f32::from(run.bounds.left() - position.x)
            } else if position.x > run.bounds.right() {
                f32::from(position.x - run.bounds.right())
            } else {
                0.0
            }
        };
        let nearest = placed.iter().min_by(|a, b| {
            line_distance(a).total_cmp(&line_distance(b)).then_with(|| run_distance(a).total_cmp(&run_distance(b)))
        });
        let Some(run) = nearest else {
            return 0;
        };
        match run.layout.as_ref() {
            Some(layout) => {
                let x = if position.x < run.bounds.left() {
                    run.bounds.left()
                } else if position.x > run.bounds.right() {
                    run.bounds.right()
                } else {
                    position.x
                };
                let inside = point(x, run.bounds.center().y);
                let index = match layout.index_for_position(inside) {
                    Ok(index) | Err(index) => index,
                };
                (run.display_range.start + index).min(run.display_range.end)
            }
            None if position.x < run.bounds.center().x => run.display_range.start,
            None => run.display_range.end,
        }
    }

    fn paint_selection(selected: Range<usize>, placed: &[InlineRunFlowPlacedRun], color: gpui::Hsla, window: &mut Window) {
        if selected.is_empty() {
            return;
        }
        for run in placed {
            let start = selected.start.max(run.display_range.start);
            let end = selected.end.min(run.display_range.end);
            if start >= end {
                continue;
            }
            let rect = match run.layout.as_ref() {
                Some(layout) => {
                    let (Some(from), Some(to)) = (
                        layout.position_for_index(start - run.display_range.start),
                        layout.position_for_index(end - run.display_range.start),
                    ) else {
                        continue;
                    };
                    Bounds::from_corners(point(from.x, run.bounds.top()), point(to.x, run.bounds.bottom()))
                }
                None => run.bounds,
            };
            window.paint_quad(fill(rect, color));
        }
    }

    fn install_selection_listeners(
        hitbox: &Hitbox,
        state: &InlineRunFlowSelectionState,
        selection: &InlineRunFlowSelection,
        placed: &Rc<Vec<InlineRunFlowPlacedRun>>,
        window: &mut Window,
    ) {
        let press_gate = selection.configuration.press_gate.clone();
        let display = selection.display.clone();

        window.on_mouse_event({
            let selection_state = state.selection.clone();
            let focus = state.focus.clone();
            let hitbox = hitbox.clone();
            let placed = placed.clone();
            let display = display.clone();
            let press_gate = press_gate.clone();
            let single_click_when = selection.configuration.single_click_when.clone();
            let drag = state.drag.clone();
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                if !hitbox.is_hovered(window) {
                    if selection_state.borrow_mut().clear() {
                        window.refresh();
                    }
                    return;
                }
                if event.button != MouseButton::Left || press_gate(cx) {
                    return;
                }
                mark_selectable_text_press(event.position, cx);
                let on_glyph = placed.iter().any(|run| {
                    (run.line_top..=run.line_bottom).contains(&event.position.y)
                        && (run.bounds.left()..=run.bounds.right()).contains(&event.position.x)
                });
                if on_glyph {
                    mark_text_glyph_press(event.position, cx);
                }
                drag.set((Some(event.position), None));
                let click_count = if !on_glyph || single_click_when.as_ref().is_some_and(|single| single(cx)) {
                    1
                } else {
                    event.click_count
                };
                let index = Self::index_at(&placed, event.position);
                selection_state.borrow_mut().press(&display, index, click_count, event.modifiers.shift);
                if !selection_state.borrow().is_empty() {
                    window.focus(&focus, cx);
                }
                window.refresh();
            }
        });

        window.on_mouse_event({
            let selection_state = state.selection.clone();
            let focus = state.focus.clone();
            let placed = placed.clone();
            let display = display.clone();
            let drag = state.drag.clone();
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !selection_state.borrow().is_dragging() {
                    return;
                }
                if event.pressed_button != Some(MouseButton::Left) {
                    selection_state.borrow_mut().release();
                    return;
                }
                if let (Some(press), axis) = drag.get() {
                    if text_press_swiped(press, cx) {
                        let mut selection = selection_state.borrow_mut();
                        selection.release();
                        selection.clear();
                        drop(selection);
                        window.refresh();
                        return;
                    }
                    let axis = match axis {
                        Some(axis) => axis,
                        None => {
                            let Some(axis) = text_drag_axis(press, event.position) else {
                                return;
                            };
                            drag.set((Some(press), Some(axis)));
                            axis
                        }
                    };
                    if axis != TextDragAxis::Select {
                        if axis == TextDragAxis::Neither || text_press_swiped(press, cx) {
                            selection_state.borrow_mut().release();
                        }
                        return;
                    }
                }
                if press_gate(cx) {
                    selection_state.borrow_mut().release();
                    selection_state.borrow_mut().clear();
                    window.refresh();
                    return;
                }
                let index = Self::index_at(&placed, event.position);
                if selection_state.borrow_mut().drag_to(&display, index) {
                    if !selection_state.borrow().is_empty() {
                        window.focus(&focus, cx);
                    }
                    window.refresh();
                }
            }
        });

        window.on_mouse_event({
            let selection_state = state.selection.clone();
            move |event: &MouseUpEvent, phase, _window, _cx| {
                if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                    selection_state.borrow_mut().release();
                }
            }
        });

        if let Ok(context) = KeyContext::parse(input_actions::KEY_CONTEXT) {
            window.set_key_context(context);
        }

        window.on_action(TypeId::of::<input_actions::Copy>(), {
            let selection_state = state.selection.clone();
            let display = display.clone();
            move |_, phase, _window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                match selection_state.borrow().selected_text(&display) {
                    Some(selected) => UIPasteboard::general().set_string(&MessagePreview::plain_custom_emoji(selected), cx),
                    None => cx.propagate(),
                }
            }
        });

        window.on_action(TypeId::of::<input_actions::SelectAll>(), {
            let selection_state = state.selection.clone();
            move |_, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                if selection_state.borrow().is_empty() {
                    cx.propagate();
                    return;
                }
                selection_state.borrow_mut().select_all(&display);
                window.refresh();
            }
        });
    }
}

impl IntoElement for InlineRunFlow {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for InlineRunFlow {
    type RequestLayoutState = InlineRunFlowCache;
    type PrepaintState = InlineRunFlowPrepaint;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
        let sizes: Vec<Size<f32>> = self
            .children
            .iter_mut()
            .map(|child| {
                let measured = child.element.layout_as_root(Self::IDEAL_SPACE, window, cx);
                size(f32::from(measured.width), f32::from(measured.height))
            })
            .collect();
        let roles: Rc<Vec<InlineRunRole>> = Rc::new(self.children.iter().map(|child| child.role).collect());
        let sizes = Rc::new(sizes);
        let (measure_roles, measure_sizes) = (roles.clone(), sizes.clone());
        let (spacing, line_spacing, meta_spacing) = (self.spacing, self.line_spacing, self.meta_spacing);
        let layout_id = window.request_measured_layout(Style::default(), move |known, available, _window, _cx| {
            let proposal = known.width.map(f32::from).or(match available.width {
                AvailableSpace::Definite(width) => Some(f32::from(width)),
                AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
            });
            let fitted = Self::size_that_fits(proposal, spacing, line_spacing, meta_spacing, &measure_roles, &measure_sizes);
            size(px(fitted.width), px(fitted.height))
        });
        (layout_id, InlineRunFlowCache { roles, sizes })
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        cache: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let selection = self.selection.as_ref().map(|_| {
            input_actions::install_key_bindings(cx);
            let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
            let state = match global_id {
                Some(global_id) => window.with_element_state::<InlineRunFlowSelectionState, _>(global_id, |state, _| {
                    let state = state.unwrap_or_else(|| InlineRunFlowSelectionState {
                        selection: Rc::default(),
                        focus: cx.focus_handle(),
                        drag: Rc::default(),
                    });
                    (state.clone(), state)
                }),
                None => InlineRunFlowSelectionState {
                    selection: Rc::default(),
                    focus: cx.focus_handle(),
                    drag: Rc::default(),
                },
            };
            window.set_focus_handle(&state.focus, cx);
            (hitbox, state)
        });

        let plan = Self::plan(
            &cache.roles,
            &cache.sizes,
            f32::from(bounds.size.width),
            self.spacing,
            self.line_spacing,
            self.meta_spacing,
        );
        let (min_x, min_y) = (f32::from(bounds.left()), f32::from(bounds.top()));
        let mut placed = Vec::new();
        let mut y = min_y;
        for line in &plan.lines {
            let mut x = min_x;
            for &index in &line.indices {
                let run_size = cache.sizes[index];
                let origin = point(px(x), px(y + (line.height - run_size.height) / 2.0));
                let child = &mut self.children[index];
                child.element.prepaint_at(origin, window, cx);
                placed.push(InlineRunFlowPlacedRun {
                    bounds: Bounds::new(origin, size(px(run_size.width), px(run_size.height))),
                    line_top: px(y),
                    line_bottom: px(y + line.height),
                    display_range: child.display_range.clone(),
                    layout: child.layout.clone(),
                });
                x += run_size.width + self.spacing;
            }
            y += line.height + self.line_spacing;
        }
        if let Some(meta) = plan.meta {
            let meta_size = cache.sizes[meta.index];
            self.children[meta.index].element.prepaint_at(
                point(bounds.right() - px(meta_size.width), px(min_y + meta.min_y)),
                window,
                cx,
            );
        }
        InlineRunFlowPrepaint { selection, placed: Rc::new(placed) }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let (Some(selection), Some((hitbox, state))) = (self.selection.as_ref(), prepaint.selection.as_ref()) {
            state.selection.borrow_mut().fit(&selection.display);
            let selected = state.selection.borrow().range();
            Self::paint_selection(selected, &prepaint.placed, selection.configuration.color, window);
            window.set_cursor_style(CursorStyle::IBeam, hitbox);
            Self::install_selection_listeners(hitbox, state, selection, &prepaint.placed, window);
        }
        for child in &mut self.children {
            child.element.paint(window, cx);
        }
    }
}
