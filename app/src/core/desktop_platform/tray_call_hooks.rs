use gpui::{App, Subscription};

use crate::core::feature_hooks::FeatureHookSlot;

/// call the status icon menu acts on, none outside a call
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TrayCallState {
    pub microphone_enabled: bool,
    pub can_use_microphone: bool,
    pub deafened: bool,
}

/// what the status icon menu needs from the calls
pub struct TrayCallHooks {
    pub state: fn(&mut App) -> Option<TrayCallState>,
    pub observe: fn(Box<dyn Fn(&mut App)>, &mut App) -> Vec<Subscription>,
    pub toggle_microphone: fn(&mut App),
    pub toggle_deafened: fn(&mut App),
    pub open_call: fn(&mut App),
    pub leave_call: fn(&mut App),
}

pub(crate) static TRAY_CALL_HOOKS: FeatureHookSlot<TrayCallHooks> = FeatureHookSlot::new("TRAY_CALL_HOOKS");

pub fn set_tray_call_hooks(hooks: TrayCallHooks) {
    let _ = TRAY_CALL_HOOKS.set(hooks);
}

pub fn tray_call_hooks() -> Option<&'static TrayCallHooks> {
    TRAY_CALL_HOOKS.get()
}
