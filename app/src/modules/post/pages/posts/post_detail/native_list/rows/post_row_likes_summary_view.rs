use std::rc::{Rc, Weak};

use gpui::{
    AppContext as _, Bounds, Context, ElementId, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Pixels, Render, Role, SharedString, StatefulInteractiveElement, Styled, Window, div, point,
};
use rise_i18n::{FormatArg, tr_args};
use rise_ui::swiftui::button_style::plain;

use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostRowActionRouting, PostRowLikesSummaryLayout, PostRowMainContext, PostRowSeedLikedBy,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_layout_calculator::PostRowMetrics;
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_native_row_view::{
    PostRowChromeFonts, PostRowTextLineView, PostRowTextLineViewAlignment, post_row_frame,
};
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_header_view::{
    PostRowAvatarView, PostRowAvatarViewStroke,
};

pub struct PostRowLikesSummaryView {
    avatar_views: [Entity<PostRowAvatarView>; 3],
    text_view: Entity<PostRowTextLineView>,

    summary_layout: Option<PostRowLikesSummaryLayout>,
    post_id: Option<String>,
    router: Option<Weak<dyn PostRowActionRouting>>,
    visible_avatar_count: usize,
    accessibility_label: SharedString,
}

impl PostRowLikesSummaryView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            avatar_views: [
                cx.new(|cx| PostRowAvatarView::new(false, PostRowAvatarViewStroke::BackgroundRing, cx)),
                cx.new(|cx| PostRowAvatarView::new(false, PostRowAvatarViewStroke::BackgroundRing, cx)),
                cx.new(|cx| PostRowAvatarView::new(false, PostRowAvatarViewStroke::BackgroundRing, cx)),
            ],
            text_view: cx.new(|_| PostRowTextLineView::new()),
            summary_layout: None,
            post_id: None,
            router: None,
            visible_avatar_count: 0,
            accessibility_label: SharedString::default(),
        }
    }

    pub fn configure(
        &mut self,
        post_id: &str,
        liked_by: &[PostRowSeedLikedBy],
        likes: i64,
        layout: &PostRowLikesSummaryLayout,
        context: &PostRowMainContext,
        cx: &mut Context<Self>,
    ) {
        self.post_id = Some(post_id.to_owned());
        self.router = Some(Rc::downgrade(&context.router));
        self.summary_layout = Some(layout.clone());

        let palette = &context.palette;
        self.visible_avatar_count = liked_by.len().min(layout.avatars.len()).min(self.avatar_views.len());
        for (index, avatar) in self.avatar_views.iter().enumerate() {
            let Some(user) = liked_by.get(index).filter(|_| index < self.visible_avatar_count) else {
                avatar.update(cx, |avatar, cx| avatar.prepare_for_reuse(cx));
                continue;
            };
            avatar.update(cx, |avatar, cx| {
                avatar.configure(
                    &user.logo_url,
                    &user.name,
                    Some(&user.id),
                    false,
                    PostRowMetrics::LIKES_AVATAR_SIZE,
                    palette,
                    &context.spec,
                    context.account_user_id.as_deref(),
                    context.account_epoch.as_deref(),
                    cx,
                );
            });
        }

        let text = match liked_by.first() {
            Some(first) => {
                let identity = if first.tag.is_empty() { &first.name } else { &first.tag };
                let key = if likes > 1 { "post_liked_by_and_others" } else { "post_liked_by" };
                tr_args(key, &[FormatArg::from(identity.as_str())])
            }
            None => String::new(),
        };
        let font = PostRowChromeFonts::text(PostRowMetrics::LIKES_FONT_SIZE, FontWeight::NORMAL, &context.spec);
        let color = palette.text_primary;
        self.text_view.update(cx, |view, cx| {
            view.set_text(&text, font, color, PostRowTextLineViewAlignment::Leading, cx);
        });
        self.accessibility_label = text.into();
        cx.notify();
    }

    pub fn prepare_for_reuse(&mut self, cx: &mut Context<Self>) {
        self.post_id = None;
        self.summary_layout = None;
        self.visible_avatar_count = 0;
        for avatar in &self.avatar_views {
            avatar.update(cx, |avatar, cx| avatar.prepare_for_reuse(cx));
        }
        cx.notify();
    }
}

impl Render for PostRowLikesSummaryView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(layout) = self.summary_layout.as_ref() else {
            return div().into_any_element();
        };
        let entity = cx.entity_id().as_u64();
        let router = self.router.clone();
        let post_id = self.post_id.clone();

        let frames: Vec<Bounds<Pixels>> = layout.avatars.iter().take(self.visible_avatar_count).copied().collect();
        let stack = frames.first().zip(frames.last()).map(|(first, last)| {
            let origin = first.origin;
            let mut stack = post_row_frame(Bounds {
                origin,
                size: gpui::size(last.origin.x + last.size.width - origin.x, first.size.height),
            })
            .overflow_hidden();
            for (index, frame) in frames.iter().enumerate().rev() {
                stack = stack.child(
                    post_row_frame(Bounds {
                        origin: point(frame.origin.x - origin.x, frame.origin.y - origin.y),
                        size: frame.size,
                    })
                    .child(self.avatar_views[index].clone()),
                );
            }
            stack
        });

        div()
            .id(ElementId::NamedInteger("likes-summary-hint".into(), entity))
            .role(Role::Button)
            .aria_label(self.accessibility_label.clone())
            .relative()
            .size_full()
            .child(
                plain(ElementId::NamedInteger("likes-summary".into(), entity))
                    .relative()
                    .size_full()
                    .on_press(move |_: &mut Window, cx: &mut gpui::App| {
                        let Some(router) = router.as_ref().and_then(Weak::upgrade) else {
                            return;
                        };
                        let Some(post_id) = post_id.as_deref() else {
                            return;
                        };
                        router.open_likes(post_id, cx);
                    })
                    .children(stack)
                    .child(post_row_frame(layout.text).child(self.text_view.clone())),
            )
            .into_any_element()
    }
}
