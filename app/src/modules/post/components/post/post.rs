use std::rc::Rc;

use gpui::{
    ClickEvent, Context, Div, ElementId, FontWeight, IntoElement, MouseButton, MouseDownEvent,
    Pixels, Point, SharedString, Stateful, Window, div, prelude::*,
};
use rise_i18n::tr;
use rise_theme::{AppTheme, FeedMetrics};
use rise_ui::{AvatarSpec, AvatarUi, BadgeUi, IconUi, ImageUi, StoryRing};

use crate::modules::post::engine::rise_post_engine_models::PostEntry;
use rise_i18n::relative_time;

#[derive(Clone, PartialEq, Debug)]
pub enum PostAction {
    Like,
    DoubleTapLike,
    Favorite,
    Comments,
    Repost,
    ShowLikes,
    OpenAuthor,
    OpenOptions(Point<Pixels>),
    OpenHashtag(String),
}

pub type PostHandler<V> = Rc<dyn Fn(&mut V, PostAction, String, &mut Window, &mut Context<V>)>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PostSurface {
    FeedRow,
    Embedded,
}

pub struct PostCard;

impl PostCard {
    pub fn render<V: 'static>(
        theme: &AppTheme,
        post: &PostEntry,
        surface: PostSurface,
        now_ms: i64,
        shows_heart: bool,
        handler: &PostHandler<V>,
        cx: &mut Context<V>,
    ) -> Div {
        Self::standard(theme, post, surface, now_ms, shows_heart, handler, cx)
    }

    fn standard<V: 'static>(
        theme: &AppTheme,
        post: &PostEntry,
        surface: PostSurface,
        now_ms: i64,
        shows_heart: bool,
        handler: &PostHandler<V>,
        cx: &mut Context<V>,
    ) -> Div {
        let metrics = theme.feed;
        let shows_summary = !post.liked_by.is_empty() && post.likes_count > 0;
        let has_media = !post.images.is_empty();

        let mut card = div().w_full().flex().flex_col().bg(theme.bg._100).child(
            Self::header(theme, post, surface, now_ms, handler, cx)
                .px(metrics.card_padding_x)
                .pt(metrics.header_top)
                .pb(metrics.header_bottom),
        );

        if post.is_moderation_disabled {
            card = card.child(Self::moderation_banner(theme));
        }

        if has_media {
            card = card.child(Self::media(theme, post, surface, shows_heart, handler, cx));
        }

        if post.has_written_content() {
            card = card.child(
                Self::copy(theme, post, handler, cx)
                    .px(metrics.card_padding_x)
                    .pt(if has_media {
                        metrics.copy_top_after_media
                    } else {
                        metrics.copy_top_after_header
                    })
                    .pb(metrics.copy_bottom),
            );
        }

        if surface == PostSurface::Embedded {
            return card.pb(metrics.copy_bottom);
        }

        card.child(
            Self::actions(theme, post, handler, cx)
                .px(metrics.card_padding_x)
                .pt(metrics.actions_top)
                .pb(if shows_summary {
                    metrics.actions_bottom_with_summary
                } else {
                    metrics.actions_bottom
                }),
        )
        .when(shows_summary, |card| {
            card.child(
                div()
                    .px(metrics.summary_padding_x)
                    .pb(metrics.summary_bottom)
                    .child(Self::likes_summary(theme, post, handler, cx)),
            )
        })
    }

    fn header<V: 'static>(
        theme: &AppTheme,
        post: &PostEntry,
        surface: PostSurface,
        now_ms: i64,
        handler: &PostHandler<V>,
        cx: &mut Context<V>,
    ) -> Div {
        let metrics = theme.feed;
        let author = &post.author;
        let row_id = post.row_id().to_owned();

        let name = theme
            .typography
            .style(FeedMetrics::AUTHOR_SIZE, FontWeight::NORMAL);
        let meta = theme
            .typography
            .style(FeedMetrics::META_SIZE, FontWeight::NORMAL);

        let mut identity = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(metrics.header_name_gap)
            .child(
                div()
                    .text_size(name.size)
                    .line_height(name.line_height)
                    .font(name.font)
                    .text_color(theme.text.primary)
                    .truncate()
                    .child(author.name.clone()),
            );

        for badge in BadgeUi::for_author(
            theme,
            metrics.badge_size,
            author.is_official,
            author.is_premium,
        ) {
            identity = identity.child(badge);
        }

        let mut meta_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(metrics.header_meta_gap);

        if !author.tag.is_empty() {
            meta_row = meta_row
                .child(
                    div()
                        .text_size(meta.size)
                        .line_height(meta.line_height)
                        .font(meta.font.clone())
                        .text_color(theme.text.secondary)
                        .truncate()
                        .child(format!("@{}", author.tag)),
                )
                .child(
                    div()
                        .text_size(meta.size)
                        .line_height(meta.line_height)
                        .font(meta.font.clone())
                        .text_color(theme.text.secondary)
                        .child("·"),
                );
        }

        let elapsed = relative_time::relative(post.created_at_ms, &post.created_at, now_ms);
        if !elapsed.is_empty() {
            meta_row = meta_row.child(
                div()
                    .text_size(meta.size)
                    .line_height(meta.line_height)
                    .font(meta.font)
                    .text_color(theme.text.secondary)
                    .child(elapsed),
            );
        }

        let open_author = handler.clone();
        let author_row_id = row_id.clone();

        let mut row = div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(metrics.header_gap)
            .child(
                div()
                    .id(ElementId::Name(SharedString::from(format!(
                        "post.avatar.{row_id}"
                    ))))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                        open_author(
                            view,
                            PostAction::OpenAuthor,
                            author_row_id.clone(),
                            window,
                            cx,
                        );
                    }))
                    .child(AvatarUi::render(
                        theme,
                        theme.avatar.post_header,
                        AvatarSpec {
                            source: Some(author.logo.as_str()),
                            name: Some(author.name.as_str()),
                            user_id: Some(author.id.as_str()),
                            ring: StoryRing::None,
                            show_border: false,
                        },
                        cx,
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(identity)
                    .child(meta_row),
            );

        if surface == PostSurface::FeedRow {
            let open_options = handler.clone();
            let options_row_id = row_id.clone();

            row = row.child(
                div()
                    .id(ElementId::Name(SharedString::from(format!(
                        "post.options.{row_id}"
                    ))))
                    .size(metrics.options_button)
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(metrics.options_button)
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.bg._300))
                    .on_click(cx.listener(move |view, event: &ClickEvent, window, cx| {
                        let position = match event {
                            ClickEvent::Mouse(mouse) => mouse.up.position,
                            _ => Point::default(),
                        };
                        open_options(
                            view,
                            PostAction::OpenOptions(position),
                            options_row_id.clone(),
                            window,
                            cx,
                        );
                    }))
                    .children(IconUi::sized(
                        "ellipsis",
                        theme.icon.regular,
                        theme.text.secondary,
                        false,
                    )),
            );
        }

        row
    }

    fn moderation_banner(theme: &AppTheme) -> Div {
        let metrics = theme.feed;
        let token = theme
            .typography
            .style(FeedMetrics::BANNER_SIZE, FontWeight::MEDIUM);

        div()
            .mx(metrics.card_padding_x)
            .mb(metrics.banner_bottom)
            .px(metrics.banner_padding_x)
            .py(metrics.banner_padding_y)
            .rounded(metrics.banner_radius)
            .bg(theme.semantic.error_100.opacity(0.1))
            .flex()
            .flex_row()
            .items_center()
            .gap(metrics.banner_gap)
            .children(IconUi::sized(
                "eye.slash",
                theme.icon.small,
                theme.semantic.error_100,
                false,
            ))
            .child(
                div()
                    .text_size(token.size)
                    .line_height(token.line_height)
                    .font(token.font)
                    .text_color(theme.semantic.error_100)
                    .truncate()
                    .child(tr("post_disabled_by_moderation")),
            )
    }

    fn media<V: 'static>(
        theme: &AppTheme,
        post: &PostEntry,
        surface: PostSurface,
        shows_heart: bool,
        handler: &PostHandler<V>,
        cx: &mut Context<V>,
    ) -> Stateful<Div> {
        let metrics = theme.feed;
        let aspect = metrics.gallery_aspect(post.image_aspects.first().copied().flatten());
        let row_id = post.row_id().to_owned();

        let mut strip = div()
            .id(ElementId::Name(SharedString::from(format!(
                "post.media.{row_id}"
            ))))
            .relative()
            .w_full()
            .aspect_ratio(aspect)
            .overflow_hidden()
            .bg(theme.bg._300);

        if let Some(url) = post.images.first() {
            strip = strip.child(ImageUi::remote_with_states(theme, url.clone(), cx).size_full());
        }

        if post.images.len() > 1 {
            strip = strip.child(Self::page_dots(theme, post.images.len()));
        }

        if surface == PostSurface::FeedRow {
            let double_tap = handler.clone();
            let media_row_id = row_id.clone();

            strip = strip.on_click(cx.listener(move |view, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    double_tap(
                        view,
                        PostAction::DoubleTapLike,
                        media_row_id.clone(),
                        window,
                        cx,
                    );
                }
            }));
        }

        if shows_heart {
            strip = strip.child(Self::double_tap_heart(theme));
        }

        strip
    }

    fn page_dots(theme: &AppTheme, count: usize) -> Div {
        let metrics = theme.feed;
        let mut dots = div()
            .absolute()
            .bottom(metrics.gallery_dots_bottom)
            .left_0()
            .right_0()
            .flex()
            .justify_center();

        let mut row = div()
            .flex()
            .flex_row()
            .gap(metrics.gallery_dot_gap)
            .px(metrics.gallery_dots_padding_x)
            .py(metrics.gallery_dots_padding_y)
            .rounded(metrics.gallery_dot_size * 4.0)
            .bg(theme.material.scrim);

        for index in 0..count.min(10) {
            row = row.child(
                div()
                    .size(metrics.gallery_dot_size)
                    .rounded(metrics.gallery_dot_size)
                    .bg(if index == 0 {
                        theme.text.primary
                    } else {
                        theme.text.secondary
                    }),
            );
        }

        dots = dots.child(row);
        dots
    }

    fn double_tap_heart(theme: &AppTheme) -> Div {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized(
                "heart.fill",
                theme.feed.double_tap_heart,
                theme.primary._100,
                true,
            ))
    }

    fn copy<V: 'static>(
        theme: &AppTheme,
        post: &PostEntry,
        handler: &PostHandler<V>,
        cx: &mut Context<V>,
    ) -> Div {
        let metrics = theme.feed;
        let mut column = div().w_full().flex().flex_col().gap(metrics.copy_gap);

        if !post.title.is_empty() {
            let token = theme
                .typography
                .style(FeedMetrics::TITLE_SIZE, FontWeight::SEMIBOLD);
            column = column.child(
                div()
                    .text_size(token.size)
                    .line_height(token.line_height)
                    .font(token.font)
                    .text_color(theme.text.primary)
                    .child(post.title.clone()),
            );
        }

        if !post.content.is_empty() {
            let token = theme
                .typography
                .style(FeedMetrics::CONTENT_SIZE, FontWeight::NORMAL);
            column = column.child(
                div()
                    .text_size(token.size)
                    .line_height(token.line_height)
                    .font(token.font)
                    .text_color(theme.text.primary)
                    .child(post.content.clone()),
            );
        }

        if !post.hashtags.is_empty() {
            column = column.child(Self::hashtags(theme, post, handler, cx));
        }

        column
    }

    fn hashtags<V: 'static>(
        theme: &AppTheme,
        post: &PostEntry,
        handler: &PostHandler<V>,
        cx: &mut Context<V>,
    ) -> Div {
        let token = theme
            .typography
            .style(FeedMetrics::CONTENT_SIZE, FontWeight::NORMAL);
        let row_id = post.row_id().to_owned();

        let mut row = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(theme.feed.hashtag_gap);

        for tag in &post.hashtags {
            let open = handler.clone();
            let owner = row_id.clone();
            let hashtag = tag.clone();

            row = row.child(
                div()
                    .id(ElementId::Name(SharedString::from(format!(
                        "post.tag.{row_id}.{tag}"
                    ))))
                    .cursor_pointer()
                    .text_size(token.size)
                    .line_height(token.line_height)
                    .font(token.font.clone())
                    .text_color(theme.primary._100)
                    .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                        open(
                            view,
                            PostAction::OpenHashtag(hashtag.clone()),
                            owner.clone(),
                            window,
                            cx,
                        );
                    }))
                    .child(format!("#{tag}")),
            );
        }

        row
    }

    fn actions<V: 'static>(
        theme: &AppTheme,
        post: &PostEntry,
        handler: &PostHandler<V>,
        cx: &mut Context<V>,
    ) -> Div {
        let counters = post;
        let muted = theme.text.secondary;
        let row_id = post.row_id().to_owned();

        div()
            .w_full()
            .flex()
            .flex_row()
            .gap(theme.feed.actions_gap)
            .child(Self::action(
                theme,
                &row_id,
                "like",
                "heart",
                counters.is_liked,
                if counters.is_liked {
                    theme.primary._100
                } else {
                    muted
                },
                counters.likes_count,
                PostAction::Like,
                handler,
                cx,
            ))
            .child(Self::action(
                theme,
                &row_id,
                "favorite",
                "bookmark",
                counters.is_favorited,
                if counters.is_favorited {
                    theme.primary._100
                } else {
                    muted
                },
                counters.favorites_count,
                PostAction::Favorite,
                handler,
                cx,
            ))
            .child(Self::action(
                theme,
                &row_id,
                "comments",
                "bubble.right",
                false,
                muted,
                counters.comments_count,
                PostAction::Comments,
                handler,
                cx,
            ))
            .child(Self::action(
                theme,
                &row_id,
                "repost",
                "arrow.2.squarepath",
                false,
                if counters.is_reposted {
                    theme.primary._100
                } else {
                    muted
                },
                counters.repost_count,
                PostAction::Repost,
                handler,
                cx,
            ))
    }

    #[allow(clippy::too_many_arguments)]
    fn action<V: 'static>(
        theme: &AppTheme,
        row_id: &str,
        slot: &'static str,
        sf_symbol: &'static str,
        is_filled: bool,
        tint: gpui::Hsla,
        count: i64,
        action: PostAction,
        handler: &PostHandler<V>,
        cx: &mut Context<V>,
    ) -> Stateful<Div> {
        let metrics = theme.feed;
        let token = theme
            .typography
            .style(FeedMetrics::ACTION_COUNT_SIZE, FontWeight::NORMAL);
        let press = handler.clone();
        let owner = row_id.to_owned();

        div()
            .id(ElementId::Name(SharedString::from(format!(
                "post.action.{slot}.{row_id}"
            ))))
            .flex_1()
            .h(metrics.actions_height)
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(metrics.banner_gap)
            .rounded(theme.radius._100)
            .cursor_pointer()
            .hover(|style| style.bg(theme.bg._300))
            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                press(view, action.clone(), owner.clone(), window, cx);
            }))
            .children(IconUi::sized(
                sf_symbol,
                theme.icon.regular,
                tint,
                is_filled,
            ))
            .child(
                div()
                    .min_w(metrics.actions_count_min_width)
                    .text_size(token.size)
                    .line_height(token.line_height)
                    .font(token.font)
                    .text_color(theme.text.secondary)
                    .child(count_label(count)),
            )
    }

    fn likes_summary<V: 'static>(
        theme: &AppTheme,
        post: &PostEntry,
        handler: &PostHandler<V>,
        cx: &mut Context<V>,
    ) -> gpui::AnyElement {
        let counters = post;
        let faces: Vec<_> = counters.liked_by.iter().take(3).collect();
        let Some(first) = faces.first() else {
            return div().into_any_element();
        };

        let size = theme.avatar.likes_summary;
        let step = theme.avatar.likes_summary_step;
        let mut stack = div()
            .relative()
            .h(size)
            .w(size + step * (faces.len().saturating_sub(1)) as f32)
            .flex_shrink_0();

        for (index, face) in faces.iter().enumerate() {
            stack = stack.child(
                div()
                    .absolute()
                    .top_0()
                    .left(step * index as f32)
                    .rounded(size)
                    .border(theme.avatar.border_width)
                    .border_color(theme.bg._100)
                    .child(AvatarUi::render(
                        theme,
                        size,
                        AvatarSpec {
                            source: Some(face.logo.as_str()),
                            name: Some(face.name.as_str()),
                            user_id: Some(face.id.as_str()),
                            ring: StoryRing::None,
                            show_border: false,
                        },
                        cx,
                    )),
            );
        }

        let identity = if first.tag.is_empty() {
            first.name.clone()
        } else {
            first.tag.clone()
        };
        let key = if counters.likes_count > 1 {
            "post_liked_by_and_others"
        } else {
            "post_liked_by"
        };
        let token = theme
            .typography
            .style(FeedMetrics::META_SIZE, FontWeight::NORMAL);
        let press = handler.clone();
        let owner = post.row_id().to_owned();

        div()
            .id(ElementId::Name(SharedString::from(format!(
                "post.likes.{owner}"
            ))))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.feed.actions_gap)
            .cursor_pointer()
            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                press(view, PostAction::ShowLikes, owner.clone(), window, cx);
            }))
            .child(stack)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(token.size)
                    .line_height(token.line_height)
                    .font(token.font)
                    .text_color(theme.text.primary)
                    .child(rise_i18n::tr_args(key, &[identity.as_str().into()])),
            )
            .into_any_element()
    }

    pub fn separator(theme: &AppTheme) -> Div {
        div()
            .w_full()
            .h(theme.feed.separator_height)
            .bg(theme.border._100)
    }

    pub fn with_context_menu<V: 'static>(
        card: Div,
        row_id: String,
        handler: &PostHandler<V>,
        cx: &mut Context<V>,
    ) -> Div {
        let open = handler.clone();

        card.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                open(
                    view,
                    PostAction::OpenOptions(event.position),
                    row_id.clone(),
                    window,
                    cx,
                );
            }),
        )
    }
}

pub fn count_label(count: i64) -> String {
    if count >= 1_000 {
        format!("{:.1}K", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

pub fn placeholder_height(theme: &AppTheme) -> Pixels {
    let metrics = theme.feed;
    theme.avatar.post_header
        + metrics.header_top
        + metrics.header_bottom
        + metrics.actions_height
        + metrics.actions_top
        + metrics.actions_bottom
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_read_the_way_the_reference_formats_them() {
        assert_eq!(count_label(0), "0");
        assert_eq!(count_label(999), "999");
        assert_eq!(count_label(1_000), "1.0K");
        assert_eq!(count_label(1_234), "1.2K");
        assert_eq!(count_label(1_000_000), "1000.0K");
    }

    #[test]
    fn a_negative_count_never_reaches_the_label_but_does_not_panic_if_it_does() {
        assert_eq!(count_label(-1), "-1");
    }

    #[test]
    fn a_placeholder_is_at_least_as_tall_as_a_cards_chrome() {
        let theme = AppTheme::dark();
        assert!(placeholder_height(&theme) > theme.avatar.post_header);
    }
}
