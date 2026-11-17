use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, ElementId, FontWeight, Hsla, IntoElement, Pixels, RenderOnce, ScrollHandle, SharedString, Size, TextRun,
    Window, point, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::haptics::{NotificationKind, notification};
use rise_platform::pasteboard::UIPasteboard;
use rise_ui::IconUi;
use rise_ui::hold_context_menu_ui::bubble_scroll_passthrough::BubbleScrollPassthroughModifiers;
use rise_ui::rich_text::{CodePalette, SyntaxHighlighter};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::rich_text::{AttributedString, RichFont, RichText};

use crate::core::ui::formatted_text_ui::{FormattedTextSelection, FormattedTextUi};
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;

const CODE_LINE_SPACING: f32 = 3.0;
const CODE_HORIZONTAL_PADDING: f32 = 12.0;
const CODE_VERTICAL_PADDING: f32 = 10.0;
const CORNER_RADIUS: f32 = 12.0;
const COPIED_CONFIRMATION: Duration = Duration::from_millis(1600);

#[derive(IntoElement)]
pub struct CodeBlockView {
    id: ElementId,
    language: Option<String>,
    code: String,
    font_size: f32,
    palette: CodePalette,
    in_bubble: bool,
    text_selection: Option<FormattedTextSelection>,
}

/// copied and copy token state of one block, with the color they animate
struct CodeBlockViewState {
    copied: bool,
    copy_token: u64,
    label_color: Animated<Hsla>,
}

struct RenderedCode {
    attributed_text: AttributedString,
    text_size: Size<Pixels>,
    line_height: f32,
}

impl CodeBlockView {
    pub fn new(id: impl Into<ElementId>, language: Option<String>, code: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            language,
            code: code.into(),
            font_size: 13.5,
            palette: CodePalette::editor_dark(),
            in_bubble: false,
            text_selection: None,
        }
    }

    pub fn font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    pub fn palette(mut self, palette: CodePalette) -> Self {
        self.palette = palette;
        self
    }

    pub fn in_bubble(mut self, in_bubble: bool) -> Self {
        self.in_bubble = in_bubble;
        self
    }

    /// desktop mouse selection of the code, on the surface that owns the press
    pub fn text_selection(mut self, text_selection: Option<FormattedTextSelection>) -> Self {
        self.text_selection = text_selection;
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn normalized_code(&self) -> String {
        let lines: Vec<&str> = self.code.split('\n').collect();
        let mut lower_bound = 0;
        let mut upper_bound = lines.len();
        while lower_bound < upper_bound && trimming_whitespaces_and_newlines(lines[lower_bound]).is_empty() {
            lower_bound += 1;
        }
        while upper_bound > lower_bound && trimming_whitespaces_and_newlines(lines[upper_bound - 1]).is_empty() {
            upper_bound -= 1;
        }
        if lower_bound >= upper_bound {
            return String::new();
        }
        lines[lower_bound..upper_bound].join("\n")
    }

    fn rendered_code(&self, normalized_code: &str, font: RichFont, window: &Window) -> RenderedCode {
        let attributed_text = SyntaxHighlighter::highlight(normalized_code, self.language.as_deref(), font, &self.palette);
        let monospaced = Font::system(self.font_size, FontWeight::NORMAL, FontDesign::Monospaced);
        let ui_font = gpui::Font {
            family: monospaced.family(),
            features: monospaced.features(),
            fallbacks: None,
            weight: monospaced.weight,
            style: monospaced.style(),
        };
        let font_size = px(self.font_size);
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&ui_font);
        let ascent = f32::from(text_system.ascent(font_id, font_size));
        let descent = f32::from(text_system.descent(font_id, font_size));
        let line_height = ascent + descent.abs();
        let lines: Vec<&str> = normalized_code.split('\n').collect();
        let mut measured_width = 0.0_f32;
        for line in &lines {
            if line.is_empty() {
                continue;
            }
            let run = TextRun {
                len: line.len(),
                font: ui_font.clone(),
                color: self.palette.plain,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped = text_system.shape_line(SharedString::from((*line).to_owned()), font_size, &[run], None);
            measured_width = measured_width.max(f32::from(shaped.width()));
        }
        let line_count = lines.len().max(1) as f32;
        let measured_height = line_count * line_height + (line_count - 1.0) * CODE_LINE_SPACING;
        RenderedCode {
            attributed_text,
            text_size: size(
                px(measured_width.ceil().max(1.0)),
                px(line_height.ceil().max(measured_height.ceil())),
            ),
            line_height,
        }
    }

    fn header(&self, state: &gpui::Entity<CodeBlockViewState>, code: Rc<str>, label_color: Hsla, copied: bool) -> Div {
        let palette = self.palette;
        let mut row = hstack(8.0).child(div().children(IconUi::sized(
            "chevron.left.forwardslash.chevron.right",
            px(11.0),
            palette.language_label,
            false,
        )));
        if let Some(name) = SyntaxHighlighter::display_name(self.language.as_deref()) {
            row = row.child(
                div()
                    .swiftui_font(Font::system(11.5, FontWeight::SEMIBOLD, FontDesign::Monospaced))
                    .text_color(palette.language_label)
                    .line_clamp(1)
                    .text_ellipsis()
                    .child(SharedString::from(name)),
            );
        }
        row.child(spacer_min_w(8.0))
            .child(self.copy_button(state, code, label_color, copied))
            .padding_edges(EdgeSet::HORIZONTAL, 12.0)
            .padding_edges(EdgeSet::VERTICAL, 7.0)
    }

    fn copy_button(
        &self,
        state: &gpui::Entity<CodeBlockViewState>,
        code: Rc<str>,
        label_color: Hsla,
        copied: bool,
    ) -> AnyElement {
        let copy_action: Rc<dyn Fn(&mut Window, &mut App)> = {
            let state = state.clone();
            Rc::new(move |_window: &mut Window, cx: &mut App| copy(&state, &code, cx))
        };
        let label = hstack(4.0)
            .text_color(label_color)
            .child(div().children(IconUi::sized("doc.on.doc", px(11.0), label_color, false)))
            .child(
                div()
                    .swiftui_font(Font::system_weight(11.0, FontWeight::SEMIBOLD))
                    .child(SharedString::from(if copied { tr("copied") } else { tr("copy") })),
            );
        let action = copy_action.clone();
        let button = plain(self.child_id("copy")).on_press(move |window, cx| action(window, cx)).child(label);
        if self.in_bubble {
            button
                .bubble_tap_target(self.child_id("copy-tap"), move |window: &mut Window, cx: &mut App| {
                    copy_action(window, cx)
                })
                .into_any_element()
        } else {
            button.into_any_element()
        }
    }

    fn code_scroll(&self, rendered_code: RenderedCode) -> CodeBlockHorizontalScrollView {
        CodeBlockHorizontalScrollView {
            id: self.child_id("code-scroll"),
            attributed_text: rendered_code.attributed_text,
            text_size: rendered_code.text_size,
            line_height: rendered_code.line_height,
            font_size: self.font_size,
            plain: self.palette.plain,
            horizontal_padding: CODE_HORIZONTAL_PADDING,
            vertical_padding: CODE_VERTICAL_PADDING,
            registers_bubble_passthrough: self.in_bubble,
            text_selection: self.text_selection.clone(),
        }
    }
}

fn copy(state: &gpui::Entity<CodeBlockViewState>, code: &str, cx: &mut App) {
    UIPasteboard::general().set_string(code, cx);
    let _ = notification(NotificationKind::Success);
    let token = state.update(cx, |state, cx| {
        state.copy_token = state.copy_token.wrapping_add(1);
        state.copied = true;
        cx.notify();
        state.copy_token
    });
    let weak = state.downgrade();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(COPIED_CONFIRMATION).await;
        let _ = weak.update(cx, |state, cx| {
            if token != state.copy_token {
                return;
            }
            state.copied = false;
            cx.notify();
        });
    })
    .detach();
}

impl RenderOnce for CodeBlockView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = self.palette;
        let normalized_code = self.normalized_code();
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| CodeBlockViewState {
            copied: false,
            copy_token: 0,
            label_color: Animated::new(palette.language_label),
        });

        {
            let state = state.clone();
            on_change(self.child_id("code-change"), normalized_code.clone(), false, window, cx, move |_, _, _, cx| {
                state.update(cx, |state, cx| {
                    state.copy_token = state.copy_token.wrapping_add(1);
                    state.copied = false;
                    state.label_color.jump(palette.language_label);
                    cx.notify();
                });
            });
        }

        let now = RiseClock::monotonic();
        let copied = state.read(cx).copied;
        let target = if copied { palette.string } else { palette.language_label };
        let animation = if copied { ease_out(0.15) } else { ease_out(0.2) };
        state.update(cx, |state, cx| state.label_color.set_in(target, animation, now, cx));
        let label_color = state.read(cx).label_color.frame_value(now, window);

        let font = RichFont::system_design(self.font_size, FontDesign::Monospaced);
        let rendered_code = self.rendered_code(&normalized_code, font, window);
        let code: Rc<str> = Rc::from(normalized_code.as_str());

        let card = vstack(0.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .bg(palette.background)
            .clip_shape(Shape::continuous(CORNER_RADIUS))
            .child(self.header(&state, code, label_color, copied))
            .child(div().w_full().h(px(1.0)).bg(palette.chrome))
            .child(self.code_scroll(rendered_code));

        div()
            .relative()
            .w_full()
            .child(card)
            .child(centred_stroke(palette.chrome, 1.0, Shape::continuous(CORNER_RADIUS)))
    }
}

#[derive(IntoElement)]
struct CodeBlockHorizontalScrollView {
    id: ElementId,
    attributed_text: AttributedString,
    text_size: Size<Pixels>,
    line_height: f32,
    font_size: f32,
    plain: Hsla,
    horizontal_padding: f32,
    vertical_padding: f32,
    registers_bubble_passthrough: bool,
    text_selection: Option<FormattedTextSelection>,
}

struct CodeBlockHorizontalScrollContainer {
    scroll: ScrollHandle,
    text: String,
}

impl CodeBlockHorizontalScrollContainer {
    fn configure(&mut self, attributed_text: &AttributedString) {
        if self.text != attributed_text.characters() {
            let offset = self.scroll.offset();
            self.scroll.set_offset(point(px(0.0), offset.y));
            self.text = attributed_text.characters().to_owned();
        }
    }
}

impl RenderOnce for CodeBlockHorizontalScrollView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let container_id = ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("container"));
        let container = window.use_keyed_state(container_id, cx, |_, _| CodeBlockHorizontalScrollContainer {
            scroll: ScrollHandle::new(),
            text: String::new(),
        });
        let scroll = container.update(cx, |container, _| {
            container.configure(&self.attributed_text);
            container.scroll.clone()
        });

        let label_id = ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("label"));
        let label_body = match self.text_selection {
            Some(selection) => FormattedTextUi::selectable_body(label_id, self.attributed_text, selection),
            None => RichText::new(label_id, self.attributed_text).into_any_element(),
        };
        let label = div()
            .flex_none()
            .w(self.text_size.width)
            .mt(px(self.vertical_padding - CODE_LINE_SPACING / 2.0))
            .whitespace_nowrap()
            .swiftui_font(Font::system(self.font_size, FontWeight::NORMAL, FontDesign::Monospaced))
            .line_height(px(self.line_height + CODE_LINE_SPACING))
            .text_color(self.plain)
            .child(label_body);
        let content = div().flex_none().flex().flex_row().items_start().px(px(self.horizontal_padding)).child(label);
        let height = (f32::from(self.text_size.height) + self.vertical_padding * 2.0).ceil();
        let overflows = scroll.max_offset().x > px(1.0);
        let passthrough_id = ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("passthrough"));
        let scroll_view = div()
            .id(self.id)
            .w_full()
            .h(px(height))
            .flex()
            .flex_row()
            .items_start()
            .overflow_hidden()
            .overflow_x_scroll()
            .track_scroll(&scroll)
            .child(content);
        if self.registers_bubble_passthrough && overflows {
            scroll_view
                .bubble_scroll_passthrough(passthrough_id)
                .into_any_element()
        } else {
            scroll_view.into_any_element()
        }
    }
}
