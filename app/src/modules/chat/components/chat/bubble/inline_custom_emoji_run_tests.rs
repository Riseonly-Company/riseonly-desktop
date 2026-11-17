use crate::modules::chat::components::chat::bubble::inline_custom_emoji_run::InlineCustomEmojiVideoPlaybackPolicy;

#[test]
fn accepts_av_foundation_and_web_m_http_and_file_sources() {
    assert_eq!(
        InlineCustomEmojiVideoPlaybackPolicy::playable_source("https://cdn.example/emoji.mp4?token=42").as_deref(),
        Some("https://cdn.example/emoji.mp4?token=42")
    );
    assert_eq!(
        InlineCustomEmojiVideoPlaybackPolicy::playable_source("file:///tmp/emoji.mov").as_deref(),
        Some("file:///tmp/emoji.mov")
    );
    assert_eq!(InlineCustomEmojiVideoPlaybackPolicy::playable_source("/tmp/emoji.m4v").as_deref(), Some("/tmp/emoji.m4v"));
    assert_eq!(
        InlineCustomEmojiVideoPlaybackPolicy::playable_source("https://cdn.example/emoji.webm").as_deref(),
        Some("https://cdn.example/emoji.webm")
    );
    assert_eq!(InlineCustomEmojiVideoPlaybackPolicy::playable_source("ftp://cdn.example/emoji.mp4"), None);
}
