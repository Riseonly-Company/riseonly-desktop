use gpui::{
    App, ElementId, FocusHandle, FontWeight, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    StatefulInteractiveElement, Styled, Window, black, div, px,
};
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::keyboard_dismiss::LayerFocus;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::swiftui::shapes::SwiftUiShapes;
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};

use crate::core::ui::bottom_sheet_ui::experimental_bottom_sheet_presentation_host::ExperimentalBottomSheetPresentationHost;

const SCRIM_OPACITY: f32 = 0.4;
const CARD_MAX_WIDTH: f32 = 420.0;
const CARD_CORNER_RADIUS: f32 = 28.0;
const CARD_OUTER_PADDING: f32 = 24.0;
const ICON_SIDE: f32 = 30.0;
const ICON_CIRCLE_SIDE: f32 = 64.0;
const ICON_CIRCLE_OPACITY: f32 = 0.12;
const SHADOW_OPACITY: f32 = 0.16;
const SHADOW_RADIUS: f32 = 24.0;
const SHADOW_OFFSET_Y: f32 = 12.0;

#[derive(IntoElement)]
pub struct DownloadLimitHelpModal {
    id: ElementId,
    is_presented: Binding<bool>,
}

impl DownloadLimitHelpModal {
    /// id keys the modal own state so two live copies never share it
    pub fn new(id: impl Into<ElementId>, is_presented: Binding<bool>) -> Self {
        Self {
            id: id.into(),
            is_presented,
        }
    }

    fn card(id: ElementId, is_presented: Binding<bool>, theme: &AppTheme) -> impl IntoElement + use<> {
        div()
            .w_full()
            .p_6()
            .flex()
            .flex_col()
            .items_center()
            .gap_5()
            .continuous_corners(CARD_CORNER_RADIUS)
            .bg(theme.bg._100)
            .border_1()
            .border_color(theme.bg._300)
            .shadow(vec![gpui::BoxShadow {
                color: black().opacity(SHADOW_OPACITY),
                offset: gpui::point(px(0.0), px(SHADOW_OFFSET_Y)),
                blur_radius: px(SHADOW_RADIUS),
                spread_radius: px(0.0),
                inset: false,
            }])
            .child(
                div()
                    .size(px(ICON_CIRCLE_SIDE))
                    .rounded_full()
                    .bg(theme.primary._100.opacity(ICON_CIRCLE_OPACITY))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized("arrow.down.circle", px(ICON_SIDE), theme.primary._100, false)),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .child(
                        MainText::new(tr("download_limit_help_title"))
                            .px(22.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::BOLD),
                    )
                    .child(
                        div().w_full().child(
                            MainText::new(tr("download_limit_help"))
                                .px(16.0)
                                .color(theme.text.secondary)
                                .line_height(24.0),
                        ),
                    ),
            )
            .child(
                ButtonUi::new((id, "done"))
                    .background_color(theme.primary._100)
                    .height(50.0)
                    .on_press(move |window, cx| is_presented.set(false, window, cx))
                    .child(
                        MainText::new(tr("release_notes_done"))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.content_on(theme.primary._100)),
                    ),
            )
    }
}

impl RenderOnce for DownloadLimitHelpModal {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let is_presented = self.is_presented.clone();
        let id = self.id;
        div().size_0().child(
            ExperimentalBottomSheetPresentationHost::new(id.clone(), self.is_presented, move |window, cx| {
                let focus_handle = window.use_keyed_state(
                    (id.clone(), "focus"),
                    cx,
                    |_, cx| cx.focus_handle(),
                );
                let focus_handle: FocusHandle = focus_handle.read(cx).clone();

                let scrim_close = is_presented.clone();
                let escape_close = is_presented.clone();
                let layer_focus = LayerFocus::keyed((id.clone(), "focus-return"), window, cx);
                let theme_store = ThemeStore::shared(cx);
                let theme = theme_store.read(cx).current_theme();
                let (appearing, leaving) = (layer_focus.clone(), layer_focus);
                let focus_on_appear = focus_handle.clone();
                let focus_on_disappear = focus_handle.clone();
                div()
                    .id((id.clone(), "layer"))
                    .size_full()
                    .relative()
                    .track_focus(&focus_handle)
                    .layer_keys(LayerCancel::dismiss(move |window, cx| escape_close.set(false, window, cx)))
                    .child(
                        div()
                            .id((id.clone(), "scrim"))
                            .absolute()
                            .inset_0()
                            .bg(black().opacity(SCRIM_OPACITY))
                            .on_click(move |_, window, cx| scrim_close.set(false, window, cx)),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .id((id.clone(), "scroll"))
                                    .w_full()
                                    .max_w(px(CARD_MAX_WIDTH + 2.0 * CARD_OUTER_PADDING))
                                    .max_h_full()
                                    .overflow_y_scroll()
                                    .px(px(CARD_OUTER_PADDING))
                                    .py(px(SHADOW_OFFSET_Y + SHADOW_RADIUS))
                                    .child(Self::card(id.clone(), is_presented.clone(), theme)),
                            ),
                    )
                    .on_appear((id.clone(), "appear"), move |window, cx| {
                        LayerFocus::did_appear(&appearing, &focus_on_appear, window, cx);
                    })
                    .and_on_disappear(move |window, cx| {
                        if let Some(window) = window {
                            LayerFocus::will_disappear(&leaving, &focus_on_disappear, window, cx);
                        }
                    })
            })
            .dismissal_delay(0.0),
        )
    }
}
