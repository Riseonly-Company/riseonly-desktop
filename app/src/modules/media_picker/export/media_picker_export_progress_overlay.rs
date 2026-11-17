use std::sync::Arc;

use gpui::{App, ElementId, Entity, FocusHandle, FontWeight, IntoElement, RenderOnce, Window, black};
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::keyboard_dismiss::LayerFocus;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::TransitionModifiers;
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};

use crate::modules::media_picker::export::export_progress_view_state::ExportProgressViewState;


#[derive(IntoElement)]
pub struct MediaPickerExportProgressOverlay {
    state: Entity<ExportProgressViewState>,
    theme_store: Entity<ThemeStore>,
}

impl MediaPickerExportProgressOverlay {
    pub fn new(state: Entity<ExportProgressViewState>, theme_store: Entity<ThemeStore>) -> Self {
        Self { state, theme_store }
    }

    fn cancel(state: &Entity<ExportProgressViewState>, cx: &mut App) {
        state.update(cx, |state, cx| state.cancel(cx));
    }
}

impl RenderOnce for MediaPickerExportProgressOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (primary, text_primary, bg_300, bg_200) = {
            let theme = self.theme_store.read(cx).current_theme();
            (theme.primary._100, theme.text.primary, theme.bg._300, theme.bg._200)
        };
        let (total_items, current_label, is_cancelled) = {
            let state = self.state.read(cx);
            (state.total_items, state.current_label.clone(), state.is_cancelled)
        };
        let base_id = ElementId::View(self.state.entity_id());
        let child_id = |name: &'static str| ElementId::NamedChild(Arc::new(base_id.clone()), name.into());
        let focus_handle: Entity<FocusHandle> =
            window.use_keyed_state(child_id("focus"), cx, |_, cx| cx.focus_handle());
        let focus_handle = focus_handle.read(cx).clone();

        let spinner = div()
            .child(progress_view().control_size(ControlSize::Large).tint(primary))
            .scale_effect(1.2);

        let label = (total_items > 1).then(|| {
            let is_empty = current_label.is_empty();
            div().opacity(if is_empty { 0.0 } else { 1.0 }).child(
                MainText::new(current_label)
                    .px(15.0)
                    .font_weight(FontWeight::MEDIUM)
                    .color(text_primary)
                    .monospaced_digits(true)
                    .line_limit(1),
            )
        });

        let press_state = self.state.clone();
        let cancel_button = plain(child_id("cancel"))
            .on_press(move |_, cx| Self::cancel(&press_state, cx))
            .disabled(is_cancelled)
            .child(
                div()
                    .padding_edges(EdgeSet::HORIZONTAL, 18.0)
                    .padding_edges(EdgeSet::VERTICAL, 10.0)
                    .bg(bg_300)
                    .clip_shape(Shape::Capsule)
                    .child(
                        MainText::new(tr("media_picker_cancel"))
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(text_primary),
                    ),
            );

        let card = vstack(16.0)
            .child(spinner)
            .children(label)
            .child(cancel_button)
            .padding_edges(EdgeSet::HORIZONTAL, 28.0)
            .padding_edges(EdgeSet::TOP, 26.0)
            .padding_edges(EdgeSet::BOTTOM, 20.0)
            .frame_min(180.0, None)
            .bg(bg_200)
            .continuous_corners(22.0)
            .overflow_hidden();

        let escape_state = self.state.clone();
        let layer_focus = LayerFocus::keyed(child_id("focus-return"), window, cx);
        let (appearing, leaving) = (layer_focus.clone(), layer_focus);
        let appear_focus = focus_handle.clone();
        let disappear_focus = focus_handle.clone();
        div()
            .id(child_id("root"))
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .track_focus(&focus_handle)
            .layer_keys(LayerCancel::dismiss(move |_, cx| {
                if !escape_state.read(cx).is_cancelled {
                    Self::cancel(&escape_state, cx);
                }
            }))
            .child(div().absolute().inset_0().bg(black().opacity(0.25)))
            .child(card)
            .on_appear(child_id("appear"), move |window, cx| {
                LayerFocus::did_appear(&appearing, &appear_focus, window, cx);
            })
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    LayerFocus::will_disappear(&leaving, &disappear_focus, window, cx);
                }
            })
    }
}
