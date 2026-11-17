use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Entity, Hsla, SharedString, Subscription, Window};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_ui::expandable_text_ui::EdgeInsets;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::MainText;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Presence, Transition};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{MainTab, RootRoute};
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, ListShellUi, list_shell_safe_area_insets,
};
use crate::modules::chat::components::chat_item::chat_item::ChatItem;
use crate::modules::chat::components::chat_item::chat_item_skeleton::ChatListSkeleton;
use crate::modules::chat::components::chat_item::chat_list_preview_row::ChatListRowMetrics;
use crate::modules::chat::engine::chats::rise_chat_repository::RiseChatRepository;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatListKind, RiseConversationPresentation,
};
use crate::modules::chat::pages::chats::chat_list_history_warmup_controller::ChatListHistoryWarmupController;
use crate::modules::chat::pages::chats::chats::{
    ChatDeleteDialog, ChatListCommonHoldActionHandler, ChatListCommonHoldActionResult, ChatsListRowLocalState,
};
use crate::modules::chat::pages::chats::chats_page_viewport_store::ChatsPageViewportStore;
use crate::modules::chat::pages::chats::embedded_forum_transition::EmbeddedForumPresentationStore;
use crate::modules::chat::pages::chats::rise_chat_native_list::{RiseChatNativeList, RiseChatNativeListRow};
use crate::modules::chat::pages::chats::rise_chat_viewport::{RiseChatViewport, RiseChatViewportReader};
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_interactions::chat_archive_interactions::ChatArchiveOnError;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::{
    ChatInteractionsDone, ChatInteractionsStore,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const PAGE_ID: &str = "archived";

/// screen state kept as long as the route is drawn
struct ArchivedChatsScreenState {
    page_viewports: Rc<ChatsPageViewportStore>,
    history_warmup: ChatListHistoryWarmupController,
    chat_row_local: HashMap<String, ChatsListRowLocalState>,
    pending_delete_chat: Entity<Presence<ChatInfo>>,
}

impl ArchivedChatsScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let pending_delete_chat =
            cx.new(|_| Presence::new(None).with_transition(Transition::opacity().combined(Transition::scale(0.96))));
        Self {
            page_viewports: Rc::new(ChatsPageViewportStore::new()),
            history_warmup: ChatListHistoryWarmupController::new(),
            chat_row_local: HashMap::new(),
            pending_delete_chat,
        }
    }
}

#[derive(IntoElement)]
pub struct ArchivedChatsScreen;

impl ArchivedChatsScreen {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ArchivedChatsScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for ArchivedChatsScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("archived-chats-screen-state", cx, |_, cx| ArchivedChatsScreenState::new(cx));
        let actions = ArchivedChatsActions { state: state.clone(), nav_store: env::<NavigationStoreKey>(cx) };

        use_task("archived-chats-task", (), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_initial_archived_chats(cx));
            });
        });

        let restore_chat_ids = state.read(cx).page_viewports.restoration_ids(PAGE_ID);
        let appear_state = state.clone();
        let disappear_state = state;
        RiseChatViewportReader::new("archived-chats-viewport", RiseChatListKind::Archive, move |viewport, window, cx| {
            actions.body(viewport, window, cx)
        })
        .restore_chat_ids(restore_chat_ids)
        .on_appear("archived-chats-lifecycle", move |_, cx| {
            appear_state.update(cx, |state, cx| state.history_warmup.activate(PAGE_ID, cx));
        })
        .and_on_disappear(move |_, cx| {
            disappear_state.update(cx, |state, cx| state.history_warmup.cancel_all(cx));
        })
    }
}

#[derive(Clone)]
struct ArchivedChatsActions {
    state: Entity<ArchivedChatsScreenState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl ArchivedChatsActions {
    fn body(&self, viewport: Entity<RiseChatViewport>, window: &mut Window, cx: &mut App) -> AnyElement {
        let snapshot = viewport.update(cx, |viewport, cx| viewport.snapshot(cx));
        let chats = if snapshot.is_none() {
            let limit = usize::try_from(RiseChatRepository::default_page_limit()).unwrap_or_default();
            let archived = ChatServicesStore::shared(cx).update(cx, |service, cx| service.archived_chats(cx));
            archived.into_iter().take(limit).collect::<Vec<_>>()
        } else {
            viewport.update(cx, |viewport, cx| viewport.chats(cx))
        };
        let display_chats = chats.iter().map(|chat| self.display_chat(chat, cx)).collect::<Vec<_>>();

        let presentation = RiseConversationPresentation::shared(cx);
        let actions_store = ChatActionsStore::shared(cx);
        let (render_status, render_error) = if presentation.read(cx).is_active() {
            let presentation = presentation.read(cx);
            (
                presentation.chat_render_status(RiseChatListKind::Archive),
                presentation.chat_render_error(RiseChatListKind::Archive),
            )
        } else {
            let slice = &actions_store.read(cx).get_archived_chats_sai;
            (slice.status(), slice.error())
        };
        let restoration_ready =
            snapshot.is_some() || !self.state.read(cx).page_viewports.has_bookmark(PAGE_ID);

        let shell_actions = self.clone();
        let reader = ArchivedChatsSafeAreaReader {
            content: Box::new(move |safe_insets, _, _| {
                let is_empty = chats.is_empty();
                let error_text = render_error.clone();
                let needs_pending = is_empty && render_status != SaiWsSliceStatus::Fulfilled;
                let list_actions = shell_actions.clone();
                SaiDataRender::new(render_status, move |window, cx| {
                    list_actions.list(viewport, display_chats, safe_insets, restoration_ready, window, cx)
                })
                .id("archived-chats-render")
                .is_empty(is_empty)
                .no_data_text(tr("chat_archive_empty"))
                .error_text(error_text)
                .need_pending(needs_pending)
                .message_height_percent(24.0)
                .no_data_height_percent(24.0)
                .sai_data_render_loading(|_, cx| archived_chats_skeleton(cx))
                .sai_data_render_error(move |_, cx| {
                    let theme = rise_ui::theme(cx).clone();
                    vstack(14.0)
                        .size_full()
                        .items_center()
                        .justify_center()
                        .child(
                            MainText::new(render_error.clone().unwrap_or_else(|| tr("error_fetching_data")))
                                .px(15.0)
                                .color(theme.text.secondary),
                        )
                        .child(
                            LiquidGlassButton::new("archived-chats-retry", LiquidGlassButtonShape::Capsule)
                                .accessibility_label(Some(tr("privacy_settings_retry")))
                                .on_press(|_, cx| {
                                    ChatInteractionsStore::shared(cx)
                                        .update(cx, |store, cx| store.refresh_archived_chats(cx));
                                })
                                .child(MainText::new(tr("privacy_settings_retry")).px(14.0).color(theme.text.primary)),
                        )
                })
                .into_any_element()
            }),
        };

        let back_store = self.nav_store.clone();
        let shell = ListShellUi::new()
            .title(SharedString::from(tr("chat_archive_title")))
            .shows_back_button(true)
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()))
            .on_back(move |_, cx| {
                if let Some(nav_store) = &back_store {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            })
            .content(reader);

        let now = RiseClock::monotonic();
        let pending_delete_chat = self.state.read(cx).pending_delete_chat.clone();
        let dialog = pending_delete_chat.read(cx).render_transitioned(now, window, |chat| {
            let (for_me, for_everyone, cancel) = (self.clone(), self.clone(), self.clone());
            let (me_chat, everyone_chat) = (chat.clone(), chat.clone());
            div().absolute().inset_0().child(ChatDeleteDialog::new(
                chat.clone(),
                move |_, cx| for_me.delete_chat(&me_chat, false, cx),
                move |_, cx| for_everyone.delete_chat(&everyone_chat, true, cx),
                move |_, cx| cancel.dismiss_delete_dialog(cx),
            ))
        });

        div().relative().size_full().child(shell).children(dialog).into_any_element()
    }

    fn list(
        &self,
        viewport: Entity<RiseChatViewport>,
        display_chats: Vec<ChatInfo>,
        safe_insets: EdgeInsets,
        restoration_ready: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let control = self.state.read(cx).page_viewports.control(PAGE_ID);
        let first_rows = control.first_rows(cx);
        let host = window.current_view();
        let observed_first_rows = first_rows.clone();
        let _first_rows_observer =
            window.use_keyed_state("archived-chats-first-rows-observer", cx, move |_, cx| -> Subscription {
                App::observe(cx, &observed_first_rows, move |_, cx: &mut App| cx.notify(host))
            });
        let shows_skeleton =
            !first_rows.read(cx).installed() && (!restoration_ready || !display_chats.is_empty());
        let border = rise_ui::theme(cx).border._100;
        let index_by_id = display_chats
            .iter()
            .enumerate()
            .map(|(index, chat)| (chat.id.clone(), index))
            .collect::<HashMap<_, _>>();
        let rows = display_chats.iter().map(|chat| self.row(chat, &viewport, border, cx)).collect::<Vec<_>>();
        let bookmarks = self.state.read(cx).page_viewports.clone();

        let visible_viewport = viewport;
        let warmup_state = self.state.clone();
        let list = RiseChatNativeList::new("archived-chats-list", rows)
            .control(Some(control))
            .footer_height(96.0 + safe_insets.bottom)
            .scroll_enabled(true)
            .restoration_ready(restoration_ready)
            .page_id(PAGE_ID)
            .bookmarks(Some(bookmarks))
            .on_visible(move |ids, _, cx| {
                visible_viewport.update(cx, |viewport, cx| viewport.visible(ids.to_vec(), cx));
                if let Some(index) = ids.iter().filter_map(|id| index_by_id.get(id).copied()).last() {
                    warmup_state.update(cx, |state, cx| {
                        state.history_warmup.update(PAGE_ID, index as i64, &display_chats, cx);
                    });
                }
            })
            .on_refresh(Some(Rc::new(|cx: &mut App| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_archived_chats(cx));
            })));
        let skeleton = shows_skeleton.then(|| div().absolute().inset_0().child(archived_chats_skeleton(cx)));
        div().relative().size_full().child(list).children(skeleton).into_any_element()
    }

    fn row(
        &self,
        chat: &ChatInfo,
        viewport: &Entity<RiseChatViewport>,
        border: Hsla,
        cx: &mut App,
    ) -> RiseChatNativeListRow {
        let version = viewport.read(cx).content_version(chat);
        let content = {
            let (chat, viewport, actions) = (chat.clone(), viewport.clone(), self.clone());
            Rc::new(move |_: &mut Window, _: &mut App| {
                let (press_chat, press_viewport, press_actions) = (chat.clone(), viewport.clone(), actions.clone());
                let (hold_chat, hold_viewport, hold_actions) = (chat.clone(), viewport.clone(), actions.clone());
                vstack(0.0)
                    .w_full()
                    .child(
                        ChatItem::new(SharedString::from(format!("archived-chat-{}", chat.id)), chat.clone())
                            .on_press(move |_, cx| {
                                press_viewport.update(cx, |viewport, cx| viewport.cache_detail(&press_chat, cx));
                                press_actions.open(&press_chat, cx);
                            })
                            .on_hold_action(move |action_id, _, cx| {
                                hold_viewport.update(cx, |viewport, cx| viewport.cache_detail(&hold_chat, cx));
                                hold_actions.apply_hold_action(&hold_chat, action_id, cx);
                            })
                            .archived(true),
                    )
                    .child(div().w_full().flex_none().h(px(ChatListRowMetrics::SEPARATOR_HEIGHT)).bg(border))
                    .into_any_element()
            })
        };
        let measurement = {
            let chat = chat.clone();
            Rc::new(move |_: &mut Window, _: &mut App| {
                vstack(0.0)
                    .w_full()
                    .child(
                        ChatItem::new(SharedString::from(format!("archived-chat-measure-{}", chat.id)), chat.clone())
                            .is_active(false)
                            .measurement_only(true),
                    )
                    .child(div().w_full().flex_none().h(px(ChatListRowMetrics::SEPARATOR_HEIGHT)))
                    .into_any_element()
            })
        };
        RiseChatNativeListRow { id: chat.id.clone(), version, content, measurement, geometry_version: None }
    }

    fn display_chat(&self, base: &ChatInfo, cx: &mut App) -> ChatInfo {
        let mut chat = base.clone();
        let mute_all = UserServicesStore::shared(cx).read(cx).mute_all_chats_enabled();
        if mute_all {
            let service = ChatServicesStore::shared(cx);
            chat.is_muted = service.read(cx).effective_is_muted(base, cx);
        }
        if let Some(local) = self.state.read(cx).chat_row_local.get(&base.id) {
            if local.unread_cleared {
                chat.unread_count = 0;
                chat.unread_mentions_count = 0;
                chat.unread_replies_count = 0;
                chat.unread_reactions_count = 0;
            }
            if local.unread_marked {
                chat.unread_count = chat.unread_count.max(1);
            }
        }
        chat
    }

    fn apply_hold_action(&self, chat: &ChatInfo, action_id: &str, cx: &mut App) {
        let source = ChatServicesStore::shared(cx).update(cx, |service, cx| service.archived_chats(cx));
        let base = source.into_iter().find(|candidate| candidate.id == chat.id).unwrap_or_else(|| chat.clone());
        let current = self.display_chat(&base, cx);
        let mut row = self.state.read(cx).chat_row_local.get(&chat.id).cloned().unwrap_or_default();

        let (unread_state, unread_chat_id) = (self.state.downgrade(), chat.id.clone());
        let (read_state, read_chat_id) = (self.state.downgrade(), chat.id.clone());
        let result = ChatListCommonHoldActionHandler::apply(
            action_id,
            &base,
            &current,
            &mut row,
            move |cx: &mut App| {
                let _ = unread_state.update(cx, |state, cx| {
                    if let Some(local) = state.chat_row_local.get_mut(&unread_chat_id) {
                        local.unread_marked = false;
                        cx.notify();
                    }
                });
            },
            move |cx: &mut App| {
                let _ = read_state.update(cx, |state, cx| {
                    if let Some(local) = state.chat_row_local.get_mut(&read_chat_id) {
                        local.unread_cleared = false;
                        cx.notify();
                    }
                });
            },
            cx,
        );
        match result {
            ChatListCommonHoldActionResult::Handled => return,
            ChatListCommonHoldActionResult::UpdateLocalState => {
                let chat_id = chat.id.clone();
                self.state.update(cx, |state, cx| {
                    state.chat_row_local.insert(chat_id, row);
                    cx.notify();
                });
                return;
            }
            ChatListCommonHoldActionResult::NotHandled => {}
        }

        match action_id {
            "unarchive" => {
                self.clear_row_local(&chat.id, cx);
                self.restore(&current, cx);
            }
            "leave" => {
                HoldContextMenuPresenter::dismiss(cx);
                let (state, chat_id) = (self.state.clone(), chat.id.clone());
                let on_confirm: ChatInteractionsDone = Box::new(move |cx: &mut App| {
                    state.update(cx, |state, cx| {
                        state.chat_row_local.remove(&chat_id);
                        cx.notify();
                    });
                });
                ChatInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.confirm_leave_chat(&current, Some(on_confirm), None, cx));
            }
            "history_clear" => {
                HoldContextMenuPresenter::dismiss(cx);
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.confirm_clear_history(&current, cx));
            }
            "delete" => {
                HoldContextMenuPresenter::dismiss(cx);
                let pending_delete_chat = self.state.read(cx).pending_delete_chat.clone();
                let now = RiseClock::monotonic();
                pending_delete_chat.update(cx, |pending, cx| {
                    pending.set_in(Some(current), spring(0.26, 0.88), now, cx);
                    cx.notify();
                });
            }
            _ => {}
        }
    }

    fn clear_row_local(&self, chat_id: &str, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.chat_row_local.remove(chat_id);
            cx.notify();
        });
    }

    fn open(&self, chat: &ChatInfo, cx: &mut App) {
        if chat.topics_enabled {
            let forum_chat = chat.clone();
            EmbeddedForumPresentationStore::shared(cx).update(cx, |store, cx| store.present(forum_chat, cx));
            if let Some(nav_store) = &self.nav_store {
                nav_store.update(cx, |nav_store, cx| {
                    nav_store.pop_to_root(cx);
                    nav_store.switch_tab(MainTab::Chats, cx);
                });
            }
            return;
        }

        if let Some(nav_store) = &self.nav_store {
            let route = RootRoute::Chat { chat_id: Some(chat.id.clone()) };
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::default(), cx));
        }
    }

    fn restore(&self, chat: &ChatInfo, cx: &mut App) {
        let on_error: ChatArchiveOnError = Rc::new(|cx: &mut App| {
            Notifier::send(
                tr("chat_archive_action_failed_title"),
                NotifierSendOptions {
                    content: Some(tr("chat_unarchive_action_failed")),
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.unarchive_chat(chat, Some(on_error), cx));
    }

    fn dismiss_delete_dialog(&self, cx: &mut App) {
        let pending_delete_chat = self.state.read(cx).pending_delete_chat.clone();
        let now = RiseClock::monotonic();
        pending_delete_chat.update(cx, |pending, cx| {
            pending.set_in(None, spring(0.22, 0.9), now, cx);
            cx.notify();
        });
    }

    fn delete_chat(&self, chat: &ChatInfo, for_everyone: bool, cx: &mut App) {
        self.dismiss_delete_dialog(cx);
        self.clear_row_local(&chat.id, cx);
        let on_error: ChatInteractionsDone = Box::new(|cx: &mut App| {
            Notifier::send(
                tr("chat_delete_failed_title"),
                NotifierSendOptions {
                    content: Some(tr("chat_delete_failed_text")),
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Top,
                    icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.delete_chat(&chat.id, for_everyone, false, None, Some(on_error), None, cx);
        });
    }
}

fn archived_chats_skeleton(cx: &mut App) -> AnyElement {
    scroll_view("archived-chats-skeleton")
        .list_shell_scroll_under_chrome(cx)
        .scroll_disabled(true)
        .shows_indicators(false)
        .size_full()
        .child(ChatListSkeleton::new().rows(8))
        .into_any_element()
}

#[derive(IntoElement)]
struct ArchivedChatsSafeAreaReader {
    content: Box<dyn FnOnce(EdgeInsets, &mut Window, &mut App) -> AnyElement>,
}

impl RenderOnce for ArchivedChatsSafeAreaReader {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let safe_insets = list_shell_safe_area_insets(cx);
        (self.content)(safe_insets, window, cx)
    }
}
