use std::time::Duration;

use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_i18n::tr;
use rise_widgets::overlay::blocking_loader_store::BlockingLoaderStore;

use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::chat::engine::core::rise_conversation_rpc::RiseConversationEndpoints;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageJumpOutcome;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessageJumpProgressScope {
    pub chat_id: String,
    pub topic_id: Option<String>,
    pub thread_root_id: Option<String>,
}

/// waiting state of a jump to a message
/// local scroll if the target is loaded, round trip if not so nothing shows while its quick
/// after the appearance delay a cancellable loader shows
pub struct ChatMessageJumpProgress {
    generation: i64,
    loader_task: Option<Task<()>>,
    deadline_task: Option<Task<()>>,
    pending_scope: Option<ChatMessageJumpProgressScope>,
}

impl ChatMessageJumpProgress {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { generation: 0, loader_task: None, deadline_task: None, pending_scope: None }
    }

    /// never wait longer than the request itself, timeout comes from canon for get_messages_around
    fn deadline() -> Duration {
        RiseConversationEndpoints::get_messages_around().timeout
    }

    /// arms the wait and returns the token every later report must carry
    pub fn begin(&mut self, scope: ChatMessageJumpProgressScope, cx: &mut Context<Self>) -> i64 {
        self.retire();
        Self::hide_loader(cx);
        self.generation = self.generation.wrapping_add(1);
        let token = self.generation;
        self.pending_scope = Some(scope);

        self.loader_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(BlockingLoaderStore::APPEARANCE_DELAY))
                .await;
            let _ = this.update(cx, |this, cx| {
                if token == this.generation {
                    Self::show_loader(token, cx);
                }
            });
        }));

        self.deadline_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(Self::deadline()).await;
            let _ = this.update(cx, |this, cx| {
                if token != this.generation {
                    return;
                }
                this.cancel(token, cx);
                Self::notify_failed(cx);
            });
        }));

        token
    }

    /// reports how the jump ended
    /// superseded is silent since a newer intent owns the screen
    pub fn finish(&mut self, token: i64, outcome: RiseMessageJumpOutcome, cx: &mut Context<Self>) {
        if token != self.generation {
            return;
        }
        self.retire();
        self.pending_scope = None;
        Self::hide_loader(cx);
        if matches!(outcome, RiseMessageJumpOutcome::Failed) {
            Self::notify_failed(cx);
        }
    }

    /// screen took the list for something else or left, nothing failed but the wait must not
    /// outlive the jump
    /// doesnt cancel the request, the new destination already supersedes it
    pub fn abandon(&mut self, cx: &mut Context<Self>) {
        if self.pending_scope.is_none() {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        self.retire();
        self.pending_scope = None;
        Self::hide_loader(cx);
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.retire();
        self.pending_scope = None;
        Self::hide_loader(cx);
    }

    fn cancel(&mut self, token: i64, cx: &mut Context<Self>) {
        if token != self.generation {
            return;
        }
        let scope = self.pending_scope.take();
        self.generation = self.generation.wrapping_add(1);
        self.retire();
        Self::hide_loader(cx);
        let Some(scope) = scope else {
            return;
        };
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.cancel_pending_message_navigation_request(
                &scope.chat_id,
                scope.topic_id.as_deref(),
                scope.thread_root_id.as_deref(),
                cx,
            );
        });
    }

    fn retire(&mut self) {
        self.loader_task = None;
        self.deadline_task = None;
    }

    fn show_loader(token: i64, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        BlockingLoaderStore::shared(cx).update(cx, move |loader, cx| {
            loader.show(
                move |cx: &mut App| {
                    let _ = this.update(cx, |this, cx| this.cancel(token, cx));
                },
                cx,
            );
        });
    }

    fn hide_loader(cx: &mut Context<Self>) {
        BlockingLoaderStore::shared(cx).update(cx, |loader, cx| loader.hide(cx));
    }

    fn notify_failed(cx: &mut App) {
        let options = NotifierSendOptions {
            r#type: NotifierType::Error,
            position: NotifierPosition::Bottom,
            icon: NotifierIcon::System("arrow.turn.up.left".to_owned()),
            ..NotifierSendOptions::default()
        };
        Notifier::send(tr("message_jump_failed"), options, cx);
    }
}
