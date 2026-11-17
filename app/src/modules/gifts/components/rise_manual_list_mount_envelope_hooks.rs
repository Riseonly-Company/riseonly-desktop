use gpui::{Pixels, px};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiseManualListMountEnvelope {
    pub ahead_distance: Pixels,
    pub behind_distance: Pixels,
    pub immediate_ahead_distance: Pixels,
    pub immediate_behind_distance: Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiseManualListDeferredMountBatch {
    pub maximum_mounts: usize,
    pub time_budget: f64,
}

pub enum RiseManualListMountEnvelopePolicy {}

impl RiseManualListMountEnvelopePolicy {
    pub const MAXIMUM_FALLBACK_HOSTED_MOUNTS_PER_IDLE_BATCH: usize = 1;
    pub const FAST_SCROLL_VELOCITY_THRESHOLD: f32 = 1_100.0;
    pub const MINIMUM_AHEAD_SCREENS: f32 = 1.25;
    pub const FALLBACK_HOSTED_REUSE_IDENTIFIER_PREFIX: &str = "RiseManualMessageHostingView";

    pub fn default_preload_distance(viewport_height: Pixels) -> Pixels {
        px((f32::from(viewport_height).max(0.0) * 0.58).clamp(360.0, 560.0))
    }

    pub fn resolve(preload_distance: Pixels, viewport_height: Pixels, velocity_y: f32) -> RiseManualListMountEnvelope {
        let preload = f32::from(preload_distance).max(0.0);
        if preload <= 0.0 {
            return RiseManualListMountEnvelope {
                ahead_distance: Pixels::ZERO,
                behind_distance: Pixels::ZERO,
                immediate_ahead_distance: Pixels::ZERO,
                immediate_behind_distance: Pixels::ZERO,
            };
        }

        let height = f32::from(viewport_height).max(1.0);
        let speed = velocity_y.abs().min(12_000.0);
        let fast_progress = ((speed - 500.0) / 4_500.0).clamp(0.0, 1.0);
        let ahead_boost = (height * 0.72).min(speed * 0.065);
        let behind_distance = (preload * (1.0 - fast_progress * 0.45)).max(160.0);
        let immediate_base = preload.min(180.0_f32.max((height * 0.42).min(380.0)).max((speed * 0.045).min(460.0)));
        let immediate_ahead_distance = immediate_base.min(180.0).max(immediate_base * (1.0 - fast_progress * 0.08));
        let immediate_behind_distance = immediate_base.min(120.0).max(immediate_base * (1.0 - fast_progress * 0.42));
        RiseManualListMountEnvelope {
            ahead_distance: px((preload + ahead_boost).max(height * Self::MINIMUM_AHEAD_SCREENS)),
            behind_distance: px(behind_distance),
            immediate_ahead_distance: px(immediate_ahead_distance),
            immediate_behind_distance: px(immediate_behind_distance),
        }
    }

    pub fn deferred_mount_batch(
        _velocity_y: f32,
        frames_per_second: usize,
        _representative_row_height: Pixels,
        is_idle: bool,
    ) -> RiseManualListDeferredMountBatch {
        let frames_per_second = frames_per_second.clamp(30, 240);
        let frame_duration = 1.0 / frames_per_second as f64;
        if !is_idle {
            return RiseManualListDeferredMountBatch {
                maximum_mounts: 1,
                time_budget: (frame_duration * 0.12).clamp(0.001, 0.002),
            };
        }
        RiseManualListDeferredMountBatch {
            maximum_mounts: 2,
            time_budget: (frame_duration * 0.24).clamp(0.0015, 0.004),
        }
    }

    pub fn is_fallback_hosted_row(reuse_identifier: &str) -> bool {
        reuse_identifier
            .strip_prefix(Self::FALLBACK_HOSTED_REUSE_IDENTIFIER_PREFIX)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    }
}
