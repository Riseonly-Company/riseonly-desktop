use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{AnyView, App, Context, Global, Hsla, Pixels, Point, Size, Window};
use rise_core::foundation::foundation_date::TimeInterval;
use rise_theme::Appearance;
use rise_theme::ui_color_system::UIColor;
use rise_ui::swiftui::motion::{CubicBezier, MotionSpec};

use crate::core::ui::list_engine::rise_manual_list_integration::RiseManualListInstallCost;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::RiseManualListView;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;

/// viewport resize animation, options carry the timing the viewport change rides
/// duration stays the authoritative length
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiseManualListViewportResizeAnimation {
    pub duration: TimeInterval,
    pub options: MotionSpec,
    pub received_at: f64,
}

impl RiseManualListViewportResizeAnimation {
    pub fn new(duration: TimeInterval, options: MotionSpec, received_at: f64) -> Self {
        Self {
            duration,
            options,
            received_at,
        }
    }

    pub fn remaining_duration(&self, timestamp: f64) -> Option<TimeInterval> {
        if !(self.duration.is_finite()
            && self.received_at.is_finite()
            && timestamp.is_finite()
            && self.duration > 0.0)
        {
            return None;
        }
        let remaining = self.duration - (timestamp - self.received_at).max(0.0);
        if remaining > 1.0 / 240.0 {
            Some(remaining)
        } else {
            None
        }
    }
}

/// programmatic list travel, 0.3 s on the slide curve (0.33 0.52 0.25 0.99)
/// viewport span scrolls for real, farther is two pages edge to edge
pub enum RiseManualListScrollAnimationPolicy {}

impl RiseManualListScrollAnimationPolicy {
    pub const DURATION: TimeInterval = 0.3;

    const CURVE: RiseManualListCubicBezier = RiseManualListCubicBezier::new(0.33, 0.52, 0.25, 0.99);

    /// same control points as a kit curve, for animated motion
    pub fn timing_function() -> CubicBezier {
        CubicBezier(0.33, 0.52, 0.25, 0.99)
    }

    /// past one viewport the rows on screen and at the destination no longer overlap
    /// so the contiguous slide looks the same without mounting every row between
    pub fn continuous_distance_limit(viewport_height: f32) -> f32 {
        viewport_height.max(360.0)
    }

    /// eased progress for a linear time fraction, same curve as timing_function
    pub fn progress(fraction: f64) -> f64 {
        Self::CURVE.value(fraction)
    }
}

/// unit cubic bezier easing from (0, 0) to (1 1) solved for the time axis
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiseManualListCubicBezier {
    ax: f64,
    bx: f64,
    cx: f64,
    ay: f64,
    by: f64,
    cy: f64,
}

impl RiseManualListCubicBezier {
    pub const fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> Self {
        let cx = 3.0 * x1;
        let bx = 3.0 * (x2 - x1) - cx;
        let ax = 1.0 - cx - bx;
        let cy = 3.0 * y1;
        let by = 3.0 * (y2 - y1) - cy;
        let ay = 1.0 - cy - by;
        Self { ax, bx, cx, ay, by, cy }
    }

    pub fn value(&self, fraction: f64) -> f64 {
        if !fraction.is_finite() {
            return 1.0;
        }
        let x = fraction.clamp(0.0, 1.0);
        if x <= 0.0 || x >= 1.0 {
            return x;
        }
        self.sample_y(self.solve_t(x))
    }

    fn sample_x(&self, t: f64) -> f64 {
        ((self.ax * t + self.bx) * t + self.cx) * t
    }

    fn sample_y(&self, t: f64) -> f64 {
        ((self.ay * t + self.by) * t + self.cy) * t
    }

    fn sample_derivative_x(&self, t: f64) -> f64 {
        (3.0 * self.ax * t + 2.0 * self.bx) * t + self.cx
    }

    fn solve_t(&self, x: f64) -> f64 {
        let epsilon = 1e-7;
        let mut t = x;
        for _ in 0..8 {
            let error = self.sample_x(t) - x;
            if error.abs() < epsilon {
                return t;
            }
            let derivative = self.sample_derivative_x(t);
            if derivative.abs() <= 1e-6 {
                break;
            }
            t -= error / derivative;
        }
        let mut lower_bound = 0.0;
        let mut upper_bound = 1.0;
        t = x;
        for _ in 0..32 {
            let value = self.sample_x(t);
            if (value - x).abs() < epsilon {
                return t;
            }
            if value < x {
                lower_bound = t;
            } else {
                upper_bound = t;
            }
            t = (lower_bound + upper_bound) * 0.5;
        }
        t
    }
}

pub enum RiseManualListViewportAnchorAnimationPolicy {}

impl RiseManualListViewportAnchorAnimationPolicy {
    pub fn inverse_translation_y(
        previous_origin_y: f32,
        current_origin_y: f32,
        current_presentation_translation_y: f32,
    ) -> f32 {
        previous_origin_y + current_presentation_translation_y - current_origin_y
    }
}

pub enum RiseManualListKeyboardPresentationPolicy {}

impl RiseManualListKeyboardPresentationPolicy {
    pub fn content_translation_y(
        committed_bottom_inset: Pixels,
        presented_bottom_inset: Pixels,
        keeps_bottom_anchor: bool,
    ) -> Pixels {
        if !keeps_bottom_anchor {
            return Pixels::ZERO;
        }
        committed_bottom_inset - presented_bottom_inset
    }

    pub fn sticky_translation_y(
        committed_bottom_inset: Option<Pixels>,
        presented_bottom_inset: Option<Pixels>,
    ) -> Pixels {
        let (Some(committed_bottom_inset), Some(presented_bottom_inset)) =
            (committed_bottom_inset, presented_bottom_inset)
        else {
            return Pixels::ZERO;
        };
        committed_bottom_inset - presented_bottom_inset
    }
}

pub type RiseManualListBottomStickyHeaderViewFactory =
    Rc<dyn Fn(Option<AnyView>, &mut Window, &mut App) -> AnyView>;

#[derive(Clone)]
pub struct RiseManualListBottomStickyHeader {
    pub id: String,
    pub size: Size<Pixels>,
    pub leading_inset: f32,
    pub top_inset: f32,
    pub bottom_inset: f32,
    pub reuse_identifier: String,
    pub content_version: i64,
    pub view: RiseManualListBottomStickyHeaderViewFactory,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RiseManualListTopStickyHeader;

pub trait RiseManualListBottomStickyHeaderInteractive: Sized + 'static {
    fn activate_bottom_sticky_header(&mut self, window: &mut Window, cx: &mut Context<Self>);
}

/// top sticky row that answers taps
/// is_pinned matters, the pinned chip and the row at its own place are the same view lifted
/// into the sticky host
pub trait RiseManualListTopStickyHeaderInteractive: Sized + 'static {
    fn top_sticky_header_contains(&self, point: Point<Pixels>) -> bool;
    fn activate_top_sticky_header(&mut self, is_pinned: bool, window: &mut Window, cx: &mut Context<Self>);
}

/// top sticky row whose chip hides while it floats away from its place and the list is at rest
/// like the floating date in telegram, so it doesnt sit over rows
pub trait RiseManualListTopStickyHeaderFading: Sized + 'static {
    fn set_top_sticky_header_faded(&mut self, faded: bool, animated: bool, cx: &mut Context<Self>);
    fn is_top_sticky_header_on_screen(&self) -> bool;
}

pub enum RiseManualListTopStickyHeaderLayout {}

impl RiseManualListTopStickyHeaderLayout {
    pub fn origin_y(group_top: f32, group_bottom: f32, viewport_top: f32, header_height: f32) -> f32 {
        group_top.max(viewport_top).min(group_bottom - header_height)
    }
}

pub enum RiseManualListBottomStickyHeaderLayout {}

impl RiseManualListBottomStickyHeaderLayout {
    pub fn origin_y(group_top: f32, group_bottom: f32, viewport_bottom: f32, header_height: f32) -> f32 {
        group_top.max(group_bottom.min(viewport_bottom) - header_height)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseManualListKeyboardTimeline {
    pub model_y: f32,
    pub segments: Vec<RiseManualListKeyboardTimelineSegment>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RiseManualListKeyboardTimelineCurve {
    Linear,
    Cubic {
        c1x: f64,
        c1y: f64,
        c2x: f64,
        c2y: f64,
    },
    Spring {
        mass: f64,
        stiffness: f64,
        damping: f64,
        initial_velocity: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiseManualListKeyboardTimelineSegment {
    pub begin_time: Option<f64>,
    pub curve: RiseManualListKeyboardTimelineCurve,
    pub local_duration: f64,
    pub speed: f64,
    pub time_offset: f64,
    pub from_y: f32,
    pub to_y: f32,
    pub is_additive: bool,
    pub holds_final_value: bool,
}

impl RiseManualListKeyboardTimeline {
    pub fn new(model_y: f32, segments: Vec<RiseManualListKeyboardTimelineSegment>) -> Self {
        Self { model_y, segments }
    }

    pub fn has_pending_segments(&self) -> bool {
        self.segments.iter().any(|segment| segment.begin_time.is_none())
    }

    pub fn settle_offset(&self, now: f64) -> f64 {
        self.segments.iter().fold(0.0, |longest: f64, segment| {
            let start = segment.begin_time.map_or(0.0, |begin_time| begin_time - now);
            let remaining = (segment.local_duration - segment.time_offset) / segment.speed;
            longest.max(start + remaining)
        })
    }

    /// translation shown offset seconds after the next commit, motions already begun are placed against now
    pub fn translation_y(&self, offset: f64, now: f64) -> f32 {
        let mut value = self.model_y;
        for segment in &self.segments {
            let elapsed = segment
                .begin_time
                .map_or(offset, |begin_time| now + offset - begin_time);
            let local = elapsed.max(0.0) * segment.speed + segment.time_offset;
            let progress = if local >= segment.local_duration {
                if !segment.holds_final_value {
                    continue;
                }
                1.0
            } else {
                Self::progress(segment.curve, local, segment.local_duration)
            };
            let interpolated = segment.from_y + (segment.to_y - segment.from_y) * progress;
            value = if segment.is_additive {
                value + interpolated
            } else {
                interpolated
            };
        }
        value
    }

    pub fn progress(curve: RiseManualListKeyboardTimelineCurve, local: f64, duration: f64) -> f32 {
        if local >= duration {
            return 1.0;
        }
        match curve {
            RiseManualListKeyboardTimelineCurve::Linear => (local.max(0.0) / duration) as f32,
            RiseManualListKeyboardTimelineCurve::Cubic { c1x, c1y, c2x, c2y } => {
                Self::cubic_progress(local / duration, c1x, c1y, c2x, c2y) as f32
            }
            RiseManualListKeyboardTimelineCurve::Spring {
                mass,
                stiffness,
                damping,
                initial_velocity,
            } => Self::spring_progress(local, mass, stiffness, damping, initial_velocity) as f32,
        }
    }

    pub fn spring_progress(time: f64, mass: f64, stiffness: f64, damping: f64, initial_velocity: f64) -> f64 {
        if time <= 0.0 {
            return 0.0;
        }
        let omega = (stiffness / mass).sqrt();
        let zeta = damping / (2.0 * (stiffness * mass).sqrt());
        let displacement = if (zeta - 1.0).abs() < 0.001 {
            (1.0 + (omega - initial_velocity) * time) * (-omega * time).exp()
        } else if zeta < 1.0 {
            let damped_omega = omega * (1.0 - zeta * zeta).sqrt();
            (-zeta * omega * time).exp()
                * ((damped_omega * time).cos()
                    + (zeta * omega - initial_velocity) / damped_omega * (damped_omega * time).sin())
        } else {
            let spread = omega * (zeta * zeta - 1.0).sqrt();
            let slow_root = -zeta * omega + spread;
            let fast_root = -zeta * omega - spread;
            let fast_weight = (-initial_velocity - slow_root) / (fast_root - slow_root);
            (1.0 - fast_weight) * (slow_root * time).exp() + fast_weight * (fast_root * time).exp()
        };
        1.0 - displacement
    }

    pub fn cubic_progress(x: f64, c1x: f64, c1y: f64, c2x: f64, c2y: f64) -> f64 {
        let target = x.clamp(0.0, 1.0);
        let curve_x = |u: f64| {
            let v = 1.0 - u;
            3.0 * v * v * u * c1x + 3.0 * v * u * u * c2x + u * u * u
        };
        let curve_y = |u: f64| {
            let v = 1.0 - u;
            3.0 * v * v * u * c1y + 3.0 * v * u * u * c2y + u * u * u
        };
        let mut u = target;
        for _ in 0..8 {
            let error = curve_x(u) - target;
            if error.abs() < 1e-7 {
                return curve_y(u);
            }
            let v = 1.0 - u;
            let slope = 3.0 * v * v * c1x + 6.0 * v * u * (c2x - c1x) + 3.0 * u * u * (1.0 - c2x);
            if slope.abs() <= 1e-6 {
                break;
            }
            u = (u - error / slope).clamp(0.0, 1.0);
        }
        let mut lower = 0.0;
        let mut upper = 1.0;
        u = target;
        for _ in 0..48 {
            let value = curve_x(u);
            if (value - target).abs() < 1e-7 {
                break;
            }
            if value < target {
                lower = u;
            } else {
                upper = u;
            }
            u = (lower + upper) / 2.0;
        }
        curve_y(u)
    }

    pub fn is_linear(timing_function: Option<CubicBezier>) -> bool {
        let Some(CubicBezier(x1, y1, x2, y2)) = timing_function else {
            return true;
        };
        (x1 - y1).abs() < 0.0001 && (x2 - y2).abs() < 0.0001
    }
}

/// factory for a row view, hands back the reusable view when the engine has one with the same reuse id
pub type RiseManualListItemViewFactory = Rc<dyn Fn(Option<AnyView>, &mut Window, &mut App) -> AnyView>;

pub type RiseManualListItemPlaceholderViewFactory = Rc<dyn Fn(&mut Window, &mut App) -> AnyView>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseManualListItemMountPolicy {
    Immediate,
    DeferredDuringFastScroll,
}

#[derive(Clone)]
pub struct RiseManualListItem {
    pub height: f32,
    pub reuse_identifier: String,
    pub content_version: i64,
    pub mount_policy: RiseManualListItemMountPolicy,
    pub top_sticky_header: Option<RiseManualListTopStickyHeader>,
    pub bottom_sticky_header: Option<RiseManualListBottomStickyHeader>,
    pub placeholder_view: Option<RiseManualListItemPlaceholderViewFactory>,
    pub install_cost: RiseManualListInstallCost,
    pub view: RiseManualListItemViewFactory,
}

impl RiseManualListItem {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        height: f32,
        reuse_identifier: String,
        content_version: i64,
        mount_policy: RiseManualListItemMountPolicy,
        top_sticky_header: Option<RiseManualListTopStickyHeader>,
        bottom_sticky_header: Option<RiseManualListBottomStickyHeader>,
        placeholder_view: Option<RiseManualListItemPlaceholderViewFactory>,
        install_cost: Option<RiseManualListInstallCost>,
        view: RiseManualListItemViewFactory,
    ) -> Self {
        let install_cost =
            install_cost.unwrap_or_else(|| RiseManualListInstallCost::default_cost(&reuse_identifier));
        Self {
            height,
            reuse_identifier,
            content_version,
            mount_policy,
            top_sticky_header,
            bottom_sticky_header,
            placeholder_view,
            install_cost,
            view,
        }
    }
}

pub trait RiseManualListReusableView: Sized + 'static {
    fn prepare_for_manual_list_reuse(&mut self, cx: &mut Context<Self>);
}

pub trait RiseManualListSelectionPresentable: Sized + 'static {
    fn apply_manual_list_selection_presentation(
        &mut self,
        is_active: bool,
        content_translation_x: f32,
        cx: &mut Context<Self>,
    );
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseManualListSelectionItem {
    pub item_id: String,
    pub selection_id: Option<String>,
    pub indicator_center_x: f32,
    pub content_translation_x: f32,
    pub accessibility_label: String,
}

/// on_toggle gets the selection id of the tapped row
pub type RiseManualListSelectionPresentationOnToggle = Rc<dyn Fn(String, &mut Window, &mut App)>;

pub type RiseManualListDragSelectionStart = Rc<dyn Fn(&str, &mut Window, &mut App)>;

pub type RiseManualListTextSource = Rc<dyn Fn(&str) -> Option<(String, String)>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseManualListTextSweepRole {
    Anchor,
    Focus,
    Full,
}

#[derive(Clone)]
pub struct RiseManualListTextSweep {
    pub anchor_id: String,
    pub focus_id: String,
    pub ordered_ids: Vec<String>,
    pub downward: bool,
    pub pointer: Point<Pixels>,
    pub parts: Rc<RefCell<HashMap<String, String>>>,
    pub source: Option<RiseManualListTextSource>,
}

impl RiseManualListTextSweep {
    pub fn role(&self, item_id: &str) -> Option<RiseManualListTextSweepRole> {
        if self.ordered_ids.len() < 2 || !self.ordered_ids.iter().any(|id| id == item_id) {
            return None;
        }
        Some(if item_id == self.anchor_id {
            RiseManualListTextSweepRole::Anchor
        } else if item_id == self.focus_id {
            RiseManualListTextSweepRole::Focus
        } else {
            RiseManualListTextSweepRole::Full
        })
    }

    pub fn copy_text(&self) -> Option<String> {
        if self.ordered_ids.len() < 2 {
            return None;
        }
        let parts = self.parts.borrow();
        let pieces: Vec<(String, String)> = self
            .ordered_ids
            .iter()
            .filter_map(|id| {
                let (sender, full) = self.source.as_ref().and_then(|source| source(id))?;
                let text = parts.get(id).cloned().unwrap_or(full);
                (!text.trim().is_empty()).then_some((sender, text))
            })
            .collect();
        let several_senders = pieces.iter().any(|(sender, _)| Some(sender) != pieces.first().map(|(first, _)| first));
        let lines: Vec<String> = pieces
            .into_iter()
            .map(|(sender, text)| if several_senders && !sender.is_empty() { format!("{sender}: {text}") } else { text })
            .collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }
}

#[derive(Default)]
pub struct RiseManualListTextSweepSlot(pub Option<RiseManualListTextSweep>);

impl Global for RiseManualListTextSweepSlot {}

impl RiseManualListTextSweepSlot {
    pub fn current(cx: &App) -> Option<&RiseManualListTextSweep> {
        cx.try_global::<Self>().and_then(|slot| slot.0.as_ref())
    }

    pub fn set(sweep: Option<RiseManualListTextSweep>, cx: &mut App) {
        if sweep.is_none() && Self::current(cx).is_none() {
            return;
        }
        cx.set_global(Self(sweep));
    }
}

#[derive(Clone)]
pub struct RiseManualListSelectionPresentation {
    pub is_active: bool,
    pub selected_ids: HashSet<String>,
    pub items_by_item_id: HashMap<String, RiseManualListSelectionItem>,
    pub tint_color: Hsla,
    pub on_toggle: Option<RiseManualListSelectionPresentationOnToggle>,
}

impl RiseManualListSelectionPresentation {
    pub fn inactive() -> Self {
        Self {
            is_active: false,
            selected_ids: HashSet::new(),
            items_by_item_id: HashMap::new(),
            tint_color: UIColor::system_blue().resolved_color(Appearance::Light),
            on_toggle: None,
        }
    }
}

pub enum RiseManualListSelectionLayout {}

impl RiseManualListSelectionLayout {
    pub fn indicator_center_x() -> f32 {
        MessageSizes::avatar_leading_inset() + MessageSizes::avatar_size() * 0.5
    }

    /// leading strip the indicator takes from a row
    pub fn indicator_lane_width() -> f32 {
        MessageSizes::avatar_column_width()
    }

    pub fn content_translation_x(is_own: bool, reserves_avatar_space: bool) -> f32 {
        if is_own || reserves_avatar_space {
            return 0.0;
        }
        Self::indicator_lane_width()
    }
}

/// item provider, gives the item for an id at a viewport width
pub type RiseManualListItemProvider = Rc<dyn Fn(&str, f32, &mut Window, &mut App) -> RiseManualListItem>;

#[derive(Clone, Debug, PartialEq)]
pub struct RiseManualListViewportAnchor {
    pub id: String,
    pub viewport_offset: f32,
}

impl RiseManualListViewportAnchor {
    pub fn new(id: String, viewport_offset: f32) -> Self {
        Self { id, viewport_offset }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiseManualListScrollMetrics {
    pub content_offset_y: Pixels,
    pub minimum_content_offset_y: Pixels,
    pub maximum_content_offset_y: Pixels,
    pub viewport_size: Size<Pixels>,
    pub content_height: Pixels,
    pub distance_from_top: Pixels,
    pub distance_from_bottom: Pixels,
    pub velocity_y: Pixels,
    pub is_tracking: bool,
    pub is_dragging: bool,
    pub is_decelerating: bool,
}

/// one desired list state, a newer enqueue replaces the pending one
/// replaced one is discarded at once
/// updated_ids none means every row may have changed
#[derive(Clone)]
pub struct RiseManualListTransaction {
    pub ids: Vec<String>,
    pub anchor: RiseManualListTransactionAnchor,
    pub expected_width: Option<f32>,
    pub updated_ids: Option<HashSet<String>>,
    pub animation: Option<RiseManualListTransactionUpdateAnimation>,
    pub jump_animation: Option<RiseManualListTransactionJumpAnimation>,
    pub edge_mutation: Option<RiseManualListTransactionEdgeMutation>,
    pub allows_application_during_scroll: bool,
    pub scroll_removable_ids: HashSet<String>,
    pub provider: RiseManualListItemProvider,
    pub before_layout: Option<RiseManualListTransactionBeforeLayout>,
}

pub type RiseManualListTransactionBeforeLayout =
    Rc<dyn Fn(&mut RiseManualListView, &mut Window, &mut Context<RiseManualListView>)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseManualListTransactionJumpAnimationDirection {
    TowardOlder,
    TowardNewer,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiseManualListTransactionJumpAnimation {
    pub direction: RiseManualListTransactionJumpAnimationDirection,
    pub duration: TimeInterval,
}

impl RiseManualListTransactionJumpAnimation {
    /// duration is the scroll animation policy DURATION by default
    pub fn new(direction: RiseManualListTransactionJumpAnimationDirection, duration: TimeInterval) -> Self {
        Self { direction, duration }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseManualListTransactionUpdateAnimation {
    pub duration: TimeInterval,
    pub damping_ratio: f32,
    pub content_ids: HashSet<String>,
    pub inserted_ids: HashSet<String>,
    pub removed_ids: HashSet<String>,
    pub dust_ids: HashSet<String>,
    pub bottom_aligned_ids: HashSet<String>,
    pub reveals_growth_ids: HashSet<String>,
    pub clips_growth_ids: HashSet<String>,
    pub reveals_growth_footer_height: f32,
    pub reveals_growth_toggle_height: f32,
    pub reveals_growth_band_background: Option<Hsla>,
}

impl RiseManualListTransactionUpdateAnimation {
    /// defaults: duration 0.34, damping_ratio 0.86 empty inserted_ids and removed_ids and dust_ids
    /// growth fields start empty and are set on the value
    pub fn new(
        duration: TimeInterval,
        damping_ratio: f32,
        content_ids: HashSet<String>,
        inserted_ids: HashSet<String>,
        removed_ids: HashSet<String>,
        dust_ids: HashSet<String>,
    ) -> Self {
        Self {
            duration,
            damping_ratio,
            content_ids,
            inserted_ids,
            removed_ids,
            dust_ids,
            bottom_aligned_ids: HashSet::new(),
            reveals_growth_ids: HashSet::new(),
            clips_growth_ids: HashSet::new(),
            reveals_growth_footer_height: 0.0,
            reveals_growth_toggle_height: 0.0,
            reveals_growth_band_background: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RiseManualListTransactionEdgeMutation {
    pub removed_prefix_count: usize,
    pub removed_suffix_count: usize,
    pub inserted_prefix_count: usize,
    pub inserted_suffix_count: usize,
}

impl RiseManualListTransactionEdgeMutation {
    pub fn new(
        removed_prefix_count: usize,
        removed_suffix_count: usize,
        inserted_prefix_count: usize,
        inserted_suffix_count: usize,
    ) -> Self {
        Self {
            removed_prefix_count,
            removed_suffix_count,
            inserted_prefix_count,
            inserted_suffix_count,
        }
    }

    /// mutation from old_ids to new_ids around their longest contiguous common run
    /// equal lists give the zero mutation, no shared id gives full replace
    /// none when the lists differ and either repeats an id
    pub fn diff(old_ids: &[String], new_ids: &[String]) -> Option<Self> {
        if old_ids == new_ids {
            return Some(Self::default());
        }

        let mut old_indices: HashMap<&str, usize> = HashMap::with_capacity(old_ids.len());
        for (index, id) in old_ids.iter().enumerate() {
            if old_indices.insert(id.as_str(), index).is_some() {
                return None;
            }
        }
        let mut seen_new_ids: HashSet<&str> = HashSet::with_capacity(new_ids.len());
        let mut best_old_start = 0;
        let mut best_new_start = 0;
        let mut best_length = 0;
        let mut run_old_start = 0;
        let mut run_new_start = 0;
        let mut run_length = 0;
        let mut previous_old_index: Option<usize> = None;

        for (new_index, id) in new_ids.iter().enumerate() {
            if !seen_new_ids.insert(id.as_str()) {
                return None;
            }
            let Some(&old_index) = old_indices.get(id.as_str()) else {
                run_length = 0;
                previous_old_index = None;
                continue;
            };
            if previous_old_index.is_some_and(|previous| old_index == previous + 1) {
                run_length += 1;
            } else {
                run_old_start = old_index;
                run_new_start = new_index;
                run_length = 1;
            }
            previous_old_index = Some(old_index);
            if run_length > best_length {
                best_old_start = run_old_start;
                best_new_start = run_new_start;
                best_length = run_length;
            }
        }

        let old_retained = &old_ids[best_old_start..best_old_start + best_length];
        let new_retained = &new_ids[best_new_start..best_new_start + best_length];
        if old_retained != new_retained {
            return None;
        }
        Some(Self::new(
            best_old_start,
            old_ids.len() - best_old_start - best_length,
            best_new_start,
            new_ids.len() - best_new_start - best_length,
        ))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RiseManualListTransactionAnchor {
    PreserveVisible,
    Top,
    Bottom,
    Item { id: String, viewport_offset: f32 },
}

impl RiseManualListTransaction {
    /// defaults: anchor PreserveVisible, everything optional is none
    /// no application during scroll and no scroll removable ids
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ids: Vec<String>,
        anchor: RiseManualListTransactionAnchor,
        expected_width: Option<f32>,
        updated_ids: Option<HashSet<String>>,
        animation: Option<RiseManualListTransactionUpdateAnimation>,
        jump_animation: Option<RiseManualListTransactionJumpAnimation>,
        edge_mutation: Option<RiseManualListTransactionEdgeMutation>,
        allows_application_during_scroll: bool,
        scroll_removable_ids: HashSet<String>,
        provider: RiseManualListItemProvider,
    ) -> Self {
        Self {
            ids: Self::uniqued(ids),
            anchor,
            expected_width,
            updated_ids,
            animation,
            jump_animation,
            edge_mutation,
            allows_application_during_scroll,
            scroll_removable_ids,
            provider,
            before_layout: None,
        }
    }

    fn uniqued(ids: Vec<String>) -> Vec<String> {
        let repeated: HashSet<usize> = {
            let mut seen: HashSet<&str> = HashSet::with_capacity(ids.len());
            let mut repeated = HashSet::new();
            for (index, id) in ids.iter().enumerate() {
                let inserted = seen.insert(id.as_str());
                debug_assert!(inserted, "Manual list ids must be unique, dropping {id}");
                if !inserted {
                    repeated.insert(index);
                }
            }
            repeated
        };
        if repeated.is_empty() {
            return ids;
        }
        ids.into_iter()
            .enumerate()
            .filter(|(index, _)| !repeated.contains(index))
            .map(|(_, id)| id)
            .collect()
    }
}
