use rise_canon::rise_key::stickers as key;
use rise_canon::{rise_proto, rise_view};
use serde_json::{Map, Value};

use super::stickers_types::{
    GifItem, PackKind, StickerAssetType, StickerAssetTypeInfer, StickerItem, StickerPack, StickerPackRights,
};

pub type PackWireDTO = rise_proto::stickers::ProtoPackRecord;

pub type StickerWireDTO = rise_proto::stickers::ProtoStickerRecord;

pub type GifWireDTO = rise_proto::stickers::ProtoGifItem;
pub type GetPacksGatewayDTO = rise_view::stickers::DefaultPacks;

pub type GetSavedPacksGatewayDTO = rise_view::stickers::SavedPacks;
pub type GetMyPacksGatewayDTO = rise_view::stickers::MyPacks;

pub type GetStickersByPackIdGatewayDTO = rise_proto::stickers::ProtoGetStickersByPackIdResponse;
pub type GetCustomEmojiByIdsGatewayDTO = rise_proto::stickers::ProtoGetCustomEmojiByIdsResponse;

pub type GetStickersByPackLinkGatewayDTO = rise_proto::stickers::ProtoGetStickersByPackLinkResponse;

pub type GetGifFavouritesGatewayDTO = rise_view::stickers::GifFavourites;

pub type SearchGifsGatewayDTO = rise_view::stickers::FoundGifs;

pub type AddGifFavouriteGatewayDTO = rise_proto::stickers::ProtoAddGifFavouriteResponse;

pub trait PackWireDTOExt {
    fn to_pack(&self) -> StickerPack;
}

impl PackWireDTOExt for PackWireDTO {
    fn to_pack(&self) -> StickerPack {
        StickerPack::new(
            self.id.clone(),
            self.title.clone(),
            self.link.clone(),
            self.is_admin,
            self.is_creator,
            self.created_at_ms,
            self.updated_at_ms,
            self.is_default,
            self.is_saved,
            PackKind::from_wire(&self.pack_kind.to_lowercase()).unwrap_or(PackKind::Sticker),
            self.preview_url.clone(),
            StickerAssetType::infer(Some(&self.preview_asset_type), &self.preview_url),
            self.is_premium,
            self.unique_saves,
            self.my_rights.as_ref().map(|rights| StickerPackRights {
                can_add_stickers: rights.can_add_stickers,
                can_delete_stickers: rights.can_delete_stickers,
                can_edit_pack: rights.can_edit_pack,
                can_edit_stickers: rights.can_edit_stickers,
            }),
        )
    }
}

pub trait StickerWireDTOExt {
    fn to_sticker(&self) -> StickerItem;
}

impl StickerWireDTOExt for StickerWireDTO {
    fn to_sticker(&self) -> StickerItem {
        let url = self.file_url.clone();
        StickerItem::new(
            self.id.clone(),
            self.pack_id.clone(),
            url.clone(),
            self.thumbnail_url.clone(),
            self.width,
            self.height,
            self.duration_ms,
            self.position,
            self.created_at_ms,
            self.associated_emojis.clone(),
            StickerAssetType::infer(Some(&self.asset_type), &url),
            self.mime_type.clone(),
            self.codec.clone(),
        )
    }
}

pub trait GifWireDTOExt {
    fn to_gif(&self) -> GifItem;
}

impl GifWireDTOExt for GifWireDTO {
    fn to_gif(&self) -> GifItem {
        GifItem {
            id: self.id.clone(),
            file_url: self.file_url.clone(),
            preview_url: self.preview_url.clone(),
            width: self.width,
            height: self.height,
            duration_ms: self.duration_ms,
            source: self.source.clone(),
            source_id: self.source_id.clone(),
            added_at_ms: self.added_at_ms,
        }
    }
}

fn optional_text(value: &Option<String>) -> Value {
    match value {
        Some(text) => Value::from(text.clone()),
        None => Value::Null,
    }
}

impl GifItem {
    pub fn to_raw_row(&self) -> Map<String, Value> {
        let mut row = Map::new();
        row.insert(key::ID.to_owned(), Value::from(self.id.clone()));
        row.insert(key::FILE_URL.to_owned(), Value::from(self.file_url.clone()));
        row.insert(key::PREVIEW_URL.to_owned(), Value::from(self.preview_url.clone()));
        row.insert(key::WIDTH.to_owned(), Value::from(self.width));
        row.insert(key::HEIGHT.to_owned(), Value::from(self.height));
        row.insert(key::DURATION_MS.to_owned(), Value::from(self.duration_ms));
        row.insert(key::SOURCE.to_owned(), Value::from(self.source.clone()));
        row.insert(key::SOURCE_ID.to_owned(), Value::from(self.source_id.clone()));
        row.insert(key::ADDED_AT_MS.to_owned(), Value::from(self.added_at_ms));
        row
    }

    pub fn to_wire_dto(&self) -> GifWireDTO {
        GifWireDTO {
            added_at_ms: self.added_at_ms,
            duration_ms: self.duration_ms,
            file_url: self.file_url.clone(),
            height: self.height,
            id: self.id.clone(),
            preview_url: self.preview_url.clone(),
            source: self.source.clone(),
            source_id: self.source_id.clone(),
            width: self.width,
        }
    }
}

impl StickerPack {
    pub fn to_wire_dto(&self) -> PackWireDTO {
        PackWireDTO {
            coauthor_count: 0,
            created_at_ms: self.created_at_ms,
            id: self.id.clone(),
            is_admin: self.is_admin,
            is_creator: self.is_creator,
            is_default: self.is_default,
            is_premium: self.is_premium,
            is_saved: self.is_saved,
            link: self.link.clone(),
            my_rights: self.my_rights.map(|rights| rise_proto::stickers::ProtoPackRights {
                can_add_stickers: rights.can_add_stickers,
                can_delete_stickers: rights.can_delete_stickers,
                can_edit_pack: rights.can_edit_pack,
                can_edit_stickers: rights.can_edit_stickers,
            }),
            pack_kind: self.pack_kind.wire().to_owned(),
            preview_asset_type: self.preview_asset_type.wire().to_owned(),
            preview_url: self.preview_url.clone(),
            title: self.title.clone(),
            unique_saves: self.unique_saves,
            updated_at_ms: self.updated_at_ms,
        }
    }

    pub fn to_raw_row(&self) -> Map<String, Value> {
        let mut row = Map::new();
        row.insert(key::ID.to_owned(), Value::from(self.id.clone()));
        row.insert(key::TITLE.to_owned(), Value::from(self.title.clone()));
        row.insert(key::LINK.to_owned(), Value::from(self.link.clone()));
        row.insert(key::IS_ADMIN.to_owned(), Value::from(self.is_admin));
        row.insert(key::IS_CREATOR.to_owned(), Value::from(self.is_creator));
        row.insert(key::CREATED_AT_MS.to_owned(), Value::from(self.created_at_ms));
        row.insert(key::UPDATED_AT_MS.to_owned(), Value::from(self.updated_at_ms));
        row.insert(key::IS_DEFAULT.to_owned(), Value::from(self.is_default));
        row.insert(key::IS_SAVED.to_owned(), Value::from(self.is_saved));
        row.insert(key::PACK_KIND.to_owned(), Value::from(self.pack_kind.wire()));
        row.insert(key::PREVIEW_URL.to_owned(), Value::from(self.preview_url.clone()));
        row.insert(key::PREVIEW_ASSET_TYPE.to_owned(), Value::from(self.preview_asset_type.wire()));
        row.insert(key::IS_PREMIUM.to_owned(), Value::from(self.is_premium));
        row.insert(key::UNIQUE_SAVES.to_owned(), Value::from(self.unique_saves));
        row
    }
}

impl StickerItem {
    pub fn to_wire_dto(&self) -> StickerWireDTO {
        StickerWireDTO {
            asset_type: self.asset_type.wire().to_owned(),
            associated_emojis: self.associated_emojis.clone(),
            codec: self.codec.clone(),
            created_at_ms: self.created_at_ms,
            duration_ms: self.duration_ms,
            file_url: self.file_url.clone(),
            height: self.height,
            id: self.id.clone(),
            mime_type: self.mime_type.clone(),
            pack_id: self.pack_id.clone(),
            position: self.position,
            thumbnail_url: self.thumbnail_url.clone(),
            width: self.width,
            ..StickerWireDTO::default()
        }
    }

    pub fn to_raw_row(&self) -> Map<String, Value> {
        let mut row = Map::new();
        row.insert(key::ID.to_owned(), Value::from(self.id.clone()));
        row.insert(key::PACK_ID.to_owned(), Value::from(self.pack_id.clone()));
        row.insert(key::FILE_URL.to_owned(), Value::from(self.file_url.clone()));
        row.insert(key::THUMBNAIL_URL.to_owned(), optional_text(&self.thumbnail_url));
        row.insert(key::WIDTH.to_owned(), Value::from(self.width));
        row.insert(key::HEIGHT.to_owned(), Value::from(self.height));
        row.insert(
            key::DURATION_MS.to_owned(),
            self.duration_ms.map(Value::from).unwrap_or(Value::Null),
        );
        row.insert(key::POSITION.to_owned(), Value::from(self.position));
        row.insert(key::CREATED_AT_MS.to_owned(), Value::from(self.created_at_ms));
        row.insert(key::ASSOCIATED_EMOJIS.to_owned(), Value::from(self.associated_emojis.clone()));
        row.insert(key::ASSET_TYPE.to_owned(), Value::from(self.asset_type.wire()));
        row.insert(key::MIME_TYPE.to_owned(), optional_text(&self.mime_type));
        row.insert(key::CODEC.to_owned(), optional_text(&self.codec));
        row
    }
}
