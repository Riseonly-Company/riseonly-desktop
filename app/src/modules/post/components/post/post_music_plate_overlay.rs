use std::rc::Rc;

use gpui::{App, Bounds, Empty, Pixels, SharedString, Size, Window, container_query, point, size};
use rise_ui::swiftui::prelude::*;

use crate::modules::media_picker::editor::editor_music_plate::{EditorMusicPlatePreview, EditorMusicPlateRenderer};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

/// where the media lands inside its container so a plate normalized to the media frame stays
/// glued to the pixels the author saw
/// no aspect means unknown, container is taken as the frame
pub enum MediaFrameRect {}

impl MediaFrameRect {
    pub fn rect(container: Size<Pixels>, media_aspect: Option<f32>, fills: bool) -> Bounds<Pixels> {
        let width = f32::from(container.width);
        let height = f32::from(container.height);
        let whole = Bounds::new(point(px(0.0), px(0.0)), container);
        let Some(aspect) = media_aspect.filter(|aspect| *aspect > 0.0) else {
            return whole;
        };
        if width <= 0.0 || height <= 0.0 {
            return whole;
        }
        let container_aspect = width / height;
        if fills {
            if aspect > container_aspect {
                return Self::centered_width(width, height, height * aspect);
            }
            return Self::centered_height(width, height, width / aspect);
        }
        if aspect > container_aspect {
            return Self::centered_height(width, height, width / aspect);
        }
        Self::centered_width(width, height, height * aspect)
    }

    fn centered_width(container_width: f32, container_height: f32, width: f32) -> Bounds<Pixels> {
        Bounds::new(point(px((container_width - width) / 2.0), px(0.0)), size(px(width), px(container_height)))
    }

    fn centered_height(container_width: f32, container_height: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(0.0), px((container_height - height) / 2.0)), size(px(container_width), px(height)))
    }
}

/// authors track card rendered live over media so the equalizer keeps animating
/// position, rotation and scale come from the stored plate normalized to the media frame
#[derive(IntoElement)]
pub struct MusicPlateOverlay {
    id: SharedString,
    title: String,
    artist: String,
    cover_url: String,
    track_id: String,
    is_dark: bool,
    center_x: f64,
    center_y: f64,
    rotation: f64,
    scale: f64,
    media_aspect: Option<f32>,
    media_fills: bool,
}

impl MusicPlateOverlay {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<SharedString>,
        title: impl Into<String>,
        artist: impl Into<String>,
        cover_url: impl Into<String>,
        track_id: impl Into<String>,
        is_dark: bool,
        center_x: f64,
        center_y: f64,
        rotation: f64,
        scale: f64,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            artist: artist.into(),
            cover_url: cover_url.into(),
            track_id: track_id.into(),
            is_dark,
            center_x,
            center_y,
            rotation,
            scale,
            media_aspect: None,
            media_fills: true,
        }
    }

    /// width / height of the media itself, none means the container is the media frame
    pub fn media_aspect(mut self, media_aspect: Option<f32>) -> Self {
        self.media_aspect = media_aspect;
        self
    }

    pub fn media_fills(mut self, media_fills: bool) -> Self {
        self.media_fills = media_fills;
        self
    }
}

impl RenderOnce for MusicPlateOverlay {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        pass_through().absolute().inset_0().overflow_hidden().child(container_query(
            move |container: Size<Pixels>, _: &mut Window, _: &mut App| {
                let frame = MediaFrameRect::rect(container, self.media_aspect, self.media_fills);
                let frame_x = f32::from(frame.origin.x);
                let frame_y = f32::from(frame.origin.y);
                let frame_width = f32::from(frame.size.width);
                let frame_height = f32::from(frame.size.height);
                let short_side = frame_width.min(frame_height).max(1.0);
                let base_width = EditorMusicPlateRenderer::BASE_WIDTH_FRACTION * short_side;
                let scale = self.scale as f32;
                let card = EditorMusicPlateRenderer::metrics(&self.title, &self.artist, base_width).card_size;
                let center_x = frame_x + self.center_x as f32 * frame_width;
                let center_y = frame_y + self.center_y as f32 * frame_height;
                pass_through().relative().size_full().child(
                    pass_through()
                        .absolute()
                        .left(px(center_x - card.width * scale / 2.0))
                        .top(px(center_y - card.height * scale / 2.0))
                        .child(
                            EditorMusicPlatePreview::new(
                                self.id,
                                self.title,
                                self.artist,
                                self.cover_url,
                                self.track_id,
                                self.is_dark,
                            )
                            .base_width(base_width)
                            .transform(scale, self.rotation as f32),
                        ),
                )
            },
        ))
    }
}

#[derive(IntoElement)]
pub struct PostMusicPlateOverlay {
    post: Rc<GetPostFeedResponse>,
    media_aspect: Option<f32>,
    media_fills: bool,
}

impl PostMusicPlateOverlay {
    pub fn new(post: impl Into<Rc<GetPostFeedResponse>>) -> Self {
        Self { post: post.into(), media_aspect: None, media_fills: true }
    }

    pub fn media_aspect(mut self, media_aspect: Option<f32>) -> Self {
        self.media_aspect = media_aspect;
        self
    }

    pub fn media_fills(mut self, media_fills: bool) -> Self {
        self.media_fills = media_fills;
        self
    }
}

impl RenderOnce for PostMusicPlateOverlay {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let (Some(plate), Some(track)) = (self.post.track_plate.as_ref(), self.post.track_model()) else {
            return Empty.into_any_element();
        };
        MusicPlateOverlay::new(
            format!("post-music-plate-{}", self.post.id),
            track.title,
            track.artist,
            track.cover_image_url,
            track.id,
            plate.is_dark,
            plate.center_x,
            plate.center_y,
            plate.rotation,
            plate.scale,
        )
        .media_aspect(self.media_aspect)
        .media_fills(self.media_fills)
        .into_any_element()
    }
}
