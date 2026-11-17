use gpui::{App, Subscription};
use rise_core::foundation::foundation_url::Url;

use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::stickers::engine::rise_sticker_models::RiseStickerPresentationSnapshot;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{StickerItem, StickerPack};
use crate::modules::stickers::stores::stickers::stickers_guide_sandbox_hooks::{
    StickersGuideSandboxHooks, set_stickers_guide_sandbox_hooks,
};

pub(crate) fn install_stickers_guide_sandbox_hooks() {
    let _ = set_stickers_guide_sandbox_hooks(StickersGuideSandboxHooks {
        stickers_snapshot,
        owns_sticker_pack,
        owns_stickers_scene,
        toggle_practice_favourite,
        create_practice_pack,
        add_practice_sticker,
        observe_sandbox,
    });
}

fn stickers_snapshot(real: RiseStickerPresentationSnapshot, cx: &App) -> RiseStickerPresentationSnapshot {
    match GuideSandbox::existing(cx) {
        Some(sandbox) => sandbox.read(cx).stickers_snapshot(real),
        None => real,
    }
}

fn owns_sticker_pack(id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_sticker_pack(id))
}

fn owns_stickers_scene(cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_stickers_scene())
}

fn toggle_practice_favourite(sticker: &StickerItem, is_favourite: bool, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.toggle_practice_favourite(sticker, is_favourite, cx));
}

fn create_practice_pack(title: &str, cx: &mut App) -> Option<StickerPack> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.create_practice_pack(title.to_string(), cx))
}

fn add_practice_sticker(source: &Url, emojis: &[String], cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.add_practice_sticker(source, emojis, cx));
}

fn observe_sandbox(on_change: Box<dyn Fn(&mut App)>, cx: &mut App) -> Option<Subscription> {
    let sandbox = GuideSandbox::shared(cx);
    Some(cx.observe(&sandbox, move |_, cx| on_change(cx)))
}
