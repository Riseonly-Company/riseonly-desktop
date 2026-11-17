use std::time::Duration;

use gpui::{App, Bounds, Pixels};
use rise_core::foundation::foundation_date::RiseClock;

use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;

pub enum RiseListRemovalDiag {}

impl RiseListRemovalDiag {
    const ACTIVE_WINDOW: Duration = Duration::from_secs(4);

    pub fn is_active(cx: &mut App) -> bool {
        let restored_at = HoldContextMenuPresenter::shared(cx).read(cx).source_restored_at();
        !restored_at.is_zero() && RiseClock::monotonic().saturating_sub(restored_at) < Self::ACTIVE_WINDOW
    }

    pub fn log(message: impl FnOnce() -> String, if_active: bool, cx: &mut App) {
        if if_active && !Self::is_active(cx) {
            return;
        }
        let text = message();
        tracing::info!(target: "ListRemovalDiag", "[f9d9] {text}");
    }

    pub fn f(value: Option<Pixels>) -> String {
        match value {
            Some(value) => format!("{:.1}", f32::from(value)),
            None => "nil".to_string(),
        }
    }

    pub fn f_rect(rect: Option<Bounds<Pixels>>) -> String {
        match rect {
            Some(rect) => format!(
                "({:.1},{:.1} {:.1}x{:.1})",
                f32::from(rect.origin.x),
                f32::from(rect.origin.y),
                f32::from(rect.size.width),
                f32::from(rect.size.height)
            ),
            None => "nil".to_string(),
        }
    }
}
