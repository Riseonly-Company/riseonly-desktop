use gpui::{App, Context, Entity, Hsla, SharedString, Subscription, Window};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::prelude::*;

use crate::modules::chat::pages::chats::chat::chat_roles::forum_member_grouping::ForumMemberGrouping;
use crate::modules::chat::stores::chats::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::presence::stores::presence::presence_interactions::presence_interactions::PresenceInteractionsStore;
use crate::modules::presence::stores::presence::presence_services::presence_services::PresenceServicesStore;

/// keeps the two stores the line reads observed, the screen around it watches neither
struct ForumMembersSubtitleState {
    presence_owner: String,
    shown: Option<(String, i64)>,
    member_ids: Vec<String>,
    line: String,
    _subscriptions: [Subscription; 2],
}

impl ForumMembersSubtitleState {
    fn wake(&mut self, cx: &mut Context<Self>) {
        let Some((chat_id, member_count)) = self.shown.clone() else {
            cx.notify();
            return;
        };
        if self.rebuild(&chat_id, member_count, cx) {
            cx.notify();
        }
    }

    /// rebuilds the member ids and the line, returns whether either changed
    fn rebuild(&mut self, chat_id: &str, member_count: i64, cx: &mut App) -> bool {
        let (member_ids, line) = Self::inputs(chat_id, member_count, cx);
        let shown = Some((chat_id.to_owned(), member_count));
        if self.shown == shown && self.member_ids == member_ids && self.line == line {
            return false;
        }
        self.shown = shown;
        self.member_ids = member_ids;
        self.line = line;
        true
    }

    fn inputs(chat_id: &str, member_count: i64, cx: &mut App) -> (Vec<String>, String) {
        let members = ChatServicesStore::shared(cx).update(cx, |services, cx| services.members_in_memory(chat_id, cx));
        let member_ids: Vec<String> =
            members.iter().map(|member| member.user_id.clone()).filter(|user_id| !user_id.is_empty()).collect();
        let line = ForumMemberGrouping::members_line(chat_id, member_count, None, cx);
        (member_ids, line)
    }
}

/// shows n members and m online under a forum title
/// loads the member list itself and counts live presence
#[derive(IntoElement)]
pub struct ForumMembersSubtitle {
    chat_id: String,
    member_count: i64,
    color: Hsla,
}

impl ForumMembersSubtitle {
    pub fn new(chat_id: String, member_count: i64, color: Hsla) -> Self {
        Self { chat_id, member_count, color }
    }
}

impl RenderOnce for ForumMembersSubtitle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { chat_id, member_count, color } = self;
        let id = format!("forum-subtitle-{chat_id}");
        let state: Entity<ForumMembersSubtitleState> =
            window.use_keyed_state(SharedString::from(format!("{id}-state")), cx, |_, cx| {
                let services = ChatServicesStore::shared(cx);
                let presence = PresenceServicesStore::shared(cx);
                ForumMembersSubtitleState {
                    presence_owner: format!("forum-subtitle-{}", Uuid::new_v4().uuid_string()),
                    shown: None,
                    member_ids: Vec::new(),
                    line: String::new(),
                    _subscriptions: [
                        cx.observe(&services, |state: &mut ForumMembersSubtitleState, _, cx| state.wake(cx)),
                        cx.observe(&presence, |state: &mut ForumMembersSubtitleState, _, cx| state.wake(cx)),
                    ],
                }
            });
        let stale = state
            .read(cx)
            .shown
            .as_ref()
            .is_none_or(|(shown_chat_id, shown_count)| *shown_chat_id != chat_id || *shown_count != member_count);
        if stale {
            state.update(cx, |state, cx| {
                state.rebuild(&chat_id, member_count, cx);
            });
        }
        let (presence_owner, member_ids, line) = {
            let state = state.read(cx);
            (state.presence_owner.clone(), state.member_ids.clone(), state.line.clone())
        };

        let interactions = ChatInteractionsStore::shared(cx);
        let task_chat_id = chat_id.clone();
        use_task(SharedString::from(format!("{id}-task")), chat_id, window, cx, async move |cx| {
            interactions.update(cx, |interactions, cx| {
                interactions.load_members(&task_chat_id, None, None, true, cx);
            });
        });

        let change_owner = presence_owner.clone();
        on_change(
            SharedString::from(format!("{id}-members")),
            member_ids.clone(),
            false,
            window,
            cx,
            move |_, ids, _, cx| {
                PresenceInteractionsStore::shared(cx).update(cx, |store, cx| store.observe(ids, &change_owner, cx));
            },
        );

        let disappear_owner = presence_owner.clone();
        MainText::new(line)
            .px(11.0)
            .tac(TextAlignT::Center)
            .color(color)
            .number_of_lines(1)
            .on_appear(SharedString::from(format!("{id}-appear")), move |_, cx| {
                PresenceInteractionsStore::shared(cx).update(cx, |store, cx| store.observe(&member_ids, &presence_owner, cx));
            })
            .on_disappear(SharedString::from(format!("{id}-disappear")), move |_, cx| {
                PresenceInteractionsStore::shared(cx).update(cx, |store, cx| store.stop_observing(&disappear_owner, cx));
            })
    }
}
