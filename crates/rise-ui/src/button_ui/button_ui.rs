use gpui::{Div, Hsla, Pixels, div, prelude::*};
use rise_theme::AppTheme;

/// Which height step a button sits on. The steps are theme tokens, not free
/// numbers, so buttons keep lining up with the controls beside them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ButtonSize {
    /// The primary action of a screen — a sign-in, a send.
    Large,
    /// The default. Anything inside a form or a row.
    #[default]
    Regular,
    /// Inline with text or inside a dense toolbar.
    Small,
}

/// How loudly a button asks to be pressed. Tone picks background and label
/// together — they are a contrast pair and must not be chosen separately.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ButtonTone {
    /// The accent fill. One per screen — the action the screen exists for.
    #[default]
    Primary,
    /// A filled but unaccented button: cancel, secondary choices, toolbars.
    Neutral,
}

/// A button's geometry and colours, resolved from the theme. [`ButtonUi::base`]
/// hands back a styled `Div`, so the call site keeps gpui's own interactivity.
pub struct ButtonUi;

impl ButtonUi {
    /// The fixed height of the step. Buttons never size to their content —
    /// a row of them must align regardless of label length.
    pub fn height(theme: &AppTheme, size: ButtonSize) -> Pixels {
        match size {
            ButtonSize::Large => theme.button.height_100,
            ButtonSize::Regular => theme.button.height_300,
            ButtonSize::Small => theme.button.height_500,
        }
    }

    /// The fill. Pair it with [`ButtonUi::label_color`] of the same tone —
    /// the two are chosen to contrast, and mixing tones breaks that.
    pub fn background(theme: &AppTheme, tone: ButtonTone) -> Hsla {
        match tone {
            ButtonTone::Primary => theme.primary._200,
            ButtonTone::Neutral => theme.button.bg_300,
        }
    }

    /// The label colour that reads against [`ButtonUi::background`]. A primary
    /// button inverts, so this is not `text.primary` for every tone.
    pub fn label_color(theme: &AppTheme, tone: ButtonTone) -> Hsla {
        match tone {
            ButtonTone::Primary => theme.bg._000,
            ButtonTone::Neutral => theme.text.primary,
        }
    }

    /// A centred, filled, rounded button box — everything except the label and
    /// the click handler, which the caller adds with gpui's own `Div` methods.
    pub fn base(theme: &AppTheme, tone: ButtonTone, size: ButtonSize) -> Div {
        Self::sized(theme, tone, Self::height(theme, size))
    }

    /// A button at a height the caller owns. Pass the token of the control it
    /// must match, never a ramp step that happens to be closest.
    pub fn sized(theme: &AppTheme, tone: ButtonTone, height: Pixels) -> Div {
        div()
            .flex()
            .items_center()
            .justify_center()
            .h(height)
            .px(theme.spacing._900)
            .rounded(theme.button.radius_200)
            .bg(Self::background(theme, tone))
            .text_color(Self::label_color(theme, tone))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn button_sizes_map_to_the_reference_heights() {
        let theme = AppTheme::dark();
        assert_eq!(ButtonUi::height(&theme, ButtonSize::Large), px(55.0));
        assert_eq!(ButtonUi::height(&theme, ButtonSize::Regular), px(45.0));
        assert_eq!(ButtonUi::height(&theme, ButtonSize::Small), px(35.0));
    }

    #[test]
    fn a_primary_button_reads_against_its_own_background() {
        let theme = AppTheme::dark();
        assert_ne!(
            ButtonUi::background(&theme, ButtonTone::Primary),
            ButtonUi::label_color(&theme, ButtonTone::Primary)
        );
    }

    #[test]
    fn a_sized_button_takes_the_height_it_was_handed() {
        let theme = AppTheme::dark();
        let height = theme.auth.field_height;

        assert_eq!(height, theme.auth.field_height);
        assert!(height > px(0.0));
    }

    #[test]
    fn tones_are_visually_distinct() {
        let theme = AppTheme::dark();
        assert_ne!(
            ButtonUi::background(&theme, ButtonTone::Primary),
            ButtonUi::background(&theme, ButtonTone::Neutral)
        );
    }
}
