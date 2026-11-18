use std::rc::Rc;

use gpui::{App, ElementId, Entity, FontWeight, Hsla, KeyDownEvent, MouseButton, MouseUpEvent, SharedString, Window, white};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_i18n::tr;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::icon_ui::IconUi;
use rise_ui::keyboard_dismiss::LayerFocus;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::modules::map::components::map_editing_session::{MapEditingMode, MapEditingSession, MapTrackPeriodTitle};
use crate::modules::map::components::map_photo_card::MapLocateButton;
use crate::modules::map::pages::world_map::map_journey_page::MapJourneyComposerControls;
use crate::modules::map::stores::map_actions::map_tracks_types::MapTrackPeriod;
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::map::stores::map_services::map_services::MapServicesStore;
use crate::modules::media_picker::editor::editor_thickness_track::{EditorBrushPreviewCircle, EditorThicknessTrack};

pub type MapEditingAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// hover hint for a glyph only control, the desktop's accessibility label
fn period_glyph(color: Hsla) -> impl IntoElement {
    div()
        .size(px(44.0))
        .flex()
        .items_center()
        .justify_center()
        .children(IconUi::sized("calendar", px(18.0), color, false))
        .liquid_chrome_rounded_rectangle(22.0)
}

pub(crate) fn map_control_hint(id: impl Into<ElementId>, label: impl Into<SharedString>, control: impl IntoElement) -> impl IntoElement {
    div().id(id).tooltip_label(label).child(control)
}

fn save_erased_tracks(session: &Entity<MapEditingSession>, on_close: &MapEditingAction, window: &Window, cx: &mut App) {
    let session = session.clone();
    let on_saved = on_close.clone();
    let window = window.window_handle();
    MapInteractionsStore::shared(cx).update(cx, |interactions, cx| {
        interactions.save_erased_tracks(
            session,
            move |cx| {
                let _ = window.update(cx, |_, window, cx| on_saved(window, cx));
            },
            cx,
        );
    });
}

#[derive(IntoElement)]
pub struct MapEditingControls {
    session: Entity<MapEditingSession>,
    on_close: MapEditingAction,
}

impl MapEditingControls {
    pub const PANEL_BOTTOM_PADDING: f32 = 8.0;

    pub fn new(session: Entity<MapEditingSession>, on_close: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { session, on_close: Rc::new(on_close) }
    }
}

impl RenderOnce for MapEditingControls {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { session, on_close } = self;
        let focus_handle = window
            .use_keyed_state("map-editing-controls-focus", cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let (mode, adjusting_brush, brush_diameter, error, strokes_empty, undone_empty, saving, pending) = {
            let session = session.read(cx);
            (
                session.mode,
                session.adjusting_brush,
                session.brush_diameter(),
                session.error.clone(),
                session.strokes.is_empty(),
                session.undone.is_empty(),
                session.saving,
                session.pending_save.is_some(),
            )
        };

        on_change(
            "map-editing-controls-erased",
            mode == MapEditingMode::Eraser && !strokes_empty,
            true,
            window,
            cx,
            |_, erased, _, cx| GuideFacts::set(RiseGuideSignal::MapEraserHasStrokes, *erased, cx),
        );

        let eraser_layer = (mode == MapEditingMode::Eraser).then(|| {
            let fraction = Binding::entity(
                session.clone(),
                |session: &MapEditingSession| session.brush_fraction as f32,
                |session: &mut MapEditingSession, value: f32, _| {
                    if !session.saving && session.pending_save.is_none() {
                        session.set_brush_fraction(f64::from(value));
                    }
                },
            );
            let adjusting = session.clone();
            let track = EditorThicknessTrack::new("map-eraser-thickness", fraction, 170.0, 24.0, move |is_adjusting, _, cx| {
                adjusting.update(cx, |session, cx| {
                    session.adjusting_brush = is_adjusting && !session.saving && session.pending_save.is_none();
                    cx.notify();
                });
            });
            let preview = adjusting_brush.then(|| {
                pass_through()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(EditorBrushPreviewCircle::new(brush_diameter as f32, white()))
            });
            div()
                .absolute()
                .inset_0()
                .children(preview)
                .child(
                    div().absolute().inset_0().pr(px(16.0)).flex().items_center().child(spacer_min_w(0.0)).child(
                        map_control_hint(
                            "map-eraser-size-hint",
                            tr("map_eraser_size"),
                            div().py(px(20.0)).px(px(8.0)).child(track).liquid_chrome_rounded_rectangle(24.0),
                        )
                        .guide_anchor(RiseGuideAnchor::MapEraserSize),
                    ),
                )
        });

        let column_width = rise_ui::theme(cx).shell.feed_max_width;
        let mut panel = div().w_full().max_w(column_width).flex().flex_col().gap(px(DEFAULT_STACK_SPACING));
        if let Some(error) = error {
            panel = panel.child(
                div().w_full().flex().justify_center().child(
                    div()
                        .p(px(12.0))
                        .child(MainText::new(error).px(13.0).number_of_lines(3))
                        .liquid_chrome_rounded_rectangle(18.0),
                ),
            );
        }
        let mut journey_state = None;
        if mode == MapEditingMode::Journey {
            let on_close = on_close.clone();
            let state = MapJourneyComposerControls::composer_state(window, cx);
            journey_state = Some(state.clone());
            let band_session = session.clone();
            panel = panel.child(
                div()
                    .relative()
                    .w_full()
                    .child(MapJourneyComposerControls::new(session.clone(), state, move |window, cx| on_close(window, cx)))
                    .child(measure("map-journey-band", move |bounds, _, cx| {
                        let band = f32::from(bounds.size.height) + Self::PANEL_BOTTOM_PADDING;
                        band_session.update(cx, |session, cx| {
                            if session.journey_band != Some(band) {
                                session.journey_band = Some(band);
                                cx.notify();
                            }
                        });
                    })),
            );
        } else {
            let undo_session = session.clone();
            let redo_session = session.clone();
            panel = panel
                .child(
                    hstack(10.0)
                        .w_full()
                        .child(spacer_min_w(0.0))
                        .child(
                            map_control_hint(
                                "map-edit-undo-hint",
                                tr("map_edit_undo"),
                                PageHeaderIconButton::new("map-edit-undo", "arrow.uturn.backward", move |_, cx| {
                                    undo_session.update(cx, |session, cx| {
                                        session.undo();
                                        cx.notify();
                                    });
                                })
                                .font_size(18.0)
                                .disabled(strokes_empty || saving || pending),
                            )
                            .guide_anchor(RiseGuideAnchor::MapEditUndo),
                        )
                        .child(map_control_hint(
                            "map-edit-redo-hint",
                            tr("map_edit_redo"),
                            PageHeaderIconButton::new("map-edit-redo", "arrow.uturn.forward", move |_, cx| {
                                redo_session.update(cx, |session, cx| {
                                    session.redo();
                                    cx.notify();
                                });
                            })
                            .font_size(18.0)
                            .disabled(undone_empty || saving || pending),
                        )),
                )
                .child(MapEditingToolbar::new(session.clone()));
            let cancel = on_close.clone();
            let save_session = session.clone();
            let save_close = on_close.clone();
            let mut save_label = hstack(None);
            if saving {
                save_label = save_label.child(progress_view());
            }
            let save_label = save_label.child(MainText::new(tr("map_edit_save")).px(16.0).font_weight(FontWeight::SEMIBOLD));
            panel = panel.child(
                hstack(10.0)
                    .w_full()
                    .child(
                        div().flex_1().child(
                            MapEditingActionButton::new(
                                "map-edit-cancel",
                                move |window, cx| cancel(window, cx),
                                MainText::new(tr("common_cancel")).px(16.0).font_weight(FontWeight::SEMIBOLD),
                            )
                            .disabled(saving),
                        ),
                    )
                    .child(
                        div().flex_1().child(
                            MapEditingActionButton::new(
                                "map.eraser.save",
                                move |window, cx| save_erased_tracks(&save_session, &save_close, window, cx),
                                save_label,
                            )
                            .primary(true)
                            .disabled(saving || strokes_empty)
                            .guide_anchor(RiseGuideAnchor::MapEditSave),
                        ),
                    ),
            );
        }

        let key_session = session.clone();
        let key_close = on_close.clone();
        let layer_focus = LayerFocus::keyed("map-editing-controls-focus-return", window, cx);
        let (layer_appearing, layer_leaving) = (layer_focus.clone(), layer_focus);
        let appear_focus = focus_handle.clone();
        let disappear_focus = focus_handle.clone();
        let reclaim_focus = focus_handle.clone();
        div()
            .id("map-editing-controls")
            .absolute()
            .inset_0()
            .track_focus(&focus_handle)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let keystroke = &event.keystroke;
                if keystroke.key == "escape" {
                    match &journey_state {
                        Some(state) => {
                            MapJourneyComposerControls::escape(state, &key_session, &key_close, window, cx);
                        }
                        None => {
                            if !key_session.read(cx).saving {
                                key_close(window, cx);
                            }
                        }
                    }
                    cx.stop_propagation();
                    return;
                }
                let (mode, saving, strokes_empty) = {
                    let session = key_session.read(cx);
                    (session.mode, session.saving, session.strokes.is_empty())
                };
                if mode != MapEditingMode::Eraser {
                    return;
                }
                if keystroke.modifiers.secondary() && keystroke.key == "s" {
                    if !saving && !strokes_empty {
                        save_erased_tracks(&key_session, &key_close, window, cx);
                    }
                    cx.stop_propagation();
                    return;
                }
                let handled = key_session.update(cx, |session, cx| {
                    let handled = session.handle_undo_keystroke(keystroke);
                    if handled {
                        cx.notify();
                    }
                    handled
                });
                if handled {
                    cx.stop_propagation();
                }
            })
            .on_mouse_up(MouseButton::Left, move |_: &MouseUpEvent, window, cx| {
                if !reclaim_focus.contains_focused(window, cx) {
                    window.focus(&reclaim_focus, cx);
                }
            })
            .children(eraser_layer)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .px(px(16.0))
                    .pb(px(Self::PANEL_BOTTOM_PADDING))
                    .child(spacer_min_h(0.0))
                    .child(panel),
            )
            .on_appear("map-editing-controls-lifecycle", move |window, cx| {
                LayerFocus::did_appear(&layer_appearing, &appear_focus, window, cx);
            })
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    LayerFocus::will_disappear(&layer_leaving, &disappear_focus, window, cx);
                }
                GuideFacts::set(RiseGuideSignal::MapEraserHasStrokes, false, cx);
            })
    }
}

#[derive(IntoElement)]
pub struct MapEditingToolbar {
    session: Entity<MapEditingSession>,
}

impl MapEditingToolbar {
    pub fn new(session: Entity<MapEditingSession>) -> Self {
        Self { session }
    }
}

impl RenderOnce for MapEditingToolbar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = rise_ui::theme(cx).text.primary;
        let period = MapServicesStore::tracks(cx).period;
        let (saving, hint) = {
            let session = self.session.read(cx);
            let hint = if session.mode == MapEditingMode::Eraser {
                "map_eraser_hint"
            } else if session.drawing() {
                "map_journey_draw_hint"
            } else {
                "map_journey_select_grid"
            };
            (session.saving, hint)
        };
        let locate_session = self.session.clone();
        let is_period_menu_presented_state = window.use_keyed_state("map-edit-period-menu-state", cx, |_, _| false);
        let is_period_menu_presented = Binding::entity(
            is_period_menu_presented_state.clone(),
            |presented: &bool| *presented,
            |presented: &mut bool, value: bool, cx| {
                *presented = value;
                cx.notify();
            },
        );
        let actions: HoldContextMenuPresentationActions = Rc::new(move |_: &App| {
            MapTrackPeriod::ALL
                .iter()
                .map(|period_case| {
                    HoldContextMenuAction::new(period_case.wire(), tr(period_case.title_key())).checked(period == *period_case)
                })
                .collect()
        });
        let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            if let Some(period) = MapTrackPeriod::from_wire(&id) {
                MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.select_track_period(period, cx));
            }
        });
        let source: HoldContextMenuTapSource =
            Rc::new(move |_: &mut Window, _: &mut App| period_glyph(color).into_any_element());
        let period_menu = map_control_hint(
            "map-edit-period-hint",
            tr("map_edit_period"),
            plain("map-edit-period")
                .disabled(saving)
                .on_press(move |_, cx| {
                    is_period_menu_presented_state.update(cx, |presented, cx| {
                        *presented = true;
                        cx.notify();
                    });
                })
                .child(period_glyph(color))
                .tap_context_menu(
                    "map-edit-period-menu",
                    is_period_menu_presented,
                    HoldContextMenuAlignment::Leading,
                    false,
                    false,
                    false,
                    None,
                    actions,
                    on_action,
                    source,
                ),
        );
        hstack(10.0)
            .w_full()
            .child(period_menu)
            .child(
                div().flex_1().child(
                    div()
                        .w_full()
                        .min_h(px(44.0))
                        .px(px(8.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            MainText::new(tr(hint))
                                .px(13.0)
                                .tac(TextAlignT::Center)
                                .font_weight(FontWeight::MEDIUM)
                                .number_of_lines(2),
                        )
                        .liquid_chrome_rounded_rectangle(22.0),
                ),
            )
            .child(MapLocateButton::new(move |_, cx| {
                if !locate_session.read(cx).saving {
                    MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.focus_self(cx));
                }
            }))
    }
}

#[derive(IntoElement)]
pub struct MapEditingActionButton {
    id: ElementId,
    primary: bool,
    disabled: bool,
    action: MapEditingAction,
    label: AnyElement,
}

impl MapEditingActionButton {
    pub fn new(
        id: impl Into<ElementId>,
        action: impl Fn(&mut Window, &mut App) + 'static,
        label: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            primary: false,
            disabled: false,
            action: Rc::new(action),
            label: label.into_any_element(),
        }
    }

    pub fn primary(mut self, primary: bool) -> Self {
        self.primary = primary;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for MapEditingActionButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let tint = if self.primary { theme.primary._100 } else { theme.button.bg_400 };
        let height = theme.button.height_300;
        let action = self.action;
        LiquidGlassButton::new(self.id, LiquidGlassButtonShape::Capsule)
            .horizontal_padding(16.0)
            .vertical_padding(0.0)
            .tint(Some(tint))
            .disabled(self.disabled)
            .on_press(move |window, cx| action(window, cx))
            .child(div().w_full().h(height).flex().items_center().justify_center().child(self.label))
    }
}
