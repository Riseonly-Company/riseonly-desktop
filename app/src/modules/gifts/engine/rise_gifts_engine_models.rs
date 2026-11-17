use rise_canon::rise_proto::market::{
    Appearance, CatalogGift, GetAppearancesResponse, GetCatalogItemResponse, GetCatalogResponse,
    GetGiftInstanceResponse, GiftAttribute, GiftInstance, GiftInstanceStateResponse, GiftMedia,
    GiftUser, SendGiftResponse, SetEquippedResponse, WornCosmetic,
};
use rise_canon::rise_view::market::GiftsPage;
use rise_core::foundation::foundation_url::Url;

/// gift vocabulary of canon, the rise-gifts crate on desktop generated from canon gifts.json
pub type RiseGiftItemKind = rise_gifts::ItemKind;
pub type RiseGiftMediaKind = rise_gifts::MediaKind;
pub type RiseGiftMediaSlot = rise_gifts::MediaSlot;
pub type RiseGiftAttributeKind = rise_gifts::AttributeKind;
pub type RiseGiftRarityClass = rise_gifts::RarityClass;
pub type RiseGiftInstanceSource = rise_gifts::InstanceSource;

/// wire shapes of market service through the gateway
/// the canon decoder keeps media slot names as the wire spells them so url_for reads both
/// spellings
pub type RiseGiftMediaDTO = GiftMedia;

pub trait RiseGiftMediaExt {
    fn media_kind(&self) -> Option<RiseGiftMediaKind>;

    fn url(&self, slot: RiseGiftMediaSlot) -> Option<String>;

    /// looping movie for the platform, hevc with alpha and the large one for cards
    fn movie(&self, large: bool) -> Option<Url>;

    fn poster(&self, large: bool) -> Option<String>;

    fn lottie(&self) -> Option<Url>;

    /// a cosmetic still, a frame two layers side by side with the small strip for lists a
    /// nameplate the middle of a profile effect
    fn cosmetic_poster(&self, small: bool) -> Option<String>;

    /// a cosmetic hevc with alpha
    fn cosmetic_movie(&self) -> Option<Url>;
}

impl RiseGiftMediaExt for GiftMedia {
    fn media_kind(&self) -> Option<RiseGiftMediaKind> {
        RiseGiftMediaKind::from_slug(&self.kind)
    }

    fn url(&self, slot: RiseGiftMediaSlot) -> Option<String> {
        let name = slot.slug();
        let value = self
            .slots
            .get(name)
            .or_else(|| self.slots.get(&camel_cased(name)));
        value.filter(|value| !value.is_empty()).cloned()
    }

    fn movie(&self, large: bool) -> Option<Url> {
        let primary = if large {
            RiseGiftMediaSlot::Hevc512
        } else {
            RiseGiftMediaSlot::Hevc256
        };
        let fallback = if large {
            RiseGiftMediaSlot::Hevc256
        } else {
            RiseGiftMediaSlot::Hevc512
        };
        self.url(primary)
            .or_else(|| self.url(fallback))
            .and_then(|string| Url::from_string(&string))
    }

    fn poster(&self, large: bool) -> Option<String> {
        let primary = if large {
            RiseGiftMediaSlot::Poster512
        } else {
            RiseGiftMediaSlot::Poster256
        };
        let fallback = if large {
            RiseGiftMediaSlot::Poster256
        } else {
            RiseGiftMediaSlot::Poster512
        };
        self.url(primary).or_else(|| self.url(fallback))
    }

    fn lottie(&self) -> Option<Url> {
        self.url(RiseGiftMediaSlot::Lottie)
            .and_then(|string| Url::from_string(&string))
    }

    fn cosmetic_poster(&self, small: bool) -> Option<String> {
        match self.media_kind() {
            Some(RiseGiftMediaKind::FrameVideo) => {
                if small {
                    self.url(RiseGiftMediaSlot::FramePosterSmall)
                        .or_else(|| self.url(RiseGiftMediaSlot::FramePoster))
                } else {
                    self.url(RiseGiftMediaSlot::FramePoster)
                        .or_else(|| self.url(RiseGiftMediaSlot::FramePosterSmall))
                }
            }
            Some(RiseGiftMediaKind::PlateVideo) => self.url(RiseGiftMediaSlot::PlatePoster),
            Some(RiseGiftMediaKind::BannerVideo) => self.url(RiseGiftMediaSlot::BannerPoster),
            _ => None,
        }
    }

    fn cosmetic_movie(&self) -> Option<Url> {
        let slot = match self.media_kind() {
            Some(RiseGiftMediaKind::FrameVideo) => RiseGiftMediaSlot::FrameHevc,
            Some(RiseGiftMediaKind::PlateVideo) => RiseGiftMediaSlot::PlateHevc,
            Some(RiseGiftMediaKind::BannerVideo) => RiseGiftMediaSlot::BannerHevc,
            _ => return None,
        };
        self.url(slot).and_then(|string| Url::from_string(&string))
    }
}

fn camel_cased(name: &str) -> String {
    let parts: Vec<&str> = name.split('_').filter(|part| !part.is_empty()).collect();
    let Some((head, tail)) = parts.split_first() else {
        return name.to_string();
    };
    if tail.is_empty() {
        return name.to_string();
    }
    let mut camel = head.to_lowercase();
    for part in tail {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            camel.extend(first.to_uppercase());
            camel.push_str(&chars.as_str().to_lowercase());
        }
    }
    camel
}

pub type RiseGiftAttributeDTO = GiftAttribute;

pub trait RiseGiftAttributeExt {
    fn attribute_kind(&self) -> Option<RiseGiftAttributeKind>;

    /// a variation without a name of its own shows its key
    fn display_name(&self) -> String;

    /// chance of this variation within its kind, 0 to 1
    fn share(&self) -> f64;

    /// rarity tier the admin named for this variation, a label only since the chance is the weight
    fn tier(&self) -> Option<RiseGiftRarityClass>;
}

impl RiseGiftAttributeExt for GiftAttribute {
    fn attribute_kind(&self) -> Option<RiseGiftAttributeKind> {
        RiseGiftAttributeKind::from_slug(&self.kind)
    }

    fn display_name(&self) -> String {
        if self.name.is_empty() {
            self.key.clone()
        } else {
            self.name.clone()
        }
    }

    fn share(&self) -> f64 {
        self.weight_permille as f64 / f64::from(rise_gifts::WEIGHT_TOTAL_PERMILLE)
    }

    fn tier(&self) -> Option<RiseGiftRarityClass> {
        RiseGiftRarityClass::from_slug(&self.rarity_tier)
    }
}

pub type RiseGiftCatalogItemDTO = CatalogGift;

pub trait RiseGiftCatalogItemExt {
    fn is_limited(&self) -> bool;

    fn is_sold_out(&self) -> bool;

    /// empty on a gift listed before kinds existed, that means an item
    fn kind(&self) -> RiseGiftItemKind;
}

impl RiseGiftCatalogItemExt for CatalogGift {
    fn is_limited(&self) -> bool {
        self.supply_total > 0
    }

    fn is_sold_out(&self) -> bool {
        self.sold_out
    }

    fn kind(&self) -> RiseGiftItemKind {
        RiseGiftItemKind::from_slug(&self.item_kind).unwrap_or(RiseGiftItemKind::Item)
    }
}

pub type RiseGiftCatalogResponseDTO = GetCatalogResponse;
pub type RiseGiftCatalogItemResponseDTO = GetCatalogItemResponse;

pub trait RiseGiftCatalogItemResponseExt {
    fn likeliest(&self, kind: RiseGiftAttributeKind) -> Option<&RiseGiftAttributeDTO>;
}

impl RiseGiftCatalogItemResponseExt for GetCatalogItemResponse {
    fn likeliest(&self, kind: RiseGiftAttributeKind) -> Option<&RiseGiftAttributeDTO> {
        self.attributes
            .iter()
            .rev()
            .filter(|attribute| {
                attribute.attribute_kind() == Some(kind) && attribute.weight_permille > 0
            })
            .max_by_key(|attribute| attribute.weight_permille)
    }
}

pub type RiseGiftUserDTO = GiftUser;
pub type RiseGiftInstanceDTO = GiftInstance;

pub trait RiseGiftInstanceExt {
    fn model(&self) -> Option<&RiseGiftAttributeDTO>;
    fn backdrop(&self) -> Option<&RiseGiftAttributeDTO>;
    fn pattern(&self) -> Option<&RiseGiftAttributeDTO>;
    fn frame(&self) -> Option<&RiseGiftAttributeDTO>;
    fn effect(&self) -> Option<&RiseGiftAttributeDTO>;

    /// numbered within a limited edition as the ribbon and title print it
    fn is_numbered(&self) -> bool;

    fn rarity_class(&self) -> RiseGiftRarityClass;

    /// empty on an instance issued before kinds existed, that means an item
    fn kind(&self) -> RiseGiftItemKind;

    fn is_cosmetic(&self) -> bool;
    fn is_grant(&self) -> bool;
    fn is_displayed(&self) -> bool;
    fn is_pinned(&self) -> bool;
    fn is_equipped(&self) -> bool;

    fn is_gifted_by_other(&self) -> bool;

    fn with_displayed(&self, displayed: bool, pinned: bool, pinned_at_ms: i64) -> RiseGiftInstanceDTO;

    fn with_equipped(&self, equipped: bool) -> RiseGiftInstanceDTO;
}

fn instance_attribute(
    instance: &GiftInstance,
    kind: RiseGiftAttributeKind,
) -> Option<&RiseGiftAttributeDTO> {
    instance
        .attributes
        .iter()
        .find(|attribute| attribute.attribute_kind() == Some(kind))
}

impl RiseGiftInstanceExt for GiftInstance {
    fn model(&self) -> Option<&RiseGiftAttributeDTO> {
        instance_attribute(self, RiseGiftAttributeKind::Model)
    }

    fn backdrop(&self) -> Option<&RiseGiftAttributeDTO> {
        instance_attribute(self, RiseGiftAttributeKind::Backdrop)
    }

    fn pattern(&self) -> Option<&RiseGiftAttributeDTO> {
        instance_attribute(self, RiseGiftAttributeKind::Pattern)
    }

    fn frame(&self) -> Option<&RiseGiftAttributeDTO> {
        instance_attribute(self, RiseGiftAttributeKind::Frame)
    }

    fn effect(&self) -> Option<&RiseGiftAttributeDTO> {
        instance_attribute(self, RiseGiftAttributeKind::Effect)
    }

    fn is_numbered(&self) -> bool {
        self.supply_total > 0
    }

    fn rarity_class(&self) -> RiseGiftRarityClass {
        RiseGiftRarityClass::from_slug(&self.rarity).unwrap_or(RiseGiftRarityClass::Common)
    }

    fn kind(&self) -> RiseGiftItemKind {
        RiseGiftItemKind::from_slug(&self.item_kind).unwrap_or(RiseGiftItemKind::Item)
    }

    fn is_cosmetic(&self) -> bool {
        self.kind().equippable()
    }

    fn is_grant(&self) -> bool {
        self.source == RiseGiftInstanceSource::AdminGrant.slug()
    }

    fn is_displayed(&self) -> bool {
        self.displayed
    }

    fn is_pinned(&self) -> bool {
        self.pinned
    }

    fn is_equipped(&self) -> bool {
        self.equipped
    }

    fn is_gifted_by_other(&self) -> bool {
        if self.is_grant() {
            return true;
        }
        let Some(sender) = &self.sender else {
            return self.anonymous;
        };
        sender.id != self.owner_user_id
    }

    fn with_displayed(&self, displayed: bool, pinned: bool, pinned_at_ms: i64) -> RiseGiftInstanceDTO {
        let mut copy = self.clone();
        copy.displayed = displayed;
        copy.pinned = pinned;
        copy.pinned_at_ms = pinned_at_ms;
        copy
    }

    fn with_equipped(&self, equipped: bool) -> RiseGiftInstanceDTO {
        let mut copy = self.clone();
        copy.equipped = equipped;
        copy
    }
}

pub type RiseGiftTabResponseDTO = GiftsPage;
pub type RiseGiftInstanceResponseDTO = GetGiftInstanceResponse;
pub type RiseGiftSendResponseDTO = SendGiftResponse;
pub type RiseGiftStateResponseDTO = GiftInstanceStateResponse;

/// set_equipped answer, the slot is the cosmetic kind
pub type RiseGiftEquipResponseDTO = SetEquippedResponse;

/// one worn cosmetic, the variations its roll landed on with names in the gift base language
pub type RiseGiftWornDTO = WornCosmetic;

pub trait RiseGiftWornExt {
    fn kind(&self) -> Option<RiseGiftItemKind>;

    /// own instance as its slot shows it before get_appearances confirms it
    fn new(instance: &RiseGiftInstanceDTO) -> Self;
}

impl RiseGiftWornExt for WornCosmetic {
    fn kind(&self) -> Option<RiseGiftItemKind> {
        RiseGiftItemKind::from_slug(&self.item_kind)
    }

    fn new(instance: &RiseGiftInstanceDTO) -> Self {
        WornCosmetic {
            effect: instance.effect().cloned(),
            gift_id: instance.gift_id,
            instance_id: instance.id.clone(),
            item_kind: instance.kind().slug().to_string(),
            model: instance.model().cloned(),
        }
    }
}

/// what one person wears, only people who wear something are listed
pub type RiseGiftAppearanceDTO = Appearance;

pub trait RiseGiftAppearanceExt {
    /// someone asked about and not listed wears nothing
    fn nothing(user_id: impl Into<String>) -> RiseGiftAppearanceDTO;

    fn wears_nothing(&self) -> bool;

    fn worn(&self, kind: RiseGiftItemKind) -> Option<&RiseGiftWornDTO>;

    fn replacing(&self, kind: RiseGiftItemKind, worn: Option<RiseGiftWornDTO>) -> RiseGiftAppearanceDTO;
}

impl RiseGiftAppearanceExt for Appearance {
    fn nothing(user_id: impl Into<String>) -> RiseGiftAppearanceDTO {
        Appearance {
            user_id: user_id.into(),
            ..Appearance::default()
        }
    }

    fn wears_nothing(&self) -> bool {
        self.avatar_frame.is_none() && self.nameplate.is_none() && self.profile_effect.is_none()
    }

    fn worn(&self, kind: RiseGiftItemKind) -> Option<&RiseGiftWornDTO> {
        match kind {
            RiseGiftItemKind::AvatarFrame => self.avatar_frame.as_ref(),
            RiseGiftItemKind::Nameplate => self.nameplate.as_ref(),
            RiseGiftItemKind::ProfileEffect => self.profile_effect.as_ref(),
            RiseGiftItemKind::Item => None,
        }
    }

    fn replacing(&self, kind: RiseGiftItemKind, worn: Option<RiseGiftWornDTO>) -> RiseGiftAppearanceDTO {
        let mut copy = self.clone();
        match kind {
            RiseGiftItemKind::AvatarFrame => copy.avatar_frame = worn,
            RiseGiftItemKind::Nameplate => copy.nameplate = worn,
            RiseGiftItemKind::ProfileEffect => copy.profile_effect = worn,
            RiseGiftItemKind::Item => {}
        }
        copy
    }
}

pub type RiseGiftAppearancesResponseDTO = GetAppearancesResponse;
