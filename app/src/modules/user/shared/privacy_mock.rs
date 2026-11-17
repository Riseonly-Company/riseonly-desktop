use std::sync::LazyLock;

use rise_canon::{RisePrivacyAudience, RisePrivacyRule};

use crate::core::lib::canon_privacy::RisePrivacyRuleCanon;
use crate::modules::user::stores::user::user_actions::user_wire_types::PrivacySettingsWireDTO;

/// one privacy screen row, key and labels and default all come from canon
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PrivacySetting {
    pub rule: RisePrivacyRule,
    pub audience: RisePrivacyAudience,
}

impl PrivacySetting {
    pub fn id(&self) -> &'static str {
        self.rule.raw_value()
    }

    pub fn title_key(&self) -> &'static str {
        self.rule.i18n_key().unwrap_or_default()
    }

    pub fn group_title_key(&self) -> Option<&'static str> {
        self.rule.group_title_i18n_key()
    }

    /// rule from a navigation route key resolved through canon
    /// so legacy spelling Lastseen_rule opens the same screen
    /// canon drops rules with no screen row
    pub fn template(for_key: Option<&str>) -> Option<PrivacySetting> {
        let rule = RisePrivacyRule::from_key(for_key?)?;
        if !rule.has_ios_row() {
            return None;
        }
        Some(PrivacySetting { rule, audience: rule.default_audience() })
    }
}

pub enum PrivacyRuleCatalog {}

static SETTINGS: LazyLock<Vec<PrivacySetting>> = LazyLock::new(|| {
    let mut rules: Vec<RisePrivacyRule> =
        RisePrivacyRule::ALL.into_iter().filter(|rule| rule.has_ios_row()).collect();
    rules.sort_by_key(|rule| PrivacyRuleCatalog::position(*rule));
    rules
        .into_iter()
        .map(|rule| PrivacySetting { rule, audience: rule.default_audience() })
        .collect()
});

impl PrivacyRuleCatalog {
    /// screen rows in display order
    /// membership comes from canon not this array so a new rule lands at the tail not vanishing
    pub fn settings() -> &'static [PrivacySetting] {
        &SETTINGS
    }

    pub const AUDIENCE_ORDER: [RisePrivacyAudience; 5] = [
        RisePrivacyAudience::All,
        RisePrivacyAudience::None,
        RisePrivacyAudience::Contacts,
        RisePrivacyAudience::Friends,
        RisePrivacyAudience::CloseFriends,
    ];

    const SCREEN_ORDER: [RisePrivacyRule; 20] = [
        RisePrivacyRule::Phone,
        RisePrivacyRule::ProfilePhoto,
        RisePrivacyRule::Description,
        RisePrivacyRule::Hb,
        RisePrivacyRule::Friend,
        RisePrivacyRule::Subs,
        RisePrivacyRule::Plan,
        RisePrivacyRule::Goal,
        RisePrivacyRule::Comments,
        RisePrivacyRule::Reposts,
        RisePrivacyRule::Coauthor,
        RisePrivacyRule::Tag,
        RisePrivacyRule::Stories,
        RisePrivacyRule::Messages,
        RisePrivacyRule::Sparks,
        RisePrivacyRule::Gifts,
        RisePrivacyRule::GiftSend,
        RisePrivacyRule::LastSeen,
        RisePrivacyRule::OnlineStatus,
        RisePrivacyRule::Achievements,
    ];

    fn position(rule: RisePrivacyRule) -> usize {
        if let Some(index) = Self::SCREEN_ORDER.iter().position(|candidate| *candidate == rule) {
            return index;
        }
        Self::SCREEN_ORDER.len()
            + RisePrivacyRule::ALL.iter().position(|candidate| *candidate == rule).unwrap_or(0)
    }
}

/// helpers on the privacy settings wire dto
pub trait PrivacySettingsWireDTORules {
    /// raw wire value of a rule, exhaustive over canon on purpose so a new rule breaks the build here
    fn raw_value(&self, for_rule: RisePrivacyRule) -> Option<&str>;

    /// rule audience parsed through canon,
    /// unknown spelling gives none and a missing value the column default
    fn audience(&self, for_rule: RisePrivacyRule) -> RisePrivacyAudience;
}

impl PrivacySettingsWireDTORules for PrivacySettingsWireDTO {
    fn raw_value(&self, for_rule: RisePrivacyRule) -> Option<&str> {
        let raw = match for_rule {
            RisePrivacyRule::Plan => &self.plan_rule,
            RisePrivacyRule::Goal => &self.goal_rule,
            RisePrivacyRule::Friend => &self.friend_rule,
            RisePrivacyRule::Hb => &self.hb_rule,
            RisePrivacyRule::Phone => &self.phone_rule,
            RisePrivacyRule::Description => &self.description_rule,
            RisePrivacyRule::LastSeen => &self.last_seen_rule,
            RisePrivacyRule::ProfilePhoto => &self.profile_photo_rule,
            RisePrivacyRule::Calls => &self.calls_rule,
            RisePrivacyRule::Forwards => &self.forwards_rule,
            RisePrivacyRule::Groups => &self.groups_rule,
            RisePrivacyRule::Subs => &self.subs_rule,
            RisePrivacyRule::OnlineStatus => &self.online_status_rule,
            RisePrivacyRule::Comments => &self.comments_rule,
            RisePrivacyRule::Reposts => &self.reposts_rule,
            RisePrivacyRule::Achievements => &self.achievements_rule,
            RisePrivacyRule::Stories => &self.stories_rule,
            RisePrivacyRule::Messages => &self.messages_rule,
            RisePrivacyRule::Coauthor => &self.coauthor_rule,
            RisePrivacyRule::Tag => &self.tag_rule,
            RisePrivacyRule::Sparks => &self.sparks_rule,
            RisePrivacyRule::Gifts => &self.gifts_rule,
            RisePrivacyRule::GiftSend => &self.gift_send_rule,
        };
        Some(raw.as_str())
    }

    fn audience(&self, for_rule: RisePrivacyRule) -> RisePrivacyAudience {
        for_rule.audience(self.raw_value(for_rule))
    }
}
