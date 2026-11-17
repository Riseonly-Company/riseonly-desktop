use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{App, ElementId, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, SharedString, Window};
use rise_canon::rise_key::report;
use rise_core::foundation::foundation_text::{character_count, trimming_whitespaces_and_newlines};
use rise_i18n::tr;
use rise_moderation::{BlockTypeRule, Ladder};
use rise_platform::haptics::{ImpactStyle, impact};
use rise_theme::{AppTheme, color};
use rise_ui::button_ui::ButtonUi;
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;
use rise_ui::bottom_sheet_ui::modal_sheet_surface::ModalSheetFrame;

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::input_ui::InputUi;
use crate::core::ui::text_area_ui::TextAreaUi;
use crate::modules::moderation::components::moderation_kit_ui::{
    ModerationArtwork, moderation_child_id, moderation_symbol,
};
use crate::modules::moderation::components::moderation_type_tints::AppealDecisionTintExt;
use crate::modules::moderation::engine::rise_moderation_presentation::RiseModerationPresentation;
use crate::modules::moderation::engine::rise_moderation_wire::{ReportRowDTO, RiseStoredCaseCapabilities};
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    AdminSanctionKind, AppealDecision, AppealDecisionExt, ModerationDurationChoice, ModerationMuteScope, ResolveAction,
    ResolveActionExt, UserBlockType, UserBlockTypeExt,
};
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;

const RETURN_KEY: &str = "enter";
const MODAL_OUTLINE_WIDTH: f32 = 1.0;
const REOPEN_TINT: &str = "#F2A53C";
const SANCTION_REASON_MINIMUM_CHARACTERS: usize = 3;

/// identity comes in beside the labels since a foreign canon enum
/// cant take a local trait and a kit trait both
#[derive(IntoElement)]
pub struct ModerationChoiceRow<Choice: Clone + PartialEq + 'static> {
    id: ElementId,
    title: SharedString,
    choices: Vec<Choice>,
    choice_id: Rc<dyn Fn(&Choice) -> String>,
    label: Rc<dyn Fn(&Choice) -> String>,
    icon: Option<Rc<dyn Fn(&Choice) -> Option<String>>>,
    selection: Binding<Option<Choice>>,
    theme: AppTheme,
}

impl<Choice: Clone + PartialEq + 'static> ModerationChoiceRow<Choice> {
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        choices: Vec<Choice>,
        choice_id: impl Fn(&Choice) -> String + 'static,
        label: impl Fn(&Choice) -> String + 'static,
        selection: Binding<Option<Choice>>,
        theme: AppTheme,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            choices,
            choice_id: Rc::new(choice_id),
            label: Rc::new(label),
            icon: None,
            selection,
            theme,
        }
    }

    pub fn icon(mut self, icon: impl Fn(&Choice) -> Option<String> + 'static) -> Self {
        self.icon = Some(Rc::new(icon));
        self
    }
}

impl<Choice: Clone + PartialEq + 'static> RenderOnce for ModerationChoiceRow<Choice> {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let current = self.selection.get(cx);
        let chips: Vec<AnyElement> = self
            .choices
            .into_iter()
            .map(|choice| {
                let selected = current.as_ref() == Some(&choice);
                let foreground = if selected { theme.content_on(theme.primary._100) } else { theme.text.primary };
                let symbol = self.icon.as_ref().and_then(|icon| icon(&choice));
                let text = (self.label)(&choice);
                let chip_id = moderation_child_id(&self.id, format!("chip-{}", (self.choice_id)(&choice)));
                let selection = self.selection.clone();
                plain(chip_id)
                    .flex_none()
                    .on_press(move |window, cx| {
                        let _ = impact(ImpactStyle::Light);
                        selection.set(Some(choice.clone()), window, cx);
                    })
                    .child(
                        hstack(5.0)
                            .px(px(12.0))
                            .py(px(8.0))
                            .bg(if selected { theme.primary._100 } else { theme.bg._200 })
                            .rounded_full()
                            .children(symbol.and_then(|name| moderation_symbol(&name, 11.0, foreground)))
                            .child(MainText::new(text).px(13.0).font_weight(FontWeight::SEMIBOLD).color(foreground)),
                    )
                    .into_any_element()
            })
            .collect();

        vstack(6.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(sheet_field_label(self.title, &theme))
            .child(
                div()
                    .id(moderation_child_id(&self.id, "chips"))
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(8.0))
                    .px(px(1.0))
                    .children(chips),
            )
    }
}

pub enum ModerationDurationLabel {}

impl ModerationDurationLabel {
    pub fn text(choice: ModerationDurationChoice) -> String {
        match choice {
            ModerationDurationChoice::Days(days) => tr("case_days").replace("{n}", &days.to_string()),
            ModerationDurationChoice::Permanent => tr("blocks_permanent"),
        }
    }
}

fn sheet_field_label(text: impl Into<SharedString>, theme: &AppTheme) -> MainText {
    MainText::new(text).px(12.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.secondary)
}

#[derive(IntoElement)]
struct SheetTitle {
    id: ElementId,
    icon: SharedString,
    tint: Hsla,
    title: SharedString,
    subtitle: SharedString,
    theme: AppTheme,
}

impl RenderOnce for SheetTitle {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        vstack(12.0)
            .w_full()
            .pb(px(4.0))
            .child(ModerationArtwork::new(self.id, self.icon, self.tint).side(76.0))
            .child(
                vstack(5.0)
                    .w_full()
                    .child(MainText::new(self.title).px(20.0).tac(TextAlignT::Center).font_weight(FontWeight::BOLD))
                    .child(
                        MainText::new(self.subtitle)
                            .px(13.0)
                            .tac(TextAlignT::Center)
                            .color(self.theme.text.secondary),
                    ),
            )
    }
}

#[derive(IntoElement)]
struct ModerationSheetFooter {
    id: ElementId,
    theme: AppTheme,
    confirm: AnyElement,
    on_cancel: ButtonAction,
}

impl RenderOnce for ModerationSheetFooter {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_cancel = self.on_cancel;
        vstack(12.0)
            .w_full()
            .flex_none()
            .px(px(20.0))
            .pt(px(8.0))
            .pb(px(12.0))
            .bg(self.theme.bg._100)
            .child(self.confirm)
            .child(
                plain(moderation_child_id(&self.id, "cancel"))
                    .w_full()
                    .on_press(move |window, cx| on_cancel(window, cx))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .justify_center()
                            .child(MainText::new(tr("cancel")).px(14.0).color(self.theme.text.secondary)),
                    ),
            )
    }
}

/// scroll view with footer inset plus desktop keys
/// secondary modifier with return confirms anywhere, bare return only when the surface has focus
/// escape closes anywhere
#[allow(clippy::too_many_arguments)]
fn moderation_sheet_surface(
    id: &ElementId,
    form: impl IntoElement,
    confirm_button: impl IntoElement,
    confirm: Option<ButtonAction>,
    on_cancel: ButtonAction,
    theme: &AppTheme,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let focus_handle: Entity<FocusHandle> =
        window.use_keyed_state(moderation_child_id(id, "focus"), cx, |_, cx| cx.focus_handle());
    let focus_handle = focus_handle.read(cx).clone();
    let layer_cancel = LayerCancel::Dismiss(on_cancel.clone());
    let submit = confirm.clone();
    let key_focus = focus_handle.clone();

    let ceiling = (ModalSheetFrame::for_window(window, cx).content_max_height() - MODAL_OUTLINE_WIDTH * 2.0).max(1.0);
    div()
        .id(moderation_child_id(id, "surface"))
        .track_focus(&focus_handle)
        .flex()
        .flex_col()
        .w_full()
        .min_h_full()
        .max_h(px(ceiling))
        .bg(theme.bg._100)
        .layer_form_keys(layer_cancel, submit.is_some(), move |window, cx| {
            if let Some(submit) = &submit {
                submit(window, cx);
            }
        })
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            let is_plain_return = event.keystroke.key == RETURN_KEY && !event.keystroke.modifiers.modified();
            if is_plain_return
                && key_focus.is_focused(window)
                && let Some(confirm) = &confirm
            {
                cx.stop_propagation();
                confirm(window, cx);
            }
        })
        .child(
            div()
                .id(moderation_child_id(id, "scroll"))
                .flex_auto()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .child(vstack(16.0).align(HorizontalAlignment::Leading).w_full().p(px(20.0)).child(form)),
        )
        .child(ModerationSheetFooter {
            id: moderation_child_id(id, "footer"),
            theme: theme.clone(),
            confirm: confirm_button.into_any_element(),
            on_cancel,
        })
        .into_any_element()
}

fn confirm_label(text: String, foreground: Hsla) -> MainText {
    MainText::new(text).px(15.0).font_weight(FontWeight::SEMIBOLD).color(foreground)
}

fn close_action(cx: &App) -> ButtonAction {
    let close = dismiss(cx);
    Rc::new(move |window: &mut Window, cx: &mut App| close.call_as_function(window, cx))
}

fn sheet_id(name: String) -> ElementId {
    ElementId::from(SharedString::from(name))
}

fn single_value(key: &str, value: &str) -> HashMap<String, String> {
    HashMap::from([(key.to_owned(), value.to_owned())])
}

static RESOLVE_DECISION_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("report_resolve_report", "/decision"));

struct ResolveConfirmSheetState {
    decision: String,
    note: String,
    scope: Option<ModerationMuteScope>,
    duration: Option<ModerationDurationChoice>,
}

#[derive(IntoElement)]
pub struct ResolveConfirmSheet {
    id: ElementId,
    on_submit: Option<ButtonAction>,
    report: ReportRowDTO,
    capabilities: RiseStoredCaseCapabilities,
    action: ResolveAction,
    on_resolved: ButtonAction,
}

impl ResolveConfirmSheet {
    pub fn new(
        report: ReportRowDTO,
        capabilities: RiseStoredCaseCapabilities,
        action: ResolveAction,
        on_resolved: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: sheet_id(format!("resolve-confirm-sheet-{}-{}", report.id, action.id())),
            on_submit: None,
            report,
            capabilities,
            action,
            on_resolved: Rc::new(on_resolved),
        }
    }

    pub fn on_submit(mut self, on_submit: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Some(Rc::new(on_submit));
        self
    }

    fn scope_choices(action: ResolveAction) -> Vec<ModerationMuteScope> {
        match action.block_type_rule() {
            BlockTypeRule::RequiredPartial => UserBlockType::mute_scopes(),
            BlockTypeRule::OptionalFullDefault => UserBlockType::ALL.to_vec(),
            BlockTypeRule::AlwaysFull | BlockTypeRule::Forbidden => Vec::new(),
        }
    }

    fn duration_choices(action: ResolveAction, capabilities: &RiseStoredCaseCapabilities) -> Vec<ModerationDurationChoice> {
        ModerationDurationChoice::choices(
            if action == ResolveAction::Mute { Ladder::Mute } else { Ladder::Account },
            capabilities.max_block_days,
            capabilities.is_admin,
        )
    }

    fn initial_scope(action: ResolveAction, report: &ReportRowDTO) -> Option<ModerationMuteScope> {
        match action.block_type_rule() {
            BlockTypeRule::RequiredPartial => Some(
                report.resolved_scope().map_or(ModerationMuteScope::Post, UserBlockType::default_for),
            ),
            BlockTypeRule::OptionalFullDefault => Some(ModerationMuteScope::Full),
            BlockTypeRule::AlwaysFull | BlockTypeRule::Forbidden => None,
        }
    }
}

impl RenderOnce for ResolveConfirmSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let action = self.action;
        let report_id = self.report.id;
        let tint = if action.is_destructive() { theme.semantic.error_100 } else { theme.primary._100 };
        let is_pending = RiseModerationPresentation::shared(cx).read(cx).resolving_reports().contains(&report_id);
        let scope_choices = Self::scope_choices(action);
        let duration_choices = Self::duration_choices(action, &self.capabilities);

        let initial_scope = Self::initial_scope(action, &self.report);
        let initial_duration = if action.takes_duration() { duration_choices.first().copied() } else { None };
        let state = window.use_keyed_state(moderation_child_id(&self.id, "state"), cx, move |_, _| {
            ResolveConfirmSheetState {
                decision: String::new(),
                note: String::new(),
                scope: initial_scope,
                duration: initial_duration,
            }
        });
        let (decision, note, has_duration) = {
            let current = state.read(cx);
            (current.decision.clone(), current.note.clone(), current.duration.is_some())
        };
        let can_confirm = !(is_pending || (action.takes_duration() && !has_duration));

        let on_submit = self.on_submit;
        let on_resolved = self.on_resolved;
        let confirm_state = state.clone();
        let confirm: ButtonAction = Rc::new(move |window: &mut Window, cx: &mut App| {
            if let Some(on_submit) = &on_submit {
                on_submit(window, cx);
                return;
            }
            let (decision, note, scope, duration) = {
                let current = confirm_state.read(cx);
                (current.decision.clone(), current.note.clone(), current.scope, current.duration)
            };
            let task = ModerationInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.resolve(
                    report_id,
                    action,
                    &decision,
                    scope,
                    duration.map(|duration| duration.wire_days()),
                    Some(&note),
                    cx,
                )
            });
            let on_resolved = on_resolved.clone();
            window
                .spawn(cx, async move |cx| {
                    if task.await {
                        let _ = cx.update(|window, cx| on_resolved(window, cx));
                    }
                })
                .detach();
        });

        let scope_state = state.clone();
        let scope_row = (!scope_choices.is_empty()).then(|| {
            ModerationChoiceRow::new(
                moderation_child_id(&self.id, "scope"),
                tr("case_scope_label"),
                scope_choices,
                |scope: &ModerationMuteScope| scope.id().to_owned(),
                |scope: &ModerationMuteScope| tr(scope.title_key()),
                Binding::entity(
                    scope_state,
                    |state: &ResolveConfirmSheetState| state.scope,
                    |state: &mut ResolveConfirmSheetState, value, _| state.scope = value,
                ),
                theme.clone(),
            )
            .icon(|scope: &ModerationMuteScope| Some(scope.icon_name().to_owned()))
        });

        let duration_state = state.clone();
        let duration_row = action.takes_duration().then(|| {
            ModerationChoiceRow::new(
                moderation_child_id(&self.id, "duration"),
                tr("case_duration_label"),
                duration_choices,
                |choice: &ModerationDurationChoice| choice.id(),
                |choice: &ModerationDurationChoice| ModerationDurationLabel::text(*choice),
                Binding::entity(
                    duration_state,
                    |state: &ResolveConfirmSheetState| state.duration,
                    |state: &mut ResolveConfirmSheetState, value, _| state.duration = value,
                ),
                theme.clone(),
            )
        });
        let duration_cap = self
            .capabilities
            .max_block_days
            .filter(|_| action.takes_duration() && !self.capabilities.is_admin)
            .map(|cap| SecondaryText::new(tr("case_duration_cap").replace("{n}", &cap.to_string())).px(11.0));

        let decision_state = state.clone();
        let decision_field = vstack(6.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(sheet_field_label(tr("moderation_verdict_label"), &theme))
            .child(
                TextAreaUi::new(moderation_child_id(&self.id, "decision"))
                    .values(single_value(report::DECISION, &decision))
                    .name(report::DECISION)
                    .placeholder(tr("moderation_verdict_placeholder"))
                    .min_lines(3)
                    .max_lines(8)
                    .auto_focus(true)
                    .rule(*RESOLVE_DECISION_RULE)
                    .set_value(move |_, value, _, cx| {
                        decision_state.update(cx, |state, cx| {
                            state.decision = value.to_owned();
                            cx.notify();
                        });
                    }),
            )
            .child(SecondaryText::new(tr("case_verdict_visible_hint")).px(11.0));

        let note_state = state.clone();
        let note_field = self.capabilities.can_note.then(|| {
            vstack(6.0)
                .align(HorizontalAlignment::Leading)
                .w_full()
                .child(sheet_field_label(tr("case_note_label"), &theme))
                .child(
                    TextAreaUi::new(moderation_child_id(&self.id, "note"))
                        .values(single_value(report::NOTE, &note))
                        .name(report::NOTE)
                        .placeholder(tr("case_note_placeholder"))
                        .min_lines(2)
                        .max_lines(6)
                        .set_value(move |_, value, _, cx| {
                            note_state.update(cx, |state, cx| {
                                state.note = value.to_owned();
                                cx.notify();
                            });
                        }),
                )
        });

        let press = confirm.clone();
        let form = vstack(16.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(SheetTitle {
                id: moderation_child_id(&self.id, "artwork"),
                icon: action.icon_name().into(),
                tint,
                title: tr(action.title_key()).into(),
                subtitle: tr(action.hint_key()).into(),
                theme: theme.clone(),
            })
            .children(scope_row)
            .children(duration_row)
            .children(duration_cap)
            .child(decision_field)
            .children(note_field);
        let confirm_button = ButtonUi::new(moderation_child_id(&self.id, "confirm"))
            .disabled(!can_confirm)
            .is_pending(is_pending)
            .background_color(tint)
            .on_press(move |window, cx| press(window, cx))
            .child(confirm_label(tr("moderation_confirm_button"), theme.content_on(tint)));
        let on_cancel = close_action(cx);

        moderation_sheet_surface(
            &self.id,
            form,
            confirm_button,
            can_confirm.then_some(confirm),
            on_cancel,
            &theme,
            window,
            cx,
        )
    }
}

static REOPEN_REASON_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("report_reopen_report", "/reason"));

struct ReopenSheetState {
    reason: String,
}

pub type ModerationSheetTextSubmit = Rc<dyn Fn(String, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct ReopenSheet {
    id: ElementId,
    report_id: i64,
    on_done: ButtonAction,
    on_submit: Option<ModerationSheetTextSubmit>,
}

impl ReopenSheet {
    pub fn new(report_id: i64, on_done: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { id: sheet_id(format!("reopen-sheet-{report_id}")), report_id, on_done: Rc::new(on_done), on_submit: None }
    }

    pub fn on_submit(mut self, on_submit: impl Fn(String, &mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Some(Rc::new(on_submit));
        self
    }
}

impl RenderOnce for ReopenSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let report_id = self.report_id;
        let tint = color(REOPEN_TINT);
        let is_pending = RiseModerationPresentation::shared(cx).read(cx).reopening_reports().contains(&report_id);
        let state = window
            .use_keyed_state(moderation_child_id(&self.id, "state"), cx, |_, _| ReopenSheetState { reason: String::new() });
        let reason = state.read(cx).reason.clone();

        let on_submit = self.on_submit;
        let on_done = self.on_done;
        let confirm_state = state.clone();
        let confirm: ButtonAction = Rc::new(move |window: &mut Window, cx: &mut App| {
            let reason = confirm_state.read(cx).reason.clone();
            if let Some(on_submit) = &on_submit {
                on_submit(reason, window, cx);
                return;
            }
            let task =
                ModerationInteractionsStore::shared(cx).update(cx, |store, cx| store.reopen(report_id, &reason, cx));
            let on_done = on_done.clone();
            window
                .spawn(cx, async move |cx| {
                    if task.await {
                        let _ = cx.update(|window, cx| on_done(window, cx));
                    }
                })
                .detach();
        });

        let reason_state = state.clone();
        let form = vstack(16.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(SheetTitle {
                id: moderation_child_id(&self.id, "artwork"),
                icon: "arrow.counterclockwise.circle.fill".into(),
                tint,
                title: tr("case_reopen_title").into(),
                subtitle: tr("case_reopen_subtitle").into(),
                theme: theme.clone(),
            })
            .child(
                vstack(6.0)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .child(sheet_field_label(tr("case_reopen_reason_label"), &theme))
                    .child(
                        TextAreaUi::new(moderation_child_id(&self.id, "reason"))
                            .values(single_value(report::REASON, &reason))
                            .name(report::REASON)
                            .placeholder(tr("case_reopen_reason_placeholder"))
                            .min_lines(2)
                            .max_lines(6)
                            .auto_focus(true)
                            .rule(*REOPEN_REASON_RULE)
                            .set_value(move |_, value, _, cx| {
                                reason_state.update(cx, |state, cx| {
                                    state.reason = value.to_owned();
                                    cx.notify();
                                });
                            }),
                    ),
            );
        let press = confirm.clone();
        let confirm_button = ButtonUi::new(moderation_child_id(&self.id, "confirm"))
            .disabled(is_pending)
            .is_pending(is_pending)
            .background_color(tint)
            .on_press(move |window, cx| press(window, cx))
            .child(confirm_label(tr("case_reopen_confirm"), theme.content_on(tint)));
        let on_cancel = close_action(cx);

        moderation_sheet_surface(
            &self.id,
            form,
            confirm_button,
            (!is_pending).then_some(confirm),
            on_cancel,
            &theme,
            window,
            cx,
        )
    }
}

static SANCTION_REASON_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("report_admin_sanction", "/reason"));

struct AdminSanctionSheetState {
    block_type: Option<UserBlockType>,
    duration: Option<ModerationDurationChoice>,
    address: String,
    reason: String,
}

#[derive(IntoElement)]
pub struct AdminSanctionSheet {
    id: ElementId,
    on_submit: Option<ButtonAction>,
    kind: AdminSanctionKind,
    target_user_id: Option<String>,
    target_name: Option<String>,
    report_id: Option<i64>,
    appeal_id: Option<i64>,
    ip: Option<String>,
    initial_block_type: Option<UserBlockType>,
    on_done: ButtonAction,
}

impl AdminSanctionSheet {
    pub fn new(
        kind: AdminSanctionKind,
        target_user_id: Option<String>,
        target_name: Option<String>,
        report_id: Option<i64>,
        appeal_id: Option<i64>,
        on_done: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: sheet_id(format!("admin-sanction-sheet-{}", kind.raw_value())),
            on_submit: None,
            kind,
            target_user_id,
            target_name,
            report_id,
            appeal_id,
            ip: None,
            initial_block_type: None,
            on_done: Rc::new(on_done),
        }
    }

    pub fn on_submit(mut self, on_submit: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Some(Rc::new(on_submit));
        self
    }

    pub fn ip(mut self, ip: impl Into<Option<String>>) -> Self {
        self.ip = ip.into();
        self
    }

    pub fn initial_block_type(mut self, initial_block_type: impl Into<Option<UserBlockType>>) -> Self {
        self.initial_block_type = initial_block_type.into();
        self
    }

    fn duration_choices() -> Vec<ModerationDurationChoice> {
        ModerationDurationChoice::choices(Ladder::Account, None, true)
    }

    fn can_confirm(kind: AdminSanctionKind, state: &AdminSanctionSheetState, is_sanctioning: bool) -> bool {
        if is_sanctioning {
            return false;
        }
        if kind.takes_ip() && trimming_whitespaces_and_newlines(&state.address).is_empty() {
            return false;
        }
        if kind.takes_duration() && state.duration.is_none() {
            return false;
        }
        character_count(trimming_whitespaces_and_newlines(&state.reason)) >= SANCTION_REASON_MINIMUM_CHARACTERS
    }
}

impl RenderOnce for AdminSanctionSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let kind = self.kind;
        let tint = if kind.is_destructive() { theme.semantic.error_100 } else { theme.primary._100 };
        let is_sanctioning = RiseModerationPresentation::shared(cx).read(cx).is_sanctioning();

        let seed_ip = self.ip.unwrap_or_default();
        let seed_block_type = self.initial_block_type.unwrap_or(UserBlockType::Full);
        let state = window.use_keyed_state(moderation_child_id(&self.id, "state"), cx, move |_, _| {
            AdminSanctionSheetState {
                block_type: Some(seed_block_type),
                duration: Some(ModerationDurationChoice::Permanent),
                address: seed_ip,
                reason: String::new(),
            }
        });
        let (address, reason, can_confirm) = {
            let current = state.read(cx);
            (current.address.clone(), current.reason.clone(), Self::can_confirm(kind, current, is_sanctioning))
        };

        let on_submit = self.on_submit;
        let on_done = self.on_done;
        let target_user_id = self.target_user_id;
        let report_id = self.report_id;
        let appeal_id = self.appeal_id;
        let confirm_state = state.clone();
        let confirm: ButtonAction = Rc::new(move |window: &mut Window, cx: &mut App| {
            if let Some(on_submit) = &on_submit {
                on_submit(window, cx);
                return;
            }
            let (block_type, duration, address, reason) = {
                let current = confirm_state.read(cx);
                (current.block_type, current.duration, current.address.clone(), current.reason.clone())
            };
            let task = ModerationInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.admin_sanction(
                    kind,
                    target_user_id.as_deref(),
                    block_type,
                    duration.map(|duration| duration.wire_days()),
                    Some(&address),
                    Some(&reason),
                    report_id,
                    appeal_id,
                    cx,
                )
            });
            let on_done = on_done.clone();
            window
                .spawn(cx, async move |cx| {
                    if task.await {
                        let _ = cx.update(|window, cx| on_done(window, cx));
                    }
                })
                .detach();
        });

        let block_state = state.clone();
        let block_row = kind.takes_block_type().then(|| {
            ModerationChoiceRow::new(
                moderation_child_id(&self.id, "scope"),
                tr("case_scope_label"),
                UserBlockType::ALL.to_vec(),
                |block_type: &UserBlockType| block_type.id().to_owned(),
                |block_type: &UserBlockType| tr(block_type.title_key()),
                Binding::entity(
                    block_state,
                    |state: &AdminSanctionSheetState| state.block_type,
                    |state: &mut AdminSanctionSheetState, value, _| state.block_type = value,
                ),
                theme.clone(),
            )
            .icon(|block_type: &UserBlockType| Some(block_type.icon_name().to_owned()))
        });

        let address_state = state.clone();
        let press_from_address = confirm.clone();
        let address_can_confirm = can_confirm;
        let address_field = kind.takes_ip().then(|| {
            vstack(6.0)
                .align(HorizontalAlignment::Leading)
                .w_full()
                .child(sheet_field_label(tr("case_ip_label"), &theme))
                .child(
                    InputUi::new(moderation_child_id(&self.id, "ip"))
                        .values(single_value(report::IP, &address))
                        .name(report::IP)
                        .placeholder("0.0.0.0")
                        .auto_focus(true)
                        .on_submit(move |window, cx| {
                            if address_can_confirm {
                                press_from_address(window, cx);
                            }
                        })
                        .set_value(move |_, value, _, cx| {
                            address_state.update(cx, |state, cx| {
                                state.address = value.to_owned();
                                cx.notify();
                            });
                        }),
                )
        });

        let duration_state = state.clone();
        let duration_row = kind.takes_duration().then(|| {
            ModerationChoiceRow::new(
                moderation_child_id(&self.id, "duration"),
                tr("case_duration_label"),
                Self::duration_choices(),
                |choice: &ModerationDurationChoice| choice.id(),
                |choice: &ModerationDurationChoice| ModerationDurationLabel::text(*choice),
                Binding::entity(
                    duration_state,
                    |state: &AdminSanctionSheetState| state.duration,
                    |state: &mut AdminSanctionSheetState, value, _| state.duration = value,
                ),
                theme.clone(),
            )
        });

        let reason_state = state.clone();
        let reason_field = vstack(6.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(sheet_field_label(tr("case_reason_label"), &theme))
            .child(
                TextAreaUi::new(moderation_child_id(&self.id, "reason"))
                    .values(single_value(report::REASON, &reason))
                    .name(report::REASON)
                    .placeholder(tr("case_reason_placeholder"))
                    .min_lines(2)
                    .max_lines(6)
                    .auto_focus(!kind.takes_ip())
                    .rule(*SANCTION_REASON_RULE)
                    .set_value(move |_, value, _, cx| {
                        reason_state.update(cx, |state, cx| {
                            state.reason = value.to_owned();
                            cx.notify();
                        });
                    }),
            );

        let form = vstack(16.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(SheetTitle {
                id: moderation_child_id(&self.id, "artwork"),
                icon: kind.icon_name().into(),
                tint,
                title: tr(kind.title_key()).into(),
                subtitle: self.target_name.unwrap_or_else(|| tr(kind.hint_key())).into(),
                theme: theme.clone(),
            })
            .children(block_row)
            .children(address_field)
            .children(duration_row)
            .child(reason_field);
        let press = confirm.clone();
        let confirm_button = ButtonUi::new(moderation_child_id(&self.id, "confirm"))
            .disabled(!can_confirm)
            .is_pending(is_sanctioning)
            .background_color(tint)
            .on_press(move |window, cx| press(window, cx))
            .child(confirm_label(tr("moderation_confirm_button"), theme.content_on(tint)));
        let on_cancel = close_action(cx);

        moderation_sheet_surface(
            &self.id,
            form,
            confirm_button,
            can_confirm.then_some(confirm),
            on_cancel,
            &theme,
            window,
            cx,
        )
    }
}

static APPEAL_DECISION_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("report_resolve_appeal", "/decision_text"));

struct AppealVerdictSheetState {
    decision_text: String,
}

#[derive(IntoElement)]
pub struct AppealVerdictSheet {
    id: ElementId,
    on_submit: Option<ButtonAction>,
    appeal_id: i64,
    decision: AppealDecision,
    on_resolved: ButtonAction,
}

impl AppealVerdictSheet {
    pub fn new(
        appeal_id: i64,
        decision: AppealDecision,
        on_resolved: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: sheet_id(format!("appeal-verdict-sheet-{appeal_id}-{}", decision.id())),
            on_submit: None,
            appeal_id,
            decision,
            on_resolved: Rc::new(on_resolved),
        }
    }

    pub fn on_submit(mut self, on_submit: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Some(Rc::new(on_submit));
        self
    }
}

impl RenderOnce for AppealVerdictSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let decision = self.decision;
        let appeal_id = self.appeal_id;
        let tint = decision.color(&theme);
        let is_pending = RiseModerationPresentation::shared(cx).read(cx).resolving_appeals().contains(&appeal_id);
        let state = window.use_keyed_state(moderation_child_id(&self.id, "state"), cx, |_, _| AppealVerdictSheetState {
            decision_text: String::new(),
        });
        let decision_text = state.read(cx).decision_text.clone();

        let on_submit = self.on_submit;
        let on_resolved = self.on_resolved;
        let confirm_state = state.clone();
        let confirm: ButtonAction = Rc::new(move |window: &mut Window, cx: &mut App| {
            if let Some(on_submit) = &on_submit {
                on_submit(window, cx);
                return;
            }
            let decision_text = confirm_state.read(cx).decision_text.clone();
            let task = ModerationInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.resolve_appeal(appeal_id, decision, &decision_text, cx));
            let on_resolved = on_resolved.clone();
            window
                .spawn(cx, async move |cx| {
                    if task.await {
                        let _ = cx.update(|window, cx| on_resolved(window, cx));
                    }
                })
                .detach();
        });

        let text_state = state.clone();
        let form = vstack(16.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(SheetTitle {
                id: moderation_child_id(&self.id, "artwork"),
                icon: decision.icon_name().into(),
                tint,
                title: tr(decision.title_key()).into(),
                subtitle: if decision == AppealDecision::Accept {
                    tr("appeal_review_accept_subtitle").into()
                } else {
                    tr("appeal_review_reject_subtitle").into()
                },
                theme: theme.clone(),
            })
            .child(
                vstack(6.0)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .child(sheet_field_label(tr("appeal_review_verdict_label"), &theme))
                    .child(
                        TextAreaUi::new(moderation_child_id(&self.id, "decision-text"))
                            .values(single_value(report::DECISION_TEXT, &decision_text))
                            .name(report::DECISION_TEXT)
                            .placeholder(tr("appeal_review_verdict_placeholder"))
                            .min_lines(3)
                            .max_lines(8)
                            .auto_focus(true)
                            .rule(*APPEAL_DECISION_RULE)
                            .set_value(move |_, value, _, cx| {
                                text_state.update(cx, |state, cx| {
                                    state.decision_text = value.to_owned();
                                    cx.notify();
                                });
                            }),
                    ),
            );
        let press = confirm.clone();
        let confirm_button = ButtonUi::new(moderation_child_id(&self.id, "confirm"))
            .disabled(is_pending)
            .is_pending(is_pending)
            .background_color(tint)
            .on_press(move |window, cx| press(window, cx))
            .child(confirm_label(tr("appeal_review_confirm_button"), theme.content_on(tint)));
        let on_cancel = close_action(cx);

        moderation_sheet_surface(
            &self.id,
            form,
            confirm_button,
            (!is_pending).then_some(confirm),
            on_cancel,
            &theme,
            window,
            cx,
        )
    }
}
