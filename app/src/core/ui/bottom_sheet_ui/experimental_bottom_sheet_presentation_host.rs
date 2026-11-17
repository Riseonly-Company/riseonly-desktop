use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Context, ElementId, Empty, Entity, Global, IntoElement, RenderOnce, SharedString,
    Subscription, Task, Window, anchored, deferred, };
use rise_ui::window_cover::WindowCoverFrame;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::guide_anchor_ui::guide_anchor_registry::{GuideAnchorCoverKind, GuideAnchorCoverModifiers};
use rise_ui::swiftui::prelude::*;
use rise_ui::top_presenter::{InsidePresentedLayerKey, TopPresenter};

use crate::core::lib::sai::sai::Sai;
use crate::core::ui::bottom_sheet_ui::sticker_hold_overlay_hooks::sticker_hold_overlay_hooks;

type HostAction = Rc<dyn Fn(&mut Window, &mut App)>;
type HostContent = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

pub const SHEET_LAYER_BASE_PRIORITY: usize = 1;

const TRACE_NAME: &str = "SheetHost";

pub use rise_ui::context_menu_ui::DeferredLayerFloorKey as SheetLayerFloorKey;

/// one open host in the window layer stack
#[derive(Clone)]
struct SheetLayerRecord {
    id: u64,
    name: SharedString,
    is_retiring: bool,
}

/// layers open in the app, lowest first
/// the presented controller chain
#[derive(Default)]
pub struct SheetLayerStack {
    layers: Vec<SheetLayerRecord>,
    next_id: u64,
}

impl Global for SheetLayerStack {}

impl SheetLayerStack {
    fn push(name: SharedString, cx: &mut App) -> u64 {
        let stack = cx.default_global::<Self>();
        if let Some(orphan) = stack.layers.iter_mut().find(|layer| layer.name == name) {
            tracing::warn!(target: "riseonly::X1421", name = %name, layer = orphan.id, "sheet host adopts orphan layer");
            orphan.is_retiring = false;
            return orphan.id;
        }
        stack.next_id += 1;
        let id = stack.next_id;
        stack.layers.push(SheetLayerRecord { id, name, is_retiring: false });
        id
    }

    fn remove(id: u64, cx: &mut App) {
        if cx.has_global::<Self>() {
            cx.global_mut::<Self>().layers.retain(|layer| layer.id != id);
        }
    }

    /// true if the flag changed
    fn set_retiring(id: u64, retiring: bool, cx: &mut App) -> bool {
        let stack = cx.default_global::<Self>();
        let Some(layer) = stack.layers.iter_mut().find(|layer| layer.id == id) else {
            return false;
        };
        let changed = layer.is_retiring != retiring;
        layer.is_retiring = retiring;
        changed
    }

    /// true when the presented chain ends at the root
    /// meaning no layer is open or the lowest is already retiring
    pub fn top_is_root(cx: &App) -> bool {
        cx.try_global::<Self>()
            .and_then(|stack| stack.layers.first())
            .is_none_or(|layer| layer.is_retiring)
    }

    /// true when no sheet layer is up or retiring anywhere in the app
    pub fn is_empty(cx: &App) -> bool {
        cx.try_global::<Self>().is_none_or(|stack| stack.layers.is_empty())
    }

    fn is_retiring(id: u64, cx: &App) -> bool {
        cx.try_global::<Self>()
            .and_then(|stack| stack.layers.iter().find(|layer| layer.id == id))
            .is_some_and(|layer| layer.is_retiring)
    }

    fn position(id: u64, cx: &App) -> usize {
        cx.try_global::<Self>()
            .and_then(|stack| stack.layers.iter().position(|layer| layer.id == id))
            .unwrap_or_default()
    }
}

/// open layers from the window root up, one per line
/// for the log when a sheet host or back gesture is refused
pub enum PresentationChainTrace {}

impl PresentationChainTrace {
    pub fn describe(cx: &App) -> String {
        let mut lines = vec![String::from("root")];
        if let Some(stack) = cx.try_global::<SheetLayerStack>() {
            lines.extend(
                stack
                    .layers
                    .iter()
                    .map(|layer| format!("{} retiring={}", layer.name, layer.is_retiring)),
            );
        }
        lines.join(" > ")
    }

    pub fn log(event: &str, cx: &mut App) {
        let line = format!("{event}: {}", Self::describe(cx));
        Sai::log_channel(TRACE_NAME, move || line, TRACE_NAME, cx);
    }
}

#[derive(IntoElement)]
pub struct ExperimentalBottomSheetPresentationHost {
    id: ElementId,
    is_presented: Binding<bool>,
    pub dismissal_delay: f64,
    on_did_dismiss: HostAction,
    pub on_presentation_refused: Rc<dyn Fn(&mut Window, &mut App)>,
    content: HostContent,
}

impl ExperimentalBottomSheetPresentationHost {
    pub fn new<E: IntoElement>(
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        content: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            is_presented,
            dismissal_delay: BottomSheetMetrics::DISMISS_DURATION + 0.08,
            on_did_dismiss: Rc::new(|_, _| {}),
            on_presentation_refused: Rc::new(|_, _| {}),
            content: Rc::new(move |window, cx| content(window, cx).into_any_element()),
        }
    }

    pub fn dismissal_delay(mut self, delay: f64) -> Self {
        self.dismissal_delay = delay;
        self
    }

    pub fn on_did_dismiss(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_did_dismiss = Rc::new(action);
        self
    }

    pub fn on_presentation_refused(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_presentation_refused = Rc::new(action);
        self
    }

    /// layer tree is laid out after the window and outside every environment scope of the page
    /// so values the sheet reads from its surroundings are carried over here
    fn hosted_content(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        env_scope::<LayoutDirectionKey>(layout_direction(cx))
            .child(env_scope::<InsidePresentedLayerKey>(true).child((self.content)(window, cx)))
            .into_any_element()
    }

    fn hosted_overlay(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        match sticker_hold_overlay_hooks() {
            Some(hooks) => (hooks.hosted_overlay)(self.id.clone(), window, cx),
            None => Empty.into_any_element(),
        }
    }
}

/// what the coordinator reads from the latest render
#[derive(Clone)]
struct HostParent {
    is_presented: Binding<bool>,
    dismissal_delay: f64,
    on_did_dismiss: HostAction,
}

struct Coordinator {
    parent: HostParent,
    name: SharedString,
    layer: Option<u64>,
    pending_dismissal: Option<Task<()>>,
    window_activation_observer: Option<Subscription>,
    is_torn_down: bool,
}

impl Coordinator {
    fn new(parent: HostParent, name: SharedString) -> Self {
        tracing::warn!(target: "riseonly::X1421", name = %name, "sheet host coordinator new");
        Self {
            parent,
            name,
            layer: None,
            pending_dismissal: None,
            window_activation_observer: None,
            is_torn_down: false,
        }
    }

    fn reconcile_presentation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_torn_down {
            return;
        }
        tracing::warn!(target: "riseonly::X808", name = %self.name, presented = self.parent.is_presented.get(cx), layer = ?self.layer, "sheet host reconcile");
        if !self.parent.is_presented.get(cx) {
            self.cancel_presentation_retry();
            self.schedule_dismissal_if_needed(None, window, cx);
            return;
        }
        if !window.is_window_active() && !crate::app::dev_drive::ENABLED {
            self.wait_for_activation(window, cx);
            return;
        }
        self.cancel_presentation_retry();
        self.pending_dismissal = None;

        if let Some(layer) = self.layer {
            if SheetLayerStack::set_retiring(layer, false, cx) {
                cx.notify();
            }
            return;
        }
        self.layer = Some(SheetLayerStack::push(self.name.clone(), cx));
        cx.notify();
    }

    fn wait_for_activation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.window_activation_observer.is_some() {
            return;
        }
        PresentationChainTrace::log("presentation waits for window activation", cx);
        self.window_activation_observer =
            Some(cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.window_activation_observer = None;
                    this.reconcile_presentation(window, cx);
                }
            }));
    }

    fn cancel_presentation_retry(&mut self) {
        self.window_activation_observer = None;
    }

    fn schedule_dismissal_if_needed(
        &mut self,
        delay: Option<f64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layer) = self.layer else {
            return;
        };
        if self.pending_dismissal.is_some() {
            return;
        }
        SheetLayerStack::set_retiring(layer, true, cx);
        cx.notify();
        let delay = Duration::from_secs_f64(delay.unwrap_or(self.parent.dismissal_delay).max(0.0));
        self.pending_dismissal = Some(cx.spawn_in(window, async move |coordinator, cx| {
            cx.background_executor().timer(delay).await;
            coordinator
                .update_in(cx, |this, window, cx| this.finish_dismissal(window, cx))
                .ok();
        }));
    }

    fn finish_dismissal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_torn_down {
            return;
        }
        self.pending_dismissal = None;
        tracing::warn!(target: "riseonly::X808", name = %self.name, presented = self.parent.is_presented.get(cx), layer = ?self.layer, "sheet host finish dismissal");
        if self.parent.is_presented.get(cx) {
            return;
        }
        let Some(layer) = self.layer.take() else {
            return;
        };
        SheetLayerStack::remove(layer, cx);
        cx.notify();
        (self.parent.on_did_dismiss)(window, cx);
        if self.parent.is_presented.get(cx) {
            self.reconcile_presentation(window, cx);
        }
    }

    fn teardown(&mut self, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X1421", name = %self.name, layer = ?self.layer, "sheet host teardown");
        self.is_torn_down = true;
        self.pending_dismissal = None;
        self.cancel_presentation_retry();
        if let Some(layer) = self.layer.take() {
            PresentationChainTrace::log("abandon", cx);
            SheetLayerStack::remove(layer, cx);
            cx.notify();
        }
    }
}

impl RenderOnce for ExperimentalBottomSheetPresentationHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let parent = HostParent {
            is_presented: self.is_presented.clone(),
            dismissal_delay: self.dismissal_delay,
            on_did_dismiss: self.on_did_dismiss.clone(),
        };
        let coordinator = self.make_coordinator(parent.clone(), window, cx);
        Self::update_ui_view_controller(&coordinator, parent, cx);

        let reconcile = coordinator.clone();
        on_change(
            (self.id.clone(), "is-presented"),
            self.is_presented.get(cx),
            false,
            window,
            cx,
            move |_, _, window, cx| {
                reconcile.update(cx, |coordinator, cx| coordinator.reconcile_presentation(window, cx));
            },
        );

        let mut anchor = div().size_0();
        if let Some(layer) = coordinator.read(cx).layer {
            TopPresenter::note_presented((self.id.clone(), "top-presenter"), window, cx);
            let priority = env::<SheetLayerFloorKey>(cx) + SHEET_LAYER_BASE_PRIORITY + SheetLayerStack::position(layer, cx);
            let cover = WindowCoverFrame::resolve(window, cx);
            let content = self.hosted_content(window, cx);
            let overlay = self.hosted_overlay(window, cx);
            let closing_guard = SheetLayerStack::is_retiring(layer, cx)
                .then(|| div().id((self.id.clone(), "closing-guard")).absolute().inset_0().occlude());
            anchor = anchor.child(
                deferred(
                    anchored().position(cover.origin).child(
                        div()
                            .relative()
                            .w(cover.size.width)
                            .h(cover.size.height)
                            .child(content)
                            .child(overlay)
                            .children(closing_guard)
                            .guide_anchor_cover((self.id.clone(), "guide-cover"), GuideAnchorCoverKind::Layer),
                    ),
                )
                .with_priority(priority),
            );
        }

        let on_appear = coordinator.clone();
        let on_disappear = coordinator;
        anchor
            .on_appear((self.id, "presenter"), move |window, cx| {
                on_appear.update(cx, |coordinator, cx| {
                    coordinator.is_torn_down = false;
                    coordinator.reconcile_presentation(window, cx);
                });
            })
            .and_on_disappear(move |_, cx| {
                Self::dismantle_ui_view_controller(&on_disappear, cx);
            })
    }
}

impl ExperimentalBottomSheetPresentationHost {
    fn make_coordinator(
        &self,
        parent: HostParent,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Coordinator> {
        let name: SharedString = format!("{TRACE_NAME}({})", self.id).into();
        window.use_keyed_state(self.id.clone(), cx, move |_, _| Coordinator::new(parent, name))
    }

    fn update_ui_view_controller(coordinator: &Entity<Coordinator>, parent: HostParent, cx: &mut App) {
        coordinator.update(cx, |coordinator, _| coordinator.parent = parent);
    }

    fn dismantle_ui_view_controller(coordinator: &Entity<Coordinator>, cx: &mut App) {
        coordinator.update(cx, |coordinator, cx| coordinator.teardown(cx));
    }
}
