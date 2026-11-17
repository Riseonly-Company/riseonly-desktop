use gpui::Pixels;

use crate::tokens::density::Density;

/// a comment row, denser than a post
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CommentMetrics {
    pub row_gap: Pixels,
    pub row_padding_x: Pixels,
    pub row_padding_y: Pixels,
    pub body_gap: Pixels,
    pub name_gap: Pixels,
    pub badge_size: Pixels,

    pub actions_gap: Pixels,
    pub actions_height: Pixels,
    pub action_label_gap: Pixels,

    pub reply_indent: Pixels,
    pub placeholder_gap: Pixels,

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
