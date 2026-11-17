use std::collections::HashMap;

use gpui::{App, Empty, IntoElement, Pixels, RenderOnce, Window};
use rise_ui::bottom_sheet_ui::bottom_sheet_motion_host::BottomSheetMotionDiagnostics;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, use_task};

use crate::modules::user::components::profile_content::profile_performance_trace::ProfilePerformanceTrace;
use crate::modules::user::components::profile_content::profile_performance_trace_control::ProfilePerformanceTraceControl;

/// call screen handle on the profile performance trace, free form scope
/// recorder drops every event while off
/// nothing observes the tracer so recording from render invalidates nothing
pub enum RiseRTCCallTrace {}

impl RiseRTCCallTrace {
    pub const SCOPE: &'static str = "rtc-call";

    pub fn is_available() -> bool {
        ProfilePerformanceTrace::is_control_available()
    }

    pub fn is_recording() -> bool {
        ProfilePerformanceTrace::shared().is_recording(Self::SCOPE)
    }

    pub fn note_body_pass(state: &str, has_remote_surface: bool, floating_windows: usize) {
        ProfilePerformanceTrace::shared().record(
            "rtc.body",
            HashMap::from([
                ("state".to_owned(), state.to_owned()),
                ("remote_surface".to_owned(), flag(has_remote_surface)),
                ("floating".to_owned(), floating_windows.to_string()),
            ]),
        );
    }

    /// one line per video surface pass, the count matters even if the view looks static
    pub fn note_video_pass(role: &str) {
        ProfilePerformanceTrace::shared().record("rtc.video", HashMap::from([("role".to_owned(), role.to_owned())]));
    }

    pub fn note_media_mutation(name: &str, enabled: bool) {
        ProfilePerformanceTrace::shared().record(
            "rtc.media",
            HashMap::from([("control".to_owned(), name.to_owned()), ("enabled".to_owned(), flag(enabled))]),
        );
    }

    pub fn begin_observing_sheet_motion() {
        if !Self::is_available() {
            return;
        }
        BottomSheetMotionDiagnostics::set_on_interactive_translation(Some(|translation_y: Pixels| {
            ProfilePerformanceTrace::shared().record(
                "rtc.sheet.frame",
                HashMap::from([("translation_y".to_owned(), format!("{:.1}", f32::from(translation_y)))]),
            );
        }));
        BottomSheetMotionDiagnostics::set_on_interaction_phase(Some(|phase: &str| {
            ProfilePerformanceTrace::shared().record(&format!("rtc.sheet.{phase}"), HashMap::new());
        }));
    }

    pub fn end_observing_sheet_motion() {
        BottomSheetMotionDiagnostics::set_on_interactive_translation(None::<fn(Pixels)>);
        BottomSheetMotionDiagnostics::set_on_interaction_phase(None::<fn(&str)>);
    }
}

fn flag(value: bool) -> String {
    if value { "1" } else { "0" }.to_owned()
}

/// recorder control in the call chrome, renders nothing
/// outside debug and profiling builds so users never see it
#[derive(IntoElement, Default)]
pub struct RiseRTCCallTraceControl;

impl RenderOnce for RiseRTCCallTraceControl {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if !RiseRTCCallTrace::is_available() {
            return Empty.into_any_element();
        }
        use_task("rise-rtc-call-trace-control-task", (), window, cx, async |_cx| {
            RiseRTCCallTrace::begin_observing_sheet_motion();
        });
        ProfilePerformanceTraceControl::new(RiseRTCCallTrace::SCOPE)
            .on_disappear("rise-rtc-call-trace-control", |_window, _cx| {
                RiseRTCCallTrace::end_observing_sheet_motion();
            })
            .into_any_element()
    }
}
