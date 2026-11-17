use std::any::Any;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyElement, App, Bounds, Context, Entity, Global, Hsla, IntoElement, ParentElement, Pixels, Point, Rgba, Size, Window, px,
    rgb, size,
};
use parking_lot::Mutex;
use rise_canon::rise_proto::post::PostLinkArea;
use rise_core::foundation::foundation_calendar::{Calendar, TimeZone};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use rise_i18n::language_store::Locale;
use rise_theme::theme::app_theme::ThemePaletteReadable;
use rise_theme::{ThemePalette, ThemeStore, color};
use rise_ui::swiftui::environment::env_scope;
use rise_ui::swiftui::open_url::{OpenURLAction, OpenURLKey};
use rise_ui::window_screen_frame_reader::WindowScreenFrameReaderFrameView;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerPresentationKey, MediaViewerPresentationState,
};
use crate::core::shared_element_transition::kernel::shared_element_types::{SharedElementPart, SharedElementSurface};
use crate::core::shared_element_transition::runtime::shared_element_lift_board::SharedElementSurfaceKey;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{
    RiseManualListItemMountPolicy, RiseManualListReusableView,
};
use crate::modules::chat::stores::message::message_actions::message_types::MessageReaction;
use crate::modules::post::components::repost::repost_overlay::{
    RepostTransitionNamespace, RepostTransitionNamespaceKey,
};
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_playback_focus::PostListFocusFrameView;
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_row_action_router::PostRowMediaPageSource;
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_layout_calculator::PostRowMetrics;
use crate::modules::post::shared::post_media_slides::PostMediaSlide;
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_row_action_router::PostRowMenuItem;
use crate::modules::post::stores::post_actions::post_types::{
    GetPostFeedResponse, PostModerationBanner, PostTrackPlateDTO,
};
use crate::modules::shorts::components::shorts_zoom_namespace::{ShortsZoomNamespaceId, ShortsZoomNamespaceKey};

fn hash_f32<H: Hasher>(value: f32, state: &mut H) {
    let normalized = if value == 0.0 { 0.0_f32 } else { value };
    normalized.to_bits().hash(state);
}

fn hash_f64<H: Hasher>(value: f64, state: &mut H) {
    let normalized = if value == 0.0 { 0.0_f64 } else { value };
    normalized.to_bits().hash(state);
}

fn hash_track_plate<H: Hasher>(plate: Option<&PostTrackPlateDTO>, state: &mut H) {
    plate.is_some().hash(state);
    if let Some(plate) = plate {
        hash_f64(plate.center_x, state);
        hash_f64(plate.center_y, state);
        plate.is_dark.hash(state);
        hash_f64(plate.rotation, state);
        hash_f64(plate.scale, state);
    }
}

pub enum PostListReuseIdentifier {}

impl PostListReuseIdentifier {
    pub const NATIVE_PLAIN: &'static str = "post.native.plain";
    pub const NATIVE_GALLERY: &'static str = "post.native.gallery";
    pub const NATIVE_RIFF: &'static str = "post.native.riff";
    pub const HOSTED: &'static str = "post.hosted";
}

/// which native row class draws a post
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostRowNativeKind {
    Plain,
    PhotoGallery,
    VideoGallery,
    RiffCard,
    MusicStrip,
}

/// why a post falls back to the hosted Post instead of a native row
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostRowFallbackReason {
    CollaboratorHeader,
    Achievement,
    CoauthorInvite,
    CodeBlock,
    Upload,
    OversizeCaption,
    Unknown,
}

impl PostRowFallbackReason {
    pub const ALL: [PostRowFallbackReason; 7] = [
        PostRowFallbackReason::CollaboratorHeader,
        PostRowFallbackReason::Achievement,
        PostRowFallbackReason::CoauthorInvite,
        PostRowFallbackReason::CodeBlock,
        PostRowFallbackReason::Upload,
        PostRowFallbackReason::OversizeCaption,
        PostRowFallbackReason::Unknown,
    ];

    pub fn raw_value(self) -> &'static str {
        match self {
            PostRowFallbackReason::CollaboratorHeader => "collaboratorHeader",
            PostRowFallbackReason::Achievement => "achievement",
            PostRowFallbackReason::CoauthorInvite => "coauthorInvite",
            PostRowFallbackReason::CodeBlock => "codeBlock",
            PostRowFallbackReason::Upload => "upload",
            PostRowFallbackReason::OversizeCaption => "oversizeCaption",
            PostRowFallbackReason::Unknown => "unknown",
        }
    }
}

/// native row or whole row hosted fallback
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostRowRoute {
    Native(PostRowNativeKind),
    HostedFallback(PostRowFallbackReason),
}

impl PostRowRoute {
    pub fn reuse_identifier(&self) -> &'static str {
        match self {
            PostRowRoute::Native(PostRowNativeKind::Plain | PostRowNativeKind::MusicStrip) => {
                PostListReuseIdentifier::NATIVE_PLAIN
            }
            PostRowRoute::Native(PostRowNativeKind::PhotoGallery | PostRowNativeKind::VideoGallery) => {
                PostListReuseIdentifier::NATIVE_GALLERY
            }
            PostRowRoute::Native(PostRowNativeKind::RiffCard) => PostListReuseIdentifier::NATIVE_RIFF,
            PostRowRoute::HostedFallback(_) => PostListReuseIdentifier::HOSTED,
        }
    }

    pub fn is_hosted_fallback(&self) -> bool {
        matches!(self, PostRowRoute::HostedFallback(_))
    }

    pub fn mounts_deferred_during_fast_scroll(&self) -> bool {
        match self {
            PostRowRoute::Native(PostRowNativeKind::Plain | PostRowNativeKind::MusicStrip) => false,
            PostRowRoute::Native(
                PostRowNativeKind::PhotoGallery | PostRowNativeKind::VideoGallery | PostRowNativeKind::RiffCard,
            )
            | PostRowRoute::HostedFallback(_) => true,
        }
    }

    pub fn mount_policy(&self) -> RiseManualListItemMountPolicy {
        if self.mounts_deferred_during_fast_scroll() {
            RiseManualListItemMountPolicy::DeferredDuringFastScroll
        } else {
            RiseManualListItemMountPolicy::Immediate
        }
    }
}

/// premium marker after the author name, one case per badge kind
#[derive(Clone, Debug, Default, PartialEq)]
pub enum PostRowPremiumBadge {
    #[default]
    None,
    Diamond,
    Unicode(String),
    UnresolvedCustom { id: String },
    MissedCustom { id: String },
    Custom { id: String, width: f32, height: f32 },
}

impl Eq for PostRowPremiumBadge {}

impl Hash for PostRowPremiumBadge {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            PostRowPremiumBadge::None | PostRowPremiumBadge::Diamond => {}
            PostRowPremiumBadge::Unicode(value) => value.hash(state),
            PostRowPremiumBadge::UnresolvedCustom { id } | PostRowPremiumBadge::MissedCustom { id } => id.hash(state),
            PostRowPremiumBadge::Custom { id, width, height } => {
                id.hash(state);
                hash_f32(*width, state);
                hash_f32(*height, state);
            }
        }
    }
}

impl PostRowPremiumBadge {
    /// builds the badge from the wire payload only
    pub fn payload(is_premium: bool, emoji: Option<&str>) -> PostRowPremiumBadge {
        if !is_premium {
            return PostRowPremiumBadge::None;
        }
        let value = trimming_whitespaces_and_newlines(emoji.unwrap_or(""));
        if value.is_empty() {
            return PostRowPremiumBadge::Diamond;
        }
        match value.strip_prefix(MessageReaction::CUSTOM_EMOJI_PREFIX) {
            Some(id) => PostRowPremiumBadge::UnresolvedCustom { id: id.to_owned() },
            None => PostRowPremiumBadge::Unicode(value.to_owned()),
        }
    }

    /// badge frame in the first header line, none without a badge
    pub fn frame_size(&self) -> Option<Size<Pixels>> {
        match self {
            PostRowPremiumBadge::None => None,
            PostRowPremiumBadge::Diamond
            | PostRowPremiumBadge::Unicode(_)
            | PostRowPremiumBadge::UnresolvedCustom { .. }
            | PostRowPremiumBadge::MissedCustom { .. } => {
                Some(size(px(PostRowMetrics::BADGE_SIZE), px(PostRowMetrics::BADGE_SIZE)))
            }
            PostRowPremiumBadge::Custom { width, height, .. } => Some(size(px(*width), px(*height))),
        }
    }

    /// custom emoji sticker id, resolved or not
    pub fn custom_emoji_id(&self) -> Option<&str> {
        match self {
            PostRowPremiumBadge::UnresolvedCustom { id }
            | PostRowPremiumBadge::MissedCustom { id }
            | PostRowPremiumBadge::Custom { id, .. } => Some(id),
            PostRowPremiumBadge::None | PostRowPremiumBadge::Diamond | PostRowPremiumBadge::Unicode(_) => None,
        }
    }

    pub fn is_unresolved_custom(&self) -> bool {
        match self {
            PostRowPremiumBadge::UnresolvedCustom { .. } | PostRowPremiumBadge::MissedCustom { .. } => true,
            PostRowPremiumBadge::None
            | PostRowPremiumBadge::Diamond
            | PostRowPremiumBadge::Unicode(_)
            | PostRowPremiumBadge::Custom { .. } => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostRowBodyKind {
    Standard,
    Reel,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PostRowSeedAuthor {
    pub id: Option<String>,
    pub analytics_id: String,
    pub name: String,
    pub tag: String,
    pub logo_url: String,
    pub is_deleted: bool,
    pub is_official: bool,
    pub is_premium: bool,
    pub premium_badge_emoji: Option<String>,
    pub premium_name_color_rgb: Option<u32>,
    pub premium_badge: PostRowPremiumBadge,
}

impl PostRowSeedAuthor {
    /// parses a name color as rgb, only #RRGGBB works
    pub fn parse_name_color(raw: Option<&str>) -> Option<u32> {
        let raw = trimming_whitespaces_and_newlines(raw?);
        if raw.chars().count() != 7 {
            return None;
        }
        u32::from_str_radix(raw.strip_prefix('#')?, 16).ok()
    }
}

/// attached track, there only when the post has a track model
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PostRowSeedTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub is_original_audio: bool,
    pub formatted_duration: String,
    pub cover_url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PostRowSeedMedia {
    pub slides: Vec<PostMediaSlide>,
    pub media_slide_count: i64,
    pub track_plate: Option<PostTrackPlateDTO>,
    pub shows_sound_bar: bool,
    pub has_tagged_users: bool,
    pub tagged_slide_indices: BTreeSet<i64>,
    pub link_areas: Vec<PostLinkArea>,
}

impl Eq for PostRowSeedMedia {}

impl Hash for PostRowSeedMedia {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.slides.hash(state);
        self.media_slide_count.hash(state);
        hash_track_plate(self.track_plate.as_ref(), state);
        self.shows_sound_bar.hash(state);
        self.has_tagged_users.hash(state);
        self.tagged_slide_indices.hash(state);
        self.link_areas.len().hash(state);
        for area in self.link_areas.iter() {
            area.url.hash(state);
            area.media_index.hash(state);
            for value in [area.center_x, area.center_y, area.width, area.height, area.rotation] {
                value.to_bits().hash(state);
            }
        }
    }
}

/// counts and toggle states, they bump content_version but never change frames
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PostRowSeedCounts {
    pub likes: i64,
    pub favorites: i64,
    pub comments: i64,
    pub reposts: i64,
    pub is_liked: bool,
    pub is_favorited: bool,
    pub is_reposted: bool,
    pub shows_likes: bool,
    pub shows_comments: bool,
    pub shows_reposts: bool,
    pub shows_sparks: bool,
    pub shows_sparks_total: bool,
    pub sparks_total: i64,
}

/// viewers own repost opinion, only set when reposted
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PostRowSeedRepostOpinion {
    pub text: Option<String>,
    pub has_entities: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PostRowSeedLikedBy {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub logo_url: String,
}

/// everything a row needs to render, pulled from the feed response and nothing else
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PostRowSeed {
    pub post_id: String,
    pub presentation_id: String,
    pub view_target_post_id: String,
    pub body: PostRowBodyKind,
    pub author: PostRowSeedAuthor,
    pub created_at: Option<Date>,
    pub track: Option<PostRowSeedTrack>,
    pub moderation_banner: Option<PostModerationBanner>,
    pub media: PostRowSeedMedia,
    pub title: String,
    pub content: String,
    pub hashtags: Vec<String>,
    pub caption_has_code_block: bool,
    pub caption_utf16_count: i64,
    pub counts: PostRowSeedCounts,
    pub repost_opinion: Option<PostRowSeedRepostOpinion>,
    pub liked_by: Vec<PostRowSeedLikedBy>,
    pub has_collaborator_header: bool,
    pub is_pending_coauthor_invite: bool,
    pub shared_achievement_id: Option<i64>,
    pub has_ambient_music: bool,
    pub is_upload: bool,
    pub is_own: bool,
    pub is_short: bool,
    pub is_caption_expanded: bool,
}

impl PostRowSeed {
    pub fn has_written_content(&self) -> bool {
        !self.title.is_empty() || !self.content.is_empty() || !self.hashtags.is_empty()
    }

    /// true when the likes summary shows, standard and reel bodies differ
    pub fn shows_likes_summary(&self) -> bool {
        match self.body {
            PostRowBodyKind::Standard => !self.liked_by.is_empty() && self.counts.likes > 0,
            PostRowBodyKind::Reel => !self.liked_by.is_empty(),
        }
    }

    /// standard body with a track and no achievement or media
    pub fn shows_music_strip(&self) -> bool {
        self.body == PostRowBodyKind::Standard
            && self.shared_achievement_id.is_none()
            && self.media.media_slide_count == 0
            && self.track.is_some()
    }

    /// true with media or achievement, caption top padding is 8 instead of 5
    pub fn has_visual_media(&self) -> bool {
        self.media.media_slide_count > 0 || self.shared_achievement_id.is_some()
    }
}

/// environment facts every row layout depends on, controller builds it on main
#[derive(Clone, Debug, PartialEq)]
pub struct PostRowLayoutSpec {
    pub width: f32,
    pub screen_width: f32,
    pub screen_height: f32,
    pub display_scale: f32,
    pub text_delta: f32,
    pub locale_identifier: String,
    pub theme_id: String,
    pub is_bold_text_enabled: bool,
}

impl Eq for PostRowLayoutSpec {}

impl Hash for PostRowLayoutSpec {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_f32(self.width, state);
        hash_f32(self.screen_width, state);
        hash_f32(self.screen_height, state);
        hash_f32(self.display_scale, state);
        hash_f32(self.text_delta, state);
        self.locale_identifier.hash(state);
        self.theme_id.hash(state);
        self.is_bold_text_enabled.hash(state);
    }
}

impl PostRowLayoutSpec {
    pub fn new(
        width: f32,
        screen_size: Size<Pixels>,
        display_scale: f32,
        text_delta: f32,
        locale_identifier: String,
        theme_id: String,
        is_bold_text_enabled: bool,
    ) -> Self {
        Self {
            width,
            screen_width: f32::from(screen_size.width),
            screen_height: f32::from(screen_size.height),
            display_scale,
            text_delta,
            locale_identifier,
            theme_id,
            is_bold_text_enabled,
        }
    }

    /// screen size for the media box clamp
    pub fn screen_size(&self) -> Size<Pixels> {
        size(px(self.screen_width), px(self.screen_height))
    }
}

/// natural aspect (width over height) of each remote slide
#[derive(Clone, Debug, PartialEq)]
pub struct PostRowMediaAspects {
    pub natural: Vec<Option<f32>>,
    pub is_provisional: bool,
}

impl Eq for PostRowMediaAspects {}

impl Hash for PostRowMediaAspects {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.natural.len().hash(state);
        for aspect in self.natural.iter() {
            aspect.is_some().hash(state);
            if let Some(aspect) = aspect {
                hash_f32(*aspect, state);
            }
        }
        self.is_provisional.hash(state);
    }
}

impl PostRowMediaAspects {
    pub fn empty() -> Self {
        Self { natural: Vec::new(), is_provisional: false }
    }

    /// aspect of slide at, none if unknown or out of range
    pub fn aspect(&self, at: usize) -> Option<f32> {
        self.natural.get(at).copied().flatten()
    }
}

/// every input that changes a frame of the row layout, so the layout cache key
#[derive(Clone, Debug, PartialEq)]
pub struct PostRowLayoutInputs {
    pub spec: PostRowLayoutSpec,
    pub route: PostRowRoute,
    pub body: PostRowBodyKind,
    pub author_name: String,
    pub has_meta_line: bool,
    pub has_official_badge: bool,
    pub has_premium_badge: bool,
    pub premium_badge_width: f32,
    pub premium_badge_height: f32,
    pub has_header_track_line: bool,
    pub moderation_banner: Option<PostModerationBanner>,
    pub shows_music_strip: bool,
    pub has_visual_media: bool,
    pub slide_is_video: Vec<bool>,
    pub slide_aspects: Vec<Option<f32>>,
    pub is_aspect_provisional: bool,
    pub shows_sound_bar: bool,
    pub track_plate: Option<PostTrackPlateDTO>,
    pub has_tags_chip: bool,
    pub is_reposted: bool,
    pub title: String,
    pub content: String,
    pub hashtags: Vec<String>,
    pub shows_likes_summary: bool,
    pub liked_by_avatar_count: usize,
    pub shows_sparks: bool,
    pub is_caption_expanded: bool,
}

impl Eq for PostRowLayoutInputs {}

impl Hash for PostRowLayoutInputs {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.spec.hash(state);
        self.route.hash(state);
        self.body.hash(state);
        self.author_name.hash(state);
        self.has_meta_line.hash(state);
        self.has_official_badge.hash(state);
        self.has_premium_badge.hash(state);
        hash_f32(self.premium_badge_width, state);
        hash_f32(self.premium_badge_height, state);
        self.has_header_track_line.hash(state);
        self.moderation_banner.hash(state);
        self.shows_music_strip.hash(state);
        self.has_visual_media.hash(state);
        self.slide_is_video.hash(state);
        self.slide_aspects.len().hash(state);
        for aspect in self.slide_aspects.iter() {
            aspect.is_some().hash(state);
            if let Some(aspect) = aspect {
                hash_f32(*aspect, state);
            }
        }
        self.is_aspect_provisional.hash(state);
        self.shows_sound_bar.hash(state);
        hash_track_plate(self.track_plate.as_ref(), state);
        self.has_tags_chip.hash(state);
        self.is_reposted.hash(state);
        self.title.hash(state);
        self.content.hash(state);
        self.hashtags.hash(state);
        self.shows_likes_summary.hash(state);
        self.liked_by_avatar_count.hash(state);
        self.shows_sparks.hash(state);
        self.is_caption_expanded.hash(state);
    }
}

impl PostRowLayoutInputs {
    /// the projection all layout producers and consumers use
    pub fn new(
        seed: &PostRowSeed,
        route: PostRowRoute,
        spec: PostRowLayoutSpec,
        aspects: &PostRowMediaAspects,
    ) -> Self {
        let premium_size = seed.author.premium_badge.frame_size();
        Self {
            spec,
            route,
            body: seed.body,
            author_name: seed.author.name.clone(),
            has_meta_line: !seed.author.tag.is_empty() || seed.created_at.is_some(),
            has_official_badge: seed.author.is_official,
            has_premium_badge: premium_size.is_some(),
            premium_badge_width: premium_size.map_or(0.0, |premium_size| f32::from(premium_size.width)),
            premium_badge_height: premium_size.map_or(0.0, |premium_size| f32::from(premium_size.height)),
            has_header_track_line: seed.track.is_some(),
            moderation_banner: seed.moderation_banner,
            shows_music_strip: seed.shows_music_strip(),
            has_visual_media: seed.has_visual_media(),
            slide_is_video: seed.media.slides.iter().map(PostMediaSlide::is_video).collect(),
            slide_aspects: (0..seed.media.slides.len()).map(|index| aspects.aspect(index)).collect(),
            is_aspect_provisional: !seed.media.slides.is_empty() && aspects.is_provisional,
            shows_sound_bar: seed.media.shows_sound_bar,
            track_plate: seed.media.track_plate.clone(),
            has_tags_chip: seed.media.has_tagged_users,
            is_reposted: seed.counts.is_reposted,
            title: seed.title.clone(),
            content: seed.content.clone(),
            hashtags: seed.hashtags.clone(),
            shows_likes_summary: seed.shows_likes_summary(),
            liked_by_avatar_count: seed.liked_by.len().min(3),
            shows_sparks: seed.counts.shows_sparks,
            is_caption_expanded: seed.is_caption_expanded,
        }
    }
}

/// what a tap on the token does
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PostTextTokenKind {
    Mention { tag: String, url: Url },
    Url(Url),
    Hashtag(String),
}

/// a mention, url or hashtag inside a prepared text layout
#[derive(Clone, Debug, PartialEq)]
pub struct PostTextToken {
    pub kind: PostTextTokenKind,
    pub utf16_range: Range<usize>,
    pub rects: Vec<Bounds<Pixels>>,
}

/// immutable text layout of one block, prepared off main
pub trait PostTextLayouting: Send + Sync {
    /// size rounded up to whole pixels, the block frame size
    fn size(&self) -> Size<Pixels>;
    fn line_count(&self) -> usize;
    /// the laid out string, for accessibility and token ranges
    fn plain_text(&self) -> &str;
    fn tokens(&self) -> &[PostTextToken];
    /// paints with the top left at origin, colors come from attributes
    fn draw(&self, origin: Point<Pixels>, window: &mut Window, cx: &mut App);
    fn as_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;

    /// first token whose rect grown by slop contains at, in layout coords
    fn token(&self, at: Point<Pixels>, slop: f32) -> Option<&PostTextToken> {
        self.tokens().iter().find(|token| token.rects.iter().any(|rect| rect.dilate(px(slop)).contains(&at)))
    }
}

/// header part, nested frames are local to frame
#[derive(Clone, Debug)]
pub struct PostRowHeaderLayout {
    pub frame: Bounds<Pixels>,
    pub avatar: Bounds<Pixels>,
    pub text_column: Bounds<Pixels>,
    pub name: Bounds<Pixels>,
    pub official_badge: Option<Bounds<Pixels>>,
    pub premium_badge: Option<Bounds<Pixels>>,
    pub meta_line: Option<Bounds<Pixels>>,
    pub music_line: Option<Bounds<Pixels>>,
    pub more_button: Bounds<Pixels>,
}

/// moderation banner part, nested frames are local to frame
#[derive(Clone, Debug)]
pub struct PostRowModerationBannerLayout {
    pub banner: PostModerationBanner,
    pub frame: Bounds<Pixels>,
    pub icon: Bounds<Pixels>,
    pub label: Bounds<Pixels>,
    pub lines: Vec<String>,
}

/// music strip part, nested frames are local to frame
#[derive(Clone, Debug)]
pub struct PostRowMusicStripLayout {
    pub frame: Bounds<Pixels>,
    pub cover: Bounds<Pixels>,
    pub title_line: Bounds<Pixels>,
    pub duration_line: Bounds<Pixels>,
    pub sound_button: Bounds<Pixels>,
}

/// one media page
#[derive(Clone, Debug)]
pub struct PostRowSlideLayout {
    pub natural_aspect: Option<f32>,
    pub fills: bool,
    pub is_video: bool,
}

/// music plate transform over the box
#[derive(Clone, Debug)]
pub struct PostRowMusicPlateLayout {
    pub center: Point<Pixels>,
    pub base_width: f32,
    pub scale: f32,
    pub rotation: f32,
    pub is_dark: bool,
}

/// media box part, nested frames are local to frame
#[derive(Clone, Debug)]
pub struct PostRowMediaBoxLayout {
    pub frame: Bounds<Pixels>,
    pub box_aspect: f32,
    pub is_aspect_provisional: bool,
    pub slides: Vec<PostRowSlideLayout>,
    pub page_indicator: Option<Bounds<Pixels>>,
    pub sound_button: Option<Bounds<Pixels>>,
    pub corner_row_origin: Point<Pixels>,
    pub music_plate: Option<PostRowMusicPlateLayout>,
}

/// one prepared text block, frame is local to its caption
#[derive(Clone)]
pub struct PostRowTextBlockLayout {
    pub frame: Bounds<Pixels>,
    pub text: Arc<dyn PostTextLayouting>,
}

/// one rich paragraph with its optional link subtitle
#[derive(Clone)]
pub struct PostRowParagraphLayout {
    pub body: PostRowTextBlockLayout,
    pub link_subtitle: Option<PostRowTextBlockLayout>,
}

/// caption part, block frames are local to frame
#[derive(Clone)]
pub struct PostRowCaptionLayout {
    pub frame: Bounds<Pixels>,
    pub title: Option<PostRowTextBlockLayout>,
    pub paragraphs: Vec<PostRowParagraphLayout>,
    pub hashtags: Option<PostRowTextBlockLayout>,
    pub toggle: Option<PostRowTextBlockLayout>,
}

/// actions part, cell frames are local to frame
#[derive(Clone, Debug)]
pub struct PostRowActionsLayout {
    pub frame: Bounds<Pixels>,
    pub like: Bounds<Pixels>,
    pub favorite: Bounds<Pixels>,
    pub comments: Bounds<Pixels>,
    pub repost: Bounds<Pixels>,
    pub sparks: Option<Bounds<Pixels>>,
}

/// likes summary part, nested frames are local to frame
#[derive(Clone, Debug)]
pub struct PostRowLikesSummaryLayout {
    pub frame: Bounds<Pixels>,
    pub avatars: Vec<Bounds<Pixels>>,
    pub text: Bounds<Pixels>,
}

/// frames of one native row, only the layout calculator makes them and views just assign
#[derive(Clone)]
pub struct PostRowLayout {
    pub inputs: PostRowLayoutInputs,
    pub header: PostRowHeaderLayout,
    pub moderation_banner: Option<PostRowModerationBannerLayout>,
    pub music_strip: Option<PostRowMusicStripLayout>,
    pub media_box: Option<PostRowMediaBoxLayout>,
    pub caption: Option<PostRowCaptionLayout>,
    pub actions: PostRowActionsLayout,
    pub likes_summary: Option<PostRowLikesSummaryLayout>,
    pub separator: Bounds<Pixels>,
    pub content_height: f32,
    pub total_height: f32,
}

impl PostRowLayout {
    pub const INTER_ITEM_SPACING: f32 = 5.0;
    pub const SEPARATOR_GAP: f32 = 5.0;
    pub const SEPARATOR_THICKNESS: f32 = 1.0;
}

/// key of the prepared row cache, hosted height cache uses it too
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PreparedPostRowKey {
    pub presentation_id: String,
    pub content_version: i64,
    pub spec: PostRowLayoutSpec,
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
}

/// one row ready for the engine with route, seed and exact or estimated height
pub struct PreparedPostRow {
    pub id: String,
    pub presentation_id: String,
    pub route: PostRowRoute,
    pub seed: PostRowSeed,
    pub post: GetPostFeedResponse,
    pub layout: Option<Arc<PostRowLayout>>,
    pub hosted_height: Option<f32>,
    pub content_version: i64,
    pub is_height_estimated: bool,
    pub key: PreparedPostRowKey,
}

impl PreparedPostRow {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        seed: PostRowSeed,
        route: PostRowRoute,
        post: GetPostFeedResponse,
        layout: Option<Arc<PostRowLayout>>,
        hosted_height: Option<f32>,
        content_version: i64,
        is_height_estimated: bool,
        key: PreparedPostRowKey,
    ) -> Self {
        Self {
            id: seed.post_id.clone(),
            presentation_id: seed.presentation_id.clone(),
            route,
            seed,
            post,
            layout,
            hosted_height,
            content_version,
            is_height_estimated,
            key,
        }
    }

    /// engine item height, always finite and positive
    pub fn height(&self) -> f32 {
        let value = self.layout.as_ref().map(|layout| layout.total_height).or(self.hosted_height).unwrap_or(1.0);
        if value.is_finite() { value.max(1.0) } else { 1.0 }
    }

    pub fn reuse_identifier(&self) -> &'static str {
        self.route.reuse_identifier()
    }
}

struct PostRowContentVersionTableEntry {
    seed: PostRowSeed,
    version: i64,
}

/// captions the viewer expanded with more
/// by presentation id, lives for the app session
#[derive(Default)]
pub struct PostCaptionExpansion {
    expanded: std::collections::HashSet<String>,
}

impl Global for PostCaptionExpansion {}

impl PostCaptionExpansion {
    const ENTRY_LIMIT: usize = 1_024;

    pub fn is_expanded(presentation_id: &str, cx: &App) -> bool {
        cx.try_global::<Self>().is_some_and(|expansion| expansion.expanded.contains(presentation_id))
    }

    pub fn toggle(presentation_id: &str, cx: &mut App) {
        let expansion = cx.default_global::<Self>();
        if !expansion.expanded.remove(presentation_id) {
            if expansion.expanded.len() >= Self::ENTRY_LIMIT {
                expansion.expanded.clear();
            }
            expansion.expanded.insert(presentation_id.to_owned());
        }
    }
}

struct PostRowContentVersionTableState {
    entries: HashMap<String, PostRowContentVersionTableEntry>,
    next_version: i64,
}

/// process wide content versions, same version means same seed
pub struct PostRowContentVersionTable {
    state: Mutex<PostRowContentVersionTableState>,
    entry_limit: usize,
}

static SHARED_CONTENT_VERSION_TABLE: LazyLock<PostRowContentVersionTable> =
    LazyLock::new(PostRowContentVersionTable::default);

impl Default for PostRowContentVersionTable {
    fn default() -> Self {
        Self::new(Self::DEFAULT_ENTRY_LIMIT)
    }
}

impl PostRowContentVersionTable {
    const DEFAULT_ENTRY_LIMIT: usize = 4_096;

    pub fn shared() -> &'static PostRowContentVersionTable {
        &SHARED_CONTENT_VERSION_TABLE
    }

    pub fn new(entry_limit: usize) -> Self {
        Self {
            state: Mutex::new(PostRowContentVersionTableState { entries: HashMap::new(), next_version: 1 }),
            entry_limit: entry_limit.max(1),
        }
    }

    /// version of the seed, callable from any thread
    pub fn version(&self, seed: &PostRowSeed) -> i64 {
        let mut state = self.state.lock();
        if let Some(entry) = state.entries.get(&seed.presentation_id)
            && entry.seed == *seed
        {
            return entry.version;
        }
        if state.entries.len() >= self.entry_limit {
            state.entries.clear();
        }
        let version = state.next_version;
        state.next_version = version.wrapping_add(1);
        state
            .entries
            .insert(seed.presentation_id.clone(), PostRowContentVersionTableEntry { seed: seed.clone(), version });
        version
    }

    pub fn remove_all(&self) {
        self.state.lock().entries.clear();
    }
}

/// resolved theme colors for rows and text builders, done once per palette
#[derive(Clone, Debug, PartialEq)]
pub struct PostRowPalette {
    pub theme_id: String,
    pub text_primary: Hsla,
    pub text_secondary: Hsla,
    pub background100: Hsla,
    pub background300: Hsla,
    pub border100: Hsla,
    pub primary100: Hsla,
    pub primary200: Hsla,
    pub primary300: Hsla,
    pub error100: Hsla,
    pub link: Hsla,
    pub official_badge: Hsla,
    pub story_ring_seen: Hsla,
}

impl PostRowPalette {
    pub fn new(palette: &ThemePalette) -> Self {
        Self {
            theme_id: [
                palette.text_100.as_str(),
                palette.secondary_100.as_str(),
                palette.bg_100.as_str(),
                palette.bg_300.as_str(),
                palette.border_100.as_str(),
                palette.primary_100.as_str(),
                palette.primary_200.as_str(),
                palette.primary_300.as_str(),
                palette.error_100.as_str(),
            ]
            .join("|"),
            text_primary: palette.readable_text_100(),
            text_secondary: palette.readable_secondary_100(),
            background100: color(&palette.bg_100),
            background300: palette.readable_surface(&palette.bg_300),
            border100: palette.readable_surface(&palette.border_100),
            primary100: color(&palette.primary_100),
            primary200: color(&palette.primary_200),
            primary300: color(&palette.primary_300),
            error100: color(&palette.error_100),
            link: Rgba { r: 0.07, g: 0.45, b: 0.94, a: 1.0 }.into(),
            official_badge: Rgba { r: 1.0, g: 0.353, b: 0.353, a: 1.0 }.into(),
            story_ring_seen: Rgba { r: 0.24, g: 0.24, b: 0.26, a: 1.0 }.into(),
        }
    }

    /// premium name color from the seed author rgb
    pub fn premium_name_color(value: u32) -> Hsla {
        rgb(value).into()
    }
}

/// viewers live premium facts, they override the seed on their own posts
#[derive(Clone, Debug, PartialEq)]
pub struct PostRowOwnPremium {
    pub user_id: String,
    pub is_premium: bool,
    pub name_color: Option<Hsla>,
    pub badge_emoji: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostRowStoryRingState {
    NoStories,
    Unseen,
    Seen,
    Unknown,
}

/// environment a hosted leaf or fallback row needs
#[derive(Clone)]
pub struct PostListHostedEnvironment {
    pub navigation_store: Entity<NavigationStore>,
    pub media_viewer_presentation: Entity<MediaViewerPresentationState>,
    pub open_url: OpenURLAction,
    pub theme_store: Entity<ThemeStore>,
    pub repost_transition_namespace: Option<RepostTransitionNamespace>,
    pub shorts_zoom_namespace: Option<ShortsZoomNamespaceId>,
    pub locale: Locale,
    pub calendar: Calendar,
    pub time_zone: TimeZone,
    pub shared_element_surface: Option<SharedElementSurface>,
}

pub trait PostListHostedEnvironmentModifiers: IntoElement + Sized {
    /// injects every subtree key of the hosted environment
    fn applying_post_list_hosted_environment(self, environment: &PostListHostedEnvironment) -> AnyElement {
        env_scope::<NavigationStoreKey>(Some(environment.navigation_store.clone()))
            .child(
                env_scope::<MediaViewerPresentationKey>(Some(environment.media_viewer_presentation.clone())).child(
                    env_scope::<OpenURLKey>(environment.open_url.clone()).child(
                        env_scope::<RepostTransitionNamespaceKey>(environment.repost_transition_namespace.clone())
                            .child(
                                env_scope::<ShortsZoomNamespaceKey>(environment.shorts_zoom_namespace).child(
                                    env_scope::<SharedElementSurfaceKey>(environment.shared_element_surface.clone())
                                        .child(self),
                                ),
                            ),
                    ),
                ),
            )
            .into_any_element()
    }
}

impl<E: IntoElement> PostListHostedEnvironmentModifiers for E {}

pub type PostRowMakeAnchorRegistration = Rc<dyn Fn(&mut App) -> Rc<dyn PostRowAnchorRegistering>>;
pub type PostRowStoryRingStateReader = Rc<dyn Fn(&str, &App) -> PostRowStoryRingState>;

/// main thread snapshot handed to every configure, rebuilt when a field changes
#[derive(Clone)]
pub struct PostRowMainContext {
    pub session: u64,
    pub spec: PostRowLayoutSpec,
    pub palette: PostRowPalette,
    pub locale: Locale,
    pub calendar: Calendar,
    pub time_zone: TimeZone,
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
    pub own_premium: Option<PostRowOwnPremium>,
    pub feed_audio_muted: bool,
    pub revision: u64,
    pub router: Rc<dyn PostRowActionRouting>,
    pub make_anchor_registration: PostRowMakeAnchorRegistration,
    pub focus: Rc<dyn PostListFocusReporting>,
    pub story_ring_state: PostRowStoryRingStateReader,
    pub shared_element_surface: Option<SharedElementSurface>,
    pub hosted: PostListHostedEnvironment,
}

impl PostRowMainContext {
    pub fn text_delta(&self) -> f32 {
        self.spec.text_delta
    }

    /// name color with the own profile override
    pub fn name_color(&self, author: &PostRowSeedAuthor) -> Hsla {
        if let Some(own_premium) = self.own_premium.as_ref()
            && author.id.as_deref() == Some(own_premium.user_id.as_str())
        {
            return if own_premium.is_premium {
                own_premium.name_color.unwrap_or(self.palette.text_primary)
            } else {
                self.palette.text_primary
            };
        }
        match author.premium_name_color_rgb {
            Some(value) => PostRowPalette::premium_name_color(value),
            None => self.palette.text_primary,
        }
    }
}

/// every row action goes through this, implementations
/// look the post up by id in the latest snapshot
pub trait PostRowActionRouting {
    /// latest snapshot of a post in this list
    fn post(&self, post_id: &str, cx: &mut App) -> Option<GetPostFeedResponse>;
    /// like button, selection haptic then toggle_like
    fn toggle_like(&self, post_id: &str, cx: &mut App);
    fn like_from_double_tap(&self, post_id: &str, cx: &mut App);
    fn open_likes(&self, post_id: &str, cx: &mut App);
    /// favorite button, selection haptic then toggle_favorite
    fn toggle_favorite(&self, post_id: &str, cx: &mut App);
    fn open_comments(&self, post_id: &str, cx: &mut App);
    /// toggles repost, no haptic
    fn toggle_repost(&self, post_id: &str, cx: &mut App);
    fn sparks_tap(&self, post_id: &str, cx: &mut App) -> bool;
    fn sparks_hold(&self, post_id: &str, cx: &mut App);
    fn avatar_tapped(&self, post_id: &str, cx: &mut App);
    fn header_text_tapped(&self, post_id: &str, cx: &mut App);
    fn open_profile(&self, user_id: &str, tag: &str, cx: &mut App);
    fn avatar_menu_items(&self, post_id: &str, cx: &mut App) -> Vec<PostRowMenuItem>;
    fn open_audio_page(&self, post_id: &str, cx: &mut App);
    /// opens hashtag search for a hashtag token
    fn open_hashtag(&self, tag: &str, post_id: &str, cx: &mut App);
    fn open_mention(&self, url: &Url, post_id: &str, window: &mut Window, cx: &mut App);
    fn open_link(&self, url: &Url, window: &mut Window, cx: &mut App);
    fn toggle_caption_expansion(&self, post_id: &str, cx: &mut App);
    fn open_tagged_people(&self, post_id: &str, cx: &mut App);
    fn open_media_viewer(
        &self,
        post_id: &str,
        slide_index: usize,
        source_view: Option<PostRowMediaPageSource>,
        cx: &mut App,
    );
    fn toggle_feed_sound(&self, cx: &mut App);
    /// ellipsis menu items, built on every open from the latest post
    fn menu_items(&self, post_id: &str, cx: &mut App) -> Vec<PostRowMenuItem>;
    fn open_riff(&self, post_id: &str, cx: &mut App);
    fn record_impression(&self, post_id: &str, cx: &mut App);
    /// selection haptic owned by the controller
    fn selection_haptic(&self, cx: &mut App);
    fn light_impact_haptic(&self, cx: &mut App);
}

/// called when a part is lifted or dropped back, gets the part and hidden state
pub type PostRowAnchorLiftChange = Rc<dyn Fn(SharedElementPart, bool, &mut App)>;
/// called when a slide is lifted or dropped back, gets slide index and hidden state
pub type PostRowAnchorSlideLiftChange = Rc<dyn Fn(i64, bool, &mut App)>;

/// registry records and lift observing for one row view
pub trait PostRowAnchorRegistering {
    /// surface keys are built in, none registers nothing
    fn surface(&self) -> Option<SharedElementSurface>;
    fn register_media(&self, view: &WindowScreenFrameReaderFrameView, post_id: &str, cx: &mut App);
    fn register_text(&self, view: &WindowScreenFrameReaderFrameView, post_id: &str, cx: &mut App);
    fn register_slide(&self, view: &WindowScreenFrameReaderFrameView, post_id: &str, index: i64, cx: &mut App);
    /// removes records registered by this frame view, each through its owner check
    fn unregister(&self, if_owned_by: &WindowScreenFrameReaderFrameView, cx: &mut App);
    /// removes every record with owner check and drops slots and observers, runs on reuse
    fn unregister_all(&self, cx: &mut App);
    fn is_lifted(&self, part: SharedElementPart, cx: &App) -> bool;
    /// called right after a registered part changes hidden state
    fn on_lift_change(&self) -> Option<PostRowAnchorLiftChange>;
    fn set_on_lift_change(&self, on_lift_change: Option<PostRowAnchorLiftChange>);
    /// called right away when the viewer lifts or lands a registered slide
    fn on_slide_lift_change(&self) -> Option<PostRowAnchorSlideLiftChange>;
    fn set_on_slide_lift_change(&self, on_slide_lift_change: Option<PostRowAnchorSlideLiftChange>);
}

/// what the shared element destination drives on the list owning the window
pub trait PostListWindowOwner {
    fn open_did_settle(&self, cx: &mut App);
    fn set_needs_reveal_evaluation(&self, cx: &mut App);
}

/// which surface of the row plays
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostListPlaybackTokenKind {
    VideoPage { slide_index: usize },
    AmbientMusic,
    RiffCard,
}

/// focus token of one playback surface in one list session
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PostListPlaybackToken {
    pub session: u64,
    pub presentation_id: String,
    pub kind: PostListPlaybackTokenKind,
}

impl PostListPlaybackToken {
    /// key for the shorts focus coordinator, unique per list session and surface
    pub fn raw_value(&self) -> String {
        match self.kind {
            PostListPlaybackTokenKind::VideoPage { slide_index } => {
                format!("post-list.{}.{}.video.{}", self.session, self.presentation_id, slide_index)
            }
            PostListPlaybackTokenKind::AmbientMusic => {
                format!("post-list.{}.{}.music", self.session, self.presentation_id)
            }
            PostListPlaybackTokenKind::RiffCard => {
                format!("post-list.{}.{}.riff", self.session, self.presentation_id)
            }
        }
    }
}

/// engine driven focus reporting, replaces the leaf geometry reporters
pub trait PostListFocusReporting {
    fn begin_focus_tracking(&self, token: &PostListPlaybackToken, frame_view: &Rc<PostListFocusFrameView>, cx: &mut App);
    /// stops reporting and removes the token from the focus coordinator
    fn end_focus_tracking(&self, token: &PostListPlaybackToken, cx: &mut App);
    /// true when the coordinator active token is this one
    fn is_focused(&self, token: &PostListPlaybackToken, cx: &App) -> bool;
}

/// what native and hosted row views must do to be mounted by the engine
pub trait PostListReusableRow: RiseManualListReusableView {
    /// presentation id of the last configure, none after reuse
    fn configured_presentation_id(&self) -> Option<&str>;
    fn configure(&mut self, prepared: &Arc<PreparedPostRow>, context: &PostRowMainContext, cx: &mut Context<Self>);
    fn set_hosted_leaves_active(&mut self, is_active: bool, cx: &mut Context<Self>);
}

/// handle for unregistering a cache purge
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PostListCachePurgeRegistration {
    id: i64,
}

#[derive(Default)]
struct PostListCachePurgeState {
    purges: BTreeMap<i64, Rc<dyn Fn(&mut App)>>,
    next_id: i64,
    memory_warning_observer: Option<RiseSubscription>,
}

thread_local! {
    static PURGE_STATE: RefCell<PostListCachePurgeState> = RefCell::new(PostListCachePurgeState::default());
}

/// purges every post list cache on account reset and memory warning
pub enum PostListCachePurge {}

impl PostListCachePurge {
    /// adds a cache purge, the closure must capture its cache weakly or statically
    pub fn register(purge: impl Fn(&mut App) + 'static, cx: &mut App) -> PostListCachePurgeRegistration {
        Self::install_memory_warning_observer_if_needed(cx);
        PURGE_STATE.with_borrow_mut(|state| {
            state.next_id = state.next_id.wrapping_add(1);
            state.purges.insert(state.next_id, Rc::new(purge));
            PostListCachePurgeRegistration { id: state.next_id }
        })
    }

    /// removes a purge from a session that is going away
    pub fn unregister(registration: PostListCachePurgeRegistration) {
        PURGE_STATE.with_borrow_mut(|state| {
            state.purges.remove(&registration.id);
        });
    }

    /// runs every purge in registration order then drops remembered seeds
    pub fn purge_all(cx: &mut App) {
        let ordered: Vec<Rc<dyn Fn(&mut App)>> =
            PURGE_STATE.with_borrow(|state| state.purges.values().cloned().collect());
        for purge in ordered {
            purge(cx);
        }
        PostRowContentVersionTable::shared().remove_all();
    }

    fn install_memory_warning_observer_if_needed(cx: &mut App) {
        if PURGE_STATE.with_borrow(|state| state.memory_warning_observer.is_some()) {
            return;
        }
        let (sender, mut inbox) = mpsc::unbounded::<()>();
        let observer = RiseAppEvents::default_center()
            .observe(UIApplication::DID_RECEIVE_MEMORY_WARNING_NOTIFICATION, move |_| {
                let _ = sender.unbounded_send(());
            });
        PURGE_STATE.with_borrow_mut(|state| state.memory_warning_observer = Some(observer));
        cx.spawn(async move |cx| {
            while inbox.next().await.is_some() {
                cx.update(Self::purge_all);
            }
        })
        .detach();
    }
}
