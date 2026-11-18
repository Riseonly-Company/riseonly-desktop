use gpui::{div, px, AnyElement, App, Div, ElementId, Entity, IntoElement, ParentElement as _, RenderOnce, Size, Styled as _, Window};
use rise_ui::swiftui::environment::env;

use crate::modules::gifts::components::gift_cosmetic_playback::{
    GiftCosmeticGeometry, GiftCosmeticMotion, GiftCosmeticPart, GiftCosmeticPlacement, GiftCosmeticPlayback,
    GiftCosmeticPlaybackSource, GiftCosmeticSurface, GiftMotionStyle, GiftMotionStyleKey,
};
use crate::modules::gifts::engine::rise_gifts_engine_models::{RiseGiftMediaDTO, RiseGiftMediaExt};

#[derive(IntoElement)]
pub struct GiftFrameView {
    id: ElementId,
    media: Option<RiseGiftMediaDTO>,
    effect: Option<RiseGiftMediaDTO>,
    avatar_size: f32,
    animated: bool,
    avatar: AnyElement,
}

impl GiftFrameView {
    pub fn new(
        id: impl Into<ElementId>,
        media: Option<RiseGiftMediaDTO>,
        avatar_size: f32,
        avatar: impl IntoElement,
    ) -> Self {
        Self { id: id.into(), media, effect: None, avatar_size, animated: true, avatar: avatar.into_any_element() }
    }

    pub fn effect(mut self, effect: Option<RiseGiftMediaDTO>) -> Self {
        self.effect = effect;
        self
    }

    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }
}

impl RenderOnce for GiftFrameView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let avatar = div().size(px(self.avatar_size)).child(self.avatar);
        GiftFrameModifier::new(self.id, self.media, self.avatar_size, avatar)
            .effect(self.effect)
            .animated(self.animated)
    }
}

/// frame around the avatar it modifies, canvas is avatar diameter times canon scale and centred
/// left layer goes behind the avatar and right one in front, effect too
/// avatar keeps size and clicks, no media draws nothing
#[derive(IntoElement)]
pub struct GiftFrameModifier {
    id: ElementId,
    media: Option<RiseGiftMediaDTO>,
    effect: Option<RiseGiftMediaDTO>,
    avatar_size: f32,
    animated: bool,
    content: AnyElement,
}

impl GiftFrameModifier {
    pub const SMALL_POSTER_MAX_SIDE: f32 = 64.0;

    pub fn new(
        id: impl Into<ElementId>,
        media: Option<RiseGiftMediaDTO>,
        avatar_size: f32,
        content: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            media,
            effect: None,
            avatar_size,
            animated: true,
            content: content.into_any_element(),
        }
    }

    pub fn effect(mut self, effect: Option<RiseGiftMediaDTO>) -> Self {
        self.effect = effect;
        self
    }

    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }

    /// each half the same way, the model layer then the rolled effect over it
    fn layers(
        &self,
        front: bool,
        frame: &Entity<GiftCosmeticPlayback>,
        sparkle: &Entity<GiftCosmeticPlayback>,
        motion_style: GiftMotionStyle,
    ) -> Div {
        let canvas = self.avatar_size * GiftCosmeticGeometry::frame_canvas_scale();
        let offset = (self.avatar_size - canvas) / 2.0;
        let part = if front { GiftCosmeticPart::Front } else { GiftCosmeticPart::Back };
        let (frame_name, sparkle_name) = if front { ("frame-front", "sparkle-front") } else { ("frame-back", "sparkle-back") };
        let mut stack = div().absolute().left(px(offset)).top(px(offset)).size(px(canvas));
        if let Some(media) = &self.media {
            stack = stack.child(self.surface((self.id.clone(), frame_name).into(), frame, media, part, motion_style));
        }
        if let Some(effect) = &self.effect {
            stack = stack.child(self.surface((self.id.clone(), sparkle_name).into(), sparkle, effect, part, motion_style));
        }
        stack
    }

    fn surface(
        &self,
        id: ElementId,
        playback: &Entity<GiftCosmeticPlayback>,
        media: &RiseGiftMediaDTO,
        part: GiftCosmeticPart,
        motion_style: GiftMotionStyle,
    ) -> Div {
        let small = self.avatar_size <= Self::SMALL_POSTER_MAX_SIDE;
        let mut source = GiftCosmeticPlaybackSource::new(
            media.cosmetic_poster(small),
            media.cosmetic_movie(),
            motion_style.motion(GiftCosmeticMotion::Loop),
            self.animated,
        );
        source.poster_pixels = if small { 360 } else { 768 };
        if !small {
            source.preview_poster = media.cosmetic_poster(true).map(|url| (url, 360));
        }
        source.replays_on_return = motion_style.replays_on_return();
        let canvas = px(self.avatar_size * GiftCosmeticGeometry::frame_canvas_scale());
        div().absolute().inset_0().child(
            GiftCosmeticSurface::new(id, playback.clone(), source, GiftCosmeticGeometry::FRAME_LAYER_ASPECT)
                .size_hint(Some(Size { width: canvas, height: canvas }))
                .part(part)
                .placement(GiftCosmeticPlacement::Fill),
        )
    }
}

impl RenderOnce for GiftFrameModifier {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let motion_style = env::<GiftMotionStyleKey>(cx);
        let frame = GiftCosmeticPlayback::keyed((self.id.clone(), "frame"), window, cx);
        let sparkle = GiftCosmeticPlayback::keyed((self.id.clone(), "sparkle"), window, cx);
        let back = self.layers(false, &frame, &sparkle, motion_style);
        let front = self.layers(true, &frame, &sparkle, motion_style);
        div()
            .relative()
            .size(px(self.avatar_size))
            .child(back)
            .child(self.content)
            .child(front)
    }
}
