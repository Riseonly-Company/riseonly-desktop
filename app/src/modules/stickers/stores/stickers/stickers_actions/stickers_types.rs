use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::media::rise_media_transfer_hub_hooks::RiseMediaTransferHubHooks;

pub type StickerAssetType = rise_catalog::StickerAssetType;

pub trait StickerAssetTypeInfer: Sized {
    fn infer(asset_type: Option<&str>, file_url: &str) -> Self;
}

impl StickerAssetTypeInfer for StickerAssetType {
    fn infer(asset_type: Option<&str>, file_url: &str) -> Self {
        rise_catalog::infer_sticker_asset_type(asset_type, file_url)
    }
}

pub type PackKind = rise_catalog::StickerPackKind;

fn normalized_media_url(raw: &str) -> String {
    RiseMediaTransferHubHooks::optional_normalized(Some(raw))
        .unwrap_or_else(|| trimming_whitespaces_and_newlines(raw).to_owned())
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct StickerItem {
    pub id: String,
    pub pack_id: String,
    pub file_url: String,
    pub thumbnail_url: Option<String>,
    pub width: i64,
    pub height: i64,
    pub duration_ms: Option<i64>,
    pub position: i64,
    pub created_at_ms: i64,
    pub associated_emojis: Vec<String>,
    pub asset_type: StickerAssetType,
    pub mime_type: Option<String>,
    pub codec: Option<String>,
}

impl StickerItem {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        pack_id: String,
        file_url: String,
        thumbnail_url: Option<String>,
        width: i64,
        height: i64,
        duration_ms: Option<i64>,
        position: i64,
        created_at_ms: i64,
        associated_emojis: Vec<String>,
        asset_type: StickerAssetType,
        mime_type: Option<String>,
        codec: Option<String>,
    ) -> Self {
        Self {
            id,
            pack_id,
            file_url: normalized_media_url(&file_url),
            thumbnail_url: thumbnail_url.map(|value| normalized_media_url(&value)),
            width,
            height,
            duration_ms,
            position,
            created_at_ms,
            associated_emojis,
            asset_type,
            mime_type,
            codec,
        }
    }

    pub fn with_pack_id(&self, value: String) -> StickerItem {
        StickerItem::new(
            self.id.clone(),
            value,
            self.file_url.clone(),
            self.thumbnail_url.clone(),
            self.width,
            self.height,
            self.duration_ms,
            self.position,
            self.created_at_ms,
            self.associated_emojis.clone(),
            self.asset_type,
            self.mime_type.clone(),
            self.codec.clone(),
        )
    }

    pub fn with_position(&self, value: i64) -> StickerItem {
        StickerItem::new(
            self.id.clone(),
            self.pack_id.clone(),
            self.file_url.clone(),
            self.thumbnail_url.clone(),
            self.width,
            self.height,
            self.duration_ms,
            value,
            self.created_at_ms,
            self.associated_emojis.clone(),
            self.asset_type,
            self.mime_type.clone(),
            self.codec.clone(),
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StickerPackRights {
    pub can_add_stickers: bool,
    pub can_delete_stickers: bool,
    pub can_edit_pack: bool,
    pub can_edit_stickers: bool,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct StickerPack {
    pub id: String,
    pub title: String,
    pub link: String,
    pub is_admin: bool,
    pub is_creator: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub is_default: bool,
    pub is_saved: bool,
    pub pack_kind: PackKind,
    pub preview_url: String,
    pub preview_asset_type: StickerAssetType,
    pub is_premium: bool,
    pub unique_saves: i64,
    pub my_rights: Option<StickerPackRights>,
}

impl StickerPack {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        title: String,
        link: String,
        is_admin: bool,
        is_creator: bool,
        created_at_ms: i64,
        updated_at_ms: i64,
        is_default: bool,
        is_saved: bool,
        pack_kind: PackKind,
        preview_url: String,
        preview_asset_type: StickerAssetType,
        is_premium: bool,
        unique_saves: i64,
        my_rights: Option<StickerPackRights>,
    ) -> Self {
        Self {
            id,
            title,
            link,
            is_admin,
            is_creator,
            created_at_ms,
            updated_at_ms,
            is_default,
            is_saved,
            pack_kind,
            preview_url: normalized_media_url(&preview_url),
            preview_asset_type,
            is_premium,
            unique_saves: unique_saves.max(0),
            my_rights,
        }
    }

    pub fn is_coauthor(&self) -> bool {
        self.is_admin && !self.is_creator
    }

    pub fn can_add_stickers(&self) -> bool {
        self.my_rights.map_or(self.is_creator, |rights| rights.can_add_stickers)
    }

    pub fn can_edit_pack(&self) -> bool {
        self.my_rights.map_or(self.is_creator, |rights| rights.can_edit_pack)
    }

    pub fn can_edit_stickers(&self) -> bool {
        self.my_rights.map_or(self.is_creator, |rights| rights.can_edit_stickers)
    }

    pub fn can_delete_stickers(&self) -> bool {
        self.my_rights.map_or(self.is_creator, |rights| rights.can_delete_stickers)
    }

    pub fn cover_item(&self) -> Option<StickerItem> {
        let url = trimming_whitespaces_and_newlines(&self.preview_url);
        if url.is_empty() {
            return None;
        }
        Some(StickerItem::new(
            self.id.clone(),
            self.id.clone(),
            url.to_owned(),
            None,
            0,
            0,
            None,
            0,
            self.created_at_ms,
            Vec::new(),
            self.preview_asset_type,
            None,
            None,
        ))
    }

    pub fn with_saved(&self, value: bool) -> StickerPack {
        StickerPack::new(
            self.id.clone(),
            self.title.clone(),
            self.link.clone(),
            self.is_admin,
            self.is_creator,
            self.created_at_ms,
            self.updated_at_ms,
            self.is_default,
            value,
            self.pack_kind,
            self.preview_url.clone(),
            self.preview_asset_type,
            self.is_premium,
            self.unique_saves,
            self.my_rights,
        )
    }

    pub fn with_creator(&self, value: bool) -> StickerPack {
        StickerPack::new(
            self.id.clone(),
            self.title.clone(),
            self.link.clone(),
            self.is_admin,
            value,
            self.created_at_ms,
            self.updated_at_ms,
            self.is_default,
            self.is_saved,
            self.pack_kind,
            self.preview_url.clone(),
            self.preview_asset_type,
            self.is_premium,
            self.unique_saves,
            self.my_rights,
        )
    }

    pub fn with_pack_kind(&self, value: PackKind) -> StickerPack {
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
            value,
            self.preview_url.clone(),
            self.preview_asset_type,
            self.is_premium,
            self.unique_saves,
            self.my_rights,
        )
    }

    pub fn with_preview(&self, url: String, asset_type: StickerAssetType) -> StickerPack {
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
            self.pack_kind,
            url,
            asset_type,
            self.is_premium,
            self.unique_saves,
            self.my_rights,
        )
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct GifItem {
    pub id: String,
    pub file_url: String,
    pub preview_url: String,
    pub width: i64,
    pub height: i64,
    pub duration_ms: i64,
    pub source: String,
    pub source_id: String,
    pub added_at_ms: i64,
}
