use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use gpui::{App, Context, ElementId, Entity, FontWeight, Hsla, SharedString, Window};
use rise_core::foundation::foundation_date::RiseClock;
use rise_canon::{RiseGuideAnchor, RiseGuideSignal, RiseLimits};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::progress_view::progress_view_value;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::AnimatedTabsPagerLiveState;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::chat::components::chat_item::chat_item::ChatsListRowActionsSuppressorKey;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::user::components::profile_content::pages::profile_item_add_sheet::ProfileItemAddSheet;
use crate::modules::user::pages::goals::goal_checklist_section::GoalChecklistSection;
use crate::modules::user::stores::user::user_actions::user_types::GoalItem;
use crate::modules::user::stores::user::user_services::user_guide_sandbox_hooks::user_guide_sandbox_hooks;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

static TITLE_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("user_sync_profile_goals", "/goals/*/title"));
static MAX_GOALS: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("max_goals"));

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

/// which goals are expanded, each chevron turns with a spring
#[derive(Default)]
struct GoalsDisclosureState {
    expanded_ids: HashSet<String>,
    chevrons: HashMap<String, Animated<f32>>,
}

impl GoalsDisclosureState {
    fn toggle(&mut self, goal_id: &str, cx: &mut Context<Self>) {
        let expanded = if self.expanded_ids.remove(goal_id) {
            false
        } else {
            self.expanded_ids.insert(goal_id.to_owned());
            true
        };
        self.chevrons.entry(goal_id.to_owned()).or_insert_with(|| Animated::new(0.0)).set_in(
            if expanded { 180.0 } else { 0.0 },
            spring(0.3, 0.85),
            RiseClock::monotonic(),
            cx,
        );
        cx.notify();
    }
}

#[derive(IntoElement)]
pub struct Goals {
    id: ElementId,
    pub is_own_profile: bool,
    pub foreign_goals: Vec<GoalItem>,
}

impl Goals {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), is_own_profile: true, foreign_goals: Vec::new() }
    }

    pub fn is_own_profile(mut self, is_own_profile: bool) -> Self {
        self.is_own_profile = is_own_profile;
        self
    }

    pub fn foreign_goals(mut self, foreign_goals: Vec<GoalItem>) -> Self {
        self.foreign_goals = foreign_goals;
        self
    }

    fn is_tap_suppressed(suppressor: Option<&Entity<AnimatedTabsPagerLiveState>>, cx: &App) -> bool {
        suppressor.is_some_and(|suppressor| suppressor.read(cx).suppresses_tab_taps())
    }

    fn goal_card(goal: &GoalItem, theme: &AppTheme) -> impl IntoElement + use<> {
        vstack(8.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .p(px(12.0))
            .child(div().w_full().child(MainText::new(goal.title.clone()).px(14.0).font_weight(FontWeight::SEMIBOLD)))
            .when(!goal.description.is_empty(), |card| {
                card.child(div().w_full().child(SecondaryText::new(goal.description.clone()).px(13.0).line_limit(3)))
            })
            .child(
                hstack(10.0)
                    .w_full()
                    .child(div().flex_1().min_w_0().child(progress_view_value(goal.progress).tint(theme.primary._100)))
                    .child(
                        div().w(px(36.0)).flex().justify_end().child(
                            MainText::new(format!("{}%", (goal.progress * 100.0) as i64))
                                .px(11.0)
                                .color(theme.text.secondary),
                        ),
                    ),
            )
    }

    fn subgoals_disclosure(
        &self,
        goal: &GoalItem,
        expanded: bool,
        chevron_degrees: f32,
        disclosure: Entity<GoalsDisclosureState>,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        let secondary = theme.text.secondary;
        let toggle_id = goal.id.clone();
        let toggle = plain(child_id(&self.id, format!("goal-subgoals-{}", goal.id)))
            .w_full()
            .on_press(move |_, cx| disclosure.update(cx, |state, cx| state.toggle(&toggle_id, cx)))
            .child(
                hstack(6.0)
                    .w_full()
                    .children(IconUi::sized("checklist", px(12.0), secondary, false))
                    .child(
                        MainText::new(GoalChecklistSection::progress_text(goal))
                            .px(12.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(secondary),
                    )
                    .child(spacer())
                    .child(
                        div()
                            .children(IconUi::sized("chevron.down", px(12.0), secondary, false))
                            .rotation_effect(chevron_degrees),
                    ),
            );
        let subgoals: Vec<AnyElement> = if expanded {
            goal.subgoals
                .iter()
                .map(|sub| {
                    let (icon, tint) =
                        if sub.done { ("checkmark.circle.fill", theme.primary._100) } else { ("circle", secondary) };
                    hstack(8.0)
                        .w_full()
                        .children(IconUi::sized(icon, px(15.0), tint, false))
                        .child(
                            div().flex_1().min_w_0().child(
                                SecondaryText::new(sub.title.clone())
                                    .px(13.0)
                                    .color(if sub.done { secondary } else { theme.text.primary })
                                    .strikethrough(sub.done),
                            ),
                        )
                        .into_any_element()
                })
                .collect()
        } else {
            Vec::new()
        };

        vstack(8.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .px(px(12.0))
            .pb(px(12.0))
            .child(toggle)
            .children(subgoals)
    }

    #[allow(clippy::too_many_arguments)]
    fn goal_row(
        &self,
        goal: &GoalItem,
        is_last: bool,
        expanded: bool,
        chevron_degrees: f32,
        disclosure: Entity<GoalsDisclosureState>,
        nav_store: Option<Entity<NavigationStore>>,
        suppressor: Option<Entity<AnimatedTabsPagerLiveState>>,
        theme: &AppTheme,
    ) -> AnyElement {
        let head = if self.is_own_profile {
            let goal_id = goal.id.clone();
            plain(child_id(&self.id, format!("goal-{goal_id}")))
                .w_full()
                .on_press(move |_, cx| {
                    if Self::is_tap_suppressed(suppressor.as_ref(), cx) {
                        return;
                    }
                    if let Some(nav_store) = &nav_store {
                        let route = RootRoute::GoalDetail { goal_id: goal_id.clone() };
                        nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
                    }
                })
                .child(Self::goal_card(goal, theme))
                .into_any_element()
        } else {
            Self::goal_card(goal, theme).into_any_element()
        };
        let shows_subgoals = goal.is_checklist && !goal.subgoals.is_empty();

        vstack(0.0)
            .w_full()
            .bg(theme.bg._200)
            .rounded(px(12.0))
            .overflow_hidden()
            .child(head)
            .when(shows_subgoals, |row| {
                row.child(self.subgoals_disclosure(goal, expanded, chevron_degrees, disclosure, theme))
            })
            .guide_anchor((self.is_own_profile && is_last).then_some(RiseGuideAnchor::GoalsLast))
            .into_any_element()
    }

    fn add_button(
        &self,
        is_full: bool,
        show_add: Entity<bool>,
        suppressor: Option<Entity<AnimatedTabsPagerLiveState>>,
        tint: Hsla,
    ) -> impl IntoElement + use<> {
        plain(child_id(&self.id, "add"))
            .w_full()
            .on_press(move |_, cx| {
                if Self::is_tap_suppressed(suppressor.as_ref(), cx) {
                    return;
                }
                if is_full {
                    Notifier::send(
                        tr("server_error_limit_reached"),
                        NotifierSendOptions {
                            content: Some(format!("{}: {}", tr("goals_title"), *MAX_GOALS)),
                            r#type: NotifierType::Warning,
                            position: NotifierPosition::Top,
                            ..Default::default()
                        },
                        cx,
                    );
                    return;
                }
                show_add.update(cx, |show_add, cx| {
                    *show_add = true;
                    cx.notify();
                });
            })
            .child(
                hstack(6.0)
                    .w_full()
                    .justify_center()
                    .py(px(10.0))
                    .bg(tint.opacity(0.10))
                    .rounded_full()
                    .opacity(if is_full { 0.45 } else { 1.0 })
                    .children(IconUi::sized("plus.circle.fill", px(17.0), tint, true))
                    .child(MainText::new(tr("add_goal")).px(14.0).color(tint)),
            )
            .guide_anchor(RiseGuideAnchor::GoalsAdd)
    }
}

impl RenderOnce for Goals {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let store = UserServicesStore::shared(cx);
        let nav_store = env::<NavigationStoreKey>(cx);
        let suppressor = env::<ChatsListRowActionsSuppressorKey>(cx);
        let show_add = window.use_keyed_state(child_id(&self.id, "show-add"), cx, |_, _| false);
        let disclosure =
            window.use_keyed_state(child_id(&self.id, "expanded-ids"), cx, |_, _| GoalsDisclosureState::default());
        let now = RiseClock::monotonic();

        let own_goals = store.read(cx).goals().to_vec();
        let goals = if self.is_own_profile { own_goals.clone() } else { self.foreign_goals.clone() };
        let last_goal_id = goals.last().map(|goal| goal.id.clone());
        let rows: Vec<AnyElement> = goals
            .iter()
            .map(|goal| {
                let is_last = last_goal_id.as_deref() == Some(goal.id.as_str());
                let (expanded, chevron_degrees) = {
                    let state = disclosure.read(cx);
                    let degrees = state.chevrons.get(&goal.id).map(|chevron| chevron.frame_value(now, window)).unwrap_or(0.0);
                    (state.expanded_ids.contains(&goal.id), degrees)
                };
                self.goal_row(
                    goal,
                    is_last,
                    expanded,
                    chevron_degrees,
                    disclosure.clone(),
                    nav_store.clone(),
                    suppressor.clone(),
                    &theme,
                )
            })
            .collect();

        let is_full = own_goals.len() as i64 >= *MAX_GOALS && !(user_guide_sandbox_hooks().owns_goals_scene)(cx);
        let read_show_add = show_add.clone();
        let write_show_add = show_add.clone();
        let is_presented = Binding::new(
            move |cx: &App| *read_show_add.read(cx),
            move |presented, _window, cx| {
                write_show_add.update(cx, |show_add, cx| {
                    *show_add = presented;
                    cx.notify();
                });
            },
        );
        let sheet = NativeBottomSheetPresentationHost::new(child_id(&self.id, "add-sheet"), is_presented, move |_, _| {
            ProfileItemAddSheet::new(
                tr("add_goal"),
                tr("new_goal_placeholder"),
                *TITLE_RULE,
                RiseGuideAnchor::GoalsAddForm,
                RiseGuideSignal::GoalAdding,
                |title, _, _, cx| {
                    UserServicesStore::shared(cx).update(cx, |store, cx| store.add_goal(&title, cx));
                },
            )
        })
        .presentation_detents([PresentationDetent::Height(ProfileItemAddSheet::height(false))])
        .presentation_drag_indicator(true);

        vstack(10.0)
            .w_full()
            .px(px(12.0))
            .children(rows)
            .when(self.is_own_profile, |list| list.child(self.add_button(is_full, show_add, suppressor, theme.primary._100)))
            .when(!self.is_own_profile && goals.is_empty(), |list| {
                list.child(ChatProfileEmptyState::new("target", tr("goals_empty")))
            })
            .child(sheet)
    }
}
