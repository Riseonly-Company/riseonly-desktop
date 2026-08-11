use gpui::{App, Div, FontWeight, Pixels, div, prelude::*};
use rise_theme::{AppTheme, AvatarMetrics, AvatarPalette};

use crate::image_ui::ImageUi;

/// Whether a face carries a story ring, and whether that ring is unwatched.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum StoryRing {
    #[default]
    None,
    Seen,
    Unseen,
}

impl StoryRing {
    pub fn from_state(has_stories: bool, has_unseen: bool) -> Self {
        match (has_stories, has_unseen) {
            (false, _) => Self::None,
            (true, false) => Self::Seen,
            (true, true) => Self::Unseen,
        }
    }
}

/// Everything a face is drawn from.
#[derive(Clone, Copy, Default)]
pub struct AvatarSpec<'a> {
    /// The `logo` field off the wire, whatever is in it — empty strings and
    /// relative paths are handled here, not at the call site.
    pub source: Option<&'a str>,
    pub name: Option<&'a str>,
    /// The colour key. The id first, so a rename does not recolour the letter.
    pub user_id: Option<&'a str>,
    pub ring: StoryRing,
    pub show_border: bool,
}

/// A remote avatar, or the letter that stands in for one.
///
/// The letter is the final state for a user with no avatar and must agree with
/// `file-service`, which renders the same letter on the same colour for push
/// notifications; [`AvatarPalette`] is the shared half.
pub struct AvatarUi;

impl AvatarUi {
    pub fn render(theme: &AppTheme, size: Pixels, spec: AvatarSpec<'_>, cx: &App) -> Div {
        let mut face = div()
            .size(size)
            .rounded(size)
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center();

        let letter = LetterFace::new(theme, size, spec);

        match AvatarPalette::resolved_logo_url(spec.source) {
            Some(url) => {
                let loading = letter.clone();
                let failed = letter.clone();

                // The radius has to be on the IMAGE. gpui paints a texture with
                // the img's OWN corner radii, and a ContentMask is a rectangle,
                // so a rounded parent cannot make a square picture round.
                face = face.child(
                    ImageUi::remote(url, cx)
                        .size(size)
                        .rounded(size)
                        .with_loading(move || loading.render().into_any_element())
                        .with_fallback(move || failed.render().into_any_element()),
                );
            }
            None => face = face.child(letter.render()),
        }

        if spec.show_border {
            face = face
                .border(theme.avatar.border_width)
                .border_color(theme.border._100);
        }

        match spec.ring {
            StoryRing::None => face,
            ring => Self::with_ring(theme, size, face, ring),
        }
    }

    fn with_ring(theme: &AppTheme, size: Pixels, face: Div, ring: StoryRing) -> Div {
        let color = match ring {
            StoryRing::Unseen => theme.primary._100,
            _ => theme.border._300,
        };
        let outer = size + theme.avatar.ring_width * 4.0;

        div()
            .size(outer)
            .rounded(outer)
            .border(theme.avatar.ring_width)
            .border_color(color)
            .flex()
            .items_center()
            .justify_center()
            .child(face)
    }
}

/// The letter placeholder as a value: `Div` is not `Clone`, and the empty,
/// loading and failure states all draw the same picture.
#[derive(Clone)]
struct LetterFace {
    size: Pixels,
    background: gpui::Hsla,
    color: gpui::Hsla,
    letter: String,
    token: rise_theme::TextStyleToken,
}

impl LetterFace {
    fn new(theme: &AppTheme, size: Pixels, spec: AvatarSpec<'_>) -> Self {
        Self {
            size,
            background: AvatarPalette::color_for_user(spec.user_id, spec.name),
            color: AvatarPalette::letter_color(),
            letter: AvatarPalette::display_letter(spec.name),
            token: theme
                .typography
                .style(AvatarMetrics::letter_size(size), FontWeight::SEMIBOLD),
        }
    }

    fn render(&self) -> Div {
        div()
            .size(self.size)
            .rounded(self.size)
            .bg(self.background)
            .flex()
            .items_center()
            .justify_center()
            .text_size(self.token.size)
            .line_height(self.token.line_height)
            .font(self.token.font.clone())
            .text_color(self.color)
            .child(self.letter.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ring_is_only_drawn_for_a_user_who_actually_has_stories() {
        assert_eq!(StoryRing::from_state(false, false), StoryRing::None);
        assert_eq!(
            StoryRing::from_state(false, true),
            StoryRing::None,
            "unseen stories that do not exist are not a ring"
        );
        assert_eq!(StoryRing::from_state(true, false), StoryRing::Seen);
        assert_eq!(StoryRing::from_state(true, true), StoryRing::Unseen);
    }

    #[gpui::test]
    fn an_avatar_with_no_logo_falls_straight_to_its_letter(cx: &mut gpui::TestAppContext) {
        // The wire sends an empty string for a user with no avatar.
        assert_eq!(AvatarPalette::resolved_logo_url(Some("")), None);

        let theme = AppTheme::dark();
        cx.update(|cx| {
            let _ = AvatarUi::render(
                &theme,
                theme.avatar.post_header,
                AvatarSpec {
                    source: Some(""),
                    name: Some("Aianov"),
                    user_id: Some("42"),
                    ..Default::default()
                },
                cx,
            );
        });
    }

    #[gpui::test]
    fn every_size_the_product_draws_builds(cx: &mut gpui::TestAppContext) {
        let theme = AppTheme::dark();
        cx.update(|cx| {
            for size in [
                theme.avatar.post_header,
                theme.avatar.comment,
                theme.avatar.likes_summary,
                theme.avatar.story,
                theme.avatar.toolbar,
            ] {
                for ring in [StoryRing::None, StoryRing::Seen, StoryRing::Unseen] {
                    let _ = AvatarUi::render(
                        &theme,
                        size,
                        AvatarSpec {
                            source: Some("https://cdn.invalid/a.png"),
                            name: Some("Aianov"),
                            user_id: Some("42"),
                            ring,
                            show_border: true,
                        },
                        cx,
                    );
                }
            }
        });
    }

    #[test]
    fn the_letter_never_shrinks_out_of_legibility_at_the_smallest_face() {
        let theme = AppTheme::dark();
        assert_eq!(
            AvatarMetrics::letter_size(theme.avatar.likes_summary),
            AvatarMetrics::LETTER_MIN,
            "a 23pt face would otherwise draw a 10.5pt letter"
        );
    }
}
