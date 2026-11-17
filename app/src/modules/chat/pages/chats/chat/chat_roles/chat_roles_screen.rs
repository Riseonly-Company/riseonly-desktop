use std::rc::Rc;

use gpui::{App, Context, Entity, Pixels, Point, SharedString, Subscription, Window};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::prelude::*;
use rise_ui::text_field_alert::TextFieldAlert;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_shared::MemberTagChip;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatPerm, ChatPermExt, ChatRole};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;

const EVERYONE_ROLE_TITLE: &str = "@everyone";

const ACTION_MOVE_UP: &str = "move_up";
const ACTION_MOVE_DOWN: &str = "move_down";
const ACTION_DELETE: &str = "delete";

struct ChatRolesScreenState {
    show_create: bool,
    new_name: String,
    _subscriptions: [Subscription; 2],
}

impl ChatRolesScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        Self {
            show_create: false,
            new_name: String::new(),
            _subscriptions: [
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&sandbox, |_, _, cx| cx.notify()),
            ],
        }
    }
}

/// what the actions of the screen need, rows and menus hold their own copy
#[derive(Clone)]
struct ChatRolesScreenParts {
    resolved_chat_id: String,
    state: Entity<ChatRolesScreenState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl ChatRolesScreenParts {
    fn push_role_edit(&self, role_id: String, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            let route = RootRoute::ChatRoleEdit { chat_id: Some(self.resolved_chat_id.clone()), role_id: Some(role_id) };
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn pop(&self, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }

    fn submit_create(&self, cx: &mut App) {
        let typed = self.state.read(cx).new_name.clone();
        let name = trimming_whitespaces_and_newlines(&typed).to_owned();
        self.state.update(cx, |state, cx| {
            state.new_name = String::new();
            cx.notify();
        });
        if name.is_empty() {
            return;
        }
        let parts = self.clone();
        let created_name = name.clone();
        let on_result: Rc<dyn Fn(bool, &mut App)> = Rc::new(move |ok: bool, cx: &mut App| {
            if !ok {
                return;
            }
            let created = ChatServicesStore::shared(cx)
                .update(cx, |services, cx| services.custom_roles(&parts.resolved_chat_id, cx))
                .into_iter()
                .find(|role| role.name == created_name);
            if let Some(created) = created {
                parts.push_role_edit(created.id, cx);
            }
        });
        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.create_role(
                &self.resolved_chat_id,
                &name,
                0,
                ChatPerm::NEW_ROLE_DEFAULT,
                false,
                false,
                Some(on_result),
                cx,
            );
        });
    }

    fn handle_role_action(&self, id: &str, role: &ChatRole, cx: &mut App) {
        match id {
            ACTION_MOVE_UP => self.move_role(role, true, cx),
            ACTION_MOVE_DOWN => self.move_role(role, false, cx),
            ACTION_DELETE => self.confirm_delete(role.clone(), cx),
            _ => {}
        }
    }

    fn move_role(&self, role: &ChatRole, up: bool, cx: &mut App) {
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.move_role_action(&self.resolved_chat_id, &role.id, up, cx);
        });
    }

    fn confirm_delete(&self, role: ChatRole, cx: &mut App) {
        let chat_id = self.resolved_chat_id.clone();
        let message = role.name.clone();
        NativeConfirmAlert::present_delete(
            &tr("roles_delete_confirm"),
            &message,
            Some(tr("roles_delete")),
            None,
            false,
            None,
            None,
            move |cx| {
                ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.delete_role(&chat_id, &role.id, cx);
                });
            },
            cx,
        );
    }
}

#[derive(IntoElement)]
pub struct ChatRolesScreen {
    chat_id: Option<String>,
}

impl ChatRolesScreen {
    /// role name up to 128 chars
    fn name_rule() -> RiseFieldRule {
        RiseFieldRule::new("chat_create_role", "/name")
    }

    pub fn new(chat_id: Option<String>) -> Self {
        Self { chat_id }
    }

    fn role_actions(index: usize, roles: &[ChatRole], outranks: &[bool]) -> Vec<HoldContextMenuAction> {
        let mut items = Vec::new();
        if index > 0 && outranks[index - 1] {
            items.push(HoldContextMenuAction::new(ACTION_MOVE_UP, tr("roles_move_up")).system_image("arrow.up"));
        }
        if index + 1 < roles.len() {
            items.push(
                HoldContextMenuAction::new(ACTION_MOVE_DOWN, tr("roles_move_down"))
                    .system_image("arrow.down")
                    .guide_anchor(RiseGuideAnchor::RolesMenuMoveDown),
            );
        }
        items.push(HoldContextMenuAction::new(ACTION_DELETE, tr("roles_delete")).system_image("trash").destructive(true));
        items
    }

    fn role_row(role: &ChatRole, can_manage: bool, outranks: bool, theme: &AppTheme) -> Div {
        let swift_color = role.swift_color();
        let name_color = swift_color.map(|color| theme.legible(color));
        let muted = theme.text.secondary.opacity(0.6);
        hstack(12.0)
            .w_full()
            .px(px(16.0))
            .py(px(12.0))
            .child(
                div()
                    .flex_none()
                    .w(px(14.0))
                    .h(px(14.0))
                    .circle()
                    .bg(swift_color.unwrap_or(theme.text.secondary.opacity(0.4))),
            )
            .child(MainText::new(role.name.clone()).px(15.0).color(name_color).number_of_lines(1))
            .child(spacer_min_w(4.0))
            .when(ChatPerm::has(role.permissions, ChatPerm::ADMINISTRATOR), |row| {
                row.child(MemberTagChip::new(tr("perm_administrator")))
            })
            .when(role.hoist, |row| row.children(IconUi::sized("person.2.fill", px(11.0), muted, true)))
            .when(can_manage && !outranks, |row| row.children(IconUi::sized("lock.fill", px(11.0), muted, true)))
    }

    fn role_list(
        roles: &[ChatRole],
        outranks: &[bool],
        can_manage: bool,
        parts: &ChatRolesScreenParts,
        theme: &AppTheme,
    ) -> Div {
        let mut list = vstack(0.0).w_full().items_stretch().bg(theme.bg._200).rounded(px(20.0));
        for (index, role) in roles.iter().enumerate() {
            let actions = if can_manage && outranks[index] {
                Self::role_actions(index, roles, outranks)
            } else {
                Vec::new()
            };
            let row_role = role.clone();
            let row_outranks = outranks[index];
            let row_theme = theme.clone();
            let (action_parts, action_role) = (parts.clone(), role.clone());
            let (tap_parts, tap_role_id) = (parts.clone(), role.id.clone());
            let on_action: Rc<dyn Fn(String, &mut Window, &mut App)> =
                Rc::new(move |id: String, _: &mut Window, cx: &mut App| action_parts.handle_role_action(&id, &action_role, cx));
            let on_tap: Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)> =
                Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| tap_parts.push_role_edit(tap_role_id.clone(), cx));
            let hold = HoldContextMenuContainer::new(
                SharedString::from(format!("chat-role-hold-{}", role.id)),
                HoldContextMenuAlignment::Leading,
                Rc::new(move |_: &App| actions.clone()),
                Rc::new(move |_: &mut Window, _: &mut App| {
                    let row = ChatRolesScreen::role_row(&row_role, can_manage, row_outranks, &row_theme);
                    if index == 0 {
                        row.guide_anchor(RiseGuideAnchor::ChatRolesFirstRole).into_any_element()
                    } else {
                        row.into_any_element()
                    }
                }),
            )
            .show_emoji_list(false)
            .lifted_background(Some(theme.bg._200))
            .on_action(on_action)
            .on_tap(on_tap);
            list = list.child(hold);
            if index + 1 < roles.len() {
                list = list.child(div().w_full().pl(px(52.0)).child(
                    div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55)),
                ));
            }
        }
        div().w_full().px(px(12.0)).child(list)
    }

    fn empty_state(theme: &AppTheme) -> Div {
        vstack(12.0)
            .pt(px(30.0))
            .children(IconUi::sized("person.3.sequence.fill", px(38.0), theme.text.secondary.opacity(0.5), true))
            .child(
                div().max_w(px(280.0)).child(
                    MainText::new(tr("roles_empty")).px(14.0).tac(TextAlignT::Center).color(theme.text.secondary),
                ),
            )
    }
}

impl RenderOnce for ChatRolesScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let resolved_chat_id = self.chat_id.unwrap_or_default();
        let screen_id = format!("chat-roles-{resolved_chat_id}");
        let state = window.use_keyed_state(SharedString::from(format!("{screen_id}-state")), cx, |_, cx| {
            ChatRolesScreenState::new(cx)
        });
        let parts = ChatRolesScreenParts {
            resolved_chat_id: resolved_chat_id.clone(),
            state: state.clone(),
            nav_store: env::<NavigationStoreKey>(cx),
        };

        let (roles, can_manage, outranks, everyone) = ChatServicesStore::shared(cx).update(cx, |services, cx| {
            let roles = services.custom_roles(&resolved_chat_id, cx);
            let everyone = services.everyone_role(&resolved_chat_id, cx);
            let can_manage = services.can(&resolved_chat_id, ChatPerm::MANAGE_ROLES, cx);
            let outranks: Vec<bool> =
                roles.iter().map(|role| services.viewer_outranks(&resolved_chat_id, role, cx)).collect();
            (roles, can_manage, outranks, everyone)
        });

        let interactions = ChatInteractionsStore::shared(cx);
        let task_chat_id = resolved_chat_id.clone();
        use_task(SharedString::from(format!("{screen_id}-task")), (), window, cx, async move |cx| {
            interactions.update(cx, |interactions, cx| interactions.load_roles(&task_chat_id, cx));
        });

        let mut content = vstack(16.0).w_full().pt(px(12.0));
        if let Some(everyone) = everyone {
            let opening = parts.clone();
            content = content.child(GroupedBtns::new(
                SharedString::from(format!("{screen_id}-everyone")),
                vec![
                    GroupedBtn::new("everyone", EVERYONE_ROLE_TITLE)
                        .icon("person.2.fill")
                        .icon_background(ChatTileColor::gray())
                        .subtitle(tr("roles_everyone_subtitle"))
                        .callback(move |_, cx| opening.push_role_edit(everyone.id.clone(), cx)),
                ],
            ));
        }
        if roles.is_empty() {
            content = content.child(ChatRolesScreen::empty_state(&theme));
        } else {
            content = content.child(
                vstack(8.0)
                    .w_full()
                    .items_start()
                    .child(ChatRolesScreen::role_list(&roles, &outranks, can_manage, &parts, &theme))
                    .when(can_manage, |section| {
                        section.child(div().w_full().px(px(28.0)).child(
                            MainText::new(tr("roles_hierarchy_footer")).px(13.0).color(theme.text.secondary),
                        ))
                    }),
            );
        }
        if can_manage {
            let creating = state.clone();
            content = content.child(GroupedBtns::new(
                SharedString::from(format!("{screen_id}-create")),
                vec![
                    GroupedBtn::new("create", tr("roles_create"))
                        .icon("plus")
                        .icon_background(ChatTileColor::blue())
                        .guide_anchor(RiseGuideAnchor::ChatRolesCreate)
                        .show_chevron(false)
                        .callback(move |_, cx| {
                            creating.update(cx, |state, cx| {
                                state.new_name = String::new();
                                state.show_create = true;
                                cx.notify();
                            });
                        }),
                ],
            ));
        }
        content = content.child(spacer_min_h(60.0));

        let scroll = scroll_view(SharedString::from(format!("{screen_id}-scroll")))
            .list_shell_scroll_under_chrome(cx)
            .child(content);

        let is_presented = Binding::entity(
            state.clone(),
            |state: &ChatRolesScreenState| state.show_create,
            |state, value, _| state.show_create = value,
        );
        let new_name = ChatRolesScreen::name_rule().binding(Binding::entity(
            state,
            |state: &ChatRolesScreenState| state.new_name.clone(),
            |state, value, _| state.new_name = value,
        ));
        let create_parts = parts.clone();
        let alert = TextFieldAlert::new(
            SharedString::from(format!("{screen_id}-create-alert")),
            is_presented,
            tr("roles_create_title"),
            tr("roles_create_placeholder"),
            new_name,
            tr("common_cancel"),
            tr("roles_create"),
            move |_, cx| create_parts.submit_create(cx),
        );

        SettingsWrapperUi::new(tr("roles_title"), move |_, cx| parts.pop(cx)).content(
            div().size_full().bg(theme.bg._100).child(scroll).child(alert),
        )
    }
}
