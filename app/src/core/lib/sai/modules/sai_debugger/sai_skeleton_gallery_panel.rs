#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use gpui::{Context, FontWeight, SharedString, Window};
use rise_theme::AppTheme;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonCircle, SkeletonLine, SkeletonListUi, SkeletonShimmerExt};
use rise_ui::swiftui::prelude::*;

use crate::modules::chat::components::chat_item::chat_item_skeleton::ChatListSkeleton;
use crate::modules::comment::components::comment_skeleton::{CommentListSkeleton, CommentRowSkeletonStyle};
use crate::modules::music::widgets::music_skeletons::TrackListSkeleton;
use crate::modules::notify::components::notify_skeleton::NotifyListSkeleton;
use crate::modules::post::components::post::post_skeleton::{
    PostCardSkeleton, PostCardSkeletonVariant, PostGridSkeleton,
};
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;

pub struct SaiSkeletonGalleryPanel {
    theme: AppTheme,
}

impl SaiSkeletonGalleryPanel {
    pub fn new(theme: AppTheme, _cx: &mut Context<Self>) -> Self {
        Self { theme }
    }

    pub fn set_theme(&mut self, theme: AppTheme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    fn section(&self, title: &'static str, content: impl IntoElement) -> Div {
        let theme = &self.theme;
        vstack(8.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(
                div()
                    .child(SharedString::new_static(title))
                    .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                    .text_color(theme.text.secondary),
            )
            .child(
                div()
                    .w_full()
                    .py(px(8.))
                    .bg(theme.bg._100)
                    .clip_shape(Shape::continuous(10.))
                    .child(content),
            )
    }
}

impl Render for SaiSkeletonGalleryPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let primitives = hstack(12.)
            .child(SkeletonCircle::new(40.))
            .child(
                vstack(8.)
                    .align(HorizontalAlignment::Leading)
                    .child(SkeletonLine::new(140., 10.))
                    .child(SkeletonLine::new(None::<f32>, 8.)),
            )
            .child(SkeletonBlock::new(56., 56., 10.))
            .px(px(12.))
            .skeleton_shimmer(None);

        scroll_view("sai-skeleton-gallery").child(
            vstack(14.)
                .align(HorizontalAlignment::Leading)
                .w_full()
                .child(self.section("Primitives", primitives))
                .child(self.section("SkeletonListUi", SkeletonListUi { rows: 3, ..Default::default() }))
                .child(self.section(
                    "PostCardSkeleton \u{2014} text",
                    PostCardSkeleton::default().variant(PostCardSkeletonVariant::Text),
                ))
                .child(self.section(
                    "PostCardSkeleton \u{2014} media",
                    PostCardSkeleton::default()
                        .variant(PostCardSkeletonVariant::Media { aspect: 1.5 })
                        .alternation_index(1),
                ))
                .child(self.section("ChatListSkeleton", ChatListSkeleton::new().rows(3)))
                .child(self.section(
                    "CommentListSkeleton \u{2014} sheet",
                    CommentListSkeleton::default().style(CommentRowSkeletonStyle::Sheet).rows(2),
                ))
                .child(self.section(
                    "CommentListSkeleton \u{2014} profile",
                    CommentListSkeleton::default().style(CommentRowSkeletonStyle::Profile).rows(2),
                ))
                .child(self.section("NotifyListSkeleton", NotifyListSkeleton::new().rows(3)))
                .child(self.section("UserListSkeleton", UserListSkeleton::new().rows(3)))
                .child(self.section("TrackListSkeleton", TrackListSkeleton::new().rows(3)))
                .child(self.section("PostGridSkeleton", PostGridSkeleton::default().rows(2)))
                .p(px(12.)),
        )
    }
}
