use gpui::{
    div, px, App, ElementId, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _, Window,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_theme::ThemeStore;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::transition::UnitPoint;

use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::gifts::components::gift_cosmetic_playback::{
    GiftCosmeticGeometry, GiftCosmeticMotion, GiftCosmeticPart, GiftCosmeticPlacement, GiftCosmeticPlayback,
    GiftCosmeticPlaybackSource, GiftCosmeticSurface, GiftMotionStyle, GiftMotionStyleKey,
};
use crate::modules::gifts::engine::rise_gifts_engine_models::{RiseGiftMediaDTO, RiseGiftMediaExt};
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::GiftRecipient;

#[derive(IntoElement)]
pub struct GiftProfileEffectView {
    id: ElementId,
    media: Option<RiseGiftMediaDTO>,
    animated: bool,
    once: bool,
    rests_on_poster: bool,
    replay: i64,
}

impl GiftProfileEffectView {
    pub fn new(id: impl Into<ElementId>, media: Option<RiseGiftMediaDTO>) -> Self {
        Self { id: id.into(), media, animated: true, once: false, rests_on_poster: true, replay: 0 }
    }

    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }

    pub fn once(mut self, once: bool) -> Self {
        self.once = once;
        self
    }

    pub fn rests_on_poster(mut self, rests_on_poster: bool) -> Self {
        self.rests_on_poster = rests_on_poster;
        self
    }

    pub fn replay(mut self, replay: i64) -> Self {
        self.replay = replay;
        self
    }
}

impl RenderOnce for GiftProfileEffectView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let motion_style = env::<GiftMotionStyleKey>(cx);
        let playback = GiftCosmeticPlayback::keyed((self.id.clone(), "playback"), window, cx);
        let plays_per_appearance = !self.once && motion_style == GiftMotionStyle::OncePerAppearance;
        let surface = self.media.as_ref().map(|media| {
            let motion = if self.once {
                GiftCosmeticMotion::Once
            } else {
                motion_style.motion(GiftCosmeticMotion::EFFECT_PREVIEW)
            };
            let poster = if self.rests_on_poster { media.cosmetic_poster(false) } else { None };
            let mut source = GiftCosmeticPlaybackSource::new(poster, media.cosmetic_movie(), motion, self.animated);
            source.rests_on_poster = self.rests_on_poster;
            source.poster_pixels = 786;
            source.replay = self.replay;
            source.replays_on_return = plays_per_appearance;
            source.fades_poster = plays_per_appearance;
            GiftCosmeticSurface::new(
                (self.id.clone(), "surface"),
                playback,
                source,
                GiftCosmeticGeometry::BANNER_ASPECT,
            )
            .part(GiftCosmeticPart::Whole)
            .placement(GiftCosmeticPlacement::Fill)
        });
        div().size_full().children(surface)
    }
}

/// a profile banner in small, the person cover or a neutral gradient with the effect over it
/// and the photo on top
/// the effect plays once each time the banner comes into view
#[derive(IntoElement)]
pub struct GiftProfileEffectBanner {
    id: ElementId,
    person: Option<GiftRecipient>,
    media: Option<RiseGiftMediaDTO>,
    animated: bool,
}

impl GiftProfileEffectBanner {
    pub fn new(id: impl Into<ElementId>, person: Option<GiftRecipient>, media: Option<RiseGiftMediaDTO>) -> Self {
        Self { id: id.into(), person, media, animated: true }
    }

    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }
}

impl RenderOnce for GiftProfileEffectBanner {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let measured = window.use_keyed_state((self.id.clone(), "width"), cx, |_, _| None::<f32>);
        let width = measured.read(cx).unwrap_or(0.0);
        let avatar = (width * 0.2).max(18.0);

        let cover_url = self
            .person
            .as_ref()
            .and_then(|person| person.cover.as_deref())
            .map(trimming_whitespaces_and_newlines)
            .filter(|cover| cover.to_lowercase().starts_with("http"));
        let cover = match cover_url {
            Some(cover) => div().absolute().inset_0().bg(gpui::black()).overflow_hidden().child(
                CleverImageUi::new(SharedString::from(cover.to_owned()))
                    .id((self.id.clone(), "cover"))
                    .content_mode(ContentMode::Fill)
                    .corner_radius(0.0)
                    .persist_to_disk(true)
                    .shows_progress_indicator(false)
                    .maximum_decode_pixel_dimension(((width * 3.0) as i64).max(256))
                    .shows_failure_icon(false),
            ),
            None => div().absolute().inset_0().child(
                LinearGradient::colors([theme.bg._400, theme.bg._200], UnitPoint::TOP_LEADING, UnitPoint::BOTTOM_TRAILING)
                    .view(),
            ),
        };

        let person = self.person.as_ref();
        let photo = UserLogoUi::new((self.id.clone(), "avatar"))
            .source(
                person
                    .map(|person| person.avatar.clone())
                    .filter(|avatar| !avatar.is_empty())
                    .map(SharedString::from),
            )
            .name(person.map(|person| SharedString::from(person.name.clone())))
            .user_id(person.map(|person| SharedString::from(person.id.clone())))
            .size(avatar)
            .border_color(theme.bg._100)
            .show_story_ring(false);

        let size_reader = measured.clone();
        div()
            .relative()
            .w_full()
            .aspect_ratio(GiftCosmeticGeometry::BANNER_ASPECT)
            .rounded(px(14.0))
            .overflow_hidden()
            .child(cover)
            .child(
                div().absolute().inset_0().child(
                    GiftProfileEffectView::new((self.id.clone(), "effect"), self.media).animated(self.animated),
                ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(width * 0.05))
                    .bottom(px(width * 0.05))
                    .child(photo),
            )
            .child(measure((self.id, "measure"), move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                size_reader.update(cx, |measured, cx| {
                    if *measured != Some(width) {
                        *measured = Some(width);
                        cx.notify();
                    }
                });
            }))
    }
}
