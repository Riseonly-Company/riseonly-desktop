use std::sync::atomic::{AtomicU64, Ordering};

use gpui::{App, Bounds, Pixels, TestAppContext, px, size};
use rise_theme::ThemeStore;

use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostRowFallbackReason, PostRowLayout, PostRowLayoutInputs, PostRowLayoutSpec, PostRowMediaAspects, PostRowNativeKind,
    PostRowPalette, PostRowRoute,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_layout_calculator::{
    PostRowLayoutCalculator, PostRowMetrics,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_media_aspect_resolver::{
    PostRowAspectResolution, PostRowMediaAspectResolver,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_seed_extractor::PostRowSeedExtractor;
use crate::modules::post::shared::post_media_box::PostMediaBox;
use crate::modules::post::stores::post_actions::post_types::{
    AuthorInfo, GetPostFeedResponse, PostLikedByUser, PostMediaSize, PostModerationBanner,
};

const WIDTH: f32 = 393.0;

static UNIQUE: AtomicU64 = AtomicU64::new(0);

fn unique() -> u64 {
    UNIQUE.fetch_add(1, Ordering::Relaxed)
}

fn spec_with(text_delta: f32) -> PostRowLayoutSpec {
    PostRowLayoutSpec::new(
        WIDTH,
        size(px(WIDTH), px(852.0)),
        3.0,
        text_delta,
        "en".to_owned(),
        "test".to_owned(),
        false,
    )
}

fn spec() -> PostRowLayoutSpec {
    spec_with(0.0)
}

fn post(configure: impl FnOnce(&mut GetPostFeedResponse)) -> GetPostFeedResponse {
    let mut post = GetPostFeedResponse {
        id: "layout-post".to_owned(),
        author: AuthorInfo {
            id: Some("author-1".to_owned()),
            name: "Author".to_owned(),
            tag: "author".to_owned(),
            ..AuthorInfo::default()
        },
        author_id: "author-1".to_owned(),
        created_at: "2026-09-01T10:00:00Z".to_owned(),
        ..GetPostFeedResponse::default()
    };
    configure(&mut post);
    post
}

fn plain_post() -> GetPostFeedResponse {
    post(|_| {})
}

fn media_size(height: i64, width: i64) -> PostMediaSize {
    PostMediaSize {
        height,
        width,
        ..PostMediaSize::default()
    }
}

fn liker() -> Vec<PostLikedByUser> {
    vec![PostLikedByUser {
        id: "liker".to_owned(),
        name: "Liker".to_owned(),
        tag: "liker".to_owned(),
        ..PostLikedByUser::default()
    }]
}

fn palette(cx: &mut App) -> PostRowPalette {
    let store = ThemeStore::shared(cx);
    PostRowPalette::new(store.read(cx).current_palette())
}

fn resolved_layout(post: &GetPostFeedResponse, spec: PostRowLayoutSpec, cx: &mut App) -> (PostRowLayout, PostRowAspectResolution) {
    let seed = PostRowSeedExtractor::make_seed(post, cx);
    let route = PostRowSeedExtractor::route(&seed);
    let resolution = PostRowMediaAspectResolver::resolve(&seed, Some("viewer"), Some("epoch"));
    let inputs = PostRowLayoutInputs::new(&seed, route, spec, &resolution.aspects);
    let palette = palette(cx);
    (PostRowLayoutCalculator::layout(&inputs, &palette), resolution)
}

fn bind(cx: &mut TestAppContext) {
    cx.update(|cx| EditorImageBake::install(cx));
}

fn min_x(frame: &Bounds<Pixels>) -> f32 {
    f32::from(frame.origin.x)
}

fn min_y(frame: &Bounds<Pixels>) -> f32 {
    f32::from(frame.origin.y)
}

fn frame_width(frame: &Bounds<Pixels>) -> f32 {
    f32::from(frame.size.width)
}

fn frame_height(frame: &Bounds<Pixels>) -> f32 {
    f32::from(frame.size.height)
}

fn max_x(frame: &Bounds<Pixels>) -> f32 {
    min_x(frame) + frame_width(frame)
}

fn max_y(frame: &Bounds<Pixels>) -> f32 {
    min_y(frame) + frame_height(frame)
}

#[gpui::test]
fn height_changes_with_text_delta(cx: &mut TestAppContext) {
    bind(cx);
    cx.update(|cx| {
        let post = plain_post();
        let regular = resolved_layout(&post, spec_with(0.0), cx).0;
        let larger = resolved_layout(&post, spec_with(6.0), cx).0;
        assert!(frame_height(&larger.header.name) > frame_height(&regular.header.name));
        assert!(min_y(&larger.actions.frame) > min_y(&regular.actions.frame));
        assert!(larger.total_height > regular.total_height);
    });
}

#[gpui::test]
fn height_ignores_likes_count(cx: &mut TestAppContext) {
    bind(cx);
    cx.update(|cx| {
        let few = resolved_layout(&post(|post| post.likes_count = 1), spec(), cx).0;
        let many = resolved_layout(
            &post(|post| {
                post.likes_count = 12_500;
                post.is_liked = true;
                post.favorites_count = 40;
            }),
            spec(),
            cx,
        )
        .0;
        assert_eq!(few.total_height, many.total_height);
    });
}

#[gpui::test]
fn likes_summary_adds_its_block_only_with_likes(cx: &mut TestAppContext) {
    bind(cx);
    cx.update(|cx| {
        let hidden = resolved_layout(
            &post(|post| {
                post.liked_by = liker();
                post.likes_count = 0;
            }),
            spec(),
            cx,
        )
        .0;
        let shown = resolved_layout(
            &post(|post| {
                post.liked_by = liker();
                post.likes_count = 3;
            }),
            spec(),
            cx,
        )
        .0;

        assert!(hidden.likes_summary.is_none());
        let summary = shown.likes_summary.as_ref().expect("summary with likes");
        assert!(frame_height(&summary.frame) >= PostRowMetrics::LIKES_AVATAR_SIZE);
        assert_eq!(summary.avatars.len(), 1);
        assert_eq!(min_x(&summary.frame), PostRowMetrics::LIKES_HORIZONTAL_PAD);
        let expected = hidden.total_height - PostRowMetrics::ACTIONS_BOTTOM
            + PostRowMetrics::ACTIONS_BOTTOM_BEFORE_SUMMARY
            + frame_height(&summary.frame)
            + PostRowMetrics::LIKES_BOTTOM;
        assert!((shown.total_height - expected).abs() < 0.001);
    });
}

#[gpui::test]
fn plain_row_stacks_header_actions_and_separator(cx: &mut TestAppContext) {
    bind(cx);
    cx.update(|cx| {
        let layout = resolved_layout(&plain_post(), spec(), cx).0;
        assert_eq!(min_y(&layout.header.frame), PostRowMetrics::HEADER_TOP);
        assert_eq!(min_x(&layout.header.frame), PostRowMetrics::HORIZONTAL_PAD);
        assert!(frame_height(&layout.header.frame) >= PostRowMetrics::AVATAR_SIZE);
        assert_eq!(max_x(&layout.header.more_button), frame_width(&layout.header.frame));
        assert_eq!(
            min_y(&layout.actions.frame),
            max_y(&layout.header.frame) + PostRowMetrics::HEADER_BOTTOM + PostRowMetrics::ACTIONS_TOP
        );
        assert_eq!(frame_height(&layout.actions.frame), PostRowMetrics::ACTIONS_MINIMUM_HEIGHT);
        assert_eq!(layout.content_height, max_y(&layout.actions.frame) + PostRowMetrics::ACTIONS_BOTTOM);
        assert_eq!(min_y(&layout.separator), layout.content_height + PostRowLayout::SEPARATOR_GAP);
        assert_eq!(frame_height(&layout.separator), PostRowLayout::SEPARATOR_THICKNESS);
        assert_eq!(
            layout.total_height,
            layout.content_height + PostRowLayout::SEPARATOR_GAP + PostRowLayout::SEPARATOR_THICKNESS
        );
        assert!(layout.media_box.is_none());
        assert!(layout.caption.is_none());
    });
}

#[gpui::test]
fn action_cells_split_the_row_equally(cx: &mut TestAppContext) {
    bind(cx);
    cx.update(|cx| {
        let actions = resolved_layout(&plain_post(), spec(), cx).0.actions;
        assert!((min_x(&actions.favorite) - (frame_width(&actions.like) + PostRowMetrics::ACTIONS_SPACING)).abs() < 0.001);
        assert!((max_x(&actions.repost) - frame_width(&actions.frame)).abs() < 0.001);
    });
}

#[gpui::test]
fn moderation_banner_sits_between_header_and_actions(cx: &mut TestAppContext) {
    bind(cx);
    cx.update(|cx| {
        let layout = resolved_layout(&post(|post| post.moderation_status = "disabled".to_owned()), spec(), cx).0;
        let banner = layout.moderation_banner.as_ref().expect("banner");
        assert_eq!(min_y(&banner.frame), max_y(&layout.header.frame) + PostRowMetrics::HEADER_BOTTOM);
        assert_eq!(
            min_y(&layout.actions.frame),
            max_y(&banner.frame) + PostRowMetrics::BANNER_BOTTOM + PostRowMetrics::ACTIONS_TOP
        );
    });
}

#[gpui::test]
fn review_statuses_take_the_banner_slot(cx: &mut TestAppContext) {
    bind(cx);
    cx.update(|cx| {
        let pending = resolved_layout(&post(|post| post.moderation_status = "pending_review".to_owned()), spec(), cx).0;
        let rejected = resolved_layout(&post(|post| post.moderation_status = "rejected".to_owned()), spec(), cx).0;
        let active = resolved_layout(&plain_post(), spec(), cx).0;
        assert!(pending.moderation_banner.as_ref().expect("pending").banner == PostModerationBanner::UnderReview);
        assert!(rejected.moderation_banner.as_ref().expect("rejected").banner == PostModerationBanner::Rejected);
        assert!(active.moderation_banner.is_none());
    });
}

#[gpui::test]
fn music_strip_height_is_set_by_the_sound_button(cx: &mut TestAppContext) {
    bind(cx);
    cx.update(|cx| {
        let layout = resolved_layout(
            &post(|post| {
                post.track_id = Some("track-1".to_owned());
                post.track = Some(RiseStoredMusicTrack {
                    id: Some("track-1".to_owned()),
                    title: Some("Song".to_owned()),
                    duration_ms: Some(180_000),
                    ..RiseStoredMusicTrack::default()
                });
            }),
            spec(),
            cx,
        )
        .0;
        let strip = layout.music_strip.as_ref().expect("strip");
        let column_height = frame_height(&strip.title_line) + PostRowMetrics::STRIP_LINE_SPACING + frame_height(&strip.duration_line);
        let expected = PostRowMetrics::STRIP_VERTICAL_INSET * 2.0
            + PostRowMetrics::STRIP_COVER_SIZE
                .max(column_height)
                .max(PostRowMetrics::SOUND_BUTTON_TAP_SIZE);
        assert_eq!(frame_height(&strip.frame), expected);
        assert!(layout.header.music_line.is_some());
        assert_eq!(
            min_y(&layout.actions.frame),
            max_y(&strip.frame) + PostRowMetrics::STRIP_BOTTOM + PostRowMetrics::ACTIONS_TOP
        );
    });
}

#[gpui::test]
fn unresolved_aspect_uses_the_placeholder(cx: &mut TestAppContext) {
    bind(cx);
    cx.update(|cx| {
        let post = post(|post| {
            post.video = Some("https://media.invalid/unresolved.mp4".to_owned());
            post.video_thumbnail = Some(format!("https://media.invalid/unresolved-poster-{}.jpg", unique()));
            post.images = vec!["https://media.invalid/second.jpg".to_owned()];
            post.image_sizes = vec![media_size(1000, 1000)];
            post.video_position = Some(0);
        });
        let (layout, resolution) = resolved_layout(&post, spec(), cx);
        assert!(resolution.probe.is_some());
        assert!(!resolution.is_settled);
        assert!(resolution.aspects.is_provisional);
        assert_eq!(resolution.aspects.aspect(0), None);
        assert_eq!(resolution.aspects.aspect(1), Some(1.0));

        let media_box = layout.media_box.as_ref().expect("box");
        let placeholder = PostRowLayoutCalculator::box_aspect(None, &spec());
        assert!((placeholder - PostMediaBox::PLACEHOLDER_ASPECT).abs() < 0.0001);
        assert_eq!(media_box.box_aspect, placeholder);
        assert!(media_box.is_aspect_provisional);
        assert!((frame_height(&media_box.frame) - WIDTH / placeholder).abs() <= 1.0 / 3.0);
        assert!(media_box.page_indicator.is_some());
        assert!(layout.inputs.is_aspect_provisional);
    });
}

#[gpui::test]
fn server_aspect_wins_for_photos(cx: &mut TestAppContext) {
    bind(cx);
    cx.update(|cx| {
        let post = post(|post| {
            post.images = vec![format!("https://media.invalid/wide-{}.jpg", unique())];
            post.image_sizes = vec![media_size(800, 1200)];
        });
        let (layout, resolution) = resolved_layout(&post, spec(), cx);
        assert!(resolution.is_settled);
        assert!(resolution.probe.is_none());
        assert_eq!(resolution.aspects.aspect(0), Some(1.5));

        let media_box = layout.media_box.as_ref().expect("box");
        assert_eq!(media_box.box_aspect, 1.5);
        assert_eq!(media_box.slides.first().map(|slide| slide.fills), Some(true));
        assert_eq!(min_y(&media_box.frame), max_y(&layout.header.frame) + PostRowMetrics::HEADER_BOTTOM);
        assert!((frame_height(&media_box.frame) - WIDTH / 1.5).abs() <= 1.0 / 3.0);
        assert!(media_box.page_indicator.is_none());
        assert_eq!(min_y(&layout.actions.frame), max_y(&media_box.frame) + PostRowMetrics::ACTIONS_TOP);
    });
}

#[gpui::test]
fn tall_photo_clamps_to_the_ceiling_and_fits(cx: &mut TestAppContext) {
    bind(cx);
    cx.update(|cx| {
        let spec = spec();
        let post = post(|post| {
            post.images = vec![format!("https://media.invalid/tall-{}.jpg", unique())];
            post.image_sizes = vec![media_size(3000, 1000)];
        });
        let layout = resolved_layout(&post, spec.clone(), cx).0;
        let media_box = layout.media_box.as_ref().expect("box");
        let ceiling = spec.screen_width / (spec.screen_height * PostMediaBox::MAXIMUM_HEIGHT_FRACTION);
        assert!((media_box.box_aspect - ceiling).abs() < 0.0001);
        assert_eq!(media_box.slides.first().map(|slide| slide.fills), Some(false));
    });
}

#[gpui::test]
fn riff_card_uses_reel_paddings(cx: &mut TestAppContext) {
    bind(cx);
    cx.update(|cx| {
        let post = post(|post| {
            post.is_riff = true;
            post.video = Some("https://media.invalid/riff.mp4".to_owned());
            post.video_size = Some(media_size(1280, 720));
        });
        let layout = resolved_layout(&post, spec(), cx).0;
        let media_box = layout.media_box.as_ref().expect("box");
        assert!(media_box.sound_button.is_none());
        assert!(media_box.music_plate.is_none());
        assert_eq!(
            media_box.box_aspect,
            PostRowLayoutCalculator::box_aspect(Some(720.0 / 1280.0), &spec())
        );
        assert_eq!(min_y(&layout.actions.frame), max_y(&media_box.frame) + PostRowMetrics::REEL_ACTIONS_TOP);
    });
}

#[gpui::test]
fn hosted_estimate_adds_the_achievement_plate(cx: &mut TestAppContext) {
    bind(cx);
    cx.update(|cx| {
        let spec = spec();
        let palette = palette(cx);
        let seed = PostRowSeedExtractor::make_seed(&post(|post| post.achievement_id = Some(3)), cx);
        let plain = PostRowSeedExtractor::make_seed(&plain_post(), cx);
        let estimate = PostRowLayoutCalculator::estimated_hosted_height(
            &seed,
            PostRowFallbackReason::Achievement,
            &PostRowMediaAspects::empty(),
            &spec,
            &palette,
        );
        let plain_height = PostRowLayoutCalculator::layout(
            &PostRowLayoutInputs::new(
                &plain,
                PostRowRoute::Native(PostRowNativeKind::Plain),
                spec.clone(),
                &PostRowMediaAspects::empty(),
            ),
            &palette,
        )
        .total_height;
        assert!((estimate - (plain_height + PostRowMetrics::ACHIEVEMENT_HEIGHT)).abs() < 0.001);
    });
}
