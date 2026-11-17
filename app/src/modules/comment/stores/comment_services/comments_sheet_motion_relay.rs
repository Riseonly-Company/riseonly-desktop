use std::rc::Rc;

use gpui::{App, Bounds, Context, Entity, Pixels, Point, px};

struct CommentsSheetMotionRelayBackdropControl {
    post_id: String,
    frame: Bounds<Pixels>,
    action: Rc<dyn Fn(&mut App)>,
}

pub struct CommentsSheetMotionRelay {
    engaged_post_id: Option<String>,
    progress: f32,
    sheet_fraction: f32,
    sheet_frame: Option<Bounds<Pixels>>,
    resting_height: f32,
    backdrop_control: Option<CommentsSheetMotionRelayBackdropControl>,
}

impl CommentsSheetMotionRelay {
    const EPSILON: f32 = 0.002;
    const BACKDROP_CONTROL_SLOP: f32 = 8.0;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { engaged_post_id: None, progress: 0.0, sheet_fraction: 0.0, sheet_frame: None, resting_height: 1.0, backdrop_control: None }
    }

    pub fn engaged_post_id(&self) -> Option<&str> {
        self.engaged_post_id.as_deref()
    }

    pub fn progress(&self) -> f32 {
        self.progress
    }

    pub fn sheet_fraction(&self) -> f32 {
        self.sheet_fraction
    }

    /// live sheet frame in window coordinates
    pub fn sheet_frame(&self) -> Option<Bounds<Pixels>> {
        self.sheet_frame
    }

    pub fn set_backdrop_control(&mut self, post_id: &str, frame: Bounds<Pixels>, action: Rc<dyn Fn(&mut App)>) {
        self.backdrop_control =
            Some(CommentsSheetMotionRelayBackdropControl { post_id: post_id.to_owned(), frame, action });
    }

    pub fn clear_backdrop_control(&mut self, post_id: &str) {
        if self.backdrop_control.as_ref().is_some_and(|control| control.post_id == post_id) {
            self.backdrop_control = None;
        }
    }

    pub fn handle_backdrop_tap(&mut self, point: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some(control) = &self.backdrop_control else {
            return false;
        };
        if self.engaged_post_id.as_deref() != Some(control.post_id.as_str())
            || self.progress <= 0.5
            || !control.frame.dilate(px(Self::BACKDROP_CONTROL_SLOP)).contains(&point)
        {
            return false;
        }
        let action = control.action.clone();
        cx.defer(move |cx| action(cx));
        true
    }

    pub fn engage(&mut self, post_id: &str, resting_fraction: f32, window_height: Pixels, cx: &mut Context<Self>) {
        let window_height = f32::from(window_height).max(1.0);
        self.resting_height = (window_height * resting_fraction).max(1.0);
        if self.engaged_post_id.as_deref() != Some(post_id) {
            self.engaged_post_id = Some(post_id.to_owned());
            cx.notify();
        }
    }

    pub fn update(&mut self, sheet_frame: Option<Bounds<Pixels>>, window_height: Pixels, cx: &mut Context<Self>) {
        let Some(sheet_frame) = sheet_frame else {
            self.disengage(cx);
            return;
        };
        let window_height = f32::from(window_height).max(1.0);
        let visible = (window_height - f32::from(sheet_frame.origin.y)).max(0.0);
        let next_fraction = (visible / window_height).min(1.0);
        let mut changed = false;
        if self.sheet_frame != Some(sheet_frame) {
            self.sheet_frame = Some(sheet_frame);
            changed = true;
        }
        if (next_fraction - self.sheet_fraction).abs() > Self::EPSILON
            || (next_fraction == 0.0 && self.sheet_fraction != 0.0)
            || (next_fraction == 1.0 && self.sheet_fraction != 1.0)
        {
            self.sheet_fraction = next_fraction;
            changed = true;
        }
        let next = (visible / self.resting_height).min(1.0);
        if (next - self.progress).abs() > Self::EPSILON
            || (next == 0.0 && self.progress != 0.0)
            || (next == 1.0 && self.progress != 1.0)
        {
            self.progress = next;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    pub fn disengage(&mut self, cx: &mut Context<Self>) {
        let changed = self.progress != 0.0 || self.sheet_fraction != 0.0 || self.engaged_post_id.is_some();
        self.progress = 0.0;
        self.sheet_fraction = 0.0;
        self.sheet_frame = None;
        self.engaged_post_id = None;
        self.backdrop_control = None;
        if changed {
            cx.notify();
        }
    }
}
