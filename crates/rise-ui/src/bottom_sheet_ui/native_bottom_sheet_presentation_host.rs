use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, ElementId, FocusHandle, Hsla, IntoElement, RenderOnce, WeakFocusHandle, Window,
    anchored, deferred,
};

use crate::bottom_sheet_ui::bottom_sheet_backdrop::BottomSheetBackdrop;
use crate::context_menu_ui::DeferredLayerFloorKey;
use crate::guide_anchor_ui::guide_anchor_registry::{GuideAnchorCoverKind, GuideAnchorCoverModifiers};
use crate::bottom_sheet_ui::modal_sheet_surface::{
    ModalSheetFrame, ModalSheetMetrics, ModalSheetStack, modal_sheet_centering, modal_sheet_chrome, modal_sheet_cycle_focus,
    modal_sheet_keep_focus_inside, modal_sheet_outline, modal_sheet_settle_pointer,
};
use crate::keyboard_dismiss::{ScreenFocus, SheetFocusKey};
use crate::swiftui::animated::Animated;
use crate::swiftui::motion::ease_out;
use crate::swiftui::prelude::*;
use crate::top_presenter::{InsidePresentedLayerKey, TopPresenter};
use crate::window_cover::WindowCoverFrame;

const MEDIUM_DETENT_FRACTION: f32 = 0.5;

/// a height the sheet rests at
/// the desktop modal has no stops, it only learns from detents that the content fills the surface
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PresentationDetent {
    Medium,
    Large,
    Fraction(f32),
    Height(f32),
}

impl PresentationDetent {
    /// sheet height this detent asks for in a container container_height points tall
    pub fn resolved_height(self, container_height: f32) -> f32 {
        let container_height = container_height.max(0.0);
        let height = match self {
            Self::Medium => container_height * MEDIUM_DETENT_FRACTION,
            Self::Large => container_height,
            Self::Fraction(fraction) => container_height * fraction,
            Self::Height(height) => height,
        };
        height.max(0.0).min(container_height)
    }
}

/// whether a scroll over the content resizes the sheet first
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum PresentationContentInteraction {
    #[default]
    Automatic,
    Resizes,
    Scrolls,
}

impl PresentationContentInteraction {
    /// automatic behaves as resizes on a sheet with several detents
    pub fn resizes_first(self) -> bool {
        !matches!(self, Self::Scrolls)
    }
}

type HostAction = Rc<dyn Fn(&mut Window, &mut App)>;
type HostContent = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

#[derive(IntoElement)]
pub struct NativeBottomSheetPresentationHost {
    id: ElementId,
    is_presented: Binding<bool>,
    pub is_content_ready: bool,
    pub presentation_drag_indicator: bool,
    pub presentation_background: Option<Hsla>,
    pub interactive_dismiss_disabled: bool,
    pub presentation_detents: Vec<PresentationDetent>,
    pub presentation_corner_radius: Option<f32>,
    pub presentation_content_interaction: PresentationContentInteraction,
    pub fills_window: bool,
    pub compact: bool,
    pub layer_priority: usize,
    on_prepare: HostAction,
    on_dismiss: HostAction,
    content: HostContent,
}

impl NativeBottomSheetPresentationHost {
    pub const DEFAULT_LAYER_PRIORITY: usize = 1;

    pub fn new<E: IntoElement>(
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        content: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            is_presented,
            is_content_ready: true,
            presentation_drag_indicator: true,
            presentation_background: None,
            interactive_dismiss_disabled: false,
            presentation_detents: Vec::new(),
            presentation_corner_radius: None,
            presentation_content_interaction: PresentationContentInteraction::default(),
            fills_window: false,
            compact: false,
            layer_priority: Self::DEFAULT_LAYER_PRIORITY,
            on_prepare: Rc::new(|_, _| {}),
            on_dismiss: Rc::new(|_, _| {}),
            content: Rc::new(move |window, cx| content(window, cx).into_any_element()),
        }
    }

    pub fn is_content_ready(mut self, is_content_ready: bool) -> Self {
        self.is_content_ready = is_content_ready;
        self
    }

    /// shows or hides the drag indicator on the sheet content, a modal has no grabber either way
    pub fn presentation_drag_indicator(mut self, visible: bool) -> Self {
        self.presentation_drag_indicator = visible;
        self
    }

    /// background of the sheet content, bg._200 when not set
    pub fn presentation_background(mut self, background: impl Into<Hsla>) -> Self {
        self.presentation_background = Some(background.into());
        self
    }

    /// backdrop click and Escape stop closing the sheet
    /// a binding or dismiss action still does
    pub fn interactive_dismiss_disabled(mut self, disabled: bool) -> Self {
        self.interactive_dismiss_disabled = disabled;
        self
    }

    pub fn presentation_detents(
        mut self,
        detents: impl IntoIterator<Item = PresentationDetent>,
    ) -> Self {
        self.presentation_detents = detents.into_iter().collect();
        self
    }

    pub fn presentation_detents_selection(
        mut self,
        detents: impl IntoIterator<Item = PresentationDetent>,
        _selection: Binding<PresentationDetent>,
    ) -> Self {
        self.presentation_detents = detents.into_iter().collect();
        self
    }

    /// a modal keeps ModalSheetMetrics CORNER_RADIUS since its content inset is computed from it
    pub fn presentation_corner_radius(mut self, radius: f32) -> Self {
        self.presentation_corner_radius = Some(radius);
        self
    }

    /// without detents the content always scrolls
    pub fn presentation_content_interaction(
        mut self,
        behavior: PresentationContentInteraction,
    ) -> Self {
        self.presentation_content_interaction = behavior;
        self
    }

    /// desktop layout, a long form takes the wide modal at the height ceiling
    /// instead of the phone sized floor
    pub fn fills_window(mut self, fills_window: bool) -> Self {
        self.fills_window = fills_window;
        self
    }

    /// desktop layout, a small dialog over the modal that opened it (eg the phone code of a purchase)
    /// stays narrow and short so the modal under shows around it
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    /// deferred paint order of the modal, a higher one draws above every modal with less
    /// a dialog a sheet asked for (eg the phone code of a purchase) takes one above all sheets so it never swaps places with them
    pub fn layer_priority(mut self, priority: usize) -> Self {
        self.layer_priority = priority;
        self
    }

    pub fn on_prepare(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_prepare = Rc::new(action);
        self
    }

    pub fn on_dismiss(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Rc::new(action);
        self
    }
}

struct HostState {
    native_presented: bool,
    presentation_scheduled: bool,
    preparation_requested: bool,
    focus_handle: FocusHandle,
    focus_before_presentation: Option<WeakFocusHandle>,
    origin: Instant,
    appearance: Animated<f32>,
}

/// what the deferred closures need, the render time values of the host cloned
#[derive(Clone)]
struct HostWiring {
    state: gpui::Entity<HostState>,
    is_presented: Binding<bool>,
    is_content_ready: bool,
    on_prepare: HostAction,
    on_dismiss: HostAction,
}

impl HostWiring {
    fn schedule_presentation(&self, window: &mut Window, cx: &mut App) {
        let host = self.state.read(cx);
        if !self.is_presented.get(cx) || host.native_presented || host.presentation_scheduled {
            return;
        }

        if !host.preparation_requested {
            self.state.update(cx, |host, _| host.preparation_requested = true);
            (self.on_prepare)(window, cx);
        }
        if !self.is_content_ready {
            return;
        }

        self.state.update(cx, |host, _| host.presentation_scheduled = true);
        let wiring = self.clone();
        window.defer(cx, move |window, cx| {
            wiring.state.update(cx, |host, _| host.presentation_scheduled = false);
            if !wiring.is_presented.get(cx)
                || !wiring.is_content_ready
                || wiring.state.read(cx).native_presented
            {
                return;
            }
            let focus_before_presentation = window.focused(cx).map(|focused| focused.downgrade());
            let focus_handle = wiring.state.update(cx, |host, cx| {
                host.native_presented = true;
                host.focus_before_presentation = focus_before_presentation;
                cx.notify();
                host.focus_handle.clone()
            });
            ScreenFocus::register_sheet(&focus_handle, cx);
            window.focus(&focus_handle, cx);
            modal_sheet_settle_pointer(wiring.state.entity_id(), window);
        });
    }

    /// the sheet leaves the tree but its handle lives on here, so window.focused stays Some
    /// gpui would dispatch keys from the window root past every key context
    /// runs before the sheet closes while the frame still has it
    fn return_focus(&self, window: &mut Window, cx: &mut App) {
        let host = self.state.read(cx);
        if !host.native_presented {
            return;
        }
        let sheet_focus = host.focus_handle.clone();
        let previous = self.state.update(cx, |host, _| host.focus_before_presentation.take());
        if !sheet_focus.contains_focused(window, cx) {
            return;
        }
        let previous = previous
            .and_then(|previous| previous.upgrade())
            .filter(|previous| previous.is_in_rendered_frame(window));
        match previous {
            Some(previous) => window.focus(&previous, cx),
            None => window.blur(),
        }
    }

    /// a sheet on top closing can leave the focus empty and the root view takes it before this host renders
    /// so Escape stays dead until a click
    /// focus inside the sheet or on a sibling stays
    fn reclaim_focus(&self, window: &mut Window, cx: &mut App) {
        let sheet_focus = self.state.read(cx).focus_handle.clone();
        let is_off_the_key_path = match window.focused(cx) {
            None => true,
            Some(focused) => !sheet_focus.is_focused(window) && focused.contains(&sheet_focus, window),
        };
        if is_off_the_key_path {
            window.focus(&sheet_focus, cx);
        }
    }

    fn close(&self, window: &mut Window, cx: &mut App) {
        self.return_focus(window, cx);
        self.state.update(cx, |host, cx| {
            host.presentation_scheduled = false;
            if host.native_presented {
                host.native_presented = false;
                host.appearance.jump(0.0);
                cx.notify();
            }
        });
    }

    fn finish_dismissal(&self, window: &mut Window, cx: &mut App) {
        self.return_focus(window, cx);
        self.state.update(cx, |host, cx| {
            host.native_presented = false;
            host.presentation_scheduled = false;
            host.preparation_requested = false;
            host.appearance.jump(0.0);
            cx.notify();
        });
        if self.is_presented.get(cx) {
            self.is_presented.set(false, window, cx);
        }
        (self.on_dismiss)(window, cx);
    }

    fn motion_time(&self, cx: &App) -> Duration {
        cx.background_executor()
            .now()
            .saturating_duration_since(self.state.read(cx).origin)
    }

    fn frame_appearance(&self, window: &Window, cx: &mut App) -> f32 {
        let now = self.motion_time(cx);
        self.state.update(cx, |host, cx| {
            host.appearance.set_in(1.0, ease_out(ModalSheetMetrics::PRESENT_DURATION), now, cx);
            host.appearance.frame_value(now, window)
        })
    }
}

impl RenderOnce for NativeBottomSheetPresentationHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let clock = cx.background_executor().now();
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| HostState {
            native_presented: false,
            presentation_scheduled: false,
            preparation_requested: false,
            focus_handle: cx.focus_handle(),
            focus_before_presentation: None,
            origin: clock,
            appearance: Animated::new(0.0),
        });
        let wiring = HostWiring {
            state: state.clone(),
            is_presented: self.is_presented.clone(),
            is_content_ready: self.is_content_ready,
            on_prepare: self.on_prepare.clone(),
            on_dismiss: self.on_dismiss.clone(),
        };

        let is_presented = self.is_presented.get(cx);
        let on_open = wiring.clone();
        on_change(
            (self.id.clone(), "is-presented"),
            is_presented,
            false,
            window,
            cx,
            move |_, open, window, cx| {
                if *open {
                    on_open.schedule_presentation(window, cx);
                } else {
                    on_open.close(window, cx);
                }
            },
        );
        let on_ready = wiring.clone();
        on_change(
            (self.id.clone(), "is-content-ready"),
            self.is_content_ready,
            false,
            window,
            cx,
            move |_, ready, window, cx| {
                if *ready && on_ready.is_presented.get(cx) {
                    on_ready.schedule_presentation(window, cx);
                }
            },
        );

        let anchor = div().size_0();
        let on_appear = wiring.clone();
        let anchor = if state.read(cx).native_presented {
            TopPresenter::note_presented_sheet((self.id.clone(), "top-presenter"), &self.is_presented, window, cx);
            anchor.child(env_scope::<InsidePresentedLayerKey>(true).child(self.render_sheet(&wiring, window, cx)))
        } else {
            anchor
        };
        anchor.on_appear((self.id, "appear"), move |window, cx| {
            if on_appear.is_presented.get(cx) {
                on_appear.schedule_presentation(window, cx);
            }
        })
    }
}

impl NativeBottomSheetPresentationHost {
    /// least surface height, only height(h) detents name their points and the surface takes the tallest
    /// any relative detent means the content fills the surface, so fill_floor applies
    pub fn surface_floor(detents: &[PresentationDetent], frame: &ModalSheetFrame) -> Option<f32> {
        if detents.is_empty() {
            return None;
        }
        let fixed_heights = detents.iter().map(|detent| match detent {
            PresentationDetent::Height(height) => Some(*height),
            PresentationDetent::Medium | PresentationDetent::Large | PresentationDetent::Fraction(_) => None,
        });
        match fixed_heights.collect::<Option<Vec<f32>>>() {
            Some(heights) => {
                let tallest = heights.into_iter().fold(0.0_f32, f32::max);
                Some(tallest.max(0.0).min(frame.max_height))
            }
            None => Some(frame.fill_floor()),
        }
    }

    fn render_sheet(&self, wiring: &HostWiring, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = crate::theme(cx);
        let background = self.presentation_background.unwrap_or(theme.bg._200);
        let allows_interactive_dismiss = !self.interactive_dismiss_disabled;
        let cover = WindowCoverFrame::resolve(window, cx);
        let focus_handle = wiring.state.read(cx).focus_handle.clone();
        wiring.reclaim_focus(window, cx);

        let on_backdrop = wiring.clone();
        let on_escape = wiring.clone();
        let on_dismiss_action = wiring.clone();
        let dismiss_action = DismissAction::new(move |window, cx| {
            on_dismiss_action.finish_dismissal(window, cx);
        });
        let content = (self.content)(window, cx);

        let frame = if self.fills_window {
            ModalSheetFrame::for_window_wide(window, cx)
        } else if self.compact {
            ModalSheetFrame::for_window_small(window, cx)
        } else {
            ModalSheetFrame::for_window(window, cx)
        };
        let stacked = ModalSheetStack::enter((self.id.clone(), "modal-stack"), window, cx);
        let surface_floor = Self::surface_floor(&self.presentation_detents, &frame)
            .map(|floor| if self.fills_window { frame.max_height } else { floor })
            .map(|floor| if self.compact { floor } else { stacked.raised(floor, &frame) })
            .or(stacked.floor.filter(|_| !self.compact).map(|floor| floor.min(frame.max_height)));
        let appearance = wiring.frame_appearance(window, cx);

        let body = div()
            .id((self.id.clone(), "modal-body"))
            .w_full()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                env_scope::<DismissKey>(dismiss_action)
                    .child(env_scope::<SheetFocusKey>(Some(focus_handle.clone())).child(content)),
            );
        let surface = vstack(0.0)
            .align(HorizontalAlignment::Leading)
            .w(px(frame.width))
            .max_h(px(frame.max_height))
            .when_some(surface_floor, |surface, floor| surface.min_h(px(floor)))
            .id((self.id.clone(), "surface"))
            .occlude()
            .child(body)
            .child(stacked.measurer((self.id.clone(), "modal-stack-measure")));
        let outline = modal_sheet_outline(crate::theme(cx));
        let surface = modal_sheet_chrome(surface, background, outline, appearance);

        let capture_focus = focus_handle.clone();
        let cycle_focus = focus_handle.clone();
        deferred(
            anchored().position(cover.origin).child(
                div()
                    .relative()
                    .w(cover.size.width)
                    .h(cover.size.height)
                    .track_focus(&focus_handle)
                    .key_context(crate::layer_keys::LAYER_KEY_CONTEXT)
                    .capture_key_down(move |event, window, cx| {
                        modal_sheet_keep_focus_inside(event, &capture_focus, window, cx);
                    })
                    .on_key_down(move |event, window, cx| {
                        if allows_interactive_dismiss && event.keystroke.key == "escape" {
                            cx.stop_propagation();
                            on_escape.finish_dismissal(window, cx);
                        } else if modal_sheet_cycle_focus(event, &cycle_focus, window, cx) {
                            cx.stop_propagation();
                        }
                    })
                    .child(BottomSheetBackdrop::new(true, move |_, window, cx| {
                        if allows_interactive_dismiss {
                            on_backdrop.finish_dismissal(window, cx);
                        }
                    }))
                    .child(
                        env_scope::<DeferredLayerFloorKey>(self.layer_priority)
                            .child(modal_sheet_centering().child(surface)),
                    )
                    .guide_anchor_cover((self.id.clone(), "guide-cover"), GuideAnchorCoverKind::Layer),
            ),
        )
        .with_priority(self.layer_priority)
        .into_any_element()
    }
}
