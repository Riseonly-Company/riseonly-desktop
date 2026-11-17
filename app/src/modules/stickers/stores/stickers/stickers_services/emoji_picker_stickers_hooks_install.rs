use gpui::App;

use crate::core::ui::emoji_picker_ui::emoji_picker_stickers_hooks::EmojiPickerStickersHooks;
use crate::modules::stickers::engine::rise_sticker_models::RiseStickerResourceID;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::PackKind;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

fn services(cx: &App) -> Option<&StickersServicesStore> {
    rise_ui::shared::try_read::<StickersServicesStore>(cx)
}

pub(crate) fn install_emoji_picker_stickers_hooks() {
    EmojiPickerStickersHooks::install(EmojiPickerStickersHooks {
        has_resolved_pack: Box::new(|pack_id, cx| services(cx).is_some_and(|store| store.has_resolved_pack(pack_id))),
        is_loading: Box::new(|cx| {
            services(cx).is_some_and(|store| store.is_loading(RiseStickerResourceID::SavedPacks(PackKind::Emoji)))
        }),
        pack_stickers: Box::new(|pack_id, cx| {
            services(cx).map(|store| store.pack_stickers(pack_id)).unwrap_or_default()
        }),
        recents: Box::new(|user_id, cx| {
            services(cx).map(|store| store.recents(user_id, Some(PackKind::Emoji))).unwrap_or_default()
        }),
        saved_pack_summaries: Box::new(|user_id, cx| {
            services(cx).map(|store| store.saved_pack_summaries(user_id, PackKind::Emoji)).unwrap_or_default()
        }),
        saved_packs_has_more: Box::new(|user_id, cx| {
            services(cx).is_some_and(|store| store.saved_packs_has_more(user_id, PackKind::Emoji))
        }),
        saved_packs_next_relative_id: Box::new(|user_id, cx| {
            services(cx).and_then(|store| store.saved_packs_next_relative_id(user_id, PackKind::Emoji))
        }),
        saved_packs_resolved: Box::new(|user_id, cx| {
            services(cx).is_some_and(|store| store.saved_packs_resolved(user_id, PackKind::Emoji))
        }),
        observe: Box::new(|on_change, cx| {
            let store = StickersServicesStore::shared(cx);
            cx.observe(&store, move |_, cx| on_change(cx))
        }),
        load_recents: Box::new(|user_id, cx| {
            StickersInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.load_recents(user_id, Some(PackKind::Emoji), cx));
        }),
        load_saved_packs: Box::new(|user_id, relative_id, force, cx| {
            let relative_id = relative_id.map(str::to_owned);
            StickersInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.load_saved_packs(user_id, PackKind::Emoji, relative_id, force, cx));
        }),
        load_pack_stickers: Box::new(|pack_id, cx| {
            StickersInteractionsStore::shared(cx).update(cx, |store, cx| store.load_pack_stickers(pack_id, cx));
        }),
        write_recent: Box::new(|user_id, sticker_id, cx| {
            StickersInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.write_recent(user_id, sticker_id, Some(PackKind::Emoji), cx));
        }),
        clear_recents: Box::new(|user_id, cx| {
            StickersInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.clear_recents(user_id, PackKind::Emoji, cx));
        }),
    });
}
