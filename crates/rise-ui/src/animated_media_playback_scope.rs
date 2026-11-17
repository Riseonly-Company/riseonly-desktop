use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Bounds, ElementId, IntoElement, ParentElement, Pixels, RenderOnce, Styled, Window, canvas,
    div,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::rise_app_events::RiseNotificationName;

use crate::swiftui::environment::{EnvKey, env};
use crate::window_screen_frame_reader::ScrollFrameSync;

pub struct AnimatedMediaViewportPlaybackPolicy;

impl AnimatedMediaViewportPlaybackPolicy {
    pub const MINIMUM_VISIBLE_HEIGHT_FRACTION: f32 = 0.1;
}

/// env key, true while animated media may play
pub struct AnimatedMediaPlaybackActiveKey;

impl EnvKey for AnimatedMediaPlaybackActiveKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        true
    }
}

/// reads whether animated media playback is active
pub fn is_animated_media_playback_active(cx: &App) -> bool {
    env::<AnimatedMediaPlaybackActiveKey>(cx)
}

/// desktop only, for the content of a page the fullscreen media viewer can cover
/// viewer is a layer above the page so an animated cell under it pauses while it covers the page
/// value stays constant for the page, the cell reads the viewer flip live
pub struct AnimatedMediaPausesUnderMediaViewerKey;

impl EnvKey for AnimatedMediaPausesUnderMediaViewerKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        false
    }
}

/// a container that can pause every animated cell inside it
pub trait AnimatedMediaPlaybackScopeProviding {
    fn is_animated_media_playback_active(&self) -> bool;
}

/// a container that knows whether its animated cells are inside its viewport
pub trait AnimatedMediaViewportScopeProviding {
    fn is_animated_media_visible_in_viewport(&self) -> bool;
}

pub const ANIMATED_MEDIA_PLAYBACK_SCOPE_DID_CHANGE: RiseNotificationName =
    RiseNotificationName::new("riseonly.animated-media.playback-scope-did-change");

pub const ANIMATED_MEDIA_VIEWPORT_SCOPE_DID_CHANGE: RiseNotificationName =
    RiseNotificationName::new("riseonly.animated-media.viewport-scope-did-change");

pub struct AnimatedMediaPlaybackScopeViewKey;

impl EnvKey for AnimatedMediaPlaybackScopeViewKey {
    type Value = Option<Rc<dyn AnimatedMediaPlaybackScopeProviding>>;

    fn default_value() -> Self::Value {
        None
    }
}

/// nearest enclosing viewport scope, put in place like AnimatedMediaPlaybackScopeViewKey
/// a scope never sees its own value, only its content does
pub struct AnimatedMediaViewportScopeViewKey;

impl EnvKey for AnimatedMediaViewportScopeViewKey {
    type Value = Option<Rc<dyn AnimatedMediaViewportScopeProviding>>;

    fn default_value() -> Self::Value {
        None
    }
}

/// nearest enclosing playback scope view, read it in render
pub fn nearest_animated_media_playback_scope_view(
    cx: &App,
) -> Option<Rc<dyn AnimatedMediaPlaybackScopeProviding>> {
    env::<AnimatedMediaPlaybackScopeViewKey>(cx)
}

/// nearest enclosing viewport scope view, read it in render
pub fn nearest_animated_media_viewport_scope_view(
    cx: &App,
) -> Option<Rc<dyn AnimatedMediaViewportScopeProviding>> {
    env::<AnimatedMediaViewportScopeViewKey>(cx)
}

const SETTLED_TICKS: u32 = 2;
const MAXIMUM_TRACKING: Duration = Duration::from_secs(1);
const MOVE_TOLERANCE: f32 = 0.5;

struct ProbeView {
    started_at: Duration,
    last_frame: Option<Bounds<Pixels>>,
    stable_ticks: u32,
    stopped: bool,
}

impl ProbeView {
    fn new(now: Duration) -> Self {
        Self {
            started_at: now,
            last_frame: None,
            stable_ticks: 0,
            stopped: false,
        }
    }

    fn stop(&mut self) {
        self.stopped = true;
    }

    /// one frame, true once the frame held still for SETTLED_TICKS frames or tracking ran out of time
    fn tick(&mut self, frame: Bounds<Pixels>, now: Duration) -> bool {
        let moved = match self.last_frame {
            None => true,
            Some(last) => {
                f32::from(frame.origin.x - last.origin.x).abs() > MOVE_TOLERANCE
                    || f32::from(frame.origin.y - last.origin.y).abs() > MOVE_TOLERANCE
                    || f32::from(frame.size.height - last.size.height).abs() > MOVE_TOLERANCE
            }
        };
        self.last_frame = Some(frame);
        self.stable_ticks = if moved { 0 } else { self.stable_ticks + 1 };
        self.stable_ticks >= SETTLED_TICKS || now.saturating_sub(self.started_at) >= MAXIMUM_TRACKING
    }
}

/// animated cells load and play from scroll ticks and size changes, a sliding container gives neither
/// so cells below the edge stay blank until a scroll
/// put it behind such a container and it follows until it stops
#[derive(IntoElement)]
pub struct AnimatedMediaSettleProbe {
    id: ElementId,
}

impl AnimatedMediaSettleProbe {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into() }
    }
}

impl RenderOnce for AnimatedMediaSettleProbe {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let probe = window.use_keyed_state(self.id, cx, |_, _| ProbeView::new(RiseClock::monotonic()));
        let tracker = canvas(
            |_, _, _| (),
            move |frame, _, window, cx| {
                if probe.read(cx).stopped {
                    return;
                }
                let settled = probe.update(cx, |probe, _| {
                    let settled = probe.tick(frame, RiseClock::monotonic());
                    if settled {
                        probe.stop();
                    }
                    settled
                });
                if !settled {
                    window.request_animation_frame();
                    return;
                }
                let window_handle = window.window_handle();
                ScrollFrameSync::shared().refresh_subscribers(window_handle, frame, cx);
            },
        )
        .size_full();
        div().size_full().child(tracker)
    }
}
