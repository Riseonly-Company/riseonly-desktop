use std::rc::Rc;

use gpui::{App, Context, Empty, ListAlignment, Pixels, Point, SharedString, Subscription, Task, Window};
use rise_i18n::tr;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, list_shell_floating_top_inset,
};
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_shared::ChatModerationRow;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;

const PREFETCH_THRESHOLD: usize = 8;
const ROW_SPACING: f32 = 1.0;
const SEPARATOR_HEIGHT: f32 = 0.5;
const UNBAN_ACTION_ID: &str = "unban";
const UNBAN_ACTION_IMAGE: &str = "person.badge.plus";

fn page_id(chat_id: &str, part: &str) -> SharedString {
    SharedString::from(format!("chat-settings-blacklist-{part}-{chat_id}"))
}

fn load_blocked(chat_id: &str, cx: &mut App) {
    ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_blocked(chat_id, cx));
}

struct ChatSettingsBlacklistState {
    list: ListUi,
    ids: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl ChatSettingsBlacklistState {
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

    /// a next page or unban or restored row splices only the changed run so the viewport stays put
    fn sync(&mut self, ids: Vec<String>) {
        if ids == self.ids {
            return;
        }
        if self.ids.is_empty() || ids.is_empty() {
            self.list.replaced(ids.len());
            self.ids = ids;
            return;
        }
        let prefix = self.ids.iter().zip(&ids).take_while(|(old, new)| old == new).count();
        let max_suffix = self.ids.len().min(ids.len()) - prefix;
        let suffix = self
            .ids
            .iter()
            .rev()
            .zip(ids.iter().rev())
            .take(max_suffix)
            .take_while(|(old, new)| old == new)
            .count();
        let old_end = self.ids.len() - suffix;
        let inserted = ids.len() - suffix - prefix;
        self.list.state().splice(prefix..old_end, inserted);
        self.ids = ids;
    }
}

#[derive(IntoElement)]
pub struct ChatSettingsBlacklist {
    chat_id: Option<String>,
}

impl ChatSettingsBlacklist {
    pub fn new(chat_id: Option<String>) -> Self {
        Self { chat_id }
    }
}

impl RenderOnce for ChatSettingsBlacklist {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let resolved_chat_id = self.chat_id.unwrap_or_default();
        let state = window.use_keyed_state(page_id(&resolved_chat_id, "state"), cx, |_, cx| {
            ChatSettingsBlacklistState::new(cx)
        });

        let task_chat_id = resolved_chat_id.clone();
        use_task(page_id(&resolved_chat_id, "task"), (), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| load_blocked(&task_chat_id, cx));
        });

        let services = ChatServicesStore::shared(cx);
        let (blocked, relative_id, has_more) = services.update(cx, |services, cx| {
            (
                services.blocked(&resolved_chat_id, cx),
                services.blocked_last_id(&resolved_chat_id, cx),
                services.blocked_has_more(&resolved_chat_id, cx),
            )
        });
        state.update(cx, |state, _| state.sync(blocked.iter().map(|member| member.user_id.clone()).collect()));

        let is_empty = blocked.is_empty();
        let blocked = Rc::new(blocked);
        let slice = ChatActionsStore::shared(cx).read(cx).blocked_sai.clone();
        let cache_key = SharedString::from(ChatServicesStore::blocked_cache_key(&resolved_chat_id));

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
                    load_blocked(&refresh_chat_id, cx);
                    Task::ready(())
                },
                move |index, _, cx| {
                    let Some(member) = blocked.get(index) else {
                        return Empty.into_any_element();
                    };
                    let theme = rise_ui::theme(cx).clone();
                    let content_member = member.clone();
                    let action_chat_id = list_chat_id.clone();
                    let action_user_id = member.user_id.clone();
                    let tap_nav_store = row_nav_store.clone();
                    let route = RootRoute::UserPage {
                        user_id: Some(member.user_id.clone()),
                        tag: Some(member.tag.clone()),
                    };
                    let more_chat_id = list_chat_id.clone();
                    let row = HoldContextMenuContainer::new(
                        SharedString::from(format!("chat-settings-blacklist-hold-{}", member.user_id)),
                        HoldContextMenuAlignment::Leading,
                        Rc::new(|_: &App| {
                            vec![
                                HoldContextMenuAction::new(UNBAN_ACTION_ID, tr("chat_blacklist_unban"))
                                    .system_image(UNBAN_ACTION_IMAGE),
                            ]
                        }),
                        Rc::new(move |_: &mut Window, _: &mut App| {
                            let subtitle = if content_member.banned_by_name.is_empty() {
                                tr("chat_blacklist_banned")
                            } else {
                                format!("{} {}", tr("chat_blacklist_banned_by"), content_member.banned_by_name)
                            };
                            ChatModerationRow::new(
                                content_member.avatar_url.clone(),
                                content_member.name.clone(),
                                subtitle,
                                content_member.user_id.clone(),
                            )
                            .is_official(content_member.is_official)
                            .is_premium(content_member.is_premium)
                            .into_any_element()
                        }),
                    )
                    .show_emoji_list(false)
                    .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                        if id == UNBAN_ACTION_ID {
                            ChatInteractionsStore::shared(cx)
                                .update(cx, |store, cx| store.unban(&action_chat_id, &action_user_id, cx));
                        }
                    }))
                    .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
                        if let Some(nav_store) = &tap_nav_store {
                            let route = route.clone();
                            nav_store.update(cx, |nav_store, cx| {
                                nav_store.push(route, NavigationStoreSettlingPush::Drop, cx);
                            });
                        }
                    }))
                    .sai_pagination_prefetch(
                        SharedString::from(format!("chat-settings-blacklist-prefetch-{}", member.user_id)),
                        SaiPaginationPrefetchOptions::new(
                            index,
                            blocked.len(),
                            cache_key.clone(),
                            relative_id.clone(),
                            has_more,
                        )
                        .threshold(PREFETCH_THRESHOLD),
                        move |_, cx| {
                            ChatInteractionsStore::shared(cx)
                                .update(cx, |store, cx| store.load_more_blocked(&more_chat_id, cx));
                        },
                    );
                    let top_padding = if index == 0 { top_inset } else { ROW_SPACING };
                    div()
                        .w_full()
                        .pt(px(top_padding))
                        .child(div().w_full().child(row))
                        .child(div().w_full().mt(px(ROW_SPACING)).h(px(SEPARATOR_HEIGHT)).bg(theme.border._100))
                        .into_any_element()
                },
            );
            div().size_full().bg(theme.bg._100).child(list)
        })
        .id(page_id(&resolved_chat_id, "render"))
        .is_empty(is_empty)
        .no_data_text(tr("chat_settings_blacklist_empty"))
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
        let shell = SettingsWrapperUi::new(tr("chat_settings_blacklist"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(div().size_full().bg(theme.bg._100).child(body));

        PageRefreshKeys::new(
            page_id(&resolved_chat_id, "keys"),
            move |_, cx| load_blocked(&keys_chat_id, cx),
            shell,
        )
    }
}
