use std::rc::Rc;

use gpui::{App, EntityId, Global, Window};

use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

pub type StickerPackChatSend = Rc<dyn Fn(StickerItem, &mut Window, &mut App)>;

struct StickerPackChatSendEntry {
    owner: EntityId,
    send: StickerPackChatSend,
}

/// chats on screen that a pack shown beside them sends into, newest last
/// the last entry is the chat in the center
#[derive(Default)]
pub struct StickerPackChatSendTargets {
    entries: Vec<StickerPackChatSendEntry>,
}

impl Global for StickerPackChatSendTargets {}

impl StickerPackChatSendTargets {
    pub fn register(owner: EntityId, send: StickerPackChatSend, cx: &mut App) {
        let targets = cx.default_global::<Self>();
        targets.entries.retain(|entry| entry.owner != owner);
        targets.entries.push(StickerPackChatSendEntry { owner, send });
    }

    pub fn unregister(owner: EntityId, cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        cx.default_global::<Self>().entries.retain(|entry| entry.owner != owner);
    }

    pub fn current(cx: &App) -> Option<StickerPackChatSend> {
        cx.try_global::<Self>().and_then(|targets| targets.entries.last()).map(|entry| entry.send.clone())
    }
}
