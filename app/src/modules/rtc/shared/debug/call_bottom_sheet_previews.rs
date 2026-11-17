#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationMode, BottomSheetDebugRegistrationModule,
};
use crate::modules::rtc::media::rise_rtc_audio_settings::RiseRTCAudioSettingsContent;

pub enum CallBottomSheetPreviews {}

impl CallBottomSheetPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        vec![
            BottomSheetDebugRegistration::sheet(
                "calls.audio",
                BottomSheetDebugRegistrationModule::Calls,
                "rtc_voice_processing",
                "RiseRTCAudioSettingsContent",
                vec![0.6, 0.86],
                None,
                |_, _, _| {
                    scroll_view("calls-audio-preview-scroll")
                        .child(div().py(px(20.0)).child(RiseRTCAudioSettingsContent))
                },
            )
            .with_mode(BottomSheetDebugRegistrationMode::Live),
        ]
    }
}
