use gpui::App;

use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services_hooks::{
    ShortsServicesHooks, ShortsServicesPostHooks,
};

pub fn install_shorts_services_hooks() {
    ShortsServicesHooks::install_post(ShortsServicesPostHooks {
        cached_feed_page: Box::new(|key, disk_cache_policy, cx: &mut App| {
            PostServicesStore::observed_cached_feed_page(key, disk_cache_policy, cx)
        }),
        cache_own_posts: Box::new(|payload, cx: &mut App| PostServicesStore::cache_own_posts_from_payload(payload, cx)),
    });
}
