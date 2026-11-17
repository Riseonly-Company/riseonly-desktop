use std::fmt;

use rise_canon::RiseLimits;
use rise_core::foundation::foundation_date::Date;
use rise_i18n::tr;
use serde::{Deserialize, Serialize};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokensAndOtherData {
    pub access_token: String,
    pub refresh_token: String,
    pub session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredAccessTokens {
    pub access_token: String,
    pub refresh_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AuthAccountSummary {
    pub user_id: String,
    pub name: String,
    pub tag: String,
    pub avatar_url: Option<String>,
    pub is_premium: bool,
    pub last_used_at: i64,
    pub is_mirror: bool,
}

impl AuthAccountSummary {
    pub fn id(&self) -> &str {
        &self.user_id
    }
}

pub enum AuthAccountLimitPolicy {}

impl AuthAccountLimitPolicy {
    pub fn standard_limit() -> i64 {
        RiseLimits::client_value("auth_account_limit_policy_standard_limit")
    }

    pub fn premium_limit() -> i64 {
        RiseLimits::client_value("auth_account_limit_policy_premium_limit")
    }

    pub fn limit(is_premium: bool) -> i64 {
        if is_premium {
            Self::premium_limit()
        } else {
            Self::standard_limit()
        }
    }

    pub fn can_add_account(existing_count: i64, is_premium: bool) -> bool {
        existing_count < Self::limit(is_premium)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthAccountError {
    LimitReached,
}

impl AuthAccountError {
    pub fn error_description(&self) -> Option<String> {
        Some(tr("account_limit_reached"))
    }
}

impl fmt::Display for AuthAccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for AuthAccountError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignInFailure {
    MissingPayload,
}

impl SignInFailure {
    pub fn error_description(&self) -> Option<String> {
        match self {
            Self::MissingPayload => Some(tr("auth_signin_error_title")),
        }
    }
}

impl fmt::Display for SignInFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for SignInFailure {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AuthCodeChannel {
    Sms,
    FlashCall,
    Viber,
    TelegramBot,
}

impl AuthCodeChannel {
    const ALL: [AuthCodeChannel; 4] = [
        AuthCodeChannel::Sms,
        AuthCodeChannel::FlashCall,
        AuthCodeChannel::Viber,
        AuthCodeChannel::TelegramBot,
    ];

    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Sms => rise_otp::Channel::Sms.wire(),
            Self::FlashCall => rise_otp::Channel::FlashCall.wire(),
            Self::Viber => rise_otp::Channel::Viber.wire(),
            Self::TelegramBot => rise_otp::Channel::TelegramBot.wire(),
        }
    }

    pub fn from_raw(raw_value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|channel| channel.raw_value() == raw_value)
    }

    pub fn switch_title_key(&self) -> Option<&'static str> {
        match self {
            Self::Sms => Some("otp_sms_instead"),
            Self::FlashCall => Some("otp_call_instead"),
            Self::Viber => Some("otp_viber_instead"),
            Self::TelegramBot => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthCodeDelivery {
    pub phone_digits: String,
    pub purpose: Option<String>,
    pub channel: Option<AuthCodeChannel>,
    pub destination_masked: String,
    pub code_length: i64,
    pub resend_available_at: Date,
    pub alternatives: Vec<AuthCodeChannel>,
    pub offers_telegram: bool,
    pub named_by_server: bool,
}

impl AuthCodeDelivery {
    pub fn offers_autofill(&self) -> bool {
        self.channel != Some(AuthCodeChannel::FlashCall)
    }

    /// a bot code with own delivery keeps its row too
    /// its the way back into the bot and a fresh code when no provider serves the country
    pub fn offers_resend(&self) -> bool {
        let Some(channel) = self.channel else {
            return false;
        };
        channel != AuthCodeChannel::TelegramBot || self.named_by_server
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthFlowFailure {
    MissingPayload,
}

impl AuthFlowFailure {
    pub fn error_description(&self) -> Option<String> {
        match self {
            Self::MissingPayload => Some(tr("register_error")),
        }
    }
}

impl fmt::Display for AuthFlowFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for AuthFlowFailure {}
