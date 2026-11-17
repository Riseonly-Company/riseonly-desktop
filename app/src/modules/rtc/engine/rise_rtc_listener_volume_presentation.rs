use std::collections::HashMap;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;

/// per listener volume rail, 0 is the mute for me end of it so roster label
/// menu row and knob cant disagree about audibility
pub enum RiseRTCListenerVolume {}

impl RiseRTCListenerVolume {
    pub const MINIMUM_PERCENT: f64 = 0.0;
    pub const MAXIMUM_PERCENT: f64 = 200.0;
    pub const DEFAULT_PERCENT: f64 = 100.0;
    pub const STEP_PERCENT: f64 = 5.0;
    pub const BREAKPOINTS: [f64; 3] = [0.0, 100.0, 200.0];

    /// explicit choice or else 0 if totally blocked and 100 if not
    /// explicit outranks the block and the derived zero
    /// is never stored so unblocking restores audibility
    pub fn resolved_percent(override_percent: Option<f64>, is_totally_blocked: bool) -> f64 {
        if let Some(override_percent) = override_percent {
            return Self::clamped(override_percent);
        }
        if is_totally_blocked { Self::MINIMUM_PERCENT } else { Self::DEFAULT_PERCENT }
    }

    pub fn clamped(percent: f64) -> f64 {
        if !percent.is_finite() {
            return Self::DEFAULT_PERCENT;
        }
        percent.max(Self::MINIMUM_PERCENT).min(Self::MAXIMUM_PERCENT)
    }

    /// snaps onto the rail stops so a value from anywhere
    /// but the slider still lands where the knob can be drawn
    pub fn normalized(percent: f64) -> f64 {
        let value = Self::clamped(percent);
        Self::clamped((value / Self::STEP_PERCENT).round() * Self::STEP_PERCENT)
    }

    pub fn is_muted(percent: f64) -> bool {
        Self::clamped(percent) <= Self::MINIMUM_PERCENT
    }

    /// remote audio track volume is a plain multiplier with DEFAULT_PERCENT as unity
    pub fn gain(for_percent: f64) -> f64 {
        Self::clamped(for_percent) / Self::DEFAULT_PERCENT
    }

    pub fn percent_label(percent: f64) -> String {
        format!("{}%", Self::clamped(percent).round() as i64)
    }
}

/// explicit per listener choices only, the rest is derived at use
/// since the block tier changes without this snapshot changing
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RiseRTCListenerVolumeSnapshot {
    pub overrides: HashMap<String, f64>,
    pub is_loaded: bool,
}

impl RiseRTCListenerVolumeSnapshot {
    pub fn empty() -> Self {
        Self { overrides: HashMap::new(), is_loaded: false }
    }

    pub fn override_for(&self, user_id: &str) -> Option<f64> {
        self.overrides.get(user_id).copied()
    }
}

/// publish gate, a snapshot whose account
/// epoch or session no longer matches is dropped so a write
/// that outlived a logout cant repaint the next account room
pub struct RiseRTCListenerVolumePresentation {
    snapshot: RiseRTCListenerVolumeSnapshot,
    account_id: Option<String>,
    account_epoch: Option<String>,
    session_id: Option<Uuid>,
}

impl RiseRTCListenerVolumePresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { snapshot: RiseRTCListenerVolumeSnapshot::empty(), account_id: None, account_epoch: None, session_id: None }
    }

    pub fn snapshot(&self) -> &RiseRTCListenerVolumeSnapshot {
        &self.snapshot
    }

    pub fn activate(&mut self, account_id: &str, account_epoch: &str, session_id: Uuid, cx: &mut Context<Self>) {
        self.account_id = Some(account_id.to_owned());
        self.account_epoch = Some(account_epoch.to_owned());
        self.session_id = Some(session_id);
        self.snapshot = RiseRTCListenerVolumeSnapshot::empty();
        cx.notify();
    }

    pub fn publish(
        &mut self,
        next: RiseRTCListenerVolumeSnapshot,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.account_id.as_deref() != Some(account_id)
            || self.account_epoch.as_deref() != Some(account_epoch)
            || self.session_id != Some(session_id)
        {
            return false;
        }
        if next == self.snapshot {
            return true;
        }
        self.snapshot = next;
        cx.notify();
        true
    }

    pub fn deactivate(&mut self, account_id: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.account_id.as_deref() != Some(account_id) || self.session_id != Some(session_id) {
            return;
        }
        self.account_id = None;
        self.account_epoch = None;
        self.session_id = None;
        self.snapshot = RiseRTCListenerVolumeSnapshot::empty();
        cx.notify();
    }
}
