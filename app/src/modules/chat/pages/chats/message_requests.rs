use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, BoxShadow, Entity, FontWeight, Hsla, SharedString, Window, point};
use rise_canon::ChatMessageRequestAction;
use rise_i18n::tr;
use rise_ui::IconUi;
use rise_ui::expandable_text_ui::EdgeInsets;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonModifiers, LiquidGlassButtonShape, LiquidPressShape};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
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
use crate::modules::chat::pages::chats::chats_page_viewport_store::ChatsPageViewportStore;
use crate::modules::chat::pages::chats::rise_chat_native_list::{RiseChatNativeList, RiseChatNativeListRow};
use crate::modules::chat::pages::chats::rise_chat_viewport::{RiseChatViewport, RiseChatViewportReader};
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

const PAGE_ID: &str = "message-requests";
const PROFILE_ACTION: &str = "PROFILE";

/// screen state kept as long as the route is drawn
struct MessageRequestsScreenState {
    page_viewports: Rc<ChatsPageViewportStore>,
    history_warmup: ChatListHistoryWarmupController,
}

#[derive(IntoElement)]
pub struct MessageRequestsScreen;

impl MessageRequestsScreen {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MessageRequestsScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for MessageRequestsScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("message-requests-screen-state", cx, |_, _| MessageRequestsScreenState {
            page_viewports: Rc::new(ChatsPageViewportStore::new()),
            history_warmup: ChatListHistoryWarmupController::new(),
        });
        let actions = MessageRequestsActions { state: state.clone(), nav_store: env::<NavigationStoreKey>(cx) };

        use_task("message-requests-task", (), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_message_requests(cx));
            });
        });

        let restore_chat_ids = state.read(cx).page_viewports.restoration_ids(PAGE_ID);
        let appear_state = state.clone();
        let disappear_state = state;
        RiseChatViewportReader::new("message-requests-viewport", RiseChatListKind::Requests, move |viewport, _, cx| {
            actions.body(viewport, cx)
        })
        .restore_chat_ids(restore_chat_ids)
        .on_appear("message-requests-lifecycle", move |_, cx| {
            appear_state.update(cx, |state, cx| state.history_warmup.activate(PAGE_ID, cx));
        })
        .and_on_disappear(move |_, cx| {
            disappear_state.update(cx, |state, cx| state.history_warmup.cancel_all(cx));
        })
    }
}

#[derive(Clone)]
struct MessageRequestsActions {
    state: Entity<MessageRequestsScreenState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl MessageRequestsActions {
    fn body(&self, viewport: Entity<RiseChatViewport>, cx: &mut App) -> AnyElement {
        let snapshot = viewport.update(cx, |viewport, cx| viewport.snapshot(cx));
        let requests = if snapshot.is_none() {
            let limit = usize::try_from(RiseChatRepository::default_page_limit()).unwrap_or_default();
            let requests = ChatServicesStore::shared(cx).update(cx, |service, cx| service.message_requests(cx));
            requests.into_iter().take(limit).collect::<Vec<_>>()
        } else {
            viewport.update(cx, |viewport, cx| viewport.chats(cx))
        };

        let presentation = RiseConversationPresentation::shared(cx);
        let actions_store = ChatActionsStore::shared(cx);
        let (render_status, render_error) = if presentation.read(cx).is_active() {
            let presentation = presentation.read(cx);
            (
                presentation.chat_render_status(RiseChatListKind::Requests),
                presentation.chat_render_error(RiseChatListKind::Requests),
            )
        } else {
            let slice = &actions_store.read(cx).get_message_requests_sai;
            (slice.status(), slice.error())
        };
        let restoration_ready =
            snapshot.is_some() || !self.state.read(cx).page_viewports.has_bookmark(PAGE_ID);
        let is_deleting_all = actions_store.read(cx).delete_all_message_requests_sai.is_pending();

        let shell_actions = self.clone();
        let reader = MessageRequestsSafeAreaReader {
            content: Box::new(move |safe_insets, _, cx| {
                let is_empty = requests.is_empty();
                let theme = rise_ui::theme(cx).clone();
                let error_text = render_error.clone();
                let needs_pending = is_empty && render_status != SaiWsSliceStatus::Fulfilled;
                let list_actions = shell_actions.clone();
                let list_requests = requests;
                let render = SaiDataRender::new(render_status, move |_, cx| {
                    list_actions.list(viewport, list_requests, safe_insets, restoration_ready, cx)
                })
                .id("message-requests-render")
                .is_empty(is_empty)
                .no_data_text(tr("message_requests_empty"))
                .error_text(error_text)
                .need_pending(needs_pending)
                .message_height_percent(24.0)
                .no_data_height_percent(24.0)
                .sai_data_render_loading(|_, cx| {
                    scroll_view("message-requests-skeleton")
                        .list_shell_scroll_under_chrome(cx)
                        .scroll_disabled(true)
                        .shows_indicators(false)
                        .size_full()
                        .child(ChatListSkeleton::new().rows(8))
                })
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
                            LiquidGlassButton::new("message-requests-retry", LiquidGlassButtonShape::Capsule)
                                .accessibility_label(Some(tr("privacy_settings_retry")))
                                .on_press(|_, cx| {
                                    ChatInteractionsStore::shared(cx)
                                        .update(cx, |store, cx| store.refresh_message_requests(cx));
                                })
                                .child(MainText::new(tr("privacy_settings_retry")).px(14.0).color(theme.text.primary)),
                        )
                });

                div()
                    .relative()
                    .size_full()
                    .child(render)
                    .when(!is_empty, |stack| {
                        stack.child(Self::delete_all_pill(theme.primary._100, theme.content_on(theme.primary._100), is_deleting_all, safe_insets))
                    })
                    .into_any_element()
            }),
        };

        let back_store = self.nav_store.clone();
        ListShellUi::new()
            .title(SharedString::from(tr("message_requests_title")))
            .shows_back_button(true)
            .on_back(move |_, cx| {
                if let Some(nav_store) = &back_store {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            })
            .content(reader)
            .into_any_element()
    }

    fn delete_all_pill(primary: Hsla, content: Hsla, is_pending: bool, safe_insets: EdgeInsets) -> impl IntoElement {
        let leading = if is_pending {
            progress_view().tint(content).into_any_element()
        } else {
            div().children(IconUi::sized("trash", px(15.0), content, false)).into_any_element()
        };
        let pill = hstack(8.0)
            .w_full()
            .min_h(px(54.0))
            .items_center()
            .justify_center()
            .capsule()
            .bg(primary)
            .shadow(vec![BoxShadow {
                color: primary.opacity(0.28),
                offset: point(px(0.0), px(8.0)),
                blur_radius: px(18.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .child(leading)
            .child(MainText::new(tr("message_requests_delete_all")).px(16.0).font_weight(FontWeight::SEMIBOLD).color(content))
            .liquid_press(
                "message-requests-delete-all",
                LiquidPressShape::Capsule,
                true,
                is_pending,
                false,
                Some(Rc::new(|_: &mut Window, cx: &mut App| {
                    ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.delete_all_message_requests(cx));
                })),
            );
        div()
            .absolute()
            .left(px(12.0))
            .right(px(12.0))
            .bottom(px(18.0_f32.max(safe_insets.bottom + 10.0)))
            .occlude()
            .child(pill)
    }

    fn list(
        &self,
        viewport: Entity<RiseChatViewport>,
        requests: Vec<ChatInfo>,
        safe_insets: EdgeInsets,
        restoration_ready: bool,
        cx: &mut App,
    ) -> AnyElement {
        let border = rise_ui::theme(cx).border._100;
        let index_by_id =
            requests.iter().enumerate().map(|(index, chat)| (chat.id.clone(), index)).collect::<HashMap<_, _>>();
        let rows = requests.iter().map(|chat| self.row(chat, &viewport, border, cx)).collect::<Vec<_>>();
        let bookmarks = self.state.read(cx).page_viewports.clone();
        let scroll_enabled = HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_none();

        let visible_viewport = viewport;
        let warmup_state = self.state.clone();
        RiseChatNativeList::new("message-requests-list", rows)
            .footer_height(96.0 + safe_insets.bottom)
            .scroll_enabled(scroll_enabled)
            .restoration_ready(restoration_ready)
            .page_id(PAGE_ID)
            .bookmarks(Some(bookmarks))
            .on_visible(move |ids, _, cx| {
                visible_viewport.update(cx, |viewport, cx| viewport.visible(ids.to_vec(), cx));
                if let Some(index) = ids.iter().filter_map(|id| index_by_id.get(id).copied()).last() {
                    warmup_state.update(cx, |state, cx| {
                        state.history_warmup.update(PAGE_ID, index as i64, &requests, cx);
                    });
                }
            })
            .on_refresh(Some(Rc::new(|cx: &mut App| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_message_requests(cx));
            })))
            .into_any_element()
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
                let (hold_chat, hold_actions) = (chat.clone(), actions.clone());
                vstack(0.0)
                    .w_full()
                    .child(
                        ChatItem::new(SharedString::from(format!("message-request-{}", chat.id)), chat.clone())
                            .on_press(move |_, cx| {
                                press_viewport.update(cx, |viewport, cx| viewport.cache_detail(&press_chat, cx));
                                press_actions.push(RootRoute::Chat { chat_id: Some(press_chat.id.clone()) }, cx);
                            })
                            .on_hold_action(move |action_id, _, cx| hold_actions.resolve(&hold_chat, action_id, cx))
                            .custom_hold_actions(Some(Self::request_actions())),
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
                        ChatItem::new(SharedString::from(format!("message-request-measure-{}", chat.id)), chat.clone())
                            .is_active(false)
                            .measurement_only(true),
                    )
                    .child(div().w_full().flex_none().h(px(ChatListRowMetrics::SEPARATOR_HEIGHT)))
                    .into_any_element()
            })
        };
        RiseChatNativeListRow { id: chat.id.clone(), version, content, measurement, geometry_version: None }
    }

    fn request_actions() -> Vec<HoldContextMenuAction> {
        vec![
            HoldContextMenuAction::new(ChatMessageRequestAction::Delete.wire(), tr("message_request_delete"))
                .system_image("trash")
                .destructive(true),
            HoldContextMenuAction::new(
                ChatMessageRequestAction::DeleteAndBlock.wire(),
                tr("message_request_delete_and_block"),
            )
            .system_image("hand.raised")
            .destructive(true),
            HoldContextMenuAction::new(ChatMessageRequestAction::Accept.wire(), tr("message_request_accept"))
                .system_image("checkmark.circle"),
            HoldContextMenuAction::new(PROFILE_ACTION, tr("ctx_open_profile"))
                .system_image("person.crop.circle")
                .section_break(true),
        ]
    }

    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::default(), cx));
        }
    }

    fn resolve(&self, chat: &ChatInfo, action_id: &str, cx: &mut App) {
        if action_id == PROFILE_ACTION {
            let participant = &chat.participant;
            self.push(
                RootRoute::UserPage { user_id: Some(participant.id.clone()), tag: Some(participant.tag.clone()) },
                cx,
            );
            return;
        }
        let Some(action) = ChatMessageRequestAction::from_wire(action_id) else {
            return;
        };
        ChatInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.resolve_message_request(&chat.id, action, None, cx));
    }
}

#[derive(IntoElement)]
struct MessageRequestsSafeAreaReader {
    content: Box<dyn FnOnce(EdgeInsets, &mut Window, &mut App) -> AnyElement>,
}

impl RenderOnce for MessageRequestsSafeAreaReader {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let safe_insets = list_shell_safe_area_insets(cx);
        (self.content)(safe_insets, window, cx)
    }
}
