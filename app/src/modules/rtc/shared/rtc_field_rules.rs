use std::sync::LazyLock;

use rise_canon::{RiseRequestBody, rise_request};

use crate::core::lib::canon_field_rule::RiseFieldRule;

const RTC_REQUEST_TO_SPEAK: &str = <rise_request::rtc::RequestToSpeak as RiseRequestBody>::CONTRACT;
const RTC_MODERATE_PARTICIPANT: &str = <rise_request::rtc::ModerateParticipant as RiseRequestBody>::CONTRACT;

static SPEAK_NOTE: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new(RTC_REQUEST_TO_SPEAK, "/note"));
static MODERATION_REASON: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(RTC_MODERATE_PARTICIPANT, "/reason"));

pub enum RTCFieldRules {}

impl RTCFieldRules {
    pub fn speak_note() -> &'static RiseFieldRule {
        &SPEAK_NOTE
    }

    pub fn moderation_reason() -> &'static RiseFieldRule {
        &MODERATION_REASON
    }
}
