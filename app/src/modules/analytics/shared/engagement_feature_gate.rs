use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{ENGAGEMENT_ATTENTION_DISABLED, ENGAGEMENT_DISABLED};

/// the only kill switch for engagement telemetry, no feature flag or remote config exists
/// check it at the call site before building the payload, gating later still allocates in the gesture
pub enum EngagementFeatureGate {}

/// event classes silenced separately because their volume is incomparable
/// an action is a few per session, an attention signal is dozens per second
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EngagementFeatureGateChannel {
    Action,
    Attention,
}

static ALL_DISABLED: LazyLock<AtomicBool> =
    LazyLock::new(|| AtomicBool::new(UserDefaults::standard().bool(ENGAGEMENT_DISABLED)));
static ATTENTION_DISABLED: LazyLock<AtomicBool> =
    LazyLock::new(|| AtomicBool::new(UserDefaults::standard().bool(ENGAGEMENT_ATTENTION_DISABLED)));

impl EngagementFeatureGate {
    #[inline(always)]
    pub fn allows(channel: EngagementFeatureGateChannel) -> bool {
        if ALL_DISABLED.load(Ordering::Relaxed) {
            return false;
        }
        match channel {
            EngagementFeatureGateChannel::Action => true,
            EngagementFeatureGateChannel::Attention => !ATTENTION_DISABLED.load(Ordering::Relaxed),
        }
    }

    /// persisted, a channel switched off stays off across cold start
    pub fn set(channel: Option<EngagementFeatureGateChannel>, disabled: bool) {
        match channel {
            None => {
                ALL_DISABLED.store(disabled, Ordering::Relaxed);
                UserDefaults::standard().set(ENGAGEMENT_DISABLED, disabled);
            }
            Some(EngagementFeatureGateChannel::Attention) => {
                ATTENTION_DISABLED.store(disabled, Ordering::Relaxed);
                UserDefaults::standard().set(ENGAGEMENT_ATTENTION_DISABLED, disabled);
            }
            Some(EngagementFeatureGateChannel::Action) => {
                ALL_DISABLED.store(disabled, Ordering::Relaxed);
                UserDefaults::standard().set(ENGAGEMENT_DISABLED, disabled);
            }
        }
    }
}
