use gpui::{AnyElement, IntoElement};
use rise_i18n::tr;
use rise_ui::animated_tabs_ui::animated_tabs_types::TabConfig;
use rise_ui::swiftui::environment::EnvKey;

use crate::modules::chat::components::chat_search::chat_search_chats_tab::ChatSearchChatsTab;
use crate::modules::chat::components::chat_search::chat_search_overlay_store::ChatSearchMessageKind;
use crate::modules::chat::components::chat_search::chat_search_simple_tabs::{
    ChatSearchChannelsTab, ChatSearchDownloadsTab, ChatSearchFilesTab, ChatSearchLinksTab, ChatSearchMediaTab, ChatSearchMusicTab,
    ChatSearchPostsTab, ChatSearchVoiceTab,
};

pub enum ChatSearchTabsCatalog {}

/// the tab the search overlay shows
/// a typed result page more than one page away is not on screen even mid swipe
/// so it keeps its last query instead of redrawing each keystroke
pub struct ChatSearchActiveTabKey;

impl EnvKey for ChatSearchActiveTabKey {
    type Value = Option<i64>;

    fn default_value() -> Self::Value {
        None
    }
}

impl ChatSearchTabsCatalog {
    pub fn all_tabs(query: &str) -> Vec<TabConfig> {
        let tab = |id: i64, text: String, content: fn(String) -> AnyElement| {
            let query = query.to_owned();
            Self::tab(id, text, move || content(query.clone()))
        };
        vec![
            tab(0, tr("chat_search_tab_chats"), |query| ChatSearchChatsTab::new(query).into_any_element()),
            tab(1, tr("chat_search_tab_channels"), |query| ChatSearchChannelsTab::new(query).into_any_element()),
            tab(2, tr("chat_search_tab_posts"), |query| ChatSearchPostsTab::new(query).into_any_element()),
            tab(3, tr("chat_search_tab_media"), |query| ChatSearchMediaTab::new(query).into_any_element()),
            tab(4, tr("chat_search_tab_downloads"), |_| ChatSearchDownloadsTab::new().into_any_element()),
            tab(5, tr("chat_search_tab_links"), |query| ChatSearchLinksTab::new(query).into_any_element()),
            tab(6, tr("chat_search_tab_files"), |query| ChatSearchFilesTab::new(query).into_any_element()),
            tab(7, tr("chat_search_tab_music"), |query| ChatSearchMusicTab::new(query).into_any_element()),
            tab(8, tr("chat_search_tab_voice"), |query| ChatSearchVoiceTab::new(query).into_any_element()),
        ]
    }

    pub fn typed_tab_id(kind: ChatSearchMessageKind) -> i64 {
        match kind {
            ChatSearchMessageKind::Media => 3,
            ChatSearchMessageKind::Links => 5,
            ChatSearchMessageKind::Files => 6,
            ChatSearchMessageKind::Music => 7,
            ChatSearchMessageKind::Voices => 8,
        }
    }

    fn tab(id: i64, text: String, content: impl Fn() -> AnyElement + 'static) -> TabConfig {
        TabConfig::new(id, text, move |_, _| content())
    }
}
