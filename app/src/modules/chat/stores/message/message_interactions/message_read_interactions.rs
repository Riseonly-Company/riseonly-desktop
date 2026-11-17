use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AsyncApp, Context, Task, WeakEntity};
use rise_bot_api::ChatAction;
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};

use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::modules::chat::stores::message::message_actions::message_actions::MessageActionsStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::stickers::engine::rise_sticker_models::RiseStickerPickerTab;

const TYPING_GROUP_KEY: &str = "messageTyping";
const READ_UP_TO_GROUP_KEY: &str = "messageReadUpTo";
const TYPING_RESEND_SECONDS: f64 = 3.0;
const TYPING_STOP_DELAY_MILLISECONDS: i64 = 5_000;
const DEBOUNCER_DEFAULT_MAX_CACHE_DATA: i64 = 10;

const READ_UP_TO_DEBOUNCE_MILLISECONDS: i64 = 300;
const TEXT_TYPING_PRIORITY_WINDOW: TimeInterval = 5.0;
const PICKER_TYPING_TICK: Duration = Duration::from_secs(1);
const TYPING_REPEAT_INTERVAL: Duration = Duration::from_secs(4);
const TYPING_REPEAT_SECONDS: TimeInterval = 4.0;
const UPLOAD_TYPING_DELAY: Duration = Duration::from_secs(1);
const UPLOAD_TYPING_REPEATS: usize = 30;

#[derive(Clone)]
struct ReadUpTo {
    message_id: String,
    created_at: f64,
}

impl ReadUpTo {
    fn covers(&self, message_id: &str, created_at: f64) -> bool {
        self.message_id == message_id || self.created_at > created_at
    }
}

thread_local! {
    static READ_UP_TO_BY_SCOPE: RefCell<HashMap<String, ReadUpTo>> = RefCell::new(HashMap::new());
    static LAST_TYPING_ACTION_BY_CHAT: RefCell<HashMap<String, ChatAction>> = RefCell::new(HashMap::new());
    static TYPING_TOPIC_BY_CHAT: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static PICKER_TYPING_TASK: RefCell<Option<Task<()>>> = const { RefCell::new(None) };
    static PICKER_TYPING_ACTION: Cell<Option<ChatAction>> = const { Cell::new(None) };
    static UPLOAD_TYPING_TASKS: RefCell<HashMap<String, Task<()>>> = RefCell::new(HashMap::new());
    static LAST_TEXT_TYPING_AT_BY_CHAT: RefCell<HashMap<String, TimeInterval>> = RefCell::new(HashMap::new());
}

impl MessageInteractionsStore {
    pub fn observe_chat_list_typing(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        shows_names: bool,
        on_update: Rc<dyn Fn(Option<String>, &mut App)>,
        cx: &mut App,
    ) -> Task<()> {
        if chat_id.is_empty() {
            return Task::ready(());
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.observe_chat_list_typing_action(chat_id, topic_id, shows_names, on_update, cx)
        })
    }

    pub fn mark_chat_as_read(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        clear_marks: bool,
        cx: &mut App,
    ) -> Option<Task<()>> {
        self.resolved_wire_chat_id(chat_id, cx)?;
        Self::actions(cx)
            .update(cx, |actions, cx| actions.mark_chat_as_read_action(chat_id, topic_id, clear_marks, None, cx))
    }

    pub fn mark_chat_read_up_to(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        message_id: &str,
        created_at: f64,
        cx: &mut App,
    ) {
        let Some(wire_chat_id) = self.resolved_wire_chat_id(chat_id, cx) else {
            return;
        };
        if message_id.is_empty() {
            return;
        }
        let key = format!("read-up-to-{wire_chat_id}|{}", topic_id.unwrap_or_default());
        if Self::read_up_to_covers(&key, message_id, created_at) {
            return;
        }
        let message_id = message_id.to_owned();
        let topic_id = topic_id.map(str::to_owned);
        let action_key = key.clone();
        let _ = SaiDebouncer::debounced_action_delay_milliseconds(
            &key,
            Box::new(move |cx: &mut App| {
                if Self::read_up_to_covers(&action_key, &message_id, created_at) {
                    return;
                }
                READ_UP_TO_BY_SCOPE.with(|scopes| {
                    scopes
                        .borrow_mut()
                        .insert(action_key.clone(), ReadUpTo { message_id: message_id.clone(), created_at })
                });
                Self::actions(cx).update(cx, |actions, cx| {
                    if let Some(task) = actions.mark_chat_as_read_action(
                        &wire_chat_id,
                        topic_id.as_deref(),
                        false,
                        Some(&message_id),
                        cx,
                    ) {
                        task.detach();
                    }
                });
            }),
            READ_UP_TO_DEBOUNCE_MILLISECONDS,
            READ_UP_TO_GROUP_KEY,
            DEBOUNCER_DEFAULT_MAX_CACHE_DATA,
            None,
            cx,
        );
    }

    fn read_up_to_covers(key: &str, message_id: &str, created_at: f64) -> bool {
        READ_UP_TO_BY_SCOPE
            .with(|scopes| scopes.borrow().get(key).is_some_and(|previous| previous.covers(message_id, created_at)))
    }

    pub fn set_chat_unread_mark(
        &self,
        chat_id: &str,
        unread: bool,
        on_settled: Option<Rc<dyn Fn(&mut App)>>,
        cx: &mut App,
    ) {
        let Some(chat_id) = self.resolved_wire_chat_id(chat_id, cx) else {
            if let Some(on_settled) = on_settled {
                cx.defer(move |cx| on_settled(cx));
            }
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.set_chat_unread_mark_action(&chat_id, unread, on_settled, cx);
        });
    }

    pub fn mark_media_played(&self, chat_id: &str, message_id: &str, cx: &mut App) {
        Self::actions(cx).update(cx, |actions, cx| actions.mark_media_played_action(chat_id, message_id, cx));
    }

    /// starts the viewer window for view once or timed media and says whether the server took it
    pub fn mark_media_viewed(&self, chat_id: &str, message_id: &str, closed: bool, cx: &mut App) -> Task<bool> {
        let wire_chat_id = self.resolved_wire_chat_id(chat_id, cx).unwrap_or_else(|| chat_id.to_owned());
        Self::actions(cx)
            .update(cx, |actions, cx| actions.mark_media_viewed_action(&wire_chat_id, message_id, closed, cx))
    }

    pub fn set_picker_typing(&mut self, chat_id: &str, tab: Option<RiseStickerPickerTab>, cx: &mut Context<Self>) {
        let action = match tab {
            Some(RiseStickerPickerTab::Stickers) => Some(ChatAction::ChooseSticker),
            Some(RiseStickerPickerTab::Emoji) => Some(ChatAction::ChooseEmoji),
            _ => None,
        };
        if action.is_none() && PICKER_TYPING_ACTION.with(Cell::get) == Some(ChatAction::ChooseMedia) {
            return;
        }
        self.set_picker_typing_action(chat_id, action, cx);
    }

    /// shows chooses media the whole time the media picker of a chat is open, editor included
    /// closing it takes the status back
    pub fn set_media_picker_typing(&mut self, chat_id: &str, is_open: bool, cx: &mut Context<Self>) {
        if !is_open && PICKER_TYPING_ACTION.with(Cell::get) != Some(ChatAction::ChooseMedia) {
            return;
        }
        self.set_picker_typing_action(chat_id, is_open.then_some(ChatAction::ChooseMedia), cx);
    }

    /// repeats a lasting picker status while text typing is quiet, none stops it
    fn set_picker_typing_action(&mut self, chat_id: &str, action: Option<ChatAction>, cx: &mut Context<Self>) {
        let previous = PICKER_TYPING_TASK.with(|slot| slot.borrow_mut().take());
        drop(previous);
        PICKER_TYPING_ACTION.with(|slot| slot.set(action));
        let Some(action) = action else {
            self.set_typing(chat_id, false, ChatAction::Typing, cx);
            return;
        };
        let chat_id = chat_id.to_owned();
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut last_sent_at: Option<TimeInterval> = None;
            loop {
                let ticked = this.update(cx, |store, cx| {
                    let now = RiseClock::now().time_interval_since_1970();
                    let last_text = store.resolved_wire_chat_id(&chat_id, cx).and_then(|wire_chat_id| {
                        LAST_TEXT_TYPING_AT_BY_CHAT.with(|chats| chats.borrow().get(&wire_chat_id).copied())
                    });
                    let text_quiet = last_text.is_none_or(|at| now - at >= TEXT_TYPING_PRIORITY_WINDOW);
                    if !text_quiet {
                        last_sent_at = None;
                    } else if last_sent_at.is_none_or(|at| now - at >= TYPING_REPEAT_SECONDS) {
                        store.set_typing(&chat_id, true, action, cx);
                        last_sent_at = Some(now);
                    }
                });
                if ticked.is_err() {
                    return;
                }
                cx.background_executor().timer(PICKER_TYPING_TICK).await;
            }
        });
        PICKER_TYPING_TASK.with(|slot| *slot.borrow_mut() = Some(task));
    }

    pub fn begin_upload_typing(&mut self, chat_id: &str, upload_id: &str, action: ChatAction, cx: &mut Context<Self>) {
        let previous = UPLOAD_TYPING_TASKS.with(|tasks| tasks.borrow_mut().remove(upload_id));
        drop(previous);
        let chat_id = chat_id.to_owned();
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(UPLOAD_TYPING_DELAY).await;
            for _ in 0..UPLOAD_TYPING_REPEATS {
                if this.update(cx, |store, cx| store.set_typing(&chat_id, true, action, cx)).is_err() {
                    return;
                }
                cx.background_executor().timer(TYPING_REPEAT_INTERVAL).await;
            }
        });
        UPLOAD_TYPING_TASKS.with(|tasks| tasks.borrow_mut().insert(upload_id.to_owned(), task));
    }

    pub fn end_upload_typing(&mut self, chat_id: &str, upload_id: &str, cx: &mut Context<Self>) {
        let Some(task) = UPLOAD_TYPING_TASKS.with(|tasks| tasks.borrow_mut().remove(upload_id)) else {
            return;
        };
        drop(task);
        self.set_typing(chat_id, false, ChatAction::Typing, cx);
    }

    pub fn set_typing_topic(&mut self, chat_id: &str, topic_id: Option<&str>, cx: &mut Context<Self>) {
        let Some(chat_id) = self.resolved_wire_chat_id(chat_id, cx) else {
            return;
        };
        let topic = topic_id.filter(|topic_id| !topic_id.is_empty()).map(str::to_owned);
        let current = TYPING_TOPIC_BY_CHAT.with(|topics| topics.borrow().get(&chat_id).cloned());
        if current == topic {
            return;
        }
        self.set_typing(&chat_id, false, ChatAction::Typing, cx);
        TYPING_TOPIC_BY_CHAT.with(|topics| {
            let mut topics = topics.borrow_mut();
            match topic {
                Some(topic) => topics.insert(chat_id, topic),
                None => topics.remove(&chat_id),
            }
        });
    }

    pub fn set_typing(&mut self, chat_id: &str, is_typing: bool, action: ChatAction, cx: &mut Context<Self>) {
        let Some(chat_id) = self.resolved_wire_chat_id(chat_id, cx) else {
            return;
        };
        let stop_id = format!("typing-stop-{chat_id}");
        let topic_id = TYPING_TOPIC_BY_CHAT.with(|topics| topics.borrow().get(&chat_id).cloned());

        if !is_typing {
            SaiDebouncer::cancel(&stop_id, TYPING_GROUP_KEY, cx);
            if self.last_typing_sent_by_chat.remove(&chat_id).is_none() {
                return;
            }
            LAST_TYPING_ACTION_BY_CHAT.with(|actions| actions.borrow_mut().remove(&chat_id));
            cx.notify();
            Self::actions(cx).update(cx, |actions, cx| {
                actions.typing_action(&chat_id, false, ChatAction::Typing, topic_id.as_deref(), cx)
            });
            return;
        }

        let now = RiseClock::now().time_interval_since_1970();
        if action == ChatAction::Typing {
            LAST_TEXT_TYPING_AT_BY_CHAT.with(|chats| chats.borrow_mut().insert(chat_id.clone(), now));
        }
        let last_sent = self.last_typing_sent_by_chat.get(&chat_id).copied().unwrap_or(0.0);
        let last_action = LAST_TYPING_ACTION_BY_CHAT.with(|actions| actions.borrow().get(&chat_id).copied());
        if now - last_sent > TYPING_RESEND_SECONDS || last_action != Some(action) {
            self.last_typing_sent_by_chat.insert(chat_id.clone(), now);
            LAST_TYPING_ACTION_BY_CHAT.with(|actions| actions.borrow_mut().insert(chat_id.clone(), action));
            cx.notify();
            Self::actions(cx)
                .update(cx, |actions, cx| actions.typing_action(&chat_id, true, action, topic_id.as_deref(), cx));
        }

        let _ = SaiDebouncer::debounced_action_delay_milliseconds(
            &stop_id,
            Box::new(move |cx: &mut App| {
                Self::shared(cx).update(cx, |store, cx| {
                    store.last_typing_sent_by_chat.remove(&chat_id);
                    cx.notify();
                });
                LAST_TYPING_ACTION_BY_CHAT.with(|actions| actions.borrow_mut().remove(&chat_id));
                MessageActionsStore::shared(cx).update(cx, |actions, cx| {
                    actions.typing_action(&chat_id, false, ChatAction::Typing, topic_id.as_deref(), cx)
                });
            }),
            TYPING_STOP_DELAY_MILLISECONDS,
            TYPING_GROUP_KEY,
            DEBOUNCER_DEFAULT_MAX_CACHE_DATA,
            None,
            cx,
        );
    }

    pub fn stop_typing_if_active(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.set_typing(chat_id, false, ChatAction::Typing, cx);
    }
}
