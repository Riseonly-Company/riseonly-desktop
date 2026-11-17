use gpui::{App, Bounds, Pixels};
use rise_canon::{RiseGuideAdvance, RiseGuideAnchor, RiseGuideScreen, RiseGuideSignal, RiseGuideSpotlightShape};
use rise_ui::guide_anchor_ui::guide_anchor_registry::GuideAnchorSighting;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::modules::guides::engine::rise_guides_models::RiseGuideStepModel;

/// every decision a running guide makes goes to the log in order
/// a stalled guide only shows nothing happened and that fits many causes
pub enum GuideTrace {}

impl GuideTrace {
    const TAG: &'static str = "Guides";

    pub fn started(key: &str, steps: i64, origin: &str, cx: &mut App) {
        Self::log(format!("start {key} · {steps} steps · from {origin}"), cx);
    }

    pub fn step(index: i64, total: i64, step: &RiseGuideStepModel, cx: &mut App) {
        let anchor = step.anchor.map_or("—", RiseGuideAnchor::raw_value);
        let route = step.route_screen.map_or("—", RiseGuideScreen::raw_value);
        Self::log(
            format!(
                "step {}/{}: stage={} anchor={} route={} advance={}",
                index + 1,
                total,
                step.stage.raw_value(),
                anchor,
                route,
                Self::advance_description(step)
            ),
            cx,
        );
    }

    pub fn routing(screen: RiseGuideScreen, accepted: bool, cx: &mut App) {
        Self::log(
            if accepted {
                format!("route → {}, waiting for arrival", screen.raw_value())
            } else {
                format!("route → {} REFUSED", screen.raw_value())
            },
            cx,
        );
    }

    pub fn arrived(screen: RiseGuideScreen, cx: &mut App) {
        Self::log(format!("arrived at {}", screen.raw_value()), cx);
    }

    pub fn anchor_resolved(anchor: RiseGuideAnchor, frame: Bounds<Pixels>, cx: &mut App) {
        Self::log(format!("anchor {} settled at {}", anchor.raw_value(), Self::short(frame)), cx);
    }

    pub fn anchor_pending(anchor: RiseGuideAnchor, cx: &mut App) {
        Self::log(
            format!("anchor {} is not on screen yet; saying the step and waiting for it", anchor.raw_value()),
            cx,
        );
    }

    pub fn anchor_scrolled(anchor: RiseGuideAnchor, cx: &mut App) {
        Self::log(format!("anchor {} was off screen; scrolled to it", anchor.raw_value()), cx);
    }

    pub fn anchor_failed(anchor: RiseGuideAnchor, registered: bool, cx: &mut App) {
        Self::log(
            if registered {
                format!("anchor {} is registered but never settled on screen", anchor.raw_value())
            } else {
                format!("anchor {} is NOT registered in this build", anchor.raw_value())
            },
            cx,
        );
    }

    pub fn anchor_moved(anchor: RiseGuideAnchor, frame: Bounds<Pixels>, cx: &mut App) {
        Self::log(format!("anchor {} moved to {}", anchor.raw_value(), Self::short(frame)), cx);
    }

    pub fn anchor_lost(anchor: RiseGuideAnchor, sighting: GuideAnchorSighting, cx: &mut App) {
        match sighting {
            GuideAnchorSighting::Covered => Self::log(
                format!(
                    "anchor {} is behind something the app put on top; standing aside, still waiting",
                    anchor.raw_value()
                ),
                cx,
            ),
            GuideAnchorSighting::Absent | GuideAnchorSighting::Visible(_) => Self::log(
                format!("anchor {} is off screen; standing aside, still waiting", anchor.raw_value()),
                cx,
            ),
        }
    }

    pub fn anchor_returned(anchor: RiseGuideAnchor, frame: Bounds<Pixels>, cx: &mut App) {
        Self::log(format!("anchor {} is visible again at {}", anchor.raw_value(), Self::short(frame)), cx);
    }

    pub fn shape_adjusted(
        anchor: RiseGuideAnchor,
        requested: RiseGuideSpotlightShape,
        drawn: RiseGuideSpotlightShape,
        cx: &mut App,
    ) {
        Self::log(
            format!(
                "anchor {} is too wide for {}; drawing {} instead",
                anchor.raw_value(),
                requested.raw_value(),
                drawn.raw_value()
            ),
            cx,
        );
    }

    pub fn moved_while_steering(key: &str, screen: &str, cx: &mut App) {
        Self::log(format!("{key} moved to {screen} while the guide was steering; not a leave"), cx);
    }

    pub fn waiting(signal: RiseGuideSignal, cx: &mut App) {
        Self::log(
            if signal.is_state() {
                format!("waiting for state {} (checked now and on every change)", signal.raw_value())
            } else {
                format!("waiting for event {} (latched from now)", signal.raw_value())
            },
            cx,
        );
    }

    pub fn condition_already_true(signal: RiseGuideSignal, cx: &mut App) {
        Self::log(format!("state {} is already true, step satisfied at once", signal.raw_value()), cx);
    }

    pub fn fact(signal: RiseGuideSignal, value: bool, cx: &mut App) {
        #[cfg(debug_assertions)]
        crate::core::guides::guide_facts::GuideFacts::note_reported(signal);
        Self::log(format!("fact {} = {}", signal.raw_value(), value), cx);
    }

    pub fn event(signal: RiseGuideSignal, waiters: i64, cx: &mut App) {
        if waiters <= 0 {
            return;
        }
        Self::log(format!("event {} delivered to {} waiter(s)", signal.raw_value(), waiters), cx);
    }

    pub fn advanced(index: i64, cx: &mut App) {
        Self::log(format!("step {} satisfied", index + 1), cx);
    }

    pub fn finished(key: &str, outcome: &str, failure: Option<&str>, cx: &mut App) {
        match failure {
            Some(failure) => Self::log(format!("end {key}: {outcome} · {failure}"), cx),
            None => Self::log(format!("end {key}: {outcome}"), cx),
        }
    }

    pub fn note(message: &str, cx: &mut App) {
        Self::log(message.to_owned(), cx);
    }

    fn advance_description(step: &RiseGuideStepModel) -> String {
        match step.advance {
            RiseGuideAdvance::Signal => {
                format!("signal:{}", step.advance_signal.map_or("?", RiseGuideSignal::raw_value))
            }
            RiseGuideAdvance::Screen => {
                format!("screen:{}", step.advance_screen.map_or("?", RiseGuideScreen::raw_value))
            }
            RiseGuideAdvance::Delay => format!("delay:{}ms", step.delay_ms),
            other => other.raw_value().to_owned(),
        }
    }

    fn short(rect: Bounds<Pixels>) -> String {
        let round = |value: Pixels| (f32::from(value).round() as i64).to_string();
        format!(
            "({},{} {}×{})",
            round(rect.origin.x),
            round(rect.origin.y),
            round(rect.size.width),
            round(rect.size.height)
        )
    }

    #[track_caller]
    fn log(message: String, cx: &mut App) {
        Sai::log(Self::TAG, move || message, SaiLogKind::Custom("guide".to_owned()), cx);
    }
}
