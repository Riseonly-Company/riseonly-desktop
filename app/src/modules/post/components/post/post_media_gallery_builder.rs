use std::collections::HashMap;

use gpui::App;
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_i18n::date_format::iso8601_date_formatter::{ISO8601DateFormatter, ISO8601DateFormatterOptions};

use crate::core::media_viewer::media_viewer_types::{
    MediaViewerFrameKeys, MediaViewerGalleryItem, MediaViewerGalleryItemKind, PostDownloadRef,
};
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::post::components::download::post_download_store::PostDownloadSheetStore;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::shared::post_media_slides::PostMediaSlide;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

pub enum PostMediaGalleryBuilder {}

struct PostMediaGalleryBuilderViewerMedia {
    kind: MediaViewerGalleryItemKind,
    url: String,
    thumbnail: Option<String>,
    width: f32,
    height: f32,
    duration: Option<TimeInterval>,
}

impl PostMediaGalleryBuilder {
    /// one viewer item per slide at the slides own position
    /// measured_aspects holds shapes read from decoded pictures by slide index
    /// only video uses them since the server measures video without rotation
    pub fn items(
        post: &GetPostFeedResponse,
        slides: &[PostMediaSlide],
        measured_aspects: &HashMap<usize, f32>,
        cx: &mut App,
    ) -> Vec<MediaViewerGalleryItem> {
        let created_at = Self::parse_date(&post.created_at);
        let logo = RemoteMediaURL::optional_normalized(Some(post.author.more.logo.as_str()));
        let is_own = PostMediaCachePolicy::is_own_post(post, cx);
        let can_save = post.download_policy(cx).is_some_and(|policy| policy.allowed);
        if can_save {
            PostDownloadSheetStore::shared(cx).update(cx, |store, _| store.remember_viewer_post(post.clone()));
        }
        slides
            .iter()
            .enumerate()
            .map(|(index, slide)| {
                let media = Self::viewer_media(slide, measured_aspects.get(&index).copied());
                MediaViewerGalleryItem {
                    id: format!("post-{}-{index}", post.id),
                    kind: media.kind,
                    url: media.url,
                    thumbnail_url: media.thumbnail,
                    width: media.width,
                    height: media.height,
                    duration: media.duration,
                    sender_id: post.author.id.clone().unwrap_or_else(|| post.author_id.clone()),
                    sender_name: post.author.name.clone(),
                    sender_logo: logo.clone(),
                    created_at,
                    caption_title: post.title.clone(),
                    caption: post.content.clone(),
                    message_id: post.id.clone(),
                    index_in_message: index,
                    media_item_id: format!("{}-{index}", post.id),
                    uses_fit_letterbox: false,
                    frame_registry_id: MediaViewerFrameKeys::post_image(&post.id, index),
                    is_own,
                    can_delete: false,
                    can_save,
                    can_forward: true,
                    can_reply: false,
                    can_show_in_context: false,
                    can_delete_for_me: false,
                    can_delete_for_everyone: false,
                    can_set_as_main: false,
                    is_album_row: false,
                    display_source_override: None,
                    persist_media_to_disk: is_own,
                    deletes_from_favourites: false,
                    image_cache_policy: PostMediaCachePolicy::image_policy(is_own),
                    post_download: can_save.then(|| PostDownloadRef { post_id: post.id.clone(), index }),
                }
            })
            .collect()
    }

    fn viewer_media(slide: &PostMediaSlide, measured_aspect: Option<f32>) -> PostMediaGalleryBuilderViewerMedia {
        match slide {
            PostMediaSlide::Image { url, .. } => PostMediaGalleryBuilderViewerMedia {
                kind: MediaViewerGalleryItemKind::Image,
                url: url.clone(),
                thumbnail: None,
                width: 0.0,
                height: 0.0,
                duration: None,
            },
            PostMediaSlide::Video { url, thumbnail, size } => {
                let duration_ms = size.as_ref().map_or(0, |size| size.duration_ms);
                let duration = (duration_ms > 0).then(|| duration_ms as TimeInterval / 1000.0);
                if let Some(aspect) = measured_aspect.filter(|aspect| aspect.is_finite() && *aspect > 0.0) {
                    return PostMediaGalleryBuilderViewerMedia {
                        kind: MediaViewerGalleryItemKind::Video,
                        url: url.clone(),
                        thumbnail: thumbnail.clone(),
                        width: aspect * 1000.0,
                        height: 1000.0,
                        duration,
                    };
                }
                PostMediaGalleryBuilderViewerMedia {
                    kind: MediaViewerGalleryItemKind::Video,
                    url: url.clone(),
                    thumbnail: thumbnail.clone(),
                    width: size.as_ref().map_or(0.0, |size| size.width as f32),
                    height: size.as_ref().map_or(0.0, |size| size.height as f32),
                    duration,
                }
            }
        }
    }

    fn parse_date(iso: &str) -> TimeInterval {
        let mut with_fraction = ISO8601DateFormatter::new();
        with_fraction.set_format_options(
            ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME | ISO8601DateFormatterOptions::WITH_FRACTIONAL_SECONDS,
        );
        if let Some(date) = with_fraction.date_from(iso) {
            return date.time_interval_since_1970();
        }
        let mut plain = ISO8601DateFormatter::new();
        plain.set_format_options(ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME);
        if let Some(date) = plain.date_from(iso) {
            return date.time_interval_since_1970();
        }
        Date::now().time_interval_since_1970()
    }
}
