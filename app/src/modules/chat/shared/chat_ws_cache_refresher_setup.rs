use std::collections::HashMap;
use std::rc::Rc;

use gpui::App;
use rise_canon::rise_key::guide_screen::camel as guide_screen_camel_key;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_cache_refresher::SaiWsCacheRefresherRule;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{SaiActiveScreen, SaiWsScreenRefreshParams};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tab::{ChatProfileTab, ChatProfileTabExt};
use crate::modules::chat::stores::message::message_actions::message_actions::MessageActionsStore;
use crate::modules::chat::stores::message::message_actions::message_tabs_actions::MessageTabPageArgs;

const CHAT_PROFILE_SCREEN: &str = "ChatProfile";
const CURRENT_TAB_PARAM: &str = "currentTab";
const MEDIA_MESSAGES_CACHE_PREFIX: &str = "getMediaMessages";

fn screen_chat_id(screen: &SaiActiveScreen) -> Option<&str> {
    let chat_id = trimming_whitespaces_and_newlines(screen.params.get(guide_screen_camel_key::CHAT_ID)?);
    (!chat_id.is_empty()).then_some(chat_id)
}

pub enum ChatWsCacheRefresherSetup {}

impl ChatWsCacheRefresherSetup {
    pub fn install_if_needed(cx: &mut App) {
        SaiWs::configure_cache_refresher(
            vec![SaiWsCacheRefresherRule::new(
                vec![CHAT_PROFILE_SCREEN.to_owned()],
                Some(SaiWsScreenRefreshParams::PerScreenPresentKeys(HashMap::from([(
                    CHAT_PROFILE_SCREEN.to_owned(),
                    vec![guide_screen_camel_key::CHAT_ID.to_owned()],
                )]))),
                Rc::new(|screen: &SaiActiveScreen, _cx: &mut App| -> Vec<String> {
                    let Some(chat_id) = screen_chat_id(screen) else {
                        return Vec::new();
                    };
                    vec![SaiCache::new([MEDIA_MESSAGES_CACHE_PREFIX, chat_id]).key]
                }),
                vec![SaiCacheCleanLayer::Memory],
                Some(Rc::new(|screen: &SaiActiveScreen, _keys: &[String], cx: &mut App| {
                    if screen.params.get(CURRENT_TAB_PARAM).map(String::as_str) != Some(ChatProfileTab::Media.tab_key()) {
                        return;
                    }
                    let Some(chat_id) = screen_chat_id(screen) else {
                        return;
                    };
                    MessageActionsStore::shared(cx).update(cx, |store, cx| {
                        store.get_media_messages_action(chat_id, None, MessageTabPageArgs::default(), cx);
                    });
                })),
            )],
            cx,
        );
    }
}
