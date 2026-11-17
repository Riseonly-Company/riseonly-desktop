use std::any::Any;

use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsDiskCachePolicy;
use crate::modules::post::stores::post_services::post_services::PostServicesStoreCachedFeedPage;

pub struct ShortsServicesPostHooks {
    pub cached_feed_page:
        Box<dyn Fn(&str, SaiWsDiskCachePolicy, &mut App) -> Option<PostServicesStoreCachedFeedPage> + Send + Sync>,
    pub cache_own_posts: Box<dyn Fn(Option<&dyn Any>, &mut App) + Send + Sync>,
}

pub(crate) static SHORTS_SERVICES_POST_HOOKS: FeatureHookSlot<ShortsServicesPostHooks> = FeatureHookSlot::new("SHORTS_SERVICES_POST_HOOKS");

pub enum ShortsServicesHooks {}

impl ShortsServicesHooks {
    pub fn install_post(hooks: ShortsServicesPostHooks) {
        let _ = SHORTS_SERVICES_POST_HOOKS.set(hooks);
    }

    pub fn post() -> Option<&'static ShortsServicesPostHooks> {
        SHORTS_SERVICES_POST_HOOKS.get()
    }

    pub fn cached_feed_page(
        key: &str,
        disk_cache_policy: SaiWsDiskCachePolicy,
        cx: &mut App,
    ) -> Option<PostServicesStoreCachedFeedPage> {
        let hooks = Self::post()?;
        (hooks.cached_feed_page)(key, disk_cache_policy, cx)
    }

    pub fn cache_own_posts(payload: Option<&dyn Any>, cx: &mut App) {
        if let Some(hooks) = Self::post() {
            (hooks.cache_own_posts)(payload, cx);
        }
    }
}
