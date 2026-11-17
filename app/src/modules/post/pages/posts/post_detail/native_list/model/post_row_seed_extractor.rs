use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

use gpui::App;
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::date_format::iso8601_date_formatter::{ISO8601DateFormatter, ISO8601DateFormatterOptions};
use rise_ui::rich_text::rich_text_markup::RichTextMarkup;

use crate::core::ui::formatted_text_ui::FormattedTextPreparationCachePolicy;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostRowBodyKind, PostRowFallbackReason, PostRowNativeKind, PostRowPremiumBadge, PostRowRoute, PostRowSeed,
    PostRowSeedAuthor, PostRowSeedCounts, PostRowSeedLikedBy, PostRowSeedMedia, PostRowSeedRepostOpinion,
    PostRowSeedTrack,
};
use crate::modules::post::shared::post_media_slides::PostMediaSlide;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

struct PostRowSeedExtractorMemoEntry {
    post: GetPostFeedResponse,
    viewer_id: Option<String>,
    seed: PostRowSeed,
}

thread_local! {
    static MEMO: RefCell<HashMap<String, PostRowSeedExtractorMemoEntry>> = RefCell::new(HashMap::new());

    static DATE_FORMATTERS: (ISO8601DateFormatter, ISO8601DateFormatter) = {
        let mut fractional_date_formatter = ISO8601DateFormatter::new();
        fractional_date_formatter.set_format_options(
            ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME | ISO8601DateFormatterOptions::WITH_FRACTIONAL_SECONDS,
        );
        let mut internet_date_formatter = ISO8601DateFormatter::new();
        internet_date_formatter.set_format_options(ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME);
        (fractional_date_formatter, internet_date_formatter)
    };
}

pub enum PostRowSeedExtractor {}

impl PostRowSeedExtractor {
    const MEMO_LIMIT: usize = 512;

    pub fn seed(post: &GetPostFeedResponse, cx: &mut App) -> PostRowSeed {
        let viewer_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let key = post.presentation_id();
        let memoized = MEMO.with_borrow(|memo| {
            memo.get(&key)
                .filter(|entry| {
                    entry.viewer_id == viewer_id
                        && entry.post == *post
                        && (!post.author.is_deleted || entry.seed.author.name == post.author.display_name())
                })
                .map(|entry| entry.seed.clone())
        });
        if let Some(seed) = memoized {
            return seed;
        }
        let seed = Self::make_seed(post, cx);
        MEMO.with_borrow_mut(|memo| {
            if memo.len() >= Self::MEMO_LIMIT {
                memo.clear();
            }
            memo.insert(
                key,
                PostRowSeedExtractorMemoEntry { post: post.clone(), viewer_id, seed: seed.clone() },
            );
        });
        seed
    }

    pub fn remove_all() {
        MEMO.with_borrow_mut(HashMap::clear);
    }

    /// extracts the seed with no cache
    pub fn make_seed(post: &GetPostFeedResponse, cx: &mut App) -> PostRowSeed {
        let track_model = post.track_model();
        let all_slides = post.media_slides();
        let slides = Self::remote_slides(&all_slides);
        let uses_inline_short_card = post.is_riff && post.images.is_empty() && post.has_video_media();
        let more = &post.author.more;

        let author = PostRowSeedAuthor {
            id: post.author.id.clone(),
            analytics_id: post.author.id.clone().unwrap_or_else(|| post.author_id.clone()),
            name: post.author.display_name(),
            tag: post.author.display_tag(),
            logo_url: more.logo.clone(),
            is_deleted: post.author.is_deleted,
            is_official: more.is_official,
            is_premium: more.is_premium,
            premium_badge_emoji: more.premium_badge_emoji.clone(),
            premium_name_color_rgb: if more.is_premium {
                PostRowSeedAuthor::parse_name_color(more.name_color.as_deref())
            } else {
                None
            },
            premium_badge: PostRowPremiumBadge::payload(more.is_premium, more.premium_badge_emoji.as_deref()),
        };

        let track = track_model.as_ref().map(|model| PostRowSeedTrack {
            id: model.id.clone(),
            title: model.title.clone(),
            artist: model.artist.clone(),
            is_original_audio: model.status == RiseMusicTrackStatus::OriginalAudio.wire(),
            formatted_duration: model.formatted_duration(),
            cover_url: model.cover_image_url.clone(),
        });

        let mut tagged_slide_indices = BTreeSet::new();
        for entry in post.tagged_users.iter() {
            for placement in entry.placements.iter() {
                tagged_slide_indices.insert(placement.slide_index);
            }
        }

        let slide_count_of_all = all_slides.len();
        let remote_slide_count = slides.len();
        let media = PostRowSeedMedia {
            slides,
            media_slide_count: post.media_slide_count(),
            track_plate: if track_model.is_none() { None } else { post.track_plate.clone() },
            shows_sound_bar: post.track.as_ref().and_then(|track| track.id.as_ref()).is_some_and(|id| !id.is_empty()),
            has_tagged_users: !post.tagged_users.is_empty(),
            tagged_slide_indices,
            link_areas: post.link_areas.clone(),
        };

        let viewer_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let viewer = viewer_id.as_deref();
        let counts = PostRowSeedCounts {
            likes: post.likes_count,
            favorites: post.favorites_count,
            comments: post.comments_count,
            reposts: post.repost_count,
            is_liked: post.is_liked,
            is_favorited: post.is_favorited,
            is_reposted: post.is_reposted,
            shows_likes: post.shows_count(|visibility| visibility.likes, viewer),
            shows_comments: post.shows_count(|visibility| visibility.comments, viewer),
            shows_reposts: post.shows_count(|visibility| visibility.reposts, viewer),
            shows_sparks: post.shows_sparks_button(viewer),
            shows_sparks_total: post.shows_count(|visibility| visibility.sparks, viewer),
            sparks_total: post.sparks_total,
        };

        let repost_opinion = post.is_reposted.then(|| PostRowSeedRepostOpinion {
            text: Self::non_empty_trimmed(post.my_repost_content.as_deref()).map(str::to_owned),
            has_entities: Self::has_repost_entities(post.my_repost_entities.as_deref()),
        });

        let liked_by = post
            .liked_by
            .iter()
            .take(3)
            .map(|user| PostRowSeedLikedBy {
                id: user.id.clone(),
                name: user.name.clone(),
                tag: user.tag.clone(),
                logo_url: user.logo.clone(),
            })
            .collect();

        let is_upload = post.is_temp
            || post.is_uploading
            || post.is_compressing
            || Self::non_empty_trimmed(post.local_preview_url.as_deref()).is_some()
            || remote_slide_count != slide_count_of_all;

        PostRowSeed {
            post_id: post.id.clone(),
            presentation_id: post.presentation_id(),
            view_target_post_id: post.rise_view_target_post_id(),
            body: if uses_inline_short_card { PostRowBodyKind::Reel } else { PostRowBodyKind::Standard },
            author,
            created_at: Self::date(&post.created_at),
            track,
            moderation_banner: post.moderation_banner(),
            media,
            title: post.title.clone(),
            content: post.content.clone(),
            hashtags: post.hashtags.clone(),
            caption_has_code_block: RichTextMarkup::has_code_block(&post.content),
            caption_utf16_count: post.content.encode_utf16().count() as i64,
            counts,
            repost_opinion,
            liked_by,
            has_collaborator_header: !post.coauthors.is_empty(),
            is_pending_coauthor_invite: post.is_pending_coauthor_invite(),
            shared_achievement_id: post.shared_achievement_id(),
            has_ambient_music: post.music_audio_url().is_some(),
            is_upload,
            is_own: PostMediaCachePolicy::is_own_post(post, cx),
            is_short: post.is_short,
            is_caption_expanded: false,
        }
    }

    /// native row or whole row fallback for this seed
    pub fn route(seed: &PostRowSeed) -> PostRowRoute {
        if let Some(reason) = Self::fallback_reason(seed) {
            return PostRowRoute::HostedFallback(reason);
        }
        PostRowRoute::Native(Self::native_kind(seed))
    }

    pub fn fallback_reason(seed: &PostRowSeed) -> Option<PostRowFallbackReason> {
        if seed.is_upload {
            return Some(PostRowFallbackReason::Upload);
        }
        if seed.has_collaborator_header {
            return Some(PostRowFallbackReason::CollaboratorHeader);
        }
        if seed.shared_achievement_id.is_some() {
            return Some(PostRowFallbackReason::Achievement);
        }
        if seed.is_pending_coauthor_invite {
            return Some(PostRowFallbackReason::CoauthorInvite);
        }
        if seed.caption_has_code_block {
            return Some(PostRowFallbackReason::CodeBlock);
        }
        if seed.caption_utf16_count > FormattedTextPreparationCachePolicy::maximum_cacheable_utf16_count() {
            return Some(PostRowFallbackReason::OversizeCaption);
        }
        if seed.body == PostRowBodyKind::Reel && !seed.liked_by.is_empty() && seed.counts.likes <= 0 {
            return Some(PostRowFallbackReason::Unknown);
        }
        if seed.body == PostRowBodyKind::Standard && seed.media.media_slide_count > 0 && seed.media.slides.is_empty() {
            return Some(PostRowFallbackReason::Unknown);
        }
        None
    }

    /// native class this seed would use if there was no fallback reason
    pub fn native_kind(seed: &PostRowSeed) -> PostRowNativeKind {
        if seed.body == PostRowBodyKind::Reel {
            return PostRowNativeKind::RiffCard;
        }
        if seed.shared_achievement_id.is_none() && seed.media.media_slide_count > 0 && !seed.media.slides.is_empty() {
            return if seed.media.slides.iter().any(PostMediaSlide::is_video) {
                PostRowNativeKind::VideoGallery
            } else {
                PostRowNativeKind::PhotoGallery
            };
        }
        if seed.shows_music_strip() {
            return PostRowNativeKind::MusicStrip;
        }
        PostRowNativeKind::Plain
    }

    pub fn remote_slides(slides: &[PostMediaSlide]) -> Vec<PostMediaSlide> {
        slides
            .iter()
            .filter_map(|slide| match slide {
                PostMediaSlide::Image { url, size, image_index } => {
                    let trimmed = trimming_whitespaces_and_newlines(url);
                    if !trimmed.to_lowercase().starts_with("http") {
                        return None;
                    }
                    Some(PostMediaSlide::Image {
                        url: trimmed.to_owned(),
                        size: size.clone(),
                        image_index: *image_index,
                    })
                }
                PostMediaSlide::Video { url, .. } => url.to_lowercase().starts_with("http").then(|| slide.clone()),
            })
            .collect()
    }

    /// parses iso 8601 date with or without fractional seconds
    fn date(raw: &str) -> Option<Date> {
        let value = trimming_whitespaces_and_newlines(raw);
        if value.is_empty() {
            return None;
        }
        DATE_FORMATTERS.with(|(fractional_date_formatter, internet_date_formatter)| {
            fractional_date_formatter.date_from(value).or_else(|| internet_date_formatter.date_from(value))
        })
    }

    fn non_empty_trimmed(raw: Option<&str>) -> Option<&str> {
        let value = trimming_whitespaces_and_newlines(raw?);
        if value.is_empty() { None } else { Some(value) }
    }

    fn has_repost_entities(raw: Option<&str>) -> bool {
        let Some(value) = Self::non_empty_trimmed(raw) else {
            return false;
        };
        value != "[]" && value != "{}" && value != "null"
    }
}
