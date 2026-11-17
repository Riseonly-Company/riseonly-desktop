use rise_canon::rise_event::user::AchievementEarned;
use rise_core::foundation::foundation_date::Date;
use rise_engine::RiseWirePushEnvelope;

use crate::modules::user::engine::core::rise_user_achievement_wire_models::RiseAchievement;

/// achievement awarded over the socket,
/// keyed by the achievement so a duplicate frame doesnt queue a second modal
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseAchievementGrant {
    pub achievement: RiseAchievement,
    pub occurred_at_ms: i64,
}

impl RiseAchievementGrant {
    pub fn id(&self) -> i64 {
        self.achievement.id
    }
}

pub enum RiseUserAchievementsRealtime {}

impl RiseUserAchievementsRealtime {
    pub const EVENT_TYPE: &'static str = rise_events::name::ACHIEVEMENT_EARNED;

    /// must be total, a failing reducer forces a full resync so anything malformed returns none
    pub fn decode(envelope: &RiseWirePushEnvelope) -> Option<RiseAchievementGrant> {
        let payload: AchievementEarned = envelope.data.decode_as().ok()?;
        let achievement = RiseAchievement::from_event(&payload);
        if achievement.id <= 0 {
            return None;
        }

        let occurred_at_ms = if achievement.granted_at_ms > 0 {
            achievement.granted_at_ms
        } else {
            Date::now().ms()
        };
        Some(RiseAchievementGrant {
            achievement,
            occurred_at_ms,
        })
    }
}
