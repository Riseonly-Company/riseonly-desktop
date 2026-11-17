use std::collections::HashMap;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::modules::post::shared::post_media_slides::PostMediaSlides;
use crate::modules::post::stores::post_actions::post_types::{GetPostFeedResponse, PostTagPlacement, PostTaggedUser};

/// carries person tags across a media edit
/// a slide is known by its media url so its tag follows it
/// server does the same when an update sends no tags
pub enum PostMediaTagRemap {}

#[derive(Clone, Debug, PartialEq)]
pub struct PostMediaTagRemapResult {
    pub tagged_users: Vec<PostTaggedUser>,
    pub remapped: i64,
    pub dropped_removed_media: i64,
    pub removed_tags: i64,
}

impl PostMediaTagRemap {
    pub fn slide_keys(images: &[String], video: Option<&str>, video_position: Option<i64>) -> Vec<String> {
        let mut keys: Vec<String> = images.iter().map(|image| key(image)).collect();
        let video_key = key(video.unwrap_or(""));
        if video_key.is_empty() {
            return keys;
        }
        let at = PostMediaSlides::clamped_video_position(video_position, images.len() as i64);
        keys.insert(at as usize, video_key);
        keys
    }

    /// a person tagged without placement is a mention and stays
    /// everyone else keeps placements whose media survived
    /// one per slide, and leaves the post when none did
    pub fn remap(tagged_users: &[PostTaggedUser], from: &[String], to: &[String]) -> PostMediaTagRemapResult {
        let mut new_index_by_key: HashMap<&str, i64> = HashMap::with_capacity(to.len());
        for (index, key) in to.iter().enumerate() {
            if !key.is_empty() && !new_index_by_key.contains_key(key.as_str()) {
                new_index_by_key.insert(key.as_str(), index as i64);
            }
        }

        let mut kept: Vec<PostTaggedUser> = Vec::with_capacity(tagged_users.len());
        let mut remapped: i64 = 0;
        let mut dropped: i64 = 0;
        let mut removed: i64 = 0;
        for entry in tagged_users {
            if entry.placements.is_empty() {
                kept.push(entry.clone());
                continue;
            }
            let mut moved: Vec<PostTagPlacement> = Vec::with_capacity(entry.placements.len());
            for placement in &entry.placements {
                let target = usize::try_from(placement.slide_index)
                    .ok()
                    .and_then(|old| from.get(old))
                    .and_then(|old_key| new_index_by_key.get(old_key.as_str()).copied());
                let Some(target) = target else {
                    dropped += 1;
                    continue;
                };
                let mut relocated = placement.clone();
                relocated.slide_index = target;
                moved.push(relocated);
            }
            let unique = PostMediaSlides::first_placement_per_slide(&moved);
            if unique.is_empty() {
                removed += 1;
                continue;
            }
            remapped += unique.len() as i64;
            kept.push(PostTaggedUser { placements: unique, user: entry.user.clone() });
        }
        PostMediaTagRemapResult {
            tagged_users: kept,
            remapped,
            dropped_removed_media: dropped,
            removed_tags: removed,
        }
    }
}

fn key(value: &str) -> String {
    trimming_whitespaces_and_newlines(value).to_owned()
}

impl GetPostFeedResponse {
    /// media identity of every slide, in the order tag placements address them
    pub fn media_slide_keys(&self) -> Vec<String> {
        PostMediaTagRemap::slide_keys(&self.images, self.video.as_deref(), self.video_position)
    }
}
