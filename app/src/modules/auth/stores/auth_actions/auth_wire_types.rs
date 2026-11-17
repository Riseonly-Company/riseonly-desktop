use rise_canon::rise_key::tag;
use rise_canon::{rise_proto, rise_reply, rise_request, rise_response, rise_view};
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::AppLanguageCatalog;
use rise_platform::HostOs;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::core::lib::device_info::DeviceInfo;
use crate::modules::auth::services::app_attest_client::{AppAttestClientEnvelope, RiseAttestedBody};
use crate::modules::auth::shared::device_identity::DeviceIdentity;
use crate::modules::auth::stores::auth_actions::author_info_more_hooks::AuthorInfoMore;
use crate::modules::auth::stores::auth_actions::profile_model_hooks::{ProfileModel, SubscriptionStatus};
use crate::modules::auth::stores::auth_actions::selectable_emoji_hooks::normalized_selectable_emoji_or_custom_ref;

pub type AuthSendCodeResponse = rise_view::auth::Sent;

pub trait AuthSendCodeResponseExt {
    fn code_sent(&self) -> bool;
}

impl AuthSendCodeResponseExt for AuthSendCodeResponse {
    fn code_sent(&self) -> bool {
        self.success
    }
}

/// http tag check forwards its body unchanged into check_tag_exist
pub type AuthCheckTagBody = rise_request::tag::CheckTagExist;

/// tag availability view, every field is optional so the screen can tell a missing verdict
/// from a negative one
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthCheckTagResponse {
    pub success: Option<bool>,
    pub available: Option<bool>,
    pub exists: Option<bool>,
    pub normalized_tag: Option<String>,
    pub error: Option<String>,
}

impl<'de> Deserialize<'de> for AuthCheckTagResponse {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_canon::rise_decode(deserializer, |container| {
            Ok(Self {
                success: container.rise(tag::SUCCESS)?,
                available: container.rise(tag::AVAILABLE)?,
                exists: container.rise(tag::EXISTS)?,
                normalized_tag: container.rise(tag::NORMALIZED_TAG)?,
                error: container.rise(tag::ERROR)?,
            })
        })
    }
}

impl Serialize for AuthCheckTagResponse {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(success) = &self.success {
            container.serialize_entry(tag::SUCCESS, success)?;
        }
        if let Some(available) = &self.available {
            container.serialize_entry(tag::AVAILABLE, available)?;
        }
        if let Some(exists) = &self.exists {
            container.serialize_entry(tag::EXISTS, exists)?;
        }
        if let Some(normalized_tag) = &self.normalized_tag {
            container.serialize_entry(tag::NORMALIZED_TAG, normalized_tag)?;
        }
        if let Some(error) = &self.error {
            container.serialize_entry(tag::ERROR, error)?;
        }
        container.end()
    }
}

pub type AuthRegisterBody = RiseAttestedBody<rise_request::auth::Register>;

impl RiseAttestedBody<rise_request::auth::Register> {
    /// registration is the only write of signup_region and signup_language, both frozen after
    /// without them the server guesses from the connection and a vpn would freeze the wrong market
    /// birth date stays optional on the wire
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        number: String,
        name: String,
        tag: String,
        code: String,
        password: String,
        birth_date: Option<String>,
        device_token: Option<String>,
        invite_code: Option<String>,
        attestation: Option<AppAttestClientEnvelope>,
    ) -> AuthRegisterBody {
        AuthRegisterBody {
            body: rise_request::auth::Register {
                birth_date,
                browser: Some(false),
                code: Some(code),
                device_info: Some(DeviceInfo::display_name()),
                device_languages: Some(
                    AppLanguageCatalog::device_preferences()
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                ),
                device_region: rise_platform::device_locale::current().region,
                device_token,
                device_type: Some(DeviceInfo::device_type().to_owned()),
                invite_code,
                name: Some(name),
                password: Some(password),
                phone_number: Some(number),
                platform: Some(DeviceIdentity::platform(HostOs::current()).to_owned()),
                tag: Some(tag),
                timezone: Some(TimeZone::current().identifier()),
                user_agent: Some(DeviceInfo::user_agent()),
                ..Default::default()
            },
            attestation,
        }
    }
}

pub type AuthRegisterResponse = rise_view::auth::Registered;

/// auth refresh answers with the canon refresh reply, tokens at top level
pub type AuthRefreshResponse = rise_response::auth::RefreshToken;

pub trait RefreshTokenResponseExt {
    fn resolved_tokens(&self) -> Option<(String, String, Option<String>)>;
}

impl RefreshTokenResponseExt for rise_proto::auth::RefreshTokenResponse {
    fn resolved_tokens(&self) -> Option<(String, String, Option<String>)> {
        if self.access_token.is_empty() || self.refresh_token.is_empty() {
            return None;
        }
        let session_id = (!self.session_id.is_empty()).then(|| self.session_id.clone());
        Some((self.access_token.clone(), self.refresh_token.clone(), session_id))
    }
}

pub type AuthLogoutResponse = rise_response::auth::Logout;

pub type AuthLoginBody = RiseAttestedBody<rise_request::auth::Login>;

impl RiseAttestedBody<rise_request::auth::Login> {
    /// the server keeps the country from its own address lookup, the zone only names the city in it
    pub fn build(
        phone_number: String,
        password: String,
        device_token: Option<String>,
        attestation: Option<AppAttestClientEnvelope>,
    ) -> AuthLoginBody {
        AuthLoginBody {
            body: rise_request::auth::Login {
                browser: Some(false),
                device_info: Some(DeviceInfo::display_name()),
                device_token,
                device_type: Some(DeviceInfo::device_type().to_owned()),
                password: Some(password),
                phone_number: Some(phone_number),
                platform: Some(DeviceIdentity::platform(HostOs::current()).to_owned()),
                timezone: Some(TimeZone::current().identifier()),
                user_agent: Some(DeviceInfo::user_agent()),
                ..Default::default()
            },
            attestation,
        }
    }
}

/// password recovery sends no device envelope
/// it makes no session and ends all of them, so nothing about this device is sent
pub type AuthResetPasswordResponse = rise_response::auth::ResetPassword;

pub type AuthLoginResponse = rise_view::auth::LoggedIn;
pub type AuthManagedSessionResponse = rise_reply::auth::ManagedSession;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuthUserMoreDTO {
    pub banner: Option<String>,
    pub description: Option<String>,
    pub friends: Option<i64>,
    pub hb: Option<String>,
    pub id: Option<String>,
    pub level: Option<i64>,
    pub logo: Option<String>,
    pub p_lang: Option<Vec<String>>,
    pub posts_count: Option<i64>,
    pub rating: Option<i64>,
    pub status: Option<String>,
    pub streak: Option<i64>,
    pub subscribers: Option<i64>,
    pub who: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuthUserDTO {
    pub id: String,
    pub name: Option<String>,
    pub tag: Option<String>,
    pub phone: Option<String>,
    pub is_phone_public: Option<bool>,
    pub gender: Option<String>,
    pub role: Option<String>,
    pub user_chat_id: Option<String>,
    pub customer_id: Option<String>,
    pub is_blocked: Option<bool>,
    pub is_premium: Option<bool>,
    pub is_official: Option<bool>,
    pub theme_id: Option<String>,
    pub more_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub subscription_status: Option<String>,
    pub subscription_auto_renew: Option<bool>,
    pub subscription_cancelled_at: Option<String>,
    pub subscription_end_date: Option<String>,
    pub subscription_id: Option<String>,
    pub subscription_payment_method_id: Option<String>,
    pub subscription_period: Option<String>,
    pub subscription_price_id: Option<String>,
    pub subscription_provider: Option<String>,
    pub subscription_start_date: Option<String>,
    pub more: Option<AuthUserMoreDTO>,
    pub posts_count: Option<i64>,
    pub subs_count: Option<i64>,
    pub subscribers_count: Option<i64>,
    pub friends_count: Option<i64>,
    pub avatar_url: Option<String>,
    pub cover_image_url: Option<String>,
    pub description: Option<String>,
    pub quick_reaction: Option<String>,
    pub premium_badge_emoji: Option<String>,
    pub name_color: Option<String>,
    pub alias_tags: Option<Vec<String>>,
}

impl AuthUserDTO {
    pub fn as_profile_model(&self) -> ProfileModel {
        let m = self.more.as_ref();
        let posts = self
            .posts_count
            .or_else(|| m.and_then(|more| more.posts_count))
            .unwrap_or(0);
        let subs = self.subs_count.unwrap_or(0);
        let subscribers = self
            .subscribers_count
            .or_else(|| m.and_then(|more| more.subscribers))
            .unwrap_or(0);
        let friends = self
            .friends_count
            .or_else(|| m.and_then(|more| more.friends))
            .unwrap_or(0);
        let desc = m
            .and_then(|more| more.description.clone())
            .or_else(|| self.description.clone());
        let sub_raw = self
            .subscription_status
            .as_deref()
            .map(|status| trimming_whitespaces_and_newlines(status).to_lowercase())
            .unwrap_or_default();
        let sub = if sub_raw.is_empty() {
            SubscriptionStatus::Inactive
        } else {
            SubscriptionStatus::from_raw(&sub_raw).unwrap_or(SubscriptionStatus::Inactive)
        };
        let logo = m.and_then(|more| more.logo.clone());

        ProfileModel {
            id: self.id.clone(),
            name: self.name.clone().unwrap_or_default(),
            tag: self.tag.clone().unwrap_or_default(),
            alias_tags: self.alias_tags.clone().unwrap_or_default(),
            description: desc,
            more: AuthorInfoMore {
                p_lang: m.and_then(|more| more.p_lang.clone()).unwrap_or_default(),
                who: m.and_then(|more| more.who.clone()).unwrap_or_default(),
                role: self.role.clone().unwrap_or_default(),
                logo: logo.clone().unwrap_or_default(),
                is_premium: self.is_premium.unwrap_or(false),
                is_official: self.is_official.unwrap_or(false),
                phone: self.phone.clone(),
                premium_badge_emoji: self.premium_badge_emoji.clone(),
                name_color: self.name_color.clone(),
            },
            posts_count: posts,
            subs_count: subs,
            subscribers_count: subscribers,
            friends_count: friends,
            subscription_status: sub,
            cover_image_url: self.cover_image_url.clone(),
            avatar_url: self.avatar_url.clone().or(logo),
            hb: m.and_then(|more| more.hb.clone()),
            gender: self.gender.clone(),
            level: m.and_then(|more| more.level),
            streak: m.and_then(|more| more.streak),
            rating: m.and_then(|more| more.rating),
            quick_reaction: normalized_selectable_emoji_or_custom_ref(self.quick_reaction.as_deref())
                .unwrap_or_else(|| ProfileModel::DEFAULT_QUICK_REACTION.to_owned()),
            is_phone_public: self.is_phone_public.unwrap_or(false),
            ..ProfileModel::default()
        }
    }
}

impl AuthUserMoreDTO {
    pub fn from_canon(more: &rise_view::auth::MoreData) -> Self {
        Self {
            banner: Some(more.banner.clone()),
            description: Some(more.description.clone()),
            friends: Some(more.friends),
            hb: Some(more.hb.clone()),
            id: Some(more.id.clone()),
            level: Some(more.level),
            logo: Some(more.logo.clone()),
            p_lang: Some(more.p_lang.clone()),
            posts_count: Some(more.posts_count),
            rating: Some(more.rating),
            status: Some(more.status.clone()),
            streak: Some(more.streak),
            subscribers: Some(more.subscribers),
            who: Some(more.who.clone()),
        }
    }
}

impl AuthUserDTO {
    pub fn from_canon(user: &rise_view::auth::UserData) -> Self {
        Self {
            id: user.id.clone(),
            name: Some(user.name.clone()),
            tag: Some(user.tag.clone()),
            phone: Some(user.phone.clone()),
            gender: Some(user.gender.clone()),
            role: Some(user.role.clone()),
            user_chat_id: Some(user.user_chat_id.clone()),
            customer_id: Some(user.customer_id.clone()),
            is_blocked: Some(user.is_blocked),
            is_premium: Some(user.is_premium),
            is_official: Some(user.is_official),
            theme_id: Some(user.theme_id.clone()),
            more_id: Some(user.more_id.clone()),
            created_at: Some(user.created_at.clone()),
            updated_at: Some(user.updated_at.clone()),
            subscription_status: Some(user.subscription_status.clone()),
            subscription_auto_renew: Some(user.subscription_auto_renew),
            subscription_cancelled_at: Some(user.subscription_cancelled_at.clone()),
            subscription_end_date: Some(user.subscription_end_date.clone()),
            subscription_id: Some(user.subscription_id.clone()),
            subscription_payment_method_id: Some(user.subscription_payment_method_id.clone()),
            subscription_period: Some(user.subscription_period.clone()),
            subscription_price_id: Some(user.subscription_price_id.clone()),
            subscription_provider: Some(user.subscription_provider.clone()),
            subscription_start_date: Some(user.subscription_start_date.clone()),
            more: user.more.as_ref().map(AuthUserMoreDTO::from_canon),
            ..Self::default()
        }
    }

    pub fn from_managed(user: &rise_reply::auth::ManagedUser) -> Self {
        Self {
            id: user.id.clone(),
            name: Some(user.name.clone()),
            tag: Some(user.tag.clone()),
            more: Some(AuthUserMoreDTO {
                logo: Some(user.logo.clone()),
                ..AuthUserMoreDTO::default()
            }),
            avatar_url: Some(user.avatar_url.clone()),
            ..Self::default()
        }
    }
}
