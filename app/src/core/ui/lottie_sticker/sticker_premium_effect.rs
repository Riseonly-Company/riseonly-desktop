use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::{
    AnyWindowHandle, App, Bounds, ContentMask, EntityId, Global, IntoElement, ParentElement, Pixels, RenderOnce,
    Size, Styled, Task, Window, canvas, div, point, px,
};
use parking_lot::Mutex;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle};
use rise_ui::swiftui::lifecycle::LifecycleModifiers;

use crate::core::desktop_platform::rise_app_events::RiseNotificationName;
use crate::core::ui::animated_media_playback_coordinator::AnimatedMediaPlaybackKind;
use crate::core::ui::lottie_sticker::lottie_sticker_loader::LottieStickerLoader;
use crate::core::ui::lottie_sticker::lottie_sticker_view::LottieStickerView;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;

#[derive(Default)]
struct StickerPremiumEffectRegistryState {
    effects: HashMap<String, String>,
    armed_sends: HashMap<String, Date>,
}

/// premium effects of the stickers this session has loaded, keyed by sticker file url
/// server sends the effect url to every reader and only an admin may set it
/// so the client checks no role
pub struct StickerPremiumEffectRegistry {
    state: Mutex<StickerPremiumEffectRegistryState>,
}

static SHARED: LazyLock<StickerPremiumEffectRegistry> =
    LazyLock::new(|| StickerPremiumEffectRegistry { state: Mutex::new(StickerPremiumEffectRegistryState::default()) });

impl StickerPremiumEffectRegistry {
    const ARMED_SEND_LIFETIME: f64 = 6.0;

    pub fn shared() -> &'static StickerPremiumEffectRegistry {
        &SHARED
    }

    pub fn key(raw: &str) -> String {
        RemoteMediaURL::optional_normalized(Some(raw))
            .unwrap_or_else(|| trimming_whitespaces_and_newlines(raw).to_string())
    }

    /// records the effect of one sticker, a missing effect removes a stale entry
    pub fn record(&self, sticker_url: &str, effect_url: Option<&str>) {
        let key = Self::key(sticker_url);
        if key.is_empty() {
            return;
        }
        let effect = effect_url.map(Self::key).unwrap_or_default();
        let mut state = self.state.lock();
        if effect.is_empty() {
            state.effects.remove(&key);
        } else {
            state.effects.insert(key, effect);
        }
    }

    pub fn effect_url(&self, sticker_url: &str) -> Option<String> {
        let key = Self::key(sticker_url);
        if key.is_empty() {
            return None;
        }
        self.state.lock().effects.get(&key).cloned()
    }

    pub fn arm_send(&self, sticker_url: &str) {
        let key = Self::key(sticker_url);
        if key.is_empty() {
            return;
        }
        let mut state = self.state.lock();
        if !state.effects.contains_key(&key) {
            return;
        }
        state.armed_sends.insert(key, RiseClock::now());
    }

    pub fn consume_armed_send(&self, sticker_url: &str) -> Option<String> {
        let key = Self::key(sticker_url);
        if key.is_empty() {
            return None;
        }
        let mut state = self.state.lock();
        let armed_at = state.armed_sends.remove(&key)?;
        if RiseClock::now().time_interval_since(&armed_at) > Self::ARMED_SEND_LIFETIME {
            return None;
        }
        state.effects.get(&key).cloned()
    }

    pub fn remove_all(&self) {
        let mut state = self.state.lock();
        state.effects.clear();
        state.armed_sends.clear();
    }
}

/// telegram premium sticker layout the effect art is drawn for
pub enum StickerPremiumEffectGeometry {}

impl StickerPremiumEffectGeometry {
    pub const EDGE_GROWTH: f32 = 0.245;
    pub const CANVAS_SCALE: f32 = 1.0 + 2.0 * Self::EDGE_GROWTH;
    pub const INWARD_SHIFT: f32 = Self::EDGE_GROWTH - 21.0 / 184.0;
}

pub const STICKER_PREMIUM_EFFECT_DID_START: RiseNotificationName =
    RiseNotificationName::new("riseonly.stickerPremiumEffectDidStart");

/// where the sticker behind an anchor was drawn in the last frame
#[derive(Clone, Copy)]
struct StickerPremiumEffectAnchorFrame {
    window: AnyWindowHandle,
    view: EntityId,
    sticker: Bounds<Pixels>,
    feed: ContentMask<Pixels>,
    window_size: Size<Pixels>,
}

static NEXT_ANCHOR_ID: AtomicU64 = AtomicU64::new(1);

/// handle to the drawn sticker so the effect can follow it in the window
/// clones share one anchor
#[derive(Clone)]
pub struct StickerPremiumEffectAnchor {
    id: u64,
    view: Rc<Cell<Option<StickerPremiumEffectAnchorFrame>>>,
}

impl Default for StickerPremiumEffectAnchor {
    fn default() -> Self {
        Self::new()
    }
}

impl StickerPremiumEffectAnchor {
    pub fn new() -> Self {
        Self { id: NEXT_ANCHOR_ID.fetch_add(1, Ordering::Relaxed), view: Rc::new(Cell::new(None)) }
    }

    fn same(&self, other: &StickerPremiumEffectAnchor) -> bool {
        Rc::ptr_eq(&self.view, &other.view)
    }

    fn frame(&self) -> Option<StickerPremiumEffectAnchorFrame> {
        self.view.get()
    }
}

/// invisible layer that publishes the sticker frame to the anchor and draws the effect run pinned to it
/// fill the sticker with it
#[derive(IntoElement)]
pub struct StickerPremiumEffectAnchorView {
    anchor: StickerPremiumEffectAnchor,
}

impl StickerPremiumEffectAnchorView {
    pub fn new(anchor: StickerPremiumEffectAnchor) -> Self {
        Self { anchor }
    }
}

impl RenderOnce for StickerPremiumEffectAnchorView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let anchor = self.anchor.clone();
        let leaving = self.anchor.clone();
        canvas(
            move |bounds, window, cx| {
                let frame = StickerPremiumEffectAnchorFrame {
                    window: window.window_handle(),
                    view: window.current_view(),
                    sticker: bounds,
                    feed: window.content_mask(),
                    window_size: window.viewport_size(),
                };
                anchor.view.set(Some(frame));
                StickerPremiumEffectOverlay::follow_anchor(&anchor, frame, window, cx);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
        .on_disappear(("sticker-premium-effect-anchor", self.anchor.id), move |_window, cx| {
            leaving.view.set(None);
            StickerPremiumEffectPlayer::anchor_did_leave(&leaving, cx);
        })
    }
}

/// one effect run, loading and then drawn over the sticker until its last frame
struct StickerPremiumEffectRun {
    id: u64,
    effect_url: String,
    anchor: StickerPremiumEffectAnchor,
    is_loaded: bool,
    has_started: bool,
    on_start: Option<Box<dyn FnOnce(&mut App)>>,
    _load_task: Option<Task<()>>,
}

#[derive(Default)]
struct StickerPremiumEffectPlayerState {
    current: Option<StickerPremiumEffectRun>,
    next_run_id: u64,
}

impl Global for StickerPremiumEffectPlayerState {}

static HAPTICS: ImpactFeedbackGenerator = ImpactFeedbackGenerator::new(ImpactStyle::Light);

pub enum StickerPremiumEffectPlayer {}

impl StickerPremiumEffectPlayer {
    /// plays effect_url once over the window pinned to the sticker behind anchor
    /// a second call for the same sticker while it plays is ignored
    /// on_start runs on the first frame so the sticker can restart in step
    pub fn play(
        effect_url: &str,
        anchor: &StickerPremiumEffectAnchor,
        on_start: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) {
        if anchor.frame().is_none() {
            return;
        }
        if let Some(current) = cx.try_global::<StickerPremiumEffectPlayerState>().and_then(|state| state.current.as_ref())
            && current.anchor.same(anchor)
            && current.effect_url == effect_url
        {
            return;
        }
        Self::finish_current(cx);
        let state = cx.default_global::<StickerPremiumEffectPlayerState>();
        state.next_run_id = state.next_run_id.wrapping_add(1);
        let run_id = state.next_run_id;
        HAPTICS.prepare();
        let url = effect_url.to_owned();
        let load_task = cx.spawn(async move |cx| {
            let loaded = LottieStickerLoader::shared().payload(&url).await.is_ok();
            cx.update(|cx| Self::did_load(run_id, loaded, cx));
        });
        cx.default_global::<StickerPremiumEffectPlayerState>().current = Some(StickerPremiumEffectRun {
            id: run_id,
            effect_url: effect_url.to_owned(),
            anchor: anchor.clone(),
            is_loaded: false,
            has_started: false,
            on_start: Some(Box::new(on_start)),
            _load_task: Some(load_task),
        });
    }

    fn did_load(run_id: u64, loaded: bool, cx: &mut App) {
        let Some(run) = Self::current_run(run_id, cx) else {
            return;
        };
        run._load_task = None;
        if !loaded {
            Self::finish(run_id, cx);
            return;
        }
        let Some(frame) = run.anchor.frame() else {
            Self::finish(run_id, cx);
            return;
        };
        run.is_loaded = true;
        cx.notify(frame.view);
    }

    fn did_start(run_id: u64, cx: &mut App) {
        let Some(run) = Self::current_run(run_id, cx) else {
            return;
        };
        if run.has_started {
            return;
        }
        run.has_started = true;
        let on_start = run.on_start.take();
        let view = run.anchor.frame().map(|frame| frame.view);
        if let Some(on_start) = on_start {
            on_start(cx);
        }
        HAPTICS.impact_occurred();
        if let Some(view) = view {
            cx.notify(view);
        }
    }

    fn current_run(run_id: u64, cx: &mut App) -> Option<&mut StickerPremiumEffectRun> {
        cx.default_global::<StickerPremiumEffectPlayerState>().current.as_mut().filter(|run| run.id == run_id)
    }

    fn finish(run_id: u64, cx: &mut App) {
        if Self::current_run(run_id, cx).is_some() {
            Self::finish_current(cx);
        }
    }

    fn finish_current(cx: &mut App) {
        let Some(run) = cx.default_global::<StickerPremiumEffectPlayerState>().current.take() else {
            return;
        };
        if let Some(frame) = run.anchor.frame() {
            cx.notify(frame.view);
        }
    }

    fn anchor_did_leave(anchor: &StickerPremiumEffectAnchor, cx: &mut App) {
        let leaves_current = cx
            .try_global::<StickerPremiumEffectPlayerState>()
            .and_then(|state| state.current.as_ref())
            .is_some_and(|run| run.anchor.same(anchor));
        if leaves_current {
            Self::finish_current(cx);
        }
    }
}

enum StickerPremiumEffectOverlay {}

impl StickerPremiumEffectOverlay {
    fn follow_anchor(
        anchor: &StickerPremiumEffectAnchor,
        frame: StickerPremiumEffectAnchorFrame,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some((run_id, effect_url, has_started)) = cx
            .try_global::<StickerPremiumEffectPlayerState>()
            .and_then(|state| state.current.as_ref())
            .filter(|run| run.is_loaded && run.anchor.same(anchor))
            .map(|run| (run.id, run.effect_url.clone(), run.has_started))
        else {
            return;
        };
        let sticker = frame.sticker;
        let side = sticker.size.width.max(sticker.size.height);
        if side <= px(0.0) {
            cx.defer(move |cx| StickerPremiumEffectPlayer::finish(run_id, cx));
            return;
        }
        let is_trailing = sticker.center().x > frame.window_size.width * 0.5;
        let canvas_side = side * StickerPremiumEffectGeometry::CANVAS_SCALE;
        let shift = side * StickerPremiumEffectGeometry::INWARD_SHIFT;
        let center_x = sticker.center().x + shift * if is_trailing { -1.0 } else { 1.0 };
        let center_y = sticker.center().y - px(1.0);
        let origin = point(center_x - canvas_side * 0.5, center_y - canvas_side * 0.5);
        let effect = div()
            .size(canvas_side)
            .opacity(if has_started { 1.0 } else { 0.0 })
            .child(
                LottieStickerView::new(("sticker-premium-effect", run_id), effect_url)
                    .size(f32::from(canvas_side))
                    .r#loop(false)
                    .playback_id(Some(format!("sticker-premium-effect-{run_id}")))
                    .playback_kind(AnimatedMediaPlaybackKind::OpenedAnimation)
                    .mirrored(!is_trailing)
                    .on_first_frame(move |cx| StickerPremiumEffectPlayer::did_start(run_id, cx))
                    .on_complete(move |cx| StickerPremiumEffectPlayer::finish(run_id, cx)),
            )
            .into_any_element();
        window.defer_draw(effect, origin, 1, Some(frame.feed));
        window.request_animation_frame();
    }
}
