use std::rc::Rc;

use gpui::{App, Window};
use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::emoji_picker_ui::emoji_picker_pinned_pack::{EmojiPickerPinnedItem, EmojiPickerPinnedPack};
use crate::core::ui::emoji_picker_ui::emoji_picker_ui::EmojiPickerUi;
use crate::core::ui::premium_diamond::PremiumDiamond;
use crate::modules::chat::stores::message::message_actions::message_types::MessageReaction;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

pub enum PremiumBadgePickerCatalog {}

impl PremiumBadgePickerCatalog {
    pub const PINNED_PACK_SECTION_ID: &'static str = "sec_premium_default";

    pub fn default_badge_item() -> EmojiPickerPinnedItem {
        EmojiPickerPinnedItem {
            id: "premium_default_diamond".to_owned(),
            lottie_name: PremiumDiamond::ANIMATION.to_owned(),
        }
    }
}

/// badge chooser with pack glyphs only
/// plus the Riseonly Premium pack at the bottom with the default mark
/// no unicode emoji, looks like part of the name
#[derive(IntoElement)]
pub struct PremiumBadgeEmojiPicker {
    chrome: bool,
    on_committed: Rc<dyn Fn(&mut Window, &mut App)>,
    rows_settling: bool,
}

impl Default for PremiumBadgeEmojiPicker {
    fn default() -> Self {
        Self { chrome: false, on_committed: Rc::new(|_: &mut Window, _: &mut App| {}), rows_settling: false }
    }
}

impl PremiumBadgeEmojiPicker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn chrome(mut self, chrome: bool) -> Self {
        self.chrome = chrome;
        self
    }

    pub fn rows_settling(mut self, rows_settling: bool) -> Self {
        self.rows_settling = rows_settling;
        self
    }

    pub fn on_committed(mut self, on_committed: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_committed = Rc::new(on_committed);
        self
    }
}

impl RenderOnce for PremiumBadgeEmojiPicker {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let committed_custom = self.on_committed.clone();
        let committed_default = self.on_committed;

        EmojiPickerUi::new("premium-badge-emoji-picker", |_, _, _| {})
            .chrome(self.chrome)
            .rows_settling(self.rows_settling)
            .shows_custom_emoji(true)
            .on_pick_custom(move |item, window, cx| {
                let raw_value = format!("{}{}", MessageReaction::CUSTOM_EMOJI_PREFIX, item.id);
                UserInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.set_premium_badge(Some(raw_value.as_str()), cx));
                committed_custom(window, cx);
            })
            .custom_emoji_only(true)
            .pinned_pack(Some(EmojiPickerPinnedPack {
                id: PremiumBadgePickerCatalog::PINNED_PACK_SECTION_ID.to_owned(),
                title: tr("premium_badge_pack_title"),
                symbol: "sparkle".to_owned(),
                items: vec![PremiumBadgePickerCatalog::default_badge_item()],
                on_pick: Rc::new(move |_: EmojiPickerPinnedItem, window: &mut Window, cx: &mut App| {
                    UserInteractionsStore::shared(cx)
                        .update(cx, |interactions, cx| interactions.set_premium_badge(None, cx));
                    committed_default(window, cx);
                }),
            }))
    }
}
