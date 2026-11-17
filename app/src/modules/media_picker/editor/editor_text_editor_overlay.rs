use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext, Bounds, ClickEvent, Context, Entity, Focusable, FontWeight, Hsla,
    InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, RenderOnce, SharedString, StatefulInteractiveElement,
    Styled, Subscription, TextAlign, Window, div, hsla, px, rgb,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_ui::icon_ui::IconUi;
use rise_ui::input_ui::input_ui_state::{InputUiEvent, InputUiState};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::swiftui::angular_gradient::{AngularGradient, angular_gradient_circle_stroke};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::button_chrome;
use rise_ui::swiftui::font::{Font, SwiftUiFont};
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke};
use rise_ui::swiftui::stacks::{hstack, spacer, spacer_min_h, vstack};

use crate::modules::media_picker::editor::editor_color_picker_bridge::EditorColorPickerLauncher;
use crate::modules::media_picker::editor::editor_state::{
    EditorTextEntity, EditorTextEntityStyleMode, EditorTextEntityTextAlignment,
};
use crate::modules::media_picker::editor::editor_text_entity_view::{
    EditorTextEntityView, editor_semibold_text_size,
};

type EditorTextEditorFinish = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct EditorTextEditorOverlay {
    draft: Binding<EditorTextEntity>,
    on_cancel: Rc<dyn Fn(&mut Window, &mut App)>,
    on_commit: Rc<dyn Fn(Option<EditorTextEntity>, &mut Window, &mut App)>,
    image_rect: Bounds<f32>,
    top_inset: f32,
}

impl EditorTextEditorOverlay {
    const BASE_FONT_SIZE: f32 = 28.0;
    const TOOL_HIT_SIDE: f32 = 40.0;

    pub fn new(
        draft: Binding<EditorTextEntity>,
        on_cancel: impl Fn(&mut Window, &mut App) + 'static,
        on_commit: impl Fn(Option<EditorTextEntity>, &mut Window, &mut App) + 'static,
        image_rect: Bounds<f32>,
        top_inset: f32,
    ) -> Self {
        Self {
            draft,
            on_cancel: Rc::new(on_cancel),
            on_commit: Rc::new(on_commit),
            image_rect,
            top_inset,
        }
    }

    fn top_bar(&self, finish: EditorTextEditorFinish) -> impl IntoElement {
        let on_cancel = self.on_cancel.clone();
        hstack(None)
            .w_full()
            .px(px(16.0))
            .pt(px(self.top_inset + 6.0))
            .child(
                PageHeaderIconButton::new("editor-text-cancel", "xmark", move |window, cx| {
                    on_cancel(window, cx);
                })
                .font_size(15.0),
            )
            .child(spacer())
            .child(
                PageHeaderIconButton::new("editor-text-done", "checkmark", move |window, cx| {
                    finish(window, cx);
                })
                .font_size(17.0),
            )
    }

    fn toolbar(
        &self,
        draft: &EditorTextEntity,
        input: &Entity<InputUiState>,
    ) -> impl IntoElement + use<> {
        let side = px(Self::TOOL_HIT_SIDE);

        let color_binding = self.draft.clone();
        let done_binding = self.draft.clone();
        let refocus_input = input.clone();
        let initial = draft.color;
        let color_button = button_chrome(
            div()
                .id("editor-text-color")
                .w(side)
                .h(side)
                .flex()
                .items_center()
                .justify_center()
                .occlude()
                .child(Self::rainbow_dot(draft.color)),
            false,
            None,
        )
            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let live_binding = color_binding.clone();
                let done_binding = done_binding.clone();
                let refocus_input = refocus_input.clone();
                EditorColorPickerLauncher::present(
                    initial,
                    true,
                    move |color, window, cx| {
                        let mut draft = live_binding.get(cx);
                        draft.color = color;
                        live_binding.set(draft, window, cx);
                    },
                    move |color, window, cx| {
                        let mut draft = done_binding.get(cx);
                        draft.color = color;
                        done_binding.set(draft, window, cx);
                        Self::refocus_first_inline_text_view(&refocus_input, window, cx);
                    },
                    window,
                    cx,
                );
            });

        let style_binding = self.draft.clone();
        let style_button = button_chrome(
            div()
                .id("editor-text-style")
                .w(side)
                .h(side)
                .flex()
                .items_center()
                .justify_center()
                .occlude()
                .child(
                    div()
                        .w(px(26.0))
                        .h(px(26.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .continuous_corners(6.0)
                        .bg(hsla(0.0, 0.0, 1.0, 1.0))
                        .swiftui_font(Font::system_weight(15.0, FontWeight::EXTRA_BOLD))
                        .text_color(hsla(0.0, 0.0, 0.0, 1.0))
                        .child("A"),
                ),
            false,
            None,
        )
            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let mut draft = style_binding.get(cx);
                draft.style_mode = Self::next_style(draft.style_mode);
                style_binding.set(draft, window, cx);
            });

        let alignment_binding = self.draft.clone();
        let alignment_button = button_chrome(
            div()
                .id("editor-text-alignment")
                .w(side)
                .h(side)
                .flex()
                .items_center()
                .justify_center()
                .occlude()
                .children(Self::alignment_icon(draft.alignment)),
            false,
            None,
        )
            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let mut draft = alignment_binding.get(cx);
                draft.alignment = Self::next_alignment(draft.alignment);
                alignment_binding.set(draft, window, cx);
            });

        hstack(4.0)
            .w_full()
            .px(px(8.0))
            .child(color_button)
            .child(style_button)
            .child(alignment_button)
            .child(spacer())
    }

    fn alignment_icon(alignment: EditorTextEntityTextAlignment) -> Option<impl IntoElement> {
        let name = match alignment {
            EditorTextEntityTextAlignment::Leading => "text.alignleft",
            EditorTextEntityTextAlignment::Center => "text.aligncenter",
            EditorTextEntityTextAlignment::Trailing => "text.alignright",
        };
        IconUi::sized(name, px(20.0), hsla(0.0, 0.0, 1.0, 1.0), false)
    }

    fn rainbow_dot(color: Hsla) -> AnyElement {
        let colors: [Hsla; 8] = [
            rgb(0xFF3B30).into(),
            rgb(0xFF9500).into(),
            rgb(0xFFCC00).into(),
            rgb(0x34C759).into(),
            rgb(0x32ADE6).into(),
            rgb(0x007AFF).into(),
            rgb(0xAF52DE).into(),
            rgb(0xFF3B30).into(),
        ];
        const SIDE: f32 = 24.0;
        div()
            .relative()
            .w(px(SIDE))
            .h(px(SIDE))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .child(angular_gradient_circle_stroke(
                        AngularGradient::new(colors),
                        SIDE / 2.0,
                        SIDE / 2.0,
                    ))
                    .pl(px(SIDE / 4.0))
                    .pt(px(SIDE / 4.0)),
            )
            .child(div().w(px(10.0)).h(px(10.0)).circle().bg(color))
            .child(centred_stroke(hsla(0.0, 0.0, 1.0, 0.7), 0.5, Shape::Circle))
            .into_any_element()
    }

    fn next_style(style: EditorTextEntityStyleMode) -> EditorTextEntityStyleMode {
        match style {
            EditorTextEntityStyleMode::Background => EditorTextEntityStyleMode::TranslucentBg,
            EditorTextEntityStyleMode::TranslucentBg => EditorTextEntityStyleMode::TextOnly,
            EditorTextEntityStyleMode::TextOnly => EditorTextEntityStyleMode::Background,
        }
    }

    fn next_alignment(alignment: EditorTextEntityTextAlignment) -> EditorTextEntityTextAlignment {
        match alignment {
            EditorTextEntityTextAlignment::Leading => EditorTextEntityTextAlignment::Center,
            EditorTextEntityTextAlignment::Center => EditorTextEntityTextAlignment::Trailing,
            EditorTextEntityTextAlignment::Trailing => EditorTextEntityTextAlignment::Leading,
        }
    }

    fn finish(
        draft: &Binding<EditorTextEntity>,
        on_commit: &Rc<dyn Fn(Option<EditorTextEntity>, &mut Window, &mut App)>,
        image_rect: Bounds<f32>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut final_entity = draft.get(cx);
        if trimming_whitespaces_and_newlines(&final_entity.text).is_empty() {
            on_commit(None, window, cx);
            return;
        }
        let canvas_short_side = image_rect.size.width.min(image_rect.size.height).max(1.0);
        final_entity.font_size = Self::BASE_FONT_SIZE / canvas_short_side;
        on_commit(Some(final_entity), window, cx);
    }

    fn refocus_first_inline_text_view(input: &Entity<InputUiState>, window: &mut Window, cx: &mut App) {
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }
}

impl RenderOnce for EditorTextEditorOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let draft = self.draft.get(cx);
        let screen_width = f32::from(window.viewport_size().width);

        let finish: EditorTextEditorFinish = {
            let binding = self.draft.clone();
            let on_commit = self.on_commit.clone();
            let image_rect = self.image_rect;
            Rc::new(move |window: &mut Window, cx: &mut App| {
                Self::finish(&binding, &on_commit, image_rect, window, cx);
            })
        };

        let inline = EditorInlineTextView {
            text: draft.text.clone(),
            draft: self.draft.clone(),
            placeholder: tr("editor_text_placeholder").into(),
            color: draft.color,
            style_mode: draft.style_mode,
            alignment: draft.alignment,
            font_size: Self::BASE_FONT_SIZE,
            max_width: screen_width * 0.8,
            screen_width,
            on_finish: finish.clone(),
        };
        let input = inline.input_state(draft.id, window, cx);

        let tap_finish = finish.clone();
        let key_finish = finish.clone();
        let backdrop = div()
            .id("editor-text-backdrop")
            .absolute()
            .inset_0()
            .bg(hsla(0.0, 0.0, 0.0, 0.32))
            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                tap_finish(window, cx);
            });

        div()
            .absolute()
            .inset_0()
            .on_key_down(move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                let keystroke = &event.keystroke;
                if keystroke.key == "enter" && keystroke.modifiers.secondary() {
                    cx.stop_propagation();
                    key_finish(window, cx);
                }
            })
            .child(backdrop)
            .child(
                vstack(0.0)
                    .size_full()
                    .child(self.top_bar(finish))
                    .child(spacer_min_h(0.0))
                    .child(div().px(px(16.0)).child(div().occlude().child(inline.render_field(&input, window, cx))))
                    .child(spacer_min_h(0.0))
                    .child(self.toolbar(&draft, &input))
                    .child(div().h(px(6.0))),
            )
    }
}

pub struct EditorInlineTextView {
    text: String,
    draft: Binding<EditorTextEntity>,
    placeholder: SharedString,
    color: Hsla,
    style_mode: EditorTextEntityStyleMode,
    alignment: EditorTextEntityTextAlignment,
    font_size: f32,
    max_width: f32,
    screen_width: f32,
    on_finish: EditorTextEditorFinish,
}

/// coordinator for the inline text view
/// holds the bindings of the latest frame since the subscription outlives them
struct EditorInlineTextViewCoordinator {
    input: Entity<InputUiState>,
    latest: Rc<RefCell<(Binding<EditorTextEntity>, EditorTextEditorFinish)>>,
    _subscription: Subscription,
}

impl EditorInlineTextView {
    fn input_state(
        &self,
        entity_id: rise_core::foundation::foundation_uuid::Uuid,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<InputUiState> {
        let key = SharedString::from(format!("editor-inline-text-{entity_id:?}"));
        let text = self.text.clone();
        let font_size = self.font_size;
        let draft = self.draft.clone();
        let on_finish = self.on_finish.clone();
        let coordinator = window.use_keyed_state(key, cx, move |window: &mut Window, cx: &mut Context<EditorInlineTextViewCoordinator>| {
            let input = cx.new(|cx| {
                let mut field = InputUiState::multi_line(cx);
                field.set_chromeless(true, cx);
                field.set_escape_to_layer(false);
                field.set_scroll_enabled(false, cx);
                field.set_font_size(Some(px(font_size)), cx);
                field.set_font_weight(Some(FontWeight::SEMIBOLD), cx);
                field.set_placeholder_color(Some(hsla(0.0, 0.0, 1.0, 0.55)), cx);
                field.reset(text, cx);
                field
            });
            let latest = Rc::new(RefCell::new((draft, on_finish)));
            let events = latest.clone();
            let subscription = window.subscribe(
                &input,
                cx,
                move |input: Entity<InputUiState>, event: &InputUiEvent, window, cx| match event {
                    InputUiEvent::Changed => {
                        let binding = events.borrow().0.clone();
                        let text = input.read(cx).text().to_owned();
                        let mut draft = binding.get(cx);
                        if draft.text != text {
                            draft.text = text;
                            binding.set(draft, window, cx);
                        }
                    }
                    InputUiEvent::Cancelled => {
                        let finish = events.borrow().1.clone();
                        finish(window, cx);
                    }
                    InputUiEvent::Submitted => {}
                },
            );
            let handle = input.read(cx).focus_handle(cx);
            window.defer(cx, move |window, cx| window.focus(&handle, cx));
            EditorInlineTextViewCoordinator { input, latest, _subscription: subscription }
        });
        let coordinator = coordinator.read(cx);
        *coordinator.latest.borrow_mut() = (self.draft.clone(), self.on_finish.clone());
        coordinator.input.clone()
    }

    fn apply_style(&self, input: &Entity<InputUiState>, cx: &mut App) {
        let (_, text_color, _) = self.style_values();
        let align = match self.alignment {
            EditorTextEntityTextAlignment::Leading => TextAlign::Left,
            EditorTextEntityTextAlignment::Center => TextAlign::Center,
            EditorTextEntityTextAlignment::Trailing => TextAlign::Right,
        };
        let placeholder = self.placeholder.clone();
        let text = self.text.clone();
        input.update(cx, |field, cx| {
            if field.text() != text {
                field.set_text(&text, cx);
            }
            if *field.placeholder() != placeholder {
                field.set_placeholder(placeholder, cx);
            }
            field.set_text_color(Some(text_color), cx);
            field.set_text_align(align, cx);
        });
    }

    /// background, text color and corner radius of the style mode
    fn style_values(&self) -> (Option<Hsla>, Hsla, f32) {
        match self.style_mode {
            EditorTextEntityStyleMode::Background => {
                (Some(self.color), Self::best_foreground_color(self.color), 12.0)
            }
            EditorTextEntityStyleMode::TranslucentBg => {
                (Some(hsla(0.0, 0.0, 1.0, 0.5)), self.color, 12.0)
            }
            EditorTextEntityStyleMode::TextOnly => (None, self.color, 0.0),
        }
    }

    fn best_foreground_color(color: Hsla) -> Hsla {
        EditorTextEntityView::best_foreground_color(color)
    }

    fn intrinsic_width(&self, window: &Window, cx: &App) -> f32 {
        const HORIZONTAL_INSETS: f32 = 24.0;
        let width_cap = self.max_width.min(self.screen_width - 32.0);
        let measure = |text: &str, wrap_width: Option<f32>| {
            editor_semibold_text_size(SharedString::from(text.to_owned()), self.font_size, wrap_width, window)
        };
        let single = measure(&self.text, None).width + HORIZONTAL_INSETS;
        let use_width = width_cap.min(single.max(80.0));
        let measured = measure(&self.text, Some((use_width - HORIZONTAL_INSETS).max(1.0))).width
            + HORIZONTAL_INSETS;
        let min_width = if self.text.is_empty() {
            (measure(&self.placeholder, None).width + HORIZONTAL_INSETS).max(80.0)
        } else {
            measured
        };
        let caret_width = f32::from(rise_ui::theme(cx).input.caret_width);
        (width_cap.min(min_width.max(measured)) + caret_width).ceil()
    }

    fn render_field(
        &self,
        input: &Entity<InputUiState>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement + use<> {
        self.apply_style(input, cx);
        let (background, _, corner_radius) = self.style_values();
        let width = self.intrinsic_width(window, cx);
        let mut field = div()
            .w(px(width))
            .pt(px(6.0))
            .pb(px(6.0))
            .pl(px(12.0))
            .pr(px(12.0))
            .rounded(px(corner_radius))
            .overflow_hidden()
            .child(input.clone());
        if let Some(background) = background {
            field = field.bg(background);
        }
        field
    }
}
