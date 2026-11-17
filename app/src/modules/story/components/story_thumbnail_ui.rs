use std::sync::Arc;

use gpui::{
    App, AsyncWindowContext, ElementId, Entity, ImageSource, IntoElement, ObjectFit, ParentElement, RenderOnce,
    SharedString, Styled, StyledImage, Window, div, img, px, white,
};
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::IconUi;
use rise_ui::swiftui::lifecycle::use_task;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes};

use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::modules::story::shared::story_poster_cache::StoryPosterCache;
use crate::modules::story::stores::story::story_actions::story_wire_types::{StoryDTO, StoryDTOExt};

/// still of a story for covers and pickers,
/// a video story shows its generated first frame since the mp4 cant be drawn as image
#[derive(IntoElement)]
pub struct StoryThumbnailUi {
    id: Option<ElementId>,
    pub story_id: i64,
    pub source: String,
    pub is_video: bool,
    pub width: f32,
    pub height: f32,
    pub corner_radius: f32,
    pub shows_progress_indicator: bool,
    pub decode_dimension: i64,
}

impl StoryThumbnailUi {
    const VIDEO_MEDIA_TYPE: &'static str = "video";

    pub fn from_story(story: &StoryDTO, width: f32, height: f32, decode_dimension: i64) -> Self {
        Self::new(
            story.id,
            story.media_source(),
            story.media_type.to_lowercase() == Self::VIDEO_MEDIA_TYPE,
            width,
            height,
            decode_dimension,
        )
    }

    /// builds a thumbnail from its parts
    pub fn new(
        story_id: i64,
        source: impl Into<String>,
        is_video: bool,
        width: f32,
        height: f32,
        decode_dimension: i64,
    ) -> Self {
        Self {
            id: None,
            story_id,
            source: source.into(),
            is_video,
            width,
            height,
            corner_radius: 0.0,
            shows_progress_indicator: false,
            decode_dimension,
        }
    }

    pub fn corner_radius(mut self, corner_radius: f32) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    pub fn shows_progress_indicator(mut self, shows_progress_indicator: bool) -> Self {
        self.shows_progress_indicator = shows_progress_indicator;
        self
    }

    /// identity of the poster state, by default the story itself
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    async fn load_poster_if_needed(
        poster: Entity<Option<CleverImage>>,
        story_id: i64,
        source: String,
        is_video: bool,
        decode_dimension: i64,
        cx: &mut AsyncWindowContext,
    ) {
        if !is_video || source.is_empty() || story_id <= 0 {
            poster.update(cx, |poster, cx| {
                if poster.is_some() {
                    *poster = None;
                    cx.notify();
                }
            });
            return;
        }
        if let Some(hit) = StoryPosterCache::cached(story_id) {
            poster.update(cx, |poster, cx| {
                if !poster.as_ref().is_some_and(|current| Arc::ptr_eq(current, &hit)) {
                    *poster = Some(hit);
                    cx.notify();
                }
            });
            return;
        }
        let generated = StoryPosterCache::poster(story_id, &source, decode_dimension, &**cx).await;
        let Some(generated) = generated else {
            return;
        };
        poster.update(cx, |poster, cx| {
            *poster = Some(generated);
            cx.notify();
        });
    }
}

impl RenderOnce for StoryThumbnailUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self
            .id
            .clone()
            .unwrap_or_else(|| ElementId::NamedInteger(SharedString::new_static("story-thumbnail"), self.story_id as u64));
        let is_video = self.is_video;
        let story_id = self.story_id;
        let poster: Entity<Option<CleverImage>> = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static("story-thumbnail-poster")),
            cx,
            move |_, _| {
                if is_video {
                    StoryPosterCache::cached(story_id)
                } else {
                    None
                }
            },
        );

        let task_poster = poster.clone();
        let task_source = self.source.clone();
        let decode_dimension = self.decode_dimension;
        use_task(
            ElementId::NamedChild(Arc::new(id), SharedString::new_static("story-thumbnail-task")),
            (story_id, is_video, decode_dimension),
            window,
            cx,
            async move |cx| {
                Self::load_poster_if_needed(task_poster, story_id, task_source, is_video, decode_dimension, cx).await;
            },
        );

        let placeholder = ThemeStore::shared(cx).read(cx).current_theme().bg._300;
        let width = px(self.width);
        let height = px(self.height);
        let corner_radius = self.corner_radius;
        let filled_placeholder = move || {
            div()
                .w(width)
                .h(height)
                .bg(placeholder)
                .clip_shape(Shape::continuous(corner_radius))
                .into_any_element()
        };
        let video_placeholder = move || {
            div()
                .w(width)
                .h(height)
                .bg(placeholder)
                .flex()
                .items_center()
                .justify_center()
                .children(IconUi::sized("play.fill", px(15.0), white().opacity(0.55), true))
                .clip_shape(Shape::continuous(corner_radius))
                .into_any_element()
        };

        if self.is_video {
            match poster.read(cx).clone() {
                Some(poster) => div()
                    .relative()
                    .w(width)
                    .h(height)
                    .child(
                        img(ImageSource::Render(poster))
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                            .continuous_corners(corner_radius),
                    )
                    .into_any_element(),
                None => video_placeholder(),
            }
        } else if !self.source.is_empty() {
            div()
                .w(width)
                .h(height)
                .child(
                    CleverImageUi::new(self.source)
                        .content_mode(ContentMode::Fill)
                        .corner_radius(corner_radius)
                        .persist_to_disk(true)
                        .shows_progress_indicator(self.shows_progress_indicator)
                        .maximum_decode_pixel_dimension(self.decode_dimension),
                )
                .into_any_element()
        } else {
            filled_placeholder()
        }
    }
}
