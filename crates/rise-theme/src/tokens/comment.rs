use gpui::Pixels;

use crate::tokens::density::Density;

/// A comment row: denser than a post, and deliberately its own group rather
/// than a reuse of [`crate::FeedMetrics`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CommentMetrics {
    /// Between the avatar and the body column.
    pub row_gap: Pixels,
    pub row_padding_x: Pixels,
    pub row_padding_y: Pixels,
    /// Between the author line, the text, the media and the actions.
    pub body_gap: Pixels,
    /// Between the name and its badges.
    pub name_gap: Pixels,
    pub badge_size: Pixels,

    /// Between the three action buttons, and the height each one occupies.
    pub actions_gap: Pixels,
    pub actions_height: Pixels,
    /// Between an action's glyph and its count.
    pub action_label_gap: Pixels,

    /// A reply is indented under the comment it answers.
    pub reply_indent: Pixels,
    /// The moderation placeholder's gap.
    pub placeholder_gap: Pixels,

    /// How far a comment still being sent recedes.
    pub pending_opacity: f32,
}

impl CommentMetrics {
    pub const AUTHOR_SIZE: f32 = 13.0;
    pub const TIME_SIZE: f32 = 10.0;
    pub const CONTENT_SIZE: f32 = 14.0;
    pub const ACTION_SIZE: f32 = 12.0;
    pub const PLACEHOLDER_SIZE: f32 = 13.0;

    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            row_gap: l(9.0),
            row_padding_x: l(12.0),
            row_padding_y: l(10.0),
            body_gap: l(5.0),
            name_gap: l(5.0),
            badge_size: l(15.0),

            actions_gap: l(16.0),
            actions_height: l(32.0),
            action_label_gap: l(5.0),

            reply_indent: l(22.0),
            placeholder_gap: l(5.0),

            pending_opacity: 0.55,
        }
    }
}

impl Default for CommentMetrics {
    fn default() -> Self {
        Self::new(Density::NORMAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn the_numbers_are_the_reference_ones() {
        let comment = CommentMetrics::default();
        assert_eq!(comment.row_gap, px(9.0));
        assert_eq!(comment.row_padding_x, px(12.0));
        assert_eq!(comment.row_padding_y, px(10.0));
        assert_eq!(comment.actions_gap, px(16.0));
    }

    #[test]
    fn a_pending_comment_is_dimmed_but_still_readable() {
        let comment = CommentMetrics::default();
        assert!(comment.pending_opacity > 0.4 && comment.pending_opacity < 1.0);
    }

    #[test]
    fn a_comment_is_tighter_than_a_post() {
        use crate::tokens::feed::FeedMetrics;

        let comment = CommentMetrics::default();
        let feed = FeedMetrics::default();
        const { assert!(CommentMetrics::CONTENT_SIZE < FeedMetrics::CONTENT_SIZE) };
        assert!(comment.body_gap < feed.copy_gap);
    }
}
