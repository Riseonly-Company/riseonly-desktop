use std::rc::Rc;

use gpui::{App, Entity, SharedString, Window};
use rise_i18n::tr;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use rise_ui::skeleton_ui::SkeletonListUi;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::modules::chat::components::chat_item::chat_item::ChatItem;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::{
    ChatProfileEmptyState, ChatProfileTabBottomSpacer, ChatProfileTabScrollContainer,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tab_window::{
    ChatProfileTabHost, ChatProfileWindowedRow, ChatProfileWindowedRows,
};
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

/// groups shared by the viewer and the peer, only exists for one on one chats
#[derive(IntoElement)]
pub(crate) struct ChatProfileGroupsTab {
    peer_user_id: String,
}

impl ChatProfileGroupsTab {
    const ESTIMATED_ROW_HEIGHT: f32 = 72.0;
    const PREFETCH_THRESHOLD: usize = 8;
    const SKELETON_ROWS: usize = 8;

    pub(crate) fn new(peer_user_id: impl Into<String>) -> Self {
        Self { peer_user_id: peer_user_id.into() }
    }

    fn hold_actions() -> Vec<HoldContextMenuAction> {
        vec![
            HoldContextMenuAction::new("open", tr("ctx_open")).system_image("bubble.left.and.bubble.right"),
            HoldContextMenuAction::new("profile", tr("ctx_open_profile")).system_image("person.2"),
        ]
    }

    fn handle_hold(id: &str, chat_id: &str, nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
        match id {
            "open" => Self::push(RootRoute::Chat { chat_id: Some(chat_id.to_owned()) }, nav_store, cx),
            "profile" => Self::push(RootRoute::ChatProfile { chat_id: Some(chat_id.to_owned()) }, nav_store, cx),
            _ => {}
        }
    }

    fn push(route: RootRoute, nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn row(
        peer_user_id: &str,
        chat: &ChatInfo,
        index: usize,
        item_count: usize,
        nav_store: &Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let services = ChatServicesStore::shared(cx);
        let cache_key = services.update(cx, |services, cx| services.common_chats_cache_key(peer_user_id, cx));
        let relative_id = services.update(cx, |services, cx| services.common_chats_relative_id(peer_user_id, cx));
        let has_more = services.update(cx, |services, cx| services.common_chats_has_more(peer_user_id, cx));

        let press_nav = nav_store.clone();
        let press_chat = chat.id.clone();
        let hold_nav = nav_store.clone();
        let hold_chat = chat.id.clone();
        let load_peer = peer_user_id.to_owned();
        vstack(0.0)
            .w_full()
            .child(
                ChatItem::new(SharedString::from(format!("chat-profile-group-{}", chat.id)), chat.clone())
                    .on_press(move |_, cx| {
                        Self::push(RootRoute::Chat { chat_id: Some(press_chat.clone()) }, press_nav.as_ref(), cx);
                    })
                    .on_hold_action(move |id, _, cx| Self::handle_hold(id, &hold_chat, hold_nav.as_ref(), cx))
                    .custom_hold_actions(Some(Self::hold_actions()))
                    .sai_pagination_prefetch(
                        SharedString::from(format!("chat-profile-group-prefetch-{}", chat.id)),
                        SaiPaginationPrefetchOptions::new(
                            index,
                            item_count,
                            cache_key.unwrap_or_default(),
                            relative_id,
                            has_more,
                        )
                        .threshold(Self::PREFETCH_THRESHOLD),
                        move |_, cx| {
                            ChatInteractionsStore::shared(cx)
                                .update(cx, |store, cx| store.load_more_common_chats(&load_peer, cx));
                        },
                    ),
            )
            .child(div().w_full().h(px(0.5)).bg(theme.border._100))
            .into_any_element()
    }

    fn body(peer_user_id: &str, cx: &mut App) -> AnyElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let groups =
            ChatServicesStore::shared(cx).update(cx, |services, cx| services.common_chats(peer_user_id, cx));
        let slice = ChatActionsStore::shared(cx).read(cx).get_common_chats_sai.clone();
        let is_empty = groups.is_empty();
        let empty_peer = SharedString::from(format!("chat-profile-groups-{peer_user_id}"));
        let peer_user_id = peer_user_id.to_owned();
        let rows_id = empty_peer.clone();
        let scroll_id = empty_peer.clone();
        let loading_id = SharedString::from(format!("{empty_peer}-loading"));
        SaiDataRender::slice(&*slice, move |_, _| {
            ChatProfileTabScrollContainer::new(scroll_id, move |_, _| {
                let groups = Rc::new(groups);
                let item_count = groups.len();
                let rows: Vec<ChatProfileWindowedRow> = (0..item_count)
                    .map(|index| {
                        let groups = groups.clone();
                        let peer_user_id = peer_user_id.clone();
                        let nav_store = nav_store.clone();
                        ChatProfileWindowedRow::new(
                            format!("chat-profile-group-row-{}", groups[index].id),
                            move |_, cx| {
                                Self::row(&peer_user_id, &groups[index], index, item_count, &nav_store, cx)
                            },
                        )
                    })
                    .collect();
                div()
                    .w_full()
                    .child(ChatProfileWindowedRows::new(rows_id, Self::ESTIMATED_ROW_HEIGHT, rows))
                    .child(ChatProfileTabBottomSpacer)
            })
        })
        .id(empty_peer)
        .is_empty(is_empty)
        .no_data_text(tr("chat_profile_empty_groups"))
        .need_pending(is_empty)
        .sai_data_render_loading(move |_, _| {
            ChatProfileTabScrollContainer::new(loading_id.clone(), |_, _| SkeletonListUi {
                rows: Self::SKELETON_ROWS,
                ..SkeletonListUi::default()
            })
            .scroll_enabled(false)
            .shows_indicators(false)
        })
        .sai_data_render_empty(|_, _| ChatProfileEmptyState::new("person.2", tr("chat_profile_empty_groups")))
        .into_any_element()
    }
}

impl RenderOnce for ChatProfileGroupsTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let peer_user_id = self.peer_user_id;
        ChatProfileTabHost::hosted(
            SharedString::from(format!("chat-profile-groups-host-{peer_user_id}")),
            window,
            cx,
            |cx| {
                let actions = ChatActionsStore::shared(cx);
                vec![cx.observe(&actions, |_, _, cx| cx.notify())]
            },
            move |_, cx| Self::body(&peer_user_id, cx),
        )
    }
}
