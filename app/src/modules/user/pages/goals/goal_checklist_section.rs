use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Context, ElementId, Entity, Hsla, MouseButton, SharedString, Task, Window};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HOLD_CONTEXT_MENU_DEFAULT_MAXIMUM_PREVIEW_WIDTH, HoldContextMenuPresentationActions,
    HoldContextMenuPresentationModifiers, HoldContextMenuPresentationOnAction, HoldContextMenuPresentationPreview,
};
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::modules::user::stores::user::user_actions::user_types::{GoalItem, GoalSubgoalItem};
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const HOLD_DURATION: Duration = Duration::from_millis(400);
const HOLD_MAXIMUM_DISTANCE: f32 = 10.0;
const LIFT_DISTANCE: f32 = 4.0;
const DROP_SPRING_RESPONSE: f64 = 0.3;
const DROP_SPRING_DAMPING: f64 = 0.85;
const ROW_MIN_HEIGHT: f32 = 44.0;
const ROW_HORIZONTAL_PADDING: f32 = 14.0;
const ROW_SPACING: f32 = 12.0;
const CHECK_SIDE: f32 = 22.0;
const DIVIDER_LEADING: f32 = 44.0;
const CORNER_RADIUS: f32 = 10.0;
const PREVIEW_MAX_WIDTH: f32 = 320.0;

#[derive(IntoElement)]
pub struct GoalChecklistSection {
    goal: GoalItem,
}

impl GoalChecklistSection {
    pub fn new(goal: GoalItem) -> Self {
        Self { goal }
    }

    pub fn progress_text(goal: &GoalItem) -> String {
        tr("goal_subgoals_progress")
            .replace("{done}", &goal.done_subgoal_count().to_string())
            .replace("{total}", &goal.subgoals.len().to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum GoalChecklistSectionField {
    Add,
    Row(String),
}

/// the press half of the press then drag gesture, one press of the row
struct HoldPress {
    subgoal_id: String,
    generation: u64,
    recognized: bool,
}

/// state of the section, drag_offset_motion is the drawn drag offset so a drop can spring back
struct GoalChecklistSectionState {
    subgoal_rule: RiseFieldRule,
    new_title: String,
    editing_id: Option<String>,
    editing_draft: String,
    dragging_id: Option<String>,
    drag_offset: f32,
    drag_base: f32,
    drag_offset_motion: Animated<f32>,
    offset_owner: Option<String>,
    row_height: f32,
    menu_subgoal_id: Option<String>,
    hold: Option<HoldPress>,
    hold_generation: u64,
    hold_task: Option<Task<()>>,
    focus: Option<GoalChecklistSectionField>,
}

impl GoalChecklistSectionState {
    fn new() -> Self {
        Self {
            subgoal_rule: RiseFieldRule::new("user_sync_profile_goals", "/goals/*/steps/*/title"),
            new_title: String::new(),
            editing_id: None,
            editing_draft: String::new(),
            dragging_id: None,
            drag_offset: 0.0,
            drag_base: 0.0,
            drag_offset_motion: Animated::new(0.0),
            offset_owner: None,
            row_height: 0.0,
            menu_subgoal_id: None,
            hold: None,
            hold_generation: 0,
            hold_task: None,
            focus: None,
        }
    }

    fn begin_rename(&mut self, goal_id: &str, subgoal: &GoalSubgoalItem, cx: &mut Context<Self>) {
        if let Some(current) = self.editing_id.clone()
            && current != subgoal.id
        {
            self.commit_rename(goal_id, &current, cx);
        }
        self.editing_draft = subgoal.title.clone();
        self.editing_id = Some(subgoal.id.clone());
        self.focus = Some(GoalChecklistSectionField::Row(subgoal.id.clone()));
        cx.notify();
    }

    fn commit_rename(&mut self, goal_id: &str, subgoal_id: &str, cx: &mut Context<Self>) {
        let title = self.editing_draft.clone();
        UserServicesStore::shared(cx).update(cx, |store, cx| store.rename_subgoal(goal_id, subgoal_id, &title, cx));
        if self.editing_id.as_deref() == Some(subgoal_id) {
            self.editing_id = None;
        }
        cx.notify();
    }

    fn commit_add(&mut self, goal_id: &str, cx: &mut Context<Self>) {
        if trimming_whitespaces_and_newlines(&self.new_title).is_empty() {
            return;
        }
        let title = std::mem::take(&mut self.new_title);
        UserServicesStore::shared(cx).update(cx, |store, cx| store.add_subgoal(goal_id, &title, cx));
        self.focus = Some(GoalChecklistSectionField::Add);
        cx.notify();
    }

    fn delete_subgoal(&mut self, goal_id: &str, subgoal_id: &str, cx: &mut Context<Self>) {
        UserServicesStore::shared(cx).update(cx, |store, cx| store.delete_subgoal(goal_id, subgoal_id, cx));
        if self.editing_id.as_deref() == Some(subgoal_id) {
            self.editing_id = None;
        }
        cx.notify();
    }

    fn press_down(&mut self, subgoal_id: &str) -> u64 {
        self.hold_generation = self.hold_generation.wrapping_add(1);
        self.hold = Some(HoldPress { subgoal_id: subgoal_id.to_owned(), generation: self.hold_generation, recognized: false });
        self.hold_generation
    }

    fn hold_elapsed(&mut self, generation: u64) {
        if let Some(hold) = self.hold.as_mut()
            && hold.generation == generation
        {
            hold.recognized = true;
        }
    }

    fn press_moved(&mut self, goal_id: &str, subgoal_id: &str, translation_x: f32, translation_y: f32, cx: &mut Context<Self>) {
        let Some(hold) = self.hold.as_ref() else {
            return;
        };
        if hold.subgoal_id != subgoal_id {
            return;
        }
        if !hold.recognized {
            if translation_x.hypot(translation_y) > HOLD_MAXIMUM_DISTANCE {
                self.hold = None;
            }
            return;
        }
        self.drag_changed(goal_id, subgoal_id, translation_y, cx);
    }

    fn drag_changed(&mut self, goal_id: &str, subgoal_id: &str, translation: f32, cx: &mut Context<Self>) {
        if self.dragging_id.as_deref() != Some(subgoal_id) {
            if translation.abs() <= LIFT_DISTANCE {
                return;
            }
            self.dragging_id = Some(subgoal_id.to_owned());
            self.offset_owner = Some(subgoal_id.to_owned());
            self.drag_base = 0.0;
        }
        self.drag_offset = translation - self.drag_base;
        let step = self.row_height;
        if step > 0.0 {
            self.reorder_if_passed(goal_id, subgoal_id, step, cx);
        }
        self.drag_offset_motion.jump(self.drag_offset);
        cx.notify();
    }

    fn reorder_if_passed(&mut self, goal_id: &str, subgoal_id: &str, step: f32, cx: &mut Context<Self>) {
        let store = UserServicesStore::shared(cx);
        let list: Vec<String> = store
            .read(cx)
            .goal(goal_id)
            .map(|goal| goal.subgoals.iter().map(|subgoal| subgoal.id.clone()).collect())
            .unwrap_or_default();
        let Some(index) = list.iter().position(|id| id == subgoal_id) else {
            return;
        };
        if self.drag_offset > step / 2.0 && index + 1 < list.len() {
            let target = list[index + 1].clone();
            store.update(cx, |store, cx| store.move_subgoal(goal_id, subgoal_id, &target, cx));
            self.drag_base += step;
            self.drag_offset -= step;
        } else if self.drag_offset < -step / 2.0 && index > 0 {
            let target = list[index - 1].clone();
            store.update(cx, |store, cx| store.move_subgoal(goal_id, subgoal_id, &target, cx));
            self.drag_base -= step;
            self.drag_offset += step;
        }
    }

    /// end of the sequenced gesture, the press is taken so a second end is a no-op
    fn press_ended(&mut self, subgoal_id: &str, cx: &mut Context<Self>) {
        let Some(hold) = self.hold.take() else {
            return;
        };
        if !hold.recognized || hold.subgoal_id != subgoal_id {
            return;
        }
        if self.dragging_id.as_deref() == Some(subgoal_id) {
            self.dragging_id = None;
            self.drag_offset = 0.0;
            self.drag_offset_motion.set_in(
                0.0,
                spring(DROP_SPRING_RESPONSE, DROP_SPRING_DAMPING),
                RiseClock::monotonic(),
                cx,
            );
            self.drag_base = 0.0;
        } else {
            self.menu_subgoal_id = Some(subgoal_id.to_owned());
        }
        cx.notify();
    }
}

impl RenderOnce for GoalChecklistSection {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(
            ElementId::Name(SharedString::from(format!("goal-checklist-section-{}", self.goal.id))),
            cx,
            |_: &mut Window, _: &mut Context<GoalChecklistSectionState>| GoalChecklistSectionState::new(),
        );
        let goal = self.goal;
        let can_add = goal.subgoals.len() < UserServicesStore::max_goal_subgoals();

        let mut column = vstack(0.0).items_start().w_full().py(px(6.0)).bg(theme.bg._200).rounded(px(CORNER_RADIUS)).overflow_hidden();
        column = column.children(Self::header(&goal, &theme));
        for subgoal in &goal.subgoals {
            column = column.child(Self::row(&state, &goal.id, subgoal, &theme, window, cx)).child(Self::divider(&theme));
        }
        if can_add {
            column = column.child(Self::add_row(&state, &goal.id, &theme, cx));
        }
        column
    }
}

impl GoalChecklistSection {
    fn header(goal: &GoalItem, theme: &AppTheme) -> Vec<AnyElement> {
        let mut parts = vec![
            hstack(DEFAULT_STACK_SPACING)
                .w_full()
                .px(px(ROW_HORIZONTAL_PADDING))
                .py(px(8.0))
                .child(MainText::new(tr("goal_subgoals_title")).px(13.0).color(theme.text.secondary))
                .child(spacer())
                .when(!goal.subgoals.is_empty(), |header| {
                    header.child(MainText::new(Self::progress_text(goal)).px(13.0).color(theme.text.secondary))
                })
                .into_any_element(),
        ];
        if goal.subgoals.is_empty() {
            parts.push(
                div()
                    .px(px(ROW_HORIZONTAL_PADDING))
                    .pb(px(6.0))
                    .child(SecondaryText::new(tr("goal_subgoals_empty")).px(13.0))
                    .into_any_element(),
            );
        }
        parts
    }

    fn divider(theme: &AppTheme) -> Div {
        div().w_full().pl(px(DIVIDER_LEADING)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55)))
    }

    fn row(
        state: &Entity<GoalChecklistSectionState>,
        goal_id: &str,
        subgoal: &GoalSubgoalItem,
        theme: &AppTheme,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let local = state.read(cx);
        let is_editing = local.editing_id.as_deref() == Some(subgoal.id.as_str());
        let is_dragging = local.dragging_id.as_deref() == Some(subgoal.id.as_str());
        let offset = if local.offset_owner.as_deref() == Some(subgoal.id.as_str()) {
            local.drag_offset_motion.frame_value(RiseClock::monotonic(), window)
        } else {
            0.0
        };
        let row_focused = local.focus == Some(GoalChecklistSectionField::Row(subgoal.id.clone()));
        let subgoal_rule_binding = local.subgoal_rule.binding(Binding::entity(
            state.clone(),
            |state: &GoalChecklistSectionState| state.editing_draft.clone(),
            |state: &mut GoalChecklistSectionState, value, _| state.editing_draft = value,
        ));

        let toggle_goal = goal_id.to_owned();
        let toggle_id = subgoal.id.clone();
        let check_symbol = if subgoal.done { "checkmark.circle.fill" } else { "circle" };
        let check_color = if subgoal.done { theme.primary._100 } else { theme.text.secondary };
        let toggle = plain(ElementId::Name(SharedString::from(format!("goal-subgoal-toggle-{}", subgoal.id))))
            .on_press(move |_, cx| {
                UserServicesStore::shared(cx).update(cx, |store, cx| store.toggle_subgoal(&toggle_goal, &toggle_id, cx));
            })
            .child(div().flex_none().children(IconUi::sized(check_symbol, px(CHECK_SIDE), check_color, false)));

        let mut row = hstack(ROW_SPACING)
            .relative()
            .top(px(offset))
            .w_full()
            .min_h(px(ROW_MIN_HEIGHT))
            .px(px(ROW_HORIZONTAL_PADDING))
            .bg(theme.bg._200)
            .opacity(if is_dragging { 0.85 } else { 1.0 })
            .child(toggle);

        if is_editing {
            let submit_state = state.clone();
            let submit_goal = goal_id.to_owned();
            let submit_id = subgoal.id.clone();
            let delete_state = state.clone();
            let delete_goal = goal_id.to_owned();
            let delete_id = subgoal.id.clone();
            row = row
                .child(
                    div().flex_1().min_w_0().child(
                        ShapedTextField::new(
                            ElementId::Name(SharedString::from(format!("goal-subgoal-field-{}", subgoal.id))),
                            "",
                            subgoal_rule_binding,
                        )
                        .auto_focus(row_focused)
                        .on_submit(move |_, cx| {
                            submit_state.update(cx, |state, cx| state.commit_rename(&submit_goal, &submit_id, cx));
                        }),
                    ),
                )
                .child(
                    plain(ElementId::Name(SharedString::from(format!("goal-subgoal-delete-{}", subgoal.id))))
                        .on_press(move |_, cx| {
                            delete_state.update(cx, |state, cx| state.delete_subgoal(&delete_goal, &delete_id, cx));
                        })
                        .child(div().flex_none().children(IconUi::sized("trash", px(17.0), theme.semantic.error_100, false))),
                );
        } else {
            let rename_state = state.clone();
            let rename_goal = goal_id.to_owned();
            let rename_subgoal = subgoal.clone();
            let title_color = if subgoal.done { theme.text.secondary } else { theme.text.primary };
            row = row.child(
                div()
                    .id(ElementId::Name(SharedString::from(format!("goal-subgoal-title-{}", subgoal.id))))
                    .flex_1()
                    .min_w_0()
                    .when(subgoal.done, |title| title.line_through())
                    .on_click(move |_, _, cx| {
                        rename_state.update(cx, |state, cx| state.begin_rename(&rename_goal, &rename_subgoal, cx));
                    })
                    .child(MainText::new(subgoal.title.clone()).px(15.0).color(title_color)),
            );
        }

        let measured = state.clone();
        let row = row.child(measure(
            ElementId::Name(SharedString::from(format!("goal-subgoal-height-{}", subgoal.id))),
            move |bounds, _, cx| {
                let height = f32::from(bounds.size.height);
                measured.update(cx, |state, _| state.row_height = height);
            },
        ));

        let content = if is_editing {
            row.into_any_element()
        } else {
            Self::hold_gesture(row, state, goal_id, &subgoal.id).into_any_element()
        };
        Self::row_menu(content, state, goal_id, subgoal, theme.bg._200, theme.text.primary)
    }

    fn hold_gesture(row: Div, state: &Entity<GoalChecklistSectionState>, goal_id: &str, subgoal_id: &str) -> impl IntoElement {
        let down_state = state.clone();
        let down_id = subgoal_id.to_owned();
        let up_state = state.clone();
        let up_id = subgoal_id.to_owned();
        let move_state = state.clone();
        let move_goal = goal_id.to_owned();
        let move_id = subgoal_id.to_owned();
        let end_state = state.clone();
        let end_id = subgoal_id.to_owned();
        let gesture = DragGesture::new(0.0)
            .on_changed(move |value: &DragGestureValue, _: &mut Window, cx: &mut App| {
                let translation_x = f32::from(value.translation.width);
                let translation_y = f32::from(value.translation.height);
                move_state.update(cx, |state, cx| state.press_moved(&move_goal, &move_id, translation_x, translation_y, cx));
            })
            .on_ended(move |_: &DragGestureValue, _: &mut Window, cx: &mut App| {
                end_state.update(cx, |state, cx| state.press_ended(&end_id, cx));
            });
        row.on_mouse_down(MouseButton::Left, move |_, _, cx| Self::press_down(&down_state, &down_id, cx))
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                up_state.update(cx, |state, cx| state.press_ended(&up_id, cx));
            })
            .gesture(ElementId::Name(SharedString::from(format!("goal-subgoal-hold-{subgoal_id}"))), gesture)
    }

    fn press_down(state: &Entity<GoalChecklistSectionState>, subgoal_id: &str, cx: &mut App) {
        let generation = state.update(cx, |state, _| state.press_down(subgoal_id));
        let weak = state.downgrade();
        let task = cx.spawn(async move |cx| {
            cx.background_executor().timer(HOLD_DURATION).await;
            let _ = weak.update(cx, |state, _| state.hold_elapsed(generation));
        });
        state.update(cx, |state, _| state.hold_task = Some(task));
    }

    fn row_menu(
        content: AnyElement,
        state: &Entity<GoalChecklistSectionState>,
        goal_id: &str,
        subgoal: &GoalSubgoalItem,
        surface: Hsla,
        text_color: Hsla,
    ) -> AnyElement {
        let get_id = subgoal.id.clone();
        let set_id = subgoal.id.clone();
        let is_presented = Binding::entity(
            state.clone(),
            move |state: &GoalChecklistSectionState| state.menu_subgoal_id.as_deref() == Some(get_id.as_str()),
            move |state: &mut GoalChecklistSectionState, presented: bool, cx| {
                if !presented && state.menu_subgoal_id.as_deref() == Some(set_id.as_str()) {
                    state.menu_subgoal_id = None;
                    cx.notify();
                }
            },
        );
        let actions: HoldContextMenuPresentationActions = Rc::new(|_: &App| {
            vec![
                HoldContextMenuAction::new("edit", tr("ctx_edit")).system_image("pencil"),
                HoldContextMenuAction::new("delete", tr("ctx_delete")).system_image("trash").destructive(true),
            ]
        });
        let action_state = state.clone();
        let action_goal = goal_id.to_owned();
        let action_subgoal = subgoal.clone();
        let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            match id.as_str() {
                "edit" => action_state.update(cx, |state, cx| state.begin_rename(&action_goal, &action_subgoal, cx)),
                "delete" => {
                    UserServicesStore::shared(cx)
                        .update(cx, |store, cx| store.delete_subgoal(&action_goal, &action_subgoal.id, cx));
                }
                _ => {}
            }
        });
        let preview_title = subgoal.title.clone();
        let preview: HoldContextMenuPresentationPreview = Rc::new(move |width: f32, _: &mut Window, _: &mut App| {
            div()
                .w(px(width.min(PREVIEW_MAX_WIDTH)))
                .min_h(px(ROW_MIN_HEIGHT))
                .px(px(ROW_HORIZONTAL_PADDING))
                .flex()
                .items_center()
                .bg(surface)
                .rounded(px(CORNER_RADIUS))
                .child(MainText::new(preview_title.clone()).px(15.0).color(text_color).line_limit(2))
                .into_any_element()
        });
        content
            .hold_context_menu(
                ElementId::Name(SharedString::from(format!("goal-subgoal-menu-{}", subgoal.id))),
                is_presented,
                HOLD_CONTEXT_MENU_DEFAULT_MAXIMUM_PREVIEW_WIDTH,
                actions,
                on_action,
                preview,
            )
            .into_any_element()
    }

    fn add_row(state: &Entity<GoalChecklistSectionState>, goal_id: &str, theme: &AppTheme, cx: &App) -> Div {
        let add_focused = state.read(cx).focus == Some(GoalChecklistSectionField::Add);
        let binding = state.read(cx).subgoal_rule.binding(Binding::entity(
            state.clone(),
            |state: &GoalChecklistSectionState| state.new_title.clone(),
            |state: &mut GoalChecklistSectionState, value, _| state.new_title = value,
        ));
        let submit_state = state.clone();
        let submit_goal = goal_id.to_owned();
        hstack(ROW_SPACING)
            .w_full()
            .min_h(px(ROW_MIN_HEIGHT))
            .px(px(ROW_HORIZONTAL_PADDING))
            .child(div().flex_none().children(IconUi::sized("plus.circle.fill", px(CHECK_SIDE), theme.primary._100, false)))
            .child(
                div().flex_1().min_w_0().child(
                    ShapedTextField::new("goal-subgoal-add-field", tr("goal_subgoal_placeholder"), binding)
                        .auto_focus(add_focused)
                        .on_submit(move |_, cx| {
                            submit_state.update(cx, |state, cx| state.commit_add(&submit_goal, cx));
                        }),
                ),
            )
    }
}
