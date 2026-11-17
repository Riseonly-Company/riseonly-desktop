use gpui::App;

use crate::modules::chat::engine::media::rise_chat_media_gallery_presentation::RiseChatMediaGalleryPresentation;
use crate::modules::chat::engine::media::rise_chat_media_gallery_repository::RiseChatMediaGalleryWindow;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

impl MessageServicesStore {
    pub fn media_gallery_window(&self, cx: &mut App) -> Option<RiseChatMediaGalleryWindow> {
        RiseChatMediaGalleryPresentation::shared(cx).read(cx).window().cloned()
    }
}
