use std::sync::Arc;

use gpui::{
    App, AppContext, Context, Entity, IntoElement, ParentElement, Pixels, Render, Styled, Window,
    div,
};
use image::{Delay, Frame, RgbaImage};
use smallvec::SmallVec;
use std::time::Duration;

use super::raster_policy::{self, SequenceKey};
use super::sequence::{DecodedFrameCache, FrameSequence, LottieRasterizer};

/// A Lottie animation on screen.
///
/// One `RenderImage` per animation, never one per frame. gpui advances the frame
/// index of a multi-frame image itself and uploads each frame into the atlas
/// under ONE image id, so the whole animation is one live texture set — which is
/// the rule `PERFORMANCE_GUIDE.md` states as "one persistent texture per stream"
/// and the difference between the documented 300 MB case and 12 MB.
///
/// It also means gpui owns the timing, so the animation honours Reduce Motion
/// without this file knowing what that is.
pub struct LottieView {
    image: Option<Arc<gpui::RenderImage>>,
    side: Pixels,
    /// Set when rasterisation produced nothing a user would see. rlottie
    /// renders an EMPTY frame rather than failing when an animation uses a
    /// feature it does not implement, so a caller has to be able to tell "not
    /// loaded yet" from "this will never draw" and put something else there.
    is_blank: bool,
}

/// What a caller has to supply. Kept as data so the loading policy — which
/// thread, which cache, which budget — belongs to the app rather than to the
/// element.
pub struct LottieRequest {
    pub json: Vec<u8>,
    pub cache_key: String,
    pub side: Pixels,
    pub scale_factor: f32,
    /// How much decoded frame data this animation may hold. The element
    /// rasterises its whole timeline up front, so this is a real ceiling rather
    /// than a hint.
    pub budget_bytes: u64,
}

impl LottieView {
    pub fn empty(side: Pixels) -> Self {
        Self {
            image: None,
            side,
            is_blank: false,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.image.is_some()
    }

    pub fn is_blank(&self) -> bool {
        self.is_blank
    }

    pub fn set_image(&mut self, image: Option<Arc<gpui::RenderImage>>, cx: &mut Context<Self>) {
        self.is_blank = image.is_none();
        self.image = image;
        cx.notify();
    }

    /// Rasterises a whole animation into one animatable image.
    ///
    /// Blocking and CPU-bound: it belongs on a background executor, never on the
    /// GPUI thread. The frame set is strided down from the source rate by
    /// `raster_policy::presentation_stride`, so a 60 fps source costs half the
    /// frames of a naive port at a cadence nobody can see the difference in.
    pub fn rasterize(
        request: &LottieRequest,
        open: impl FnOnce(&[u8], &str) -> Option<Box<dyn LottieRasterizer>>,
    ) -> Option<Arc<gpui::RenderImage>> {
        let dimension = raster_policy::dimension(f32::from(request.side), request.scale_factor);
        let key = SequenceKey::new(request.cache_key.clone(), dimension);

        let json = super::container::to_animation_json(&request.json).ok()?;
        let rasterizer = open(&json, &request.cache_key)?;

        let cache = Arc::new(DecodedFrameCache::new(request.budget_bytes));
        let sequence = FrameSequence::open(key, rasterizer, cache).ok()?;

        // An animation that never paints anything is reported as such rather
        // than drawn as a hole.
        sequence.first_visible_frame()?;

        let stride = sequence.presentation_stride().max(1);
        let source_rate = sequence.frame_rate().max(1.0);
        let delay = frame_delay(source_rate, stride);

        let mut frames: SmallVec<[Frame; 1]> = SmallVec::new();
        let mut index = 0usize;
        while index < sequence.frame_count() {
            let decoded = sequence.frame(index)?;
            let buffer =
                RgbaImage::from_raw(decoded.dimension, decoded.dimension, decoded.bytes().to_vec())?;
            frames.push(Frame::from_parts(buffer, 0, 0, delay));
            index += stride;
        }

        if frames.is_empty() {
            return None;
        }

        Some(Arc::new(gpui::RenderImage::new(frames)))
    }
}

/// How long each SAMPLED frame is shown.
///
/// The stride drops frames, so the delay has to grow by the same factor or the
/// animation plays back faster than it was authored.
fn frame_delay(source_frame_rate: f64, stride: usize) -> Delay {
    let seconds = stride as f64 / source_frame_rate;
    Delay::from_saturating_duration(Duration::from_secs_f64(seconds.clamp(0.001, 1.0)))
}

impl Render for LottieView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut container = div().w(self.side).h(self.side).flex_none();

        if let Some(image) = self.image.clone() {
            container = container.child(gpui::img(image).w(self.side).h(self.side));
        }

        container
    }
}

/// Builds the view and fills it in when the raster finishes.
///
/// The entity exists immediately with nothing in it, so the screen lays out at
/// its final geometry from the first frame — the layout does not jump when the
/// animation arrives, which is the same rule the skeletons follow.
pub fn spawn_lottie(
    request: LottieRequest,
    open: impl FnOnce(&[u8], &str) -> Option<Box<dyn LottieRasterizer>> + Send + 'static,
    cx: &mut App,
) -> Entity<LottieView> {
    let side = request.side;
    let view = cx.new(|_| LottieView::empty(side));

    let handle = view.clone();
    cx.spawn(async move |cx| {
        // Rasterising a whole timeline is CPU-bound work measured in tens of
        // milliseconds. On the GPUI thread that is a dropped frame at best; the
        // background executor is where it belongs, and the result comes back as
        // one short commit.
        let image = cx
            .background_spawn(async move { LottieView::rasterize(&request, open) })
            .await;
        let _ = handle.update(cx, |view, cx| view.set_image(image, cx));
    })
    .detach();

    view
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_strided_animation_keeps_its_authored_duration() {
        // 60 fps sampled every other frame is 30 fps, so each sampled frame has
        // to be shown twice as long or the animation runs at double speed.
        let one_to_one = frame_delay(60.0, 1);
        let strided = frame_delay(60.0, 2);

        assert!(
            Duration::from(strided) > Duration::from(one_to_one),
            "dropping frames without lengthening the delay speeds the animation up"
        );
        // Compared as a ratio rather than in whole milliseconds: 1/60 s is
        // 16.67 ms, and asserting 2 x 16 == 33 fails on the rounding rather
        // than on the behaviour.
        let ratio = Duration::from(strided).as_secs_f64() / Duration::from(one_to_one).as_secs_f64();
        assert!(
            (ratio - 2.0).abs() < 0.01,
            "a stride of 2 must double the per-frame delay; got {ratio}"
        );
    }

    #[test]
    fn an_implausible_frame_rate_cannot_produce_a_zero_or_endless_delay() {
        for rate in [0.0001, 1.0, 240.0, f64::MAX] {
            let delay = Duration::from(frame_delay(rate, 1));
            assert!(delay >= Duration::from_millis(1), "{rate} produced {delay:?}");
            assert!(delay <= Duration::from_secs(1), "{rate} produced {delay:?}");
        }
    }
}
