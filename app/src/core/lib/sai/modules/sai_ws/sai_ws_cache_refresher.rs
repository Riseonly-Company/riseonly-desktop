use std::rc::Rc;

use gpui::App;

use crate::core::desktop_platform::rise_app_events::RiseNotificationName;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{
    SaiActiveScreen, SaiWsScreenRefreshParams,
};

/// gives the cache keys to refresh for the active screen
pub type SaiWsCacheRefresherRuleRefreshCacheKeys = Rc<dyn Fn(&SaiActiveScreen, &mut App) -> Vec<String>>;

/// called after a refresh with the screen and the keys
pub type SaiWsCacheRefresherRuleOnRefresh = Rc<dyn Fn(&SaiActiveScreen, &[String], &mut App)>;

#[derive(Clone)]
pub struct SaiWsCacheRefresherRule {
    pub when_on_screen: Vec<String>,

    pub route_params: Option<SaiWsScreenRefreshParams>,

    pub refresh_cache_keys: SaiWsCacheRefresherRuleRefreshCacheKeys,

    pub refresh_cache_layers: Vec<SaiCacheCleanLayer>,

    pub on_refresh: Option<SaiWsCacheRefresherRuleOnRefresh>,
}

impl SaiWsCacheRefresherRule {
    pub fn new(
        when_on_screen: Vec<String>,
        route_params: Option<SaiWsScreenRefreshParams>,
        refresh_cache_keys: SaiWsCacheRefresherRuleRefreshCacheKeys,
        refresh_cache_layers: Vec<SaiCacheCleanLayer>,
        on_refresh: Option<SaiWsCacheRefresherRuleOnRefresh>,
    ) -> Self {
        Self {
            when_on_screen,
            route_params,
            refresh_cache_keys,
            refresh_cache_layers,
            on_refresh,
        }
    }

    pub fn new_when_on_screen(
        when_on_screen: &str,
        route_params: Option<SaiWsScreenRefreshParams>,
        refresh_cache_keys: SaiWsCacheRefresherRuleRefreshCacheKeys,
        refresh_cache_layers: Vec<SaiCacheCleanLayer>,
        on_refresh: Option<SaiWsCacheRefresherRuleOnRefresh>,
    ) -> Self {
        Self::new(
            vec![when_on_screen.to_owned()],
            route_params,
            refresh_cache_keys,
            refresh_cache_layers,
            on_refresh,
        )
    }

    pub fn with_defaults(
        when_on_screen: Vec<String>,
        refresh_cache_keys: SaiWsCacheRefresherRuleRefreshCacheKeys,
    ) -> Self {
        Self::new(
            when_on_screen,
            None,
            refresh_cache_keys,
            vec![SaiCacheCleanLayer::Everywhere],
            None,
        )
    }
}

pub const SAI_WS_CACHE_REFRESHER_DID_INVALIDATE: RiseNotificationName =
    RiseNotificationName::new("saiWsCacheRefresherDidInvalidate");

/// payload of the cache invalidate event, cache keys and trigger
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaiWsCacheRefresherDidInvalidateUserInfo {
    pub cache_keys: Vec<String>,
    pub trigger: String,
}
