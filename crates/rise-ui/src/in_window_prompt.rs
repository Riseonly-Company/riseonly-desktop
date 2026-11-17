use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext as _, Context, EventEmitter, FocusHandle, Focusable, Hsla, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, PromptButton, PromptHandle, PromptLevel, PromptResponse, Render,
    RenderablePromptHandle, SharedString, StatefulInteractiveElement, Styled, TextRun, Window, div, px,
};
use rise_platform::HostOs;
use rise_theme::ThemeStore;

use crate::bottom_sheet_ui::bottom_sheet_backdrop::BottomSheetBackdrop;
use crate::bottom_sheet_ui::modal_sheet_surface::modal_sheet_centering;
use crate::error_text_ui::system_red;

const ESCAPE_KEY: &str = "escape";
const ENTER_KEY: &str = "enter";
const CARD_WIDTH: f32 = 320.0;
const CARD_RADIUS: f32 = 14.0;
const CARD_PADDING: f32 = 20.0;
const BUTTON_HEIGHT: f32 = 36.0;
const BUTTON_RADIUS: f32 = 9.0;
const BUTTON_PADDING_X: f32 = 12.0;
const BUTTON_GAP: f32 = 8.0;
const BUTTON_TEXT_SIZE: f32 = 14.0;
const MAX_ROW_ANSWERS: usize = 2;

/// true when window.prompt on this host draws the dialog inside the window
pub const fn uses_in_window_prompt(host: HostOs) -> bool {
    matches!(host, HostOs::MacOs | HostOs::Windows | HostOs::Linux)
}

pub fn install_in_window_prompt(cx: &mut App) {
    if uses_in_window_prompt(HostOs::current()) {
        cx.set_prompt_builder(in_window_prompt_renderer);
    }
}

/// answer a key press picks, Escape the first cancel button
/// enter the first Ok button or the first button if none
pub fn answer_for_key(key: &str, answers: &[PromptButton]) -> Option<usize> {
    match key {
        ESCAPE_KEY => answers.iter().position(PromptButton::is_cancel),
        ENTER_KEY => answers
            .iter()
            .position(|answer| matches!(answer, PromptButton::Ok(_)))
            .or_else(|| (!answers.is_empty()).then_some(0)),
        _ => None,
    }
}

/// true when the button at index is drawn filled, the one Enter answers
pub fn is_default_answer(index: usize, answers: &[PromptButton]) -> bool {
    answer_for_key(ENTER_KEY, answers) == Some(index)
}

/// true when answers sit side by side, two at most and only if every label fits its half
/// else they stack full width instead of clipping
pub fn answers_fit_in_row(label_widths: &[f32]) -> bool {
    let count = label_widths.len();
    if count == 0 || count > MAX_ROW_ANSWERS {
        return count == 0;
    }
    let row_width = CARD_WIDTH - CARD_PADDING * 2.0;
    let share = (row_width - BUTTON_GAP * (count as f32 - 1.0)) / count as f32;
    label_widths.iter().all(|width| width + BUTTON_PADDING_X * 2.0 <= share)
}

fn label_width(label: &SharedString, window: &Window) -> f32 {
    if label.is_empty() {
        return 0.0;
    }
    let run = TextRun {
        len: label.len(),
        font: window.text_style().font(),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    f32::from(window.text_system().shape_line(label.clone(), px(BUTTON_TEXT_SIZE), &[run], None).width()).ceil()
}

/// gpui prompt builder that draws the in window prompt
pub fn in_window_prompt_renderer(
    level: PromptLevel,
    message: &str,
    detail: Option<&str>,
    answers: &[PromptButton],
    handle: PromptHandle,
    window: &mut Window,
    cx: &mut App,
) -> RenderablePromptHandle {
    let prompt = cx.new(|cx| InWindowPrompt {
        level,
        message: SharedString::from(message.to_owned()),
        detail: detail.map(|detail| SharedString::from(detail.to_owned())),
        answers: answers.to_vec(),
        focus: cx.focus_handle(),
    });
    handle.with_view(prompt, window, cx)
}

pub struct InWindowPrompt {
    level: PromptLevel,
    message: SharedString,
    detail: Option<SharedString>,
    answers: Vec<PromptButton>,
    focus: FocusHandle,
}

impl InWindowPrompt {
    fn answer(&mut self, index: usize, cx: &mut Context<Self>) {
        cx.emit(PromptResponse(index));
        cx.stop_propagation();
    }
}

impl EventEmitter<PromptResponse> for InWindowPrompt {}

impl Focusable for InWindowPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for InWindowPrompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let red = system_red(cx);
        let (card_bg, border, text, secondary, accent, ink_on_accent, ink_on_red) = {
            let store = ThemeStore::shared(cx);
            let theme = store.read(cx).current_theme();
            (
                theme.bg._200,
                theme.border._200,
                theme.text.primary,
                theme.text.secondary,
                theme.primary._100,
                theme.content_on(theme.primary._100),
                theme.content_on(red),
            )
        };
        let critical = matches!(self.level, PromptLevel::Critical);
        let answers = self.answers.clone();
        let label_widths: Vec<f32> = answers.iter().map(|answer| label_width(answer.label(), window)).collect();
        let in_row = answers_fit_in_row(&label_widths);
        let cancel_index = answer_for_key(ESCAPE_KEY, &answers);
        let this = cx.entity().downgrade();

        let buttons = answers.iter().enumerate().map(|(index, answer)| {
            let filled = is_default_answer(index, &answers);
            let (fill, ink): (Option<Hsla>, Hsla) = match (filled, critical) {
                (true, true) => (Some(red), ink_on_red),
                (true, false) => (Some(accent), ink_on_accent),
                (false, _) => (None, text),
            };
            div()
                .id(("in-window-prompt-answer", index))
                .when(in_row, |button| button.flex_1().min_w_0())
                .when(!in_row, |button| button.w_full())
                .h(px(BUTTON_HEIGHT))
                .px(px(BUTTON_PADDING_X))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(BUTTON_RADIUS))
                .border_1()
                .border_color(border)
                .when_some(fill, |button, fill| button.bg(fill).border_color(fill))
                .text_size(px(BUTTON_TEXT_SIZE))
                .text_color(ink)
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .cursor_pointer()
                .child(answer.label().clone())
                .on_click(cx.listener(move |prompt, _, _, cx| prompt.answer(index, cx)))
        });

        let card = div()
            .id("in-window-prompt-card")
            .w(px(CARD_WIDTH))
            .p(px(CARD_PADDING))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .bg(card_bg)
            .border_1()
            .border_color(border)
            .rounded(px(CARD_RADIUS))
            .shadow_lg()
            .cursor_default()
            .occlude()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(text)
                    .child(self.message.clone()),
            )
            .when_some(self.detail.clone(), |card, detail| {
                card.child(div().text_size(px(13.0)).text_color(secondary).child(detail))
            })
            .child(
                div()
                    .mt(px(8.0))
                    .flex()
                    .when(in_row, |row| row.flex_row())
                    .when(!in_row, |column| column.flex_col())
                    .gap(px(BUTTON_GAP))
                    .children(buttons),
            );

        div()
            .id("in-window-prompt")
            .track_focus(&self.focus)
            .size_full()
            .absolute()
            .top_0()
            .left_0()
            .occlude()
            .on_key_down(cx.listener(move |prompt, event: &KeyDownEvent, _, cx| {
                if let Some(index) = answer_for_key(event.keystroke.key.as_str(), &answers) {
                    cx.stop_propagation();
                    prompt.answer(index, cx);
                }
            }))
            .child(BottomSheetBackdrop::new(true, move |_, _, cx| {
                if let Some(index) = cancel_index {
                    let _ = this.update(cx, |prompt, cx| prompt.answer(index, cx));
                }
            }))
            .child(modal_sheet_centering().child(card))
    }
}
