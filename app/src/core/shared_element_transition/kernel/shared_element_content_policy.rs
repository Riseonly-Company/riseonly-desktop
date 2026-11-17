use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementContentPolicy, SharedElementContentPolicyVideoSource,
};

impl SharedElementContentPolicy {
    /// content for a reel flight, a stop frame flies only for a playing reel whose frame is ready
    /// paused or ended reels fly the poster, misses go poster then grid cover then replicant then fade
    pub fn video_choice(
        is_playing: bool,
        ended: bool,
        stop_frame_ready: bool,
        has_poster: bool,
        has_grid_cover: bool,
        can_replicate: bool,
    ) -> SharedElementContentPolicyVideoSource {
        if is_playing && !ended && stop_frame_ready {
            return SharedElementContentPolicyVideoSource::StopFrame;
        }
        if has_poster {
            return SharedElementContentPolicyVideoSource::Poster;
        }
        if has_grid_cover {
            return SharedElementContentPolicyVideoSource::GridCover;
        }
        if can_replicate {
            return SharedElementContentPolicyVideoSource::Replicant;
        }
        SharedElementContentPolicyVideoSource::FadeOnly
    }

    /// a cover crossfade is needed unless the flying content already is the cover image
    pub fn needs_cover(flying_identity: Option<&str>, cover_identity: Option<&str>) -> bool {
        let Some(cover_identity) = cover_identity else {
            return false;
        };
        let Some(flying_identity) = flying_identity else {
            return true;
        };
        flying_identity != cover_identity
    }
}
