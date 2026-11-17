use gpui::{App, Context, ElementId, Entity, FontWeight, Hsla, Rgba, SharedString, Subscription, Window};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::{AppTheme, Appearance, RiseThemeAppearance, ThemeStore};
use rise_theme::ui_color_system::UIColor;
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::environment_values::IsEnabledKey;
use rise_ui::swiftui::prelude::*;
use rise_ui::switch_ui::SwitchRowUi;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatPerm, ChatRole, rgb_int};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::guides::stores::guides_services::guides_services::GuidesServicesStore;
use crate::modules::media_picker::editor::editor_color_picker_bridge::EditorColorPickerBridge;
use crate::modules::user::pages::profile::settings::customization_settings::settings::color_picker_well::ColorPickerWell;

const EVERYONE_ROLE_TITLE: &str = "@everyone";

fn system_blue(cx: &mut App) -> Hsla {
    let appearance = match ThemeStore::shared(cx).read(cx).resolved_appearance() {
        RiseThemeAppearance::Light => Appearance::Light,
        RiseThemeAppearance::Dark => Appearance::Dark,
    };
    UIColor::system_blue().resolved_color(appearance)
}

struct ChatRoleEditScreenState {
    name: String,
    color: Hsla,
    has_color: bool,
    color_picked: bool,
    mentionable: bool,
    hoist: bool,
    perms: i64,
    seeded: bool,
    saving: bool,
    on_screen: bool,
    _subscriptions: [Subscription; 2],
}

impl ChatRoleEditScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        Self {
            name: String::new(),
            color: system_blue(cx),
            has_color: false,
            color_picked: false,
            mentionable: false,
            hoist: false,
            perms: 0,
            seeded: false,
            saving: false,
            on_screen: false,
            _subscriptions: [
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&sandbox, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn seed_from_role(&mut self, role: &ChatRole) {
        if self.seeded {
            return;
        }
        self.seeded = true;
        self.name = role.name.clone();
        self.perms = role.permissions;
        self.mentionable = role.mentionable;
        self.hoist = role.hoist;
        self.has_color = role.color != 0;
        self.color_picked = self.has_color;
        if let Some(color) = role.swift_color() {
            self.color = color;
        }
    }

    /// only a real change counts as picking
    fn set_color(&mut self, color: Hsla) {
        if self.color != color {
            self.color = color;
            if self.has_color {
                self.color_picked = true;
            }
        }
    }

    /// reacts to the has color flag changing
    fn set_has_color(&mut self, has_color: bool) {
        self.has_color = has_color;
        if !has_color {
            self.color_picked = false;
        }
    }
}

/// what the actions of the screen need
#[derive(Clone)]
struct ChatRoleEditScreenParts {
    resolved_chat_id: String,
    role_id: String,
    state: Entity<ChatRoleEditScreenState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl ChatRoleEditScreenParts {
    fn pop(&self, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }

    fn role(&self, cx: &mut App) -> Option<ChatRole> {
        ChatServicesStore::shared(cx).update(cx, |services, cx| services.role(&self.resolved_chat_id, &self.role_id, cx))
    }

    /// the default role holds only the rights every member has, no name or color or flags to edit
    fn is_everyone(role: Option<&ChatRole>) -> bool {
        role.is_some_and(|role| role.is_default)
    }

    /// same gate as the roles list
    /// rights not loaded yet leave the editor open, the server still refuses
    /// an owner shouldnt see a locked form
    fn can_manage(&self, cx: &mut App) -> bool {
        ChatServicesStore::shared(cx).update(cx, |services, cx| {
            if services.effective_permissions(&self.resolved_chat_id, None, cx).is_none() {
                return true;
            }
            if !services.can(&self.resolved_chat_id, ChatPerm::MANAGE_ROLES, cx) {
                return false;
            }
            match services.role(&self.resolved_chat_id, &self.role_id, cx) {
                Some(role) => services.viewer_outranks(&self.resolved_chat_id, &role, cx),
                None => true,
            }
        })
    }

    /// closes only once the server took the change
    /// closing at once looked like success even when the request failed
    fn save(&self, cx: &mut App) {
        if !self.can_manage(cx) || self.state.read(cx).saving {
            return;
        }
        let role = self.role(cx);
        let (name, color_int, perms, mentionable, hoist) = {
            let state = self.state.read(cx);
            let color_int = if ChatRoleEditScreenParts::is_everyone(role.as_ref()) {
                role.as_ref().map_or(0, |role| role.color)
            } else if state.has_color {
                ChatRoleEditScreen::color_to_int(state.color)
            } else {
                0
            };
            (
                trimming_whitespaces_and_newlines(&state.name).to_owned(),
                color_int,
                state.perms,
                state.mentionable,
                state.hoist,
            )
        };
        self.state.update(cx, |state, cx| {
            state.saving = true;
            cx.notify();
        });
        let parts = self.clone();
        let on_result: std::rc::Rc<dyn Fn(bool, &mut App)> = std::rc::Rc::new(move |ok: bool, cx: &mut App| {
            parts.state.update(cx, |state, cx| {
                state.saving = false;
                cx.notify();
            });
            if ok && parts.state.read(cx).on_screen {
                parts.pop(cx);
            }
        });
        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.update_role(
                &self.resolved_chat_id,
                &self.role_id,
                &name,
                color_int,
                perms,
                mentionable,
                hoist,
                Some(on_result),
                cx,
            );
        });
    }

    fn bool_binding(
        &self,
        get: impl Fn(&ChatRoleEditScreenState) -> bool + 'static,
        set: impl Fn(&mut ChatRoleEditScreenState, bool) + 'static,
    ) -> Binding<bool> {
        Binding::entity(self.state.clone(), get, move |state, value, _| set(state, value))
    }
}

#[derive(IntoElement)]
pub struct ChatRoleEditScreen {
    chat_id: Option<String>,
    role_id: Option<String>,
}

impl ChatRoleEditScreen {
    /// role name up to 128 chars
    fn name_rule() -> RiseFieldRule {
        RiseFieldRule::new("chat_create_role", "/name")
    }

    pub fn new(chat_id: Option<String>, role_id: Option<String>) -> Self {
        Self { chat_id, role_id }
    }

    fn color_to_int(color: Hsla) -> i64 {
        let rgba = Rgba::from(color);
        rgb_int(f64::from(rgba.r), f64::from(rgba.g), f64::from(rgba.b))
    }

    fn save_button(id: ElementId, saving: bool, parts: ChatRoleEditScreenParts, theme: &AppTheme) -> impl IntoElement {
        let tint = theme.primary._100;
        LiquidGlassButton::new(id, LiquidGlassButtonShape::Capsule)
            .horizontal_padding(16.0)
            .vertical_padding(10.0)
            .disabled(saving)
            .on_press(move |_: &mut Window, cx: &mut App| parts.save(cx))
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div().when(saving, |label| label.opacity(0.0)).child(
                            MainText::new(tr("common_save"))
                                .px(15.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(tint),
                        ),
                    )
                    .when(saving, |label| {
                        label.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(progress_view().control_size(ControlSize::Small).tint(tint)),
                        )
                    }),
            )
            .guide_anchor(RiseGuideAnchor::RoleEditSave)
    }

    fn name_card(id: ElementId, parts: &ChatRoleEditScreenParts, theme: &AppTheme) -> Div {
        let binding = ChatRoleEditScreen::name_rule().binding(Binding::entity(
            parts.state.clone(),
            |state: &ChatRoleEditScreenState| state.name.clone(),
            |state, value, _| state.name = value,
        ));
        vstack(8.0)
            .w_full()
            .items_start()
            .px(px(16.0))
            .child(MainText::new(tr("roles_name")).px(13.0).color(theme.text.secondary))
            .child(
                div()
                    .w_full()
                    .px(px(14.0))
                    .py(px(12.0))
                    .bg(theme.bg._200)
                    .rounded(px(14.0))
                    .child(ShapedTextField::new(id, tr("roles_create_placeholder"), binding)),
            )
    }

    fn color_card(id: &str, has_color: bool, parts: &ChatRoleEditScreenParts, theme: &AppTheme) -> Div {
        let mut row = hstack(DEFAULT_STACK_SPACING)
            .w_full()
            .px(px(16.0))
            .py(px(14.0))
            .bg(theme.bg._200)
            .rounded(px(16.0))
            .child(MainText::new(tr("roles_color")).px(15.0))
            .child(spacer());
        if has_color {
            let selection = Binding::entity(
                parts.state.clone(),
                |state: &ChatRoleEditScreenState| state.color,
                |state, value, _| state.set_color(value),
            );
            let clearing = parts.state.clone();
            row = row
                .child(ColorPickerWell::new(SharedString::from(format!("{id}-color-well")), selection))
                .child(
                    plain(SharedString::from(format!("{id}-color-clear")))
                        .on_press(move |_, cx| {
                            clearing.update(cx, |state, cx| {
                                state.set_has_color(false);
                                cx.notify();
                            });
                        })
                        .children(IconUi::sized("xmark.circle.fill", px(17.0), theme.text.secondary.opacity(0.6), true)),
                );
        } else {
            let setting = parts.state.clone();
            row = row.child(
                plain(SharedString::from(format!("{id}-color-set")))
                    .on_press(move |_, cx| {
                        setting.update(cx, |state, cx| {
                            state.set_has_color(true);
                            cx.notify();
                        });
                    })
                    .child(MainText::new(tr("roles_color_set")).px(14.0).color(theme.primary._100)),
            );
        }
        div().w_full().px(px(12.0)).child(row.guide_anchor(RiseGuideAnchor::RoleEditColor))
    }

    fn row_separator(theme: &AppTheme) -> Div {
        div().w_full().pl(px(16.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.5)))
    }

    fn switch_row(id: SharedString, title: String, binding: Binding<bool>) -> Div {
        div().w_full().py(px(2.0)).child(SwitchRowUi::new(id, title, binding))
    }

    fn flags_card(id: &str, parts: &ChatRoleEditScreenParts, theme: &AppTheme) -> Div {
        let card = vstack(0.0)
            .w_full()
            .bg(theme.bg._200)
            .rounded(px(16.0))
            .child(Self::switch_row(
                SharedString::from(format!("{id}-mentionable")),
                tr("roles_mentionable"),
                parts.bool_binding(|state| state.mentionable, |state, value| state.mentionable = value),
            ))
            .child(Self::row_separator(theme))
            .child(Self::switch_row(
                SharedString::from(format!("{id}-hoist")),
                tr("roles_hoist"),
                parts.bool_binding(|state| state.hoist, |state, value| state.hoist = value),
            ));
        div().w_full().px(px(12.0)).child(card.guide_anchor(RiseGuideAnchor::RoleEditFlags))
    }

    fn permissions_card(id: &str, parts: &ChatRoleEditScreenParts, theme: &AppTheme) -> Div {
        let rows = ChatPerm::editor_rows();
        let mut card = vstack(0.0).w_full().bg(theme.bg._200).rounded(px(16.0));
        for (index, row) in rows.iter().enumerate() {
            let bit = row.mask;
            card = card.child(Self::switch_row(
                SharedString::from(format!("{id}-perm-{bit}")),
                tr(row.i18n_key),
                parts.bool_binding(
                    move |state| (state.perms & bit) != 0,
                    move |state, value| state.perms = if value { state.perms | bit } else { state.perms & !bit },
                ),
            ));
            if index + 1 < rows.len() {
                card = card.child(Self::row_separator(theme));
            }
        }
        div().w_full().px(px(12.0)).child(card)
    }
}

impl RenderOnce for ChatRoleEditScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let resolved_chat_id = self.chat_id.unwrap_or_default();
        let role_id = self.role_id.unwrap_or_default();
        let screen_id = format!("chat-role-edit-{resolved_chat_id}-{role_id}");
        let child_id = |name: &str| SharedString::from(format!("{screen_id}-{name}"));

        let state = window.use_keyed_state(child_id("state"), cx, |_, cx| ChatRoleEditScreenState::new(cx));
        let parts = ChatRoleEditScreenParts {
            resolved_chat_id: resolved_chat_id.clone(),
            role_id: role_id.clone(),
            state: state.clone(),
            nav_store: env::<NavigationStoreKey>(cx),
        };

        let role = parts.role(cx);
        let is_everyone = ChatRoleEditScreenParts::is_everyone(role.as_ref());
        let can_manage = parts.can_manage(cx);

        let interactions = ChatInteractionsStore::shared(cx);
        let task_chat_id = resolved_chat_id.clone();
        use_task(child_id("task"), (), window, cx, async move |cx| {
            interactions.update(cx, |interactions, cx| interactions.load_roles(&task_chat_id, cx));
        });

        let seeding = state.clone();
        on_change(child_id("role"), role, true, window, cx, move |_, role: &Option<ChatRole>, _, cx| {
            let Some(role) = role else {
                return;
            };
            seeding.update(cx, |state, cx| {
                state.seed_from_role(role);
                cx.notify();
            });
        });

        let (name, has_color, colored, mentionable, saving) = {
            let state = state.read(cx);
            (
                state.name.clone(),
                state.has_color,
                state.has_color && state.color_picked,
                state.mentionable,
                state.saving,
            )
        };
        on_change(child_id("fact-colored"), colored, true, window, cx, |_, value: &bool, window, cx| {
            GuideFacts::set(RiseGuideSignal::RoleColored, *value, cx);
            let services = GuidesServicesStore::shared(cx);
            if *value
                && services.read(cx).is_running(cx)
                && EditorColorPickerBridge::shared(cx).read(cx).picker_in_flight().is_some()
            {
                EditorColorPickerBridge::color_picker_view_controller_did_finish(window, cx);
            }
        });
        on_change(child_id("fact-mentionable"), mentionable, true, window, cx, |_, value: &bool, _, cx| {
            GuideFacts::set(RiseGuideSignal::RoleMentionable, *value, cx);
        });

        let form_enabled = can_manage && !saving;
        let form = vstack(16.0)
            .relative()
            .w_full()
            .pt(px(12.0))
            .when(!is_everyone, |form| {
                form.child(ChatRoleEditScreen::name_card(child_id("name").into(), &parts, &theme))
                    .child(ChatRoleEditScreen::color_card(&screen_id, has_color, &parts, &theme))
                    .child(ChatRoleEditScreen::flags_card(&screen_id, &parts, &theme))
            })
            .child(ChatRoleEditScreen::permissions_card(&screen_id, &parts, &theme))
            .child(spacer_min_h(60.0))
            .when(!form_enabled, |form| form.child(div().absolute().inset_0().occlude()));

        let scroll = scroll_view(child_id("scroll"))
            .list_shell_scroll_under_chrome(cx)
            .child(env_scope::<IsEnabledKey>(form_enabled).child(form));

        let title = if is_everyone {
            EVERYONE_ROLE_TITLE.to_owned()
        } else if name.is_empty() {
            tr("roles_title")
        } else {
            name
        };
        let back = parts.clone();
        let mut shell = SettingsWrapperUi::new(title, move |_, cx| back.pop(cx));
        if can_manage {
            shell = shell.right_jsx(ChatRoleEditScreen::save_button(child_id("save").into(), saving, parts, &theme));
        }

        let appearing = state.clone();
        let disappearing = state;
        shell
            .content(div().size_full().bg(theme.bg._100).child(scroll))
            .on_appear(child_id("appear"), move |_, cx| {
                appearing.update(cx, |state, _| state.on_screen = true);
            })
            .on_disappear(child_id("disappear"), move |_, cx| {
                disappearing.update(cx, |state, _| state.on_screen = false);
                GuideFacts::set(RiseGuideSignal::RoleColored, false, cx);
                GuideFacts::set(RiseGuideSignal::RoleMentionable, false, cx);
            })
    }
}
