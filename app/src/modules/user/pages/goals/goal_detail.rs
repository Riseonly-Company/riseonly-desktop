use std::f32::consts::PI;

use gpui::{App, Axis, Bounds, Context, ElementId, Entity, FontWeight, Hsla, Pixels, Point, SharedString, Subscription, Window, canvas, fill, point, size, white};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::segmented_picker::SegmentedPicker;
use rise_ui::swiftui::slider::slider;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::user::pages::goals::goal_checklist_section::GoalChecklistSection;
use crate::modules::user::stores::user::user_actions::user_types::GoalItem;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const RING_SIDE: f32 = 180.0;
const RING_LINE_WIDTH: f32 = 14.0;
const RING_SPRING_RESPONSE: f64 = 0.5;
const RING_SPRING_DAMPING: f64 = 0.85;

/// state of the goal detail,
/// title,
/// progress and description drafts plus the progress ring animation
struct GoalDetailState {
    title_rule: RiseFieldRule,
    description_rule: RiseFieldRule,
    editing_title: bool,
    title_draft: String,
    progress_draft: f64,
    description_draft: String,
    progress_ring: Animated<f32>,
    ring_target: f32,
    _store_changes: Subscription,
}

impl GoalDetailState {
    fn new(cx: &mut Context<Self>) -> Self {
        let store = UserServicesStore::shared(cx);
        Self {
            title_rule: RiseFieldRule::new("user_sync_profile_goals", "/goals/*/title"),
            description_rule: RiseFieldRule::new("user_sync_profile_goals", "/goals/*/description"),
            editing_title: false,
            title_draft: String::new(),
            progress_draft: 0.0,
            description_draft: String::new(),
            progress_ring: Animated::new(0.0),
            ring_target: 0.0,
            _store_changes: cx.observe(&store, |_, _, cx| cx.notify()),
        }
    }

    fn set_progress_draft(&mut self, progress: f64) {
        self.progress_draft = progress;
    }

    fn sync_ring(&mut self, progress: f64, cx: &mut Context<Self>) {
        let target = progress.clamp(0.0, 1.0) as f32;
        if target == self.ring_target {
            return;
        }
        self.ring_target = target;
        self.progress_ring.set_in(
            target,
            spring(RING_SPRING_RESPONSE, RING_SPRING_DAMPING),
            RiseClock::monotonic(),
            cx,
        );
    }
}

#[derive(IntoElement)]
pub struct GoalDetail {
    pub goal_id: String,
}

impl GoalDetail {
    pub fn new(goal_id: impl Into<String>) -> Self {
        Self { goal_id: goal_id.into() }
    }

    fn ring_progress(state: &Entity<GoalDetailState>, goal: &GoalItem, cx: &App) -> f64 {
        if goal.is_checklist { goal.checklist_progress() } else { state.read(cx).progress_draft }
    }

    fn progress_ring(state: &Entity<GoalDetailState>, goal: &GoalItem, theme: &AppTheme, window: &Window, cx: &App) -> Div {
        let progress = Self::ring_progress(state, goal, cx);
        let local = state.read(cx);
        let caption = if goal.is_checklist && !goal.subgoals.is_empty() {
            GoalChecklistSection::progress_text(goal)
        } else {
            tr("completed")
        };
        let drawn = local.progress_ring.frame_value(RiseClock::monotonic(), window);
        let track_color = theme.bg._300;
        let ring_color = theme.primary._100;

        div()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .frame_size(RING_SIDE, RING_SIDE)
            .child(
                div().absolute().inset_0().child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, (), window, _| {
                            Self::paint_ring(bounds, drawn, track_color, ring_color, window);
                        },
                    )
                    .size_full(),
                ),
            )
            .child(
                vstack(2.0)
                    .child(
                        MainText::new(format!("{}%", (progress * 100.0) as i64))
                            .px(32.0)
                            .font_weight(FontWeight::BOLD)
                            .color(theme.text.primary),
                    )
                    .child(MainText::new(caption).px(12.0).color(theme.text.secondary)),
            )
    }

    fn paint_ring(bounds: Bounds<Pixels>, progress: f32, track_color: Hsla, ring_color: Hsla, window: &mut Window) {
        let center = bounds.center();
        let radius = RING_SIDE / 2.0;
        Self::stroke_arc(&Self::arc_points(center, radius, 0.0, 2.0 * PI), true, track_color, window);
        if progress <= 0.0 {
            return;
        }
        let arc = Self::arc_points(center, radius, -PI / 2.0, 2.0 * PI * progress);
        Self::stroke_arc(&arc, false, ring_color, window);
        if let (Some(first), Some(last)) = (arc.first(), arc.last()) {
            Self::round_cap(*first, ring_color, window);
            Self::round_cap(*last, ring_color, window);
        }
    }

    fn arc_points(center: Point<Pixels>, radius: f32, start: f32, sweep: f32) -> Vec<Point<Pixels>> {
        let steps = ((sweep.abs() / (2.0 * PI)) * 180.0).ceil().max(2.0) as usize;
        (0..=steps)
            .map(|step| {
                let angle = start + sweep * step as f32 / steps as f32;
                point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()))
            })
            .collect()
    }

    fn stroke_arc(points: &[Point<Pixels>], closed: bool, color: Hsla, window: &mut Window) {
        let mut builder = gpui::PathBuilder::stroke(px(RING_LINE_WIDTH));
        builder.add_polygon(points, closed);
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
    }

    fn round_cap(at: Point<Pixels>, color: Hsla, window: &mut Window) {
        let half = RING_LINE_WIDTH / 2.0;
        let bounds = Bounds::new(point(at.x - px(half), at.y - px(half)), size(px(RING_LINE_WIDTH), px(RING_LINE_WIDTH)));
        window.paint_quad(fill(bounds, color).corner_radii(px(half)));
    }

    fn save_title(state: &Entity<GoalDetailState>, goal_id: &str, cx: &mut App) {
        let title = state.read(cx).title_draft.clone();
        UserServicesStore::shared(cx).update(cx, |store, cx| store.update_goal(goal_id, Some(title), None, cx));
        state.update(cx, |state, cx| {
            state.editing_title = false;
            cx.notify();
        });
    }

    fn cancel_title(state: &Entity<GoalDetailState>, title: &str, cx: &mut App) {
        let title = title.to_owned();
        state.update(cx, |state, cx| {
            state.title_draft = title;
            state.editing_title = false;
            cx.notify();
        });
    }

    fn title_block(state: &Entity<GoalDetailState>, goal: &GoalItem, theme: &AppTheme, cx: &App) -> Div {
        let block = vstack(10.0).w_full();
        if !state.read(cx).editing_title {
            let edit_state = state.clone();
            let edit_title = goal.title.clone();
            return block.child(
                hstack(8.0)
                    .align(VerticalAlignment::FirstTextBaseline)
                    .w_full()
                    .px(px(14.0))
                    .py(px(12.0))
                    .bg(theme.bg._200)
                    .rounded_rect(10.0)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(MainText::new(goal.title.clone()).px(18.0).font_weight(FontWeight::SEMIBOLD)),
                    )
                    .child(
                        plain("goal-detail-edit-title")
                            .on_press(move |_, cx| {
                                let title = edit_title.clone();
                                edit_state.update(cx, |state, cx| {
                                    state.title_draft = title;
                                    state.editing_title = true;
                                    cx.notify();
                                });
                            })
                            .children(IconUi::sized("pencil", px(17.0), theme.primary._100, false)),
                    ),
            );
        }

        let draft = Binding::entity(
            state.clone(),
            |state: &GoalDetailState| state.title_draft.clone(),
            |state: &mut GoalDetailState, value, _| state.title_draft = value,
        );
        let submit_state = state.clone();
        let submit_id = goal.id.clone();
        let escape_state = state.clone();
        let escape_title = goal.title.clone();
        let save_state = state.clone();
        let save_id = goal.id.clone();
        let cancel_state = state.clone();
        let cancel_title = goal.title.clone();

        block
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
                        ShapedTextField::new("goal-detail-title-field", "", state.read(cx).title_rule.binding(draft))
                            .auto_focus(true)
                            .on_submit(move |_, cx| Self::save_title(&submit_state, &submit_id, cx))
                            .on_cancel(move |_, cx| Self::cancel_title(&escape_state, &escape_title, cx)),
                    ),
            )
            .child(
                hstack(10.0)
                    .child(
                        plain("goal-detail-save-title")
                            .on_press(move |_, cx| Self::save_title(&save_state, &save_id, cx))
                            .child(MainText::new(tr("save")).px(14.0).color(theme.primary._100)),
                    )
                    .child(
                        plain("goal-detail-cancel-title")
                            .on_press(move |_, cx| Self::cancel_title(&cancel_state, &cancel_title, cx))
                            .child(MainText::new(tr("cancel")).px(14.0).color(theme.text.secondary)),
                    ),
            )
    }

    fn save_description(state: &Entity<GoalDetailState>, goal_id: &str, cx: &mut App) {
        let draft = state.read(cx).description_draft.clone();
        let store = UserServicesStore::shared(cx);
        let Some(current) = store.read(cx).goal(goal_id) else {
            return;
        };
        if current.description == trimming_whitespaces_and_newlines(&draft) {
            return;
        }
        store.update(cx, |store, cx| store.update_goal_description(goal_id, &draft, cx));
    }

    fn description_block(state: &Entity<GoalDetailState>, goal: &GoalItem, theme: &AppTheme, cx: &App) -> Div {
        let draft = Binding::entity(
            state.clone(),
            |state: &GoalDetailState| state.description_draft.clone(),
            |state: &mut GoalDetailState, value, _| state.description_draft = value,
        );
        let blur_state = state.clone();
        let blur_id = goal.id.clone();
        div()
            .w_full()
            .px(px(14.0))
            .py(px(12.0))
            .bg(theme.bg._200)
            .rounded_rect(10.0)
            .child(
                ShapedTextField::new(
                    "goal-detail-description-field",
                    tr("goal_description_placeholder"),
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

    fn mode_picker(goal: &GoalItem) -> SegmentedPicker {
        let goal_id = goal.id.clone();
        SegmentedPicker::new(
            ElementId::Name(SharedString::from(format!("goal-detail-mode-{}", goal.id))),
            vec![SharedString::from(tr("goal_mode_scale")), SharedString::from(tr("goal_mode_checklist"))],
            usize::from(goal.is_checklist),
            move |index, _, cx| {
                let is_checklist = index == 1;
                UserServicesStore::shared(cx).update(cx, |store, cx| store.set_goal_checklist(&goal_id, is_checklist, cx));
            },
        )
    }

    fn progress_slider(state: &Entity<GoalDetailState>, goal_id: &str, theme: &AppTheme) -> impl IntoElement {
        let draft = Binding::entity(
            state.clone(),
            |state: &GoalDetailState| state.progress_draft,
            |state: &mut GoalDetailState, value, _| state.set_progress_draft(value),
        );
        let editing_state = state.clone();
        let goal_id = goal_id.to_owned();

        vstack(8.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .px(px(14.0))
            .py(px(12.0))
            .bg(theme.bg._200)
            .rounded_rect(10.0)
            .child(MainText::new(tr("progress_label")).px(13.0).color(theme.text.secondary))
            .child(
                slider("goal-detail-progress", draft, 0.0..=1.0)
                    .step(0.01)
                    .tint(theme.primary._100)
                    .on_editing_changed(move |editing, _, cx| {
                        if editing {
                            return;
                        }
                        let progress = editing_state.read(cx).progress_draft;
                        UserServicesStore::shared(cx)
                            .update(cx, |store, cx| store.update_goal(&goal_id, None, Some(progress), cx));
                    }),
            )
            .guide_anchor(RiseGuideAnchor::GoalProgress)
    }

    fn delete_button(goal_id: String, theme: &AppTheme, cx: &App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        ButtonUi::new("goal-detail-delete")
            .background_color(theme.primary._100)
            .height(50.0)
            .on_press(move |_, cx| {
                UserServicesStore::shared(cx).update(cx, |store, cx| store.delete_goal(&goal_id, cx));
                if let Some(nav_store) = &nav_store {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            })
            .child(MainText::new(tr("delete_goal")).px(16.0).font_weight(FontWeight::SEMIBOLD).color(white()))
    }
}

impl RenderOnce for GoalDetail {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(
            ElementId::Name(SharedString::from(format!("goal-detail-{}", self.goal_id))),
            cx,
            |_, cx| GoalDetailState::new(cx),
        );
        let goal = UserServicesStore::shared(cx).read(cx).goal(&self.goal_id).cloned();
        let nav_store = env::<NavigationStoreKey>(cx);

        let content = match goal {
            Some(goal) => {
                let ring = Self::ring_progress(&state, &goal, cx);
                state.update(cx, |state, cx| state.sync_ring(ring, cx));
                let progress_state = state.clone();
                on_change(
                    ElementId::Name(SharedString::from(format!("goal-detail-progress-change-{}", self.goal_id))),
                    goal.progress,
                    false,
                    window,
                    cx,
                    move |_, value, _, cx| {
                        let value = *value;
                        progress_state.update(cx, |state, cx| {
                            state.set_progress_draft(value);
                            cx.notify();
                        });
                    },
                );
                let appear_state = state.clone();
                let appear_title = goal.title.clone();
                let appear_progress = goal.progress;
                let appear_description = goal.description.clone();
                let disappear_state = state.clone();
                let disappear_id = self.goal_id.clone();
                let lower: AnyElement = if goal.is_checklist {
                    GoalChecklistSection::new(goal.clone()).into_any_element()
                } else {
                    Self::progress_slider(&state, &self.goal_id, &theme).into_any_element()
                };
                vstack(24.0)
                    .w_full()
                    .px(px(16.0))
                    .pt(px(12.0))
                    .child(div().pt(px(16.0)).child(Self::progress_ring(&state, &goal, &theme, window, cx)))
                    .child(Self::title_block(&state, &goal, &theme, cx))
                    .child(Self::description_block(&state, &goal, &theme, cx))
                    .child(Self::mode_picker(&goal))
                    .child(lower)
                    .child(Self::delete_button(goal.id.clone(), &theme, cx))
                    .child(div().flex_none().h(px(60.0)))
                    .on_appear(ElementId::Name(SharedString::from(format!("goal-detail-appear-{}", self.goal_id))), move |_, cx| {
                        appear_state.update(cx, |state, cx| {
                            state.title_draft = appear_title;
                            state.set_progress_draft(appear_progress);
                            state.description_draft = appear_description;
                            cx.notify();
                        });
                    })
                    .and_on_disappear(move |_, cx| Self::save_description(&disappear_state, &disappear_id, cx))
                    .into_any_element()
            }
            None => SecondaryText::new(tr("goal_not_found"))
                .px(13.0)
                .tac(TextAlignT::Center)
                .w_full()
                .pt(px(80.0))
                .into_any_element(),
        };

        SettingsWrapperUi::new(tr("goal_detail_title"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(
            scroll_view("goal-detail-scroll")
                .size_full()
                .bg(theme.bg._100)
                .list_shell_scroll_under_chrome(cx)
                .child(content),
        )
    }
}
