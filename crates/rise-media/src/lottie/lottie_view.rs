use std::sync::Arc;

use gpui::{
    App, AppContext, Context, ElementId, Entity, InteractiveElement, IntoElement, ParentElement,
    Pixels, Render, SharedString, Styled, Window, div,
};
use image::{Delay, Frame, RgbaImage};
use smallvec::SmallVec;
use std::time::Duration;

use super::raster_policy::{self, SequenceKey};
use super::sequence::{DecodedFrameCache, FrameSequence, LottieRasterizer};

/// A Lottie animation on screen.
///
/// One `RenderImage` per animation, never one per frame: gpui advances the
/// frame index itself under ONE image id, so the whole animation is a single
/// live texture set and gpui owns the timing — Reduce Motion and inactive
/// windows are honoured without this file knowing what either is.
pub struct LottieView {
    image: Option<Arc<gpui::RenderImage>>,
    side: Pixels,
    key: SharedString,
    is_blank: bool,
}

/// What a caller has to supply. Data rather than behaviour, so the loading
/// policy stays with the app.
pub struct LottieRequest {
    pub json: Vec<u8>,
    pub cache_key: String,
    pub side: Pixels,
    pub scale_factor: f32,
    /// How much decoded frame data this animation may hold. The whole timeline
    /// is rasterised up front, so this is a ceiling rather than a hint.
    pub budget_bytes: u64,
}

impl LottieView {
    pub fn empty(key: impl Into<SharedString>, side: Pixels) -> Self {
        Self {
            image: None,
            side,
            key: key.into(),
            is_blank: false,
        }
    }

    /// The id the animation frame is drawn under, and the whole reason it moves.
    ///
    /// gpui's `Img` only animates a multi-frame image when the element carries
    /// an `ElementId`, because it keeps the frame index in element state.
    /// Without an id the index stays 0 and nothing even repaints; with one,
    /// autoplay and looping come for free. Every animation needs its own id.
    pub fn element_id(&self) -> ElementId {
        ElementId::Name(self.key.clone())
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

    /// Rasterises a whole animation into one animatable image, strided down to
    /// `raster_policy`'s cadence. Blocking and CPU-bound: it belongs on a
    /// background executor, never on the GPUI thread.
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

        // An animation that never paints anything is refused, not drawn as a hole.
        sequence.first_visible_frame()?;

        let stride = sequence.presentation_stride().max(1);
        let source_rate = sequence.frame_rate().max(1.0);
        let delay = frame_delay(source_rate, stride);

        let mut frames: SmallVec<[Frame; 1]> = SmallVec::new();
        let mut index = 0usize;
        while index < sequence.frame_count() {
            let decoded = sequence.frame(index)?;
            let buffer = RgbaImage::from_raw(
                decoded.dimension,
                decoded.dimension,
                decoded.bytes().to_vec(),
            )?;
            frames.push(Frame::from_parts(buffer, 0, 0, delay));
            index += stride;
        }

        if frames.is_empty() {
            return None;
        }

        Some(Arc::new(gpui::RenderImage::new(frames)))
    }
}

// The delay grows with the stride, or the animation plays faster than authored.
fn frame_delay(source_frame_rate: f64, stride: usize) -> Delay {
    let seconds = stride as f64 / source_frame_rate;
    Delay::from_saturating_duration(Duration::from_secs_f64(seconds.clamp(0.001, 1.0)))
}

impl Render for LottieView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut container = div().w(self.side).h(self.side).flex_none();

        if let Some(image) = self.image.clone() {
            container = container.child(
                gpui::img(image)
                    // Not decoration: an unidentified `img` never animates. See `element_id`.
                    .id(self.element_id())
                    .w(self.side)
                    .h(self.side),
            );
        }

        container
    }
}

/// Builds the view and fills it in when the raster finishes. The entity exists
/// immediately at its final geometry, so the layout does not jump when the
/// animation arrives.
pub fn spawn_lottie(
    request: LottieRequest,
    open: impl FnOnce(&[u8], &str) -> Option<Box<dyn LottieRasterizer>> + Send + 'static,
    cx: &mut App,
) -> Entity<LottieView> {
    let side = request.side;
    let key = request.cache_key.clone();
    let view = cx.new(|_| LottieView::empty(key, side));

    let handle = view.clone();
    cx.spawn(async move |cx| {
        let image = cx
            .background_spawn(async move { LottieView::rasterize(&request, open) })
            .await;
        // A screen closed mid-raster drops the view; that is normal, not an error.
        handle.update(cx, |view, cx| view.set_image(image, cx));
    })
    .detach();

    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    // Drop the id and gpui freezes the picture on frame 0, with nothing logged anywhere.
    #[test]
    fn a_view_is_identified_or_it_will_never_animate() {
        let view = LottieView::empty("red_panda/hello", px(168.0));
        assert_eq!(
            view.element_id(),
            gpui::ElementId::Name("red_panda/hello".into())
        );
    }

    // Shared element state would let one animation drive the other's frame index.
    #[test]
    fn two_animations_never_share_an_identity() {
        let hello = LottieView::empty("red_panda/hello", px(168.0));
        let party = LottieView::empty("party", px(168.0));
        assert_ne!(hello.element_id(), party.element_id());
    }

    #[test]
    fn a_strided_animation_keeps_its_authored_duration() {
        let one_to_one = frame_delay(60.0, 1);
        let strided = frame_delay(60.0, 2);

        assert!(
            Duration::from(strided) > Duration::from(one_to_one),
            "dropping frames without lengthening the delay speeds the animation up"
        );
        // A ratio, not milliseconds: 2 x 16.67 ms rounds to 33 and would fail on rounding.
        let ratio =
            Duration::from(strided).as_secs_f64() / Duration::from(one_to_one).as_secs_f64();
        assert!(
            (ratio - 2.0).abs() < 0.01,
            "a stride of 2 must double the per-frame delay; got {ratio}"
        );
    }

    #[test]
    fn an_implausible_frame_rate_cannot_produce_a_zero_or_endless_delay() {
        for rate in [0.0001, 1.0, 240.0, f64::MAX] {
            let delay = Duration::from(frame_delay(rate, 1));
            assert!(
                delay >= Duration::from_millis(1),
                "{rate} produced {delay:?}"
            );
            assert!(delay <= Duration::from_secs(1), "{rate} produced {delay:?}");
        }
    }
}
