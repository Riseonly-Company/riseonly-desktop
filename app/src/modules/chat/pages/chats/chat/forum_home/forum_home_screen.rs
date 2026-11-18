use std::rc::Rc;

use gpui::{App, Context, Entity, FontWeight, Hsla, SharedString, Subscription, Window};
use rise_i18n::tr;
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::{PageHeaderIconButton, PageHeaderMetrics};
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{RiseRTCRoomTarget, RootRoute};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellUi, list_shell_floating_top_inset};
use crate::modules::chat::pages::chats::chat::forum_home::forum_topic_list_content::ForumTopicListContent;
use crate::modules::chat::pages::chats::embedded_forum_transition::{EmbeddedForumAction, EmbeddedForumOpenTopicAction};
pub use crate::modules::chat::pages::chats::forum_members_subtitle_hooks::ForumMembersSubtitle;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatTopic};
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
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
                        hstack(4.0)
                            .max_w(px(220.0))
                            .justify_center()
                            .child(
                                div().min_w(px(0.0)).child(
                                    MainText::new(chat.title.clone())
                                        .px(15.0)
                                        .tac(TextAlignT::Center)
                                        .font_weight(FontWeight::BOLD)
                                        .number_of_lines(1),
                                ),
                            )
                            .children(IconUi::sized("person.3.fill", px(12.0), secondary, true).map(|icon| icon.flex_none())),
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
