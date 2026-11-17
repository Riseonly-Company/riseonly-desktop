use gpui::{App, Task};

use crate::modules::chat::engine::chat_engine_hooks::{ChatEngineRtcHooks, set_chat_engine_rtc_hooks};
use crate::modules::chat::stores::chat_stores_hooks::{ChatStoresRtcHooks, set_chat_stores_rtc_hooks};
use crate::modules::rtc::stores::group_voice::rise_rtc_group_voice_settings_stores::RTCGroupVoiceSettingsInteractionsStore;
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;

fn active_room_config(cx: &mut App) -> Option<(String, Option<String>)> {
    let services = RTCServicesStore::shared(cx);
    services
        .read(cx)
        .active_room(cx)
        .map(|room| (room.config.chat_id.clone(), room.config.channel_id.clone()))
}

fn group_voice_settings_load(chat_id: &str, cx: &mut App) -> Task<bool> {
    RTCGroupVoiceSettingsInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.load(chat_id, cx))
}

pub fn install_chat_rtc_hooks() {
    set_chat_engine_rtc_hooks(ChatEngineRtcHooks { active_room_config, group_voice_settings_load });
    set_chat_stores_rtc_hooks(ChatStoresRtcHooks {
        active_room_config,
        group_voice_settings_load: Box::new(|chat_id: String, cx: &mut App| -> Task<bool> {
            RTCGroupVoiceSettingsInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.load(&chat_id, cx))
        }),
    });
}
