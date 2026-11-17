use crate::modules::gifts::engine::rise_gifts_engine_models::RiseGiftItemKind;

/// the market four shelves as the app names them, items and frames and banners which are
/// nameplates in canon and effects
pub trait GiftMarketKinds: Sized {
    const SHOP_ORDER: [Self; 4];

    fn cosmetics() -> Vec<Self>;

    fn shop_index(self) -> usize;

    fn shop_tab_key(self) -> &'static str;

    fn shop_description_key(self) -> Option<&'static str>;

    /// columns of the shelf, a nameplate row and a banner need the width of two
    fn shop_columns(self) -> usize;

    fn gifted_offer_key(self) -> Option<&'static str>;

    fn bought_offer_key(self) -> Option<&'static str>;

    fn appearance_title_key(self) -> Option<&'static str>;
}

impl GiftMarketKinds for RiseGiftItemKind {
    const SHOP_ORDER: [Self; 4] = [
        RiseGiftItemKind::Item,
        RiseGiftItemKind::AvatarFrame,
        RiseGiftItemKind::Nameplate,
        RiseGiftItemKind::ProfileEffect,
    ];

    fn cosmetics() -> Vec<Self> {
        RiseGiftItemKind::ALL
            .into_iter()
            .filter(|kind| kind.equippable())
            .collect()
    }

    fn shop_index(self) -> usize {
        Self::SHOP_ORDER
            .iter()
            .position(|kind| *kind == self)
            .unwrap_or(0)
    }

    fn shop_tab_key(self) -> &'static str {
        match self {
            RiseGiftItemKind::Item => "gift_market_tab_items",
            RiseGiftItemKind::AvatarFrame => "gift_market_tab_frames",
            RiseGiftItemKind::Nameplate => "gift_market_tab_banners",
            RiseGiftItemKind::ProfileEffect => "gift_market_tab_effects",
        }
    }

    fn shop_description_key(self) -> Option<&'static str> {
        match self {
            RiseGiftItemKind::Item => None,
            RiseGiftItemKind::AvatarFrame => Some("gift_market_desc_frames"),
            RiseGiftItemKind::Nameplate => Some("gift_market_desc_banners"),
            RiseGiftItemKind::ProfileEffect => Some("gift_market_desc_effects"),
        }
    }

    fn shop_columns(self) -> usize {
        match self {
            RiseGiftItemKind::Item | RiseGiftItemKind::AvatarFrame => 3,
            RiseGiftItemKind::Nameplate | RiseGiftItemKind::ProfileEffect => 2,
        }
    }

    fn gifted_offer_key(self) -> Option<&'static str> {
        match self {
            RiseGiftItemKind::Item => None,
            RiseGiftItemKind::AvatarFrame => Some("gift_equip_offer_frame"),
            RiseGiftItemKind::Nameplate => Some("gift_equip_offer_nameplate"),
            RiseGiftItemKind::ProfileEffect => Some("gift_equip_offer_effect"),
        }
    }

    fn bought_offer_key(self) -> Option<&'static str> {
        match self {
            RiseGiftItemKind::Item => None,
            RiseGiftItemKind::AvatarFrame => Some("gift_equip_offer_self_frame"),
            RiseGiftItemKind::Nameplate => Some("gift_equip_offer_self_nameplate"),
            RiseGiftItemKind::ProfileEffect => Some("gift_equip_offer_self_effect"),
        }
    }

    fn appearance_title_key(self) -> Option<&'static str> {
        match self {
            RiseGiftItemKind::Item => None,
            RiseGiftItemKind::AvatarFrame => Some("customization_appearance_frame"),
            RiseGiftItemKind::Nameplate => Some("customization_appearance_nameplate"),
            RiseGiftItemKind::ProfileEffect => Some("customization_appearance_effect"),
        }
    }
}
