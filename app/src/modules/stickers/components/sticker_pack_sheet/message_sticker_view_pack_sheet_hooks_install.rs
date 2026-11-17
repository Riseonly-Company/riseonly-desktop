use gpui::App;

use crate::modules::chat::components::chat::bubble::message_sticker_view_hooks::{
    MessageStickerViewPackSheetHooks, set_message_sticker_view_pack_sheet_hooks,
};
use crate::modules::stickers::components::sticker_pack_sheet::sticker_pack_sheet_store::StickerPackSheetStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::PackKind;

pub fn install_message_sticker_view_pack_sheet_hooks() {
    set_message_sticker_view_pack_sheet_hooks(MessageStickerViewPackSheetHooks { present });
}

fn present(pack_id: &str, kind: PackKind, cx: &mut App) {
    StickerPackSheetStore::shared(cx).update(cx, |store, cx| store.present(pack_id, kind, cx));
}
