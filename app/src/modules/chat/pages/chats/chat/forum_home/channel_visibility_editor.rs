use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, ElementId, Entity, FontWeight, SharedString, Subscription, Window};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::modal_sheet_surface::ModalSheetFrame;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::shapes::SwiftUiShapes;
use rise_ui::swiftui::stacks::{hstack, spacer, spacer_min_h, vstack};
use rise_ui::switch_ui::SwitchUi;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::chat::engine::chat_engine_hooks::ChatEngineMainCallback;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_roles_actions::ChatRolesResultCallback;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatPerm, ChatPermOverwrite, ChatRole, OverwriteScope,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

const VIEW: i64 = ChatPerm::VIEW_CHANNELS;
const ROLE_TARGET: &str = ChatPerm::OVERWRITE_TARGETS[0].wire;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VisibilityTargetKind {
    Topic,
    Section,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VisibilityTarget {
    pub kind: VisibilityTargetKind,
    pub id: String,
    pub title: String,
}

impl VisibilityTarget {
    pub fn scope(&self) -> OverwriteScope {
        if self.kind == VisibilityTargetKind::Topic { OverwriteScope::Topic } else { OverwriteScope::Section }
    }
}

/// page state, confirmed fields are the server confirmed state
/// writes go one at a time per toggle so a quick on off cant reorder, a refusal rolls the toggle back
struct ChannelVisibilityEditorState {
    is_private: bool,
    allowed_role_ids: HashSet<String>,
    loaded: bool,
    confirmed_private: bool,
    confirmed_role_ids: HashSet<String>,
    private_in_flight: bool,
    roles_in_flight: HashSet<String>,
    _services: Subscription,
}

impl ChannelVisibilityEditorState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        Self {
            is_private: false,
            allowed_role_ids: HashSet::new(),
            loaded: false,
            confirmed_private: false,
            confirmed_role_ids: HashSet::new(),
            private_in_flight: false,
            roles_in_flight: HashSet::new(),
            _services: cx.observe(&services, |_, _, cx| cx.notify()),
        }
    }
}

#[derive(IntoElement)]
pub struct ChannelVisibilityEditor {
    chat_id: String,
    target: VisibilityTarget,
}

impl ChannelVisibilityEditor {
    pub fn new(chat_id: impl Into<String>, target: VisibilityTarget) -> Self {
        Self { chat_id: chat_id.into(), target }
    }
}

impl RenderOnce for ChannelVisibilityEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let id = ElementId::Name(format!("channel-visibility-{}-{}", self.chat_id, self.target.id).into());
        let state =
            window.use_keyed_state(named_child(&id, "state"), cx, |_, cx| ChannelVisibilityEditorState::new(cx));
        let model = ChannelVisibilityEditorModel {
            id: id.clone(),
            chat_id: self.chat_id,
            scope: self.target.scope(),
            scope_id: self.target.id,
            state: state.clone(),
        };

        let (is_private, has_allowed_roles) = {
            let editor = state.read(cx);
            (editor.is_private, !editor.allowed_role_ids.is_empty())
        };
        on_change(named_child(&id, "fact-private"), is_private, true, window, cx, |_, value, _, cx| {
            GuideFacts::set(RiseGuideSignal::TopicPrivate, *value, cx);
        });
        on_change(named_child(&id, "fact-role"), has_allowed_roles, true, window, cx, |_, value, _, cx| {
            GuideFacts::set(RiseGuideSignal::TopicRoleAllowed, *value, cx);
        });

        let custom_roles = model.custom_roles(cx);
        let private_card = model.private_card(is_private, custom_roles.is_empty(), &theme);
        let dismiss_action = dismiss(cx);
        let loading = model.clone();
        let sheet_height = ModalSheetFrame::for_window(window, cx).fill_floor();

        let page = SettingsWrapperUi::new(self.target.title, move |window, cx| dismiss_action.call_as_function(window, cx))
            .content(
                div().size_full().bg(theme.bg._100).child(
                    scroll_view(named_child(&id, "scroll")).size_full().list_shell_scroll_under_chrome(cx).child(
                        vstack(16.0)
                            .w_full()
                            .pt(px(12.0))
                            .child(private_card)
                            .when(is_private, |stack| stack.child(model.roles_card(custom_roles, &theme)))
                            .child(spacer_min_h(60.0)),
                    ),
                ),
            )
            .on_appear(named_child(&id, "lifecycle"), move |_, cx| {
                GuideFacts::set(RiseGuideSignal::TopicVisibilityOpen, true, cx);
                cx.defer(move |cx| {
                    if !loading.state.read(cx).loaded {
                        loading.load(cx);
                    }
                });
            })
            .and_on_disappear(|_, cx| {
                GuideFacts::set(RiseGuideSignal::TopicVisibilityOpen, false, cx);
                GuideFacts::set(RiseGuideSignal::TopicPrivate, false, cx);
                GuideFacts::set(RiseGuideSignal::TopicRoleAllowed, false, cx);
            });
        div().w_full().h(px(sheet_height)).child(page)
    }
}

fn named_child(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

fn section_title(text: String, theme: &AppTheme) -> Div {
    div().px(px(24.0)).pb(px(6.0)).child(
        MainText::new(text.to_uppercase()).px(12.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.secondary),
    )
}

#[derive(Clone)]
struct ChannelVisibilityEditorModel {
    id: ElementId,
    chat_id: String,
    scope: OverwriteScope,
    scope_id: String,
    state: Entity<ChannelVisibilityEditorState>,
}

impl ChannelVisibilityEditorModel {
    fn custom_roles(&self, cx: &mut App) -> Vec<ChatRole> {
        ChatServicesStore::shared(cx).update(cx, |store, cx| store.custom_roles(&self.chat_id, cx))
    }

    fn everyone_role_id(&self, cx: &mut App) -> Option<String> {
        ChatServicesStore::shared(cx)
            .update(cx, |store, cx| store.everyone_role(&self.chat_id, cx))
            .map(|role| role.id)
    }

    fn private_card(&self, is_private: bool, has_no_roles: bool, theme: &AppTheme) -> Div {
        let reading = self.state.clone();
        let writing = self.clone();
        vstack(0.0).items_start().w_full().child(section_title(tr("forum_visibility_title"), theme)).child(
            div().w_full().px(px(12.0)).child(
                hstack(12.0)
                    .w_full()
                    .p(px(14.0))
                    .bg(theme.bg._200)
                    .continuous_corners(18.0)
                    .children(IconUi::sized(
                        if is_private { "lock.fill" } else { "lock.open.fill" },
                        px(15.0),
                        theme.primary._100,
                        true,
                    ))
                    .child(
                        vstack(2.0)
                            .items_start()
                            .flex_shrink_1()
                            .min_w_0()
                            .child(MainText::new(tr("forum_visibility_private")).px(15.0))
                            .child(
                                MainText::new(if has_no_roles {
                                    tr("forum_visibility_no_roles")
                                } else {
                                    tr("forum_visibility_private_hint")
                                })
                                .px(12.0)
                                .color(theme.text.secondary),
                            ),
                    )
                    .child(spacer())
                    .child(SwitchUi::new(
                        named_child(&self.id, "private-switch"),
                        Binding::new(
                            move |cx: &App| reading.read(cx).is_private,
                            move |value: bool, _: &mut Window, cx: &mut App| writing.set_private(value, cx),
                        ),
                    ))
                    .guide_anchor(RiseGuideAnchor::VisibilityPrivate),
            ),
        )
    }

    fn roles_card(&self, custom_roles: Vec<ChatRole>, theme: &AppTheme) -> Div {
        let title = section_title(tr("forum_visibility_roles"), theme);
        if custom_roles.is_empty() {
            return vstack(0.0).items_start().w_full().child(title).child(
                div().w_full().px(px(12.0)).child(
                    div()
                        .w_full()
                        .px(px(16.0))
                        .py(px(12.0))
                        .bg(theme.bg._200)
                        .continuous_corners(18.0)
                        .child(MainText::new(tr("forum_visibility_no_roles")).px(13.0).color(theme.text.secondary)),
                ),
            );
        }
        let last_index = custom_roles.len() - 1;
        let rows: Vec<AnyElement> = custom_roles
            .into_iter()
            .enumerate()
            .map(|(index, role)| self.role_row(role, index < last_index, theme))
            .collect();
        vstack(0.0).items_start().w_full().child(title).child(
            div().w_full().px(px(12.0)).child(
                vstack(0.0)
                    .w_full()
                    .bg(theme.bg._200)
                    .continuous_corners(18.0)
                    .children(rows)
                    .guide_anchor(RiseGuideAnchor::VisibilityRoles),
            ),
        )
    }

    fn role_row(&self, role: ChatRole, shows_separator: bool, theme: &AppTheme) -> AnyElement {
        let role_color = role.swift_color();
        let reading = self.state.clone();
        let writing = self.clone();
        let read_id = role.id.clone();
        let write_id = role.id.clone();
        let switch_id = ElementId::NamedChild(
            Arc::new(named_child(&self.id, "role-switch")),
            SharedString::from(role.id.clone()),
        );
        div()
            .w_full()
            .child(
                hstack(11.0)
                    .w_full()
                    .px(px(16.0))
                    .py(px(11.0))
                    .child(
                        div()
                            .flex_none()
                            .size(px(12.0))
                            .rounded_full()
                            .bg(role_color.unwrap_or_else(|| theme.text.secondary.opacity(0.4))),
                    )
                    .child(
                        div().flex_shrink_1().min_w_0().child(
                            MainText::new(role.name)
                                .px(15.0)
                                .color(role_color.map(|color| theme.legible(color)))
                                .number_of_lines(1),
                        ),
                    )
                    .child(spacer())
                    .child(SwitchUi::new(
                        switch_id,
                        Binding::new(
                            move |cx: &App| reading.read(cx).allowed_role_ids.contains(&read_id),
                            move |value: bool, _: &mut Window, cx: &mut App| {
                                writing.set_role(write_id.clone(), value, cx);
                            },
                        ),
                    )),
            )
            .when(shows_separator, |row| {
                row.child(div().ml(px(39.0)).h(px(0.5)).bg(theme.border._200.opacity(0.5)))
            })
            .into_any_element()
    }

    fn load(&self, cx: &mut App) {
        self.state.update(cx, |editor, _| editor.loaded = true);
        if self.everyone_role_id(cx).is_some() {
            self.load_overwrites(cx);
            return;
        }
        let model = self.clone();
        let on_done: ChatEngineMainCallback = Rc::new(move |cx: &mut App| {
            let model = model.clone();
            cx.defer(move |cx| model.load_overwrites(cx));
        });
        self.get_forum_structure(Some(on_done.clone()), Some(on_done), cx);
    }

    fn get_forum_structure(
        &self,
        on_loaded: Option<ChatEngineMainCallback>,
        on_failed: Option<ChatEngineMainCallback>,
        cx: &mut App,
    ) {
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.get_forum_structure_action(&self.chat_id, true, on_loaded, on_failed, cx);
        });
    }

    fn load_overwrites(&self, cx: &mut App) {
        let everyone_id = self.everyone_role_id(cx);
        let state = self.state.clone();
        let on_result: Rc<dyn Fn(Vec<ChatPermOverwrite>, &mut App)> =
            Rc::new(move |overwrites: Vec<ChatPermOverwrite>, cx: &mut App| {
                let mut private = false;
                let mut allowed = HashSet::new();
                for overwrite in overwrites.iter().filter(|overwrite| overwrite.target_type == ROLE_TARGET) {
                    if Some(&overwrite.target_id) == everyone_id.as_ref() {
                        if overwrite.deny & VIEW != 0 {
                            private = true;
                        }
                    } else if overwrite.allow & VIEW != 0 {
                        allowed.insert(overwrite.target_id.clone());
                    }
                }
                state.update(cx, |editor, cx| {
                    if editor.private_in_flight || !editor.roles_in_flight.is_empty() {
                        return;
                    }
                    editor.is_private = private;
                    editor.allowed_role_ids = allowed.clone();
                    editor.confirmed_private = private;
                    editor.confirmed_role_ids = allowed;
                    cx.notify();
                });
            });
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.get_overwrites_action(&self.chat_id, self.scope, &self.scope_id, on_result, cx);
        });
    }

    fn set_private(&self, on: bool, cx: &mut App) {
        self.state.update(cx, |editor, cx| {
            editor.is_private = on;
            cx.notify();
        });
        self.sync_private(cx);
    }

    fn sync_private(&self, cx: &mut App) {
        let (private_in_flight, wanted, confirmed) = {
            let editor = self.state.read(cx);
            (editor.private_in_flight, editor.is_private, editor.confirmed_private)
        };
        if private_in_flight || wanted == confirmed {
            return;
        }
        self.state.update(cx, |editor, _| editor.private_in_flight = true);

        let Some(everyone_id) = self.everyone_role_id(cx) else {
            let model = self.clone();
            let resume: ChatEngineMainCallback = Rc::new(move |cx: &mut App| {
                let model = model.clone();
                cx.defer(move |cx| model.resume_private(cx));
            });
            self.get_forum_structure(Some(resume.clone()), Some(resume), cx);
            return;
        };

        let model = self.clone();
        let done: ChatRolesResultCallback = Rc::new(move |ok: bool, cx: &mut App| {
            let model = model.clone();
            cx.defer(move |cx| model.finish_private(ok, wanted, cx));
        });
        if wanted {
            self.set_overwrite(&everyone_id, 0, VIEW, done, cx);
        } else {
            self.remove_overwrite(&everyone_id, done, cx);
        }
    }

    fn resume_private(&self, cx: &mut App) {
        let has_everyone = self.everyone_role_id(cx).is_some();
        self.state.update(cx, |editor, cx| {
            editor.private_in_flight = false;
            if !has_everyone {
                editor.is_private = editor.confirmed_private;
            }
            cx.notify();
        });
        if has_everyone {
            self.sync_private(cx);
        }
    }

    fn finish_private(&self, ok: bool, wanted: bool, cx: &mut App) {
        self.state.update(cx, |editor, cx| {
            editor.private_in_flight = false;
            if ok {
                editor.confirmed_private = wanted;
            } else {
                editor.is_private = editor.confirmed_private;
            }
            cx.notify();
        });
        if ok {
            self.refresh(cx);
            self.sync_private(cx);
        }
    }

    fn set_role(&self, role_id: String, can_view: bool, cx: &mut App) {
        self.state.update(cx, |editor, cx| {
            if can_view {
                editor.allowed_role_ids.insert(role_id.clone());
            } else {
                editor.allowed_role_ids.remove(&role_id);
            }
            cx.notify();
        });
        self.sync_role(role_id, cx);
    }

    fn sync_role(&self, role_id: String, cx: &mut App) {
        let (wanted, should_write) = {
            let editor = self.state.read(cx);
            let wanted = editor.allowed_role_ids.contains(&role_id);
            let should_write =
                !editor.roles_in_flight.contains(&role_id) && wanted != editor.confirmed_role_ids.contains(&role_id);
            (wanted, should_write)
        };
        if !should_write {
            return;
        }
        self.state.update(cx, |editor, _| {
            editor.roles_in_flight.insert(role_id.clone());
        });

        let model = self.clone();
        let done_role_id = role_id.clone();
        let done: ChatRolesResultCallback = Rc::new(move |ok: bool, cx: &mut App| {
            let model = model.clone();
            let role_id = done_role_id.clone();
            cx.defer(move |cx| model.finish_role(role_id, ok, wanted, cx));
        });
        if wanted {
            self.set_overwrite(&role_id, VIEW, 0, done, cx);
        } else {
            self.remove_overwrite(&role_id, done, cx);
        }
    }

    fn finish_role(&self, role_id: String, ok: bool, wanted: bool, cx: &mut App) {
        self.state.update(cx, |editor, cx| {
            editor.roles_in_flight.remove(&role_id);
            if !ok {
                if editor.confirmed_role_ids.contains(&role_id) {
                    editor.allowed_role_ids.insert(role_id.clone());
                } else {
                    editor.allowed_role_ids.remove(&role_id);
                }
            } else if wanted {
                editor.confirmed_role_ids.insert(role_id.clone());
            } else {
                editor.confirmed_role_ids.remove(&role_id);
            }
            cx.notify();
        });
        if ok {
            self.refresh(cx);
            self.sync_role(role_id, cx);
        }
    }

    fn set_overwrite(&self, target_id: &str, allow: i64, deny: i64, done: ChatRolesResultCallback, cx: &mut App) {
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.set_overwrite_action(
                &self.chat_id,
                self.scope,
                &self.scope_id,
                ROLE_TARGET,
                target_id,
                allow,
                deny,
                Some(done),
                cx,
            );
        });
    }

    fn remove_overwrite(&self, target_id: &str, done: ChatRolesResultCallback, cx: &mut App) {
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.remove_overwrite_action(
                &self.chat_id,
                self.scope,
                &self.scope_id,
                ROLE_TARGET,
                target_id,
                Some(done),
                cx,
            );
        });
    }

    fn refresh(&self, cx: &mut App) {
        self.get_forum_structure(None, None, cx);
    }
}
