use gpui::{App, ElementId, Entity, FontWeight, Hsla, SharedString, Window, transparent_black, white};
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::theme::app_theme::readable_ink;
use rise_theme::{AppTheme, ThemeStore, color};
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::modules::theme::stores::theme_presets::theme_presets_actions::theme_presets_actions::ThemePresetsActionsStore;
use crate::modules::theme::stores::theme_presets::theme_presets_services::theme_presets_services::{
    ThemePreset, ThemePresetsServicesStore,
};

const SHEET_ID: &str = "theme-replace-saved-sheet";

/// state of the sheet, the picked theme and the request it was picked for
struct ThemeReplaceSavedState {
    selected_id: Option<String>,
    request_id: Option<String>,
}

/// mounted next to the theme preview, saving one more theme over the cap asks which saved one gives way
/// one pick and confirm or the oldest in one tap
#[derive(IntoElement)]
pub struct ThemeReplaceSavedSheet {
    id: ElementId,
}

impl Default for ThemeReplaceSavedSheet {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemeReplaceSavedSheet {
    pub fn new() -> Self {
        Self { id: ElementId::Name(SharedString::new_static(SHEET_ID)) }
    }

    fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
        (id.clone(), name.into()).into()
    }

    fn content(id: &ElementId, state: &Entity<ThemeReplaceSavedState>, theme: &AppTheme, cx: &mut App) -> Div {
        let resolved = ThemeStore::shared(cx).read(cx).resolved_appearance();
        let saved: Vec<ThemePreset> =
            ThemePresetsServicesStore::shared(cx).read(cx).presets().iter().filter(|preset| preset.is_saved()).cloned().collect();
        let selected_id = state.read(cx).selected_id.clone();
        let rows = saved
            .iter()
            .map(|preset| Self::row(state, preset, selected_id.as_deref() == Some(preset.id.as_str()), resolved, theme))
            .collect();
        vstack(12.)
            .w_full()
            .pt(px(8.))
            .child(
                MainText::new(tr("theme_replace_saved_title"))
                    .px(20.)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD),
            )
            .child(
                div().w_full().px(px(16.)).child(
                    MainText::new(
                        tr("theme_replace_saved_message")
                            .replace("%d", &ThemePresetsActionsStore::saved_limit().to_string()),
                    )
                    .px(15.)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary),
                ),
            )
            .child(GroupedBtns::new(Self::child_id(id, "rows"), rows))
    }

    fn row(
        state: &Entity<ThemeReplaceSavedState>,
        preset: &ThemePreset,
        selected: bool,
        resolved: rise_theme::RiseThemeAppearance,
        theme: &AppTheme,
    ) -> GroupedBtn {
        let picking = state.clone();
        let preset_id = preset.id.clone();
        let mark_color = if selected { theme.primary._100 } else { theme.text.secondary.opacity(0.5) };
        GroupedBtn::new(format!("theme_replace_{}", preset.id), preset.name.clone())
            .leading_slot(ThemeReplaceSwatches { hexes: preset.swatches(resolved), stroke: theme.text.primary })
            .text_line_limit(1)
            .accessibility_selected(selected)
            .show_chevron(false)
            .trailing_slot(
                div().children(IconUi::sized(
                    if selected { "checkmark.circle.fill" } else { "circle" },
                    px(20.),
                    mark_color,
                    false,
                )),
            )
            .callback(move |_, cx| {
                selection_changed();
                let preset_id = preset_id.clone();
                picking.update(cx, |state, cx| {
                    state.selected_id = Some(preset_id);
                    cx.notify();
                });
            })
    }

    fn footer(id: &ElementId, state: &Entity<ThemeReplaceSavedState>, theme: &AppTheme, cx: &mut App) -> Div {
        let ink = readable_ink(theme.primary._100, white());
        let is_mutating = ThemePresetsServicesStore::shared(cx).read(cx).is_mutating();
        let has_selection = state.read(cx).selected_id.is_some();
        let confirming = state.clone();
        vstack(8.)
            .w_full()
            .px(px(16.))
            .pt(px(12.))
            .pb(px(4.))
            .child(
                div().w_full().opacity(if has_selection { 1. } else { 0.5 }).child(
                    ButtonUi::new(Self::child_id(id, "confirm"))
                        .background_color(theme.primary._100)
                        .height(52.)
                        .disabled(!has_selection || is_mutating)
                        .on_press(move |_, cx| {
                            let Some(selected_id) = confirming.read(cx).selected_id.clone() else {
                                return;
                            };
                            ThemePresetsActionsStore::shared(cx)
                                .update(cx, |actions, cx| actions.confirm_replace_saved_action(Some(selected_id), cx));
                        })
                        .child(if is_mutating {
                            progress_view().tint(ink).into_any_element()
                        } else {
                            MainText::new(tr("theme_replace_saved_confirm"))
                                .px(16.)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(ink)
                                .into_any_element()
                        }),
                ),
            )
            .child(
                plain(Self::child_id(id, "oldest"))
                    .w_full()
                    .disabled(is_mutating)
                    .on_press(|_, cx| {
                        ThemePresetsActionsStore::shared(cx)
                            .update(cx, |actions, cx| actions.confirm_replace_saved_action(None, cx));
                    })
                    .child(
                        div().w_full().min_h(px(44.)).flex().items_center().justify_center().child(
                            MainText::new(tr("theme_replace_oldest"))
                                .px(15.)
                                .tac(TextAlignT::Center)
                                .font_weight(FontWeight::MEDIUM)
                                .color(theme.primary._100),
                        ),
                    ),
            )
    }
}

impl RenderOnce for ThemeReplaceSavedSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(Self::child_id(&self.id, "state"), cx, |_, _| ThemeReplaceSavedState {
            selected_id: None,
            request_id: None,
        });
        let request_id = ThemePresetsServicesStore::shared(cx)
            .read(cx)
            .replace_request()
            .map(|request| request.preset.id.clone());
        if state.read(cx).request_id != request_id {
            state.update(cx, |state, _| {
                state.request_id = request_id;
                state.selected_id = None;
            });
        }
        let sheet_background = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        let services = ThemePresetsServicesStore::shared(cx);
        let is_presented = Binding::new(
            move |cx| services.read(cx).replace_request().is_some(),
            |is_presented, _, cx| {
                if !is_presented {
                    ThemePresetsActionsStore::shared(cx).update(cx, |actions, cx| actions.cancel_replace_saved_action(cx));
                }
            },
        );
        let content_id = self.id.clone();
        let content_state = state.clone();
        let footer_id = self.id.clone();
        let footer_state = state;
        BottomSheetUi::new(
            self.id,
            is_presented,
            move |_, cx| {
                let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
                Self::content(&content_id, &content_state, &theme, cx)
            },
            move |_, cx| {
                let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
                Self::footer(&footer_id, &footer_state, &theme, cx)
            },
        )
        .snap_fractions(vec![0.7])
        .full_page(false)
        .dismisses_on_backdrop_tap(true)
        .full_page_shows_drag_indicator(true)
        .footer_shows_top_border(false)
        .footer_overlays_content(Some(false))
        .footer_horizontal_padding(Some(0.))
        .footer_top_padding(Some(0.))
        .content_scrollable(true)
        .fits_content(true)
        .sheet_background(Some(sheet_background))
        .footer_background(Some(transparent_black()))
    }
}

#[derive(IntoElement)]
struct ThemeReplaceSwatches {
    hexes: Vec<String>,
    stroke: Hsla,
}

impl RenderOnce for ThemeReplaceSwatches {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let stroke = self.stroke;
        hstack(0.).w(px(44.)).h(px(28.)).items_center().justify_center().children(
            self.hexes.into_iter().enumerate().map(move |(index, hex)| {
                div()
                    .relative()
                    .flex_none()
                    .size(px(20.))
                    .circle()
                    .bg(color(&hex))
                    .when(index > 0, |dot| dot.ml(px(-8.)))
                    .child(centred_stroke(stroke.opacity(0.15), 1., Shape::Circle))
            }),
        )
    }
}
