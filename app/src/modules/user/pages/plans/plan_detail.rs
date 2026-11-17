use gpui::{
    Anchor, App, Axis, Context, ElementId, Entity, FocusHandle, Hsla, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point,
    SharedString, Subscription, Window, anchored, deferred,
};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::date_picker::{DatePicker, DatePickerComponents};
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::user::stores::user::user_actions::user_types::{PlanDateFormatting, PlanItem};
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const ESCAPE_KEY: &str = "escape";

/// state of the plan detail,
/// title and date drafts,
/// the open calendar stands for the picker popover
struct PlanDetailState {
    title_rule: RiseFieldRule,
    description_rule: RiseFieldRule,
    title_draft: String,
    date_draft: Date,
    description_draft: String,
    shows_date_picker: bool,
    date_picker_closed_at: Option<Point<Pixels>>,
    press_only_closed_date_picker: bool,
    focus: FocusHandle,
    _store_changes: Subscription,
}

impl PlanDetailState {
    fn new(cx: &mut Context<Self>) -> Self {
        let store = UserServicesStore::shared(cx);
        Self {
            title_rule: RiseFieldRule::new("user_sync_profile_plans", "/plans/*/title"),
            description_rule: RiseFieldRule::new("user_sync_profile_plans", "/plans/*/description"),
            title_draft: String::new(),
            date_draft: RiseClock::now(),
            description_draft: String::new(),
            shows_date_picker: false,
            date_picker_closed_at: None,
            press_only_closed_date_picker: false,
            focus: cx.focus_handle(),
            _store_changes: cx.observe(&store, |_, _, cx| cx.notify()),
        }
    }
}

#[derive(IntoElement)]
pub struct PlanDetail {
    pub plan_id: String,
}

impl PlanDetail {
    pub fn new(plan_id: impl Into<String>) -> Self {
        Self { plan_id: plan_id.into() }
    }

    fn editable_field(state: &Entity<PlanDetailState>, plan: &PlanItem, theme: &AppTheme, cx: &App) -> Div {
        let draft = Binding::entity(
            state.clone(),
            |state: &PlanDetailState| state.title_draft.clone(),
            |state: &mut PlanDetailState, value, _| state.title_draft = value,
        );
        let commit_state = state.clone();
        let commit_id = plan.id.clone();
        let escape_state = state.clone();
        let escape_title = plan.title.clone();

        vstack(6.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(MainText::new(tr("plan_title_label")).px(12.0).color(theme.text.secondary))
            .child(
                div()
                    .flex()
                    .items_center()
                    .w_full()
                    .min_h(px(44.0))
                    .px(px(12.0))
                    .bg(theme.bg._200)
                    .rounded_rect(10.0)
                    .child(
                        ShapedTextField::new("plan-detail-title-field", "", state.read(cx).title_rule.binding(draft))
                            .on_submit(move |_, cx| {
                                let title = commit_state.read(cx).title_draft.clone();
                                UserServicesStore::shared(cx)
                                    .update(cx, |store, cx| store.update_plan(&commit_id, Some(title), None, cx));
                            })
                            .on_cancel(move |_, cx| {
                                let title = escape_title.clone();
                                escape_state.update(cx, |state, cx| {
                                    state.title_draft = title;
                                    cx.notify();
                                });
                            }),
                    ),
            )
    }

    fn save_description(state: &Entity<PlanDetailState>, plan_id: &str, cx: &mut App) {
        let draft = state.read(cx).description_draft.clone();
        let store = UserServicesStore::shared(cx);
        let Some(current) = store.read(cx).plan(plan_id) else {
            return;
        };
        if current.description == trimming_whitespaces_and_newlines(&draft) {
            return;
        }
        store.update(cx, |store, cx| store.update_plan_description(plan_id, &draft, cx));
    }

    fn description_block(state: &Entity<PlanDetailState>, plan_id: &str, theme: &AppTheme, cx: &App) -> Div {
        let draft = Binding::entity(
            state.clone(),
            |state: &PlanDetailState| state.description_draft.clone(),
            |state: &mut PlanDetailState, value, _| state.description_draft = value,
        );
        let blur_state = state.clone();
        let blur_id = plan_id.to_owned();
        div()
            .w_full()
            .px(px(14.0))
            .py(px(12.0))
            .bg(theme.bg._200)
            .rounded_rect(10.0)
            .child(
                ShapedTextField::new(
                    "plan-detail-description-field",
                    tr("plan_description_placeholder"),
                    state.read(cx).description_rule.binding(draft),
                )
                .axis(Axis::Vertical)
                .max_visible_lines(8)
                .font_size(15.0)
                .on_focus_change(move |focused, _, cx| {
                    if !focused {
                        Self::save_description(&blur_state, &blur_id, cx);
                    }
                }),
            )
    }

    fn date_picker_field(state: &Entity<PlanDetailState>, theme: &AppTheme, cx: &App) -> Div {
        let (date, shows_date_picker) = {
            let state = state.read(cx);
            (state.date_draft, state.shows_date_picker)
        };
        let toggle_state = state.clone();
        let down_state = state.clone();
        let close_state = state.clone();
        let selection = Binding::entity(
            state.clone(),
            |state: &PlanDetailState| state.date_draft,
            |state: &mut PlanDetailState, value, _| state.date_draft = value,
        );
        let tint: Hsla = theme.primary._100;
        let popover_background = theme.bg._100;
        let popover_border = theme.border._100;

        vstack(6.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(MainText::new(tr("plan_date_label")).px(12.0).color(theme.text.secondary))
            .child(
                hstack(None)
                    .w_full()
                    .min_h(px(44.0))
                    .px(px(12.0))
                    .bg(theme.bg._200)
                    .rounded_rect(10.0)
                    .child(
                        div()
                            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                                down_state.update(cx, |state, _| {
                                    state.press_only_closed_date_picker = state.date_picker_closed_at == Some(event.position);
                                    state.date_picker_closed_at = None;
                                });
                            })
                            .child(
                                plain("plan-detail-date")
                                    .on_press(move |_, cx| {
                                        toggle_state.update(cx, |state, cx| {
                                            if state.press_only_closed_date_picker {
                                                state.press_only_closed_date_picker = false;
                                                return;
                                            }
                                            state.shows_date_picker = !state.shows_date_picker;
                                            cx.notify();
                                        });
                                    })
                                    .px(px(10.0))
                                    .py(px(6.0))
                                    .bg(theme.bg._300)
                                    .rounded(px(8.0))
                                    .child(MainText::new(PlanDateFormatting::display(PlanDateFormatting::ms(date))).px(14.0)),
                            )
                            .when(shows_date_picker, |picker| {
                                picker.child(deferred(
                                    anchored().anchor(Anchor::TopLeft).snap_to_window_with_margin(px(8.0)).child(
                                        div()
                                            .occlude()
                                            .w(px(300.0))
                                            .p(px(12.0))
                                            .bg(popover_background)
                                            .border_1()
                                            .border_color(popover_border)
                                            .rounded(px(12.0))
                                            .on_mouse_down_out(move |event: &MouseDownEvent, _, cx| {
                                                close_state.update(cx, |state, cx| {
                                                    state.shows_date_picker = false;
                                                    state.date_picker_closed_at = Some(event.position);
                                                    cx.notify();
                                                });
                                            })
                                            .child(
                                                DatePicker::new("plan-detail-date-picker", selection)
                                                    .displayed_components(DatePickerComponents::Date)
                                                    .tint(tint),
                                            ),
                                    ),
                                ))
                            }),
                    )
                    .child(spacer()),
            )
    }

    fn delete_button(plan_id: String, theme: &AppTheme, cx: &App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let error = theme.semantic.error_100;
        plain("plan-detail-delete")
            .w_full()
            .on_press(move |_, cx| {
                UserServicesStore::shared(cx).update(cx, |store, cx| store.delete_plan(&plan_id, cx));
                if let Some(nav_store) = &nav_store {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            })
            .child(
                hstack(8.0)
                    .w_full()
                    .justify_center()
                    .py(px(12.0))
                    .bg(error.opacity(0.12))
                    .capsule()
                    .children(IconUi::sized("trash", px(17.0), error, false))
                    .child(MainText::new(tr("delete_plan")).px(15.0).color(error)),
            )
    }
}

impl RenderOnce for PlanDetail {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(
            ElementId::Name(SharedString::from(format!("plan-detail-{}", self.plan_id))),
            cx,
            |_, cx| PlanDetailState::new(cx),
        );
        let plan = UserServicesStore::shared(cx).read(cx).plan(&self.plan_id).cloned();
        let nav_store = env::<NavigationStoreKey>(cx);
        let focus = state.read(cx).focus.clone();
        let escape_state = state.clone();

        let content = match plan {
            Some(plan) => {
                let change_id = self.plan_id.clone();
                let date_ms = PlanDateFormatting::ms(state.read(cx).date_draft);
                on_change(
                    ElementId::Name(SharedString::from(format!("plan-detail-date-change-{}", self.plan_id))),
                    date_ms,
                    false,
                    window,
                    cx,
                    move |_, ms, _, cx| {
                        let store = UserServicesStore::shared(cx);
                        if store.read(cx).plan(&change_id).map(|plan| plan.date_ms) == Some(*ms) {
                            return;
                        }
                        store.update(cx, |store, cx| store.update_plan(&change_id, None, Some(*ms), cx));
                    },
                );

                let appear_state = state.clone();
                let appear_title = plan.title.clone();
                let appear_description = plan.description.clone();
                let disappear_state = state.clone();
                let disappear_id = self.plan_id.clone();
                let appear_date = PlanDateFormatting::date(plan.date_ms);
                vstack(16.0)
                    .w_full()
                    .px(px(16.0))
                    .pt(px(12.0))
                    .child(
                        div().pt(px(16.0)).child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .flex_none()
                                .frame_size(88.0, 88.0)
                                .bg(theme.primary._100.opacity(0.18))
                                .circle()
                                .children(IconUi::sized("calendar", px(38.0), theme.primary._100, false)),
                        ),
                    )
                    .child(Self::editable_field(&state, &plan, &theme, cx))
                    .child(Self::description_block(&state, &plan.id, &theme, cx))
                    .child(Self::date_picker_field(&state, &theme, cx))
                    .child(Self::delete_button(plan.id.clone(), &theme, cx))
                    .child(div().flex_none().h(px(60.0)))
                    .on_appear(ElementId::Name(SharedString::from(format!("plan-detail-appear-{}", self.plan_id))), move |_, cx| {
                        appear_state.update(cx, |state, cx| {
                            state.title_draft = appear_title;
                            state.description_draft = appear_description;
                            if let Some(date) = appear_date {
                                state.date_draft = date;
                            }
                            cx.notify();
                        });
                    })
                    .and_on_disappear(move |_, cx| Self::save_description(&disappear_state, &disappear_id, cx))
                    .into_any_element()
            }
            None => SecondaryText::new(tr("plan_not_found"))
                .px(13.0)
                .tac(TextAlignT::Center)
                .w_full()
                .pt(px(80.0))
                .into_any_element(),
        };

        div()
            .size_full()
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if event.keystroke.key != ESCAPE_KEY || !escape_state.read(cx).shows_date_picker {
                    return;
                }
                cx.stop_propagation();
                escape_state.update(cx, |state, cx| {
                    state.shows_date_picker = false;
                    cx.notify();
                });
            })
            .child(
                SettingsWrapperUi::new(tr("plan_detail_title"), move |_, cx| {
                    if let Some(nav_store) = &nav_store {
                        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                    }
                })
                .content(
                    scroll_view("plan-detail-scroll")
                        .size_full()
                        .bg(theme.bg._100)
                        .list_shell_scroll_under_chrome(cx)
                        .child(content),
                ),
            )
    }
}
