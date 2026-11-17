use std::collections::HashSet;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::modules::post::stores::post_actions::post_types::{GetPostFeedResponse, PostMediaSize, PostTagPlacement};

/// one page of a posts media in the order the author picked
/// image_index points into images and person tags address a
/// slide by position in this combined order, video included
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PostMediaSlide {
    Image { url: String, size: Option<PostMediaSize>, image_index: i64 },
    Video { url: String, thumbnail: Option<String>, size: Option<PostMediaSize> },
}

impl PostMediaSlide {
    pub fn id(&self) -> String {
        match self {
            PostMediaSlide::Image { image_index, .. } => format!("image:{image_index}"),
            PostMediaSlide::Video { .. } => "video".to_owned(),
        }
    }

    pub fn is_video(&self) -> bool {
        matches!(self, PostMediaSlide::Video { .. })
    }

    pub fn image_index(&self) -> Option<i64> {
        match self {
            PostMediaSlide::Image { image_index, .. } => Some(*image_index),
            PostMediaSlide::Video { .. } => None,
        }
    }

    pub fn size(&self) -> Option<&PostMediaSize> {
        match self {
            PostMediaSlide::Image { size, .. } => size.as_ref(),
            PostMediaSlide::Video { size, .. } => size.as_ref(),
        }
    }

    pub fn poster_url(&self) -> Option<&str> {
        match self {
            PostMediaSlide::Image { url, .. } => Some(url.as_str()),
            PostMediaSlide::Video { thumbnail, .. } => thumbnail.as_deref(),
        }
    }
}

pub enum PostMediaSlides {}

impl PostMediaSlides {
    /// a legacy row has no position and means video first
    /// out of range is pulled back to the nearest end, not dropped
    pub fn clamped_video_position(position: Option<i64>, image_count: i64) -> i64 {
        position.unwrap_or(0).max(0).min(image_count.max(0))
    }

    pub fn make(
        images: &[String],
        image_sizes: &[PostMediaSize],
        video: Option<&str>,
        video_thumbnail: Option<&str>,
        video_size: Option<&PostMediaSize>,
        video_position: Option<i64>,
    ) -> Vec<PostMediaSlide> {
        let mut slides: Vec<PostMediaSlide> = Vec::with_capacity(images.len() + 1);
        for (index, url) in images.iter().enumerate() {
            slides.push(PostMediaSlide::Image {
                url: url.clone(),
                size: image_sizes.get(index).cloned(),
                image_index: index as i64,
            });
        }
        let Some(url) = normalized(video) else {
            return slides;
        };
        let at = Self::clamped_video_position(video_position, images.len() as i64);
        slides.insert(
            at as usize,
            PostMediaSlide::Video { url, thumbnail: normalized(video_thumbnail), size: video_size.cloned() },
        );
        slides
    }

    /// one placement per slide for one person
    /// the first wins, same as the server does
    pub fn first_placement_per_slide(placements: &[PostTagPlacement]) -> Vec<PostTagPlacement> {
        let mut seen: HashSet<i64> = HashSet::with_capacity(placements.len());
        placements.iter().filter(|placement| seen.insert(placement.slide_index)).cloned().collect()
    }
}

fn normalized(value: Option<&str>) -> Option<String> {
    let trimmed = trimming_whitespaces_and_newlines(value?);
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

impl GetPostFeedResponse {
    pub fn has_video_media(&self) -> bool {
        normalized(self.video.as_deref()).is_some()
    }

    /// built on every read so compute once per render input, not per frame
    pub fn media_slides(&self) -> Vec<PostMediaSlide> {
        PostMediaSlides::make(
            &self.images,
            &self.image_sizes,
            self.video.as_deref(),
            self.video_thumbnail.as_deref(),
            self.video_size.as_ref(),
            self.video_position,
        )
    }

    pub fn media_slide_count(&self) -> i64 {
        self.images.len() as i64 + i64::from(self.has_video_media())
    }

    /// position of the video page among all slides, the index tag placements on the video carry
    pub fn video_slide_index(&self) -> Option<i64> {
        if !self.has_video_media() {
            return None;
        }
        Some(PostMediaSlides::clamped_video_position(self.video_position, self.images.len() as i64))
    }

    pub fn tag_slide_index(&self, slide: &PostMediaSlide) -> i64 {
        let video = self.video_slide_index();
        match slide {
            PostMediaSlide::Video { .. } => video.unwrap_or(0),
            PostMediaSlide::Image { image_index, .. } => match video {
                Some(video) if video <= *image_index => image_index + 1,
                _ => *image_index,
            },
        }
    }

    /// first slide without building the whole list
    pub fn cover_slide(&self) -> Option<PostMediaSlide> {
        if self.video_slide_index() == Some(0)
            && let Some(url) = normalized(self.video.as_deref())
        {
            return Some(PostMediaSlide::Video {
                url,
                thumbnail: normalized(self.video_thumbnail.as_deref()),
                size: self.video_size.clone(),
            });
        }
        let first = self.images.first()?;
        Some(PostMediaSlide::Image { url: first.clone(), size: self.image_sizes.first().cloned(), image_index: 0 })
    }

    pub fn is_riff_only(&self) -> bool {
        self.is_riff && !self.show_in_publications
    }
}
