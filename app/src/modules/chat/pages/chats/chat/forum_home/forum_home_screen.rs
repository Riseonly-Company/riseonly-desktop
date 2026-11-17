use std::rc::Rc;

use gpui::{App, Context, Entity, FontWeight, Hsla, SharedString, Subscription, Window};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::{PageHeaderIconButton, PageHeaderMetrics};
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{RiseRTCRoomTarget, RootRoute};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellUi, list_shell_floating_top_inset};
use crate::modules::chat::pages::chats::chat::chat_roles::forum_member_grouping::ForumMemberGrouping;
use crate::modules::chat::pages::chats::chat::forum_home::forum_topic_list_content::ForumTopicListContent;
use crate::modules::chat::pages::chats::embedded_forum_transition::{EmbeddedForumAction, EmbeddedForumOpenTopicAction};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatTopic};
use crate::modules::chat::stores::chats::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::presence::stores::presence::presence_interactions::presence_interactions::PresenceInteractionsStore;
use crate::modules::presence::stores::presence::presence_services::presence_services::PresenceServicesStore;
use crate::modules::rtc::shared::rise_rtc_navigation_gate::RiseRTCNavigationGate;

pub struct ForumHomeScreen {
    chat_id: Option<String>,
    _subscriptions: [Subscription; 1],
}

impl ForumHomeScreen {
    pub fn new(chat_id: Option<String>, cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        Self {
            chat_id,
            _subscriptions: [cx.observe(&services, |_, _, cx| cx.notify())],
        }
    }

    fn resolved_chat_id(&self) -> String {
        self.chat_id.clone().unwrap_or_default()
    }

    fn chat(chat_id: &str, cx: &mut App) -> ChatInfo {
        let services = ChatServicesStore::shared(cx);
        let live = services.read(cx).chat_info(chat_id, cx);
        live.unwrap_or_else(|| ChatInfo::new(chat_id.to_owned()))
    }

    fn push(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn profile_header(
        chat: &ChatInfo,
        chat_id: &str,
        nav_store: &Option<Entity<NavigationStore>>,
        secondary: Hsla,
    ) -> impl IntoElement + use<> {
        let (nav_store, route_chat_id) = (nav_store.clone(), chat_id.to_owned());
        plain(SharedString::from(format!("forum-home-profile-{chat_id}")))
            .w(px(228.0))
            .h(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
            .on_press(move |_, cx| {
                Self::push(&nav_store, RootRoute::ChatProfile { chat_id: Some(route_chat_id.clone()) }, cx);
            })
            .child(
                vstack(0.0)
                    .size_full()
                    .justify_center()
                    .child(
                        div().max_w(px(220.0)).child(
                            MainText::new(chat.title.clone())
                                .px(15.0)
                                .tac(TextAlignT::Center)
                                .font_weight(FontWeight::BOLD)
                                .number_of_lines(1),
                        ),
                    )
                    .child(div().max_w(px(220.0)).child(ForumMembersSubtitle::new(
                        chat_id.to_owned(),
                        chat.member_count,
                        secondary,
                    ))),
            )
    }

    fn settings_button(
        chat: &ChatInfo,
        chat_id: &str,
        nav_store: &Option<Entity<NavigationStore>>,
    ) -> PageHeaderIconButton {
        let (nav_store, route_chat_id, chat) = (nav_store.clone(), chat_id.to_owned(), chat.clone());
        PageHeaderIconButton::new(
            SharedString::from(format!("forum-home-settings-{chat_id}")),
            "ellipsis",
            move |_, cx| {
                let id = Some(route_chat_id.clone());
                let route = if ChatProfilePermissions::can_manage(&chat) {
                    RootRoute::ChatSettings { chat_id: id }
                } else {
                    RootRoute::ChatProfile { chat_id: id }
                };
                Self::push(&nav_store, route, cx);
            },
        )
        .font_size(16.0)
    }
}

impl Render for ForumHomeScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let resolved_chat_id = self.resolved_chat_id();
        let chat = Self::chat(&resolved_chat_id, cx);
        let nav_store = env::<NavigationStoreKey>(cx);

        let back_store = nav_store.clone();
        let on_back = move |_: &mut Window, cx: &mut App| {
            if let Some(nav_store) = &back_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        };

        let (all_store, all_chat_id) = (nav_store.clone(), resolved_chat_id.clone());
        let on_open_all: EmbeddedForumAction = Rc::new(move |_: &mut Window, cx: &mut App| {
            Self::push(&all_store, RootRoute::ForumAll { chat_id: Some(all_chat_id.clone()) }, cx);
        });
        let (topic_store, topic_chat_id) = (nav_store.clone(), resolved_chat_id.clone());
        let on_open_topic: EmbeddedForumOpenTopicAction = Rc::new(move |topic: Option<ChatTopic>, _: &mut Window, cx: &mut App| {
            if let Some(topic) = topic.as_ref().filter(|topic| topic.is_rtc_channel()) {
                if let Some(nav_store) = &topic_store {
                    RiseRTCNavigationGate::open(
                        RiseRTCRoomTarget::Topic { chat_id: topic_chat_id.clone(), topic_id: topic.id.clone() },
                        nav_store,
                        cx,
                    );
                }
            } else {
                Self::push(
                    &topic_store,
                    RootRoute::ChatTopic {
                        chat_id: Some(topic_chat_id.clone()),
                        topic_id: topic.map(|topic| topic.id),
                    },
                    cx,
                );
            }
        });
        div().size_full().child(
            ListShellUi::new()
                .center_slot(Self::profile_header(&chat, &resolved_chat_id, &nav_store, theme.text.secondary))
                .center_slot_self_styled(true)
                .use_safe_zone(true)
                .shows_back_button(true)
                .on_back(on_back)
                .right_jsx(Self::settings_button(&chat, &resolved_chat_id, &nav_store))
                .fills_viewport_background(true)
                .content(div().size_full().child(ForumHomeStandaloneContent {
                    chat_id: resolved_chat_id,
                    on_open_all,
                    on_open_topic,
                })),
        )
    }
}

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

#[derive(IntoElement)]
struct ForumHomeStandaloneContent {
    chat_id: String,
    on_open_all: EmbeddedForumAction,
    on_open_topic: EmbeddedForumOpenTopicAction,
}

impl RenderOnce for ForumHomeStandaloneContent {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        ForumTopicListContent::new(self.chat_id, self.on_open_all, self.on_open_topic)
            .top_inset(list_shell_floating_top_inset(cx))
            .bottom_inset(40.0)
    }
}
