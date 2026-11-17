use super::RiseStickerPerformancePolicy;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    PackKind, StickerAssetType, StickerPack,
};

struct RiseStickerPerformancePolicyTests;

impl RiseStickerPerformancePolicyTests {
    fn pack(id: &str) -> StickerPack {
        StickerPack::new(
            id.to_owned(),
            id.to_owned(),
            id.to_owned(),
            false,
            false,
            0,
            0,
            false,
            true,
            PackKind::Sticker,
            String::new(),
            StickerAssetType::StaticImage,
            false,
            0,
            None,
        )
    }
}

#[test]
fn test_initial_prefetch_prioritizes_saved_packs_and_deduplicates() {
    let saved = [
        RiseStickerPerformancePolicyTests::pack("saved-1"),
        RiseStickerPerformancePolicyTests::pack("shared"),
    ];
    let defaults = [
        RiseStickerPerformancePolicyTests::pack("shared"),
        RiseStickerPerformancePolicyTests::pack("default-1"),
    ];

    assert_eq!(
        RiseStickerPerformancePolicy::initial_pack_ids(&saved, &defaults),
        vec!["saved-1".to_owned(), "shared".to_owned()]
    );
}

#[test]
fn test_every_picker_collection_has_a_bounded_budget() {
    assert!(RiseStickerPerformancePolicy::favourites_limit(false) > 0);
    assert!(RiseStickerPerformancePolicy::recents_limit() > 0);
    assert!(RiseStickerPerformancePolicy::packs_limit() >= RiseStickerPerformancePolicy::packs_page_limit());
    assert!(RiseStickerPerformancePolicy::stickers_per_pack_limit() > 0);
    assert!(RiseStickerPerformancePolicy::gif_limit() >= RiseStickerPerformancePolicy::gif_page_limit());
    assert!(RiseStickerPerformancePolicy::search_cache_limit() > 0);
    assert!(RiseStickerPerformancePolicy::search_items_limit() > 0);
    assert!(
        RiseStickerPerformancePolicy::persisted_pack_limit()
            >= RiseStickerPerformancePolicy::initial_pack_prefetch_count()
    );
}
