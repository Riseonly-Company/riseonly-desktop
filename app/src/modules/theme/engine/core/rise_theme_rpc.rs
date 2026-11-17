use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rise_canon::{RiseWireRequestMetadata, rise_request, rise_response, rise_wire_methods};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_endpoints::EndpointKind as RiseEndpointKind;
use rise_engine::{RiseEngine, RiseWireMethodDescriptor, RiseWireReplayPolicy};

use crate::core::rise_engine_bridge::rise_canon_endpoints::{
    RiseCanonEndpoints, RiseEndpointClientTimeout,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseThemeEndpointReplay {
    ReadOnly,
    Never,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseThemeEndpoint {
    pub method: String,
    pub timeout: Duration,
    pub replay: RiseThemeEndpointReplay,
}

impl RiseThemeEndpoint {
    #[track_caller]
    pub fn canonical(method: &str, expecting: RiseEndpointKind) -> Self {
        let entry = RiseCanonEndpoints::required(rise_wire_methods::theme::SERVICE, method);
        debug_assert!(
            entry.kind == expecting,
            "{}.{method}: canon declares {}, not {}",
            rise_wire_methods::theme::SERVICE,
            entry.kind.name(),
            expecting.name()
        );
        Self {
            method: entry.method.to_owned(),
            timeout: entry.client_timeout(),
            replay: if entry.kind == RiseEndpointKind::Read {
                RiseThemeEndpointReplay::ReadOnly
            } else {
                RiseThemeEndpointReplay::Never
            },
        }
    }

    pub fn descriptor(&self) -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(rise_wire_methods::theme::SERVICE, self.method.clone())
            .with_timeout(self.timeout)
            .with_replay_policy(match self.replay {
                RiseThemeEndpointReplay::ReadOnly => RiseWireReplayPolicy::ReadOnly,
                RiseThemeEndpointReplay::Never => RiseWireReplayPolicy::Never,
            })
    }
}

static LIST_WALLPAPERS: LazyLock<RiseThemeEndpoint> = LazyLock::new(|| {
    RiseThemeEndpoint::canonical(rise_wire_methods::theme::LIST_WALLPAPERS, RiseEndpointKind::Read)
});

static CREATE_WALLPAPER: LazyLock<RiseThemeEndpoint> = LazyLock::new(|| {
    RiseThemeEndpoint::canonical(rise_wire_methods::theme::CREATE_WALLPAPER, RiseEndpointKind::Mutation)
});

static SET_DEFAULT_WALLPAPER: LazyLock<RiseThemeEndpoint> = LazyLock::new(|| {
    RiseThemeEndpoint::canonical(rise_wire_methods::theme::SET_DEFAULT_WALLPAPER, RiseEndpointKind::Mutation)
});

static DELETE_WALLPAPER: LazyLock<RiseThemeEndpoint> = LazyLock::new(|| {
    RiseThemeEndpoint::canonical(rise_wire_methods::theme::DELETE_WALLPAPER, RiseEndpointKind::Mutation)
});

static UPDATE_WALLPAPER: LazyLock<RiseThemeEndpoint> = LazyLock::new(|| {
    RiseThemeEndpoint::canonical(rise_wire_methods::theme::UPDATE_WALLPAPER, RiseEndpointKind::Mutation)
});

static GET_WALLPAPER_PREFERENCES: LazyLock<RiseThemeEndpoint> = LazyLock::new(|| {
    RiseThemeEndpoint::canonical(rise_wire_methods::theme::GET_WALLPAPER_PREFERENCES, RiseEndpointKind::Read)
});

static SET_WALLPAPER_PREFERENCES: LazyLock<RiseThemeEndpoint> = LazyLock::new(|| {
    RiseThemeEndpoint::canonical(
        rise_wire_methods::theme::SET_WALLPAPER_PREFERENCES,
        RiseEndpointKind::Mutation,
    )
});

static SET_CHAT_WALLPAPER_PREFERENCE: LazyLock<RiseThemeEndpoint> = LazyLock::new(|| {
    RiseThemeEndpoint::canonical(
        rise_wire_methods::theme::SET_CHAT_WALLPAPER_PREFERENCE,
        RiseEndpointKind::Mutation,
    )
});

static CREATE_THEME: LazyLock<RiseThemeEndpoint> = LazyLock::new(|| {
    RiseThemeEndpoint::canonical(rise_wire_methods::theme::CREATE_THEME, RiseEndpointKind::Mutation)
});

static GET_THEMES: LazyLock<RiseThemeEndpoint> = LazyLock::new(|| {
    RiseThemeEndpoint::canonical(rise_wire_methods::theme::GET_THEMES, RiseEndpointKind::Read)
});

static GET_THEME: LazyLock<RiseThemeEndpoint> = LazyLock::new(|| {
    RiseThemeEndpoint::canonical(rise_wire_methods::theme::GET_THEME, RiseEndpointKind::Read)
});

static UPDATE_THEME: LazyLock<RiseThemeEndpoint> = LazyLock::new(|| {
    RiseThemeEndpoint::canonical(rise_wire_methods::theme::UPDATE_THEME, RiseEndpointKind::Mutation)
});

static DELETE_THEME: LazyLock<RiseThemeEndpoint> = LazyLock::new(|| {
    RiseThemeEndpoint::canonical(rise_wire_methods::theme::DELETE_THEME, RiseEndpointKind::Mutation)
});

pub enum RiseThemeEndpoints {}

impl RiseThemeEndpoints {
    pub fn list_wallpapers() -> &'static RiseThemeEndpoint {
        &LIST_WALLPAPERS
    }

    pub fn create_wallpaper() -> &'static RiseThemeEndpoint {
        &CREATE_WALLPAPER
    }

    pub fn set_default_wallpaper() -> &'static RiseThemeEndpoint {
        &SET_DEFAULT_WALLPAPER
    }

    pub fn delete_wallpaper() -> &'static RiseThemeEndpoint {
        &DELETE_WALLPAPER
    }

    pub fn update_wallpaper() -> &'static RiseThemeEndpoint {
        &UPDATE_WALLPAPER
    }

    pub fn get_wallpaper_preferences() -> &'static RiseThemeEndpoint {
        &GET_WALLPAPER_PREFERENCES
    }

    pub fn set_wallpaper_preferences() -> &'static RiseThemeEndpoint {
        &SET_WALLPAPER_PREFERENCES
    }

    pub fn set_chat_wallpaper_preference() -> &'static RiseThemeEndpoint {
        &SET_CHAT_WALLPAPER_PREFERENCE
    }

    /// not replayable, each call mints a new saved theme against the per user quota
    pub fn create_theme() -> &'static RiseThemeEndpoint {
        &CREATE_THEME
    }

    pub fn get_themes() -> &'static RiseThemeEndpoint {
        &GET_THEMES
    }

    pub fn get_theme() -> &'static RiseThemeEndpoint {
        &GET_THEME
    }

    pub fn update_theme() -> &'static RiseThemeEndpoint {
        &UPDATE_THEME
    }

    pub fn delete_theme() -> &'static RiseThemeEndpoint {
        &DELETE_THEME
    }

    pub fn all() -> [&'static RiseThemeEndpoint; 13] {
        [
            Self::create_theme(),
            Self::get_themes(),
            Self::get_theme(),
            Self::update_theme(),
            Self::delete_theme(),
            Self::list_wallpapers(),
            Self::create_wallpaper(),
            Self::set_default_wallpaper(),
            Self::delete_wallpaper(),
            Self::update_wallpaper(),
            Self::get_wallpaper_preferences(),
            Self::set_wallpaper_preferences(),
            Self::set_chat_wallpaper_preference(),
        ]
    }

    pub fn endpoint(method: &str) -> Option<&'static RiseThemeEndpoint> {
        let method = trimming_whitespaces_and_newlines(method);
        Self::all().into_iter().find(|endpoint| endpoint.method == method)
    }
}

#[derive(Clone)]
pub struct RiseThemeRPC {
    pub engine: Arc<RiseEngine>,
}

impl RiseThemeRPC {
    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self { engine }
    }

    pub async fn list_wallpapers(
        &self,
        request: rise_request::theme::ListWallpapers,
    ) -> anyhow::Result<rise_response::theme::ListWallpapers> {
        self.call(RiseThemeEndpoints::list_wallpapers(), &request).await
    }

    pub async fn create_wallpaper(
        &self,
        request: rise_request::theme::CreateWallpaper,
    ) -> anyhow::Result<rise_response::theme::CreateWallpaper> {
        self.call(RiseThemeEndpoints::create_wallpaper(), &request).await
    }

    pub async fn set_default_wallpaper(
        &self,
        request: rise_request::theme::SetDefaultWallpaper,
    ) -> anyhow::Result<rise_response::theme::SetDefaultWallpaper> {
        self.call(RiseThemeEndpoints::set_default_wallpaper(), &request).await
    }

    pub async fn delete_wallpaper(
        &self,
        request: rise_request::theme::DeleteWallpaper,
    ) -> anyhow::Result<rise_response::theme::DeleteWallpaper> {
        self.call(RiseThemeEndpoints::delete_wallpaper(), &request).await
    }

    pub async fn update_wallpaper(
        &self,
        request: rise_request::theme::UpdateWallpaper,
    ) -> anyhow::Result<rise_response::theme::UpdateWallpaper> {
        self.call(RiseThemeEndpoints::update_wallpaper(), &request).await
    }

    pub async fn get_wallpaper_preferences(
        &self,
    ) -> anyhow::Result<rise_response::theme::GetWallpaperPreferences> {
        self.call(
            RiseThemeEndpoints::get_wallpaper_preferences(),
            &rise_request::theme::GetWallpaperPreferences {},
        )
        .await
    }

    pub async fn set_wallpaper_preferences(
        &self,
        request: rise_request::theme::SetWallpaperPreferences,
    ) -> anyhow::Result<rise_response::theme::GetWallpaperPreferences> {
        self.call(RiseThemeEndpoints::set_wallpaper_preferences(), &request).await
    }

    pub async fn set_chat_wallpaper_preference(
        &self,
        request: rise_request::theme::SetChatWallpaperPreference,
    ) -> anyhow::Result<rise_response::theme::GetWallpaperPreferences> {
        self.call(RiseThemeEndpoints::set_chat_wallpaper_preference(), &request).await
    }

    pub async fn create_theme(
        &self,
        request: rise_request::theme::CreateTheme,
    ) -> anyhow::Result<rise_response::theme::CreateTheme> {
        self.call(RiseThemeEndpoints::create_theme(), &request).await
    }

    pub async fn get_themes(
        &self,
        request: rise_request::theme::GetThemes,
    ) -> anyhow::Result<rise_response::theme::GetThemes> {
        self.call(RiseThemeEndpoints::get_themes(), &request).await
    }

    pub async fn get_theme(
        &self,
        request: rise_request::theme::GetTheme,
    ) -> anyhow::Result<rise_response::theme::GetTheme> {
        self.call(RiseThemeEndpoints::get_theme(), &request).await
    }

    pub async fn update_theme(
        &self,
        request: rise_request::theme::UpdateTheme,
    ) -> anyhow::Result<rise_response::theme::UpdateTheme> {
        self.call(RiseThemeEndpoints::update_theme(), &request).await
    }

    pub async fn delete_theme(
        &self,
        request: rise_request::theme::DeleteTheme,
    ) -> anyhow::Result<rise_response::theme::DeleteTheme> {
        self.call(RiseThemeEndpoints::delete_theme(), &request).await
    }

    async fn call<Request, Response>(
        &self,
        endpoint: &RiseThemeEndpoint,
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
