use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rise_canon::rise_request::user::update_theme_settings;
use rise_canon::{RiseWireRequestMetadata, rise_request, rise_response, rise_wire_methods};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_endpoints::EndpointKind as RiseEndpointKind;
use rise_engine::{RiseEngine, RiseWireMethodDescriptor, RiseWireReplayPolicy};
use rise_theme::{RiseThemeAppearance, RiseThemePlatform};

use crate::core::rise_engine_bridge::rise_canon_endpoints::{
    RiseCanonEndpoints, RiseEndpointClientTimeout,
};
use crate::core::widgets::navigations::main_tab_bar_settings::{
    RiseTabBarPlatform, RiseTabBarSettingsChange,
};
use crate::modules::user::engine::core::rise_user_tab_bar_wire_models::UpdateTabBarSettingsChange;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseUserEndpointReplay {
    ReadOnly,
    Never,
}

/// user service method from the canon catalogue, deadline and replay policy come from there
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseUserEndpoint {
    pub method: String,
    pub timeout: Duration,
    pub replay: RiseUserEndpointReplay,
}

impl RiseUserEndpoint {
    /// endpoint from a canon method name, expecting checks the declared kind
    #[track_caller]
    pub fn canonical(method: &str, expecting: RiseEndpointKind) -> Self {
        let entry = RiseCanonEndpoints::required(rise_wire_methods::user::SERVICE, method);
        debug_assert!(
            entry.kind == expecting,
            "{}.{method}: canon declares {}, not {}",
            rise_wire_methods::user::SERVICE,
            entry.kind.name(),
            expecting.name()
        );
        Self {
            method: entry.method.to_owned(),
            timeout: entry.client_timeout(),
            replay: if entry.kind == RiseEndpointKind::Read {
                RiseUserEndpointReplay::ReadOnly
            } else {
                RiseUserEndpointReplay::Never
            },
        }
    }

    pub fn descriptor(&self) -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(rise_wire_methods::user::SERVICE, self.method.clone())
            .with_timeout(self.timeout)
            .with_replay_policy(match self.replay {
                RiseUserEndpointReplay::ReadOnly => RiseWireReplayPolicy::ReadOnly,
                RiseUserEndpointReplay::Never => RiseWireReplayPolicy::Never,
            })
    }
}

macro_rules! rise_user_endpoints {
    ($( $(#[$meta:meta])* $name:ident => ($static_name:ident, $method:ident, $kind:ident), )*) => {
        $(
            static $static_name: LazyLock<RiseUserEndpoint> = LazyLock::new(|| {
                RiseUserEndpoint::canonical(rise_wire_methods::user::$method, RiseEndpointKind::$kind)
            });
        )*

        pub enum RiseUserEndpoints {}

        impl RiseUserEndpoints {
            $(
                $(#[$meta])*
                pub fn $name() -> &'static RiseUserEndpoint {
                    &$static_name
                }
            )*

            pub fn all() -> Vec<&'static RiseUserEndpoint> {
                vec![$(Self::$name(),)*]
            }
        }
    };
}

rise_user_endpoints! {
    get_my_profile => (GET_MY_PROFILE, GET_MY_PROFILE, Read),
    get_theme_settings => (GET_THEME_SETTINGS, GET_THEME_SETTINGS, Read),
    update_theme_settings => (UPDATE_THEME_SETTINGS, UPDATE_THEME_SETTINGS, Mutation),
    get_tab_bar_settings => (GET_TAB_BAR_SETTINGS, GET_TAB_BAR_SETTINGS, Read),
    /// not replayable,
    /// a failed save stays on this device until the server answers a read and the server wins
    update_tab_bar_settings => (UPDATE_TAB_BAR_SETTINGS, UPDATE_TAB_BAR_SETTINGS, Mutation),
    get_call_audio_settings => (GET_CALL_AUDIO_SETTINGS, GET_CALL_AUDIO_SETTINGS, Read),
    /// not replayable, slider is sent on release and a replay would push a stale threshold
    update_call_audio_settings => (UPDATE_CALL_AUDIO_SETTINGS, UPDATE_CALL_AUDIO_SETTINGS, Mutation),
    get_locale_settings => (GET_LOCALE_SETTINGS, GET_LOCALE_SETTINGS, Read),
    /// not replayable, a retry would resurrect a pref changed since on another device
    update_locale_settings => (UPDATE_LOCALE_SETTINGS, UPDATE_LOCALE_SETTINGS, Mutation),
    /// not replayable, these observe the current device state and a replay could report one thats gone
    observe_locale_signals => (OBSERVE_LOCALE_SIGNALS, OBSERVE_LOCALE_SIGNALS, Mutation),
    get_my_achievements => (GET_MY_ACHIEVEMENTS, GET_MY_ACHIEVEMENTS, Read),
    get_user_achievements => (GET_USER_ACHIEVEMENTS, GET_USER_ACHIEVEMENTS, Read),
    /// not replayable, a retried toggle would re-hide an achievement the owner restored
    set_achievement_hidden => (SET_ACHIEVEMENT_HIDDEN, SET_ACHIEVEMENT_HIDDEN, Mutation),
    /// not replayable,
    /// seen claims the reader was present and a replay would report a moment that never happened
    mark_achievements_seen => (MARK_ACHIEVEMENTS_SEEN, MARK_ACHIEVEMENTS_SEEN, Mutation),
    /// erases the account on demand, must never be replayable
    /// a queued retry could erase an account the user kept
    delete_my_account => (DELETE_MY_ACCOUNT, DELETE_MY_ACCOUNT, Mutation),
    get_profile_photos => (GET_PROFILE_PHOTOS, GET_PROFILE_PHOTOS, Read),
    /// not replayable, a retry after picking another photo would restore the old one
    set_main_profile_photo => (SET_MAIN_PROFILE_PHOTO, SET_MAIN_PROFILE_PHOTO, Mutation),
    /// server dedups on user_id and file_url and promotes, so repeats are harmless
    /// still a plain mutation, a replay after a delete would restore the photo
    /// uploader retries on its own
    add_profile_photo => (ADD_PROFILE_PHOTO, ADD_PROFILE_PHOTO, Mutation),
    /// not replayable,
    /// the row is gone after the first call so a replay hits an id the album no longer holds
    delete_profile_photo => (DELETE_PROFILE_PHOTO, DELETE_PROFILE_PHOTO, Mutation),
    get_blocked_users => (GET_BLOCKED_USERS, GET_BLOCKED_USERS, Read),
    /// not replayable, a block replayed after the user lifted it would restore it at the queued tier
    block_user => (BLOCK_USER, BLOCK_USER, Mutation),
    /// not replayable, an unblock arriving after a re-block would unblock someone just blocked
    unblock_user => (UNBLOCK_USER, UNBLOCK_USER, Mutation),
}

impl RiseUserEndpoints {
    pub fn endpoint(method: &str) -> Option<&'static RiseUserEndpoint> {
        let method = trimming_whitespaces_and_newlines(method);
        Self::all().into_iter().find(|endpoint| endpoint.method == method)
    }
}

/// converts the theme platform into the update request platform
pub(crate) fn update_theme_settings_platform(
    platform: RiseThemePlatform,
) -> update_theme_settings::Platform {
    match platform {
        RiseThemePlatform::Mobile => update_theme_settings::Platform::Mobile,
        RiseThemePlatform::Desktop => update_theme_settings::Platform::Desktop,
        RiseThemePlatform::Web => update_theme_settings::Platform::Web,
    }
}

fn update_theme_settings_theme(theme: RiseThemeAppearance) -> Option<update_theme_settings::Theme> {
    update_theme_settings::Theme::ALL_CASES
        .iter()
        .copied()
        .find(|candidate| candidate.raw_value() == theme.raw_value())
}

#[derive(Clone)]
pub struct RiseUserRPC {
    pub engine: Arc<RiseEngine>,
}

impl RiseUserRPC {
    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self { engine }
    }

    pub async fn get_my_profile(&self) -> anyhow::Result<rise_response::user::GetMyProfile> {
        self.call(RiseUserEndpoints::get_my_profile(), &rise_request::user::GetMyProfile {})
            .await
    }

    pub async fn get_theme_settings(&self) -> anyhow::Result<rise_response::user::GetThemeSettings> {
        self.call(
            RiseUserEndpoints::get_theme_settings(),
            &rise_request::user::GetThemeSettings {},
        )
        .await
    }

    pub async fn update_theme_settings(
        &self,
        platform: RiseThemePlatform,
        theme: Option<RiseThemeAppearance>,
    ) -> anyhow::Result<rise_response::user::UpdateThemeSettings> {
        self.call(
            RiseUserEndpoints::update_theme_settings(),
            &rise_request::user::UpdateThemeSettings {
                platform: Some(update_theme_settings_platform(platform)),
                theme: theme.and_then(update_theme_settings_theme),
            },
        )
        .await
    }

    pub async fn get_tab_bar_settings(
        &self,
    ) -> anyhow::Result<rise_response::user::GetTabBarSettings> {
        self.call(
            RiseUserEndpoints::get_tab_bar_settings(),
            &rise_request::user::GetTabBarSettings {},
        )
        .await
    }

    pub async fn update_tab_bar_settings(
        &self,
        platform: RiseTabBarPlatform,
        change: RiseTabBarSettingsChange,
    ) -> anyhow::Result<rise_response::user::UpdateTabBarSettings> {
        self.call(
            RiseUserEndpoints::update_tab_bar_settings(),
            &rise_request::user::UpdateTabBarSettings::from_change(platform, change),
        )
        .await
    }

    pub async fn get_call_audio_settings(
        &self,
    ) -> anyhow::Result<rise_response::user::GetCallAudioSettings> {
        self.call(
            RiseUserEndpoints::get_call_audio_settings(),
            &rise_request::user::GetCallAudioSettings {},
        )
        .await
    }

    pub async fn update_call_audio_settings(
        &self,
        platform: RiseTabBarPlatform,
        input_sensitivity_permille: i64,
    ) -> anyhow::Result<rise_response::user::UpdateCallAudioSettings> {
        self.call(
            RiseUserEndpoints::update_call_audio_settings(),
            &rise_request::user::UpdateCallAudioSettings {
                input_sensitivity_permille,
                platform: platform.raw_value().to_owned(),
                reset: None,
            },
        )
        .await
    }

    pub async fn get_locale_settings(
        &self,
    ) -> anyhow::Result<rise_response::user::GetLocaleSettings> {
        self.call(
            RiseUserEndpoints::get_locale_settings(),
            &rise_request::user::GetLocaleSettings {},
        )
        .await
    }

    pub async fn update_locale_settings(
        &self,
        request: rise_request::user::UpdateLocaleSettings,
    ) -> anyhow::Result<rise_response::user::UpdateLocaleSettings> {
        self.call(RiseUserEndpoints::update_locale_settings(), &request).await
    }

    pub async fn observe_locale_signals(
        &self,
        request: rise_request::user::ObserveLocaleSignals,
    ) -> anyhow::Result<rise_response::user::ObserveLocaleSignals> {
        self.call(RiseUserEndpoints::observe_locale_signals(), &request).await
    }

    pub async fn delete_my_account(&self) -> anyhow::Result<rise_response::user::DeleteMyAccount> {
        self.call(
            RiseUserEndpoints::delete_my_account(),
            &rise_request::user::DeleteMyAccount {},
        )
        .await
    }

    pub async fn get_profile_photos(
        &self,
        request: rise_request::user::GetProfilePhotos,
    ) -> anyhow::Result<rise_response::user::GetProfilePhotos> {
        self.call(RiseUserEndpoints::get_profile_photos(), &request).await
    }

    pub async fn set_main_profile_photo(
        &self,
        request: rise_request::user::SetMainProfilePhoto,
    ) -> anyhow::Result<rise_response::user::SetMainProfilePhoto> {
        self.call(RiseUserEndpoints::set_main_profile_photo(), &request).await
    }

    pub async fn add_profile_photo(
        &self,
        request: rise_request::user::AddProfilePhoto,
    ) -> anyhow::Result<rise_response::user::AddProfilePhoto> {
        self.call(RiseUserEndpoints::add_profile_photo(), &request).await
    }

    pub async fn delete_profile_photo(
        &self,
        request: rise_request::user::DeleteProfilePhoto,
    ) -> anyhow::Result<rise_response::user::DeleteProfilePhoto> {
        self.call(RiseUserEndpoints::delete_profile_photo(), &request).await
    }

    pub async fn get_blocked_users(
        &self,
        request: rise_request::user::GetBlockedUsers,
    ) -> anyhow::Result<rise_response::user::GetBlockedUsers> {
        self.call(RiseUserEndpoints::get_blocked_users(), &request).await
    }

    pub async fn block_user(
        &self,
        request: rise_request::user::BlockUser,
    ) -> anyhow::Result<rise_response::user::BlockUser> {
        self.call(RiseUserEndpoints::block_user(), &request).await
    }

    pub async fn unblock_user(
        &self,
        request: rise_request::user::UnblockUser,
    ) -> anyhow::Result<rise_response::user::UnblockUser> {
        self.call(RiseUserEndpoints::unblock_user(), &request).await
    }

    pub async fn get_my_achievements(
        &self,
    ) -> anyhow::Result<rise_response::user::GetMyAchievements> {
        self.call(
            RiseUserEndpoints::get_my_achievements(),
            &rise_request::user::GetMyAchievements {},
        )
        .await
    }

    pub async fn get_user_achievements(
        &self,
        request: rise_request::user::GetUserAchievements,
    ) -> anyhow::Result<rise_response::user::GetUserAchievements> {
        self.call(RiseUserEndpoints::get_user_achievements(), &request).await
    }

    pub async fn set_achievement_hidden(
        &self,
        request: rise_request::user::SetAchievementHidden,
    ) -> anyhow::Result<rise_response::user::SetAchievementHidden> {
        self.call(RiseUserEndpoints::set_achievement_hidden(), &request).await
    }

    pub async fn mark_achievements_seen(
        &self,
        request: rise_request::user::MarkAchievementsSeen,
    ) -> anyhow::Result<rise_response::user::MarkAchievementsSeen> {
        self.call(RiseUserEndpoints::mark_achievements_seen(), &request).await
    }

    async fn call<Request, Response>(
        &self,
        endpoint: &RiseUserEndpoint,
        request: &Request,
    ) -> anyhow::Result<Response>
    where
        Request: serde::Serialize + Sync,
        Response: serde::de::DeserializeOwned,
    {
        self.engine
            .call_descriptor_response(
                &endpoint.descriptor(),
                request,
                None,
                RiseWireRequestMetadata::default(),
            )
            .await
    }
}
