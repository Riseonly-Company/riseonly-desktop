use gpui::{
    black, div, px, white, App, Div, ElementId, Entity, FontWeight, IntoElement, ParentElement as _, RenderOnce,
    SharedString, Styled as _, Window,
};
use rise_theme::ThemeStore;
use rise_ui::swiftui::environment::env;

use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::gifts::components::gift_cosmetic_playback::{
    GiftCosmeticGeometry, GiftCosmeticMotion, GiftCosmeticPart, GiftCosmeticPlacement, GiftCosmeticPlayback,
    GiftCosmeticPlaybackSource, GiftCosmeticSurface, GiftMotionStyle, GiftMotionStyleKey,
};
use crate::modules::gifts::engine::rise_gifts_engine_models::{RiseGiftMediaDTO, RiseGiftMediaExt};
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::GiftRecipient;

#[derive(IntoElement)]
pub struct GiftNameplateView {
    id: ElementId,
    media: Option<RiseGiftMediaDTO>,
    effect: Option<RiseGiftMediaDTO>,
    animated: bool,
}

impl GiftNameplateView {
    pub fn new(id: impl Into<ElementId>, media: Option<RiseGiftMediaDTO>) -> Self {
        Self { id: id.into(), media, effect: None, animated: true }
    }

    pub fn effect(mut self, effect: Option<RiseGiftMediaDTO>) -> Self {
        self.effect = effect;
        self
    }

    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }

    fn surface(
        &self,
        id: ElementId,
        playback: Entity<GiftCosmeticPlayback>,
        media: &RiseGiftMediaDTO,
        motion_style: GiftMotionStyle,
    ) -> Div {
        let mut source = GiftCosmeticPlaybackSource::new(
            media.cosmetic_poster(false),
            media.cosmetic_movie(),
            motion_style.motion(GiftCosmeticMotion::Loop),
            self.animated,
        );
        source.poster_pixels = 784;
        source.replays_on_return = motion_style.replays_on_return();
        div().absolute().inset_0().child(
            GiftCosmeticSurface::new(id, playback, source, GiftCosmeticGeometry::PLATE_ASPECT)
                .part(GiftCosmeticPart::Whole)
                .placement(GiftCosmeticPlacement::TrailingFullHeight),
        )
    }
}

impl RenderOnce for GiftNameplateView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let motion_style = env::<GiftMotionStyleKey>(cx);
        let plate = GiftCosmeticPlayback::keyed((self.id.clone(), "plate"), window, cx);
        let glow = GiftCosmeticPlayback::keyed((self.id.clone(), "glow"), window, cx);
        let mut stack = div().relative().size_full();
        if let Some(effect) = &self.effect {
            stack = stack.child(self.surface((self.id.clone(), "glow-surface").into(), glow, effect, motion_style));
        }
        if let Some(media) = &self.media {
            stack = stack.child(self.surface((self.id.clone(), "plate-surface").into(), plate, media, motion_style));
        }
        stack
    }
}

/// a member row as the lists draw it, scaled to height over a nameplate
/// row keeps list proportions, 56 tall with a 40 photo without shows_person the row is empty
/// and only the nameplate shows
#[derive(IntoElement)]
pub struct GiftNameplateRow {
    id: ElementId,
    person: Option<GiftRecipient>,
    media: Option<RiseGiftMediaDTO>,
    effect: Option<RiseGiftMediaDTO>,
    height: f32,
    animated: bool,
    shows_person: bool,
}

impl GiftNameplateRow {
    pub fn new(id: impl Into<ElementId>, person: Option<GiftRecipient>, media: Option<RiseGiftMediaDTO>) -> Self {
        Self {
            id: id.into(),
            person,
            media,
            effect: None,
            height: GiftCosmeticGeometry::MEMBER_ROW_HEIGHT,
            animated: true,
            shows_person: true,
        }
    }

    pub fn effect(mut self, effect: Option<RiseGiftMediaDTO>) -> Self {
        self.effect = effect;
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }

    pub fn shows_person(mut self, shows_person: bool) -> Self {
        self.shows_person = shows_person;
        self
    }
}

impl RenderOnce for GiftNameplateRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let scale = self.height / GiftCosmeticGeometry::MEMBER_ROW_HEIGHT;
        let status = self
            .person
            .as_ref()
            .map(|person| if person.tag.is_empty() { String::new() } else { format!("@{}", person.tag) })
            .unwrap_or_default();
        let has_artwork = self.media.is_some() || self.effect.is_some();

        let mut row = div()
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0 * scale))
            .px(px(12.0 * scale))
            .h(px(self.height))
            .w_full();
        if self.shows_person {
            let person = self.person.as_ref();
            let avatar = person
                .map(|person| person.avatar.clone())
                .filter(|avatar| !avatar.is_empty())
                .map(SharedString::from);
            row = row.child(
                UserLogoUi::new((self.id.clone(), "avatar"))
                    .source(avatar)
                    .name(person.map(|person| SharedString::from(person.name.clone())))
                    .user_id(person.map(|person| SharedString::from(person.id.clone())))
                    .size(40.0 * scale)
                    .show_story_ring(false),
            );
            let name = person.map(|person| person.name.clone()).unwrap_or_default();
            let mut texts = div()
                .relative()
                .flex()
                .flex_col()
                .gap(px(2.0 * scale))
                .min_w_0();
            if has_artwork {
                texts = texts.child(
                    div()
                        .absolute()
                        .left(px(-4.0 * scale))
                        .right(px(-4.0 * scale))
                        .top(px(-2.0 * scale))
                        .bottom(px(-2.0 * scale))
                        .rounded(px(4.0 * scale))
                        .bg(black().opacity(0.7)),
                );
            }
            texts = texts.child(
                div()
                    .text_size(px(15.0 * scale))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(if has_artwork { white() } else { theme.text.primary })
                    .truncate()
                    .child(name),
            );
            if !status.is_empty() {
                texts = texts.child(
                    div()
                        .text_size(px(12.0 * scale))
                        .text_color(if has_artwork { white().opacity(0.9) } else { theme.text.secondary })
                        .truncate()
                        .child(status),
                );
            }
            row = row.child(texts);
        }

        div()
            .relative()
            .h(px(self.height))
            .w_full()
            .bg(theme.bg._100)
            .rounded(px(12.0 * scale))
            .overflow_hidden()
            .child(
                div().absolute().inset_0().child(
                    GiftNameplateView::new((self.id.clone(), "nameplate"), self.media)
                        .effect(self.effect)
                        .animated(self.animated),
                ),
            )
            .child(row)
    }
}
