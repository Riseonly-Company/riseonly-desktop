#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use gpui::{App, IntoElement, RenderOnce, Window};
use rise_theme::ThemeStore;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationModule,
};
use crate::modules::post::stores::post_actions::post_types::{AuthorInfo, GetPostFeedResponse};
use crate::modules::shorts::components::shorts_more_sheet::ShortsMoreSheetContent;

pub enum ShortsBottomSheetPreviews {}

impl ShortsBottomSheetPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        vec![BottomSheetDebugRegistration::sheet(
            "shorts.more",
            BottomSheetDebugRegistrationModule::Shorts,
            "more",
            "ShortsMoreSheetContent",
            vec![0.65, 0.93],
            None,
            |open: Binding<bool>, _: &mut Window, _: &mut App| DebugShortsMenu { is_presented: open },
        )]
    }
}

#[derive(IntoElement)]
struct DebugShortsMenu {
    is_presented: Binding<bool>,
}

impl RenderOnce for DebugShortsMenu {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let navigation_store = env::<NavigationStoreKey>(cx);
        let is_presented = self.is_presented;
        let post = GetPostFeedResponse {
            id: "debug-short".to_owned(),
            content: "A moment worth sharing".to_owned(),
            views_count: 1234,
            author_id: "debug-alex".to_owned(),
            comments_count: 24,
            likes_count: 128,
            author: AuthorInfo {
                id: Some("debug-alex".to_owned()),
                name: "Alex Morgan".to_owned(),
                tag: "alex".to_owned(),
                ..Default::default()
            },
            is_short: true,
            ..Default::default()
        };
        ShortsMoreSheetContent::new(
            post,
            theme,
            navigation_store,
            |_, _, _| {},
            move |window, cx| is_presented.set(false, window, cx),
        )
        .allows_actions(false)
    }
}
