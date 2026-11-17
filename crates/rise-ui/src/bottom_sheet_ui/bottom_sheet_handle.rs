use gpui::{App, IntoElement, RenderOnce, Window, white};

use crate::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use crate::swiftui::prelude::*;

#[derive(IntoElement, Clone, Copy, Debug, Default)]
pub struct BottomSheetHandle {
    pub prominent_on_dark: bool,
}

impl BottomSheetHandle {
    pub fn new(prominent_on_dark: bool) -> Self {
        Self { prominent_on_dark }
    }
}

impl RenderOnce for BottomSheetHandle {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = crate::theme(cx);
        let fill = if self.prominent_on_dark {
            white().opacity(0.38)
        } else {
            theme.border._400.opacity(0.85)
        };

        vstack(0.0)
            .child(div().h(px(BottomSheetMetrics::HANDLE_TOP_PADDING)))
            .child(
                div()
                    .capsule()
                    .bg(fill)
                    .frame_size(
                        Some(BottomSheetMetrics::HANDLE_WIDTH),
                        Some(BottomSheetMetrics::HANDLE_HEIGHT),
                    ),
            )
            .child(div().h(px(BottomSheetMetrics::HANDLE_BOTTOM_PADDING)))
            .frame_max_width(Alignment::Center)
    }
}
