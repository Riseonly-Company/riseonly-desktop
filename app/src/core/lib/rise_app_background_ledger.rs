use gpui::{App, Global};
use rise_core::foundation::foundation_date::Date;

/// tells a foreground handler whether the app really left the foreground
/// window focus moves dont count, only all hidden or minimised or system asleep
/// first activation counts as a return
pub enum RiseAppBackgroundLedger {}

struct RiseAppBackgroundLedgerState {
    background_generation: u64,
    entered_background_at: Option<Date>,
}

impl Default for RiseAppBackgroundLedgerState {
    fn default() -> Self {
        Self {
            background_generation: 1,
            entered_background_at: None,
        }
    }
}

impl Global for RiseAppBackgroundLedgerState {}

impl RiseAppBackgroundLedger {
    pub fn background_generation(cx: &App) -> u64 {
        cx.try_global::<RiseAppBackgroundLedgerState>()
            .map_or(1, |state| state.background_generation)
    }

    pub fn entered_background_at(cx: &App) -> Option<Date> {
        cx.try_global::<RiseAppBackgroundLedgerState>()
            .and_then(|state| state.entered_background_at)
    }

    pub fn note_entered_background(now: Date, cx: &mut App) {
        let state = cx.default_global::<RiseAppBackgroundLedgerState>();
        state.background_generation = state.background_generation.wrapping_add(1);
        state.entered_background_at = Some(now);
    }

    /// true once per trip through the background for the caller owning handled_generation
    /// and true on its first call ever
    pub fn consume_return_from_background(handled_generation: &mut Option<u64>, cx: &App) -> bool {
        let background_generation = Self::background_generation(cx);
        if *handled_generation == Some(background_generation) {
            return false;
        }
        *handled_generation = Some(background_generation);
        true
    }
}
