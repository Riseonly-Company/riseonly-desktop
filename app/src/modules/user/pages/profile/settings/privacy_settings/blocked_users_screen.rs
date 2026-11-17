use std::rc::Rc;

use gpui::{App, Context, Empty, Entity, FontWeight, ListAlignment, Pixels, Point, SharedString, Subscription, Task, Window};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRenderDefaultsEmpty;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, list_shell_floating_top_inset,
};
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;
use crate::modules::user::components::user_relationship_list_contract::UserRelationshipRowMetrics;
use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;
use crate::modules::user::engine::rise_user_block_presentation::{
    RiseBlockedUser, RiseUserBlockLoadState, RiseUserBlockPresentation, RiseUserBlockSnapshot,
};
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const UNBLOCK_ACTION_ID: &str = "unblock";
const LOAD_MORE_DISTANCE: usize = 5;
const LIST_TOP_PADDING: f32 = 8.0;
const SUBTITLE_SEPARATOR: &str = " \u{b7} ";

enum BlockedUsersRow {
    Entry { entry: RiseBlockedUser, is_pending: bool, loads_more: bool },
    LoadingMore,
}

impl BlockedUsersRow {
    fn key(&self) -> String {
        match self {
            Self::Entry { entry, .. } => format!("entry-{}", entry.id()),
            Self::LoadingMore => "loading-more".to_owned(),
        }
    }
}

struct BlockedUsersScreenState {
    list: ListUi,
    row_keys: Vec<String>,
    _observations: [Subscription; 1],
}

impl BlockedUsersScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseUserBlockPresentation::shared(cx);
        Self {
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            row_keys: Vec::new(),
            _observations: [cx.observe(&presentation, |_, _, cx| cx.notify())],
        }
    }

    fn sync_list(&mut self, rows: &[BlockedUsersRow]) {
        let keys: Vec<String> = rows.iter().map(BlockedUsersRow::key).collect();
        if keys == self.row_keys {
            return;
        }
        let start = keys.iter().zip(&self.row_keys).take_while(|(new, old)| new == old).count();
        let tail_limit = keys.len().min(self.row_keys.len()) - start;
        let end = keys
            .iter()
            .rev()
            .zip(self.row_keys.iter().rev())
            .take(tail_limit)
            .take_while(|(new, old)| new == old)
            .count();
        self.list.state().splice(start..self.row_keys.len() - end, keys.len() - start - end);
        self.row_keys = keys;
    }
}

/// what the screen actions need, rows and menus hold their own copy
#[derive(Clone)]
struct BlockedUsersScreenParts {
    state: Entity<BlockedUsersScreenState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl BlockedUsersScreenParts {
    fn load(force: bool, cx: &mut App) {
        UserInteractionsStore::shared(cx).update(cx, |store, cx| store.load_blocked_users(force, cx));
    }

    fn unblock(entry: &RiseBlockedUser, cx: &mut App) {
        let user_id = entry.user_id.clone();
        UserInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.set_user_block_scope(&user_id, None, |_, _| {}, cx);
        });
    }

    fn open_profile(&self, entry: &RiseBlockedUser, cx: &mut App) {
        if entry.is_deleted {
            return;
        }
        if let Some(nav_store) = &self.nav_store {
            let route = RootRoute::UserPage { user_id: Some(entry.user_id.clone()), tag: Some(entry.tag.clone()) };
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn rows(snapshot: &RiseUserBlockSnapshot) -> Vec<BlockedUsersRow> {
        let count = snapshot.entries.len();
        let mut rows: Vec<BlockedUsersRow> = snapshot
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| BlockedUsersRow::Entry {
                entry: entry.clone(),
                is_pending: snapshot.pending.contains(&entry.user_id),
                loads_more: index + LOAD_MORE_DISTANCE >= count,
            })
            .collect();
        if snapshot.is_loading_more {
            rows.push(BlockedUsersRow::LoadingMore);
        }
        rows
    }

    fn row_labels(entry: &RiseBlockedUser, name: &str, subtitle: &str, theme: &AppTheme) -> AnyElement {
        hstack(UserRelationshipRowMetrics::CONTENT_SPACING)
            .w_full()
            .min_h(px(UserRelationshipRowMetrics::AVATAR_SIZE))
            .child(
                UserLogoUi::new(SharedString::from(format!("blocked-user-logo-{}", entry.user_id)))
                    .source(SharedString::from(entry.logo.clone()))
                    .name(SharedString::from(name.to_owned()))
                    .user_id(SharedString::from(entry.user_id.clone()))
                    .size(UserRelationshipRowMetrics::AVATAR_SIZE),
            )
            .child(
                vstack(4.0)
                    .flex_1()
                    .min_w_0()
                    .align(HorizontalAlignment::Leading)
                    .child(
                        hstack(4.0)
                            .child(
                                MainText::new(name.to_owned())
                                    .px(16.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .number_of_lines(1),
                            )
                            .when(entry.is_premium, |stack| {
                                stack.child(
                                    PremiumBadge::new().user_id(Some(SharedString::from(entry.user_id.clone()))).size(14.0),
                                )
                            }),
                    )
                    .when(!subtitle.is_empty(), |stack| {
                        stack.child(
                            MainText::new(subtitle.to_owned()).px(13.0).color(theme.text.secondary).number_of_lines(1),
                        )
                    }),
            )
            .into_any_element()
    }

    fn row(&self, entry: &RiseBlockedUser, is_pending: bool, theme: &AppTheme) -> impl IntoElement {
        let name = if entry.is_deleted || entry.name.is_empty() { tr("chat_deleted_account") } else { entry.name.clone() };
        let tag = if entry.tag.is_empty() { String::new() } else { format!("@{}", entry.tag) };
        let subtitle = if entry.scope == RiseBlockScope::Total {
            [tag.clone(), tr("contact_block_scope_total")]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(SUBTITLE_SEPARATOR)
        } else {
            tag
        };

        let actions = vec![
            HoldContextMenuAction::new(UNBLOCK_ACTION_ID, tr("contact_unblock_action"))
                .system_image("person.crop.circle.badge.checkmark"),
        ];
        let content_entry = entry.clone();
        let content_theme = theme.clone();
        let action_entry = entry.clone();
        let tap_parts = self.clone();
        let tap_entry = entry.clone();
        let hold = HoldContextMenuContainer::new(
            SharedString::from(format!("blocked-user-hold-{}", entry.user_id)),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, _: &mut App| {
                Self::row_labels(&content_entry, &name, &subtitle, &content_theme)
            }),
        )
        .show_emoji_list(false)
        .lifted_background(Some(theme.bg._100))
        .on_action(Rc::new(move |action: String, _: &mut Window, cx: &mut App| {
            if action == UNBLOCK_ACTION_ID {
                Self::unblock(&action_entry, cx);
            }
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            tap_parts.open_profile(&tap_entry, cx);
        }));

        let title = tr("contact_unblock_action");
        let button_entry = entry.clone();
        hstack(UserRelationshipRowMetrics::CONTENT_SPACING)
            .w_full()
            .px(px(UserRelationshipRowMetrics::HORIZONTAL_PADDING))
            .py(px(UserRelationshipRowMetrics::VERTICAL_PADDING))
            .bg(theme.bg._100)
            .child(div().flex_1().min_w_0().child(hold))
            .child(
                div().flex_none().child(
                    LiquidGlassButton::new(
                        SharedString::from(format!("blocked-user-unblock-{}", entry.user_id)),
                        LiquidGlassButtonShape::Capsule,
                    )
                    .horizontal_padding(10.0)
                    .vertical_padding(0.0)
                    .tint(Some(theme.primary._100.opacity(0.12)))
                    .disabled(is_pending)
                    .accessibility_label(Some(title.clone()))
                    .on_press(move |_, cx| Self::unblock(&button_entry, cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .min_h(px(UserRelationshipRowMetrics::relationship_button_minimum_height()))
                            .child(
                                MainText::new(title)
                                    .px(13.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.primary._100)
                                    .number_of_lines(1)
                                    .minimum_scale_factor(0.85),
                            ),
                    ),
                ),
            )
    }

    fn failure(theme: &AppTheme) -> impl IntoElement {
        vstack(12.0)
            .size_full()
            .justify_center()
            .px(px(24.0))
            .child(
                MainText::new(tr("error_fetching_data")).px(16.0).tac(TextAlignT::Center).color(theme.text.primary),
            )
            .child(plain("blocked-users-retry").on_press(|_, cx| Self::load(true, cx)).child(
                MainText::new(tr("common_retry")).px(15.0).font_weight(FontWeight::SEMIBOLD).color(theme.primary._100),
            ))
    }
}

#[derive(IntoElement)]
struct BlockedUsersScreenBody {
    parts: BlockedUsersScreenParts,
}

impl RenderOnce for BlockedUsersScreenBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let snapshot = UserServicesStore::shared(cx).update(cx, |service, cx| service.blocked_users(cx));
        let parts = self.parts;
        let body = div().size_full().bg(theme.bg._100);
        if !snapshot.entries.is_empty() {
            let rows = Rc::new(BlockedUsersScreenParts::rows(&snapshot));
            parts.state.update(cx, |state, _| state.sync_list(&rows));
            let top_inset = list_shell_floating_top_inset(cx);
            let screen_state = parts.state.clone();
            let list = screen_state.read(cx).list.refreshable(
                "blocked-users-list",
                |_, cx| {
                    BlockedUsersScreenParts::load(true, cx);
                    Task::ready(())
                },
                move |index, _, cx| {
                    let theme = rise_ui::theme(cx).clone();
                    let Some(row) = rows.get(index) else {
                        return Empty.into_any_element();
                    };
                    let top_padding = if index == 0 { top_inset + LIST_TOP_PADDING } else { 0.0 };
                    let content = match row {
                        BlockedUsersRow::Entry { entry, is_pending, loads_more } => {
                            let element = parts.row(entry, *is_pending, &theme);
                            if *loads_more {
                                element
                                    .on_appear(
                                        SharedString::from(format!("blocked-user-appear-{}", entry.user_id)),
                                        |_, cx| {
                                            UserInteractionsStore::shared(cx)
                                                .update(cx, |store, cx| store.load_more_blocked_users(cx));
                                        },
                                    )
                                    .into_any_element()
                            } else {
                                element.into_any_element()
                            }
                        }
                        BlockedUsersRow::LoadingMore => {
                            div().w_full().flex().justify_center().py(px(10.0)).child(progress_view()).into_any_element()
                        }
                    };
                    div().w_full().pt(px(top_padding)).child(content).into_any_element()
                },
            );
            return body.child(list).into_any_element();
        }
        match snapshot.load_state {
            RiseUserBlockLoadState::Loaded => body
                .child(SaiDataRenderDefaultsEmpty {
                    text: tr("privacy_blocked_users_empty"),
                    no_data_height_percent: 15.0,
                    geometry_key: "blocked-users-empty".into(),
                })
                .into_any_element(),
            RiseUserBlockLoadState::Failed => body.child(BlockedUsersScreenParts::failure(&theme)).into_any_element(),
            RiseUserBlockLoadState::Idle | RiseUserBlockLoadState::Loading => body
                .child(
                    scroll_view("blocked-users-skeleton")
                        .size_full()
                        .scroll_disabled(true)
                        .shows_indicators(false)
                        .list_shell_scroll_under_chrome(cx)
                        .child(UserListSkeleton::new()),
                )
                .into_any_element(),
        }
    }
}

/// people this account blocked and newest first,
/// with unblock. the list is owned by RiseUserBlockRepository and unblock drops the row
/// optimistically
#[derive(IntoElement, Default)]
pub struct BlockedUsersScreen;

impl BlockedUsersScreen {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for BlockedUsersScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("blocked-users-state", cx, |_, cx| BlockedUsersScreenState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let parts = BlockedUsersScreenParts { state, nav_store: nav_store.clone() };

        use_task("blocked-users-load", (), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| BlockedUsersScreenParts::load(false, cx));
        });

        let back_store = nav_store.clone();
        SettingsWrapperUi::new(tr("privacy_blocked_users_title"), move |_, cx| {
            if let Some(nav_store) = &back_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(PageRefreshKeys::new(
            "blocked-users-keys",
            |_, cx| BlockedUsersScreenParts::load(true, cx),
            BlockedUsersScreenBody { parts },
        ))
        .on_appear("blocked-users-lifecycle", |_, _| {})
        .and_on_disappear(move |_, cx| {
            let still_stacked = nav_store
                .as_ref()
                .is_some_and(|nav_store| nav_store.read(cx).path.iter().any(|route| matches!(route, RootRoute::BlockedUsers)));
            if !still_stacked {
                UserInteractionsStore::shared(cx).update(cx, |store, cx| store.trim_blocked_users(cx));
            }
        })
    }
}
