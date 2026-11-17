use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use rise_canon::{
    RiseGuideAdvance, RiseGuideAnchor, RiseGuideCanon, RiseGuideFailure, RiseGuideGesture,
    RiseGuideIcon, RiseGuideNarrator, RiseGuideOutcome, RiseGuidePlacement, RiseGuidePose,
    RiseGuideScreen, RiseGuideSignal, RiseGuideSpotlightShape, RiseGuideStage, RiseGuideSurface,
    RiseGuideTier, RiseGuideTopic, RiseGuideTrigger,
};
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_core::foundation::foundation_text::{character_count, trimming_whitespaces_and_newlines};
use serde::{Deserialize, Serialize};

use crate::modules::guides::engine::rise_guides_wire_models::{
    RiseGuideDTO, RiseGuideReceiptDTO, RiseGuideStepDTO,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseGuidesError {
    #[error("missingActiveSession")]
    MissingActiveSession,
    #[error("staleAccount")]
    StaleAccount,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseGuidesCacheSource {
    Miss,
    Memory,
    PersistentStore,
    Network,
}

pub trait RiseGuideWireValue: Copy + 'static {
    fn all_cases() -> &'static [Self];
    fn raw_value(self) -> &'static str;
}

macro_rules! rise_guide_wire_value {
    ($($value:ty),+ $(,)?) => {
        $(
            impl RiseGuideWireValue for $value {
                fn all_cases() -> &'static [Self] {
                    &<$value>::ALL
                }

                fn raw_value(self) -> &'static str {
                    <$value>::raw_value(self)
                }
            }
        )+
    };
}

rise_guide_wire_value!(
    RiseGuideAdvance,
    RiseGuideAnchor,
    RiseGuideGesture,
    RiseGuideIcon,
    RiseGuideNarrator,
    RiseGuideOutcome,
    RiseGuidePlacement,
    RiseGuidePose,
    RiseGuideScreen,
    RiseGuideSignal,
    RiseGuideSpotlightShape,
    RiseGuideStage,
    RiseGuideSurface,
    RiseGuideTier,
    RiseGuideTopic,
    RiseGuideTrigger,
);

/// wire strings become canon enums here and nowhere else
/// unknown name resolves to none instead of failing so a stage from a newer server costs one
/// guide not the catalogue
pub enum RiseGuideWireEnum {}

impl RiseGuideWireEnum {
    pub fn resolve<Value: RiseGuideWireValue>(raw: &str) -> Option<Value> {
        let trimmed = trimming_whitespaces_and_newlines(raw);
        if trimmed.is_empty() {
            return None;
        }
        let cases = Value::all_cases();
        if let Some(exact) = cases.iter().copied().find(|case| case.raw_value() == trimmed) {
            return Some(exact);
        }
        cases
            .iter()
            .copied()
            .find(|case| compare_case_insensitive(case.raw_value(), trimmed) == Ordering::Equal)
    }
}

/// one step resolved against this build
/// any init failure means this build cant perform the step as authored, a half built step
/// would spotlight empty space or wait for a signal nothing emits
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RiseGuideStepModel {
    pub position: i64,
    pub stage: RiseGuideStage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<RiseGuideAnchor>,
    #[serde(rename = "spotlightShape")]
    pub spotlight_shape: RiseGuideSpotlightShape,
    pub narrator: RiseGuideNarrator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pose: Option<RiseGuidePose>,
    pub surface: RiseGuideSurface,
    pub placement: RiseGuidePlacement,
    pub title: String,
    pub body: String,
    #[serde(rename = "mediaURL")]
    pub media_url: String,
    pub advance: RiseGuideAdvance,
    #[serde(rename = "advanceSignal", default, skip_serializing_if = "Option::is_none")]
    pub advance_signal: Option<RiseGuideSignal>,
    #[serde(rename = "advanceScreen", default, skip_serializing_if = "Option::is_none")]
    pub advance_screen: Option<RiseGuideScreen>,
    #[serde(rename = "delayMs")]
    pub delay_ms: i64,
    #[serde(rename = "timeoutMs")]
    pub timeout_ms: i64,
    #[serde(rename = "routeScreen", default, skip_serializing_if = "Option::is_none")]
    pub route_screen: Option<RiseGuideScreen>,
    #[serde(rename = "routeParams")]
    pub route_params: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gesture: Option<RiseGuideGesture>,
}

impl RiseGuideStepModel {
    /// steps have no wire id, position is unique inside a guide
    pub fn id(&self) -> i64 {
        self.position
    }

    pub fn from_dto(dto: &RiseGuideStepDTO) -> Option<Self> {
        if dto.position < 0 {
            return None;
        }
        let stage = RiseGuideWireEnum::resolve::<RiseGuideStage>(&dto.stage)?;
        let surface = RiseGuideWireEnum::resolve::<RiseGuideSurface>(&dto.surface)?;
        let advance = RiseGuideWireEnum::resolve::<RiseGuideAdvance>(&dto.advance)?;

        let anchor_name = trimming_whitespaces_and_newlines(&dto.anchor);
        let anchor = RiseGuideWireEnum::resolve::<RiseGuideAnchor>(anchor_name);
        if !(anchor_name.is_empty() || anchor.is_some()) {
            return None;
        }
        if stage.requires_anchor() && anchor.is_none() {
            return None;
        }

        let narrator = RiseGuideWireEnum::resolve::<RiseGuideNarrator>(&dto.narrator)
            .unwrap_or(RiseGuideNarrator::None);
        if surface.requires_narrator() && narrator == RiseGuideNarrator::None {
            return None;
        }
        if surface.is_major() && stage.requires_anchor() {
            return None;
        }
        if surface.carries_text() && trimming_whitespaces_and_newlines(&dto.body).is_empty() {
            return None;
        }

        let media_url = trimming_whitespaces_and_newlines(&dto.media_url);
        if surface == RiseGuideSurface::Video && media_url.is_empty() {
            return None;
        }

        let target = trimming_whitespaces_and_newlines(&dto.advance_target);
        let mut advance_signal: Option<RiseGuideSignal> = None;
        let mut advance_screen: Option<RiseGuideScreen> = None;
        match advance {
            RiseGuideAdvance::Signal => {
                advance_signal = Some(RiseGuideWireEnum::resolve::<RiseGuideSignal>(target)?);
            }
            RiseGuideAdvance::Screen => {
                advance_screen = Some(RiseGuideWireEnum::resolve::<RiseGuideScreen>(target)?);
            }
            RiseGuideAdvance::TapContinue
            | RiseGuideAdvance::TapAnchor
            | RiseGuideAdvance::Delay
            | RiseGuideAdvance::VideoEnd => {}
        }
        let requires_surface = advance.requires_surface();
        if !(requires_surface.is_empty() || requires_surface == surface.raw_value()) {
            return None;
        }
        if advance.forbids_anchored_stage() && stage.requires_anchor() {
            return None;
        }

        let route_name = trimming_whitespaces_and_newlines(&dto.route_screen);
        let route_screen = RiseGuideWireEnum::resolve::<RiseGuideScreen>(route_name);
        if !(route_name.is_empty() || route_screen.is_some()) {
            return None;
        }
        let route_params = Self::route_params(&dto.route_params)?;

        let spotlight_shape =
            RiseGuideWireEnum::resolve::<RiseGuideSpotlightShape>(&dto.spotlight_shape)
                .or_else(|| {
                    anchor.and_then(|anchor| {
                        RiseGuideWireEnum::resolve::<RiseGuideSpotlightShape>(anchor.shape())
                    })
                })
                .unwrap_or(RiseGuideSpotlightShape::RoundedRect);
        let pose = if narrator == RiseGuideNarrator::Panda {
            Some(
                RiseGuideWireEnum::resolve::<RiseGuidePose>(&dto.pose)
                    .unwrap_or(RiseGuidePose::Explain),
            )
        } else {
            None
        };
        let placement = RiseGuideWireEnum::resolve::<RiseGuidePlacement>(&dto.placement)
            .unwrap_or(RiseGuidePlacement::Auto);
        let delay_ms = if advance == RiseGuideAdvance::Delay {
            dto.delay_ms
                .max(RiseGuideCanon::TIMINGS_DELAY_MIN_MS)
                .min(RiseGuideCanon::TIMINGS_DELAY_MAX_MS)
        } else {
            0
        };
        let timeout_ms = if dto.timeout_ms > 0 {
            dto.timeout_ms
                .max(RiseGuideCanon::TIMINGS_STEP_TIMEOUT_MIN_MS)
                .min(RiseGuideCanon::TIMINGS_STEP_TIMEOUT_MAX_MS)
        } else {
            RiseGuideCanon::TIMINGS_STEP_TIMEOUT_DEFAULT_MS
        };
        let gesture = if stage.requires_anchor() {
            RiseGuideWireEnum::resolve::<RiseGuideGesture>(&dto.gesture)
        } else {
            None
        };

        Some(Self {
            position: dto.position,
            stage,
            anchor,
            spotlight_shape,
            narrator,
            pose,
            surface,
            placement,
            title: dto.title.clone(),
            body: dto.body.clone(),
            media_url: media_url.to_string(),
            advance,
            advance_signal,
            advance_screen,
            delay_ms,
            timeout_ms,
            route_screen,
            route_params,
            gesture,
        })
    }

    /// none means unreadable params, a route reaching the right screen without its subject
    fn route_params(raw: &str) -> Option<HashMap<String, String>> {
        let trimmed = trimming_whitespaces_and_newlines(raw);
        if trimmed.is_empty() {
            return Some(HashMap::new());
        }
        if character_count(trimmed) as i64 > RiseGuideCanon::LIMITS_ROUTE_PARAMS_MAX_CHARS {
            return None;
        }
        serde_json::from_str::<HashMap<String, String>>(trimmed).ok()
    }
}

/// one guide as the runtime executes it with canon vocabularies resolved
/// separate from the wire dto since this is what the encrypted store holds, renaming a wire
/// field later must not invalidate cached guides
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RiseGuide {
    pub id: i64,
    pub key: String,
    pub revision: i64,
    pub tier: RiseGuideTier,
    pub trigger: RiseGuideTrigger,
    #[serde(rename = "triggerScreen", default, skip_serializing_if = "Option::is_none")]
    pub trigger_screen: Option<RiseGuideScreen>,
    #[serde(rename = "triggerSignal", default, skip_serializing_if = "Option::is_none")]
    pub trigger_signal: Option<RiseGuideSignal>,
    pub reteach: bool,
    pub priority: i64,
    pub locale: String,
    pub title: String,
    pub summary: String,
    #[serde(rename = "minAppVersion")]
    pub min_app_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<RiseGuideTopic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<RiseGuideIcon>,
    pub steps: Vec<RiseGuideStepModel>,
}

impl RiseGuide {
    pub fn from_dto(dto: &RiseGuideDTO) -> Option<Self> {
        if dto.id <= 0 {
            return None;
        }
        let key = trimming_whitespaces_and_newlines(&dto.key);
        if key.is_empty() || character_count(key) as i64 > RiseGuideCanon::LIMITS_KEY_MAX_CHARS {
            return None;
        }
        let tier = RiseGuideWireEnum::resolve::<RiseGuideTier>(&dto.tier)?;
        let trigger = RiseGuideWireEnum::resolve::<RiseGuideTrigger>(&dto.trigger_kind)?;

        let screen_name = trimming_whitespaces_and_newlines(&dto.trigger_screen);
        let trigger_screen = RiseGuideWireEnum::resolve::<RiseGuideScreen>(screen_name);
        if !(screen_name.is_empty() || trigger_screen.is_some()) {
            return None;
        }
        if trigger.needs_screen() && trigger_screen.is_none() {
            return None;
        }
        let trigger_signal = if trigger.needs_signal() {
            RiseGuideWireEnum::resolve::<RiseGuideSignal>(&dto.trigger_signal)
        } else {
            None
        };

        let mut sorted: Vec<&RiseGuideStepDTO> = dto.steps.iter().collect();
        sorted.sort_by_key(|step| step.position);
        let mut steps: Vec<RiseGuideStepModel> = Vec::with_capacity(sorted.len());
        for step in sorted {
            steps.push(RiseGuideStepModel::from_dto(step)?);
        }
        let max_steps = tier.max_steps().min(RiseGuideCanon::LIMITS_MAX_STEPS_PER_GUIDE);
        let positions: HashSet<i64> = steps.iter().map(|step| step.position).collect();
        if steps.is_empty() || steps.len() as i64 > max_steps || positions.len() != steps.len() {
            return None;
        }

        Some(Self {
            id: dto.id,
            key: key.to_string(),
            revision: dto.revision.max(1),
            tier,
            trigger,
            trigger_screen,
            trigger_signal,
            reteach: dto.reteach,
            priority: dto
                .priority
                .max(RiseGuideCanon::LIMITS_PRIORITY_MIN)
                .min(RiseGuideCanon::LIMITS_PRIORITY_MAX),
            locale: dto.locale.clone(),
            title: dto.title.clone(),
            summary: dto.summary.clone(),
            min_app_version: trimming_whitespaces_and_newlines(&dto.min_app_version).to_string(),
            topic: RiseGuideWireEnum::resolve::<RiseGuideTopic>(&dto.topic),
            icon: RiseGuideWireEnum::resolve::<RiseGuideIcon>(&dto.icon),
            steps,
        })
    }

    /// server already applies the version window so this only matters for the cache
    /// a catalogue written by one build and read by an older one points at missing ui
    pub fn is_supported(&self, by_app_version: &str) -> bool {
        Self::is_version(by_app_version, &self.min_app_version)
    }

    pub fn is_version(version: &str, at_least: &str) -> bool {
        let minimum_parts = Self::numeric_parts(at_least);
        if minimum_parts.is_empty() {
            return true;
        }
        let version_parts = Self::numeric_parts(version);
        for index in 0..version_parts.len().max(minimum_parts.len()) {
            let left = version_parts.get(index).copied().unwrap_or(0);
            let right = minimum_parts.get(index).copied().unwrap_or(0);
            if left != right {
                return left > right;
            }
        }
        true
    }

    /// dotted numeric, the only form either end accepts
    /// the gateway refuses an unreadable app_version and min_app_version is computed from
    /// anchors of the same shape
    fn numeric_parts(value: &str) -> Vec<i64> {
        let trimmed = trimming_whitespaces_and_newlines(value);
        if trimmed.is_empty() {
            return Vec::new();
        }
        let parts: Option<Vec<i64>> = trimmed.split('.').map(int_from_string).collect();
        parts.unwrap_or_default()
    }
}

/// failed carries a reason and no other case may, the gateway refuses both halves of that
/// mismatch so its encoded in the type
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseGuideAck {
    Shown,
    Skipped,
    Completed,
    Failed(RiseGuideFailure),
}

impl RiseGuideAck {
    pub fn outcome(&self) -> RiseGuideOutcome {
        match self {
            RiseGuideAck::Shown => RiseGuideOutcome::Shown,
            RiseGuideAck::Skipped => RiseGuideOutcome::Skipped,
            RiseGuideAck::Completed => RiseGuideOutcome::Completed,
            RiseGuideAck::Failed(_) => RiseGuideOutcome::Failed,
        }
    }

    pub fn failure(&self) -> Option<RiseGuideFailure> {
        match self {
            RiseGuideAck::Shown | RiseGuideAck::Skipped | RiseGuideAck::Completed => None,
            RiseGuideAck::Failed(reason) => Some(*reason),
        }
    }
}

/// what the account did with a guide as far as the ratchet and the automatic start budget see it
/// kept on the device too since the ack is fire and forget and a dropped one would let the
/// next refresh re-offer a finished guide
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiseGuideReceipt {
    #[serde(rename = "guideID")]
    pub guide_id: i64,
    #[serde(rename = "guideRevision")]
    pub guide_revision: i64,
    pub outcome: RiseGuideOutcome,
    #[serde(rename = "reachedStep")]
    pub reached_step: i64,
    #[serde(rename = "autoStarts")]
    pub auto_starts: i64,
    #[serde(rename = "lastAutoStartAtMs")]
    pub last_auto_start_at_ms: i64,
    #[serde(rename = "recordedAtMs")]
    pub recorded_at_ms: i64,
}

impl RiseGuideReceipt {
    pub fn id(&self) -> i64 {
        self.guide_id
    }

    pub fn from_dto(dto: &RiseGuideReceiptDTO, recorded_at_ms: i64) -> Option<Self> {
        if dto.guide_id <= 0 {
            return None;
        }
        let outcome = RiseGuideWireEnum::resolve::<RiseGuideOutcome>(&dto.outcome)?;
        Some(Self {
            guide_id: dto.guide_id,
            guide_revision: dto.guide_revision.max(1),
            outcome,
            reached_step: dto.reached_step.max(0),
            auto_starts: dto.auto_starts.max(0),
            last_auto_start_at_ms: dto.last_auto_start_at_ms.max(0),
            recorded_at_ms,
        })
    }

    pub fn new(
        guide_id: i64,
        guide_revision: i64,
        outcome: RiseGuideOutcome,
        reached_step: i64,
        auto_starts: i64,
        last_auto_start_at_ms: i64,
        recorded_at_ms: i64,
    ) -> Self {
        Self {
            guide_id,
            guide_revision,
            outcome,
            reached_step,
            auto_starts,
            last_auto_start_at_ms,
            recorded_at_ms,
        }
    }

    /// every field is a ratchet, the revision included
    /// a newer revision doesnt restart the receipt, a typo fix grows it too and the first shown
    /// of the fixed revision must not erase a completed
    pub fn ratcheted(&self, with: &RiseGuideReceipt) -> RiseGuideReceipt {
        RiseGuideReceipt::new(
            self.guide_id,
            self.guide_revision.max(with.guide_revision),
            if with.outcome.rank() > self.outcome.rank() { with.outcome } else { self.outcome },
            self.reached_step.max(with.reached_step),
            self.auto_starts.max(with.auto_starts),
            self.last_auto_start_at_ms.max(with.last_auto_start_at_ms),
            self.recorded_at_ms.max(with.recorded_at_ms),
        )
    }
}

/// whether a guide may start on its own now, from a screen or signal or first launch or
/// empty state
/// checked against the same golden vectors as canon, the catalogue never asks
pub enum RiseGuideAutoStartPolicy {}

impl RiseGuideAutoStartPolicy {
    pub fn allows_auto_start(
        receipt: Option<&RiseGuideReceipt>,
        guide_revision: i64,
        reteach: bool,
        now_ms: i64,
    ) -> bool {
        let Some(receipt) = receipt else { return true };
        if reteach && guide_revision > receipt.guide_revision {
            return true;
        }
        if receipt.outcome.suppresses_reoffer() {
            return false;
        }
        if receipt.auto_starts >= RiseGuideCanon::LIMITS_MAX_AUTO_STARTS_PER_GUIDE {
            return false;
        }
        now_ms - receipt.last_auto_start_at_ms >= RiseGuideCanon::TIMINGS_REOFFER_COOLDOWN_MS
    }
}
