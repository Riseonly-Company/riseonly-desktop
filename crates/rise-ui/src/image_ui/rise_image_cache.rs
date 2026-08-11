use std::collections::HashMap;
use std::sync::Arc;

use futures::FutureExt;
use gpui::{
    App, AppContext, Asset, Entity, Global, ImageCache, ImageCacheError, ImageCacheItem,
    ImgResourceLoader, RenderImage, Resource, Window, hash,
};

use super::image_budget::{ImageBudget, ImageLimits};

/// A bounded image cache.
///
/// gpui's own `ImageCache` never evicts; this is the same load path with an LRU
/// over it, holding to the two ceilings in [`ImageLimits`].
pub struct RiseImageCache {
    items: HashMap<u64, ImageCacheItem>,
    budget: ImageBudget,
}

/// The one cache the whole application shares. A screen that wants its own
/// working set builds one with [`RiseImageCache::new`].
pub struct SharedImageCache(Entity<RiseImageCache>);

impl Global for SharedImageCache {}

impl RiseImageCache {
    pub fn new(limits: ImageLimits, cx: &mut App) -> Entity<Self> {
        let entity = cx.new(|_| Self {
            items: HashMap::new(),
            budget: ImageBudget::new(limits),
        });

        // gpui keeps the textures in its own atlas, so dropping the map without `drop_image` leaks every one.
        cx.observe_release(&entity, |cache, cx| {
            for (_, mut item) in std::mem::take(&mut cache.items) {
                if let Some(Ok(image)) = item.get() {
                    cx.drop_image(image, None);
                }
            }
            cache.budget.drain();
        })
        .detach();

        entity
    }

    /// Installs the shared cache. Call once, during app setup.
    pub fn install(limits: ImageLimits, cx: &mut App) {
        let entity = Self::new(limits, cx);
        cx.set_global(SharedImageCache(entity));
    }

    /// The shared cache, or `None` when [`install`](Self::install) has not run —
    /// an image element then falls back to gpui's own unbounded cache.
    pub fn shared(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<SharedImageCache>()
            .map(|cache| cache.0.clone())
    }

    pub fn len(&self) -> usize {
        self.budget.len()
    }

    pub fn is_empty(&self) -> bool {
        self.budget.is_empty()
    }

    pub fn bytes(&self) -> u64 {
        self.budget.bytes()
    }

    /// What one decoded image costs to hold: four bytes a pixel, every frame, so
    /// an animated source costs one buffer per frame.
    pub fn cost_of(image: &RenderImage) -> u64 {
        (0..image.frame_count())
            .map(|index| {
                let size = image.size(index);
                u64::from(size.width.0.unsigned_abs()) * u64::from(size.height.0.unsigned_abs()) * 4
            })
            .sum()
    }

    fn evict(&mut self, keys: Vec<u64>, window: &mut Window, cx: &mut App) {
        for key in keys {
            if let Some(mut item) = self.items.remove(&key)
                && let Some(Ok(image)) = item.get()
            {
                cx.drop_image(image, Some(window));
            }
        }
    }
}

impl ImageCache for RiseImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let key = hash(resource);

        if let Some(item) = self.items.get_mut(&key) {
            let was_loading = matches!(item, ImageCacheItem::Loading(_));
            let resolved = item.get();

            match (&resolved, was_loading) {
                // The frame a decode lands on is the only moment the size is known.
                (Some(Ok(image)), true) => {
                    let cost = Self::cost_of(image);
                    let evicted = self.budget.resolve(key, cost);
                    self.evict(evicted, window, cx);
                }
                (Some(Err(_)), true) => {
                    let evicted = self.budget.resolve(key, 0);
                    self.evict(evicted, window, cx);
                }
                _ => {
                    self.budget.touch(key);
                }
            }

            return resolved;
        }

        let evicted = self.budget.insert_loading(key);
        self.evict(evicted, window, cx);

        let load = ImgResourceLoader::load(resource.clone(), cx);
        let task = cx.background_executor().spawn(load).shared();
        self.items
            .insert(key, ImageCacheItem::Loading(task.clone()));

        // Without this redraw a still feed never invalidates, so the bytes arrive but nothing paints them.
        let view = window.current_view();
        window
            .spawn(cx, async move |cx| {
                let _ = task.await;
                cx.on_next_frame(move |_, cx| cx.notify(view));
            })
            .detach();

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_limits_are_bounded_on_both_axes() {
        const { assert!(ImageLimits::FEED.max_entries > 0) };
        const { assert!(ImageLimits::FEED.max_bytes > 0) };
    }

    #[gpui::test]
    fn a_cache_with_nothing_in_it_reports_nothing(cx: &mut gpui::TestAppContext) {
        let cache = cx.update(|cx| RiseImageCache::new(ImageLimits::FEED, cx));
        cx.update(|cx| {
            let cache = cache.read(cx);
            assert!(cache.is_empty());
            assert_eq!(cache.bytes(), 0);
        });
    }

    #[gpui::test]
    fn the_shared_cache_is_absent_until_it_is_installed(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            assert!(RiseImageCache::shared(cx).is_none());
            RiseImageCache::install(ImageLimits::FEED, cx);
            assert!(RiseImageCache::shared(cx).is_some());
        });
    }

    #[gpui::test]
    fn every_screen_that_asks_gets_the_same_cache(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            RiseImageCache::install(ImageLimits::FEED, cx);

            let first = RiseImageCache::shared(cx).unwrap();
            let second = RiseImageCache::shared(cx).unwrap();
            assert_eq!(
                first.entity_id(),
                second.entity_id(),
                "an avatar drawn in the feed and in the comments panel must decode once"
            );
        });
    }
}
