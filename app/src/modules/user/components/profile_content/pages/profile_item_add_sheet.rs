use std::rc::Rc;

use gpui::{
    Anchor, App, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, Window,
    anchored, deferred,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_ui::context_menu_ui::DeferredLayerFloorKey;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::date_picker::{DatePicker, DatePickerComponents};
use rise_ui::swiftui::dismiss::{DismissAction, dismiss};
use rise_ui::swiftui::prelude::*;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::modules::user::stores::user::user_actions::user_types::PlanDateFormatting;

const ESCAPE_KEY: &str = "escape";

pub type ProfileItemAddSheetOnSave = Rc<dyn Fn(String, Date, &mut Window, &mut App)>;

/// state of the add sheet, the entered text and date
struct ProfileItemAddSheetState {
    text: String,
    date: Date,
    shows_date_picker: bool,
    date_picker_closed_at: Option<Point<Pixels>>,
    press_only_closed_date_picker: bool,
    focus: FocusHandle,
}

/// bottom sheet that adds a goal or a plan
#[derive(IntoElement)]
pub struct ProfileItemAddSheet {
    pub title: String,
    pub placeholder: String,
    pub rule: RiseFieldRule,
    pub with_date: bool,
    pub guide_anchor: RiseGuideAnchor,
    pub guide_fact: RiseGuideSignal,
    pub on_save: ProfileItemAddSheetOnSave,
}

impl ProfileItemAddSheet {
    pub fn new(
        title: impl Into<String>,
        placeholder: impl Into<String>,
        rule: RiseFieldRule,
        guide_anchor: RiseGuideAnchor,
        guide_fact: RiseGuideSignal,
        on_save: impl Fn(String, Date, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            title: title.into(),
            placeholder: placeholder.into(),
            rule,
            with_date: false,
            guide_anchor,
            guide_fact,
            on_save: Rc::new(on_save),
        }
    }

    pub fn with_date(mut self, with_date: bool) -> Self {
        self.with_date = with_date;
        self
    }

    pub fn height(with_date: bool) -> f32 {
        if with_date { 268.0 } else { 212.0 }
    }

    fn can_save(text: &str) -> bool {
        !trimming_whitespaces_and_newlines(text).is_empty()
    }

    fn save(
        state: &Entity<ProfileItemAddSheetState>,
        on_save: &ProfileItemAddSheetOnSave,
        dismiss: &DismissAction,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (text, date) = {
            let state = state.read(cx);
            (state.text.clone(), state.date)
        };
        if !Self::can_save(&text) {
            return;
        }
        on_save(text, date, window, cx);
        dismiss.call_as_function(window, cx);
    }

    fn compact_date_picker(state: &Entity<ProfileItemAddSheetState>, tint: Hsla, chip: Hsla, cx: &App) -> Div {
        let (date, shows_date_picker) = {
            let state = state.read(cx);
            (state.date, state.shows_date_picker)
        };
        let toggle_state = state.clone();
        let down_state = state.clone();
        let close_state = state.clone();
        let selection = Binding::entity(
            state.clone(),
            |state: &ProfileItemAddSheetState| state.date,
            |state: &mut ProfileItemAddSheetState, value, _| state.date = value,
        );
        let theme = rise_ui::theme(cx);
        let popover_background = theme.bg._100;
        let popover_border = theme.border._100;
        let popover_priority = env::<DeferredLayerFloorKey>(cx) + 1;
        div()
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                down_state.update(cx, |state, _| {
                    state.press_only_closed_date_picker = state.date_picker_closed_at == Some(event.position);
                    state.date_picker_closed_at = None;
                });
            })
            .child(
                plain("profile-item-add-date")
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
                    .bg(chip)
                    .rounded(px(8.0))
                    .child(MainText::new(PlanDateFormatting::display(PlanDateFormatting::ms(date))).px(14.0)),
            )
            .when(shows_date_picker, |picker| {
                picker.child(
                    deferred(
                        anchored().anchor(Anchor::TopRight).snap_to_window_with_margin(px(8.0)).child(
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
                                    DatePicker::new("profile-item-add-date-picker", selection)
                                        .displayed_components(DatePickerComponents::Date)
                                        .tint(tint),
                                ),
                        ),
                    )
                    .with_priority(popover_priority),
                )
            })
    }
}

impl RenderOnce for ProfileItemAddSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let dismiss = dismiss(cx);
        let state = window.use_keyed_state("profile-item-add-sheet-state", cx, |_, cx| ProfileItemAddSheetState {
            text: String::new(),
            date: RiseClock::now(),
            shows_date_picker: false,
            date_picker_closed_at: None,
            press_only_closed_date_picker: false,
            focus: cx.focus_handle(),
        });
        let focus = state.read(cx).focus.clone();
        let escape_state = state.clone();
        let can_save = Self::can_save(&state.read(cx).text);
        let text = Binding::entity(
            state.clone(),
            |state: &ProfileItemAddSheetState| state.text.clone(),
            |state: &mut ProfileItemAddSheetState, value, _| state.text = value,
        );

        let submit_state = state.clone();
        let submit_save = self.on_save.clone();
        let submit_dismiss = dismiss.clone();
        let press_state = state.clone();
        let press_save = self.on_save.clone();
        let press_dismiss = dismiss.clone();
        let appear_fact = self.guide_fact;
        let disappear_fact = self.guide_fact;

        vstack(10.0)
            .align(HorizontalAlignment::Leading)
            .size_full()
            .justify_start()
            .px(px(16.0))
            .pt(px(20.0))
            .pb(px(12.0))
            .bg(theme.bg._100)
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
                div()
                    .w_full()
                    .pb(px(4.0))
                    .child(MainText::new(self.title).px(17.0).font_weight(FontWeight::SEMIBOLD)),
            )
            .child(
                div()
                    .w_full()
                    .min_h(px(44.0))
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .bg(theme.bg._200)
                    .rounded(px(12.0))
                    .child(
                        ShapedTextField::new("profile-item-add-field", self.placeholder, self.rule.binding(text))
                            .auto_focus(true)
                            .on_submit(move |window, cx| {
                                Self::save(&submit_state, &submit_save, &submit_dismiss, window, cx);
                            }),
                    ),
            )
            .when(self.with_date, |form| {
                form.child(
                    hstack(None)
                        .w_full()
                        .min_h(px(44.0))
                        .px(px(12.0))
                        .bg(theme.bg._200)
                        .rounded(px(12.0))
                        .child(MainText::new(tr("plan_date_label")).px(14.0).color(theme.text.secondary))
                        .child(spacer())
                        .child(Self::compact_date_picker(&state, theme.primary._100, theme.bg._300, cx)),
                )
            })
            .child(
                hstack(10.0)
                    .w_full()
                    .pt(px(4.0))
                    .child(
                        plain("profile-item-add-save")
                            .flex_1()
                            .disabled(!can_save)
                            .on_press(move |window, cx| {
                                Self::save(&press_state, &press_save, &press_dismiss, window, cx);
                            })
                            .flex()
                            .items_center()
                            .justify_center()
                            .min_h(px(44.0))
                            .bg(theme.primary._100.opacity(if can_save { 1.0 } else { 0.45 }))
                            .capsule()
                            .child(MainText::new(tr("save")).px(15.0).font_weight(FontWeight::SEMIBOLD).color(theme.content_on(theme.primary._100))),
                    )
                    .child(
                        plain("profile-item-add-cancel")
                            .flex_1()
                            .on_press(move |window, cx| dismiss.call_as_function(window, cx))
                            .flex()
                            .items_center()
                            .justify_center()
                            .min_h(px(44.0))
                            .bg(theme.bg._200)
                            .capsule()
                            .child(MainText::new(tr("cancel")).px(15.0).color(theme.text.secondary)),
                    ),
            )
            .guide_anchor(self.guide_anchor)
            .on_appear("profile-item-add-sheet-appear", move |_, cx| GuideFacts::set(appear_fact, true, cx))
            .and_on_disappear(move |_, cx| GuideFacts::set(disappear_fact, false, cx))
    }
}
