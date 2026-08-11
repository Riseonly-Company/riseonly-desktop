use gpui::{AnyElement, ClickEvent, ElementId, IntoElement, Window, div, prelude::*};
use rise_theme::AppTheme;

use crate::main_text::{MainText, TextTone};

/// A one-time code, drawn as one cell per digit.
///
/// The cells are decoration over ONE real text field held at 1% opacity; that
/// field owns the caret, the key handling and the paste. This draws only: the
/// caller owns the `InputUiState`, keeps it filtered to digits and capped at
/// [`CodeFieldUi::LENGTH`], and calls [`CodeFieldUi::is_complete`] to submit.
pub struct CodeFieldUi;

impl CodeFieldUi {
    /// Four, because that is what auth-service generates with `{:04}`.
    pub const LENGTH: usize = 4;

    /// Not zero: gpui may skip a fully transparent element, and the field has to
    /// stay in the tree to hold the caret.
    const FIELD_OPACITY: f32 = 0.01;

    pub fn is_complete(code: &str) -> bool {
        code.chars().count() == Self::LENGTH
    }

    /// The row of cells, with `field` — the caller's `Entity<InputUiState>` —
    /// laid over it invisibly. The cells are not focusable, so `on_focus` must
    /// focus that field.
    pub fn render(
        theme: &AppTheme,
        id: impl Into<ElementId>,
        code: &str,
        field: impl IntoElement,
        focused: bool,
        on_focus: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> AnyElement {
        let metrics = theme.auth;
        let digits: Vec<char> = code.chars().collect();
        // Past the end once the code is full, so no cell is then outlined.
        let active = focused.then_some(digits.len());

        let cells = (0..Self::LENGTH).map(|index| {
            let is_active = active == Some(index);
            let digit = digits.get(index).copied();

            div()
                .flex_1()
                .max_w(metrics.code_cell_width)
                .h(metrics.code_cell_height)
                .flex()
                .items_center()
                .justify_center()
                .rounded(metrics.code_radius)
                .bg(theme.bg._200)
                .border_1()
                .when(is_active, |cell| {
                    cell.border(metrics.code_active_border)
                        .border_color(theme.primary._100)
                })
                .when(!is_active, |cell| cell.border_color(theme.input.border_300))
                .children(digit.map(|digit| {
                    MainText::body(theme, TextTone::Primary)
                        .text_size(gpui::px(metrics.code_digit_size))
                        .font(theme.typography.font(gpui::FontWeight::SEMIBOLD))
                        .child(digit.to_string())
                }))
        });

        div()
            .id(id)
            .relative()
            .w_full()
            .h(metrics.code_cell_height)
            .cursor_pointer()
            .on_click(on_focus)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .gap(metrics.code_gap)
                    .w_full()
                    .children(cells),
            )
            // Over the cells, not under them: the caret lives in it.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(Self::FIELD_OPACITY)
                    .child(field),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completeness_is_the_servers_own_four() {
        assert!(!CodeFieldUi::is_complete(""));
        assert!(!CodeFieldUi::is_complete("123"));
        assert!(CodeFieldUi::is_complete("1234"));
        assert!(
            !CodeFieldUi::is_complete("12345"),
            "a longer value is the caller's filter failing, not a complete code"
        );
    }

    #[test]
    fn the_real_field_is_never_fully_transparent() {
        const { assert!(CodeFieldUi::FIELD_OPACITY > 0.0) };
        const { assert!(CodeFieldUi::FIELD_OPACITY < 0.1) };
    }
}
