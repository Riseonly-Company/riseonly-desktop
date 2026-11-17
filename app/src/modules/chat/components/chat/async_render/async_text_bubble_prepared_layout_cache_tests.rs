use std::sync::Arc;

use gpui::{Font, TestAppContext, TextSystem, font, point, px, size};
use rise_core::foundation::foundation_date::Date;

use crate::modules::chat::components::chat::async_render::async_text_bubble_prepared_layout_cache::AsyncTextBubblePreparedLayoutCache;
use crate::modules::chat::components::chat::async_render::async_text_bubble_types::{
    AsyncTextBubbleLayoutSpec, AsyncTextBubbleLine, AsyncTextBubblePreparedLayout, AsyncTextBubbleSeed,
};

fn cache() -> AsyncTextBubblePreparedLayoutCache {
    AsyncTextBubblePreparedLayoutCache::new(64 * 1_024, 4)
}

fn seed() -> AsyncTextBubbleSeed {
    AsyncTextBubbleSeed::new("message-1", "hello", Date::from_time_interval_since_1970(1_000.0)).time_zone_identifier("UTC")
}

fn spec(maximum_width: f32) -> AsyncTextBubbleLayoutSpec {
    AsyncTextBubbleLayoutSpec {
        maximum_width: px(maximum_width),
        ..AsyncTextBubbleLayoutSpec::default()
    }
}

fn make_layout(
    seed: AsyncTextBubbleSeed,
    spec: AsyncTextBubbleLayoutSpec,
    text_system: &Arc<TextSystem>,
) -> Arc<AsyncTextBubblePreparedLayout> {
    let face: Font = font(".SystemUIFont");
    let line = AsyncTextBubbleLine::new("12:00", 0, &face, px(10.0), text_system);
    Arc::new(AsyncTextBubblePreparedLayout::new(
        seed,
        spec,
        size(px(120.0), px(40.0)),
        Vec::new().into(),
        line,
        point(px(0.0), px(0.0)),
        None,
        None,
        point(px(0.0), px(0.0)),
        None,
        Vec::new(),
    ))
}

#[gpui::test]
fn test_exact_seed_and_spec_reuse_prepared_layout(cx: &mut TestAppContext) {
    let cache = cache();
    let seed = seed();
    let spec = spec(240.0);
    let layout = make_layout(seed.clone(), spec, cx.text_system());

    cache.insert(Arc::clone(&layout));

    let cached = cache.layout(&seed, &spec);
    assert!(cached.is_some_and(|cached| Arc::ptr_eq(&cached, &layout)));
}

#[gpui::test]
fn test_changed_content_or_width_does_not_reuse_layout(cx: &mut TestAppContext) {
    let cache = cache();
    let seed = seed();
    let spec = spec(240.0);
    cache.insert(make_layout(seed.clone(), spec, cx.text_system()));

    let edited = AsyncTextBubbleSeed::new(seed.id.clone(), "edited", seed.timestamp)
        .time_zone_identifier(seed.time_zone_identifier.clone());
    assert!(cache.layout(&edited, &spec).is_none());
    assert!(cache.layout(&seed, &self::spec(320.0)).is_none());
}

#[gpui::test]
fn test_remove_all_invalidates_entries(cx: &mut TestAppContext) {
    let cache = cache();
    let seed = seed();
    let spec = spec(240.0);
    cache.insert(make_layout(seed.clone(), spec, cx.text_system()));

    cache.remove_all();

    assert!(cache.layout(&seed, &spec).is_none());
}
