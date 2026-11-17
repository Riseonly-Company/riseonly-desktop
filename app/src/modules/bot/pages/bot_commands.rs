use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Axis, ElementId, Entity, FontWeight, Hsla, SharedString, Subscription, Task, Window};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal, RiseValidation};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_theme::ui_color_system::UIColor;
use rise_theme::{AppTheme, color};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_field_rule::{RiseFocusedTextInput, RiseFocusedTextInputOutcome};
use crate::core::lib::canon_rules::RiseValidationCanonRules;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::components::bot_ui_components::{BotSettingsFieldRow, BotStateView};
use crate::modules::bot::shared::bot_ui_validation::{BotFieldRules, BotFormValidation};
use crate::modules::bot::stores::bot::bot_actions::bot_actions::BotActionsStore;
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotCommandModel;
use crate::modules::bot::stores::bot::bot_interactions::bot_interactions::BotInteractionsStore;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;

const COMMANDS_LIMIT_NAME: &str = "bot_commands_per_scope";
const DEFAULT_SCOPE: &str = "default";
const DISABLED_SAVE_OPACITY: f32 = 0.42;
const DESCRIPTION_MIN_HEIGHT: f32 = 34.0;
const DESCRIPTION_MAX_LINES: usize = 4;

/// limit is per scope and not per bot, canon and gateway count the same way
fn maximum_commands() -> usize {
    RiseValidation::collection_limit(COMMANDS_LIMIT_NAME).maximum
}

#[derive(Clone, PartialEq, Eq)]
struct BotCommandDraft {
    id: Uuid,
    command: String,
    description: String,
}

impl BotCommandDraft {
    fn new(command: String, description: String) -> Self {
        Self { id: Uuid::new_v4(), command, description }
    }
}

struct BotCommandsState {
    drafts: Vec<BotCommandDraft>,
    scope: String,
    language_code: String,
    did_hydrate: bool,
    has_loaded_once: bool,
    requested_context: Option<Vec<String>>,
    load_generation: Uuid,
    is_active: bool,
    _observations: Vec<Subscription>,
}

impl BotCommandsState {
    fn new(cx: &mut gpui::Context<Self>) -> Self {
        let actions = BotActionsStore::shared(cx);
        let services = BotServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        Self {
            drafts: Vec::new(),
            scope: String::new(),
            language_code: String::new(),
            did_hydrate: false,
            has_loaded_once: false,
            requested_context: None,
            load_generation: Uuid::new_v4(),
            is_active: false,
            _observations: vec![
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&auth, |_, _, cx| cx.notify()),
            ],
        }
    }

    /// empty field means the default scope so the word default never shows as typed text
    fn wire_scope(&self) -> String {
        let trimmed = trimming_whitespaces_and_newlines(&self.scope);
        if trimmed.is_empty() { DEFAULT_SCOPE.to_owned() } else { trimmed.to_owned() }
    }

    fn command_context(&self, bot_id: &str, cx: &mut App) -> Vec<String> {
        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        vec![bot_id.to_owned(), self.wire_scope(), self.language_code.clone(), user_id]
    }

    fn can_save(&self, command_context: &[String]) -> bool {
        let commands: HashSet<&str> = self.drafts.iter().map(|draft| draft.command.as_str()).collect();
        self.did_hydrate
            && self.requested_context.as_deref() == Some(command_context)
            && !self.drafts.is_empty()
            && self.drafts.iter().all(|draft| {
                BotFormValidation::is_valid_command(&draft.command)
                    && BotFieldRules::command_description().is_satisfied(&draft.description)
            })
            && commands.len() == self.drafts.len()
    }

    fn draft_error_text(&self) -> Option<String> {
        for draft in &self.drafts {
            if !draft.command.is_empty()
                && let Some(text) = BotFieldRules::command().error_text(&draft.command)
            {
                return Some(text);
            }
            if !draft.description.is_empty()
                && let Some(text) = BotFieldRules::command_description().error_text(&draft.description)
            {
                return Some(text);
            }
        }
        None
    }

    fn add_command(state: &Entity<Self>, cx: &mut App) {
        state.update(cx, |state, cx| {
            if !state.did_hydrate || state.drafts.len() >= maximum_commands() {
                return;
            }
            state.drafts.push(BotCommandDraft::new(String::new(), String::new()));
            cx.notify();
        });
    }

    fn remove_command(state: &Entity<Self>, id: &Uuid, cx: &mut App) {
        state.update(cx, |state, cx| {
            state.drafts.retain(|draft| draft.id != *id);
            cx.notify();
        });
    }

    fn reload(state: &Entity<Self>, bot_id: &str, cx: &mut App) {
        if !state.read(cx).is_active {
            return;
        }
        let generation = Uuid::new_v4();
        let answer_generation = generation;
        let context = state.update(cx, |state, cx| {
            state.did_hydrate = false;
            state.drafts.clear();
            let context = state.command_context(bot_id, cx);
            state.load_generation = generation;
            state.requested_context = Some(context.clone());
            cx.notify();
            context
        });
        let (scope, language_code) = {
            let state = state.read(cx);
            (state.wire_scope(), state.language_code.clone())
        };
        let answering = state.downgrade();
        let owner = bot_id.to_owned();
        let interactions = BotInteractionsStore::shared(cx);
        let accepted = interactions.update(cx, |store, cx| {
            store.load_commands(
                bot_id,
                &scope,
                &language_code,
                Some(Rc::new(move |commands: Vec<BotCommandModel>, cx: &mut App| {
                    let _ = answering.update(cx, |state, cx| {
                        if !state.is_active
                            || state.load_generation != answer_generation
                            || state.command_context(&owner, cx) != context
                        {
                            return;
                        }
                        state.drafts = commands
                            .into_iter()
                            .map(|command| BotCommandDraft::new(command.command, command.description))
                            .collect();
                        state.did_hydrate = true;
                        state.has_loaded_once = true;
                        cx.notify();
                    });
                })),
                cx,
            )
        });
        if !accepted {
            state.update(cx, |state, cx| {
                state.requested_context = None;
                cx.notify();
            });
        }
    }

    fn save(state: &Entity<Self>, bot_id: &str, cx: &mut App) {
        let command_context = state.update(cx, |state, cx| state.command_context(bot_id, cx));
        let (can_save, commands, scope, language_code) = {
            let state = state.read(cx);
            let commands: Vec<BotCommandModel> = state
                .drafts
                .iter()
                .map(|draft| BotCommandModel {
                    command: draft.command.clone(),
                    description: trimming_whitespaces_and_newlines(&draft.description).to_owned(),
                })
                .collect();
            (state.can_save(&command_context), commands, state.wire_scope(), state.language_code.clone())
        };
        if !can_save {
            return;
        }
        let interactions = BotInteractionsStore::shared(cx);
        interactions.update(cx, |store, cx| {
            store.set_commands(
                bot_id,
                commands,
                &scope,
                &language_code,
                Some(Rc::new(|_: Vec<BotCommandModel>, cx: &mut App| {
                    Notifier::send(
                        tr("bot_changes_saved"),
                        NotifierSendOptions {
                            r#type: NotifierType::Success,
                            position: NotifierPosition::Bottom,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                })),
                cx,
            );
        });
    }

    fn delete_all(state: &Entity<Self>, bot_id: &str, cx: &mut App) {
        let (scope, language_code) = {
            let state = state.read(cx);
            (state.wire_scope(), state.language_code.clone())
        };
        let answering = state.downgrade();
        let interactions = BotInteractionsStore::shared(cx);
        interactions.update(cx, |store, cx| {
            store.delete_commands(
                bot_id,
                &scope,
                &language_code,
                Some(Rc::new(move |cx: &mut App| {
                    let _ = answering.update(cx, |state, cx| {
                        state.drafts.clear();
                        cx.notify();
                    });
                })),
                cx,
            );
        });
    }
}

#[derive(IntoElement)]
pub struct BotCommands {
    bot_id: String,
}

impl BotCommands {
    pub fn new(bot_id: impl Into<String>) -> Self {
        Self { bot_id: bot_id.into() }
    }

    fn child_id(&self, name: &str) -> ElementId {
        ElementId::Name(SharedString::from(format!("bot-commands-{}-{name}", self.bot_id)))
    }

    fn scope_fields(&self, state: &Entity<BotCommandsState>, is_disabled: bool) -> GroupedBtns {
        let row = |row_id: &str,
                   icon: &'static str,
                   background: Hsla,
                   placeholder: String,
                   get: fn(&BotCommandsState) -> String,
                   set: fn(&mut BotCommandsState, String)| {
            let submitting = state.clone();
            let bot_id = self.bot_id.clone();
            GroupedBtn::new(row_id.to_owned(), "").disabled(is_disabled).content_slot(
                BotSettingsFieldRow::new(
                    self.child_id(row_id),
                    icon,
                    background,
                    placeholder,
                    Binding::entity(state.clone(), get, move |state, value, _| {
                        if !is_disabled {
                            set(state, value);
                        }
                    }),
                )
                .on_submit(move |_, cx| BotCommandsState::save(&submitting, &bot_id, cx)),
            )
        };
        GroupedBtns::new(
            self.child_id("scope-fields"),
            vec![
                row(
                    "scope",
                    "scope",
                    color("#5856D6"),
                    tr("bot_commands_scope"),
                    |state| state.scope.clone(),
                    |state, value| state.scope = value,
                ),
                row(
                    "language",
                    "globe",
                    color("#34C759"),
                    tr("bot_commands_language"),
                    |state| state.language_code.clone(),
                    |state, value| state.language_code = value,
                ),
            ],
        )
        .title(tr("bot_commands_scope_section"))
        .corner_radius(22.0)
    }

    fn command_card(&self, state: &Entity<BotCommandsState>, draft_id: &Uuid, theme: &AppTheme) -> Div {
        let id_text = draft_id.uuid_string();
        let draft_field = |get: fn(&BotCommandDraft) -> String, set: fn(&mut BotCommandDraft, String)| {
            let reader = state.clone();
            let writer = state.clone();
            let read_id = *draft_id;
            let write_id = *draft_id;
            Binding::new(
                move |cx| {
                    reader
                        .read(cx)
                        .drafts
                        .iter()
                        .find(|draft| draft.id == read_id)
                        .map(get)
                        .unwrap_or_default()
                },
                move |value, _, cx| {
                    writer.update(cx, |state, cx| {
                        if let Some(draft) = state.drafts.iter_mut().find(|draft| draft.id == write_id) {
                            set(draft, value);
                            cx.notify();
                        }
                    });
                },
            )
        };
        let raw_command = draft_field(|draft| draft.command.clone(), |draft, value| draft.command = value);
        let reading_command = raw_command.clone();
        let command = Binding::new(
            move |cx| reading_command.get(cx),
            move |raw: String, window: &mut Window, cx: &mut App| {
                let kept = BotFormValidation::normalized_command(&raw);
                if kept == raw {
                    raw_command.set(raw, window, cx);
                    return;
                }
                match RiseFocusedTextInput::shape(&raw, &kept, window, cx) {
                    RiseFocusedTextInputOutcome::Composing => raw_command.set(raw, window, cx),
                    RiseFocusedTextInputOutcome::Clamped | RiseFocusedTextInputOutcome::NotFocused => {
                        raw_command.set(kept, window, cx);
                    }
                }
            },
        );
        let description = BotFieldRules::command_description()
            .binding(draft_field(|draft| draft.description.clone(), |draft, value| draft.description = value));
        let submit = |state: &Entity<BotCommandsState>| {
            let state = state.clone();
            let bot_id = self.bot_id.clone();
            move |_: &mut Window, cx: &mut App| BotCommandsState::save(&state, &bot_id, cx)
        };
        let removing = state.clone();
        let removed_id = *draft_id;

        vstack(0.0)
            .w_full()
            .bg(theme.bg._200)
            .rounded(px(18.0))
            .overflow_hidden()
            .child(
                hstack(10.0)
                    .w_full()
                    .px(px(16.0))
                    .min_h(px(52.0))
                    .child(MainText::new("/").px(16.0).font_weight(FontWeight::SEMIBOLD).color(theme.primary._100))
                    .child(
                        div().flex_1().child(
                            ShapedTextField::new(self.child_id(&format!("{id_text}-command")), tr("bot_command_name"), command)
                                .font_size(15.0)
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.text.primary)
                                .on_submit(submit(state))
                                .on_cancel(|window, cx| ScreenFocus::release_text_field(window, cx)),
                        ),
                    )
                    .child(
                        plain(self.child_id(&format!("{id_text}-remove")))
                            .on_press(move |_, cx| BotCommandsState::remove_command(&removing, &removed_id, cx))
                            .children(IconUi::sized("minus.circle.fill", px(20.0), theme.semantic.error_100, true)),
                    ),
            )
            .child(div().w_full().pl(px(16.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55))))
            .child(
                div().w_full().px(px(16.0)).py(px(13.0)).child(
                    div().w_full().min_h(px(DESCRIPTION_MIN_HEIGHT)).child(
                        ShapedTextField::new(
                            self.child_id(&format!("{id_text}-description")),
                            tr("bot_command_description"),
                            description,
                        )
                        .axis(Axis::Vertical)
                        .max_visible_lines(DESCRIPTION_MAX_LINES)
                        .font_size(14.0)
                        .text_color(theme.text.primary)
                        .on_submit(submit(state))
                        .on_cancel(|window, cx| ScreenFocus::release_text_field(window, cx)),
                    ),
                ),
            )
    }

    fn load_error_view(&self, name: &str, state: &Entity<BotCommandsState>, message: Option<String>) -> BotStateView {
        let retrying = state.clone();
        let bot_id = self.bot_id.clone();
        BotStateView::new(
            self.child_id(name),
            "exclamationmark.arrow.triangle.2.circlepath",
            tr("bot_commands_load_error"),
        )
        .message(message)
        .action_title(tr("bot_retry"))
        .action(move |_, cx| BotCommandsState::reload(&retrying, &bot_id, cx))
    }
}

impl RenderOnce for BotCommands {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, cx| BotCommandsState::new(cx));
        let actions = BotActionsStore::shared(cx);

        let command_context = state.update(cx, |state, cx| state.command_context(&self.bot_id, cx));
        let (can_save, drafts_empty, did_hydrate, has_loaded_once, drafts_count) = {
            let current = state.read(cx);
            (
                current.can_save(&command_context),
                current.drafts.is_empty(),
                current.did_hydrate,
                current.has_loaded_once,
                current.drafts.len(),
            )
        };
        let (wire_scope, language_code) = {
            let current = state.read(cx);
            (current.wire_scope(), current.language_code.clone())
        };
        let stored_commands_empty =
            BotServicesStore::commands(&self.bot_id, &wire_scope, &language_code, cx).is_empty();
        let (get_status, get_error, get_pending, set_pending, set_error, delete_pending) = {
            let actions = actions.read(cx);
            (
                actions.get_bot_commands_sai.status(),
                actions.get_bot_commands_sai.error(),
                actions.get_bot_commands_sai.is_pending(),
                actions.set_bot_commands_sai.is_pending(),
                actions.set_bot_commands_sai.error(),
                actions.delete_bot_commands_sai.is_pending(),
            )
        };
        let status = if has_loaded_once { SaiWsSliceStatus::Fulfilled } else { get_status };
        let error_text = state.read(cx).draft_error_text().or(set_error);

        let context_state = state.clone();
        let context_bot_id = self.bot_id.clone();
        on_change(self.child_id("context-change"), command_context, false, window, cx, move |_, _, _, cx| {
            BotCommandsState::reload(&context_state, &context_bot_id, cx);
        });
        let pending_state = state.clone();
        let pending_bot_id = self.bot_id.clone();
        on_change(self.child_id("pending-change"), get_pending, false, window, cx, move |_, pending, _, cx| {
            let needs_reload = {
                let current = pending_state.read(cx);
                current.requested_context.is_none() && !current.did_hydrate
            };
            if !*pending && needs_reload {
                BotCommandsState::reload(&pending_state, &pending_bot_id, cx);
            }
        });
        on_change(self.child_id("drafted-change"), drafts_empty, true, window, cx, |_, empty, _, cx| {
            GuideFacts::set(RiseGuideSignal::BotCommandDrafted, !*empty, cx);
        });
        on_change(self.child_id("valid-change"), can_save, true, window, cx, |_, ready, _, cx| {
            GuideFacts::set(RiseGuideSignal::BotCommandsValid, *ready, cx);
        });
        let save_state = state.clone();
        let save_bot_id = self.bot_id.clone();
        let save_enabled = can_save && !set_pending;
        let save_button = div()
            .opacity(if save_enabled { 1.0 } else { DISABLED_SAVE_OPACITY })
            .child(
                PageHeaderIconButton::new(self.child_id("save"), "checkmark", move |_, cx| {
                    BotCommandsState::save(&save_state, &save_bot_id, cx);
                })
                .disabled(!save_enabled),
            )
            .guide_anchor(RiseGuideAnchor::BotCommandsSave);

        let error_retry_state = state.clone();
        let error_retry_bot_id = self.bot_id.clone();
        let error_id = self.child_id("render-error");
        let error_message = get_error.clone();
        let appear_id = self.child_id("appear");
        let disappear_id = self.child_id("disappear");
        let appearing = state.clone();
        let appear_bot_id = self.bot_id.clone();
        let disappearing = state.clone();

        let content_state = state.clone();
        let content_theme = theme.clone();
        let content_error = get_error.clone();
        let content = SaiDataRender::new(status, move |_, cx| {
            let state = content_state;
            let theme = content_theme;
            let refresh_state = state.clone();
            let refresh_bot_id = self.bot_id.clone();

            let body: AnyElement = if !did_hydrate {
                if let Some(error) = content_error {
                    self.load_error_view("load-error", &state, Some(error)).into_any_element()
                } else {
                    div()
                        .w_full()
                        .min_h(px(100.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(progress_view())
                        .into_any_element()
                }
            } else if drafts_empty {
                div()
                    .w_full()
                    .min_h(px(280.0))
                    .child(
                        BotStateView::new(self.child_id("empty"), "terminal", tr("bot_commands_empty"))
                            .message(tr("bot_commands_empty_hint")),
                    )
                    .into_any_element()
            } else {
                let ids: Vec<Uuid> = state.read(cx).drafts.iter().map(|draft| draft.id).collect();
                let first = ids.first().copied();
                vstack(12.0)
                    .w_full()
                    .children(ids.iter().map(|draft_id| {
                        let anchor = (first == Some(*draft_id)).then_some(RiseGuideAnchor::BotCommandFirst);
                        div()
                            .w_full()
                            .px(px(12.0))
                            .child(self.command_card(&state, draft_id, &theme))
                            .guide_anchor(anchor)
                    }))
                    .into_any_element()
            };

            let adding = state.clone();
            let confirming = state.clone();
            let confirming_bot_id = self.bot_id.clone();
            let group_buttons = GroupedBtns::new(
                self.child_id("buttons"),
                vec![
                    GroupedBtn::new("add", tr("bot_command_add"))
                        .icon("plus")
                        .icon_background(theme.primary._100)
                        .disabled(!did_hydrate || drafts_count >= maximum_commands())
                        .guide_anchor(RiseGuideAnchor::BotCommandsAdd)
                        .show_chevron(false)
                        .callback(move |_, cx| BotCommandsState::add_command(&adding, cx)),
                    GroupedBtn::new("deleteAll", tr("bot_commands_delete_all"))
                        .icon("trash.fill")
                        .icon_background(UIColor::system_red().resolved_color(theme.appearance))
                        .destructive(true)
                        .disabled(!did_hydrate || (drafts_empty && stored_commands_empty) || delete_pending)
                        .show_chevron(false)
                        .callback(move |_, cx| {
                            let deleting = confirming.clone();
                            let deleting_bot_id = confirming_bot_id.clone();
                            NativeConfirmAlert::present_delete(
                                &tr("bot_commands_delete_all"),
                                &tr("bot_commands_delete_message"),
                                Some(tr("bot_commands_delete_all")),
                                None,
                                false,
                                None,
                                None,
                                move |cx: &mut App| BotCommandsState::delete_all(&deleting, &deleting_bot_id, cx),
                                cx,
                            );
                        }),
                ],
            )
            .corner_radius(22.0);

            scroll_view(self.child_id("scroll"))
                .size_full()
                .list_shell_scroll_under_chrome(cx)
                .bg(theme.bg._100)
                .refreshable(move |_, cx| {
                    BotCommandsState::reload(&refresh_state, &refresh_bot_id, cx);
                    Task::ready(())
                })
                .child(
                    vstack(18.0)
                        .w_full()
                        .pt(px(12.0))
                        .child(
                            div()
                                .w_full()
                                .child(self.scope_fields(&state, set_pending || delete_pending))
                                .guide_anchor(RiseGuideAnchor::BotCommandsScope),
                        )
                        .child(body)
                        .child(div().w_full().child(group_buttons))
                        .children(error_text.map(|error| {
                            div()
                                .w_full()
                                .px(px(18.0))
                                .child(MainText::new(error).px(13.0).color(theme.semantic.error_100))
                        }))
                        .child(spacer_min_h(80.0)),
                )
        })
        .is_empty(false)
        .error_text(get_error.or_else(|| Some(tr("bot_commands_load_error"))))
        .sai_data_render_error(move |_, _| {
            let retrying = error_retry_state.clone();
            let bot_id = error_retry_bot_id.clone();
            BotStateView::new(
                error_id.clone(),
                "exclamationmark.arrow.triangle.2.circlepath",
                tr("bot_commands_load_error"),
            )
            .message(error_message.clone())
            .action_title(tr("bot_retry"))
            .action(move |_, cx| BotCommandsState::reload(&retrying, &bot_id, cx))
        });

        SettingsWrapperUi::new(tr("bot_commands_title"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .right_jsx(save_button)
        .respects_keyboard_safe_area(true)
        .content(content)
        .on_appear(appear_id, move |_, cx| {
            appearing.update(cx, |state, cx| {
                state.is_active = true;
                cx.notify();
            });
            BotCommandsState::reload(&appearing, &appear_bot_id, cx);
        })
        .on_disappear(disappear_id, move |_, cx| {
            disappearing.update(cx, |state, cx| {
                state.is_active = false;
                state.load_generation = Uuid::new_v4();
                cx.notify();
            });
            GuideFacts::set(RiseGuideSignal::BotCommandDrafted, false, cx);
            GuideFacts::set(RiseGuideSignal::BotCommandsValid, false, cx);
        })
    }
}
