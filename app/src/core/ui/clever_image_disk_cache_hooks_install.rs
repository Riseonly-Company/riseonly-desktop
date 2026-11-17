use crate::core::lib::sai::sai_image_disk_cache_hooks::{SaiImageDiskCacheHooks, set_sai_image_disk_cache_hooks};
use crate::core::ui::clever_image_disk_cache::{CleverImageDiskCache, CleverImageDiskPrefetch};

pub(crate) fn install_clever_image_disk_cache_hooks() {
    set_sai_image_disk_cache_hooks(SaiImageDiskCacheHooks {
        invalidation_token: Box::new(CleverImageDiskCache::invalidation_token),
        store: Box::new(|data, cache_key, user_id, expected_invalidation_token| {
            CleverImageDiskCache::store_data(&data, cache_key, user_id, expected_invalidation_token);
        }),
        schedule_prefetch: Box::new(CleverImageDiskPrefetch::schedule_values_from),
    });
}
