use rise_core::foundation::foundation_url::Url;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::push_notifications::universal_link_parser::UniversalLinkParser;
use crate::core::widgets::sheets::share_sheet::SharePayload;

const TRACK_ID: &str = "11111111-2222-4333-8444-555555555555";
const PLAYLIST_ID: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
const JOIN_TOKEN: &str = "Abcd1234_token.value~";
const WEB_LOGIN_TOKEN: &str = "login_token-1";

fn url(raw: &str) -> Url {
    Url::from_string(raw).expect("test url parses")
}

fn playlist_invite() -> String {
    format!("{PLAYLIST_ID}.3.{}", "a".repeat(64))
}

fn public_routes() -> Vec<(RiseonlyPublicLink, String, PushDeepLink)> {
    let playlist_invite = playlist_invite();
    vec![
        (
            RiseonlyPublicLink::Profile { tag: "alice".to_owned() },
            "/u/alice".to_owned(),
            PushDeepLink::Profile { user_id: None, tag: Some("alice".to_owned()) },
        ),
        (
            RiseonlyPublicLink::Post { id: "post-1".to_owned() },
            "/p/post-1".to_owned(),
            PushDeepLink::Post { post_id: "post-1".to_owned() },
        ),
        (
            RiseonlyPublicLink::PostComment {
                post_id: "post-1".to_owned(),
                comment_id: "comment-2".to_owned(),
            },
            "/p/post-1/comment/comment-2".to_owned(),
            PushDeepLink::PostComment {
                post_id: "post-1".to_owned(),
                comment_id: Some("comment-2".to_owned()),
            },
        ),
        (
            RiseonlyPublicLink::Story { id: "story-1".to_owned() },
            "/s/story-1".to_owned(),
            PushDeepLink::Story { story_id: "story-1".to_owned() },
        ),
        (
            RiseonlyPublicLink::Chat { id: "chat-1".to_owned() },
            "/c/chat-1".to_owned(),
            PushDeepLink::Chat { chat_id: "chat-1".to_owned() },
        ),
        (
            RiseonlyPublicLink::Album { id: "album-1".to_owned() },
            "/a/album-1".to_owned(),
            PushDeepLink::MusicAlbum { album_id: "album-1".to_owned() },
        ),
        (
            RiseonlyPublicLink::StickerPack { slug: "summer-pack".to_owned() },
            "/addstickers/summer-pack".to_owned(),
            PushDeepLink::AddStickerPack { link: "summer-pack".to_owned() },
        ),
        (
            RiseonlyPublicLink::Track { id: TRACK_ID.to_owned(), playlist_id: None },
            format!("/t/{TRACK_ID}"),
            PushDeepLink::MusicTrack { track_id: TRACK_ID.to_owned() },
        ),
        (
            RiseonlyPublicLink::Track {
                id: TRACK_ID.to_owned(),
                playlist_id: Some(PLAYLIST_ID.to_owned()),
            },
            format!("/t/{TRACK_ID}?playlist={PLAYLIST_ID}"),
            PushDeepLink::PlaylistTrack {
                playlist_id: PLAYLIST_ID.to_owned(),
                track_id: TRACK_ID.to_owned(),
            },
        ),
        (
            RiseonlyPublicLink::Playlist { reference: PLAYLIST_ID.to_owned() },
            format!("/playlist/{PLAYLIST_ID}"),
            PushDeepLink::Playlist { playlist_id: PLAYLIST_ID.to_owned() },
        ),
        (
            RiseonlyPublicLink::Playlist { reference: playlist_invite.clone() },
            format!("/playlist/{playlist_invite}"),
            PushDeepLink::PlaylistInvite { token: playlist_invite.clone() },
        ),
        (
            RiseonlyPublicLink::Join { token: JOIN_TOKEN.to_owned() },
            format!("/j/{JOIN_TOKEN}"),
            PushDeepLink::JoinChat { token: JOIN_TOKEN.to_owned() },
        ),
        (
            RiseonlyPublicLink::WebLogin { token: WEB_LOGIN_TOKEN.to_owned() },
            format!("/w?t={WEB_LOGIN_TOKEN}"),
            PushDeepLink::WebLogin { login_token: WEB_LOGIN_TOKEN.to_owned() },
        ),
    ]
}

#[test]
fn every_public_route_uses_selected_environment_and_round_trips() {
    let environments = [
        (AppEnvironment::Dev, "https://dev.riseonly.net", "riseonly-dev"),
        (AppEnvironment::Staging, "https://staging.riseonly.net", "riseonly-staging"),
        (AppEnvironment::Prod, "https://riseonly.net", "riseonly"),
    ];

    for (environment, web_origin, app_scheme) in environments {
        for (link, suffix, deep_link) in public_routes() {
            let web_url = link.web_url(environment).expect("the route has a web url");
            let app_url = link.app_url(environment).expect("the route has an app url");

            assert_eq!(web_url.absolute_string(), format!("{web_origin}{suffix}"));
            assert_eq!(app_url.absolute_string(), format!("{app_scheme}://{}", &suffix[1..]));
            assert_eq!(UniversalLinkParser::parse(&web_url, environment).as_ref(), Some(&deep_link));
            assert_eq!(UniversalLinkParser::parse(&app_url, environment).as_ref(), Some(&deep_link));
        }
    }
}

#[test]
fn links_are_rejected_outside_their_environment() {
    let environments = [AppEnvironment::Dev, AppEnvironment::Staging, AppEnvironment::Prod];
    let link = RiseonlyPublicLink::Post { id: "post-1".to_owned() };

    for source in environments {
        let web_url = link.web_url(source).expect("the post has a web url");
        let app_url = link.app_url(source).expect("the post has an app url");

        for target in environments {
            if target.as_str() == source.as_str() {
                continue;
            }
            assert_eq!(UniversalLinkParser::parse(&web_url, target), None);
            assert_eq!(UniversalLinkParser::parse(&app_url, target), None);
        }
    }

    let production_www = url("https://www.riseonly.net/p/post-1");
    assert_eq!(
        UniversalLinkParser::parse(&production_www, AppEnvironment::Prod),
        Some(PushDeepLink::Post { post_id: "post-1".to_owned() })
    );
    assert_eq!(UniversalLinkParser::parse(&production_www, AppEnvironment::Staging), None);
}

#[test]
fn post_comment_requires_the_exact_route_shape() {
    let valid = url("https://staging.riseonly.net/p/post-1/comment/comment-2");
    assert_eq!(
        UniversalLinkParser::parse(&valid, AppEnvironment::Staging),
        Some(PushDeepLink::PostComment {
            post_id: "post-1".to_owned(),
            comment_id: Some("comment-2".to_owned()),
        })
    );

    let any_case = url("https://staging.riseonly.net/p/post-1/Comment/comment-2");
    assert_eq!(
        UniversalLinkParser::parse(&any_case, AppEnvironment::Staging),
        Some(PushDeepLink::PostComment {
            post_id: "post-1".to_owned(),
            comment_id: Some("comment-2".to_owned()),
        })
    );

    let malformed = [
        "https://staging.riseonly.net/p/post-1/comment",
        "https://staging.riseonly.net/p/post-1/comment/comment-2/extra",
        "https://staging.riseonly.net/p/post-1/comment/comment%2F2",
        "riseonly-staging://p/post%2F1/comment/comment-2",
    ];

    for raw in malformed {
        assert_eq!(UniversalLinkParser::parse(&url(raw), AppEnvironment::Staging), None, "{raw}");
    }
}

#[test]
fn comment_share_payload_uses_selected_environment() {
    let payload = SharePayload::Comment {
        post_id: "post-1".to_owned(),
        comment_id: "comment-2".to_owned(),
        text: "Reply".to_owned(),
    };
    let share_url = |environment| {
        payload.share_url_for(environment).map(|url| url.absolute_string().to_owned())
    };

    assert_eq!(
        share_url(AppEnvironment::Dev).as_deref(),
        Some("https://dev.riseonly.net/p/post-1/comment/comment-2")
    );
    assert_eq!(
        share_url(AppEnvironment::Staging).as_deref(),
        Some("https://staging.riseonly.net/p/post-1/comment/comment-2")
    );
    assert_eq!(
        share_url(AppEnvironment::Prod).as_deref(),
        Some("https://riseonly.net/p/post-1/comment/comment-2")
    );
}

#[test]
fn generic_routes_reject_extra_segments_and_encoded_slashes() {
    let malformed = [
        "https://riseonly.net/u/alice/extra",
        "https://riseonly.net/u/alice%2Fadmin",
        "https://riseonly.net/p/post-1/extra",
        "https://riseonly.net/p/post%2F1",
        "https://riseonly.net/s/story-1/extra",
        "https://riseonly.net/s/story%2F1",
        "https://riseonly.net/chat/chat-1/extra",
        "https://riseonly.net/chat/chat%2F1",
        "https://riseonly.net/c/chat-1/extra",
        "https://riseonly.net/c/chat%2F1",
        "https://riseonly.net/album/album-1/extra",
        "https://riseonly.net/album/album%2F1",
        "https://riseonly.net/addstickers/summer-pack/extra",
        "https://riseonly.net/addstickers/summer%2Fpack",
    ];

    for raw in malformed {
        assert_eq!(UniversalLinkParser::parse(&url(raw), AppEnvironment::Prod), None, "{raw}");
    }
}

#[test]
fn dedicated_routes_keep_strict_path_query_and_fragment_rules() {
    let playlist_invite = playlist_invite();
    let valid = [
        (
            format!("https://riseonly.net/track/{TRACK_ID}?playlist_id={PLAYLIST_ID}"),
            PushDeepLink::PlaylistTrack {
                playlist_id: PLAYLIST_ID.to_owned(),
                track_id: TRACK_ID.to_owned(),
            },
        ),
        (
            format!("https://riseonly.net/track/{TRACK_ID}#autoplay"),
            PushDeepLink::MusicTrack { track_id: TRACK_ID.to_owned() },
        ),
        (
            format!("https://riseonly.net/playlist/{PLAYLIST_ID}#tracks"),
            PushDeepLink::Playlist { playlist_id: PLAYLIST_ID.to_owned() },
        ),
        (
            format!("https://riseonly.net/playlist/{playlist_invite}"),
            PushDeepLink::PlaylistInvite { token: playlist_invite.clone() },
        ),
        (
            format!("riseonly://join?token={JOIN_TOKEN}"),
            PushDeepLink::JoinChat { token: JOIN_TOKEN.to_owned() },
        ),
        (
            format!("riseonly://weblogin?token={WEB_LOGIN_TOKEN}"),
            PushDeepLink::WebLogin { login_token: WEB_LOGIN_TOKEN.to_owned() },
        ),
    ];

    for (raw, deep_link) in valid {
        assert_eq!(
            UniversalLinkParser::parse(&url(&raw), AppEnvironment::Prod),
            Some(deep_link),
            "{raw}"
        );
    }

    let malformed = [
        format!("https://riseonly.net/track/{TRACK_ID}/extra"),
        format!("https://riseonly.net/track/{TRACK_ID}?foo=bar"),
        format!("https://riseonly.net/playlist/{PLAYLIST_ID}?foo=bar"),
        format!("https://www.riseonly.net/join/{JOIN_TOKEN}"),
        format!("riseonly://join?token={JOIN_TOKEN}&extra=1"),
        "https://riseonly.net/weblogin".to_owned(),
        format!("https://riseonly.net/weblogin/extra?t={WEB_LOGIN_TOKEN}"),
        format!("https://riseonly.net/weblogin?t={WEB_LOGIN_TOKEN}&extra=1"),
        format!("https://riseonly.net/weblogin?t={WEB_LOGIN_TOKEN}#complete"),
    ];

    for raw in malformed {
        assert_eq!(UniversalLinkParser::parse(&url(&raw), AppEnvironment::Prod), None, "{raw}");
    }
}

#[test]
fn join_url_normalizes_tokens_and_existing_links_for_selected_environment() {
    let expected = format!("https://staging.riseonly.net/j/{JOIN_TOKEN}");
    let encoded = "https://riseonly.net/join/Abcd1234_token.value%7E";
    let join_url = |raw: &str, environment| {
        RiseonlyPublicLink::join_url(raw, environment).map(|url| url.absolute_string().to_owned())
    };

    assert_eq!(join_url(JOIN_TOKEN, AppEnvironment::Staging).as_deref(), Some(expected.as_str()));
    assert_eq!(join_url(encoded, AppEnvironment::Staging).as_deref(), Some(expected.as_str()));
    assert_eq!(
        join_url(&format!("https://dev.riseonly.net/join/{JOIN_TOKEN}"), AppEnvironment::Prod),
        Some(format!("https://riseonly.net/j/{JOIN_TOKEN}"))
    );
    assert_eq!(join_url("https://riseonly.net/join/short", AppEnvironment::Staging), None);
    assert_eq!(
        join_url(&format!("https://riseonly.net/join/{JOIN_TOKEN}/extra"), AppEnvironment::Staging),
        None
    );
}
