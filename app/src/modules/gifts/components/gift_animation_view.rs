use gpui::{App, ElementId, IntoElement, RenderOnce, Size, Window, px};
use rise_ui::swiftui::environment::env;

use crate::modules::gifts::components::gift_cosmetic_playback::{
    GiftCosmeticMotion, GiftCosmeticPlacement, GiftCosmeticPlayback, GiftCosmeticPlaybackSource, GiftCosmeticSurface,
    GiftMotionStyleKey,
};
use crate::modules::gifts::engine::rise_gifts_engine_models::{RiseGiftMediaDTO, RiseGiftMediaExt};

#[derive(IntoElement)]
pub struct GiftAnimationView {
    id: ElementId,
    media: Option<RiseGiftMediaDTO>,
    large: bool,
    animated: bool,
    placeholder: bool,
    size_hint: Option<Size<f32>>,
}

impl GiftAnimationView {
    pub fn new(id: impl Into<ElementId>, media: Option<RiseGiftMediaDTO>) -> Self {
        Self { id: id.into(), media, large: true, animated: true, placeholder: false, size_hint: None }
    }

    pub fn large(mut self, large: bool) -> Self {
        self.large = large;
        self
    }

    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }

    /// soft disc until the poster or first frame draws, for a hero that would open empty
    pub fn placeholder(mut self, placeholder: bool) -> Self {
        self.placeholder = placeholder;
        self
    }

    /// view size when the parent knows it so the surface lays out its picture from the first render
    pub fn size_hint(mut self, size_hint: Option<Size<f32>>) -> Self {
        self.size_hint = size_hint;
        self
    }
}

impl RenderOnce for GiftAnimationView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let motion_style = env::<GiftMotionStyleKey>(cx);
        let playback =
            window.use_keyed_state((self.id.clone(), "playback"), cx, |_, cx| GiftCosmeticPlayback::new(cx));
        let mut source = GiftCosmeticPlaybackSource::new(
            self.media.as_ref().and_then(|media| media.poster(self.large)),
            self.media.as_ref().and_then(|media| media.movie(self.large)),
            motion_style.motion(GiftCosmeticMotion::Loop),
            self.animated,
        );
        source.poster_pixels = if self.large { 1_024 } else { 512 };
        source.replays_on_return = motion_style.replays_on_return();
        if self.large {
            source.preview_poster = self.media.as_ref().and_then(|media| media.poster(false)).map(|url| (url, 512));
        }
        GiftCosmeticSurface::new((self.id, "surface"), playback, source, 1.0)
            .placement(GiftCosmeticPlacement::Fit)
            .placeholder(self.placeholder)
            .size_hint(self.size_hint.map(|size| Size { width: px(size.width), height: px(size.height) }))
    }
}
