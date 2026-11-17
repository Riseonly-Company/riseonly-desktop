use std::rc::Rc;

use gpui::{App, ElementId, FontWeight, Window};
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::modules::music::widgets::music_track_cover_image::MusicTrackCoverImage;
use crate::modules::post::components::post::post::Post;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::shorts::components::feed_sound_button::FeedSoundButton;

#[derive(IntoElement)]
pub struct PostMusicStrip {
    post: Rc<GetPostFeedResponse>,
}

impl PostMusicStrip {
    pub fn new(post: impl Into<Rc<GetPostFeedResponse>>) -> Self {
        Self { post: post.into() }
    }
}

impl RenderOnce for PostMusicStrip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(tm) = self.post.track_model() else {
            return gpui::Empty.into_any_element();
        };
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let track_id = tm.id.clone();
        let source_selection = self.post.music_use_selection();
        let id = |name: &str| ElementId::Name(format!("post-music-strip-{}-{name}", self.post.id).into());

        hstack(10.0)
            .w_full()
            .px(px(10.0))
            .py(px(8.0))
            .bg(theme.bg._300.opacity(0.6))
            .continuous_corners(12.0)
            .child(MusicTrackCoverImage::new(id("cover"), tm.cover_image_url.clone()).size(40.0).corner_radius(8.0))
            .child(
                plain(id("open"))
                    .flex_1()
                    .on_press(move |_, cx| {
                        if let Some(nav_store) = nav_store.as_ref() {
                            let route = RootRoute::AudioPage {
                                track_id: track_id.clone(),
                                source_selection: source_selection.clone(),
                            };
                            nav_store.update(cx, |nav_store, cx| {
                                nav_store.push(route, NavigationStoreSettlingPush::Drop, cx);
                            });
                        }
                    })
                    .child(
                        vstack(2.0)
                            .align(HorizontalAlignment::Leading)
                            .w_full()
                            .child(
                                MainText::new(Post::music_line_text(&tm))
                                    .px(14.0)
                                    .font_weight(FontWeight::MEDIUM)
                                    .color(theme.text.primary)
                                    .number_of_lines(1),
                            )
                            .child(SecondaryText::new(tm.formatted_duration()).px(12.0)),
                    ),
            )
            .child(spacer_min_w(0.0))
            .child(FeedSoundButton::new())
            .into_any_element()
    }
}
