use std::collections::BTreeMap;
use std::sync::Arc;

use gpui::{
    App, Context, ElementId, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Subscription, TextRun, Window, div, px, white,
};
use rise_canon::rise_key::gift_colors;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_theme::theme::app_theme::alpha;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::swiftui::font::{Font, NATURAL_LINE_HEIGHT, SwiftUiFont};
use rise_ui::swiftui::frame::{EdgeSet, SwiftUiFrame};
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, use_task};
use rise_ui::swiftui::minimum_scale_factor::visual_line_count;
use rise_ui::swiftui::radial_gradient::RadialGradient;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes};
use rise_ui::swiftui::stacks::{Alignment, vstack, zstack};
use rise_ui::swiftui::transition::UnitPoint;

use crate::modules::chat::stores::message::message_actions::message_types::MessageSystemGiftItem;
use crate::modules::gifts::components::gift_animation_view::GiftAnimationView;
use crate::modules::gifts::components::gift_card_view::{GiftBackdropColors, GiftSerialFormat};
use crate::modules::gifts::components::gift_cosmetic_playback::GiftCosmeticGeometry;
use crate::modules::gifts::components::gift_cosmetic_preview::GiftCosmeticPreview;
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftAttributeDTO, RiseGiftAttributeKind, RiseGiftItemKind, RiseGiftMediaDTO, RiseGiftMediaKind,
    RiseGiftMediaSlot, RiseGiftRarityClass,
};
use crate::modules::gifts::shared::gift_people::GiftPeople;
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::{GiftRecipient, GiftsInteractionsStore};
use crate::modules::gifts::stores::gifts::gifts_services::gifts_services::GiftsServicesStore;

/// the card under a gift_sent pill, rolled gift on its backdrop with name serial and caption
/// a cosmetic is drawn on its recipient, frame around the photo or member row over the nameplate
#[derive(IntoElement)]
pub struct GiftMessageCard {
    id: ElementId,
    item: MessageSystemGiftItem,
}

impl GiftMessageCard {
    pub const WIDTH: f32 = 216.0;
    const CORNER_RADIUS: f32 = 20.0;
    const ANIMATION_SIDE: f32 = 120.0;
    const NAMEPLATE_HEIGHT: f32 = 48.0;
    const VERTICAL_PADDING: f32 = 14.0;
    const HORIZONTAL_PADDING: f32 = 14.0;
    const NAME_SPACING: f32 = 8.0;
    const NAME_HEIGHT: f32 = 20.0;
    const NAME_PLACEHOLDER_WIDTH: f32 = 110.0;
    const NAME_PLACEHOLDER_HEIGHT: f32 = 12.0;
    const CAPTION_SPACING: f32 = 4.0;
    const CAPTION_FONT_SIZE: f32 = 13.0;
    const CAPTION_LINE_LIMIT: f32 = 2.0;

    pub fn new(id: impl Into<ElementId>, item: MessageSystemGiftItem) -> Self {
        Self { id: id.into(), item }
    }

    /// caption font, system 13
    fn caption_font() -> Font {
        Font::system_size(Self::CAPTION_FONT_SIZE)
    }

    pub fn size(item: &MessageSystemGiftItem, window: &Window) -> gpui::Size<f32> {
        let fixed =
            Self::VERTICAL_PADDING * 2.0 + Self::preview_height(item) + Self::NAME_SPACING + Self::NAME_HEIGHT;
        gpui::Size {
            width: Self::WIDTH,
            height: (fixed + Self::caption_block_height(&item.caption, window)).ceil(),
        }
    }

    fn kind(item: &MessageSystemGiftItem) -> RiseGiftItemKind {
        RiseGiftItemKind::from_slug(&item.item_kind).unwrap_or(RiseGiftItemKind::Item)
    }

    /// the picture box, an item animation and a frame are square a nameplate is one row and an
    /// effect is the banner at the card inner width
    fn preview_height(item: &MessageSystemGiftItem) -> f32 {
        match Self::kind(item) {
            RiseGiftItemKind::Item | RiseGiftItemKind::AvatarFrame => Self::ANIMATION_SIDE,
            RiseGiftItemKind::Nameplate => Self::NAMEPLATE_HEIGHT,
            RiseGiftItemKind::ProfileEffect => {
                ((Self::WIDTH - Self::HORIZONTAL_PADDING * 2.0) / GiftCosmeticGeometry::BANNER_ASPECT).ceil()
            }
        }
    }

    fn trimmed_caption(caption: &str) -> String {
        trimming_whitespaces_and_newlines(caption).to_owned()
    }

    fn caption_text_height(caption: &str, window: &Window) -> f32 {
        let text = Self::trimmed_caption(caption);
        if text.is_empty() {
            return 0.0;
        }
        let font = Self::caption_font();
        let run = TextRun {
            len: text.len(),
            font: gpui::Font {
                family: font.family(),
                features: font.features(),
                fallbacks: None,
                weight: font.weight,
                style: font.style(),
            },
            color: gpui::Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line_height = Self::CAPTION_FONT_SIZE * NATURAL_LINE_HEIGHT;
        let lines = window
            .text_system()
            .shape_text(
                SharedString::from(text),
                px(font.size),
                &[run],
                Some(px(Self::WIDTH - Self::HORIZONTAL_PADDING * 2.0)),
                None,
            )
            .map(|lines| visual_line_count(&lines))
            .unwrap_or(1);
        (lines as f32 * line_height).ceil().min((line_height * Self::CAPTION_LINE_LIMIT).ceil())
    }

    fn caption_block_height(caption: &str, window: &Window) -> f32 {
        let height = Self::caption_text_height(caption, window);
        if height > 0.0 { Self::CAPTION_SPACING + height } else { 0.0 }
    }

    /// the message carries urls not slots, the poster and the small movie when it is one
    /// the announcement falls back to a lottie file which this player cant play, a cosmetic uses
    /// its own poster and hevc
    fn media(&self) -> RiseGiftMediaDTO {
        let mut slots = BTreeMap::new();
        let movie = self.item.animation_url.to_lowercase();
        let is_movie = movie.ends_with(".mov") || movie.ends_with(".mp4");
        let (media_kind, poster_slot, movie_slot) = match Self::kind(&self.item) {
            RiseGiftItemKind::Item => (RiseGiftMediaKind::Video, RiseGiftMediaSlot::Poster256, RiseGiftMediaSlot::Hevc256),
            RiseGiftItemKind::AvatarFrame => (
                RiseGiftMediaKind::FrameVideo,
                RiseGiftMediaSlot::FramePosterSmall,
                RiseGiftMediaSlot::FrameHevc,
            ),
            RiseGiftItemKind::Nameplate => {
                (RiseGiftMediaKind::PlateVideo, RiseGiftMediaSlot::PlatePoster, RiseGiftMediaSlot::PlateHevc)
            }
            RiseGiftItemKind::ProfileEffect => {
                (RiseGiftMediaKind::BannerVideo, RiseGiftMediaSlot::BannerPoster, RiseGiftMediaSlot::BannerHevc)
            }
        };
        if !self.item.poster_url.is_empty() {
            slots.insert(poster_slot.slug().to_owned(), self.item.poster_url.clone());
        }
        if is_movie {
            slots.insert(movie_slot.slug().to_owned(), self.item.animation_url.clone());
        }
        let kind = if Self::kind(&self.item) == RiseGiftItemKind::Item {
            RiseGiftAttributeKind::Model.slug()
        } else {
            media_kind.slug()
        };
        RiseGiftMediaDTO { kind: kind.to_owned(), slots }
    }

    fn backdrop(&self) -> Option<RiseGiftAttributeDTO> {
        if !self.item.backdrop.contains_key(gift_colors::CENTER) || !self.item.backdrop.contains_key(gift_colors::EDGE) {
            return None;
        }
        Some(RiseGiftAttributeDTO {
            colors: self.item.backdrop.iter().map(|(key, value)| (key.clone(), value.clone())).collect(),
            kind: RiseGiftAttributeKind::Backdrop.slug().to_owned(),
            ..RiseGiftAttributeDTO::default()
        })
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }
}

impl RenderOnce for GiftMessageCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let size = Self::size(&self.item, window);
        let rarity = RiseGiftRarityClass::from_slug(&self.item.rarity).unwrap_or(RiseGiftRarityClass::Common);
        let backdrop = self.backdrop();
        let colors = GiftBackdropColors::resolve(backdrop.as_ref(), rarity);
        let text_color = if backdrop.is_none() { theme.text.primary } else { white() };
        let caption = Self::trimmed_caption(&self.item.caption);
        let shape = Shape::continuous(Self::CORNER_RADIUS);
        let kind = Self::kind(&self.item);

        let recipient_state = window.use_keyed_state(self.child_id("recipient"), cx, |_, _| None::<GiftRecipient>);
        let recipient = recipient_state.read(cx).clone();
        let task_state = recipient_state.clone();
        let recipient_id = self.item.recipient_id.clone();
        use_task(self.child_id("recipient-task"), self.item.recipient_id.clone(), window, cx, async move |cx| {
            if kind == RiseGiftItemKind::Item {
                return;
            }
            let person = cx.update(|_, cx| GiftPeople::person(&recipient_id, cx)).ok().flatten();
            task_state.update(cx, |recipient, cx| {
                *recipient = person;
                cx.notify();
            });
        });

        let media = self.media();
        let preview = match kind {
            RiseGiftItemKind::Item => div()
                .frame_size(Self::ANIMATION_SIDE, Self::ANIMATION_SIDE)
                .child(GiftAnimationView::new(self.child_id("animation"), Some(media)).large(false).animated(true))
                .into_any_element(),
            RiseGiftItemKind::AvatarFrame | RiseGiftItemKind::Nameplate | RiseGiftItemKind::ProfileEffect => {
                GiftCosmeticPreview::new(self.child_id("cosmetic"), kind, recipient, Some(media)).into_any_element()
            }
        };

        let name_state = {
            let gift_id = self.item.gift_id;
            let fallback = self.item.name.clone();
            window.use_keyed_state(self.child_id("name"), cx, move |_, cx| {
                GiftMessageCardName::new(gift_id, fallback, cx)
            })
        };
        if !name_state.read(cx).is_for(self.item.gift_id, &self.item.name) {
            let (gift_id, fallback) = (self.item.gift_id, self.item.name.clone());
            name_state.update(cx, |state, cx| state.retarget(gift_id, fallback, cx));
        }
        let name = name_state.read(cx).answer.clone();
        let name_row = div()
            .h(px(Self::NAME_HEIGHT))
            .mt(px(Self::NAME_SPACING))
            .swiftui_font(Font::system_weight(15.0, FontWeight::SEMIBOLD))
            .whitespace_nowrap()
            .overflow_hidden()
            .text_ellipsis();
        let name_row = match name {
            Some(name) => name_row.child(SharedString::from(name)),
            None => name_row.flex().items_center().justify_center().child(
                div()
                    .w(px(Self::NAME_PLACEHOLDER_WIDTH))
                    .h(px(Self::NAME_PLACEHOLDER_HEIGHT))
                    .rounded(px(Self::NAME_PLACEHOLDER_HEIGHT * 0.5))
                    .bg(alpha(text_color, 0.18)),
            ),
        };
        let mut column = vstack(0.0)
            .child(div().w_full().h(px(Self::preview_height(&self.item))).flex().justify_center().child(preview))
            .child(name_row);
        if !caption.is_empty() {
            let caption_height = Self::caption_text_height(&caption, window);
            column = column.child(
                div()
                    .h(px(caption_height))
                    .mt(px(Self::CAPTION_SPACING))
                    .overflow_hidden()
                    .swiftui_font(Self::caption_font())
                    .text_center()
                    .line_clamp(Self::CAPTION_LINE_LIMIT as usize)
                    .opacity(0.85)
                    .child(SharedString::from(caption)),
            );
        }
        let column = column
            .text_color(text_color)
            .padding_edges(EdgeSet::HORIZONTAL, Self::HORIZONTAL_PADDING)
            .padding_edges(EdgeSet::VERTICAL, Self::VERTICAL_PADDING)
            .frame_size(size.width, size.height)
            .justify_start();

        let mut background = shape.round(div().absolute().inset_0());
        if backdrop.is_none() {
            background = background.bg(theme.bg._200);
        }
        let background = background.child(
            RadialGradient::new([colors.center, colors.edge], UnitPoint::new(0.5, 0.36), 0.0, 150.0)
                .view_rounded(Self::CORNER_RADIUS),
        );

        let serial = GiftSerialFormat::ribbon(self.item.serial, Some(self.item.supply_total));
        let card = zstack(Alignment::TopTrailing)
            .frame_size(size.width, size.height)
            .child(div().relative().size_full().child(background).child(column))
            .child(
                div().p(px(8.0)).child(
                    div()
                        .swiftui_font(Font::system_weight(10.0, FontWeight::SEMIBOLD).monospaced_digit())
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_color(colors.ribbon_ink)
                        .px(px(7.0))
                        .py(px(3.0))
                        .capsule()
                        .bg(colors.ribbon)
                        .child(SharedString::from(serial)),
                ),
            )
            .clip_shape(shape);

        let instance_id = self.item.instance_id.clone();
        let gift_id = self.item.gift_id;
        let stroke = shape.round(div().absolute().inset_0()).stroke_border(alpha(white(), 0.14), 1.0);
        div()
            .id(self.id.clone())
            .relative()
            .frame_size(size.width, size.height)
            .cursor_pointer()
            .child(card)
            .child(stroke)
            .on_click(move |_, _, cx| {
                let id = instance_id.clone();
                GiftsInteractionsStore::shared(cx).update(cx, |store, cx| store.open_instance_id(id, cx));
            })
            .on_appear(self.child_id("appear"), move |_, cx| {
                GiftsInteractionsStore::shared(cx).update(cx, |store, cx| store.localize_chat_card_name(gift_id, cx));
            })
    }
}

struct GiftMessageCardName {
    gift_id: i64,
    fallback: String,
    answer: Option<String>,
    _services: Subscription,
}

impl GiftMessageCardName {
    fn new(gift_id: i64, fallback: String, cx: &mut Context<Self>) -> Self {
        let services = GiftsServicesStore::shared(cx);
        let answer = Self::resolve(&services, gift_id, &fallback, cx);
        let observation = cx.observe(&services, |state: &mut Self, services, cx| {
            let next = Self::resolve(&services, state.gift_id, &state.fallback, cx);
            if next != state.answer {
                state.answer = next;
                cx.notify();
            }
        });
        Self { gift_id, fallback, answer, _services: observation }
    }

    fn is_for(&self, gift_id: i64, fallback: &str) -> bool {
        self.gift_id == gift_id && self.fallback == fallback
    }

    fn retarget(&mut self, gift_id: i64, fallback: String, cx: &mut Context<Self>) {
        let services = GiftsServicesStore::shared(cx);
        self.answer = Self::resolve(&services, gift_id, &fallback, cx);
        self.gift_id = gift_id;
        self.fallback = fallback;
    }

    fn resolve(services: &Entity<GiftsServicesStore>, gift_id: i64, fallback: &str, cx: &App) -> Option<String> {
        let services = services.read(cx);
        match services.localized_name(gift_id, cx) {
            Some(name) => Some(name),
            None if gift_id <= 0
                || services.catalog_details(cx).contains_key(&gift_id)
                || services.catalog_details_failed(cx).contains(&gift_id) =>
            {
                Some(fallback.to_owned())
            }
            None => None,
        }
    }
}
