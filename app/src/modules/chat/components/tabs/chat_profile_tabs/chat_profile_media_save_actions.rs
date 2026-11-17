use gpui::App;
use rise_i18n::tr;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;

use crate::core::media::media_save_coordinator::{
    MediaSaveCoordinator, MediaSaveTargetKind, MessageMediaSaveClassifier,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_source::{
    ChatProfileMediaEntry, ChatProfileSource,
};
use crate::modules::chat::shared::message_permissions::ChatMessageAction;

pub enum ChatProfileMediaSaveActions {}

impl ChatProfileMediaSaveActions {
    pub fn actions(entry: &ChatProfileMediaEntry) -> Vec<HoldContextMenuAction> {
        if entry.has_protected_content {
            return Vec::new();
        }
        let Some(target) = MessageMediaSaveClassifier::target(&entry.item) else {
            return Vec::new();
        };

        match target {
            MediaSaveTargetKind::Gallery => vec![
                HoldContextMenuAction::new(ChatMessageAction::SAVE_TO_GALLERY, tr("ctx_save"))
                    .system_image("square.and.arrow.down"),
            ],
            MediaSaveTargetKind::Files => vec![
                HoldContextMenuAction::new(ChatMessageAction::SAVE, tr("ctx_save")).system_image("arrow.down.doc"),
            ],
        }
    }

    /// true when the action was one of the save rows, even if nothing got saved
    pub fn handle(id: &str, entry: &ChatProfileMediaEntry, chat_id: &str, cx: &mut App) -> bool {
        if id != ChatMessageAction::SAVE && id != ChatMessageAction::SAVE_TO_GALLERY {
            return false;
        }
        if entry.has_protected_content {
            return true;
        }

        let message = ChatProfileSource::forward_message(entry, chat_id, cx);
        let items = vec![entry.item.clone()];
        let saves_to_gallery = id == ChatMessageAction::SAVE_TO_GALLERY;
        MediaSaveCoordinator::shared(cx).update(cx, |coordinator, cx| {
            if saves_to_gallery {
                coordinator.save_to_gallery(message, items, cx);
            } else {
                coordinator.save_to_files(message, items, cx);
            }
        });
        true
    }
}
