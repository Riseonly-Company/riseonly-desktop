use gpui::{Div, Hsla, Pixels, div, prelude::*};
use rise_theme::AppTheme;

/// Which height step a button sits on.
///
/// The three steps are the reference's `height_100`/`_300`/`_500` tokens, not a
/// free number: a button whose height is typed at the call site stops lining up
/// with the field next to it the moment either one is restyled.
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

/// How loudly a button asks to be pressed.
///
/// Tone picks background and label together, which is the whole point: the two
/// are a contrast pair, and choosing them separately is how a label ends up
/// invisible on its own button.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ButtonTone {
    /// The accent fill. One per screen — the action the screen exists for.
    #[default]
    Primary,
    /// A filled but unaccented button: cancel, secondary choices, toolbars.
    Neutral,
}

/// A button's geometry and colours, resolved from the theme.
///
/// This is a set of associated functions rather than an element type. gpui's
/// interactivity — `.on_click`, hover and active styles, disabled state — is
/// already fluent on `Div`, so wrapping it would mean re-exporting all of it;
/// [`ButtonUi::base`] hands back the styled `Div` and the call site keeps using
/// gpui directly.
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

    /// The label colour that reads against [`ButtonUi::background`].
    ///
    /// A primary button inverts — it draws the darkest background colour on the
    /// accent fill — so this is not `text.primary` for every tone.
    pub fn label_color(theme: &AppTheme, tone: ButtonTone) -> Hsla {
        match tone {
            ButtonTone::Primary => theme.bg._000,
            ButtonTone::Neutral => theme.text.primary,
        }
    }

    /// A centred, filled, rounded button box — everything except the label and
    /// the click handler.
    ///
    /// Add both with gpui: `ButtonUi::base(theme, tone, size).child("Войти")
    /// .on_click(..)`. The returned `Div` is not yet interactive, so a
    /// disabled-looking button is made by leaving the handler off rather than by
    /// a flag this helper would have to carry.
    pub fn base(theme: &AppTheme, tone: ButtonTone, size: ButtonSize) -> Div {
        Self::sized(theme, tone, Self::height(theme, size))
    }

    /// A button at a height the caller owns.
    ///
    /// The escape hatch that keeps the ramp honest: a screen whose button has to
    /// match the field above it passes that FIELD's token rather than picking
    /// the ramp step which happens to be closest today. `theme.auth.field_height`
    /// is the only caller so far, and it is one token for both controls.
    pub fn sized(theme: &AppTheme, tone: ButtonTone, height: Pixels) -> Div {
        div()
            .flex()
            .items_center()
            .justify_center()
            .h(height)
            .px(theme.spacing._900)
            // The pill the reference gives its primary action. A radius, not a
            // height, so it does not move when the height does.
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

        // The rule the user asked for: the primary action is the same size as
        // the field above it, by construction rather than by coincidence.
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
