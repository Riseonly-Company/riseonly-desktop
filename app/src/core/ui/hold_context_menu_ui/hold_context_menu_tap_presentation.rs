use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext, Bounds, ElementId, Entity, IntoElement, ParentElement, Pixels, RenderOnce, Size,
    Styled, Window, canvas, div, px,
};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{
    HoldContextMenuAction, HoldContextMenuAlignment, HoldSourceBubbleClip,
};
use rise_ui::main_text::{IgnoresTextSizeDeltaKey, ignores_text_size_delta};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::env_scope;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::view_anchor_probe::{ViewAnchorProbe, ViewAnchorProbeExt};
use rise_ui::view_screen_snapshot::ViewScreenSource;

use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuSourceView;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_host::HoldContextMenuWindowPresenter;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::{
    HoldContextMenuActionHandler, HoldContextMenuPresenter, HoldContextMenuRequest, HoldContextMenuRequestID,
};

pub type HoldContextMenuTapSource = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

pub trait HoldContextMenuTapPresentationModifiers: IntoElement + Sized {
    /// opens the app own context menu from a tap anchored on this view
    /// id keeps the anchor request across frames
    /// why: a system menu lets the dismissing touch fall through and kills pans, this one owns its touches
    #[allow(clippy::too_many_arguments)]
    fn tap_context_menu(
        self,
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        alignment: HoldContextMenuAlignment,
        prefers_dark_chrome: bool,
        keeps_source_editing: bool,
        source_is_capsule: bool,
        max_visible_rows: Option<i64>,
        actions: HoldContextMenuPresentationActions,
        on_action: HoldContextMenuPresentationOnAction,
        source: HoldContextMenuTapSource,
    ) -> HoldContextMenuTapPresentation {
        HoldContextMenuTapPresentation {
            content: self.into_any_element(),
            anchor: HoldContextMenuTapAnchor {
                id: id.into(),
                is_presented,
                alignment,
                prefers_dark_chrome,
                keeps_source_editing,
                source_is_capsule,
                max_visible_rows,
                actions,
                on_action,
                source,
                ignores_text_size_delta: false,
            },
        }
    }
}

impl<E: IntoElement> HoldContextMenuTapPresentationModifiers for E {}

trait HoldSourceBubbleClipCapsule {
    fn capsule(size: Size<Pixels>) -> Self;
}

impl HoldSourceBubbleClipCapsule for HoldSourceBubbleClip {
    fn capsule(size: Size<Pixels>) -> Self {
        let radius = px(f32::min(f32::from(size.width), f32::from(size.height)) / 2.0);
        HoldSourceBubbleClip::new(radius, radius, radius, radius)
    }
}

#[derive(IntoElement)]
pub struct HoldContextMenuTapPresentation {
    content: AnyElement,
    anchor: HoldContextMenuTapAnchor,
}

impl RenderOnce for HoldContextMenuTapPresentation {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut anchor = self.anchor;
        anchor.ignores_text_size_delta = ignores_text_size_delta(cx);
        let coordinator = window.use_keyed_state(anchor.id.clone(), cx, |_, cx| Coordinator::new(cx));
        let probe_handle = coordinator.read(cx).source.probe.clone();
        let is_presented = anchor.is_presented.clone();
        let disappear_id = ElementId::from((anchor.id.clone(), "disappear"));
        coordinator.update(cx, |coordinator, _| coordinator.configuration = Some(anchor));
        let scheduled = coordinator.clone();
        let probe = canvas(
            move |_, window, cx| HoldContextMenuTapAnchor::schedule_update(scheduled, window, cx),
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
                if let Some(window) = window
                    && is_presented.get(cx)
                {
                    is_presented.set(false, window, cx);
                }
                let request_id = coordinator.update(cx, |coordinator, _| {
                    coordinator.configuration = None;
                    coordinator.request_id.take()
                });
                if let Some(id) = request_id {
                    tracing::warn!(target: "riseonly::X1529", "tap menu anchor disappeared, menu dismissed");
                    HoldContextMenuPresenter::dismiss_if_active(id, cx);
                }
            });
        div().relative().child(probe).child(self.content)
    }
}

#[derive(Clone)]
struct HoldContextMenuTapAnchor {
    id: ElementId,
    is_presented: Binding<bool>,
    alignment: HoldContextMenuAlignment,
    prefers_dark_chrome: bool,
    keeps_source_editing: bool,
    source_is_capsule: bool,
    max_visible_rows: Option<i64>,
    actions: HoldContextMenuPresentationActions,
    on_action: HoldContextMenuPresentationOnAction,
    source: HoldContextMenuTapSource,
    ignores_text_size_delta: bool,
}

struct Coordinator {
    configuration: Option<HoldContextMenuTapAnchor>,
    source: ViewScreenSource,
    request_id: Option<HoldContextMenuRequestID>,
    update_scheduled: bool,
    frame_retried: bool,
}

impl Coordinator {
    fn new(cx: &mut App) -> Self {
        let view = cx.new(|_| HoldContextMenuSourceView::new(Rc::new(|_: &mut Window, _: &mut App| div().into_any_element())));
        Self {
            configuration: None,
            source: ViewScreenSource::new(view, ViewAnchorProbe::new()),
            request_id: None,
            update_scheduled: false,
            frame_retried: false,
        }
    }
}

impl HoldContextMenuTapAnchor {
    fn schedule_update(coordinator: Entity<Coordinator>, window: &mut Window, cx: &mut App) {
        let already = coordinator.update(cx, |coordinator, _| {
            let already = coordinator.update_scheduled;
            coordinator.update_scheduled = true;
            already
        });
        if already {
            return;
        }
        window.defer(cx, move |window, cx| {
            coordinator.update(cx, |coordinator, _| coordinator.update_scheduled = false);
            let Some(configuration) = coordinator.read(cx).configuration.clone() else {
                return;
            };
            let request_id = coordinator.read(cx).request_id;
            if configuration.is_presented.get(cx) != request_id.is_some() {
                tracing::warn!(
                    target: "riseonly::X1271",
                    anchor = ?configuration.id,
                    presented = configuration.is_presented.get(cx),
                    request = request_id.is_some(),
                    "tap menu update"
                );
            }
            if !configuration.is_presented.get(cx) {
                if let Some(id) = request_id {
                    HoldContextMenuPresenter::request_dismiss(id, cx);
                }
                return;
            }
            if request_id.is_some() {
                return;
            }
            let scroll_lock_from = coordinator.read(cx).source.clone();
            let frame = scroll_lock_from.window_frame();
            let usable = frame.filter(|frame| f32::from(frame.size.width) > 1.0 && f32::from(frame.size.height) > 1.0);
            let menu_active = HoldContextMenuPresenter::has_active_request(cx);
            let retried = coordinator.read(cx).frame_retried;
            tracing::warn!(
                target: "riseonly::X1271",
                frame = ?usable,
                menu_active,
                retried,
                "tap menu present attempt"
            );
            if usable.is_none() && !menu_active && !retried {
                coordinator.update(cx, |coordinator, _| coordinator.frame_retried = true);
                window.refresh();
                return;
            }
            coordinator.update(cx, |coordinator, _| coordinator.frame_retried = false);
            let Some(frame) = usable.filter(|_| !menu_active) else {
                configuration.is_presented.set(false, window, cx);
                return;
            };
            let id = HoldContextMenuRequestID::new();
            coordinator.update(cx, |coordinator, _| coordinator.request_id = Some(id));
            let source = configuration.source.clone();
            let ignores_text_size_delta = configuration.ignores_text_size_delta;
            let dismissed = coordinator.clone();
            let window_handle = window.window_handle();
            let request = HoldContextMenuRequest {
                id,
                source_frame: frame,
                initial_source_center: frame.center(),
                alignment: configuration.alignment,
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
                source_bubble_clip: configuration
                    .source_is_capsule
                    .then(|| HoldSourceBubbleClip::capsule(frame.size)),
                initial_lift_scale: 1.0,
                uses_flat_chrome: false,
                content: Rc::new(move |window: &mut Window, cx: &mut App| {
                    div()
                        .w(frame.size.width)
                        .h(frame.size.height)
                        .flex_none()
                        .child(env_scope::<IgnoresTextSizeDeltaKey>(ignores_text_size_delta).child(source(window, cx)))
                        .into_any_element()
                }),
                is_centered: false,
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
                prefers_dark_chrome: configuration.prefers_dark_chrome,
                keeps_source_editing: configuration.keeps_source_editing,
                max_visible_rows: configuration.max_visible_rows,
            };
            HoldContextMenuPresenter::present(request, Some(&scroll_lock_from), None, window, cx);
            window.on_next_frame(move |window, _| {
                window.on_next_frame(move |window, cx| {
                    let still_active = HoldContextMenuPresenter::shared(cx)
                        .read(cx)
                        .active_request()
                        .is_some_and(|request| request.id == id);
                    if still_active && !HoldContextMenuWindowPresenter::has_mounted_overlay(id, cx) {
                        window.refresh();
                    }
                });
            });
        });
    }
}

/// opens the app own menu anchored on a mounted view, lifted over the backdrop as a snapshot
/// why: system context menu interactions on ios 26 leave pans dead after the menu closes
pub enum HoldContextMenuUIKitPresentation {}

impl HoldContextMenuUIKitPresentation {
    /// rect is in the view own coordinates, none takes the whole view
    pub fn present(
        from: &ViewScreenSource,
        rect: Option<Bounds<Pixels>>,
        alignment: HoldContextMenuAlignment,
        actions: Vec<HoldContextMenuAction>,
        on_action: HoldContextMenuActionHandler,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if actions.is_empty() || HoldContextMenuPresenter::has_active_request(cx) {
            return false;
        }
        let Some(view_frame) = from.window_frame() else {
            return false;
        };
        let frame = match rect {
            Some(rect) => Bounds::new(view_frame.origin + rect.origin, rect.size),
            None => view_frame,
        };
        if f32::from(frame.size.width) <= 1.0 || f32::from(frame.size.height) <= 1.0 {
            return false;
        }
        let snapshot = None;
        let request = HoldContextMenuRequest {
            id: HoldContextMenuRequestID::new(),
            source_frame: frame,
            initial_source_center: frame.center(),
            alignment,
            show_emoji_list: false,
            emojis: Vec::new(),
            allows_custom_emoji: false,
            expanded_emojis: None,
            actions,
            accessory: None,
            on_emoji: Rc::new(|_: String, _: &mut Window, _: &mut App| {}),
            on_action,
            on_lift_source: Rc::new(|_: &mut App| {}),
            on_restore_source: Rc::new(|_: &mut App| {}),
            source_lift: None,
            snapshot,
            source_bubble_clip: None,
            initial_lift_scale: 1.0,
            uses_flat_chrome: false,
            content: Rc::new(|_: &mut Window, _: &mut App| div().into_any_element()),
            is_centered: false,
            live_actions: None,
            on_dismiss: Rc::new(|_: &mut App| {}),
            controller_preview: None,
            prefers_dark_chrome: false,
            keeps_source_editing: false,
            max_visible_rows: None,
        };
        HoldContextMenuPresenter::present(request, Some(from), None, window, cx);
        true
    }
}
