use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, Context, Entity};

use super::rise_sticker_models::{RiseStickerPresentationSnapshot, RiseStickerUploadPresentation};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

pub struct RiseStickerPresentation {
    snapshot: Arc<RiseStickerPresentationSnapshot>,
    sticker_uploads: Arc<HashMap<String, RiseStickerUploadPresentation>>,
    custom_emoji_lookup: RefCell<HashMap<String, Option<StickerItem>>>,
}

impl RiseStickerPresentation {
    const CUSTOM_EMOJI_LOOKUP_LIMIT: usize = 64;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: Arc::new(RiseStickerPresentationSnapshot::default()),
            sticker_uploads: Arc::new(HashMap::new()),
            custom_emoji_lookup: RefCell::new(HashMap::new()),
        }
    }

    pub fn snapshot(&self) -> &Arc<RiseStickerPresentationSnapshot> {
        &self.snapshot
    }

    pub fn sticker_uploads(&self) -> &Arc<HashMap<String, RiseStickerUploadPresentation>> {
        &self.sticker_uploads
    }

    pub fn activate(&mut self, account_id: &str, cx: &mut Context<Self>) {
        if self.snapshot.account_id == account_id {
            return;
        }
        self.snapshot = Arc::new(RiseStickerPresentationSnapshot::new(account_id.to_owned()));
        self.sticker_uploads = Arc::new(HashMap::new());
        self.custom_emoji_lookup.borrow_mut().clear();
        cx.notify();
    }

    pub fn publish(&mut self, value: RiseStickerPresentationSnapshot, cx: &mut Context<Self>) {
        if value.account_id != self.snapshot.account_id {
            return;
        }
        if value.sticker_uploads != *self.sticker_uploads {
            self.sticker_uploads = Arc::new(value.sticker_uploads.clone());
        }
        self.snapshot = Arc::new(value);
        self.custom_emoji_lookup.borrow_mut().clear();
        cx.notify();
    }

    pub fn deactivate(&mut self, account_id: &str, cx: &mut Context<Self>) {
        if self.snapshot.account_id != account_id {
            return;
        }
        self.snapshot = Arc::new(RiseStickerPresentationSnapshot::default());
        self.sticker_uploads = Arc::new(HashMap::new());
        self.custom_emoji_lookup.borrow_mut().clear();
        cx.notify();
    }

    pub fn custom_emoji(
        &self,
        id: &str,
        resolve: impl FnOnce(&RiseStickerPresentationSnapshot) -> Option<StickerItem>,
    ) -> Option<StickerItem> {
        let current = Arc::clone(&self.snapshot);
        if let Some(memoised) = self.custom_emoji_lookup.borrow().get(id) {
            return memoised.clone();
        }

        let resolved = resolve(&current);
        let mut lookup = self.custom_emoji_lookup.borrow_mut();
        if lookup.len() >= Self::CUSTOM_EMOJI_LOOKUP_LIMIT {
            lookup.clear();
        }
        lookup.insert(id.to_owned(), resolved.clone());
        resolved
    }
}
