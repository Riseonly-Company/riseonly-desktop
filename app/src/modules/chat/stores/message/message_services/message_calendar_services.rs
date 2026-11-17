use gpui::Context;

use crate::modules::chat::engine::messages::rise_chat_calendar::RiseChatCalendarMonth;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MessageChatCalendarPresentationLoadState {
    #[default]
    Idle,
    Loading,
    Loaded,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MessageChatCalendarPresentation {
    pub scope_key: String,
    pub chat_id: String,
    pub topic_id: Option<String>,
    pub thread_root_id: Option<String>,
    pub months: Vec<RiseChatCalendarMonth>,
    pub weekday_symbols: Vec<String>,
    pub load_state: MessageChatCalendarPresentationLoadState,
    pub has_older: bool,
    pub is_loading_older: bool,
}

impl MessageChatCalendarPresentation {
    pub fn is_empty(&self) -> bool {
        self.load_state == MessageChatCalendarPresentationLoadState::Loaded
            && !self.has_older
            && !self.months.iter().any(|month| month.has_any_content())
    }
}

impl MessageServicesStore {
    pub const CHAT_CALENDAR_MAXIMUM_MONTHS: usize = 60;

    pub fn chat_calendar_scope_key(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
    ) -> String {
        format!("{chat_id}|{}|{}", topic_id.unwrap_or_default(), thread_root_id.unwrap_or_default())
    }

    pub fn open_chat_calendar(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        weekday_symbols: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let scope_key = self.chat_calendar_scope_key(chat_id, topic_id, thread_root_id);
        if self.chat_calendar.scope_key != scope_key {
            self.chat_calendar = MessageChatCalendarPresentation::default();
        }
        self.chat_calendar.scope_key = scope_key;
        self.chat_calendar.chat_id = chat_id.to_owned();
        self.chat_calendar.topic_id = topic_id.map(str::to_owned);
        self.chat_calendar.thread_root_id = thread_root_id.map(str::to_owned);
        self.chat_calendar.weekday_symbols = weekday_symbols;
        cx.notify();
    }

    pub fn begin_chat_calendar_load(&mut self, scope_key: &str, is_older: bool, cx: &mut Context<Self>) -> bool {
        if self.chat_calendar.scope_key != scope_key {
            return false;
        }
        if is_older {
            if !self.chat_calendar.has_older || self.chat_calendar.is_loading_older {
                return false;
            }
            self.chat_calendar.is_loading_older = true;
        } else {
            if self.chat_calendar.load_state == MessageChatCalendarPresentationLoadState::Loading {
                return false;
            }
            self.chat_calendar.load_state = MessageChatCalendarPresentationLoadState::Loading;
        }
        cx.notify();
        true
    }

    pub fn apply_chat_calendar_month(
        &mut self,
        month: RiseChatCalendarMonth,
        has_older: bool,
        scope_key: &str,
        is_older: bool,
        cx: &mut Context<Self>,
    ) {
        if self.chat_calendar.scope_key != scope_key {
            return;
        }
        let month_start_ms = month.start_ms;
        let months = &mut self.chat_calendar.months;
        if let Some(index) = months.iter().position(|existing| existing.start_ms == month_start_ms) {
            months[index] = month;
        } else {
            months.push(month);
            months.sort_by_key(|existing| existing.start_ms);
        }
        if months.len() > Self::CHAT_CALENDAR_MAXIMUM_MONTHS {
            let overflow = months.len() - Self::CHAT_CALENDAR_MAXIMUM_MONTHS;
            months.drain(..overflow);
        }
        if months.first().map(|oldest| oldest.start_ms) == Some(month_start_ms) {
            self.chat_calendar.has_older = has_older;
        } else if is_older {
            self.chat_calendar.has_older = false;
        }
        self.chat_calendar.load_state = MessageChatCalendarPresentationLoadState::Loaded;
        if is_older {
            self.chat_calendar.is_loading_older = false;
        }
        cx.notify();
    }

    pub fn fail_chat_calendar_load(&mut self, scope_key: &str, is_older: bool, cx: &mut Context<Self>) {
        if self.chat_calendar.scope_key != scope_key {
            return;
        }
        if is_older {
            self.chat_calendar.is_loading_older = false;
        } else if self.chat_calendar.months.is_empty() {
            self.chat_calendar.load_state = MessageChatCalendarPresentationLoadState::Failed;
        } else {
            self.chat_calendar.load_state = MessageChatCalendarPresentationLoadState::Loaded;
        }
        cx.notify();
    }

    pub fn chat_calendar_oldest_loaded_month_start(&self) -> Option<i64> {
        self.chat_calendar.months.first().map(|month| month.start_ms)
    }
}
