use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, Context, Entity, FontWeight, ListAlignment, Pixels, Point, Render, SharedString, StyleRefinement,
    Subscription, Task, Window, canvas,
};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::{AppTheme, color};
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::progress_view::progress_view;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{MainTab, RootRoute};
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::{SaiWebSocketManager, SaiWebSocketManagerPhase};
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, list_shell_floating_top_inset,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::components::bot_ui_components::{BotAvatarView, BotStateView};
use crate::modules::bot::stores::bot::bot_actions::bot_actions::BotActionsStore;
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotModel;
use crate::modules::bot::stores::bot::bot_interactions::bot_interactions::BotInteractionsStore;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::chat::widgets::bottomsheets::create_chat_sheet::create_chat_sheet_state::CreateChatSheetState;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;

const CARD_RADIUS: f32 = 22.0;
const CARD_HORIZONTAL_PADDING: f32 = 12.0;
const CARD_TOP_PADDING: f32 = 12.0;
const CARD_BOTTOM_PADDING: f32 = 90.0;
const SEPARATOR_LEADING: f32 = 80.0;
const SEPARATOR_OPACITY: f32 = 0.55;
const PREFETCH_THRESHOLD: usize = 8;
const ACTIVE_GREEN: &str = "#34C759";
const ACTION_SETTINGS: &str = "settings";
const ACTION_CHAT: &str = "chat";
const ACTION_COPY: &str = "copy";
const ACTION_DELETE: &str = "delete";

/// what a row height depends on next to its id, gpui keeps measured heights until told
type BotRowKey = (String, bool);

fn row_key(bot: &BotModel) -> BotRowKey {
    (bot.id.clone(), bot.is_moderation_disabled() || !bot.description.is_empty())
}

/// row heights by variant (third line or not)
/// every text line is clamped to one so height doesnt follow width, one measured row stands
/// for its variant
type BotRowHeights = Rc<Cell<[Option<Pixels>; 2]>>;

/// one bot row as its own view
/// stories and avatars and presses notify only their row, a page frame replays the cached row
struct MyBotsRowView {
    bot: BotModel,
    nav_store: Option<Entity<NavigationStore>>,
    corners: MyBotsRowCorners,
    _theme: Subscription,
}

/// which card corners a row rounds, the first row the top and the last row the bottom
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct MyBotsRowCorners {
    top: bool,
    bottom: bool,
}

impl MyBotsRowView {
    fn new(
        bot: BotModel,
        nav_store: Option<Entity<NavigationStore>>,
        corners: MyBotsRowCorners,
        cx: &mut Context<Self>,
    ) -> Self {
        Self { bot, nav_store, corners, _theme: cx.observe_global::<AppTheme>(|_, cx| cx.notify()) }
    }
}

impl Render for MyBotsRowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        MyBots::row_container(self.bot.clone(), theme, self.nav_store.clone(), self.corners)
    }
}

fn interactions(cx: &mut App) -> Entity<BotInteractionsStore> {
    BotInteractionsStore::shared(cx)
}

/// page state, its stores and the lazy list one row per bot plus a footer with the page
/// spinner and bottom inset
struct MyBotsState {
    waits_for_network: bool,
    list: ListUi,
    row_keys: Vec<BotRowKey>,
    has_more: bool,
    rows: HashMap<String, Entity<MyBotsRowView>>,
    row_heights: BotRowHeights,
    _subscriptions: Vec<Subscription>,
}

impl MyBotsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = BotServicesStore::shared(cx);
        let actions = BotActionsStore::shared(cx);
        let socket = SaiWebSocketManager::shared(cx);
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        Self {
            waits_for_network: false,
            list: ListUi::new(rise_ui::theme(cx), 1, ListAlignment::Top),
            row_keys: Vec::new(),
            has_more: false,
            rows: HashMap::new(),
            row_heights: Rc::new(Cell::new([None; 2])),
            _subscriptions: vec![
                cx.observe_global::<AppTheme>(|state, cx| {
                    state.row_heights.set([None; 2]);
                    cx.notify();
                }),
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&cache_revisions, |_, _, cx| cx.notify()),
                cx.observe(&socket, |state, socket, cx| {
                    let phase = socket.read(cx).phase();
                    if MyBots::socket_waits_for_network(phase) {
                        state.waits_for_network = true;
                    } else if matches!(phase, SaiWebSocketManagerPhase::Connected(..)) {
                        state.waits_for_network = false;
                    }
                    cx.notify();
                }),
            ],
        }
    }

    fn sync_waits_for_network(&mut self, cx: &mut Context<Self>) {
        let socket = SaiWebSocketManager::shared(cx);
        self.waits_for_network = MyBots::socket_waits_for_network(socket.read(cx).phase());
        cx.notify();
    }

    /// row views in list order, a row whose bot changed renders again and the rest replay
    fn row_views(
        &mut self,
        bots: &[BotModel],
        has_more: bool,
        nav_store: Option<Entity<NavigationStore>>,
        cx: &mut Context<Self>,
    ) -> Vec<Entity<MyBotsRowView>> {
        let mut rows = HashMap::with_capacity(bots.len());
        let count = bots.len();
        let views = bots
            .iter()
            .enumerate()
            .map(|(index, bot)| {
                let corners = MyBotsRowCorners { top: index == 0, bottom: index + 1 == count && !has_more };
                let view = match self.rows.remove(&bot.id) {
                    Some(view) => {
                        view.update(cx, |row, cx| {
                            if row.bot != *bot || row.corners != corners {
                                row.bot = bot.clone();
                                row.corners = corners;
                                cx.notify();
                            }
                            row.nav_store = nav_store.clone();
                        });
                        view
                    }
                    None => {
                        let (bot, nav_store) = (bot.clone(), nav_store.clone());
                        cx.new(|cx| MyBotsRowView::new(bot, nav_store, corners, cx))
                    }
                };
                rows.insert(bot.id.clone(), view.clone());
                view
            })
            .collect();
        self.rows = rows;
        views
    }

    fn sync_list(&mut self, row_keys: Vec<BotRowKey>, has_more: bool) {
        let prefix = self.row_keys.iter().zip(&row_keys).take_while(|(old, new)| old == new).count();
        let suffix = self.row_keys[prefix..]
            .iter()
            .rev()
            .zip(row_keys[prefix..].iter().rev())
            .take_while(|(old, new)| old == new)
            .count();
        let removed = self.row_keys.len() - prefix - suffix;
        let inserted = row_keys.len() - prefix - suffix;
        if removed > 0 || inserted > 0 {
            self.list.state().splice(prefix..prefix + removed, inserted);
        }
        if has_more != self.has_more {
            self.list.state().splice(row_keys.len()..row_keys.len() + 1, 1);
        }
        self.row_keys = row_keys;
        self.has_more = has_more;
    }
}

#[derive(IntoElement, Default)]
pub struct MyBots;

impl MyBots {
    pub fn new() -> Self {
        Self
    }

    fn socket_waits_for_network(phase: &SaiWebSocketManagerPhase) -> bool {
        match phase {
            SaiWebSocketManagerPhase::Disconnected | SaiWebSocketManagerPhase::Failed(_) => true,
            SaiWebSocketManagerPhase::Idle
            | SaiWebSocketManagerPhase::Connecting(_)
            | SaiWebSocketManagerPhase::Connected(..) => false,
        }
    }

    fn open_create_menu(nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |nav_store, cx| {
                nav_store.pop_to_root(cx);
                nav_store.switch_tab(MainTab::Chats, cx);
            });
        }
        cx.defer(|cx| {
            CreateChatSheetState::shared(cx).update(cx, |state, cx| state.open(cx));
        });
    }

    fn push(nav_store: Option<&Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn present_delete_confirm(bot_id: String, cx: &mut App) {
        NativeConfirmAlert::present_delete(
            &tr("bot_delete_title"),
            &tr("bot_delete_message"),
            Some(tr("bot_delete")),
            None,
            false,
            None,
            None,
            move |cx| {
                interactions(cx).update(cx, |store, cx| store.delete_bot(&bot_id, None, cx));
            },
            cx,
        );
    }

    fn handle_action(action: &str, bot: &BotModel, nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
        match action {
            ACTION_SETTINGS => Self::push(nav_store, RootRoute::BotSettings { bot_id: bot.id.clone() }, cx),
            ACTION_CHAT => {
                if !bot.private_chat_id.is_empty() {
                    Self::push(nav_store, RootRoute::Chat { chat_id: Some(bot.private_chat_id.clone()) }, cx);
                }
            }
            ACTION_COPY => UIPasteboard::general().set_string(&format!("@{}", bot.tag), cx),
            ACTION_DELETE => Self::present_delete_confirm(bot.id.clone(), cx),
            _ => {}
        }
    }

    fn menu_actions(bot: &BotModel) -> Vec<HoldContextMenuAction> {
        let mut actions = vec![
            HoldContextMenuAction::new(ACTION_SETTINGS, tr("bot_settings_title")).system_image("gearshape.fill"),
            HoldContextMenuAction::new(ACTION_CHAT, tr("bot_open_chat"))
                .system_image("bubble.left.fill")
                .disabled(bot.private_chat_id.is_empty()),
            HoldContextMenuAction::new(ACTION_COPY, tr("bot_copy_tag")).system_image("doc.on.doc"),
        ];
        if !bot.is_system {
            actions.push(
                HoldContextMenuAction::new(ACTION_DELETE, tr("bot_delete")).system_image("trash").destructive(true),
            );
        }
        actions
    }

    fn row_content(bot: &BotModel, theme: &AppTheme, corners: MyBotsRowCorners) -> AnyElement {
        let hover_background = theme.bg._300;
        let dot = if bot.is_moderation_disabled() {
            theme.semantic.error_100
        } else if bot.is_active {
            color(ACTIVE_GREEN)
        } else {
            theme.text.secondary
        };
        hstack(13.0)
            .w_full()
            .items_center()
            .px(px(16.0))
            .py(px(12.0))
            .bg(theme.bg._200)
            .when(corners.top, |row| row.rounded_t(px(CARD_RADIUS)))
            .when(corners.bottom, |row| row.rounded_b(px(CARD_RADIUS)))
            .hover(move |row| row.bg(hover_background))
            .child(BotAvatarView::new(SharedString::from(format!("my-bots-avatar-{}", bot.id)), bot.clone()).size(54.0))
            .child(
                vstack(4.0)
                    .flex_1()
                    .min_w_0()
                    .items_start()
                    .child(
                        hstack(6.0)
                            .max_w_full()
                            .min_w_0()
                            .items_center()
                            .child(
                                MainText::new(bot.name.clone())
                                    .px(16.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .number_of_lines(1),
                            )
                            .child(div().flex_none().size(px(7.0)).rounded_full().bg(dot)),
                    )
                    .child(div().w_full().min_w_0().child(SecondaryText::new(format!("@{}", bot.tag)).px(13.0).line_limit(1)))
                    .when(bot.is_moderation_disabled(), |column| {
                        column.child(
                            MainText::new(tr("bot_disabled_by_moderation"))
                                .px(12.0)
                                .font_weight(FontWeight::MEDIUM)
                                .color(theme.semantic.error_100)
                                .line_limit(1),
                        )
                    })
                    .when(!bot.is_moderation_disabled() && !bot.description.is_empty(), |column| {
                        column.child(
                            div().w_full().min_w_0().child(SecondaryText::new(bot.description.clone()).px(12.0).line_limit(1)),
                        )
                    }),
            )
            .child(div().flex_none().w(px(4.0)))
            .children(IconUi::sized("chevron.right", px(12.0), theme.text.secondary.opacity(0.7), false))
            .into_any_element()
    }

    fn row_container(
        bot: BotModel,
        theme: AppTheme,
        nav_store: Option<Entity<NavigationStore>>,
        corners: MyBotsRowCorners,
    ) -> AnyElement {
        let actions = Self::menu_actions(&bot);
        let lifted_background = theme.bg._200;
        let content_bot = bot.clone();
        let action_bot = bot.clone();
        let action_nav_store = nav_store.clone();
        let tap_nav_store = nav_store;
        let bot_id = bot.id;
        HoldContextMenuContainer::new(
            SharedString::from(format!("my-bots-row-{bot_id}")),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, _: &mut App| Self::row_content(&content_bot, &theme, corners)),
        )
        .show_emoji_list(false)
        .lifted_background(Some(lifted_background))
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            Self::handle_action(&id, &action_bot, action_nav_store.as_ref(), cx);
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            Self::push(tap_nav_store.as_ref(), RootRoute::BotSettings { bot_id: bot_id.clone() }, cx);
        }))
        .into_any_element()
    }

    fn row_slot(row: &Entity<MyBotsRowView>, has_third_line: bool, heights: &BotRowHeights) -> AnyElement {
        let variant = usize::from(has_third_line);
        if let Some(height) = heights.get()[variant] {
            return AnyView::from(row.clone()).cached(StyleRefinement::default().w_full().h(height)).into_any_element();
        }
        let heights = heights.clone();
        div()
            .relative()
            .w_full()
            .child(AnyView::from(row.clone()))
            .child(
                canvas(
                    move |bounds, _, _| {
                        if bounds.size.height > px(0.0) {
                            let mut known = heights.get();
                            known[variant] = Some(bounds.size.height);
                            heights.set(known);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element()
    }

    fn list(
        state: &Entity<MyBotsState>,
        bots: Vec<BotModel>,
        theme: AppTheme,
        nav_store: Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) -> AnyElement {
        let top_inset = list_shell_floating_top_inset(cx);
        let (has_more, relative_id, cache_key) = {
            let has_more = BotServicesStore::owned_bots_has_more(cx);
            let relative_id = BotServicesStore::owned_bots_relative_id(cx);
            let viewer = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
            (has_more, relative_id, BotServicesStore::owned_bots_cache_key(&viewer))
        };
        let (rows, row_heights) = state.update(cx, |state, cx| {
            state.sync_list(bots.iter().map(row_key).collect(), has_more);
            (state.row_views(&bots, has_more, nav_store, cx), state.row_heights.clone())
        });
        let count = bots.len();
        let background = theme.bg._100;
        let separator = theme.border._200.opacity(SEPARATOR_OPACITY);
        let list = state.read(cx).list.refreshable(
            "my-bots-list",
            |_, cx| {
                interactions(cx).update(cx, |store, cx| store.refresh_owned_bots(cx));
                Task::ready(())
            },
            move |index, _, _| {
                let Some(bot) = bots.get(index) else {
                    return div()
                        .w_full()
                        .px(px(CARD_HORIZONTAL_PADDING))
                        .pb(px(CARD_BOTTOM_PADDING))
                        .when(has_more, |footer| {
                            footer.child(
                                div()
                                    .w_full()
                                    .py(px(20.0))
                                    .flex()
                                    .justify_center()
                                    .bg(theme.bg._200)
                                    .rounded_b(px(CARD_RADIUS))
                                    .child(progress_view()),
                            )
                        })
                        .into_any_element();
                };
                let is_first = index == 0;
                let is_last_of_card = index + 1 == count && !has_more;
                let row = Self::row_slot(&rows[index], row_key(bot).1, &row_heights).sai_pagination_prefetch(
                    SharedString::from(format!("my-bots-prefetch-{}", bot.id)),
                    SaiPaginationPrefetchOptions::new(index, count, cache_key.clone(), relative_id.clone(), has_more)
                        .threshold(PREFETCH_THRESHOLD),
                    |_, cx| {
                        interactions(cx).update(cx, |store, cx| store.load_more_owned_bots(cx));
                    },
                );
                div()
                    .w_full()
                    .px(px(CARD_HORIZONTAL_PADDING))
                    .when(is_first, |slot| slot.pt(px(top_inset + CARD_TOP_PADDING)))
                    .child(
                        div()
                            .w_full()
                            .overflow_hidden()
                            .bg(theme.bg._200)
                            .when(is_first, |card| card.rounded_t(px(CARD_RADIUS)))
                            .when(is_last_of_card, |card| card.rounded_b(px(CARD_RADIUS)))
                            .child(row)
                            .when(index + 1 < count, |card| {
                                card.child(div().ml(px(SEPARATOR_LEADING)).h(px(0.5)).bg(separator))
                            }),
                    )
                    .into_any_element()
            },
        );
        div().size_full().bg(background).child(list).into_any_element()
    }
}

impl RenderOnce for MyBots {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state("my-bots-state", cx, |_, cx| MyBotsState::new(cx));
        let bots = BotServicesStore::owned_bots(cx);
        let waits_for_network = state.read(cx).waits_for_network;
        let (list_status, list_error) = {
            let actions = BotActionsStore::shared(cx);
            let actions = actions.read(cx);
            (actions.list_bots_sai.status(), actions.list_bots_sai.error())
        };
        let status = if !bots.is_empty() {
            SaiWsSliceStatus::Fulfilled
        } else if waits_for_network {
            SaiWsSliceStatus::Rejected
        } else {
            list_status
        };
        let is_empty = bots.is_empty();

        let back_nav_store = nav_store.clone();
        let create_nav_store = nav_store.clone();
        let empty_nav_store = nav_store.clone();
        let content_state = state.clone();
        let content_theme = theme.clone();
        let loading_theme = theme;
        let retry_state = state.clone();
        let retry_error = list_error.clone();
        let appear_state = state;

        SettingsWrapperUi::new(tr("bot_my_bots_title"), move |_, cx| {
            if let Some(nav_store) = &back_nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .right_jsx(
            div()
                .child(PageHeaderIconButton::new("my-bots-create", "plus", move |_, cx| {
                    Self::open_create_menu(create_nav_store.as_ref(), cx);
                }))
                .guide_anchor(RiseGuideAnchor::MyBotsCreate),
        )
        .content(
            SaiDataRender::new(status, move |_, cx| Self::list(&content_state, bots, content_theme, nav_store, cx))
                .id("my-bots-data")
                .is_empty(is_empty)
                .no_data_text(tr("bot_my_bots_empty"))
                .error_text(list_error.unwrap_or_else(|| tr("bot_load_error")))
                .sai_data_render_loading(move |_, cx| {
                    scroll_view("my-bots-skeleton-scroll")
                        .scroll_disabled(true)
                        .shows_indicators(false)
                        .size_full()
                        .bg(loading_theme.bg._100)
                        .list_shell_scroll_under_chrome(cx)
                        .child(
                            div()
                                .w_full()
                                .px(px(CARD_HORIZONTAL_PADDING))
                                .pt(px(CARD_TOP_PADDING))
                                .pb(px(CARD_BOTTOM_PADDING))
                                .child(
                                    Shape::continuous(CARD_RADIUS)
                                        .round(div())
                                        .w_full()
                                        .overflow_hidden()
                                        .bg(loading_theme.bg._200)
                                        .child(UserListSkeleton::new().rows(12)),
                                ),
                        )
                })
                .sai_data_render_error(move |_, _| {
                    let retry_state = retry_state.clone();
                    BotStateView::new(
                        "my-bots-error",
                        "exclamationmark.arrow.triangle.2.circlepath",
                        tr("bot_load_error"),
                    )
                    .message(retry_error.clone())
                    .action_title(tr("bot_retry"))
                    .action(move |_, cx| {
                        retry_state.update(cx, |state, cx| state.sync_waits_for_network(cx));
                        interactions(cx).update(cx, |store, cx| store.load_owned_bots(cx));
                    })
                })
                .sai_data_render_empty(move |_, _| {
                    let empty_nav_store = empty_nav_store.clone();
                    BotStateView::new("my-bots-empty", "command.circle.fill", tr("bot_my_bots_empty"))
                        .message(tr("bot_my_bots_empty_message"))
                        .action_title(tr("bot_create"))
                        .action(move |_, cx| Self::open_create_menu(empty_nav_store.as_ref(), cx))
                }),
        )
        .on_appear("my-bots-appear", move |_, cx| {
            interactions(cx).update(cx, |store, cx| store.load_owned_bots(cx));
            appear_state.update(cx, |state, cx| state.sync_waits_for_network(cx));
        })
    }
}
