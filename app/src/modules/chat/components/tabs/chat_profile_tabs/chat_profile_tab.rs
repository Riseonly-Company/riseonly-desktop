use rise_chat::ChatType;

/// alias for the canon profile tab
pub type ChatProfileTab = rise_chat::ProfileTab;

/// extra methods for the profile tab
pub trait ChatProfileTabExt: Copy {
    /// the wire string of the tab
    fn id(self) -> &'static str;

    fn tab_key(self) -> &'static str;

    fn title_key(self) -> &'static str;

    /// number of the tab for the scroll coordinator, wire string is the tab key
    fn scroll_key(self) -> i64;

    /// true when the tab shows for this chat type
    fn visible(self, chat_type: ChatType) -> bool;
}

impl ChatProfileTabExt for ChatProfileTab {
    fn id(self) -> &'static str {
        self.wire()
    }

    fn tab_key(self) -> &'static str {
        self.wire()
    }

    fn title_key(self) -> &'static str {
        self.i18n_key()
    }

    fn scroll_key(self) -> i64 {
        self.tab_index() as i64
    }

    fn visible(self, chat_type: ChatType) -> bool {
        match chat_type {
            ChatType::Favourites => self.in_favourites(),
            ChatType::Direct => self.in_direct(),
            ChatType::Group | ChatType::Channel => self.in_community(),
        }
    }
}
