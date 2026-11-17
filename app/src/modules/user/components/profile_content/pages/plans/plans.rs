use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use gpui::{App, ElementId, Entity, FontWeight, Hsla, SharedString, Window};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal, RiseLimits};
use rise_core::foundation::foundation_date::Date;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::AnimatedTabsPagerLiveState;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::user::components::profile_content::pages::plans::chats_list_row_actions_suppressor_hooks::ChatsListRowActionsSuppressorKey;
use crate::modules::user::components::profile_content::pages::profile_item_add_sheet::ProfileItemAddSheet;
use crate::modules::user::stores::user::user_actions::user_types::{PlanDateFormatting, PlanItem};
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

static TITLE_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("user_sync_profile_plans", "/plans/*/title"));
static MAX_PLANS: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("max_plans"));

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

#[derive(IntoElement)]
pub struct Plans {
    id: ElementId,
    pub is_own_profile: bool,
    pub foreign_plans: Vec<PlanItem>,
}

impl Plans {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), is_own_profile: true, foreign_plans: Vec::new() }
    }

    pub fn is_own_profile(mut self, is_own_profile: bool) -> Self {
        self.is_own_profile = is_own_profile;
        self
    }

    pub fn foreign_plans(mut self, foreign_plans: Vec<PlanItem>) -> Self {
        self.foreign_plans = foreign_plans;
        self
    }

    fn is_tap_suppressed(suppressor: Option<&Entity<AnimatedTabsPagerLiveState>>, cx: &App) -> bool {
        suppressor.is_some_and(|suppressor| suppressor.read(cx).suppresses_tab_taps())
    }

    fn plan_card(&self, p: &PlanItem, theme: &AppTheme) -> Div {
        hstack(None)
            .w_full()
            .p(px(12.0))
            .bg(theme.bg._200)
            .rounded(px(12.0))
            .children(IconUi::sized("calendar", px(17.0), theme.primary._100, false))
            .child(
                vstack(None)
                    .align(HorizontalAlignment::Leading)
                    .child(MainText::new(p.title.clone()).px(14.0).font_weight(FontWeight::SEMIBOLD))
                    .when(!p.description.is_empty(), |stack| {
                        stack.child(div().w_full().child(SecondaryText::new(p.description.clone()).px(13.0).line_limit(3)))
                    })
                    .child(MainText::new(p.date_display()).px(11.0).color(theme.text.secondary)),
            )
            .child(spacer())
            .when(self.is_own_profile, |card| {
                card.children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(11.0), theme.text.secondary, false))
            })
    }

    fn plan_row(
        &self,
        p: &PlanItem,
        nav_store: Option<Entity<NavigationStore>>,
        suppressor: Option<Entity<AnimatedTabsPagerLiveState>>,
        theme: &AppTheme,
    ) -> AnyElement {
        if !self.is_own_profile {
            return self.plan_card(p, theme).into_any_element();
        }
        let plan_id = p.id.clone();
        plain(child_id(&self.id, format!("plan-{plan_id}")))
            .w_full()
            .on_press(move |_, cx| {
                if Self::is_tap_suppressed(suppressor.as_ref(), cx) {
                    return;
                }
                if let Some(nav_store) = &nav_store {
                    let route = RootRoute::PlanDetail { plan_id: plan_id.clone() };
                    nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
                }
            })
            .child(self.plan_card(p, theme))
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
                            content: Some(format!("{}: {}", tr("plans_title"), *MAX_PLANS)),
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
                    .capsule()
                    .opacity(if is_full { 0.45 } else { 1.0 })
                    .children(IconUi::sized("plus.circle.fill", px(17.0), tint, true))
                    .child(MainText::new(tr("add_plan")).px(14.0).color(tint)),
            )
            .guide_anchor(RiseGuideAnchor::PlansAdd)
    }
}

impl RenderOnce for Plans {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let suppressor = env::<ChatsListRowActionsSuppressorKey>(cx);
        let show_add: Entity<bool> = window.use_keyed_state(child_id(&self.id, "show-add"), cx, |_, _| false);
        let own_plans: Rc<[PlanItem]> = Rc::from(UserServicesStore::shared(cx).read(cx).plans());
        let plans: &[PlanItem] = if self.is_own_profile { &own_plans } else { &self.foreign_plans };
        let is_full = own_plans.len() as i64 >= *MAX_PLANS && !GuideSandbox::shared(cx).read(cx).owns_goals_scene();

        let rows: Vec<AnyElement> = plans.iter().map(|p| self.plan_row(p, nav_store.clone(), suppressor.clone(), &theme)).collect();
        let is_presented = Binding::entity(show_add.clone(), |show_add: &bool| *show_add, |show_add, value, _| *show_add = value);
        let sheet = NativeBottomSheetPresentationHost::new(child_id(&self.id, "add-sheet"), is_presented, |_, _| {
            ProfileItemAddSheet::new(
                tr("add_plan"),
                tr("new_plan_title_placeholder"),
                *TITLE_RULE,
                RiseGuideAnchor::PlansAddForm,
                RiseGuideSignal::PlanAdding,
                |title: String, date: Date, _: &mut Window, cx: &mut App| {
                    UserServicesStore::shared(cx)
                        .update(cx, |store, cx| store.add_plan(&title, PlanDateFormatting::ms(date), cx));
                },
            )
            .with_date(true)
        })
        .presentation_detents([PresentationDetent::Height(ProfileItemAddSheet::height(true))])
        .presentation_drag_indicator(true);

        vstack(10.0)
            .w_full()
            .px(px(12.0))
            .children(rows)
            .when(self.is_own_profile, |list| list.child(self.add_button(is_full, show_add, suppressor, theme.primary._100)))
            .when(!self.is_own_profile && plans.is_empty(), |list| {
                list.child(ChatProfileEmptyState::new("calendar", tr("plans_empty")))
            })
            .child(sheet)
    }
}
