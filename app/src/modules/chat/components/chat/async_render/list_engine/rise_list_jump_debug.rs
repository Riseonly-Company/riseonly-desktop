use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use gpui::{App, EntityId, Global, Pixels};
use rise_platform::process_info::process_info;

use crate::core::lib::sai::sai::Sai;

pub enum RiseListJumpDebug {}

pub trait RiseListJumpDebugValue {
    fn debug_points(self) -> f32;
}

impl RiseListJumpDebugValue for f32 {
    fn debug_points(self) -> f32 {
        self
    }
}

impl RiseListJumpDebugValue for Pixels {
    fn debug_points(self) -> f32 {
        self.as_f32()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseListJumpDebugRow {
    pub id: String,
    pub model_min_y: f32,
    pub view_min_y: f32,
    pub presentation_min_y: Option<f32>,
    pub animation_count: i64,
    pub height: f32,
    pub transform_y: f32,
}

#[derive(Clone, Copy, Debug)]
struct RiseListJumpDebugSample {
    view_min_y: f32,
    height: f32,
    screen_y: f32,
}

#[derive(Default)]
struct RiseListJumpDebugOwnerState {
    samples: HashMap<String, RiseListJumpDebugSample>,
    drift: HashMap<String, f32>,
    reported_drift_step: HashMap<String, i64>,
    gaps: HashMap<String, f32>,
    desynced: HashSet<String>,
    animating: HashSet<String>,
    host_animating: bool,
    reversal: HashMap<String, f32>,
    reported_reversal_step: HashMap<String, i64>,
    pan_translation_y: Option<f32>,
    offset_y: Option<f32>,
    origin_y: f32,
    coordinate_offset_y: f32,
    host_translation_y: f32,
    generation: u64,
}

/// states and bubble centers keyed by the owner entity
#[derive(Default)]
struct RiseListJumpDebugStates {
    states: HashMap<EntityId, RiseListJumpDebugOwnerState>,
    bubble_centers: HashMap<EntityId, f32>,
}

impl Global for RiseListJumpDebugStates {}

static DISABLES_BOTTOM_CLAMP: LazyLock<bool> = LazyLock::new(|| {
    if !RiseListJumpDebug::is_enabled() {
        return false;
    }
    process_info::arguments().iter().any(|argument| argument == "-riseDisableBottomClamp")
});

static IS_ENABLED: LazyLock<bool> = LazyLock::new(|| {
    cfg!(debug_assertions) && process_info::arguments().iter().any(|argument| argument == "-riseListJumpDebug")
});

struct RiseListJumpDebugJump {
    row: RiseListJumpDebugRow,
    delta: f32,
    screen_y: f32,
    previous: RiseListJumpDebugSample,
}

impl RiseListJumpDebug {
    pub fn disables_bottom_clamp() -> bool {
        *DISABLES_BOTTOM_CLAMP
    }

    pub fn is_enabled() -> bool {
        *IS_ENABLED
    }

    pub fn log(message: &str, cx: &mut App) {
        if !Self::is_enabled() {
            return;
        }
        let message = message.to_string();
        Sai::log_channel("RiseJump", || message, "jump", cx);
    }

    /// formats a point value, works for both Pixels and plain f32
    pub fn f(value: impl RiseListJumpDebugValue) -> String {
        Self::f_optional(Some(value))
    }

    pub fn f_optional(value: Option<impl RiseListJumpDebugValue>) -> String {
        match value {
            Some(value) => format!("{:.2}", value.debug_points()),
            None => "nil".to_string(),
        }
    }

    pub fn begin_session(owner: EntityId, cx: &mut App) {
        if !Self::is_enabled() {
            return;
        }
        let state = cx
            .default_global::<RiseListJumpDebugStates>()
            .states
            .entry(owner)
            .or_default();
        state.drift.clear();
        state.reported_drift_step.clear();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sample(
        owner: EntityId,
        rows: &[RiseListJumpDebugRow],
        offset_y: f32,
        pan_translation_y: f32,
        origin_y: f32,
        coordinate_offset_y: f32,
        host_translation_y: f32,
        host_presentation_lag: f32,
        host_animation_count: i64,
        generation: u64,
        viewport_bottom: f32,
        phase: &str,
        cx: &mut App,
    ) {
        if !Self::is_enabled() {
            return;
        }

        let mut lines: Vec<String> = Vec::new();
        let mut state = cx
            .default_global::<RiseListJumpDebugStates>()
            .states
            .remove(&owner)
            .unwrap_or_default();

        let host_is_moving = host_presentation_lag.abs() > 0.5 || host_animation_count > 0;
        if host_is_moving != state.host_animating {
            state.host_animating = host_is_moving;
            if host_is_moving {
                lines.push(format!(
                    "HOST-ANIM phase={phase} presentationLag={} animations={host_animation_count} offsetY={}",
                    Self::f(host_presentation_lag),
                    Self::f(offset_y)
                ));
            }
        }
        let mut next: HashMap<String, RiseListJumpDebugSample> = HashMap::with_capacity(rows.len());

        let mut causes: Vec<String> = Vec::new();
        if (origin_y - state.origin_y).abs() > 0.01 {
            causes.push(format!("origin {}->{}", Self::f(state.origin_y), Self::f(origin_y)));
        }
        if (coordinate_offset_y - state.coordinate_offset_y).abs() > 0.01 {
            causes.push(format!("coord {}->{}", Self::f(state.coordinate_offset_y), Self::f(coordinate_offset_y)));
        }
        if (host_translation_y - state.host_translation_y).abs() > 0.01 {
            causes.push(format!("hostTy {}->{}", Self::f(state.host_translation_y), Self::f(host_translation_y)));
        }
        if generation != state.generation {
            causes.push(format!("gen {}->{generation}", state.generation));
        }
        let cause_text = if causes.is_empty() {
            "none".to_string()
        } else {
            causes.join(" ")
        };

        let mut jumps: Vec<RiseListJumpDebugJump> = Vec::new();
        let mut next_drift: HashMap<String, f32> = HashMap::with_capacity(rows.len());

        for row in rows {
            let screen_y = origin_y + row.presentation_min_y.unwrap_or(row.view_min_y) + row.transform_y - offset_y
                + host_translation_y;
            next.insert(
                row.id.clone(),
                RiseListJumpDebugSample {
                    view_min_y: row.view_min_y,
                    height: row.height,
                    screen_y,
                },
            );

            if let Some(presentation_min_y) = row.presentation_min_y {
                let lag = presentation_min_y - row.view_min_y;
                if lag.abs() > 0.5 || row.animation_count > 0 {
                    if !state.animating.contains(&row.id) {
                        state.animating.insert(row.id.clone());
                        lines.push(format!(
                            "ROW-ANIM phase={phase} id={} presentationLag={} animations={} viewMinY={} \
                             presentationMinY={} screenY={} distToInput={}",
                            row.id,
                            Self::f(lag),
                            row.animation_count,
                            Self::f(row.view_min_y),
                            Self::f(presentation_min_y),
                            Self::f(screen_y),
                            Self::f(viewport_bottom - screen_y)
                        ));
                    }
                } else {
                    state.animating.remove(&row.id);
                }
            }

            let desync = row.view_min_y - row.model_min_y;
            if desync.abs() > 0.5 {
                if !state.desynced.contains(&row.id) {
                    state.desynced.insert(row.id.clone());
                    lines.push(format!(
                        "VIEW-DESYNC phase={phase} id={} desync={} viewMinY={} modelMinY={} ty={} screenY={} \
                         distToInput={}",
                        row.id,
                        Self::f(desync),
                        Self::f(row.view_min_y),
                        Self::f(row.model_min_y),
                        Self::f(row.transform_y),
                        Self::f(screen_y),
                        Self::f(viewport_bottom - screen_y)
                    ));
                }
            } else {
                state.desynced.remove(&row.id);
            }

            let (Some(previous_offset_y), Some(previous)) = (state.offset_y, state.samples.get(&row.id).copied())
            else {
                continue;
            };
            let delta = screen_y - previous.screen_y + (offset_y - previous_offset_y);
            if delta.abs() <= 0.01 {
                next_drift.insert(row.id.clone(), state.drift.get(&row.id).copied().unwrap_or(0.0));
                continue;
            }

            let accumulated = state.drift.get(&row.id).copied().unwrap_or(0.0) + delta;
            next_drift.insert(row.id.clone(), accumulated);
            let step = accumulated.abs() as i64;
            if step >= 1 && step > state.reported_drift_step.get(&row.id).copied().unwrap_or(0) {
                state.reported_drift_step.insert(row.id.clone(), step);
                lines.push(format!(
                    "ROW-DRIFT phase={phase} total={} id={} screenY={} distToInput={} h={} offsetY={} \
                     cause=[{cause_text}]",
                    Self::f(accumulated),
                    row.id,
                    Self::f(screen_y),
                    Self::f(viewport_bottom - screen_y),
                    Self::f(row.height),
                    Self::f(offset_y)
                ));
            }

            if delta.abs() > 0.5 {
                jumps.push(RiseListJumpDebugJump {
                    row: row.clone(),
                    delta,
                    screen_y,
                    previous,
                });
            }
        }

        if !jumps.is_empty() {
            jumps.sort_by(|a, b| b.delta.abs().total_cmp(&a.delta.abs()));
            for jump in jumps.iter().take(3) {
                let row = &jump.row;
                let height_changed = (row.height - jump.previous.height).abs() > 0.01;
                lines.push(format!(
                    "ROW-JUMP phase={phase} dy={} id={} screenY={}->{} distToInput={} h={}->{}{} viewMinY={}->{} \
                     modelDesync={} ty={} offsetY={} cause=[{cause_text}] moved={}/{}",
                    Self::f(jump.delta),
                    row.id,
                    Self::f(jump.previous.screen_y),
                    Self::f(jump.screen_y),
                    Self::f(viewport_bottom - jump.screen_y),
                    Self::f(jump.previous.height),
                    Self::f(row.height),
                    if height_changed { "(CHANGED)" } else { "" },
                    Self::f(jump.previous.view_min_y),
                    Self::f(row.view_min_y),
                    Self::f(row.view_min_y - row.model_min_y),
                    Self::f(row.transform_y),
                    Self::f(offset_y),
                    jumps.len(),
                    rows.len()
                ));
            }
        }

        let mut ordered: Vec<&RiseListJumpDebugRow> = rows.iter().collect();
        ordered.sort_by(|a, b| a.model_min_y.total_cmp(&b.model_min_y));
        let pan_delta_y = state.pan_translation_y.map(|previous| pan_translation_y - previous);
        if phase == "drag"
            && let Some(pan_delta_y) = pan_delta_y
            && pan_delta_y >= -0.05
        {
            for row in ordered.iter().skip(ordered.len().saturating_sub(3)) {
                let (Some(sample), Some(previous)) = (next.get(&row.id), state.samples.get(&row.id)) else {
                    continue;
                };
                let step = sample.screen_y - previous.screen_y;
                if step >= -0.05 {
                    state.reversal.insert(row.id.clone(), 0.0);
                    state.reported_reversal_step.insert(row.id.clone(), 0);
                    continue;
                }
                let rise = state.reversal.get(&row.id).copied().unwrap_or(0.0) - step;
                state.reversal.insert(row.id.clone(), rise);
                let rise_step = rise as i64;
                if !(rise_step >= 1 && rise_step > state.reported_reversal_step.get(&row.id).copied().unwrap_or(0)) {
                    continue;
                }
                state.reported_reversal_step.insert(row.id.clone(), rise_step);
                lines.push(format!(
                    "ROW-RISE-AGAINST-FINGER rise={} panDelta={} id={} screenY={}->{} distToInput={} offsetY={} \
                     cause=[{cause_text}]",
                    Self::f(rise),
                    Self::f(pan_delta_y),
                    row.id,
                    Self::f(previous.screen_y),
                    Self::f(sample.screen_y),
                    Self::f(viewport_bottom - sample.screen_y),
                    Self::f(offset_y)
                ));
            }
        }

        let mut next_gaps: HashMap<String, f32> = HashMap::with_capacity(ordered.len().saturating_sub(1));
        for pair in ordered.windows(2) {
            let (upper, lower) = (pair[0], pair[1]);
            let gap = lower.view_min_y - (upper.view_min_y + upper.height);
            next_gaps.insert(upper.id.clone(), gap);
            let previous_gap = state.gaps.get(&upper.id).copied();
            let changed = previous_gap.is_some_and(|previous_gap| (gap - previous_gap).abs() > 0.3);
            if !(changed || gap < -0.5) {
                continue;
            }
            lines.push(format!(
                "ROW-GAP phase={phase} gap={}->{}{} upper={} lower={} upperH={} upperScreenY={} distToInput={} \
                 cause=[{cause_text}]",
                Self::f_optional(previous_gap),
                Self::f(gap),
                if gap < -0.5 { " OVERLAP" } else { "" },
                upper.id,
                lower.id,
                Self::f(upper.height),
                Self::f(origin_y + upper.view_min_y - offset_y + host_translation_y),
                Self::f(viewport_bottom - (origin_y + lower.view_min_y - offset_y + host_translation_y))
            ));
        }

        state.samples = next;
        state.drift = next_drift;
        state.gaps = next_gaps;
        state.pan_translation_y = Some(pan_translation_y);
        state.offset_y = Some(offset_y);
        state.origin_y = origin_y;
        state.coordinate_offset_y = coordinate_offset_y;
        state.host_translation_y = host_translation_y;
        state.generation = generation;
        cx.default_global::<RiseListJumpDebugStates>()
            .states
            .insert(owner, state);
        for line in lines {
            Self::log(&line, cx);
        }
    }

    /// row level detectors are blind to the bubble moving inside its own cell
    /// the cell doesnt clip so this is what catches it leaving its container
    pub fn record_bubble_layout(
        id: EntityId,
        center_y: f32,
        top_padding: f32,
        bubble_height: f32,
        bounds_height: f32,
        is_own: bool,
        cx: &mut App,
    ) {
        if !Self::is_enabled() {
            return;
        }

        let overflow_top = -top_padding;
        let overflow_bottom = (top_padding + bubble_height) - bounds_height;
        let previous_center_y = cx
            .default_global::<RiseListJumpDebugStates>()
            .bubble_centers
            .insert(id, center_y);

        if overflow_bottom > 0.01 || overflow_top > 0.01 {
            Self::log(
                &format!(
                    "BUBBLE-OVERFLOW own={is_own} overflowTop={} overflowBottom={} topPadding={} bubbleH={} rowH={}",
                    Self::f(overflow_top),
                    Self::f(overflow_bottom),
                    Self::f(top_padding),
                    Self::f(bubble_height),
                    Self::f(bounds_height)
                ),
                cx,
            );
        }

        let Some(previous_center_y) = previous_center_y else { return };
        if (center_y - previous_center_y).abs() <= 0.5 {
            return;
        }
        Self::log(
            &format!(
                "BUBBLE-MOVE own={is_own} centerY={}->{} dy={} implicitAnimationsEnabled=false topPadding={} \
                 bubbleH={} rowH={}",
                Self::f(previous_center_y),
                Self::f(center_y),
                Self::f(center_y - previous_center_y),
                Self::f(top_padding),
                Self::f(bubble_height),
                Self::f(bounds_height)
            ),
            cx,
        );
    }

    pub fn forget(owner: EntityId, cx: &mut App) {
        if !Self::is_enabled() {
            return;
        }
        cx.default_global::<RiseListJumpDebugStates>().states.remove(&owner);
    }
}
