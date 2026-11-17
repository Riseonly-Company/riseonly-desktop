use std::collections::VecDeque;

use gpui::{
    App, Context, Global, IntoElement, ParentElement, Render, RenderOnce, SharedString, Styled,
    Task, WeakEntity, Window, div,
};
use rise_canon::RiseGuidePose;
use rise_core::foundation::foundation_date::RiseClock;

use crate::guide_overlay_ui::guide_overlay_theme::{GuideOverlayMotion, GuideOverlayTheme};
use crate::lottie_view::{LottieView, LottieViewModel};
use crate::swiftui::animated::Animated;
use crate::swiftui::motion::{MotionSpec, duration_from_seconds, ease_in_out};
use crate::swiftui::transition::TransitionModifiers;

/// the red panda
/// keeps one stage for its whole life and swaps the parsed animation in, parsing off the main thread
#[derive(IntoElement)]
pub struct GuideNarratorView {
    pub pose: RiseGuidePose,
    pub is_present: bool,
    pub motion: GuideOverlayMotion,
}

impl GuideNarratorView {
    pub fn new(pose: RiseGuidePose, is_present: bool, motion: GuideOverlayMotion) -> Self {
        Self {
            pose,
            is_present,
            motion,
        }
    }
}

impl RenderOnce for GuideNarratorView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let stage = window.use_keyed_state("guide_narrator_stage", cx, |_, cx| GuideNarratorStageView::new(cx));
        let (pose, is_present, motion) = (self.pose, self.is_present, self.motion);
        stage.update(cx, |stage, cx| stage.update(pose, is_present, motion, cx));
        stage
    }
}

/// the one stage that plays right now
#[derive(Default)]
struct GuideNarratorActivePlayer(Option<WeakEntity<GuideNarratorStageView>>);

impl Global for GuideNarratorActivePlayer {}

pub struct GuideNarratorStageView {
    requested_pose: Option<RiseGuidePose>,
    load_generation: u64,
    presence: Option<bool>,
    plays_animation: bool,
    installed_pose: Option<RiseGuidePose>,
    outgoing_pose: Option<RiseGuidePose>,
    crossfade: Animated<f32>,
    is_playing: bool,
    alpha: Animated<f32>,
    scale: Animated<f32>,
    _load: Task<()>,
    _presence: Task<()>,
    _release: gpui::Subscription,
}

impl GuideNarratorStageView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let release = cx.on_release(|stage, cx| stage.teardown(cx));
        Self {
            requested_pose: None,
            load_generation: 0,
            presence: None,
            plays_animation: true,
            installed_pose: None,
            outgoing_pose: None,
            crossfade: Animated::new(1.0),
            is_playing: false,
            alpha: Animated::new(0.0),
            scale: Animated::new(GuideOverlayTheme::NARRATOR_ENTER_SCALE),
            _load: Task::ready(()),
            _presence: Task::ready(()),
            _release: release,
        }
    }

    pub fn update(
        &mut self,
        pose: RiseGuidePose,
        is_present: bool,
        motion: GuideOverlayMotion,
        cx: &mut Context<Self>,
    ) {
        self.plays_animation = motion.plays_narrator();

        if self.requested_pose != Some(pose) {
            let is_first_pose = self.requested_pose.is_none();
            self.requested_pose = Some(pose);
            self.load_pose(pose, !is_first_pose && motion.animates_transitions(), cx);
        }

        self.set_presence(is_present, motion.animates_transitions(), cx);
        self.apply_playback(cx);
    }

    fn teardown(&mut self, cx: &mut App) {
        self.load_generation = self.load_generation.wrapping_add(1);
        let active = cx.default_global::<GuideNarratorActivePlayer>();
        if active.0.as_ref().is_some_and(|player| player.upgrade().is_none()) {
            active.0 = None;
        }
        self.installed_pose = None;
        self.outgoing_pose = None;
        self.is_playing = false;
        self._load = Task::ready(());
        self._presence = Task::ready(());
    }

    fn set_presence(&mut self, present: bool, animated: bool, cx: &mut Context<Self>) {
        if self.presence == Some(present) {
            return;
        }
        self.presence = Some(present);

        let scale = if present { 1.0 } else { GuideOverlayTheme::NARRATOR_ENTER_SCALE };
        let alpha = if present { 1.0 } else { 0.0 };

        if !animated {
            self.alpha.jump(alpha);
            self.scale.jump(scale);
            self._presence = Task::ready(());
            self.apply_playback(cx);
            cx.notify();
            return;
        }

        if present {
            self.apply_playback(cx);
        }
        let now = RiseClock::monotonic();
        let spec: MotionSpec = ease_in_out(GuideOverlayTheme::NARRATOR_TRANSITION);
        self.alpha.set(alpha, spec, now, false);
        self.scale.set(scale, spec, now, false);
        cx.notify();
        let duration = duration_from_seconds(GuideOverlayTheme::NARRATOR_TRANSITION);
        self._presence = cx.spawn(async move |stage, cx| {
            cx.background_executor().timer(duration).await;
            let _ = stage.update(cx, |stage, cx| {
                if stage.presence == Some(false) {
                    stage.apply_playback(cx);
                }
            });
        });
    }

    fn load_pose(&mut self, pose: RiseGuidePose, crossfades: bool, cx: &mut Context<Self>) {
        self.load_generation = self.load_generation.wrapping_add(1);
        let generation = self.load_generation;
        let path = Self::animation_path(pose);

        if GuideNarratorAnimationLoader::cached(&path, cx).is_some() {
            self.install(pose, crossfades, cx);
            return;
        }
        let loading = GuideNarratorAnimationLoader::load(&path, cx);
        self._load = cx.spawn(async move |stage, cx| {
            let animation = loading.await;
            let _ = stage.update(cx, |stage, cx| {
                if stage.load_generation != generation || animation.is_none() {
                    return;
                }
                stage.install(pose, crossfades, cx);
            });
        });
    }

    /// asset name of a pose, searched in the red panda folder first and then the assets root
    pub fn animation_path(pose: RiseGuidePose) -> SharedString {
        SharedString::new_static(pose.asset())
    }

    fn install(&mut self, pose: RiseGuidePose, crossfades: bool, cx: &mut Context<Self>) {
        let previous = self.installed_pose.replace(pose);
        match previous {
            Some(previous) if crossfades && previous != pose => {
                self.outgoing_pose = Some(previous);
                self.crossfade.jump(0.0);
                self.crossfade.set(
                    1.0,
                    ease_in_out(GuideOverlayTheme::POSE_CROSSFADE),
                    RiseClock::monotonic(),
                    false,
                );
            }
            _ => {
                self.outgoing_pose = None;
                self.crossfade.jump(1.0);
            }
        }
        self.apply_playback(cx);
        cx.notify();
    }

    fn apply_playback(&mut self, cx: &mut Context<Self>) {
        if self.installed_pose.is_none() {
            return;
        }
        let should_play = self.plays_animation && self.presence == Some(true);
        if should_play {
            self.claim_playback(cx);
        }
        if self.is_playing != should_play {
            self.is_playing = should_play;
            cx.notify();
        }
    }

    fn claim_playback(&mut self, cx: &mut Context<Self>) {
        let me = cx.entity().downgrade();
        let previous = cx.default_global::<GuideNarratorActivePlayer>().0.replace(me.clone());
        let Some(previous) = previous.filter(|previous| *previous != me) else {
            return;
        };
        if let Some(previous) = previous.upgrade() {
            cx.defer(move |cx| {
                previous.update(cx, |stage, cx| stage.yield_playback(cx));
            });
        }
    }

    fn yield_playback(&mut self, cx: &mut Context<Self>) {
        if self.is_playing {
            self.is_playing = false;
            cx.notify();
        }
    }
}

impl Render for GuideNarratorStageView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let alpha = self.alpha.frame_value(now, window);
        let scale = self.scale.frame_value(now, window);
        let incoming = self.crossfade.frame_value(now, window);
        if !self.crossfade.is_running(now) {
            self.outgoing_pose = None;
        }

        let side = GuideOverlayTheme::NARRATOR_SIDE;
        let speed = if self.is_playing { 1.0 } else { 0.0 };
        let mut stage = div().relative().size_full().opacity(alpha);
        if let Some(outgoing) = self.outgoing_pose {
            stage = stage.child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(1.0 - incoming)
                    .child(LottieView::new(Self::animation_path(outgoing)).size(side).speed(0.0)),
            );
        }
        if let Some(pose) = self.installed_pose {
            stage = stage.child(
                div().absolute().inset_0().opacity(incoming).child(
                    LottieView::new(Self::animation_path(pose))
                        .size(side)
                        .looping(true)
                        .speed(speed),
                ),
            );
        }
        stage.scale_effect(scale)
    }
}

/// parses a pose off the main thread and keeps a few warm
/// a pose is big json and its object graph is far bigger, so never parse it in a frame
#[derive(Default)]
pub struct GuideNarratorAnimationLoader {
    cache: VecDeque<(SharedString, LottieViewModel)>,
}

impl Global for GuideNarratorAnimationLoader {}

impl GuideNarratorAnimationLoader {
    const COUNT_LIMIT: usize = 3;

    pub fn cached(path: &SharedString, cx: &mut App) -> Option<LottieViewModel> {
        let loader = cx.default_global::<Self>();
        if let Some(hit) = loader.cache.iter().find(|(key, _)| key == path).map(|(_, model)| model.clone()) {
            return Some(hit);
        }
        let warm = LottieView::cached(path, cx)?;
        cx.default_global::<Self>().insert(path.clone(), warm.clone());
        Some(warm)
    }

    pub fn load(path: &SharedString, cx: &mut App) -> Task<Option<LottieViewModel>> {
        if let Some(hit) = Self::cached(path, cx) {
            return Task::ready(Some(hit));
        }
        let parsing = LottieView::load(path, cx);
        let key = path.clone();
        cx.spawn(async move |cx| {
            let animation = parsing.await;
            if let Some(animation) = animation.clone() {
                cx.update(|cx| cx.default_global::<Self>().insert(key, animation));
            }
            animation
        })
    }

    fn insert(&mut self, key: SharedString, animation: LottieViewModel) {
        self.cache.retain(|(known, _)| *known != key);
        self.cache.push_back((key, animation));
        while self.cache.len() > Self::COUNT_LIMIT {
            self.cache.pop_front();
        }
    }
}
