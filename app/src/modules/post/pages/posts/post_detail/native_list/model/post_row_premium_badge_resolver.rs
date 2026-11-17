use gpui::App;

use crate::core::ui::custom_emoji_glyph::CustomEmojiGlyphMetrics;
use crate::core::ui::custom_emoji_resource_warmup::CustomEmojiResourceWarmup;
use crate::core::ui::custom_emoji_stickers_hooks::custom_emoji_stickers_hooks;
use crate::core::ui::premium_badge::{PremiumBadgeMetrics, PremiumBadgeSource};
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostRowPremiumBadge, PostRowSeedAuthor,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_layout_calculator::PostRowMetrics;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerAssetType;

/// decides the premium badge for a native row, once per prepare on main
pub enum PostRowPremiumBadgeResolver {}

impl PostRowPremiumBadgeResolver {
    /// badge presence follows the post payload and the emoji follows
    /// the badge source, so own posts show the live profile badge
    /// a ce: id resolves against the account emoji resources
    pub fn resolve(author: &PostRowSeedAuthor, cx: &mut App) -> PostRowPremiumBadge {
        if !author.is_premium {
            return PostRowPremiumBadge::None;
        }
        let emoji = PremiumBadgeSource::emoji(author.id.as_deref(), author.premium_badge_emoji.as_deref(), cx);
        let badge = PostRowPremiumBadge::payload(true, emoji.as_deref());
        let PostRowPremiumBadge::UnresolvedCustom { id } = &badge else {
            return badge;
        };
        let id = id.clone();
        let item = custom_emoji_stickers_hooks().and_then(|hooks| (hooks.custom_emoji_item)(&id, &*cx));
        let Some(item) = item else {
            let warmup = CustomEmojiResourceWarmup::shared(cx);
            return if warmup.read(cx).is_unresolved_custom_emoji(&id) {
                PostRowPremiumBadge::MissedCustom { id }
            } else {
                badge
            };
        };
        let size = CustomEmojiGlyphMetrics::ink_size(
            PostRowMetrics::BADGE_SIZE * PremiumBadgeMetrics::CUSTOM_GLYPH_CANVAS_RATIO,
            CustomEmojiGlyphMetrics::aspect(item.width, item.height, item.asset_type == StickerAssetType::Lottie)
                .min(CustomEmojiGlyphMetrics::MAXIMUM_ASPECT),
            None,
        );
        PostRowPremiumBadge::Custom {
            id,
            width: f32::from(size.width),
            height: f32::from(size.height),
        }
    }
}
