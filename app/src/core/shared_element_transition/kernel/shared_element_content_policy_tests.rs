use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementContentPolicy, SharedElementContentPolicyVideoSource,
};

struct Choice {
    is_playing: bool,
    ended: bool,
    stop_frame_ready: bool,
    has_poster: bool,
    has_grid_cover: bool,
    can_replicate: bool,
}

impl Default for Choice {
    fn default() -> Self {
        Self {
            is_playing: true,
            ended: false,
            stop_frame_ready: true,
            has_poster: true,
            has_grid_cover: true,
            can_replicate: true,
        }
    }
}

fn choice(input: Choice) -> SharedElementContentPolicyVideoSource {
    SharedElementContentPolicy::video_choice(
        input.is_playing,
        input.ended,
        input.stop_frame_ready,
        input.has_poster,
        input.has_grid_cover,
        input.can_replicate,
    )
}

#[test]
fn playing_reel_with_a_ready_frame_flies_the_stop_frame() {
    assert_eq!(choice(Choice::default()), SharedElementContentPolicyVideoSource::StopFrame);
}

#[test]
fn paused_or_ended_reel_flies_the_poster() {
    assert_eq!(choice(Choice { is_playing: false, ..Choice::default() }), SharedElementContentPolicyVideoSource::Poster);
    assert_eq!(choice(Choice { ended: true, ..Choice::default() }), SharedElementContentPolicyVideoSource::Poster);
}

#[test]
fn missed_stop_frame_falls_back_to_the_poster() {
    assert_eq!(
        choice(Choice { stop_frame_ready: false, ..Choice::default() }),
        SharedElementContentPolicyVideoSource::Poster
    );
}

#[test]
fn fallback_chain_ends_in_a_fade() {
    assert_eq!(
        choice(Choice { stop_frame_ready: false, has_poster: false, ..Choice::default() }),
        SharedElementContentPolicyVideoSource::GridCover
    );
    assert_eq!(
        choice(Choice { stop_frame_ready: false, has_poster: false, has_grid_cover: false, ..Choice::default() }),
        SharedElementContentPolicyVideoSource::Replicant
    );
    assert_eq!(
        choice(Choice {
            stop_frame_ready: false,
            has_poster: false,
            has_grid_cover: false,
            can_replicate: false,
            ..Choice::default()
        }),
        SharedElementContentPolicyVideoSource::FadeOnly
    );
}

#[test]
fn cover_is_skipped_when_the_flying_content_is_the_cover() {
    let url = "https://media.riseonly.net/p/1/thumb.jpg";
    assert!(!SharedElementContentPolicy::needs_cover(Some(url), Some(url)));
    assert!(SharedElementContentPolicy::needs_cover(Some("stopframe"), Some(url)));
    assert!(SharedElementContentPolicy::needs_cover(Some("replicant"), Some("text:1")));
}

#[test]
fn missing_identities() {
    assert!(!SharedElementContentPolicy::needs_cover(Some("stopframe"), None));
    assert!(!SharedElementContentPolicy::needs_cover(None, None));
    assert!(SharedElementContentPolicy::needs_cover(None, Some("text:1")));
}
