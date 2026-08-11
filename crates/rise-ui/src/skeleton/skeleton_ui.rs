use gpui::{
    AnimationExt, Div, ElementId, Hsla, IntoElement, Pixels, div, linear_color_stop,
    linear_gradient, prelude::*, relative,
};
use rise_theme::AppTheme;

use crate::animation::{ShimmerBand, shimmer_animation, shimmer_band};

/// The geometry a placeholder stands in for. It must match the real content, or
/// the layout jumps when the data arrives.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SkeletonShape {
    /// A run of text. The height is a line height, not a guess.
    Line { width: Pixels, height: Pixels },
    /// An avatar.
    Circle { diameter: Pixels },
    /// A thumbnail, a card, a button.
    Rect {
        width: Pixels,
        height: Pixels,
        radius: Pixels,
    },
}

pub struct SkeletonUi;

impl SkeletonUi {
    /// One container, one animation, however many shapes. Never drive a shape
    /// on its own: gpui has no damage regions, so each animation repaints the
    /// whole window.
    pub fn group<E>(
        id: impl Into<ElementId>,
        build: impl Fn(ShimmerBand) -> E + 'static,
    ) -> impl IntoElement
    where
        E: IntoElement + 'static,
    {
        div().child(build(shimmer_band(0.0))).with_animation(
            id,
            shimmer_animation(),
            move |_, progress| div().child(build(shimmer_band(progress))),
        )
    }

    /// Width, height and corner radius, so the geometry a screen will actually
    /// occupy can be asserted without a window.
    pub fn geometry(_theme: &AppTheme, shape: SkeletonShape) -> (Pixels, Pixels, Pixels) {
        match shape {
            SkeletonShape::Line { width, height } => (width, height, height / 2.0),
            SkeletonShape::Circle { diameter } => (diameter, diameter, diameter / 2.0),
            SkeletonShape::Rect {
                width,
                height,
                radius,
            } => (width, height, radius),
        }
    }

    pub fn shape(theme: &AppTheme, band: ShimmerBand, shape: SkeletonShape) -> Div {
        let (width, height, radius) = Self::geometry(theme, shape);

        div()
            .w(width)
            .h(height)
            .rounded(radius)
            .bg(Self::base(theme))
            .overflow_hidden()
            .child(Self::band(theme, band))
    }

    pub fn line(theme: &AppTheme, band: ShimmerBand, width: Pixels) -> Div {
        Self::shape(
            theme,
            band,
            SkeletonShape::Line {
                width,
                height: theme.typography.body().line_height,
            },
        )
    }

    pub fn circle(theme: &AppTheme, band: ShimmerBand, diameter: Pixels) -> Div {
        Self::shape(theme, band, SkeletonShape::Circle { diameter })
    }

    fn base(theme: &AppTheme) -> Hsla {
        theme.bg._300
    }

    fn highlight(theme: &AppTheme) -> Hsla {
        theme.bg._500
    }

    /// Two halves: `gpui::linear_gradient` carries exactly two colour stops, so
    /// a band that fades in AND out cannot be one element.
    fn band(theme: &AppTheme, band: ShimmerBand) -> Div {
        let highlight = Self::highlight(theme);
        let clear = rise_theme::alpha(highlight, 0.0);
        let half = band.width() / 2.0;

        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(relative(band.leading))
            .w(relative(band.width()))
            .flex()
            .flex_row()
            .child(
                div()
                    .w(relative(half / band.width()))
                    .h_full()
                    .bg(linear_gradient(
                        90.0,
                        linear_color_stop(clear, 0.0),
                        linear_color_stop(highlight, 1.0),
                    )),
            )
            .child(
                div()
                    .w(relative(half / band.width()))
                    .h_full()
                    .bg(linear_gradient(
                        90.0,
                        linear_color_stop(highlight, 0.0),
                        linear_color_stop(clear, 1.0),
                    )),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn a_line_placeholder_is_as_tall_as_the_line_it_replaces() {
        let theme = AppTheme::dark();
        let line = theme.typography.body().line_height;

        assert_eq!(
            SkeletonUi::geometry(
                &theme,
                SkeletonShape::Line {
                    width: px(120.0),
                    height: line,
                }
            ),
            (px(120.0), line, line / 2.0),
            "a placeholder taller or shorter than its line makes the layout jump"
        );
    }

    #[test]
    fn the_placeholder_reads_against_the_surface_it_sits_on() {
        let theme = AppTheme::dark();
        assert_ne!(SkeletonUi::base(&theme), theme.bg._000);
        assert_ne!(SkeletonUi::base(&theme), SkeletonUi::highlight(&theme));
    }

    #[test]
    fn the_highlight_is_brighter_than_the_base_at_both_appearances() {
        for theme in [AppTheme::dark(), AppTheme::light()] {
            let base = SkeletonUi::base(&theme);
            let highlight = SkeletonUi::highlight(&theme);
            assert!(
                (highlight.l - base.l).abs() > 0.005,
                "the sweep is invisible at {:?}",
                theme.appearance
            );
        }
    }

    #[test]
    fn a_circle_placeholder_is_actually_round() {
        let theme = AppTheme::dark();
        let diameter = px(44.0);
        let (width, height, radius) =
            SkeletonUi::geometry(&theme, SkeletonShape::Circle { diameter });

        assert_eq!(width, height);
        assert_eq!(radius, diameter / 2.0);
    }
}
