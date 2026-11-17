use std::cell::Cell;
use std::collections::HashSet;
use std::rc::{Rc, Weak};

use gpui::{
    AnyElement, App, AsyncWindowContext, Bounds, Context, Element, ElementId, Entity, GlobalElementId,
    InspectorElementId, InteractiveElement, IntoElement, LayoutId, ParentElement, Pixels, RenderOnce, Styled, Subscription, Window, div,
    px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::frame::SwiftUiAlignedFrame;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::{MotionSpec, spring};
use rise_ui::swiftui::stacks::Alignment;

use crate::core::widgets::navigations::main_tab_chrome_metrics_hooks::MainTabChrome;

/// what occupies the bottom of the window right now, one owner per place
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BottomChromeOwner {
    MainTabBar,
    TabBarMiniPlayer,
    PushedMiniPlayer,
    FloatingAction,
    ChatComposer,
    BottomTabStrip,
}

impl BottomChromeOwner {
    pub const ALL: [Self; 6] = [
        Self::MainTabBar,
        Self::TabBarMiniPlayer,
        Self::PushedMiniPlayer,
        Self::FloatingAction,
        Self::ChatComposer,
        Self::BottomTabStrip,
    ];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::MainTabBar => "mainTabBar",
            Self::TabBarMiniPlayer => "tabBarMiniPlayer",
            Self::PushedMiniPlayer => "pushedMiniPlayer",
            Self::FloatingAction => "floatingAction",
            Self::ChatComposer => "chatComposer",
            Self::BottomTabStrip => "bottomTabStrip",
        }
    }
}

/// candidate for occupied bottom, what is really in the window at read time
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BottomChromeCandidate {
    pub owner: BottomChromeOwner,
    pub top_edge_in_window: f32,
}

/// probe view hung under a chrome subtree
/// lives in the chrome element state, exists only while drawn
/// frame is the last layout one in window coords
pub struct BottomChromeProbeView {
    pub owner: BottomChromeOwner,
    frame: Cell<Option<Bounds<Pixels>>>,
    visible_frame: Cell<Option<Bounds<Pixels>>>,
}

impl BottomChromeProbeView {
    pub fn new(owner: BottomChromeOwner) -> Self {
        Self {
            owner,
            frame: Cell::new(None),
            visible_frame: Cell::new(None),
        }
    }
}

/// single source of truth for how much window bottom is taken
/// weak probe list and a change counter, geometry read at request time
/// readers watch only revision, it moves when probes come or go
pub struct BottomChromeInsets {
    revision: u64,
    occluders: HashSet<Uuid>,
    probes: Vec<Weak<BottomChromeProbeView>>,
}

impl BottomChromeInsets {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            revision: 0,
            occluders: HashSet::new(),
            probes: Vec::new(),
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn probe_did_enter_window(&mut self, probe: &Rc<BottomChromeProbeView>, cx: &mut Context<Self>) {
        self.probes.retain(|known| known.strong_count() > 0);
        let weak = Rc::downgrade(probe);
        if !self.probes.iter().any(|known| known.ptr_eq(&weak)) {
            self.probes.push(weak);
        }
        self.bump(cx);
    }

    pub fn probe_did_leave_window(&mut self, probe: &Weak<BottomChromeProbeView>, cx: &mut Context<Self>) {
        self.probes.retain(|known| known.strong_count() > 0 && !known.ptr_eq(probe));
        self.bump(cx);
    }

    pub fn chrome_layout_did_change(&mut self, cx: &mut Context<Self>) {
        self.bump(cx);
    }

    pub fn set_occluder(&mut self, id: Uuid, is_active: bool, cx: &mut Context<Self>) {
        let changed = if is_active { self.occluders.insert(id) } else { self.occluders.remove(&id) };
        if changed {
            cx.notify();
        }
    }

    /// highest occupied edge of what is really in the window
    /// excluding is for callers that are bottom chrome themselves or the action chases its own probe
    /// declared occluders are ignored here on purpose
    pub fn top_edge_in_window(&self, excluding: &[BottomChromeOwner]) -> Option<f32> {
        let candidates: Vec<BottomChromeCandidate> = self
            .probes
            .iter()
            .filter_map(Weak::upgrade)
            .filter_map(|probe| {
                let top = BottomChromeGeometry::top_edge_in_window(&probe)?;
                Some(BottomChromeCandidate {
                    owner: probe.owner,
                    top_edge_in_window: top,
                })
            })
            .collect();

        BottomChromePolicy::top_edge_in_window(&candidates, self.native_tab_bar_top_edge_in_window(), excluding)
    }

    pub fn top_edge_in_window_under(&self, excluding: &[BottomChromeOwner], host: Bounds<Pixels>) -> Option<f32> {
        let candidates: Vec<BottomChromeCandidate> = self
            .probes
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|probe| {
                probe
                    .visible_frame
                    .get()
                    .is_some_and(|frame| frame.left() < host.right() && frame.right() > host.left())
            })
            .filter_map(|probe| {
                let top = BottomChromeGeometry::top_edge_in_window(&probe)?;
                Some(BottomChromeCandidate {
                    owner: probe.owner,
                    top_edge_in_window: top,
                })
            })
            .collect();

        BottomChromePolicy::top_edge_in_window(&candidates, self.native_tab_bar_top_edge_in_window(), excluding)
    }

    pub fn frames_in_window(&self, owner: BottomChromeOwner) -> Vec<Bounds<Pixels>> {
        self.probes
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|probe| probe.owner == owner)
            .filter_map(|probe| probe.frame.get())
            .filter(BottomChromeGeometry::is_effectively_visible)
            .collect()
    }

    /// same but from the viewer side, under a declared full screen surface no chrome is visible
    /// counts_covered keeps chrome under another screen
    /// gpui draws only the top screen
    pub fn visible_top_edge_in_window(&self, excluding: &[BottomChromeOwner], _counts_covered: bool) -> Option<f32> {
        if !self.occluders.is_empty() {
            return None;
        }
        self.top_edge_in_window(excluding)
    }

    /// the real system tab bar, desktop has none so the reserve probe always answers
    fn native_tab_bar_top_edge_in_window(&self) -> Option<f32> {
        None
    }

    fn bump(&mut self, cx: &mut Context<Self>) {
        self.revision = self.revision.wrapping_add(1);
        cx.notify();
    }
}

/// reads real geometry off the probes
pub struct BottomChromeGeometry;

impl BottomChromeGeometry {
    /// top edge in window coordinates, none if the probe isnt visible (not laid out or zero size)
    pub fn top_edge_in_window(probe: &BottomChromeProbeView) -> Option<f32> {
        let frame = probe.frame.get()?;
        if !Self::is_effectively_visible(&frame) {
            return None;
        }
        Some(f32::from(frame.top()))
    }

    /// gpui lays out nothing for hidden subtrees so a drawn probe counts as visible unless it collapsed
    pub fn is_effectively_visible(frame: &Bounds<Pixels>) -> bool {
        f32::from(frame.size.width) > 0.5 && f32::from(frame.size.height) > 0.5
    }
}

pub struct BottomChromePolicy;

impl BottomChromePolicy {
    pub const NOTIFIER_GAP: f32 = 10.0;

    pub const NOTIFIER_EXCLUSION: &'static [BottomChromeOwner] = &[];

    pub const FLOATING_ACTION_GAP: f32 = 16.0;

    pub const MAX_CLEARANCE_FRACTION: f32 = 0.42;

    /// motion of content when the occupied bottom changes
    /// clearance changes in steps so without animation the notification would teleport
    pub fn clearance_motion() -> MotionSpec {
        spring(0.34, 0.9)
    }

    pub fn max_clearance(host_height: f32) -> f32 {
        (host_height * Self::MAX_CLEARANCE_FRACTION).max(0.0)
    }

    /// the real bar replaces its own estimate, while visible the reserve under it isnt a candidate
    /// excluding MainTabBar excludes the bar too, same strip
    pub fn top_edge_in_window(
        candidates: &[BottomChromeCandidate],
        native_tab_bar_top: Option<f32>,
        excluding: &[BottomChromeOwner],
    ) -> Option<f32> {
        let mut top = candidates
            .iter()
            .filter(|candidate| !excluding.contains(&candidate.owner))
            .filter(|candidate| native_tab_bar_top.is_none() || candidate.owner != BottomChromeOwner::MainTabBar)
            .map(|candidate| candidate.top_edge_in_window)
            .reduce(f32::min);

        if let Some(native_tab_bar_top) = native_tab_bar_top
            && !excluding.contains(&BottomChromeOwner::MainTabBar)
        {
            top = Some(top.unwrap_or(native_tab_bar_top).min(native_tab_bar_top));
        }
        top
    }

    /// how far to lift content above the chrome, zero means no chrome
    /// chrome below the host bottom gives no clearance
    pub fn clearance(host_bottom_in_window: f32, host_height: f32, chrome_top_in_window: Option<f32>, gap: f32) -> f32 {
        let Some(chrome_top_in_window) = chrome_top_in_window else {
            return 0.0;
        };
        let raw = (host_bottom_in_window - chrome_top_in_window + gap).max(0.0);
        raw.min(Self::max_clearance(host_height))
    }

    /// bottom padding of the floating action, the gap is always there and clearance adds on top
    /// no safe area term on purpose
    pub fn floating_bottom_padding(
        host_bottom_in_window: f32,
        host_height: f32,
        chrome_top_in_window: Option<f32>,
        gap: f32,
    ) -> f32 {
        Self::clearance(host_bottom_in_window, host_height, chrome_top_in_window, 0.0) + gap
    }
}

/// container bottom inset, the device inset and not the keyboard
/// desktop has no bottom safe area or keyboard so it answers zero
pub struct BottomChromeSafeArea;

impl BottomChromeSafeArea {
    const DEVICE_INSET_CEILING: f32 = 60.0;

    pub fn container_bottom_inset(from_layout: f32) -> f32 {
        let layout_inset = from_layout.max(0.0);
        if layout_inset <= Self::DEVICE_INSET_CEILING {
            return layout_inset;
        }
        layout_inset.min(Self::window_bottom_inset())
    }

    fn window_bottom_inset() -> f32 {
        0.0
    }
}

/// element state of a drawn probe, dropping it means the probe left the window
struct BottomChromeProbeSlot {
    probe: Rc<BottomChromeProbeView>,
    cx: AsyncWindowContext,
}

impl Drop for BottomChromeProbeSlot {
    fn drop(&mut self) {
        let probe = Rc::downgrade(&self.probe);
        let mut cx = self.cx.clone();
        self.cx
            .foreground_executor()
            .spawn(async move {
                let _ = cx.update(move |_, cx| {
                    BottomChromeInsets::shared(cx).update(cx, |insets, cx| insets.probe_did_leave_window(&probe, cx));
                });
            })
            .detach();
    }
}

/// probe plus a geometry changed signal for chrome that reports it
/// value comes from the probe, the signal only wakes readers when chrome moves
pub struct BottomChromeReporter {
    owner: BottomChromeOwner,
    reports_layout_changes: bool,
    is_active: bool,
    child: AnyElement,
}

impl IntoElement for BottomChromeReporter {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for BottomChromeReporter {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::from(("bottom_chrome_probe", self.owner as usize)))
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if let Some(global_id) = id.filter(|_| self.is_active) {
            let owner = self.owner;
            let reports_layout_changes = self.reports_layout_changes;
            window.with_element_state::<BottomChromeProbeSlot, _>(global_id, |slot, window| {
                let visible = bounds.intersect(&window.content_mask().bounds);
                let slot = match slot {
                    Some(slot) => {
                        let moved =
                            slot.probe.frame.get() != Some(bounds) || slot.probe.visible_frame.get() != Some(visible);
                        slot.probe.frame.set(Some(bounds));
                        slot.probe.visible_frame.set(Some(visible));
                        if moved && reports_layout_changes {
                            window.defer(cx, |_, cx| {
                                BottomChromeInsets::shared(cx)
                                    .update(cx, |insets, cx| insets.chrome_layout_did_change(cx));
                            });
                        }
                        slot
                    }
                    None => {
                        let probe = Rc::new(BottomChromeProbeView::new(owner));
                        probe.frame.set(Some(bounds));
                        probe.visible_frame.set(Some(visible));
                        let entered = Rc::downgrade(&probe);
                        window.defer(cx, move |_, cx| {
                            if let Some(probe) = entered.upgrade() {
                                BottomChromeInsets::shared(cx)
                                    .update(cx, |insets, cx| insets.probe_did_enter_window(&probe, cx));
                            }
                        });
                        BottomChromeProbeSlot {
                            probe,
                            cx: window.to_async(cx),
                        }
                    }
                };
                ((), slot)
            });
        }
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

/// element state of a drawn occluder, dropping it means the surface left the window
struct BottomChromeOccluderSlot {
    id: Uuid,
    cx: AsyncWindowContext,
}

impl Drop for BottomChromeOccluderSlot {
    fn drop(&mut self) {
        let id = self.id;
        let mut cx = self.cx.clone();
        self.cx
            .foreground_executor()
            .spawn(async move {
                let _ = cx.update(move |_, cx| {
                    BottomChromeInsets::shared(cx).update(cx, |insets, cx| insets.set_occluder(id, false, cx));
                });
            })
            .detach();
    }
}

/// registers a surface that covers the whole window
pub struct BottomChromeOccluder {
    child: AnyElement,
}

impl IntoElement for BottomChromeOccluder {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for BottomChromeOccluder {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::from("bottom_chrome_occluder"))
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if let Some(global_id) = id {
            window.with_element_state::<BottomChromeOccluderSlot, _>(global_id, |slot, window| {
                let slot = match slot {
                    Some(slot) => slot,
                    None => {
                        let id = Uuid::new_v4();
                        window.defer(cx, move |_, cx| {
                            BottomChromeInsets::shared(cx).update(cx, |insets, cx| insets.set_occluder(id, true, cx));
                        });
                        BottomChromeOccluderSlot {
                            id,
                            cx: window.to_async(cx),
                        }
                    }
                };
                ((), slot)
            });
        }
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

pub trait BottomChromeInsetsModifiers: IntoElement + Sized {
    fn reports_bottom_chrome(self, owner: BottomChromeOwner) -> BottomChromeReporter {
        self.reports_bottom_chrome_active(owner, true)
    }

    /// drops the probe but keeps the subtree when is_active is false
    fn reports_bottom_chrome_active(self, owner: BottomChromeOwner, is_active: bool) -> BottomChromeReporter {
        BottomChromeReporter {
            owner,
            reports_layout_changes: true,
            is_active,
            child: self.into_any_element(),
        }
    }

    /// probe without the geometry alarm, for chrome that moves every frame of another animation
    /// readers still read the live position on render
    fn marks_bottom_chrome(self, owner: BottomChromeOwner) -> BottomChromeReporter {
        BottomChromeReporter {
            owner,
            reports_layout_changes: false,
            is_active: true,
            child: self.into_any_element(),
        }
    }

    /// while this subtree is drawn the chrome under it is invisible to readers
    /// for surfaces over the page like a modal, a pushed screen does it itself
    /// attach to content that exists only while covering
    fn occludes_bottom_chrome(self) -> BottomChromeOccluder {
        BottomChromeOccluder {
            child: self.into_any_element(),
        }
    }
}

impl<E: IntoElement> BottomChromeInsetsModifiers for E {}

/// state of the floating action, padding animation included
struct BottomChromeFloatingActionState {
    host: Option<Bounds<Pixels>>,
    chrome_top: Option<f32>,
    bottom_padding: Option<Animated<f32>>,
    _insets_observer: Subscription,
}

impl BottomChromeFloatingActionState {
    fn chrome_top_under(insets: &Entity<BottomChromeInsets>, host: Bounds<Pixels>, cx: &App) -> Option<f32> {
        insets
            .read(cx)
            .top_edge_in_window_under(&[BottomChromeOwner::FloatingAction], host)
    }

    /// what render reads off the host, bottom edge and height but not the horizontal place
    fn host_inputs(host: Option<Bounds<Pixels>>) -> Option<(Pixels, Pixels)> {
        host.map(|host| (host.bottom(), host.size.height))
    }
}

/// floating action in the bottom trailing corner, lifted above whatever sits at the window bottom
/// no background and no handlers on purpose, any hit-test area would be a dead zone for the scroll
#[derive(IntoElement)]
pub struct BottomChromeFloatingAction {
    id: ElementId,
    gap: f32,
    trailing_padding: f32,
    reports_chrome: bool,
    content: AnyElement,
}

impl BottomChromeFloatingAction {
    pub fn new(id: impl Into<ElementId>, content: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            gap: BottomChromePolicy::FLOATING_ACTION_GAP,
            trailing_padding: MainTabChrome::FLOATING_BAR_HORIZONTAL_INSET,
            reports_chrome: true,
            content: content.into_any_element(),
        }
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn trailing_padding(mut self, trailing_padding: f32) -> Self {
        self.trailing_padding = trailing_padding;
        self
    }

    pub fn reports_chrome(mut self, reports_chrome: bool) -> Self {
        self.reports_chrome = reports_chrome;
        self
    }
}

impl RenderOnce for BottomChromeFloatingAction {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| {
            let insets = BottomChromeInsets::shared(cx);
            let insets_observer = cx.observe(&insets, |state: &mut BottomChromeFloatingActionState, insets, cx| {
                let Some(host) = state.host else {
                    return;
                };
                let chrome_top = BottomChromeFloatingActionState::chrome_top_under(&insets, host, cx);
                if state.chrome_top != chrome_top {
                    state.chrome_top = chrome_top;
                    cx.notify();
                }
            });
            BottomChromeFloatingActionState {
                host: None,
                chrome_top: None,
                bottom_padding: None,
                _insets_observer: insets_observer,
            }
        });
        let gap = self.gap;
        let (host, chrome_top) = {
            let state = state.read(cx);
            (state.host, state.chrome_top)
        };
        let target = match host {
            Some(host) => {
                BottomChromePolicy::floating_bottom_padding(
                    f32::from(host.bottom()),
                    f32::from(host.size.height),
                    chrome_top,
                    gap,
                )
            }
            None => gap,
        };

        let now = RiseClock::monotonic();
        let reduce = reduce_motion(cx);
        let bottom_padding = state.update(cx, |state, _| {
            let animated = state.bottom_padding.get_or_insert_with(|| Animated::new(target));
            animated.set(target, BottomChromePolicy::clearance_motion(), now, reduce);
            animated.frame_value(now, window)
        });

        let host_state = state.clone();
        div()
            .id(self.id)
            .size_full()
            .relative()
            .child(measure("bottom_chrome_floating_action_host", move |bounds, _, cx| {
                let insets = BottomChromeInsets::shared(cx);
                let chrome_top = BottomChromeFloatingActionState::chrome_top_under(&insets, bounds, cx);
                host_state.update(cx, |state, cx| {
                    let wakes = BottomChromeFloatingActionState::host_inputs(state.host)
                        != BottomChromeFloatingActionState::host_inputs(Some(bounds))
                        || state.chrome_top != chrome_top;
                    state.host = Some(bounds);
                    state.chrome_top = chrome_top;
                    if wakes {
                        cx.notify();
                    }
                });
            }))
            .child(
                div()
                    .pr(px(self.trailing_padding))
                    .pb(px(bottom_padding))
                    .child(div().block_mouse_except_scroll().child(self.content))
                    .reports_bottom_chrome_active(BottomChromeOwner::FloatingAction, self.reports_chrome)
                    .frame_max(Alignment::BottomTrailing),
            )
    }
}
