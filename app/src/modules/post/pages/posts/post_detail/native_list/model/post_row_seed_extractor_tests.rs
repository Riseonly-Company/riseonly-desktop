use gpui::{TestAppContext, px, size};

use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostListReuseIdentifier, PostRowBodyKind, PostRowContentVersionTable, PostRowFallbackReason, PostRowLayoutInputs,
    PostRowLayoutSpec, PostRowMediaAspects, PostRowNativeKind, PostRowRoute, PostRowSeed,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_seed_extractor::PostRowSeedExtractor;
use crate::modules::post::stores::post_actions::post_types::{
    AuthorInfo, GetPostFeedResponse, PostCollaboratorUser, PostLikedByUser,
};

fn spec() -> PostRowLayoutSpec {
    PostRowLayoutSpec::new(393.0, size(px(393.0), px(852.0)), 3.0, 0.0, "en".to_owned(), "test".to_owned(), false)
}

fn post(configure: impl FnOnce(&mut GetPostFeedResponse)) -> GetPostFeedResponse {
    let mut post = GetPostFeedResponse { id: "seed-post".to_owned(), ..GetPostFeedResponse::default() };
    post.author = AuthorInfo {
        id: Some("author-1".to_owned()),
        name: "Author".to_owned(),
        tag: "author".to_owned(),
        ..AuthorInfo::default()
    };
    post.author_id = "author-1".to_owned();
    post.created_at = "2026-09-01T10:00:00Z".to_owned();
    configure(&mut post);
    post
}

fn track() -> RiseStoredMusicTrack {
    RiseStoredMusicTrack {
        id: Some("track-1".to_owned()),
        title: Some("Song".to_owned()),
        duration_ms: Some(180_000),
        ..RiseStoredMusicTrack::default()
    }
}

fn liker() -> Vec<PostLikedByUser> {
    vec![PostLikedByUser { id: "liker".to_owned(), ..PostLikedByUser::default() }]
}

fn seed_of(post: &GetPostFeedResponse, cx: &mut TestAppContext) -> PostRowSeed {
    cx.update(|cx| PostRowSeedExtractor::make_seed(post, cx))
}

fn route_of(post: &GetPostFeedResponse, cx: &mut TestAppContext) -> PostRowRoute {
    PostRowSeedExtractor::route(&seed_of(post, cx))
}

#[gpui::test]
fn text_post_is_native_plain(cx: &mut TestAppContext) {
    let post = post(|post| post.title = "Hello".to_owned());
    assert_eq!(route_of(&post, cx), PostRowRoute::Native(PostRowNativeKind::Plain));
}

#[gpui::test]
fn code_block_routes_to_hosted_fallback(cx: &mut TestAppContext) {
    let post = post(|post| post.content = "Intro\n```swift\nlet a = 1\n```".to_owned());
    assert_eq!(route_of(&post, cx), PostRowRoute::HostedFallback(PostRowFallbackReason::CodeBlock));
}

#[gpui::test]
fn riff_routes_to_native_card_geometry(cx: &mut TestAppContext) {
    let post = post(|post| {
        post.is_riff = true;
        post.video = Some("https://media.invalid/riff.mp4".to_owned());
        post.video_thumbnail = Some("https://media.invalid/riff.jpg".to_owned());
    });
    let seed = seed_of(&post, cx);
    assert_eq!(seed.body, PostRowBodyKind::Reel);
    assert_eq!(PostRowSeedExtractor::route(&seed), PostRowRoute::Native(PostRowNativeKind::RiffCard));
    assert_eq!(PostRowSeedExtractor::route(&seed).reuse_identifier(), PostListReuseIdentifier::NATIVE_RIFF);
}

#[gpui::test]
fn coauthors_route_to_collaborator_header(cx: &mut TestAppContext) {
    let post = post(|post| {
        post.coauthors = vec![PostCollaboratorUser {
            id: "coauthor-1".to_owned(),
            name: "Co".to_owned(),
            ..PostCollaboratorUser::default()
        }];
    });
    assert_eq!(route_of(&post, cx), PostRowRoute::HostedFallback(PostRowFallbackReason::CollaboratorHeader));
}

#[gpui::test]
fn achievement_and_upload_route_to_fallback(cx: &mut TestAppContext) {
    let achievement = post(|post| post.achievement_id = Some(7));
    assert_eq!(route_of(&achievement, cx), PostRowRoute::HostedFallback(PostRowFallbackReason::Achievement));
    let upload = post(|post| {
        post.is_temp = true;
        post.images = vec!["https://media.invalid/a.jpg".to_owned()];
    });
    assert_eq!(route_of(&upload, cx), PostRowRoute::HostedFallback(PostRowFallbackReason::Upload));
    let local_slide = post(|post| post.images = vec!["file:///tmp/local.jpg".to_owned()]);
    assert_eq!(route_of(&local_slide, cx), PostRowRoute::HostedFallback(PostRowFallbackReason::Upload));
}

#[gpui::test]
fn reel_with_hidden_like_count_routes_to_fallback(cx: &mut TestAppContext) {
    let post = post(|post| {
        post.is_riff = true;
        post.video = Some("https://media.invalid/riff.mp4".to_owned());
        post.liked_by = liker();
        post.likes_count = 0;
    });
    assert_eq!(route_of(&post, cx), PostRowRoute::HostedFallback(PostRowFallbackReason::Unknown));
}

#[gpui::test]
fn media_kinds_follow_the_slides(cx: &mut TestAppContext) {
    let photos = post(|post| {
        post.images = vec!["https://media.invalid/a.jpg".to_owned(), "https://media.invalid/b.jpg".to_owned()];
    });
    assert_eq!(route_of(&photos, cx), PostRowRoute::Native(PostRowNativeKind::PhotoGallery));
    let mixed = post(|post| {
        post.images = vec!["https://media.invalid/a.jpg".to_owned()];
        post.video = Some("https://media.invalid/v.mp4".to_owned());
        post.video_position = Some(1);
    });
    assert_eq!(route_of(&mixed, cx), PostRowRoute::Native(PostRowNativeKind::VideoGallery));
    let music = post(|post| {
        post.track_id = Some("track-1".to_owned());
        post.track = Some(track());
    });
    assert_eq!(route_of(&music, cx), PostRowRoute::Native(PostRowNativeKind::MusicStrip));
}

#[gpui::test]
fn remote_slides_keep_author_order(cx: &mut TestAppContext) {
    let post = post(|post| {
        post.images = vec![" https://media.invalid/a.jpg ".to_owned(), "https://media.invalid/b.jpg".to_owned()];
        post.video = Some("https://media.invalid/v.mp4".to_owned());
        post.video_position = Some(1);
    });
    let seed = seed_of(&post, cx);
    assert_eq!(seed.media.slides.len(), 3);
    assert_eq!(seed.media.slides.iter().map(|slide| slide.is_video()).collect::<Vec<_>>(), vec![false, true, false]);
    assert_eq!(seed.media.slides.first().and_then(|slide| slide.poster_url()), Some("https://media.invalid/a.jpg"));
    assert_eq!(seed.media.media_slide_count, 3);
}

#[gpui::test]
fn name_colour_only_for_premium_authors(cx: &mut TestAppContext) {
    let plain = post(|post| post.author.more.name_color = Some("#FF0000".to_owned()));
    assert_eq!(seed_of(&plain, cx).author.premium_name_color_rgb, None);
    let premium = post(|post| {
        post.author.more.is_premium = true;
        post.author.more.name_color = Some("#FF0000".to_owned());
    });
    assert_eq!(seed_of(&premium, cx).author.premium_name_color_rgb, Some(0xFF0000));
}

#[gpui::test]
fn likes_summary_rule_depends_on_body(cx: &mut TestAppContext) {
    let standard_zero = seed_of(
        &post(|post| {
            post.liked_by = liker();
            post.likes_count = 0;
        }),
        cx,
    );
    assert!(!standard_zero.shows_likes_summary());
    let standard_liked = seed_of(
        &post(|post| {
            post.liked_by = liker();
            post.likes_count = 4;
        }),
        cx,
    );
    assert!(standard_liked.shows_likes_summary());
    let reel = seed_of(
        &post(|post| {
            post.is_riff = true;
            post.video = Some("https://media.invalid/riff.mp4".to_owned());
            post.liked_by = liker();
            post.likes_count = 2;
        }),
        cx,
    );
    assert!(reel.shows_likes_summary());
}

#[gpui::test]
fn counts_change_content_version_but_not_layout_inputs(cx: &mut TestAppContext) {
    let table = PostRowContentVersionTable::default();
    let first = seed_of(&post(|post| post.likes_count = 3), cx);
    let same = seed_of(&post(|post| post.likes_count = 3), cx);
    let liked = seed_of(
        &post(|post| {
            post.likes_count = 4;
            post.is_liked = true;
        }),
        cx,
    );

    let first_version = table.version(&first);
    assert_eq!(table.version(&same), first_version);
    let liked_version = table.version(&liked);
    assert_ne!(liked_version, first_version);

    let route = PostRowSeedExtractor::route(&first);
    let first_inputs = PostRowLayoutInputs::new(&first, route, spec(), &PostRowMediaAspects::empty());
    let liked_inputs = PostRowLayoutInputs::new(&liked, route, spec(), &PostRowMediaAspects::empty());
    assert_eq!(first_inputs, liked_inputs);
}

#[gpui::test]
fn avatar_url_changes_version_not_inputs(cx: &mut TestAppContext) {
    let table = PostRowContentVersionTable::default();
    let before = seed_of(&post(|post| post.author.more.logo = "https://media.invalid/a.jpg".to_owned()), cx);
    let after = seed_of(&post(|post| post.author.more.logo = "https://media.invalid/b.jpg".to_owned()), cx);
    assert_ne!(table.version(&before), table.version(&after));
    let route = PostRowSeedExtractor::route(&before);
    assert_eq!(
        PostRowLayoutInputs::new(&before, route, spec(), &PostRowMediaAspects::empty()),
        PostRowLayoutInputs::new(&after, route, spec(), &PostRowMediaAspects::empty())
    );
}
