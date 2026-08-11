use gpui::Pixels;

use crate::tokens::density::Density;

/// The pre-auth screens' geometry.
///
/// Onboarding, sign-in and sign-up own the whole window rather than a column, so
/// their layout is neither a pane nor an overlay.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AuthMetrics {
    /// How wide the form column is allowed to grow, whatever the window does.
    pub content_width: Pixels,
    /// Around the form column.
    pub content_padding: Pixels,
    /// The onboarding slide's animation square.
    pub slide_art_size: Pixels,
    /// Between the art and the slide's title.
    pub slide_art_gap: Pixels,
    /// Between the slide's title and its description.
    pub slide_text_gap: Pixels,
    /// A pagination dot, and the pill the current one becomes.
    pub pagination_dot_size: Pixels,
    pub pagination_dot_active_width: Pixels,
    pub pagination_gap: Pixels,
    /// Between the pagination row and the primary button.
    pub pagination_margin: Pixels,
    /// How far the skip control sits from the window's top-right corner.
    pub skip_inset: Pixels,

    /// ONE height for the step's field and for the button under it.
    pub field_height: Pixels,
    /// The wizard's animation, smaller than an onboarding slide's.
    pub step_art_size: Pixels,
    /// Between the wizard's animation and the step title.
    pub step_art_gap: Pixels,
    /// Between the step's subtitle and its field.
    pub step_field_gap: Pixels,
    /// The progress bar across the top of the wizard: total width, one segment's
    /// height, and the gap between segments.
    pub progress_width: Pixels,
    pub progress_height: Pixels,
    pub progress_gap: Pixels,
    /// The circular back button, and the band the header occupies.
    pub header_button_size: Pixels,
    pub header_height: Pixels,
    /// One cell of the verification-code field. The width is a maximum: cells
    /// share the row and shrink before they overflow.
    pub code_cell_width: Pixels,
    pub code_cell_height: Pixels,
    pub code_gap: Pixels,
    pub code_radius: Pixels,
    /// The digit itself.
    pub code_digit_size: f32,
    /// The active cell is marked by a thicker border and nothing else — no
    /// caret, no scale, no shake.
    pub code_active_border: Pixels,
    /// The flag on the country button and in its list.
    pub flag_width: Pixels,
}

impl AuthMetrics {
    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            content_width: l(500.0),
            content_padding: l(22.0),
            slide_art_size: l(260.0),
            slide_art_gap: l(40.0),
            slide_text_gap: l(16.0),
            pagination_dot_size: l(8.0),
            pagination_dot_active_width: l(24.0),
            pagination_gap: l(8.0),
            pagination_margin: l(20.0),
            skip_inset: l(20.0),
            field_height: l(56.0),
            step_art_size: l(168.0),
            step_art_gap: l(4.0),
            step_field_gap: l(26.0),
            progress_width: l(190.0),
            progress_height: l(4.0),
            progress_gap: l(5.0),
            header_button_size: l(40.0),
            header_height: l(54.0),
            code_cell_width: l(62.0),
            code_cell_height: l(58.0),
            code_gap: l(10.0),
            code_radius: l(16.0),
            code_digit_size: 23.0,
            code_active_border: l(1.5),
            flag_width: l(24.0),
        }
    }
}

impl Default for AuthMetrics {
    fn default() -> Self {
        Self::new(Density::NORMAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::shell::ShellMetrics;
    use gpui::px;

    #[test]
    fn the_defaults_are_the_references_own_numbers() {
        let metrics = AuthMetrics::default();
        assert_eq!(metrics.content_width, px(500.0));
        assert_eq!(metrics.content_padding, px(22.0));
        assert_eq!(metrics.step_art_size, px(168.0));
        assert_eq!(metrics.progress_width, px(190.0));
        assert_eq!(metrics.header_height, px(54.0));
        assert_eq!(metrics.pagination_dot_size, px(8.0));
        assert_eq!(metrics.pagination_dot_active_width, px(24.0));
    }

    /// The submit button is the same size as the field above it.
    #[test]
    fn the_field_and_the_button_under_it_are_one_height() {
        for density in [Density::COMPACT, Density::NORMAL, Density::COMFORTABLE] {
            let metrics = AuthMetrics::new(density);
            assert_eq!(metrics.field_height, AuthMetrics::new(density).field_height);
            assert!(metrics.field_height > px(0.0), "{density:?}");
        }
    }

    #[test]
    fn the_back_button_fits_inside_the_header_band() {
        for density in [Density::COMPACT, Density::NORMAL, Density::COMFORTABLE] {
            let metrics = AuthMetrics::new(density);
            assert!(
                metrics.header_button_size < metrics.header_height,
                "at {density:?} the back button is taller than the row it sits in"
            );
        }
    }

    #[test]
    fn density_scales_the_form_the_same_way_it_scales_the_shell() {
        let dense = AuthMetrics::new(Density::new(1.25));
        assert_eq!(dense.content_width, px(500.0 * 1.25));
        assert_eq!(dense.step_art_size, px(168.0 * 1.25));
    }

    #[test]
    fn the_current_pagination_dot_is_wider_than_the_others_and_the_same_height() {
        let metrics = AuthMetrics::default();
        assert!(metrics.pagination_dot_active_width > metrics.pagination_dot_size);
    }

    #[test]
    fn the_form_fits_the_narrowest_window_the_shell_will_open() {
        for density in [Density::COMPACT, Density::NORMAL, Density::COMFORTABLE] {
            let auth = AuthMetrics::new(density);
            let shell = ShellMetrics::new(density);

            assert!(
                auth.content_width + auth.content_padding * 2.0 <= shell.window_default_width,
                "the sign-in form at {density:?} is wider than a first-run window"
            );
            assert!(
                auth.slide_art_size + auth.slide_art_gap * 2.0 <= shell.window_default_height,
                "an onboarding slide at {density:?} cannot fit its own art"
            );
            assert!(
                auth.progress_width <= auth.content_width,
                "the progress bar at {density:?} is wider than the column it heads"
            );
        }
    }
}
