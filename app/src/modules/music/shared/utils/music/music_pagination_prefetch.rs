use gpui::{App, ElementId, IntoElement, Window};

use crate::core::ui::sai_pagination_prefetch::{
    SaiPaginationPrefetchModifier, SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions,
};
use crate::modules::music::stores::music_services::music_services::{MusicPaginationState, MusicServicesStore};

/// prefetch modifiers for music lists using the cursor and has_more the cache holds for cache_key
pub trait MusicPaginationPrefetchModifiers: IntoElement + Sized {
    #[allow(clippy::too_many_arguments)]
    fn music_pagination_prefetch(
        self,
        id: impl Into<ElementId>,
        index: usize,
        item_count: usize,
        cache_key: &str,
        enabled: bool,
        threshold: usize,
        load_page: impl Fn(&mut Window, &mut App) + 'static,
        cx: &mut App,
    ) -> SaiPaginationPrefetchModifier {
        let page = MusicServicesStore::shared(cx).update(cx, |service, cx| service.pagination_state(cache_key, cx));
        self.music_pagination_prefetch_page(id, index, item_count, cache_key, &page, enabled, threshold, load_page)
    }

    /// prefetch with the page state read once by the caller for the whole row set
    /// reading per cell cost n payload reads every render
    #[allow(clippy::too_many_arguments)]
    fn music_pagination_prefetch_page(
        self,
        id: impl Into<ElementId>,
        index: usize,
        item_count: usize,
        cache_key: &str,
        page: &MusicPaginationState,
        enabled: bool,
        threshold: usize,
        load_page: impl Fn(&mut Window, &mut App) + 'static,
    ) -> SaiPaginationPrefetchModifier {
        let options = SaiPaginationPrefetchOptions::new(
            index,
            item_count,
            cache_key.to_owned(),
            page.cursor.clone(),
            enabled && page.has_more,
        )
        .threshold(threshold);
        self.sai_pagination_prefetch(id, options, load_page)
    }

    fn music_pagination_drain_filtered_empty(
        self,
        id: impl Into<ElementId>,
        filtered_item_count: usize,
        cache_key: &str,
        enabled: bool,
        load_page: impl Fn(&mut Window, &mut App) + 'static,
        cx: &mut App,
    ) -> SaiPaginationPrefetchModifier {
        let page = MusicServicesStore::shared(cx).update(cx, |service, cx| service.pagination_state(cache_key, cx));
        let options = SaiPaginationPrefetchOptions::drain_filtered_empty(
            filtered_item_count,
            cache_key.to_owned(),
            page.cursor,
            enabled && page.has_more,
        );
        self.sai_pagination_drain_filtered_empty(id, options, load_page)
    }
}

impl<E: IntoElement> MusicPaginationPrefetchModifiers for E {}
