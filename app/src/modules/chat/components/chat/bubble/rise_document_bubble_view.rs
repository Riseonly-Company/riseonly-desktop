use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, ElementId, Entity, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Transformation, Window, canvas, div, px,
    radians,
};
use rise_platform::haptics::selection_changed;
use rise_canon::{RiseDocumentInlineMark, RiseDocumentListStyle};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::character_count;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::rich_text::document::rise_document::{
    RiseDocument, RiseDocumentBlock, RiseDocumentButton, RiseDocumentRun, RiseDocumentTable,
};
use rise_ui::rich_text::document::rise_document_projection::RiseDocumentProjection;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::font::{Font, FontDesign, SwiftUiFont};
use rise_ui::swiftui::motion::{MotionSpec, ease_out, spring};
use rise_ui::swiftui::open_url::open_url;
use rise_ui::swiftui::rich_text::{AttributeContainer, AttributedString, LineStyle, RichFont, RichText};
use rise_ui::{IconUi, MainText, SecondaryText, TextAlignT};

use crate::core::ui::formatted_text_ui::{FormattedTextSelection, FormattedTextStyle, FormattedTextUi};
use crate::core::ui::rich_text::code_block_view::CodeBlockView;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;

/// draws the media item a media block names
/// the document stores an index never an address so the media set owner decides how to render it
pub type RiseDocumentMediaContent = Rc<dyn Fn(usize, &mut Window, &mut App) -> Option<AnyElement>>;

/// theme colours the document reads, taken once per render
#[derive(Clone, Copy)]
struct DocumentColors {
    text_primary: Hsla,
    text_secondary: Hsla,
    bg_300: Hsla,
    primary_100: Hsla,
}

impl DocumentColors {
    fn current(cx: &mut App) -> Self {
        let store = ThemeStore::shared(cx);
        let theme = store.read(cx).current_theme();
        Self {
            text_primary: theme.text.primary,
            text_secondary: theme.text.secondary,
            bg_300: theme.bg._300,
            primary_100: theme.primary._100,
        }
    }
}

fn child_id(parent: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(parent.clone()), name.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CollapsedCut {
    block_count: usize,
    last_block_line_limit: Option<usize>,
}

impl CollapsedCut {
    fn cuts_last_block(&self) -> bool {
        self.last_block_line_limit.is_some()
    }
}

#[derive(Default)]
struct RiseDocumentBubbleViewState {
    is_expanded: bool,
    revealed_spoilers: RevealedSpoilers,
    expansion: HeightTransition,
}

impl RiseDocumentBubbleViewState {
    fn expansion(&mut self) -> &mut HeightTransition {
        &mut self.expansion
    }
}

/// revealed spoilers with the animation it was last set in
/// text crossfades the runs that open so the last revealed block and progress are kept beside the set
struct RevealedSpoilers {
    revealed: HashSet<usize>,
    revealing: Option<usize>,
    progress: Animated<f32>,
}

impl Default for RevealedSpoilers {
    fn default() -> Self {
        Self {
            revealed: HashSet::new(),
            revealing: None,
            progress: Animated::new(1.0),
        }
    }
}

impl RevealedSpoilers {
    /// reveals a block, false when it was already open
    fn reveal(&mut self, block_index: usize, cx: &App) -> bool {
        if self.revealed.contains(&block_index) {
            return false;
        }
        selection_changed();
        self.revealed.insert(block_index);
        self.revealing = Some(block_index);
        self.progress.jump(0.0);
        self.progress.set_in(1.0, ease_out(0.18), RiseClock::monotonic(), cx);
        true
    }

    /// the revealed set and the crossfade of this frame
    fn frame(&self, window: &Window) -> SpoilerFrame {
        let progress = self.progress.frame_value(RiseClock::monotonic(), window);
        SpoilerFrame {
            revealed: self.revealed.clone(),
            fading: self.revealing.filter(|_| progress < 1.0).map(|block_index| (block_index, progress)),
        }
    }
}

/// what a block reads of the revealed spoilers in one frame
#[derive(Clone, Default)]
struct SpoilerFrame {
    revealed: HashSet<usize>,
    fading: Option<(usize, f32)>,
}

impl SpoilerFrame {
    fn fade_of(&self, block_index: usize) -> Option<f32> {
        self.fading.filter(|(index, _)| *index == block_index).map(|(_, progress)| progress)
    }
}

/// stack height a toggle animates through
/// gpui lays a new height out at once so the clip plays the spring from the old height to the new
struct HeightTransition {
    height: Animated<f32>,
    natural_height: f32,
    pending: Option<MotionSpec>,
    awaiting: Option<(f32, MotionSpec)>,
    drawn_expanded: Option<bool>,
}

impl Default for HeightTransition {
    fn default() -> Self {
        Self {
            height: Animated::new(0.0),
            natural_height: 0.0,
            pending: None,
            awaiting: None,
            drawn_expanded: None,
        }
    }
}

impl HeightTransition {
    fn animate_next_change(&mut self, spec: MotionSpec) {
        self.pending = Some(spec);
    }
}

/// wraps body in the clip of the height transition that lens picks out of state
fn height_transition<S: 'static>(
    state: &Entity<S>,
    lens: fn(&mut S) -> &mut HeightTransition,
    expanded: bool,
    body: AnyElement,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let now = RiseClock::monotonic();
    let clip_height = state.update(cx, |state, _| {
        let transition = lens(state);
        if transition.drawn_expanded != Some(expanded) {
            if transition.drawn_expanded.is_some()
                && let Some(spec) = transition.pending
            {
                let on_screen = if transition.height.is_running(now) {
                    transition.height.value(now)
                } else {
                    transition.natural_height
                };
                transition.awaiting = Some((on_screen, spec));
            }
            transition.drawn_expanded = Some(expanded);
            transition.pending = None;
        }
        match transition.awaiting {
            Some((held, _)) => Some(held),
            None => transition
                .height
                .is_running(now)
                .then(|| transition.height.frame_value(now, window)),
        }
    });
    let measured = state.clone();
    let probe = canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| {
            let height = f32::from(bounds.size.height);
            let now = RiseClock::monotonic();
            measured.update(cx, |state, cx| {
                let transition = lens(state);
                if let Some((from, spec)) = transition.awaiting.take() {
                    transition.natural_height = height;
                    transition.height.jump(from);
                    transition.height.set_in(height, spec, now, cx);
                    window.request_animation_frame();
                } else if !transition.height.is_running(now) {
                    transition.natural_height = height;
                    transition.height.jump(height);
                }
            });
        },
    )
    .absolute()
    .inset_0();
    let body = div().flex_shrink_0().relative().child(probe).child(body);
    match clip_height {
        Some(height) => div()
            .flex()
            .flex_col()
            .h(px(height.max(0.0)))
            .overflow_hidden()
            .child(body)
            .into_any_element(),
        None => div().flex().flex_col().child(body).into_any_element(),
    }
}

/// formatted document inside a message bubble
/// collapsed at the block limit or character budget, whichever comes first
/// a long article costs frames in a fast scrolling row
#[derive(IntoElement)]
pub struct RiseDocumentBubbleView {
    id: ElementId,
    document: RiseDocument,
    style: FormattedTextStyle,
    collapsed_block_limit: usize,
    collapsed_character_budget: usize,
    media_content: Option<RiseDocumentMediaContent>,
    text_selection: Option<FormattedTextSelection>,
}

impl RiseDocumentBubbleView {
    const CHARACTERS_PER_LINE: usize = 45;
    const MINIMUM_CUT_LINES: usize = 4;

    pub fn new(id: impl Into<ElementId>, document: RiseDocument, style: FormattedTextStyle) -> Self {
        Self {
            id: id.into(),
            document,
            style,
            collapsed_block_limit: 12,
            collapsed_character_budget: 700,
            media_content: None,
            text_selection: None,
        }
    }

    pub fn collapsed_block_limit(mut self, collapsed_block_limit: usize) -> Self {
        self.collapsed_block_limit = collapsed_block_limit;
        self
    }

    pub fn collapsed_character_budget(mut self, collapsed_character_budget: usize) -> Self {
        self.collapsed_character_budget = collapsed_character_budget;
        self
    }

    pub fn media_content(mut self, media_content: Option<RiseDocumentMediaContent>) -> Self {
        self.media_content = media_content;
        self
    }

    /// mouse selection of the document text, block by block
    /// the bubble guard decides whether there is one
    pub fn text_selection(mut self, text_selection: Option<FormattedTextSelection>) -> Self {
        self.text_selection = text_selection;
        self
    }

    fn collapsed_cut(&self) -> CollapsedCut {
        let blocks = &self.document.blocks;
        let mut used = 0usize;
        for (index, block) in blocks.iter().enumerate() {
            if index >= self.collapsed_block_limit {
                return CollapsedCut {
                    block_count: index,
                    last_block_line_limit: None,
                };
            }
            let length =
                character_count(&RiseDocumentProjection::plain_text(&RiseDocument::new(vec![block.clone()])));
            if used + length > self.collapsed_character_budget {
                if !is_plain_text_run(block) {
                    return CollapsedCut {
                        block_count: index.max(1),
                        last_block_line_limit: None,
                    };
                }
                if length <= Self::MINIMUM_CUT_LINES * Self::CHARACTERS_PER_LINE {
                    return CollapsedCut {
                        block_count: index + 1,
                        last_block_line_limit: None,
                    };
                }
                let remaining = self.collapsed_character_budget.saturating_sub(used);
                let lines = (remaining / Self::CHARACTERS_PER_LINE).max(Self::MINIMUM_CUT_LINES);
                return CollapsedCut {
                    block_count: index + 1,
                    last_block_line_limit: Some(lines),
                };
            }
            used += length;
        }
        CollapsedCut {
            block_count: blocks.len(),
            last_block_line_limit: None,
        }
    }
}

impl RenderOnce for RiseDocumentBubbleView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = DocumentColors::current(cx);
        let state: Entity<RiseDocumentBubbleViewState> =
            window.use_keyed_state(child_id(&self.id, "document-state"), cx, |_, _| RiseDocumentBubbleViewState::default());
        let (is_expanded, revealed_spoilers) = {
            let current = state.read(cx);
            (current.is_expanded, current.revealed_spoilers.frame(window))
        };
        let cut = self.collapsed_cut();
        let is_truncated = cut.block_count < self.document.blocks.len() || cut.cuts_last_block();
        let visible_count = if is_expanded {
            self.document.blocks.len()
        } else {
            cut.block_count.min(self.document.blocks.len())
        };
        let cut_index = if is_expanded || !cut.cuts_last_block() {
            None
        } else {
            cut.block_count.checked_sub(1)
        };

        let mut column = div().flex().flex_col().items_start().gap(px(6.0));
        for (index, block) in self.document.blocks.iter().take(visible_count).enumerate() {
            let reveal_state = state.clone();
            column = column.child(RiseDocumentBlockView {
                id: child_id(&self.id, format!("block-{index}")),
                block: block.clone(),
                index,
                style: self.style.clone(),
                theme,
                media_content: self.media_content.clone(),
                line_limit: if Some(index) == cut_index { cut.last_block_line_limit } else { None },
                revealed_spoilers: revealed_spoilers.clone(),
                text_selection: self.text_selection.clone(),
                on_reveal: Rc::new(move |block_index, _window, cx| {
                    reveal_state.update(cx, |state, cx| {
                        if state.revealed_spoilers.reveal(block_index, cx) {
                            cx.notify();
                        }
                    });
                }),
            });
        }
        if is_truncated {
            let toggle_state = state.clone();
            let label = if is_expanded { tr("doc_read_less") } else { tr("doc_read_more") };
            column = column.child(
                div()
                    .child(MainText::new(label).px(self.style.font_size).color(theme.text_secondary))
                    .bubble_tap_target(child_id(&self.id, "toggle"), move |_window, cx| {
                        selection_changed();
                        toggle_state.update(cx, |state, cx| {
                            state.expansion.animate_next_change(spring(0.35, 0.86));
                            state.is_expanded = !state.is_expanded;
                            cx.notify();
                        });
                    }),
            );
        }
        height_transition(
            &state,
            RiseDocumentBubbleViewState::expansion,
            is_expanded,
            column.into_any_element(),
            window,
            cx,
        )
    }
}

/// called with the block index whose spoiler was tapped
type RevealSpoiler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

#[derive(IntoElement)]
struct RiseDocumentBlockView {
    id: ElementId,
    block: RiseDocumentBlock,
    index: usize,
    style: FormattedTextStyle,
    theme: DocumentColors,
    media_content: Option<RiseDocumentMediaContent>,
    line_limit: Option<usize>,
    revealed_spoilers: SpoilerFrame,
    text_selection: Option<FormattedTextSelection>,
    on_reveal: RevealSpoiler,
}

impl RiseDocumentBlockView {
    fn inline(
        &self,
        id: ElementId,
        runs: &[RiseDocumentRun],
        size_delta: f32,
        weight: Option<FontWeight>,
        italic: bool,
    ) -> RiseDocumentInlineText {
        RiseDocumentInlineText {
            id,
            runs: runs.to_vec(),
            style: self.style.clone(),
            theme: self.theme,
            size_delta,
            base_weight: weight,
            base_italic: italic,
            block_index: self.index,
            line_limit: self.line_limit,
            is_revealed: self.revealed_spoilers.revealed.contains(&self.index),
            reveal_fade: self.revealed_spoilers.fade_of(self.index),
            text_selection: self.text_selection.clone(),
            on_reveal: self.on_reveal.clone(),
        }
    }

    fn quote(&self, blocks: &[RiseDocumentBlock], is_pull: bool) -> AnyElement {
        let mut inner = div().flex().flex_col().items_start().gap(px(4.0)).flex_1().min_w_0();
        for (position, block) in blocks.iter().enumerate() {
            if let RiseDocumentBlock::Paragraph { runs } = block {
                inner = inner.child(self.inline(
                    child_id(&self.id, format!("quote-{position}")),
                    runs,
                    if is_pull { 1.0 } else { 0.0 },
                    None,
                    is_pull,
                ));
            }
        }
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(8.0))
            .child(
                div()
                    .self_stretch()
                    .w(px(if is_pull { 4.0 } else { 3.0 }))
                    .rounded(px(1.5))
                    .bg(self.theme.primary_100),
            )
            .child(inner)
            .into_any_element()
    }

    fn marker(&self, list_style: RiseDocumentListStyle, position: usize, is_done: bool) -> AnyElement {
        match list_style {
            RiseDocumentListStyle::Bulleted => div()
                .mt(px(self.style.font_size * 0.55))
                .size(px(5.0))
                .rounded_full()
                .bg(self.theme.text_primary)
                .into_any_element(),
            RiseDocumentListStyle::Numbered => MainText::new(format!("{position}."))
                .px(self.style.font_size)
                .color(self.theme.text_secondary)
                .into_any_element(),
            RiseDocumentListStyle::Task => {
                let (symbol, color) = if is_done {
                    ("checkmark.square.fill", self.theme.primary_100)
                } else {
                    ("square", self.theme.text_secondary)
                };
                match IconUi::sized(symbol, px(self.style.font_size), color, is_done) {
                    Some(icon) => icon.into_any_element(),
                    None => div().into_any_element(),
                }
            }
        }
    }

    fn button_row(&self, buttons: &[RiseDocumentButton]) -> AnyElement {
        let mut row = div().flex().flex_row().gap(px(6.0));
        for (position, button) in buttons.iter().enumerate() {
            let destination = Url::from_string(&button.url).or_else(|| Url::from_string("about:blank"));
            row = row.child(
                div()
                    .id(child_id(&self.id, format!("button-{position}")))
                    .px(px(12.0))
                    .py(px(7.0))
                    .rounded(px(9.0))
                    .bg(self.theme.primary_100.opacity(0.14))
                    .cursor_pointer()
                    .child(
                        MainText::new(button.label.clone())
                            .px(self.style.font_size - 1.0)
                            .color(self.theme.primary_100),
                    )
                    .on_click(move |_, window, cx| {
                        if let Some(url) = &destination {
                            open_url(cx).call_as_function(url, window, cx);
                        }
                    }),
            );
        }
        row.into_any_element()
    }
}

fn is_plain_text_run(block: &RiseDocumentBlock) -> bool {
    matches!(block, RiseDocumentBlock::Paragraph { .. } | RiseDocumentBlock::Heading { .. })
}

fn runs_of(blocks: &[RiseDocumentBlock]) -> Vec<RiseDocumentRun> {
    for block in blocks {
        if let RiseDocumentBlock::Paragraph { runs } = block {
            return runs.clone();
        }
    }
    Vec::new()
}

impl RenderOnce for RiseDocumentBlockView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let style = &self.style;
        let theme = self.theme;
        match &self.block {
            RiseDocumentBlock::Paragraph { runs } => {
                self.inline(child_id(&self.id, "text"), runs, 0.0, None, false).into_any_element()
            }
            RiseDocumentBlock::Heading { level, runs } => {
                let delta = match level {
                    1 => 5.0,
                    2 => 3.0,
                    _ => 1.0,
                };
                self.inline(child_id(&self.id, "text"), runs, delta, Some(FontWeight::BOLD), false)
                    .into_any_element()
            }
            RiseDocumentBlock::Quote { blocks } => self.quote(blocks, false),
            RiseDocumentBlock::PullQuote { blocks } => self.quote(blocks, true),
            RiseDocumentBlock::Code { language, text } => {
                CodeBlockView::new(child_id(&self.id, "code"), language.clone(), text.clone())
                    .font_size(style.font_size - 1.5)
                    .in_bubble(true)
                    .text_selection(self.text_selection.clone())
                    .into_any_element()
            }
            RiseDocumentBlock::Formula { text } => {
                let formula = match &self.text_selection {
                    Some(selection) => FormattedTextUi::selectable_body(
                        child_id(&self.id, "formula"),
                        AttributedString::new(text.clone()),
                        selection.clone(),
                    ),
                    None => SharedString::from(text.clone()).into_any_element(),
                };
                div()
                    .px(px(8.0))
                    .py(px(5.0))
                    .rounded(px(6.0))
                    .bg(theme.bg_300)
                    .swiftui_font(Font::system(style.font_size, FontWeight::NORMAL, FontDesign::Monospaced))
                    .text_color(theme.text_primary)
                    .child(formula)
                    .into_any_element()
            }
            RiseDocumentBlock::List { style: list_style, items } => {
                let mut list = div().flex().flex_col().items_start().gap(px(4.0));
                for (position, item) in items.iter().enumerate() {
                    list = list.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_start()
                            .gap(px(8.0))
                            .child(self.marker(*list_style, position + 1, item.is_done.unwrap_or(false)))
                            .child(
                                div().flex_1().min_w_0().child(self.inline(
                                    child_id(&self.id, format!("item-{position}")),
                                    &runs_of(&item.blocks),
                                    0.0,
                                    None,
                                    false,
                                )),
                            ),
                    );
                }
                list.into_any_element()
            }
            RiseDocumentBlock::Collapsible { title, expanded, blocks } => RiseDocumentCollapsibleView {
                id: child_id(&self.id, "collapsible"),
                title: title.clone(),
                initially_expanded: *expanded,
                blocks: blocks.clone(),
                style: style.clone(),
                theme,
                text_selection: self.text_selection.clone(),
            }
            .into_any_element(),
            RiseDocumentBlock::Table(table) => RiseDocumentTableView {
                id: child_id(&self.id, "table"),
                table: table.clone(),
                style: style.clone(),
                theme,
                text_selection: self.text_selection.clone(),
            }
            .into_any_element(),
            RiseDocumentBlock::ButtonRow { buttons } => self.button_row(buttons),
            RiseDocumentBlock::Divider => div()
                .py(px(4.0))
                .w_full()
                .child(div().w_full().h(px(1.0)).bg(theme.text_secondary.opacity(0.3)))
                .into_any_element(),
            RiseDocumentBlock::Media { index } => {
                match &self.media_content {
                    Some(media_content) => MediaSlot {
                        media_content: media_content.clone(),
                        index: *index,
                    }
                    .into_any_element(),
                    None => div().into_any_element(),
                }
            }
            RiseDocumentBlock::Unsupported { .. } => SecondaryText::new(tr("doc_unsupported_block"))
                .px(style.font_size - 2.0)
                .into_any_element(),
        }
    }
}

/// calls the owner media closure inside a render pass where it gets window and app
#[derive(IntoElement)]
struct MediaSlot {
    media_content: RiseDocumentMediaContent,
    index: usize,
}

impl RenderOnce for MediaSlot {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        div().children((self.media_content)(self.index, window, cx))
    }
}

/// one line of marked up text, spoiler runs stay masked until tapped (tracked per block)
/// spoiler text never enters content so it never reaches a push notification
#[derive(IntoElement)]
struct RiseDocumentInlineText {
    id: ElementId,
    runs: Vec<RiseDocumentRun>,
    style: FormattedTextStyle,
    theme: DocumentColors,
    size_delta: f32,
    base_weight: Option<FontWeight>,
    base_italic: bool,
    block_index: usize,
    line_limit: Option<usize>,
    is_revealed: bool,
    reveal_fade: Option<f32>,
    text_selection: Option<FormattedTextSelection>,
    on_reveal: RevealSpoiler,
}

impl RiseDocumentInlineText {
    fn has_spoiler(&self) -> bool {
        self.runs.iter().any(|run| run.marks.contains(&RiseDocumentInlineMark::Spoiler))
    }

    fn masked(text: &str) -> String {
        "\u{2588}".repeat(character_count(text).clamp(1, 12))
    }

    fn styled(&self, run: &RiseDocumentRun) -> AttributedString {
        let size = self.style.font_size + self.size_delta;
        let is_code = run.marks.contains(&RiseDocumentInlineMark::Code);
        let is_hidden_spoiler = run.marks.contains(&RiseDocumentInlineMark::Spoiler) && !self.is_revealed;
        let mut font = if is_code {
            RichFont::system_design(size - 1.0, FontDesign::Monospaced)
        } else {
            RichFont::system(size)
        };
        if run.marks.contains(&RiseDocumentInlineMark::Bold) || self.base_weight == Some(FontWeight::BOLD) {
            font.weight = FontWeight::BOLD;
        }
        if run.marks.contains(&RiseDocumentInlineMark::Italic) || self.base_italic {
            font = font.italic();
        }
        let mut attributes = AttributeContainer {
            font: Some(font),
            ..AttributeContainer::default()
        };
        if run.marks.contains(&RiseDocumentInlineMark::Underline) {
            attributes.underline_style = Some(LineStyle::single());
        }
        if run.marks.contains(&RiseDocumentInlineMark::Strikethrough) {
            attributes.strikethrough_style = Some(LineStyle::single());
        }
        attributes.foreground_color = Some(if is_hidden_spoiler {
            self.theme.text_secondary
        } else if run.marks.contains(&RiseDocumentInlineMark::Link) {
            attributes.underline_style = Some(LineStyle::single());
            self.theme.primary_100
        } else if is_code {
            self.theme.text_primary
        } else {
            self.style.base_color
        });
        if run.marks.contains(&RiseDocumentInlineMark::Spoiler)
            && let Some(progress) = self.reveal_fade
            && let Some(color) = attributes.foreground_color.as_mut()
        {
            color.a *= progress;
        }
        let text = if is_hidden_spoiler { Self::masked(&run.text) } else { run.text.clone() };
        AttributedString::with_attributes(text, attributes)
    }
}

impl RenderOnce for RiseDocumentInlineText {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let mut text = AttributedString::new("");
        for run in &self.runs {
            text.append(self.styled(run));
        }
        let size = self.style.font_size + self.size_delta;
        let body = match self.text_selection.clone() {
            Some(selection) => FormattedTextUi::selectable_body(child_id(&self.id, "runs"), text, selection),
            None => RichText::new(child_id(&self.id, "runs"), text).into_any_element(),
        };
        let mut laid_out = div()
            .w_full()
            .swiftui_font(Font::system_size(size))
            .text_color(self.style.base_color)
            .child(body);
        if let Some(line_limit) = self.line_limit {
            laid_out = laid_out.line_clamp(line_limit);
        }
        if self.has_spoiler() {
            let on_reveal = self.on_reveal.clone();
            let block_index = self.block_index;
            let is_revealed = self.is_revealed;
            laid_out
                .bubble_tap_target(child_id(&self.id, "reveal"), move |window, cx| {
                    if is_revealed {
                        return;
                    }
                    on_reveal(block_index, window, cx);
                })
                .into_any_element()
        } else {
            laid_out.into_any_element()
        }
    }
}

struct RiseDocumentCollapsibleState {
    is_expanded: bool,
    chevron: Animated<f32>,
    revealed: RevealedSpoilers,
    expansion: HeightTransition,
}

impl RiseDocumentCollapsibleState {
    fn expansion(&mut self) -> &mut HeightTransition {
        &mut self.expansion
    }
}

#[derive(IntoElement)]
struct RiseDocumentCollapsibleView {
    id: ElementId,
    title: Vec<RiseDocumentRun>,
    initially_expanded: bool,
    blocks: Vec<RiseDocumentBlock>,
    style: FormattedTextStyle,
    theme: DocumentColors,
    text_selection: Option<FormattedTextSelection>,
}

impl RenderOnce for RiseDocumentCollapsibleView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let initially_expanded = self.initially_expanded;
        let state: Entity<RiseDocumentCollapsibleState> =
            window.use_keyed_state(child_id(&self.id, "collapsible-state"), cx, move |_, _| {
                RiseDocumentCollapsibleState {
                    is_expanded: initially_expanded,
                    chevron: Animated::new(if initially_expanded { 90.0 } else { 0.0 }),
                    revealed: RevealedSpoilers::default(),
                    expansion: HeightTransition::default(),
                }
            });
        let (is_expanded, chevron_degrees, revealed) = {
            let current = state.read(cx);
            (
                current.is_expanded,
                current.chevron.frame_value(RiseClock::monotonic(), window),
                current.revealed.frame(window),
            )
        };
        let theme = self.theme;
        let font_size = self.style.font_size;

        let mut header = div().flex().flex_row().items_center().gap(px(6.0));
        if let Some(chevron) = IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(font_size - 3.0), theme.text_secondary, false) {
            header = header.child(chevron.with_transformation(Transformation::rotate(radians(
                chevron_degrees.to_radians(),
            ))));
        }
        header = header.child(
            MainText::new(self.title.iter().map(|run| run.text.as_str()).collect::<String>())
                .px(font_size)
                .font_weight(FontWeight::SEMIBOLD)
                .color(theme.text_primary),
        );
        let toggle_state = state.clone();
        let mut column = div().flex().flex_col().items_start().gap(px(6.0)).child(
            header.bubble_tap_target(child_id(&self.id, "toggle"), move |_window, cx| {
                let reduce_motion = reduce_motion(cx);
                toggle_state.update(cx, |state, cx| {
                    state.expansion.animate_next_change(spring(0.32, 0.86));
                    state.is_expanded = !state.is_expanded;
                    let target = if state.is_expanded { 90.0 } else { 0.0 };
                    state
                        .chevron
                        .set(target, Some(spring(0.32, 0.86)), RiseClock::monotonic(), reduce_motion);
                    cx.notify();
                });
            }),
        );

        if is_expanded {
            let mut body = div().flex().flex_col().items_start().gap(px(4.0)).pl(px(font_size));
            for (index, block) in self.blocks.iter().enumerate() {
                let reveal_state = state.clone();
                body = body.child(RiseDocumentBlockView {
                    id: child_id(&self.id, format!("block-{index}")),
                    block: block.clone(),
                    index,
                    style: self.style.clone(),
                    theme,
                    media_content: None,
                    line_limit: None,
                    revealed_spoilers: revealed.clone(),
                    text_selection: self.text_selection.clone(),
                    on_reveal: Rc::new(move |block_index, _window, cx| {
                        reveal_state.update(cx, |state, cx| {
                            if state.revealed.reveal(block_index, cx) {
                                cx.notify();
                            }
                        });
                    }),
                });
            }
            column = column.child(body);
        }
        height_transition(
            &state,
            RiseDocumentCollapsibleState::expansion,
            is_expanded,
            column.into_any_element(),
            window,
            cx,
        )
    }
}

#[derive(IntoElement)]
struct RiseDocumentTableView {
    id: ElementId,
    table: RiseDocumentTable,
    style: FormattedTextStyle,
    theme: DocumentColors,
    text_selection: Option<FormattedTextSelection>,
}

impl RenderOnce for RiseDocumentTableView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let mut rows = div().flex().flex_col().gap(px(1.0)).rounded(px(8.0)).overflow_hidden();
        for (row_index, row) in self.table.rows.iter().enumerate() {
            let mut cells = div().flex().flex_row().gap(px(1.0));
            for (cell_index, cell) in row.cells.iter().enumerate() {
                let weight = if self.table.has_header_row && row_index == 0 {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                };
                let plain_text = RiseDocumentProjection::plain_text(&RiseDocument::new(cell.blocks.clone()));
                let content = match &self.text_selection {
                    Some(selection) => div()
                        .swiftui_font(Font::system(self.style.font_size - 1.0, weight, FontDesign::Default))
                        .text_color(self.theme.text_primary)
                        .child(FormattedTextUi::selectable_body(
                            child_id(&self.id, format!("cell-{row_index}-{cell_index}")),
                            AttributedString::new(plain_text),
                            selection.clone(),
                        ))
                        .into_any_element(),
                    None => MainText::new(plain_text)
                        .px(self.style.font_size - 1.0)
                        .tac(TextAlignT::Left)
                        .font_weight(weight)
                        .color(self.theme.text_primary)
                        .into_any_element(),
                };
                cells = cells.child(
                    div()
                        .px(px(8.0))
                        .py(px(6.0))
                        .min_w(px(92.0))
                        .bg(self.theme.bg_300)
                        .child(content),
                );
            }
            rows = rows.child(cells);
        }
        div().id(self.id).overflow_x_scroll().child(rows)
    }
}
