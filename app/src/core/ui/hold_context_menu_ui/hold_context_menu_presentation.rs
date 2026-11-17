use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext, AvailableSpace, Bounds, ElementId, Entity, IntoElement, ParentElement, RenderOnce,
    Size, Styled, Window, canvas, div, point, px,
};
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::view_anchor_probe::{ViewAnchorProbe, ViewAnchorProbeExt};
use rise_ui::view_screen_snapshot::ViewScreenSource;

use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuSourceView;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::{
    HoldContextMenuPresenter, HoldContextMenuRequest, HoldContextMenuRequestID,
};

pub type HoldContextMenuPresentationActions = Rc<dyn Fn(&App) -> Vec<HoldContextMenuAction>>;
pub type HoldContextMenuPresentationOnAction = Rc<dyn Fn(String, &mut Window, &mut App)>;
pub type HoldContextMenuPresentationPreview = Rc<dyn Fn(f32, &mut Window, &mut App) -> AnyElement>;

pub const HOLD_CONTEXT_MENU_DEFAULT_MAXIMUM_PREVIEW_WIDTH: f32 = 420.0;

pub trait HoldContextMenuPresentationModifiers: IntoElement + Sized {
    /// opens the same overlay as a held source with a live preview centered above its menu
    /// id keeps the request of the anchor across frames
    fn hold_context_menu(
        self,
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        maximum_preview_width: f32,
        actions: HoldContextMenuPresentationActions,
        on_action: HoldContextMenuPresentationOnAction,
        preview: HoldContextMenuPresentationPreview,
    ) -> HoldContextMenuPresentation {
        HoldContextMenuPresentation {
            content: self.into_any_element(),
            anchor: HoldContextMenuPresentationAnchor {
                id: id.into(),
                is_presented,
                maximum_preview_width,
                actions,
                on_action,
                preview,
            },
        }
    }
}

impl<E: IntoElement> HoldContextMenuPresentationModifiers for E {}

#[derive(IntoElement)]
pub struct HoldContextMenuPresentation {
    content: AnyElement,
    anchor: HoldContextMenuPresentationAnchor,
}

impl RenderOnce for HoldContextMenuPresentation {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let anchor = self.anchor;
        let coordinator = window.use_keyed_state(anchor.id.clone(), cx, |_, cx| Coordinator::new(cx));
        let probe_handle = coordinator.read(cx).source.probe.clone();
        let is_presented = anchor.is_presented.clone();
        let disappear_id = ElementId::from((anchor.id.clone(), "disappear"));
        coordinator.update(cx, |coordinator, _| coordinator.configuration = Some(anchor));
        let scheduled = coordinator.clone();
        let probe = canvas(
            move |_, window, cx| HoldContextMenuPresentationAnchor::schedule_update(scheduled, window, cx),
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();
        let probe = div()
            .absolute()
            .inset_0()
            .child(probe)
            .view_anchor_probe(&probe_handle)
            .on_disappear(disappear_id, move |window, cx| {
                if let Some(window) = window {
                    is_presented.set(false, window, cx);
                }
                let request_id = coordinator.update(cx, |coordinator, _| {
                    coordinator.configuration = None;
                    coordinator.request_id.take()
                });
                if let Some(id) = request_id {
                    HoldContextMenuPresenter::dismiss_if_active(id, cx);
                }
            });
        div().relative().child(self.content).child(probe)
    }
}

#[derive(Clone)]
struct HoldContextMenuPresentationAnchor {
    id: ElementId,
    is_presented: Binding<bool>,
    maximum_preview_width: f32,
    actions: HoldContextMenuPresentationActions,
    on_action: HoldContextMenuPresentationOnAction,
    preview: HoldContextMenuPresentationPreview,
}

struct Coordinator {
    configuration: Option<HoldContextMenuPresentationAnchor>,
    source: ViewScreenSource,
    request_id: Option<HoldContextMenuRequestID>,
    update_scheduled: bool,
}

impl Coordinator {
    fn new(cx: &mut App) -> Self {
        let view = cx.new(|_| HoldContextMenuSourceView::new(Rc::new(|_: &mut Window, _: &mut App| div().into_any_element())));
        Self {
            configuration: None,
            source: ViewScreenSource::new(view, ViewAnchorProbe::new()),
            request_id: None,
            update_scheduled: false,
        }
    }
}

impl HoldContextMenuPresentationAnchor {
    fn preview_content(preview: &HoldContextMenuPresentationPreview, width: f32, window: &mut Window, cx: &mut App) -> AnyElement {
        div()
            .w(px(width))
            .flex_none()
            .child(preview(width, window, cx))
            .into_any_element()
    }

    fn schedule_update(coordinator: Entity<Coordinator>, window: &mut Window, cx: &mut App) {
        let Some(configuration) = coordinator.update(cx, |coordinator, _| {
            if coordinator.update_scheduled {
                return None;
            }
            coordinator.configuration.clone()
        }) else {
            return;
        };
        let presented = configuration.is_presented.get(cx);
        let needs_presenting = presented && coordinator.read(cx).request_id.is_none();
        let viewport = window.viewport_size();
        let width = f32::min(
            configuration.maximum_preview_width,
            f32::max(1.0, f32::from(viewport.width) - 32.0),
        );
        let height = if needs_presenting {
            let mut measured = Self::preview_content(&configuration.preview, width, window, cx);
            let size = measured.layout_as_root(
                Size {
                    width: AvailableSpace::Definite(px(width)),
                    height: AvailableSpace::MinContent,
                },
                window,
                cx,
            );
            f32::max(1.0, f32::from(size.height))
        } else {
            1.0
        };
        coordinator.update(cx, |coordinator, _| coordinator.update_scheduled = true);
        window.defer(cx, move |window, cx| {
            coordinator.update(cx, |coordinator, _| coordinator.update_scheduled = false);
            let Some(configuration) = coordinator.read(cx).configuration.clone() else {
                return;
            };
            let request_id = coordinator.read(cx).request_id;
            if !configuration.is_presented.get(cx) {
                if let Some(id) = request_id {
                    HoldContextMenuPresenter::request_dismiss(id, cx);
                }
                return;
            }
            if request_id.is_some() || !needs_presenting {
                return;
            }
            let frame = Bounds::new(
                point(
                    px((f32::from(viewport.width) - width) / 2.0),
                    px((f32::from(viewport.height) - height) / 2.0),
                ),
                gpui::size(px(width), px(height)),
            );
            let id = HoldContextMenuRequestID::new();
            coordinator.update(cx, |coordinator, _| coordinator.request_id = Some(id));
            let preview = configuration.preview.clone();
            let dismissed = coordinator.clone();
            let window_handle = window.window_handle();
            let scroll_lock_from = coordinator.read(cx).source.clone();
            let request = HoldContextMenuRequest {
                id,
                source_frame: frame,
                initial_source_center: frame.center(),
                alignment: HoldContextMenuAlignment::Trailing,
                show_emoji_list: false,
                emojis: Vec::new(),
                allows_custom_emoji: false,
                expanded_emojis: None,
                actions: (configuration.actions)(cx),
                accessory: None,
                on_emoji: Rc::new(|_: String, _: &mut Window, _: &mut App| {}),
                on_action: configuration.on_action.clone(),
                on_lift_source: Rc::new(|_: &mut App| {}),
                on_restore_source: Rc::new(|_: &mut App| {}),
                source_lift: None,
                snapshot: None,
                source_bubble_clip: None,
                initial_lift_scale: HoldContextMenuMetrics::INITIAL_PANEL_SCALE,
                uses_flat_chrome: false,
                content: Rc::new(move |window: &mut Window, cx: &mut App| Self::preview_content(&preview, width, window, cx)),
                is_centered: true,
                live_actions: Some(configuration.actions.clone()),
                on_dismiss: Rc::new(move |cx: &mut App| {
                    let binding = dismissed.update(cx, |coordinator, _| {
                        if coordinator.request_id != Some(id) {
                            return None;
                        }
                        coordinator.request_id = None;
                        coordinator.configuration.as_ref().map(|configuration| configuration.is_presented.clone())
                    });
                    if let Some(is_presented) = binding {
                        cx.defer(move |cx| {
                            window_handle
                                .update(cx, |_, window, cx| is_presented.set(false, window, cx))
                                .ok();
                        });
                    }
                }),
                controller_preview: None,
                prefers_dark_chrome: false,
                keeps_source_editing: false,
                max_visible_rows: None,
            };
            HoldContextMenuPresenter::present(request, Some(&scroll_lock_from), None, window, cx);
        });
    }
}
