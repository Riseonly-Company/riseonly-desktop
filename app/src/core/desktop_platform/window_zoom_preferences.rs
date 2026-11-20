use gpui::{App, Context, Entity};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::DESKTOP_WINDOW_ZOOM_PERCENT;

pub const WINDOW_ZOOM_STEPS: [u32; 10] = [60, 70, 80, 90, 100, 110, 125, 150, 175, 200];

pub const WINDOW_ZOOM_DEFAULT: u32 = 100;

pub struct WindowZoomPreferences {
    percent: u32,
}

impl WindowZoomPreferences {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self::new())
    }

    fn new() -> Self {
        Self {
            percent: stored_zoom_percent(UserDefaults::standard().integer(DESKTOP_WINDOW_ZOOM_PERCENT)),
        }
    }

    pub fn percent(&self) -> u32 {
        self.percent
    }

    pub fn factor(&self) -> f32 {
        zoom_factor(self.percent)
    }

    pub fn zoom_in(&mut self, cx: &mut Context<Self>) -> bool {
        self.set_percent(next_zoom_step(self.percent), cx)
    }

    pub fn zoom_out(&mut self, cx: &mut Context<Self>) -> bool {
        self.set_percent(previous_zoom_step(self.percent), cx)
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) -> bool {
        self.set_percent(WINDOW_ZOOM_DEFAULT, cx)
    }

    fn set_percent(&mut self, percent: u32, cx: &mut Context<Self>) -> bool {
        if self.percent == percent {
            return false;
        }
        self.percent = percent;
        UserDefaults::standard().set(DESKTOP_WINDOW_ZOOM_PERCENT, i64::from(percent));
        cx.notify();
        true
    }
}

pub fn next_zoom_step(percent: u32) -> u32 {
    WINDOW_ZOOM_STEPS.iter().copied().find(|step| *step > percent).unwrap_or(WINDOW_ZOOM_STEPS[WINDOW_ZOOM_STEPS.len() - 1])
}

pub fn previous_zoom_step(percent: u32) -> u32 {
    WINDOW_ZOOM_STEPS.iter().rev().copied().find(|step| *step < percent).unwrap_or(WINDOW_ZOOM_STEPS[0])
}

pub fn stored_zoom_percent(stored: i64) -> u32 {
    let lowest = WINDOW_ZOOM_STEPS[0];
    let highest = WINDOW_ZOOM_STEPS[WINDOW_ZOOM_STEPS.len() - 1];
    u32::try_from(stored)
        .ok()
        .filter(|percent| (lowest..=highest).contains(percent))
        .and_then(|percent| WINDOW_ZOOM_STEPS.iter().copied().min_by_key(|step| (step.abs_diff(percent), *step < percent)))
        .unwrap_or(WINDOW_ZOOM_DEFAULT)
}

pub fn zoom_factor(percent: u32) -> f32 {
    percent as f32 / 100.0
}
