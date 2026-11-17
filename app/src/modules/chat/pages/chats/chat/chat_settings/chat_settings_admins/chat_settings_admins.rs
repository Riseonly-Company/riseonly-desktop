use std::rc::Rc;

use gpui::{App, Context, Empty, ListAlignment, SharedString, Subscription, Task, Window};
use rise_i18n::tr;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, list_shell_floating_top_inset,
};
use crate::modules::chat::pages::chats::chat::chat_roles::forum_member_grouping::ForumMemberGrouping;
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_shared::ChatModerationRow;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;

const PREFETCH_THRESHOLD: usize = 8;
const ROW_SPACING: f32 = 1.0;
const SEPARATOR_HEIGHT: f32 = 0.5;

fn page_id(chat_id: &str, part: &str) -> SharedString {
    SharedString::from(format!("chat-settings-admins-{part}-{chat_id}"))
}

fn load_admins(chat_id: &str, cx: &mut App) {
    ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_admins(chat_id, cx));
}

struct ChatSettingsAdminsState {
    list: ListUi,
    ids: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl ChatSettingsAdminsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        let actions = ChatActionsStore::shared(cx);
        Self {
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            ids: Vec::new(),
            _subscriptions: vec![
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
            ],
        }
    }

    /// a next page keeps the viewport, anything else is a different list
    fn sync(&mut self, ids: Vec<String>) {
        if ids == self.ids {
            return;
        }
        if !self.ids.is_empty() && ids.starts_with(&self.ids) {
            self.list.appended(ids.len() - self.ids.len());
        } else {
            self.list.replaced(ids.len());
        }
        self.ids = ids;
    }
}

#[derive(IntoElement)]
pub struct ChatSettingsAdmins {
    chat_id: Option<String>,
}

impl ChatSettingsAdmins {
    pub fn new(chat_id: Option<String>) -> Self {
        Self { chat_id }
    }
}

impl RenderOnce for ChatSettingsAdmins {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let resolved_chat_id = self.chat_id.unwrap_or_default();
        let state = window.use_keyed_state(page_id(&resolved_chat_id, "state"), cx, |_, cx| {
            ChatSettingsAdminsState::new(cx)
        });

        let task_chat_id = resolved_chat_id.clone();
        use_task(page_id(&resolved_chat_id, "task"), (), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| load_admins(&task_chat_id, cx));
        });

        let services = ChatServicesStore::shared(cx);
        let (admins, relative_id, has_more) = services.update(cx, |services, cx| {
            (
                services.admins(&resolved_chat_id, cx),
                services.admins_last_id(&resolved_chat_id, cx),
                services.admins_has_more(&resolved_chat_id, cx),
            )
        });
        let admins = ForumMemberGrouping::viewer_visible_admins(admins, &resolved_chat_id, cx);
        state.update(cx, |state, _| state.sync(admins.iter().map(|admin| admin.user_id.clone()).collect()));

        let is_empty = admins.is_empty();
        let admins = Rc::new(admins);
        let slice = ChatActionsStore::shared(cx).read(cx).admins_sai.clone();
        let cache_key = SharedString::from(ChatServicesStore::admins_cache_key(&resolved_chat_id));

        let row_nav_store = nav_store.clone();
        let list_id = page_id(&resolved_chat_id, "list");
        let list_chat_id = resolved_chat_id.clone();
        let refresh_chat_id = resolved_chat_id.clone();
        let loading_id = page_id(&resolved_chat_id, "skeleton");
        let body = SaiDataRender::slice(&*slice, move |_, cx| {
            let theme = rise_ui::theme(cx).clone();
            let top_inset = list_shell_floating_top_inset(cx);
            let list = state.read(cx).list.refreshable(
                list_id,
                move |_, cx| {
                    load_admins(&refresh_chat_id, cx);
                    Task::ready(())
                },
                move |index, _, cx| {
                    let Some(admin) = admins.get(index) else {
                        return Empty.into_any_element();
                    };
                    let theme = rise_ui::theme(cx).clone();
                    let tap_nav_store = row_nav_store.clone();
                    let route = RootRoute::UserPage {
                        user_id: Some(admin.user_id.clone()),
                        tag: Some(admin.tag.clone()),
                    };
                    let subtitle = if admin.tag.is_empty() { String::new() } else { format!("@{}", admin.tag) };
                    let badge = tr(if admin.is_owner() { "chat_member_role_owner" } else { "chat_member_role_admin" });
                    let more_chat_id = list_chat_id.clone();
                    let row = plain(SharedString::from(format!("chat-settings-admin-{}", admin.user_id)))
                        .w_full()
                        .on_press(move |_, cx| {
                            if let Some(nav_store) = &tap_nav_store {
                                let route = route.clone();
                                nav_store.update(cx, |nav_store, cx| {
                                    nav_store.push(route, NavigationStoreSettlingPush::Drop, cx);
                                });
                            }
                        })
                        .child(
                            ChatModerationRow::new(admin.avatar_url.clone(), admin.name.clone(), subtitle, admin.user_id.clone())
                                .badge(badge)
                                .online(admin.is_online)
                                .is_official(admin.is_official)
                                .is_premium(admin.is_premium),
                        )
                        .sai_pagination_prefetch(
                            SharedString::from(format!("chat-settings-admin-prefetch-{}", admin.user_id)),
                            SaiPaginationPrefetchOptions::new(
                                index,
                                admins.len(),
                                cache_key.clone(),
                                relative_id.clone(),
                                has_more,
                            )
                            .threshold(PREFETCH_THRESHOLD),
                            move |_, cx| {
                                ChatInteractionsStore::shared(cx)
                                    .update(cx, |store, cx| store.load_more_admins(&more_chat_id, cx));
                            },
                        );
                    let top_padding = if index == 0 { top_inset } else { ROW_SPACING };
                    div()
                        .w_full()
                        .pt(px(top_padding))
                        .child(row)
                        .child(div().w_full().mt(px(ROW_SPACING)).h(px(SEPARATOR_HEIGHT)).bg(theme.border._100))
                        .into_any_element()
                },
            );
            div().size_full().bg(theme.bg._100).child(list)
        })
        .id(page_id(&resolved_chat_id, "render"))
        .is_empty(is_empty)
        .no_data_text(tr("chat_settings_admins_empty"))
        .need_pending(is_empty)
        .sai_data_render_loading(move |_, cx| {
            let background = rise_ui::theme(cx).bg._100;
            scroll_view(loading_id.clone())
                .size_full()
                .scroll_disabled(true)
                .shows_indicators(false)
                .list_shell_scroll_under_chrome(cx)
                .bg(background)
                .child(UserListSkeleton::new())
        });

        let keys_chat_id = resolved_chat_id.clone();
        let shell = SettingsWrapperUi::new(tr("chat_settings_admins"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(div().size_full().bg(theme.bg._100).child(body));

        PageRefreshKeys::new(
            page_id(&resolved_chat_id, "keys"),
            move |_, cx| load_admins(&keys_chat_id, cx),
            shell,
        )
    }
}
