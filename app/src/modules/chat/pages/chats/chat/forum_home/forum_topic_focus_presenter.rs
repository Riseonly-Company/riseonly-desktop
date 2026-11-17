use std::time::Duration;

use gpui::{App, AsyncApp, Context, Entity, WeakEntity};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForumTopicFocusPresenterRequest {
    pub id: u64,
    pub chat_id: String,
    pub topic_id: String,
}

/// a link into a text topic opens its group first and the topic after, like a discord channel link
/// the forum list of that chat claims the request and brings the row to the middle then opens the topic
pub struct ForumTopicFocusPresenter {
    pending: Option<ForumTopicFocusPresenterRequest>,
    seen: bool,
    next_id: u64,
}

impl ForumTopicFocusPresenter {
    pub const CLAIM_WINDOW: Duration = Duration::from_millis(2500);
    pub const LIFETIME: Duration = Duration::from_secs(12);

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { pending: None, seen: false, next_id: 0 }
    }

    pub fn pending(&self) -> Option<&ForumTopicFocusPresenterRequest> {
        self.pending.as_ref()
    }

    pub fn enqueue(
        &mut self,
        chat_id: String,
        topic_id: String,
        on_unclaimed: impl FnOnce(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.next_id = self.next_id.wrapping_add(1);
        let request = ForumTopicFocusPresenterRequest { id: self.next_id, chat_id, topic_id };
        self.pending = Some(request.clone());
        self.seen = false;
        cx.notify();

        let claimed = request.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(Self::CLAIM_WINDOW).await;
            let unclaimed = this
                .update(cx, |this, cx| {
                    if this.pending.as_ref() != Some(&claimed) || this.seen {
                        return false;
                    }
                    this.pending = None;
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if unclaimed {
                let _ = cx.update(|cx| on_unclaimed(cx));
            }
        })
        .detach();

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(Self::LIFETIME).await;
            let _ = this.update(cx, |this, cx| {
                if this.pending.as_ref() == Some(&request) {
                    this.pending = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// the forum list of this chat takes notice
    /// the request stays until a list opens the topic
    /// the embedded forum rebuilds its list and the new list picks it up
    pub fn request(&mut self, chat_id: &str) -> Option<ForumTopicFocusPresenterRequest> {
        let request = self.pending.as_ref().filter(|request| request.chat_id == chat_id)?.clone();
        self.seen = true;
        Some(request)
    }

    /// the first list to get here opens the topic, any other one stands down
    pub fn finish(&mut self, request: &ForumTopicFocusPresenterRequest, cx: &mut Context<Self>) -> bool {
        if self.pending.as_ref() != Some(request) {
            return false;
        }
        self.pending = None;
        cx.notify();
        true
    }
}

/// pace of a topic link
/// how long the list waits for the topic to land and how it glides to it
/// and how long the row stays in view before the topic opens
pub struct ForumTopicFocusPolicy;

impl ForumTopicFocusPolicy {
    pub const TOPIC_WAIT: Duration = Duration::from_secs(6);
    pub const POLL: Duration = Duration::from_millis(120);
    pub const SCROLL_SECONDS: f64 = 0.35;
    pub const OPEN_AFTER_SCROLL: Duration = Duration::from_millis(450);
}
