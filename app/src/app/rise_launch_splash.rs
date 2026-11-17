use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use gpui::{App, Context, ElementId, Entity, ImageSource, ObjectFit, RenderImage, Subscription, Task, Window, img};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;

use crate::app::launch_snapshot_tint::LaunchSnapshotTint;
use crate::core::animations::Animation;
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::ui::lottie_sticker::rise_lottie_frame_cache::{
    RiseLottieFrameSequence, RiseLottieRasterPolicy, RiseLottieSequenceKey,
};

const LAUNCH_ASSET: &str = "riseonly-launch";
const LOGO_SIDE: f32 = 220.0;
const FADE_OUT_SECONDS: f64 = 0.4;
const ANIMATION_SPEED: f64 = 1.35;
const STILL_FRAME: i64 = 130;
const STILL_FRAME_HOLD: Duration = Duration::from_millis(200);
const STILL_FRAME_POLL: Duration = Duration::from_millis(16);
const PREFETCH_AHEAD: i64 = 2;

struct LaunchSplashVisibleKey;

impl EnvKey for LaunchSplashVisibleKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        false
    }
}

pub fn is_launch_splash_visible(cx: &App) -> bool {
    env::<LaunchSplashVisibleKey>(cx)
}

static IS_VISIBLE: AtomicBool = AtomicBool::new(false);

/// launch splash state for code outside the view tree
/// the guide must not show the panda over the launch art that hides the tabs
pub enum RiseLaunchSplashState {}

impl RiseLaunchSplashState {
    pub fn is_visible() -> bool {
        IS_VISIBLE.load(Ordering::Relaxed)
    }

    /// true when the value changed
    fn update(visible: bool) -> bool {
        IS_VISIBLE.swap(visible, Ordering::Relaxed) != visible
    }
}

#[derive(IntoElement)]
pub struct RiseLaunchSplashBoundary {
    id: ElementId,
    is_presented: Binding<bool>,
    content: AnyElement,
}

impl RiseLaunchSplashBoundary {
    pub fn new(id: impl Into<ElementId>, is_presented: Binding<bool>, content: impl IntoElement) -> Self {
        Self { id: id.into(), is_presented, content: content.into_any_element() }
    }
}

struct RiseLaunchSplashBoundaryState {
    is_presented: Binding<bool>,
    is_fading_out: bool,
    opacity: Animated<f32>,
    animation: Option<Entity<RiseLaunchSplashAnimation>>,
    fade_task: Option<Task<()>>,
    _keystroke_gate: Subscription,
}

impl RiseLaunchSplashBoundaryState {
    fn new(is_presented: Binding<bool>, cx: &mut Context<Self>) -> Self {
        let keystroke_gate = cx.intercept_keystrokes(|_, _, cx| {
            if RiseLaunchSplashState::is_visible() {
                cx.stop_propagation();
            }
        });
        Self {
            is_presented,
            is_fading_out: false,
            opacity: Animated::new(1.0),
            animation: None,
            fade_task: None,
            _keystroke_gate: keystroke_gate,
        }
    }

    fn splash_animation(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<RiseLaunchSplashAnimation> {
        if let Some(animation) = &self.animation {
            return animation.clone();
        }
        let state = cx.weak_entity();
        let on_completion: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(move |window: &mut Window, cx: &mut App| {
            let _ = state.update(cx, |state, cx| state.dismiss_splash(window, cx));
        });
        let animation = cx.new(|cx| RiseLaunchSplashAnimation::new(on_completion, window, cx));
        self.animation = Some(animation.clone());
        animation
    }

    fn dismiss_splash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_fading_out {
            return;
        }
        self.is_fading_out = true;
        self.opacity.set(0.0, ease_out(FADE_OUT_SECONDS), RiseClock::monotonic(), false);
        cx.notify();
        let is_presented = self.is_presented.clone();
        self.fade_task = Some(cx.spawn_in(window, async move |_, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(FADE_OUT_SECONDS)).await;
            let _ = cx.update(|window, cx| {
                if is_presented.get(cx) {
                    is_presented.set(false, window, cx);
                }
            });
        }));
    }
}

impl RenderOnce for RiseLaunchSplashBoundary {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_presented = self.is_presented.get(cx);
        let binding = self.is_presented.clone();
        let state = window.use_keyed_state(self.id.clone(), cx, move |_, cx| RiseLaunchSplashBoundaryState::new(binding, cx));
        if RiseLaunchSplashState::update(is_presented) {
            cx.defer(|cx: &mut App| cx.refresh_windows());
        }

        let animation = state.update(cx, |state, cx| {
            if is_presented {
                Some(state.splash_animation(window, cx))
            } else {
                state.animation = None;
                None
            }
        });

        let now = RiseClock::monotonic();
        let opacity = {
            let state = state.read(cx);
            if state.opacity.is_running(now) {
                window.request_animation_frame();
            }
            state.opacity.value(now)
        };
        let background = LaunchSnapshotTint::launch_background(cx);

        div()
            .relative()
            .size_full()
            .child(env_scope::<LaunchSplashVisibleKey>(is_presented).child(self.content))
            .when_some(animation, |root, animation| {
                root.child(
                    div()
                        .absolute()
                        .inset_0()
                        .occlude()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(background)
                        .opacity(opacity)
                        .child(div().size(px(LOGO_SIDE)).child(animation)),
                )
            })
    }
}

struct RiseLaunchSplashAnimation {
    view: Option<Entity<LaunchLogoAnimationView>>,
    on_completion: Rc<dyn Fn(&mut Window, &mut App)>,
    _load: Task<()>,
    _scene_phase: Subscription,
}

fn plays_in(phase: ScenePhase) -> bool {
    phase != ScenePhase::Background
}

impl RiseLaunchSplashAnimation {
    fn new(on_completion: Rc<dyn Fn(&mut Window, &mut App)>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let scene_phase = cx.observe_global_in::<RiseAppLifecycle>(window, |this, window, cx| {
            let is_active = plays_in(RiseAppLifecycle::scene_phase(cx));
            if let Some(view) = this.view.clone() {
                view.update(cx, |view, cx| {
                    view.is_active = is_active;
                    view.start_if_ready(window, cx);
                });
            }
            cx.notify();
        });
        let source = cx.asset_source().clone();
        let load = cx.spawn_in(window, async move |this, cx| {
            let json = cx
                .background_executor()
                .spawn(async move {
                    source
                        .load(&Animation::asset_path(LAUNCH_ASSET))
                        .ok()
                        .flatten()
                        .map(Arc::<[u8]>::from)
                })
                .await;
            let loaded = match json {
                Some(json) => {
                    let key = RiseLottieSequenceKey {
                        cache_key: LAUNCH_ASSET.to_owned(),
                        dimension: RiseLottieRasterPolicy::dimension(LOGO_SIDE),
                        mirrored: false,
                    };
                    RiseLottieFrameSequence::make(key, json, cx).await
                }
                None => None,
            };
            let _ = this.update_in(cx, |this, window, cx| match loaded {
                Some(sequence) => {
                    this.view = Some(cx.new(|_| LaunchLogoAnimationView::new(sequence)));
                    cx.notify();
                }
                None => {
                    let on_completion = this.on_completion.clone();
                    on_completion(window, cx);
                }
            });
        });
        Self { view: None, on_completion, _load: load, _scene_phase: scene_phase }
    }
}

impl Render for RiseLaunchSplashAnimation {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_active = plays_in(RiseAppLifecycle::scene_phase(cx));
        let uses_still_frame = reduce_motion(cx);
        let on_completion = self.on_completion.clone();
        div().size_full().when_some(self.view.clone(), |root, view| {
            root.child(LaunchLogoPlayer { view, is_active, uses_still_frame, on_completion })
        })
    }
}

#[derive(IntoElement)]
struct LaunchLogoPlayer {
    view: Entity<LaunchLogoAnimationView>,
    is_active: bool,
    uses_still_frame: bool,
    on_completion: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl RenderOnce for LaunchLogoPlayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.view.update(cx, |view, cx| {
            view.on_completion = Some(self.on_completion);
            view.is_active = self.is_active;
            view.uses_still_frame = self.uses_still_frame;
            view.start_if_ready(window, cx);
        });
        div().size_full().child(self.view)
    }
}

struct LaunchLogoAnimationView {
    sequence: Arc<RiseLottieFrameSequence>,
    on_completion: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    is_active: bool,
    uses_still_frame: bool,
    play_task: Option<Task<()>>,
    still_frame_task: Option<Task<()>>,
    has_finished: bool,
    current_frame: f64,
    shown_frame: Option<i64>,
    image: Option<Arc<RenderImage>>,
    requested_through: i64,
    playhead: Arc<AtomicI64>,
}

impl LaunchLogoAnimationView {
    fn new(sequence: Arc<RiseLottieFrameSequence>) -> Self {
        Self {
            sequence,
            on_completion: None,
            is_active: false,
            uses_still_frame: false,
            play_task: None,
            still_frame_task: None,
            has_finished: false,
            current_frame: 0.0,
            shown_frame: None,
            image: None,
            requested_through: -1,
            playhead: Arc::new(AtomicI64::new(0)),
        }
    }

    fn last_frame(&self) -> i64 {
        (self.sequence.frame_count - 1).max(0)
    }

    fn start_if_ready(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_finished {
            return;
        }
        if !self.is_active {
            self.play_task = None;
            self.still_frame_task = None;
            return;
        }
        if self.uses_still_frame {
            if self.still_frame_task.is_some() {
                return;
            }
            self.play_task = None;
            let still_frame = STILL_FRAME.min(self.last_frame());
            self.still_frame_task = Some(cx.spawn_in(window, async move |this, cx| {
                let mut waited = Duration::ZERO;
                while waited < STILL_FRAME_HOLD {
                    if this.update(cx, |this, cx| this.show_frame(still_frame, cx)).is_err() {
                        return;
                    }
                    cx.background_executor().timer(STILL_FRAME_POLL).await;
                    waited += STILL_FRAME_POLL;
                }
                let _ = this.update_in(cx, |this, window, cx| this.finish(window, cx));
            }));
        } else if self.play_task.is_none() && self.still_frame_task.is_none() {
            self.play(window, cx);
        }
    }

    fn play(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tick = Duration::from_secs_f64(1.0 / self.sequence.frame_rate.max(1.0));
        self.play_task = Some(cx.spawn_in(window, async move |this, cx| {
            let mut last_tick = RiseClock::monotonic();
            loop {
                let now = RiseClock::monotonic();
                let elapsed = now.saturating_sub(last_tick);
                last_tick = now;
                let keeps_playing = this.update_in(cx, |this, window, cx| this.advance(elapsed, window, cx));
                if !matches!(keeps_playing, Ok(true)) {
                    return;
                }
                cx.background_executor().timer(tick).await;
            }
        }));
    }

    fn advance(&mut self, elapsed: Duration, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let last_frame = self.last_frame();
        self.current_frame += elapsed.as_secs_f64() * self.sequence.frame_rate * ANIMATION_SPEED;
        self.show_frame((self.current_frame as i64).min(last_frame), cx);
        if self.current_frame as i64 >= last_frame {
            self.finish(window, cx);
            return false;
        }
        true
    }

    fn show_frame(&mut self, at: i64, cx: &mut Context<Self>) {
        self.playhead.store(at, Ordering::Relaxed);
        if self.shown_frame != Some(at) {
            match self.sequence.cached_frame(at) {
                Some(image) => {
                    self.shown_frame = Some(at);
                    self.image = Some(image);
                    cx.notify();
                }
                None => self.request_frame(at),
            }
        }
        for ahead in 1..=PREFETCH_AHEAD {
            let next = at + ahead;
            if next <= self.last_frame() && self.sequence.cached_frame(next).is_none() {
                self.request_frame(next);
            }
        }
    }

    fn request_frame(&mut self, at: i64) {
        if at <= self.requested_through {
            return;
        }
        self.requested_through = at;
        let playhead = Arc::clone(&self.playhead);
        self.sequence.request_frame(at, Arc::new(move || at >= playhead.load(Ordering::Relaxed)), None);
    }

    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_finished {
            return;
        }
        self.has_finished = true;
        if let Some(on_completion) = self.on_completion.clone() {
            window.defer(cx, move |window, cx| on_completion(window, cx));
        }
    }
}

impl Drop for LaunchLogoAnimationView {
    fn drop(&mut self) {
        self.playhead.store(i64::MAX, Ordering::Relaxed);
    }
}

impl Render for LaunchLogoAnimationView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().when_some(self.image.clone(), |root, image| {
            root.child(img(ImageSource::Render(image)).size_full().object_fit(ObjectFit::Contain))
        })
    }
}

#[cfg(test)]
mod launch_phase_tests {
    use super::*;

    #[test]
    fn plays_while_the_window_is_on_the_screen_and_pauses_off_it() {
        assert!(plays_in(ScenePhase::Active));
        assert!(plays_in(ScenePhase::Inactive));
        assert!(!plays_in(ScenePhase::Background));
    }
}
