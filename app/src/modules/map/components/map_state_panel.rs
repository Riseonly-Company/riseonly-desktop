use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, FontWeight, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px, white,
};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::skeleton_ui::{SkeletonCircle, SkeletonLine};
use rise_ui::{ButtonUi, IconUi, MainText};

use crate::modules::map::engine::rise_map_engine_models::RiseMapStateSnapshot;

type PanelAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// bottom map panel shown until the account joins
/// same geometry as the friends strip so skeleton, invite and error swap in place
#[derive(IntoElement)]
pub struct MapStatePanel {
    snapshot: RiseMapStateSnapshot,
    on_join: PanelAction,
    on_retry: PanelAction,
}

impl MapStatePanel {
    const CORNER_RADIUS: f32 = 28.0;
    const ICON_SIDE: f32 = 46.0;
    const CONTENT_HEIGHT: f32 = 46.0;

    pub fn new(
        snapshot: RiseMapStateSnapshot,
        on_join: impl Fn(&mut Window, &mut App) + 'static,
        on_retry: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { snapshot, on_join: Rc::new(on_join), on_retry: Rc::new(on_retry) }
    }

    fn skeleton() -> Vec<AnyElement> {
        vec![
            SkeletonCircle::new(Self::ICON_SIDE).into_any_element(),
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(SkeletonLine::new(150., 15.))
                .child(SkeletonLine::new(210., 12.))
                .into_any_element(),
            div().flex_1().into_any_element(),
        ]
    }

    fn outsider(&self, theme: &AppTheme) -> Vec<AnyElement> {
        let on_join = self.on_join.clone();
        vec![
            Self::icon_tile("location.fill", theme),
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(MainText::new(tr("map_join_title")).px(16.).font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
                .child(MainText::new(tr("map_join_subtitle")).px(13.).color(theme.text.secondary).number_of_lines(2))
                .into_any_element(),
            div().w(px(8.)).flex_none().into_any_element(),
            ButtonUi::new(ElementId::Name("map-state-panel-join".into()))
                .is_pending(self.snapshot.is_joining)
                .height(38.)
                .fit_content(true)
                .padding_horizontal(16.)
                .on_press(move |window, cx| on_join(window, cx))
                .child(Self::action_label(tr("map_join_action")))
                .guide_anchor(RiseGuideAnchor::MapJoin)
                .into_any_element(),
        ]
    }

    fn failed(&self, theme: &AppTheme) -> Vec<AnyElement> {
        let on_retry = self.on_retry.clone();
        vec![
            Self::icon_tile("wifi.exclamationmark", theme),
            div()
                .flex_1()
                .min_w_0()
                .child(
                    MainText::new(self.snapshot.error_text.clone().unwrap_or_else(|| tr("map_load_failed")))
                        .px(14.)
                        .color(theme.text.secondary)
                        .number_of_lines(2),
                )
                .into_any_element(),
            div().w(px(8.)).flex_none().into_any_element(),
            ButtonUi::new(ElementId::Name("map-state-panel-retry".into()))
                .height(38.)
                .fit_content(true)
                .padding_horizontal(16.)
                .on_press(move |window, cx| on_retry(window, cx))
                .child(Self::action_label(tr("common_retry")))
                .into_any_element(),
        ]
    }

    fn action_label(text: String) -> MainText {
        MainText::new(text).px(15.).font_weight(FontWeight::SEMIBOLD).color(white()).number_of_lines(1)
    }

    fn icon_tile(system_name: &str, theme: &AppTheme) -> AnyElement {
        div()
            .size(px(Self::ICON_SIDE))
            .flex_none()
            .rounded_full()
            .bg(theme.primary._100)
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized(system_name, px(19.), theme.content_on(theme.primary._100), true))
            .into_any_element()
    }
}

impl RenderOnce for MapStatePanel {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let children = if self.snapshot.shows_initial_loader() {
            Self::skeleton()
        } else if self.snapshot.shows_blocking_error() {
            self.failed(&theme)
        } else {
            self.outsider(&theme)
        };
        div()
            .w_full()
            .px(px(16.))
            .py(px(14.))
            .child(
                div()
                    .w_full()
                    .min_h(px(Self::CONTENT_HEIGHT))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.))
                    .children(children),
            )
            .liquid_chrome_rounded_rectangle(Self::CORNER_RADIUS)
    }
}
