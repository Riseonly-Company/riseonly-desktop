use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{
    AnyElement, AnyWindowHandle, App, Bounds, Context, ElementId, Entity, EntityId, InteractiveElement, IntoElement,
    KeystrokeEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, RenderOnce, Styled,
    Subscription, Task, Window, canvas, div,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_platform::context_click::is_context_click_here;
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle, SelectionFeedbackGenerator};
use rise_ui::hold_context_menu_ui::hold_context_menu_frame_keys::{HoldContextMenuHitTargets, HoldContextMenuHitTargetsKey};
use rise_ui::swiftui::environment::{EnvKey, env, env_scope};
use rise_ui::swiftui::modifiers::{ModifiedContent, ModifierExt, ViewModifier};
use rise_ui::swiftui::motion::{MotionSpec, ease_out, spring};

use crate::core::guides::guide_facts::GuideFacts;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

const ESCALATION_DELAY: Duration = Duration::from_millis(950);
const PHASE_SPRING_RESPONSE: f64 = 0.34;
const PHASE_SPRING_DAMPING_FRACTION: f64 = 0.82;
const DISMISS_DURATION: f64 = 0.18;
const MINIMUM_PRESS_DURATION: Duration = Duration::from_millis(220);
const ALLOWABLE_MOVEMENT: f32 = 10.0;
const TAP_SLOP: f32 = 8.0;
const ESCAPE_KEY: &str = "escape";
const PREVIEW_REST_SLOP: f32 = 12.0;
const MENU_ARM_SLOP: f32 = 10.0;

#[derive(Clone, Debug, PartialEq)]
pub enum StickerPreviewAsset {
    Sticker(StickerItem),
    UnicodeEmoji(String),
    Gif(String, f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct StickerHoldEntry {
    pub key: String,
    pub frame: Bounds<Pixels>,
    pub asset: StickerPreviewAsset,
    pub main_emoji: Option<String>,
}

/// one registered cell, view is weak
struct Record {
    view: Weak<StickerHoldCellMarkerView>,
    key: String,
    asset: StickerPreviewAsset,
    main_emoji: Option<String>,
}

type Records = RefCell<HashMap<u64, Record>>;

/// hold preview hit testing resolved from live cell markers, not frames pushed into state
/// publishing every cell frame per scroll frame re-entered layout
#[derive(Clone)]
pub struct StickerHoldFrameRegistry {
    records: Rc<Records>,
}

impl Default for StickerHoldFrameRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl StickerHoldFrameRegistry {
    pub fn new() -> Self {
        Self { records: Rc::new(RefCell::new(HashMap::new())) }
    }

    fn ptr_eq(&self, other: &Weak<Records>) -> bool {
        std::ptr::eq(Rc::as_ptr(&self.records), other.as_ptr())
    }

    fn register(
        &self,
        view: &Rc<StickerHoldCellMarkerView>,
        key: String,
        asset: StickerPreviewAsset,
        main_emoji: Option<String>,
    ) {
        self.records.borrow_mut().insert(
            view.serial,
            Record { view: Rc::downgrade(view), key, asset, main_emoji },
        );
    }

    fn unregister(records: &Records, serial: u64) {
        if let Ok(mut records) = records.try_borrow_mut() {
            records.remove(&serial);
        }
    }

    pub fn entry(&self, at_window_point: Point<Pixels>) -> Option<StickerHoldEntry> {
        let mut dead = Vec::new();
        let mut found = None;
        let mut alive = Vec::new();
        {
            let records = self.records.borrow();
            for (serial, record) in records.iter() {
                let Some(view) = record.view.upgrade() else {
                    dead.push(*serial);
                    continue;
                };
                if found.is_none()
                    && let Some(frame) = view.window_frame()
                    && frame.contains(&at_window_point)
                {
                    found = Some(StickerHoldEntry {
                        key: record.key.clone(),
                        frame,
                        asset: record.asset.clone(),
                        main_emoji: record.main_emoji.clone(),
                    });
                }
                alive.push(view);
            }
        }
        drop(alive);
        if !dead.is_empty() {
            let mut records = self.records.borrow_mut();
            for serial in dead {
                records.remove(&serial);
            }
        }
        found
    }

    pub fn contains_cell(&self, at_window_point: Point<Pixels>) -> bool {
        self.entry(at_window_point).is_some()
    }
}

thread_local! {
    static DEFAULT_REGISTRY: StickerHoldFrameRegistry = StickerHoldFrameRegistry::new();
}

/// the grid registry, or a shared one outside any grid
pub struct StickerHoldFrameRegistryKey;

impl EnvKey for StickerHoldFrameRegistryKey {
    type Value = StickerHoldFrameRegistry;

    fn default_value() -> Self::Value {
        DEFAULT_REGISTRY.with(Clone::clone)
    }
}

fn next_marker_serial() -> u64 {
    thread_local! {
        static SERIAL: Cell<u64> = const { Cell::new(0) };
    }
    SERIAL.with(|serial| {
        let next = serial.get() + 1;
        serial.set(next);
        next
    })
}

/// element state behind a cell reader, alive while the cell is drawn
/// gpui drops it after the first frame that doesnt draw the cell
struct StickerHoldCellMarkerView {
    serial: u64,
    registry: RefCell<Option<Weak<Records>>>,
    frame: Cell<Option<Bounds<Pixels>>>,
}

impl StickerHoldCellMarkerView {
    fn new() -> Self {
        Self { serial: next_marker_serial(), registry: RefCell::new(None), frame: Cell::new(None) }
    }

    /// window frame of the marker, none while it isnt in a window
    fn window_frame(&self) -> Option<Bounds<Pixels>> {
        self.frame.get()
    }
}

impl Drop for StickerHoldCellMarkerView {
    fn drop(&mut self) {
        StickerHoldCellFrameReader::dismantle_ui_view(self);
    }
}

/// transparent layer over the cell that registers it and records where it was painted
/// takes no clicks, no hitbox
#[derive(IntoElement)]
struct StickerHoldCellFrameReader {
    registry: StickerHoldFrameRegistry,
    key: String,
    asset: StickerPreviewAsset,
    main_emoji: Option<String>,
}

impl StickerHoldCellFrameReader {
    fn make_ui_view(&self, frame: Bounds<Pixels>) -> Rc<StickerHoldCellMarkerView> {
        let view = Rc::new(StickerHoldCellMarkerView::new());
        self.update_ui_view(&view, frame);
        view
    }

    fn update_ui_view(&self, ui_view: &Rc<StickerHoldCellMarkerView>, frame: Bounds<Pixels>) {
        ui_view.frame.set(Some(frame));
        self.attach(ui_view);
    }

    fn dismantle_ui_view(ui_view: &mut StickerHoldCellMarkerView) {
        if let Some(records) = ui_view.registry.get_mut().take().and_then(|registry| registry.upgrade()) {
            StickerHoldFrameRegistry::unregister(&records, ui_view.serial);
        }
    }

    fn attach(&self, view: &Rc<StickerHoldCellMarkerView>) {
        let previous = view.registry.borrow_mut().take();
        if let Some(previous) = previous
            && !self.registry.ptr_eq(&previous)
            && let Some(records) = previous.upgrade()
        {
            StickerHoldFrameRegistry::unregister(&records, view.serial);
        }
        *view.registry.borrow_mut() = Some(Rc::downgrade(&self.registry.records));
        self.registry.register(view, self.key.clone(), self.asset.clone(), self.main_emoji.clone());
    }
}

impl RenderOnce for StickerHoldCellFrameReader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let id = ElementId::Name(format!("sticker-hold-cell-{}", self.key).into());
        canvas(
            move |bounds, window, _cx| {
                let marker = window.with_global_id(id, |global_id, window| {
                    window.with_element_state::<Rc<StickerHoldCellMarkerView>, _>(global_id, |marker, _| {
                        let marker = match marker {
                            Some(marker) => {
                                self.update_ui_view(&marker, bounds);
                                marker
                            }
                            None => self.make_ui_view(bounds),
                        };
                        (marker.clone(), marker)
                    })
                });
                window.on_replayed_move(move |offset, _, _, _| marker.frame.set(Some(bounds + offset)));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }
}

pub struct StickerHoldCellFrameModifier {
    key: String,
    asset: StickerPreviewAsset,
    main_emoji: Option<String>,
}

impl ViewModifier for StickerHoldCellFrameModifier {
    fn body(self, content: AnyElement, _window: &mut Window, cx: &mut App) -> AnyElement {
        let reader = StickerHoldCellFrameReader {
            registry: env::<StickerHoldFrameRegistryKey>(cx),
            key: self.key,
            asset: self.asset,
            main_emoji: self.main_emoji,
        };
        div().relative().child(reader).child(content).into_any_element()
    }
}

/// handler of a preview action and of the primary action
pub type StickerPreviewHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone)]
pub struct StickerPreviewAction {
    pub id: String,
    pub title: String,
    pub system_image: String,
    pub destructive: bool,
    pub guide_anchor: Option<RiseGuideAnchor>,
    pub handler: StickerPreviewHandler,
}

impl StickerPreviewAction {
    /// plain constructor, destructive and guide anchor default off and the builders set them
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        system_image: impl Into<String>,
        handler: StickerPreviewHandler,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            system_image: system_image.into(),
            destructive: false,
            guide_anchor: None,
            handler,
        }
    }

    pub fn destructive(mut self, destructive: bool) -> Self {
        self.destructive = destructive;
        self
    }

    pub fn guide_anchor(mut self, guide_anchor: impl Into<Option<RiseGuideAnchor>>) -> Self {
        self.guide_anchor = guide_anchor.into();
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StickerHoldPreviewPresenterPhase {
    Preview,
    Menu,
}

/// shared observable presenter, readers hold the entity and re-render on notify
/// commands are functions on app so the primary action runs outside the entity lock
pub struct StickerHoldPreviewPresenter {
    asset: Option<StickerPreviewAsset>,
    main_emoji: Option<String>,
    phase: StickerHoldPreviewPresenterPhase,
    current_key: Option<String>,
    uses_flat_chrome: bool,
    pub primary_action: Option<StickerPreviewHandler>,
    pub menu_actions: Vec<StickerPreviewAction>,
    highlighted_menu_id: Option<String>,
    pub menu_hit_targets: HoldContextMenuHitTargets,
    overlay_hosts: Vec<EntityId>,
    rest_anchor: Option<Point<Pixels>>,
    menu_touch_origin: Option<Point<Pixels>>,
    menu_touch_armed: bool,
    phase_animation: Option<MotionSpec>,
    asset_animation: Option<MotionSpec>,
    escalate_task: Option<Task<()>>,
    presenting_window: Option<AnyWindowHandle>,
    pinned: bool,
    menu_anchor: Option<Point<Pixels>>,
    _escape_interceptor: Subscription,
}

impl StickerHoldPreviewPresenter {
    fn new(cx: &mut Context<Self>) -> Self {
        let presenter = cx.weak_entity();
        let escape_interceptor = cx.intercept_keystrokes(move |event: &KeystrokeEvent, window: &mut Window, cx| {
            if event.keystroke.key != ESCAPE_KEY {
                return;
            }
            let key_window = window.window_handle();
            let closed = presenter
                .update(cx, |this, cx| {
                    if !this.is_active() || this.presenting_window != Some(key_window) {
                        return false;
                    }
                    this.dismiss_now(cx);
                    true
                })
                .unwrap_or(false);
            if closed {
                cx.stop_propagation();
            }
        });
        Self {
            asset: None,
            main_emoji: None,
            phase: StickerHoldPreviewPresenterPhase::Preview,
            current_key: None,
            uses_flat_chrome: false,
            primary_action: None,
            menu_actions: Vec::new(),
            highlighted_menu_id: None,
            menu_hit_targets: HoldContextMenuHitTargets::new(),
            overlay_hosts: Vec::new(),
            rest_anchor: None,
            menu_touch_origin: None,
            menu_touch_armed: false,
            phase_animation: None,
            asset_animation: None,
            escalate_task: None,
            presenting_window: None,
            pinned: false,
            menu_anchor: None,
            _escape_interceptor: escape_interceptor,
        }
    }

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    pub fn asset(&self) -> Option<&StickerPreviewAsset> {
        self.asset.as_ref()
    }

    pub fn main_emoji(&self) -> Option<&str> {
        self.main_emoji.as_deref()
    }

    pub fn phase(&self) -> StickerHoldPreviewPresenterPhase {
        self.phase
    }

    pub fn current_key(&self) -> Option<&str> {
        self.current_key.as_deref()
    }

    pub fn uses_flat_chrome(&self) -> bool {
        self.uses_flat_chrome
    }

    /// where a context click opened the menu, none for the hold menu under the preview
    pub fn menu_anchor(&self) -> Option<Point<Pixels>> {
        self.menu_anchor
    }

    pub fn is_active(&self) -> bool {
        self.asset.is_some()
    }

    /// preview a context click opened without menu rows, it takes clicks like the menu
    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    pub fn menu_row_contains(&self, at_window_point: Point<Pixels>) -> bool {
        self.menu_row_at(at_window_point).is_some()
    }

    /// id of the open menu row under a window point
    pub fn menu_row_at(&self, at_window_point: Point<Pixels>) -> Option<String> {
        self.menu_actions
            .iter()
            .find(|action| {
                self.menu_hit_targets
                    .frame(&HoldContextMenuHitTargetsKey::Action(action.id.clone()))
                    .is_some_and(|frame| frame.contains(&at_window_point))
            })
            .map(|action| action.id.clone())
    }

    pub fn highlighted_menu_id(&self) -> Option<&str> {
        self.highlighted_menu_id.as_deref()
    }

    pub fn overlay_hosts(&self) -> &[EntityId] {
        &self.overlay_hosts
    }

    pub fn register_overlay_host(id: EntityId, cx: &mut App) {
        Self::shared(cx).update(cx, |this, cx| {
            this.overlay_hosts.retain(|host| *host != id);
            this.overlay_hosts.push(id);
            cx.notify();
        });
    }

    pub fn unregister_overlay_host(id: EntityId, cx: &mut App) {
        Self::shared(cx).update(cx, |this, cx| {
            this.overlay_hosts.retain(|host| *host != id);
            cx.notify();
        });
    }

    pub fn draws_overlay(&self, id: EntityId) -> bool {
        self.overlay_hosts.last() == Some(&id)
    }

    pub fn phase_animation(&self) -> Option<MotionSpec> {
        self.phase_animation
    }

    pub fn asset_animation(&self) -> Option<MotionSpec> {
        self.asset_animation
    }

    /// publishes the canon fact sticker menu open by level
    fn publish_sticker_menu_open(&self, cx: &mut App) {
        let open = self.asset.is_some() && self.phase == StickerHoldPreviewPresenterPhase::Menu;
        GuideFacts::set(RiseGuideSignal::StickerMenuOpen, open, cx);
    }

    fn phase_spring() -> MotionSpec {
        spring(PHASE_SPRING_RESPONSE, PHASE_SPRING_DAMPING_FRACTION)
    }

    fn set_presenting_window(window: AnyWindowHandle, cx: &mut App) {
        Self::shared(cx).update(cx, |this, _| this.presenting_window = Some(window));
    }

    pub fn begin(
        key: String,
        asset: StickerPreviewAsset,
        main_emoji: Option<String>,
        uses_flat_chrome: bool,
        primary: StickerPreviewHandler,
        menu: Vec<StickerPreviewAction>,
        cx: &mut App,
    ) {
        ImpactFeedbackGenerator::new(ImpactStyle::Medium).impact_occurred();
        Self::shared(cx).update(cx, |this, cx| {
            this.current_key = Some(key);
            this.asset = Some(asset);
            this.asset_animation = None;
            this.main_emoji = main_emoji;
            this.uses_flat_chrome = uses_flat_chrome;
            this.primary_action = Some(primary);
            this.menu_actions = menu;
            this.phase = StickerHoldPreviewPresenterPhase::Preview;
            this.phase_animation = None;
            this.rest_anchor = None;
            this.pinned = false;
            this.menu_anchor = None;
            this.publish_sticker_menu_open(cx);
            this.schedule_escalation(cx);
            cx.notify();
        });
    }

    /// pointer slides between cells while the preview shows
    /// once the menu is up the same pointer travels to a row instead
    pub fn r#move(
        key: String,
        asset: StickerPreviewAsset,
        main_emoji: Option<String>,
        uses_flat_chrome: bool,
        primary: StickerPreviewHandler,
        menu: Vec<StickerPreviewAction>,
        cx: &mut App,
    ) {
        Self::shared(cx).update(cx, |this, cx| {
            if !this.is_active()
                || this.phase != StickerHoldPreviewPresenterPhase::Preview
                || this.current_key.as_deref() == Some(key.as_str())
            {
                return;
            }
            SelectionFeedbackGenerator::new().selection_changed();
            this.current_key = Some(key);
            this.asset = Some(asset);
            this.asset_animation = None;
            this.main_emoji = main_emoji;
            this.uses_flat_chrome = uses_flat_chrome;
            this.primary_action = Some(primary);
            this.menu_actions = menu;
            this.publish_sticker_menu_open(cx);
            this.schedule_escalation(cx);
            cx.notify();
        });
    }

    /// without menu rows the menu phase is an empty dark screen that outlives the press
    /// so an item with nothing to offer stays a preview and closes on release
    fn schedule_escalation(&mut self, cx: &mut Context<Self>) {
        self.escalate_task = None;
        if self.menu_actions.is_empty() {
            return;
        }
        self.escalate_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ESCALATION_DELAY).await;
            let _ = this.update(cx, |this, cx| this.escalate(cx));
        }));
    }

    fn escalate(&mut self, cx: &mut Context<Self>) {
        if !self.is_active()
            || self.phase != StickerHoldPreviewPresenterPhase::Preview
            || self.menu_actions.is_empty()
        {
            return;
        }
        ImpactFeedbackGenerator::new(ImpactStyle::Rigid).impact_occurred();
        self.phase = StickerHoldPreviewPresenterPhase::Menu;
        self.phase_animation = Some(Self::phase_spring());
        self.publish_sticker_menu_open(cx);
        cx.notify();
    }

    pub fn present_menu(at_window_point: Point<Pixels>, cx: &mut App) {
        Self::shared(cx).update(cx, |this, cx| {
            this.escalate_task = None;
            if this.is_active() && this.menu_actions.is_empty() {
                this.pinned = true;
                cx.notify();
                return;
            }
            this.menu_anchor = Some(at_window_point);
            this.escalate(cx);
        });
    }

    pub fn note_preview_finger(at_window_point: Point<Pixels>, cx: &mut App) {
        Self::shared(cx).update(cx, |this, cx| {
            if !this.is_active() || this.phase != StickerHoldPreviewPresenterPhase::Preview {
                return;
            }
            let Some(anchor) = this.rest_anchor else {
                this.rest_anchor = Some(at_window_point);
                return;
            };
            if Self::travel(at_window_point, anchor) <= PREVIEW_REST_SLOP {
                return;
            }
            this.rest_anchor = Some(at_window_point);
            this.schedule_escalation(cx);
        });
    }

    fn travel(a: Point<Pixels>, b: Point<Pixels>) -> f32 {
        f32::from(a.x - b.x).hypot(f32::from(a.y - b.y))
    }

    pub fn track_menu(at_window_point: Point<Pixels>, cx: &mut App) {
        Self::shared(cx).update(cx, |this, cx| this.track_menu_at(at_window_point, cx));
    }

    fn track_menu_at(&mut self, point: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.is_active() || self.phase != StickerHoldPreviewPresenterPhase::Menu {
            return;
        }
        let Some(origin) = self.menu_touch_origin else {
            self.menu_touch_origin = Some(point);
            return;
        };
        if !self.menu_touch_armed {
            if Self::travel(point, origin) <= MENU_ARM_SLOP {
                return;
            }
            self.menu_touch_armed = true;
        }
        let id = self
            .menu_actions
            .iter()
            .find(|action| {
                self.menu_hit_targets
                    .frame(&HoldContextMenuHitTargetsKey::Action(action.id.clone()))
                    .is_some_and(|frame| frame.contains(&point))
            })
            .map(|action| action.id.clone());
        if id == self.highlighted_menu_id {
            return;
        }
        if id.is_some() {
            SelectionFeedbackGenerator::new().selection_changed();
        }
        self.highlighted_menu_id = id;
        cx.notify();
    }

    /// a release over a row selects it, elsewhere the menu stays for a click
    pub fn release(at_window_point: Option<Point<Pixels>>, window: &mut Window, cx: &mut App) {
        let highlighted = Self::shared(cx).update(cx, |this, cx| {
            if this.pinned {
                return None;
            }
            if this.phase == StickerHoldPreviewPresenterPhase::Preview {
                this.dismiss_now(cx);
                return None;
            }
            this.escalate_task = None;
            if let Some(point) = at_window_point {
                this.track_menu_at(point, cx);
            }
            this.highlighted_menu_id.clone()
        });
        if let Some(id) = highlighted {
            Self::select_menu(&id, window, cx);
        }
    }

    pub fn select_menu(id: &str, window: &mut Window, cx: &mut App) {
        let handler = Self::shared(cx).update(cx, |this, cx| {
            let handler = this.menu_actions.iter().find(|action| action.id == id).map(|action| action.handler.clone());
            tracing::warn!(target: "riseonly::X1603", id, found = handler.is_some(), actions = this.menu_actions.len(), "hold menu select");
            let handler = handler?;
            this.dismiss_now(cx);
            Some(handler)
        });
        if let Some(handler) = handler {
            handler(window, cx);
        }
    }

    fn dismiss_now(&mut self, cx: &mut Context<Self>) {
        self.escalate_task = None;
        self.presenting_window = None;
        let animation = Some(ease_out(DISMISS_DURATION));
        self.asset = None;
        self.asset_animation = animation;
        self.phase = StickerHoldPreviewPresenterPhase::Preview;
        self.phase_animation = animation;
        self.publish_sticker_menu_open(cx);
        self.current_key = None;
        self.primary_action = None;
        self.menu_actions = Vec::new();
        self.highlighted_menu_id = None;
        self.menu_touch_origin = None;
        self.menu_touch_armed = false;
        self.rest_anchor = None;
        self.pinned = false;
        cx.notify();
    }

    pub fn dismiss(cx: &mut App) {
        Self::shared(cx).update(cx, |this, cx| this.dismiss_now(cx));
    }

    pub fn reset_account_state(cx: &mut App) {
        Self::dismiss(cx);
    }

    pub fn trigger_primary(window: &mut Window, cx: &mut App) {
        let action = Self::shared(cx).update(cx, |this, cx| {
            let action = this.primary_action.clone();
            this.dismiss_now(cx);
            action
        });
        if let Some(action) = action {
            action(window, cx);
        }
    }
}

pub type StickerGridHoldPlatePointAction = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;
pub type StickerGridHoldPlateRelease = Rc<dyn Fn(Option<Point<Pixels>>, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct StickerGridHoldPlate {
    id: ElementId,
    registry: StickerHoldFrameRegistry,
    is_enabled: bool,
    coordinator: Coordinator,
}

#[derive(Clone)]
struct Coordinator {
    on_confirm: StickerGridHoldPlatePointAction,
    on_context_confirm: StickerGridHoldPlatePointAction,
    on_update: StickerGridHoldPlatePointAction,
    on_release: StickerGridHoldPlateRelease,
    on_tap: StickerGridHoldPlatePointAction,
}

impl StickerGridHoldPlate {
    pub fn new(id: impl Into<ElementId>, registry: StickerHoldFrameRegistry) -> Self {
        Self {
            id: id.into(),
            registry,
            is_enabled: true,
            coordinator: Coordinator {
                on_confirm: Rc::new(|_, _, _| {}),
                on_context_confirm: Rc::new(|_, _, _| {}),
                on_update: Rc::new(|_, _, _| {}),
                on_release: Rc::new(|_, _, _| {}),
                on_tap: Rc::new(|_, _, _| {}),
            },
        }
    }

    pub fn is_enabled(mut self, is_enabled: bool) -> Self {
        self.is_enabled = is_enabled;
        self
    }

    pub fn on_confirm(mut self, on_confirm: StickerGridHoldPlatePointAction) -> Self {
        self.coordinator.on_confirm = on_confirm;
        self
    }

    pub fn on_context_confirm(mut self, on_context_confirm: StickerGridHoldPlatePointAction) -> Self {
        self.coordinator.on_context_confirm = on_context_confirm;
        self
    }

    pub fn on_update(mut self, on_update: StickerGridHoldPlatePointAction) -> Self {
        self.coordinator.on_update = on_update;
        self
    }

    pub fn on_release(mut self, on_release: StickerGridHoldPlateRelease) -> Self {
        self.coordinator.on_release = on_release;
        self
    }

    pub fn on_tap(mut self, on_tap: StickerGridHoldPlatePointAction) -> Self {
        self.coordinator.on_tap = on_tap;
        self
    }
}

/// long press recogniser progress for the current press
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LongPressState {
    Possible,
    Began,
    Failed,
}

/// plate with its coordinator confirmed flag
/// keyed element state so a plate leaving the tree mid press drops with its timer
struct Plate {
    coordinator: Coordinator,
    registry: StickerHoldFrameRegistry,
    is_enabled: bool,
    confirmed: bool,
    is_pressed: bool,
    start_point: Point<Pixels>,
    last_point: Point<Pixels>,
    moved_beyond_slop: bool,
    held_this_touch: bool,
    long_press: LongPressState,
    long_press_timer: Option<Task<()>>,
}

type PlateEntity = Entity<Plate>;

impl Plate {
    fn new(coordinator: Coordinator, registry: StickerHoldFrameRegistry, is_enabled: bool) -> Self {
        Self {
            coordinator,
            registry,
            is_enabled,
            confirmed: false,
            is_pressed: false,
            start_point: Point::default(),
            last_point: Point::default(),
            moved_beyond_slop: false,
            held_this_touch: false,
            long_press: LongPressState::Possible,
            long_press_timer: None,
        }
    }

    /// fresh callbacks, registry and enablement every render
    /// turning enabled off disables the recogniser which fails a press in flight
    fn sync(&mut self, coordinator: Coordinator, registry: StickerHoldFrameRegistry, is_enabled: bool) {
        self.coordinator = coordinator;
        self.registry = registry;
        self.is_enabled = is_enabled;
        if !is_enabled {
            self.long_press_timer = None;
            if self.long_press == LongPressState::Possible {
                self.long_press = LongPressState::Failed;
            }
        }
    }

    fn hit_test(&self, window_point: Point<Pixels>) -> bool {
        self.is_enabled && self.registry.contains_cell(window_point)
    }

    fn distance(a: Point<Pixels>, b: Point<Pixels>) -> f32 {
        let dx = f32::from(a.x - b.x);
        let dy = f32::from(a.y - b.y);
        dx.hypot(dy)
    }

    fn touches_began(entity: &PlateEntity, location: Point<Pixels>, context: bool, window: &mut Window, cx: &mut App) {
        let (menu_is_up, on_menu_row) = {
            let presenter = StickerHoldPreviewPresenter::shared(cx);
            let presenter = presenter.read(cx);
            let menu_is_up = presenter.is_active()
                && (presenter.phase() == StickerHoldPreviewPresenterPhase::Menu || presenter.is_pinned());
            (menu_is_up, menu_is_up && presenter.menu_row_contains(location))
        };
        if menu_is_up {
            if !on_menu_row {
                StickerHoldPreviewPresenter::dismiss(cx);
            }
            return;
        }
        let began = entity.update(cx, |plate, cx| {
            if plate.is_pressed || !plate.hit_test(location) {
                return false;
            }
            plate.is_pressed = true;
            plate.moved_beyond_slop = false;
            plate.held_this_touch = false;
            plate.start_point = location;
            plate.last_point = location;
            plate.long_press = LongPressState::Possible;
            cx.notify();
            true
        });
        if !began {
            return;
        }
        if context {
            Self::handle_began(entity, location, true, window, cx);
            return;
        }
        let weak = entity.downgrade();
        let window_handle = window.window_handle();
        let timer = cx.spawn(async move |cx| {
            cx.background_executor().timer(MINIMUM_PRESS_DURATION).await;
            let Some(plate) = weak.upgrade() else { return };
            let _ = window_handle.update(cx, |_, window, cx| {
                let location = plate.read(cx).last_point;
                Self::handle_began(&plate, location, false, window, cx);
            });
        });
        entity.update(cx, |plate, _| plate.long_press_timer = Some(timer));
    }

    fn handle_began(entity: &PlateEntity, location: Point<Pixels>, context: bool, window: &mut Window, cx: &mut App) {
        let coordinator = entity.update(cx, |plate, _| {
            if plate.long_press != LongPressState::Possible || !plate.is_pressed {
                return None;
            }
            plate.long_press = LongPressState::Began;
            plate.confirmed = true;
            plate.held_this_touch = true;
            Some(plate.coordinator.clone())
        });
        let Some(coordinator) = coordinator else { return };
        if context {
            (coordinator.on_context_confirm)(location, window, cx);
        } else {
            (coordinator.on_confirm)(location, window, cx);
        }
    }

    fn touches_moved(entity: &PlateEntity, location: Point<Pixels>, window: &mut Window, cx: &mut App) {
        let on_update = entity.update(cx, |plate, _| {
            if !plate.is_pressed {
                return None;
            }
            plate.last_point = location;
            let distance = Self::distance(location, plate.start_point);
            if distance > TAP_SLOP {
                plate.moved_beyond_slop = true;
            }
            if plate.long_press == LongPressState::Possible && distance > ALLOWABLE_MOVEMENT {
                plate.long_press = LongPressState::Failed;
                plate.long_press_timer = None;
            }
            (plate.long_press == LongPressState::Began && plate.confirmed)
                .then(|| plate.coordinator.on_update.clone())
        });
        if let Some(on_update) = on_update {
            on_update(location, window, cx);
        }
    }

    /// recogniser ended then the plate own touches ended
    fn touches_ended(entity: &PlateEntity, location: Point<Pixels>, window: &mut Window, cx: &mut App) {
        let snapshot = entity.update(cx, |plate, cx| {
            if !plate.is_pressed {
                return None;
            }
            plate.is_pressed = false;
            plate.long_press_timer = None;
            if Self::distance(location, plate.start_point) > TAP_SLOP {
                plate.moved_beyond_slop = true;
            }
            cx.notify();
            Some((plate.long_press == LongPressState::Began && plate.confirmed, plate.coordinator.clone()))
        });
        let Some((held, coordinator)) = snapshot else { return };
        if held {
            (coordinator.on_release)(Some(location), window, cx);
            entity.update(cx, |plate, _| plate.confirmed = false);
        }
        let taps = {
            let plate = entity.read(cx);
            !plate.confirmed && !plate.held_this_touch && !plate.moved_beyond_slop
        };
        if taps {
            (coordinator.on_tap)(location, window, cx);
        }
    }

    fn touches_cancelled(entity: &PlateEntity, window: &mut Window, cx: &mut App) {
        let snapshot = entity.update(cx, |plate, cx| {
            if !plate.is_pressed {
                return None;
            }
            plate.is_pressed = false;
            plate.long_press_timer = None;
            cx.notify();
            Some((plate.long_press == LongPressState::Began && plate.confirmed, plate.coordinator.on_release.clone()))
        });
        let Some((held, on_release)) = snapshot else { return };
        if held {
            on_release(None, window, cx);
            entity.update(cx, |plate, _| plate.confirmed = false);
        }
    }
}

impl RenderOnce for StickerGridHoldPlate {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let coordinator = self.coordinator.clone();
        let registry = self.registry.clone();
        let is_enabled = self.is_enabled;
        let plate = window.use_keyed_state(self.id.clone(), cx, move |_, _| Plate::new(coordinator, registry, is_enabled));
        let is_pressed = plate.update(cx, |plate, _| {
            plate.sync(self.coordinator, self.registry, self.is_enabled);
            plate.is_pressed
        });

        let primary = plate.clone();
        let secondary = plate.clone();
        let mut layer = div()
            .id(self.id)
            .absolute()
            .inset_0()
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                let context = is_context_click_here(event.button, &event.modifiers);
                Plate::touches_began(&primary, event.position, context, window, cx);
            })
            .on_mouse_down(MouseButton::Right, move |event: &MouseDownEvent, window, cx| {
                Plate::touches_began(&secondary, event.position, true, window, cx);
            });
        if is_pressed {
            layer = layer.child(press_follower(plate));
        }
        layer
    }
}

fn press_follower(plate: PlateEntity) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let moved = plate.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != gpui::DispatchPhase::Bubble {
                    return;
                }
                if event.pressed_button.is_none() {
                    Plate::touches_cancelled(&moved, window, cx);
                } else {
                    Plate::touches_moved(&moved, event.position, window, cx);
                }
            });
            let ended = plate.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase == gpui::DispatchPhase::Bubble {
                    Plate::touches_ended(&ended, event.position, window, cx);
                }
            });
        },
    )
    .absolute()
    .size_0()
}

pub type StickerHoldGridPrimary = Rc<dyn Fn(&StickerHoldEntry, &mut Window, &mut App)>;
pub type StickerHoldGridMenuFor = Rc<dyn Fn(&StickerHoldEntry, &App) -> Vec<StickerPreviewAction>>;

/// grid modifier, defaults are no menu rows and no flat chrome
/// enabled by default, id keys the grid registry and its plate
pub struct StickerHoldGridModifier {
    id: ElementId,
    on_primary: StickerHoldGridPrimary,
    menu_for: StickerHoldGridMenuFor,
    uses_flat_chrome: bool,
    is_enabled: bool,
}

impl StickerHoldGridModifier {
    pub fn new(id: impl Into<ElementId>, on_primary: StickerHoldGridPrimary) -> Self {
        Self {
            id: id.into(),
            on_primary,
            menu_for: Rc::new(|_, _| Vec::new()),
            uses_flat_chrome: false,
            is_enabled: true,
        }
    }

    pub fn menu_for(mut self, menu_for: StickerHoldGridMenuFor) -> Self {
        self.menu_for = menu_for;
        self
    }

    pub fn uses_flat_chrome(mut self, uses_flat_chrome: bool) -> Self {
        self.uses_flat_chrome = uses_flat_chrome;
        self
    }

    pub fn is_enabled(mut self, is_enabled: bool) -> Self {
        self.is_enabled = is_enabled;
        self
    }
}

struct StickerHoldGrid {
    registry: StickerHoldFrameRegistry,
    on_primary: StickerHoldGridPrimary,
    menu_for: StickerHoldGridMenuFor,
    uses_flat_chrome: bool,
    is_enabled: bool,
}

impl StickerHoldGrid {
    fn entry(&self, at: Point<Pixels>) -> Option<StickerHoldEntry> {
        self.registry.entry(at)
    }

    fn primary(&self, entry: &StickerHoldEntry) -> StickerPreviewHandler {
        let on_primary = self.on_primary.clone();
        let entry = entry.clone();
        Rc::new(move |window: &mut Window, cx: &mut App| on_primary(&entry, window, cx))
    }

    fn begin(&self, at: Point<Pixels>, window: &Window, cx: &mut App) -> bool {
        if !self.is_enabled {
            return false;
        }
        let Some(entry) = self.entry(at) else {
            tracing::warn!(target: "riseonly::X1620", x = f32::from(at.x), y = f32::from(at.y), "hold begin: no cell under point");
            return false;
        };
        tracing::warn!(
            target: "riseonly::X1620",
            key = %entry.key,
            hosts = ?StickerHoldPreviewPresenter::shared(cx).read(cx).overlay_hosts(),
            window = ?window.window_handle().window_id(),
            "hold begin"
        );
        let menu = (self.menu_for)(&entry, cx);
        StickerHoldPreviewPresenter::set_presenting_window(window.window_handle(), cx);
        StickerHoldPreviewPresenter::begin(
            entry.key.clone(),
            entry.asset.clone(),
            entry.main_emoji.clone(),
            self.uses_flat_chrome,
            self.primary(&entry),
            menu,
            cx,
        );
        true
    }

    fn move_to(&self, at: Point<Pixels>, cx: &mut App) {
        if !self.is_enabled {
            return;
        }
        if StickerHoldPreviewPresenter::shared(cx).read(cx).phase() == StickerHoldPreviewPresenterPhase::Menu {
            StickerHoldPreviewPresenter::track_menu(at, cx);
            return;
        }
        StickerHoldPreviewPresenter::note_preview_finger(at, cx);
        let Some(entry) = self.entry(at) else { return };
        let menu = (self.menu_for)(&entry, cx);
        StickerHoldPreviewPresenter::r#move(
            entry.key.clone(),
            entry.asset.clone(),
            entry.main_emoji.clone(),
            self.uses_flat_chrome,
            self.primary(&entry),
            menu,
            cx,
        );
    }
}

impl ViewModifier for StickerHoldGridModifier {
    fn body(self, content: AnyElement, window: &mut Window, cx: &mut App) -> AnyElement {
        let registry = window
            .use_keyed_state(self.id.clone(), cx, |_, _| StickerHoldFrameRegistry::new())
            .read(cx)
            .clone();
        let grid = Rc::new(StickerHoldGrid {
            registry: registry.clone(),
            on_primary: self.on_primary,
            menu_for: self.menu_for,
            uses_flat_chrome: self.uses_flat_chrome,
            is_enabled: self.is_enabled,
        });
        let content = env_scope::<StickerHoldFrameRegistryKey>(registry.clone()).child(content);
        let mut wrapper = div().relative().child(content);
        if self.is_enabled {
            let on_confirm = grid.clone();
            let on_context_confirm = grid.clone();
            let on_update = grid.clone();
            let on_tap = grid;
            let plate = StickerGridHoldPlate::new((self.id, "plate"), registry)
                .is_enabled(self.is_enabled)
                .on_confirm(Rc::new(move |point: Point<Pixels>, window: &mut Window, cx: &mut App| {
                    on_confirm.begin(point, window, cx);
                }))
                .on_context_confirm(Rc::new(move |point: Point<Pixels>, window: &mut Window, cx: &mut App| {
                    if on_context_confirm.begin(point, window, cx) {
                        StickerHoldPreviewPresenter::present_menu(point, cx);
                    }
                }))
                .on_update(Rc::new(move |point: Point<Pixels>, _: &mut Window, cx: &mut App| {
                    on_update.move_to(point, cx)
                }))
                .on_release(Rc::new(|point: Option<Point<Pixels>>, window: &mut Window, cx: &mut App| {
                    StickerHoldPreviewPresenter::release(point, window, cx)
                }))
                .on_tap(Rc::new(move |point: Point<Pixels>, window: &mut Window, cx: &mut App| {
                    if on_tap.is_enabled
                        && let Some(entry) = on_tap.entry(point)
                    {
                        (on_tap.on_primary)(&entry, window, cx);
                    }
                }));
            wrapper = wrapper.child(plate);
        }
        wrapper.into_any_element()
    }
}

pub trait StickerHoldPreviewModifiers: IntoElement + Sized {
    /// marks a cell frame with key, asset and main emoji
    fn sticker_hold_cell_frame(
        self,
        key: impl Into<String>,
        asset: StickerPreviewAsset,
        main_emoji: Option<String>,
    ) -> ModifiedContent<StickerHoldCellFrameModifier> {
        self.modifier(StickerHoldCellFrameModifier { key: key.into(), asset, main_emoji })
    }

    /// makes a grid hold aware, defaults and the grid id are on the modifier
    fn sticker_hold_grid(self, modifier: StickerHoldGridModifier) -> ModifiedContent<StickerHoldGridModifier> {
        self.modifier(modifier)
    }
}

impl<T: IntoElement> StickerHoldPreviewModifiers for T {}

#[cfg(test)]
#[path = "sticker_hold_preview_tests.rs"]
mod tests;
