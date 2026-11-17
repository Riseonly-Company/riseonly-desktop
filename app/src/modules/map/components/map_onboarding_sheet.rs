use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, FontWeight, IntoElement, ParentElement, Render, RenderOnce, Styled,
    Subscription, Window, div, px, transparent_black, white,
};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::prelude::*;
use rise_ui::{ButtonUi, IconUi, MainText, TextAlignT};

use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::modules::map::stores::map_interactions::map_interactions::{MapInteractionsStore, MapOnboardingStep};

type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// entry into the map
/// intro and photos and join
/// desktop has no location source so no location or background access steps
pub struct MapOnboardingSheetHost {
    interactions: Entity<MapInteractionsStore>,
    _interactions_observation: Subscription,
}

impl MapOnboardingSheetHost {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let interactions = MapInteractionsStore::shared(cx);
        let interactions_observation = cx.observe(&interactions, |_, _, cx| cx.notify());
        Self { interactions, _interactions_observation: interactions_observation }
    }

    fn dismiss(interactions: &Entity<MapInteractionsStore>, cx: &mut App) {
        interactions.update(cx, |interactions, cx| {
            if interactions.is_joining {
                return;
            }
            interactions.dismiss_onboarding(cx);
        });
    }

    fn primary_action(interactions: &Entity<MapInteractionsStore>, cx: &mut App) {
        interactions.update(cx, |interactions, cx| match interactions.onboarding_step {
            MapOnboardingStep::Intro => interactions.advance_onboarding(cx),
            MapOnboardingStep::Photos => interactions.request_gallery_access(cx),
            MapOnboardingStep::Ready => interactions.finish_onboarding(cx),
        });
    }
}

impl Render for MapOnboardingSheetHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (step, is_joining) = {
            let interactions = self.interactions.read(cx);
            (interactions.onboarding_step, interactions.is_joining)
        };
        let reader = self.interactions.clone();
        let writer = self.interactions.clone();
        let is_presented = Binding::new(
            move |cx| reader.read(cx).is_onboarding_presented,
            move |presented, _, cx| {
                if !presented {
                    Self::dismiss(&writer, cx);
                }
            },
        );
        let primary = self.interactions.clone();
        let skip = self.interactions.clone();
        MapOnboardingSheet::new(
            is_presented,
            step,
            is_joining,
            move |_, cx| Self::primary_action(&primary, cx),
            move |_, cx| skip.update(cx, |interactions, cx| interactions.advance_onboarding(cx)),
        )
    }
}

#[derive(IntoElement)]
pub struct MapOnboardingSheet {
    is_presented: Binding<bool>,
    step: MapOnboardingStep,
    is_joining: bool,
    on_primary: SheetAction,
    on_skip: SheetAction,
    full_page: bool,
}

impl MapOnboardingSheet {
    pub fn new(
        is_presented: Binding<bool>,
        step: MapOnboardingStep,
        is_joining: bool,
        on_primary: impl Fn(&mut Window, &mut App) + 'static,
        on_skip: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            is_presented,
            step,
            is_joining,
            on_primary: Rc::new(on_primary),
            on_skip: Rc::new(on_skip),
            full_page: true,
        }
    }

    pub fn full_page(mut self, full_page: bool) -> Self {
        self.full_page = full_page;
        self
    }
}

impl RenderOnce for MapOnboardingSheet {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let (step, is_joining) = (self.step, self.is_joining);
        let on_primary = self.on_primary;
        let on_skip = self.on_skip;
        let refused = self.is_presented.clone();
        BottomSheetUi::new(
            "map-onboarding-sheet",
            self.is_presented,
            move |_, _| {
                div()
                    .w_full()
                    .pt(px(16.))
                    .pb(px(156.))
                    .child(MapOnboardingContent::new(step))
            },
            move |_, _| {
                div().w_full().pb(px(12.)).child(MapOnboardingFooter {
                    step,
                    is_joining,
                    on_primary: on_primary.clone(),
                    on_skip: on_skip.clone(),
                })
            },
        )
        .snap_fractions(vec![0.72, 0.93])
        .full_page(self.full_page)
        .full_page_allows_swipe_dismiss(!is_joining)
        .dismisses_on_backdrop_tap(!is_joining)
        .footer_shows_top_border(false)
        .footer_overlays_content(Some(true))
        .footer_horizontal_padding(Some(14.0))
        .footer_top_padding(Some(8.0))
        .content_scrollable(true)
        .full_page_interaction_locked(is_joining)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(transparent_black()))
        .on_presentation_refused(move |window, cx| refused.set(false, window, cx))
    }
}

#[derive(IntoElement)]
pub struct MapOnboardingContent {
    step: MapOnboardingStep,
}

impl MapOnboardingContent {
    pub fn new(step: MapOnboardingStep) -> Self {
        Self { step }
    }

    fn progress(&self, theme: &AppTheme) -> AnyElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .children(MapOnboardingStep::ALL.into_iter().map(|item| {
                let color = if item.raw_value() <= self.step.raw_value() {
                    theme.primary._100
                } else {
                    theme.text.secondary.opacity(0.25)
                };
                div()
                    .w(px(if item == self.step { 22. } else { 8. }))
                    .h(px(4.))
                    .rounded_full()
                    .bg(color)
            }))
            .into_any_element()
    }

    fn hero_icon(&self, theme: &AppTheme) -> AnyElement {
        div()
            .size(px(84.))
            .rounded_full()
            .bg(theme.primary._100)
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized(self.hero_symbol(), px(34.), theme.content_on(theme.primary._100), true))
            .into_any_element()
    }

    fn point(symbol: &str, key: &str, theme: &AppTheme) -> AnyElement {
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .size(px(34.))
                    .flex_none()
                    .rounded_full()
                    .bg(theme.primary._100)
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized(symbol, px(15.), theme.content_on(theme.primary._100), true)),
            )
            .child(div().flex_1().min_w_0().child(MainText::new(tr(key)).px(15.).color(theme.text.primary)))
            .into_any_element()
    }

    fn hero_symbol(&self) -> &'static str {
        match self.step {
            MapOnboardingStep::Intro => "map.fill",
            MapOnboardingStep::Photos => "photo.on.rectangle.angled",
            MapOnboardingStep::Ready => "checkmark",
        }
    }

    fn title_key(&self) -> &'static str {
        match self.step {
            MapOnboardingStep::Intro => "map_onboarding_intro_title",
            MapOnboardingStep::Photos => "map_onboarding_photos_title",
            MapOnboardingStep::Ready => "map_onboarding_ready_title",
        }
    }

    fn body_key(&self) -> &'static str {
        match self.step {
            MapOnboardingStep::Intro => "map_onboarding_intro_body",
            MapOnboardingStep::Photos => "map_onboarding_photos_body",
            MapOnboardingStep::Ready => "map_onboarding_ready_body",
        }
    }
}

impl RenderOnce for MapOnboardingContent {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let mut column = div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .px(px(22.))
            .pb(px(8.))
            .child(div().pt(px(14.)).child(self.progress(&theme)))
            .child(div().pt(px(22.)).child(self.hero_icon(&theme)))
            .child(
                div().w_full().pt(px(18.)).child(
                    MainText::new(tr(self.title_key()))
                        .px(22.)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::BOLD),
                ),
            )
            .child(
                div().w_full().pt(px(8.)).px(px(6.)).child(
                    MainText::new(tr(self.body_key()))
                        .px(15.)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary)
                        .line_height(21.),
                ),
            );
        if self.step == MapOnboardingStep::Intro {
            column = column.child(
                div()
                    .w_full()
                    .pt(px(22.))
                    .flex()
                    .flex_col()
                    .items_stretch()
                    .gap(px(14.))
                    .child(Self::point("person.2.fill", "map_onboarding_point_friends", &theme))
                    .child(Self::point(
                        "point.topleft.down.to.point.bottomright.curvepath.fill",
                        "map_onboarding_point_routes",
                        &theme,
                    ))
                    .child(Self::point("lock.fill", "map_onboarding_point_privacy", &theme)),
            );
        }
        column
    }
}

#[derive(IntoElement)]
struct MapOnboardingFooter {
    step: MapOnboardingStep,
    is_joining: bool,
    on_primary: SheetAction,
    on_skip: SheetAction,
}

impl MapOnboardingFooter {
    fn skip_button(&self, theme: &AppTheme) -> AnyElement {
        let is_skippable = self.step == MapOnboardingStep::Photos && !self.is_joining;
        let on_skip = self.on_skip.clone();
        plain(ElementId::Name("map-onboarding-skip".into()))
            .w_full()
            .h(px(40.))
            .flex()
            .items_center()
            .justify_center()
            .opacity(if is_skippable { 1. } else { 0. })
            .disabled(!is_skippable)
            .on_press(move |window, cx| on_skip(window, cx))
            .child(
                MainText::new(tr("map_onboarding_skip"))
                    .px(15.)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::MEDIUM)
                    .color(theme.text.secondary)
                    .number_of_lines(1),
            )
            .into_any_element()
    }

    fn primary_key(&self) -> &'static str {
        match self.step {
            MapOnboardingStep::Intro => "map_onboarding_continue",
            MapOnboardingStep::Photos => "map_onboarding_allow_photos",
            MapOnboardingStep::Ready => "map_join_action",
        }
    }
}

impl RenderOnce for MapOnboardingFooter {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let on_primary = self.on_primary.clone();
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(
                ButtonUi::new(ElementId::Name("map-onboarding-primary".into()))
                    .is_pending(self.is_joining)
                    .height(50.)
                    .loader_color(white())
                    .on_press(move |window, cx| on_primary(window, cx))
                    .child(
                        MainText::new(tr(self.primary_key()))
                            .px(16.)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(white())
                            .number_of_lines(1),
                    ),
            )
            .child(div().w_full().pt(px(6.)).child(self.skip_button(&theme)))
    }
}
