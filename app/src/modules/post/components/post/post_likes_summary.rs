use std::rc::Rc;

use gpui::{App, ElementId, SharedString, Window};
use rise_i18n::{FormatArg, tr_args};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::post::stores::post_actions::post_types::{GetPostFeedResponse, PostLikedByUser};
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;

#[derive(IntoElement)]
pub struct PostLikesSummary {
    post: Rc<GetPostFeedResponse>,
}

impl PostLikesSummary {
    const AVATAR_SIZE: f32 = 23.0;
    const AVATAR_STEP: f32 = 15.0;

    pub fn new(post: impl Into<Rc<GetPostFeedResponse>>) -> Self {
        Self { post: post.into() }
    }

    fn users(&self) -> &[PostLikedByUser] {
        &self.post.liked_by[..self.post.liked_by.len().min(3)]
    }

    fn avatar_stack(&self, cx: &App) -> Div {
        let border = rise_ui::theme(cx).bg._100;
        let users = self.users();
        let avatars = users.iter().enumerate().rev().map(|(index, user)| {
            div()
                .absolute()
                .top_0()
                .left(px(index as f32 * Self::AVATAR_STEP))
                .size(px(Self::AVATAR_SIZE))
                .child(
                    UserLogoUi::new(SharedString::from(format!("post-likes-summary-{}-{}", self.post.id, user.id)))
                        .source(SharedString::from(user.logo.clone()))
                        .name(SharedString::from(user.name.clone()))
                        .user_id(SharedString::from(user.id.clone()))
                        .size(Self::AVATAR_SIZE)
                        .show_border(true)
                        .show_story_ring(false),
                )
                .child(centred_stroke(border, 1.0, Shape::Circle))
        });
        div()
            .relative()
            .flex_none()
            .w(px(Self::AVATAR_SIZE + users.len().saturating_sub(1) as f32 * Self::AVATAR_STEP))
            .h(px(Self::AVATAR_SIZE))
            .overflow_hidden()
            .children(avatars)
    }

    fn summary_text(&self, first: &PostLikedByUser) -> String {
        let identity = if first.tag.is_empty() { &first.name } else { &first.tag };
        let key = if self.post.likes_count > 1 { "post_liked_by_and_others" } else { "post_liked_by" };
        tr_args(key, &[FormatArg::from(identity.as_str())])
    }
}

impl RenderOnce for PostLikesSummary {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let first = self.users().first();
        let Some(first) = first.filter(|_| self.post.likes_count > 0) else {
            return gpui::Empty.into_any_element();
        };
        let primary = rise_ui::theme(cx).text.primary;
        let summary = self.summary_text(first);
        let avatars = self.avatar_stack(cx);
        let id = ElementId::Name(format!("post-likes-summary-{}", self.post.id).into());
        let post = self.post.clone();
        plain(id)
            .w_full()
            .on_press(move |_, cx| {
                let post = post.clone();
                PostInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.open_post_likes(&post, cx);
                });
            })
            .child(
                hstack(5.0)
                    .w_full()
                    .child(avatars)
                    .child(div().min_w_0().child(MainText::new(summary).px(12.0).color(primary).number_of_lines(1)))
                    .child(spacer()),
            )
            .into_any_element()
    }
}
