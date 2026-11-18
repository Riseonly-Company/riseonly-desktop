use gpui::{
    App, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString, Styled,
    Window, div, prelude::FluentBuilder, px,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::official_badge::OfficialBadge;
use rise_ui::swiftui::lifecycle::use_task;

use crate::core::ui::custom_emoji_glyph::{CustomEmojiGlyph, CustomEmojiGlyphMetrics};
use crate::core::ui::custom_emoji_resource_warmup::CustomEmojiResourceWarmup;
use crate::core::ui::custom_emoji_stickers_hooks::custom_emoji_stickers_hooks;
use crate::core::ui::premium_auth_hooks::PremiumAuthHooks;
use crate::core::ui::premium_diamond::PremiumDiamond;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::modules::chat::stores::message::message_actions::message_types::MessageReaction;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerAssetType;

/// my badge right now lives only in the live session profile
/// all else is a wire decoded snapshot eg post author or member row, an optimistic write cant reach it
/// so the answer is given here once and a row only names the owner
pub enum PremiumBadgeSource {}

impl PremiumBadgeSource {
    pub fn emoji(user_id: Option<&str>, payload: Option<&str>, cx: &App) -> Option<String> {
        let Some(user_id) = user_id.filter(|user_id| !user_id.is_empty()) else {
            return payload.map(str::to_string);
        };
        if !PremiumAuthHooks::is_current_user(user_id, cx) {
            return payload.map(str::to_string);
        }
        let Some(profile) = PremiumAuthHooks::profile(cx) else {
            return payload.map(str::to_string);
        };
        profile.more.premium_badge_emoji
    }

    /// same rule for the tier, my own row is answered by the session profile
    /// subscription_changed patches it the moment a plan or gift lands, not by an older snapshot
    pub fn is_premium(user_id: Option<&str>, fallback: bool, cx: &App) -> bool {
        let Some(user_id) = user_id.filter(|user_id| !user_id.is_empty()) else {
            return fallback;
        };
        if !PremiumAuthHooks::is_current_user(user_id, cx) {
            return fallback;
        }
        match PremiumAuthHooks::profile(cx) {
            Some(profile) => profile.more.is_premium,
            None => fallback,
        }
    }
}

/// badge geometry, the default mark paints about 83% of its square canvas
pub enum PremiumBadgeMetrics {}

impl PremiumBadgeMetrics {
    pub const CUSTOM_GLYPH_CANVAS_RATIO: f32 = 1.15;

    pub const MAXIMUM_ASPECT: f32 = 1.6;
}

/// premium marker next to a user name
/// subscribers may swap the default mark for a unicode grapheme or a custom emoji as ce:sticker_item_id
/// same prefix as reactions unknown or unresolvable ones fall back to the default
#[derive(IntoElement)]
pub struct PremiumBadge {
    pub user_id: Option<SharedString>,
    pub emoji: Option<SharedString>,
    pub size: f32,
    pub animated: bool,
}

impl Default for PremiumBadge {
    fn default() -> Self {
        Self {
            user_id: None,
            emoji: None,
            size: 16.0,
            animated: true,
        }
    }
}

impl PremiumBadge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn user_id(mut self, user_id: Option<SharedString>) -> Self {
        self.user_id = user_id;
        self
    }

    pub fn emoji(mut self, emoji: Option<SharedString>) -> Self {
        self.emoji = emoji;
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }

    fn content(&self, value: &str, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
        let size = self.size;
        if value.is_empty() {
            return PremiumDiamond::new().size(size).looping(false).into_any_element();
        }
        if let Some(custom_emoji_id) = value.strip_prefix(MessageReaction::CUSTOM_EMOJI_PREFIX) {
            if let Some(item) = custom_emoji_stickers_hooks().and_then(|hooks| (hooks.custom_emoji_item)(custom_emoji_id, cx)) {
                let aspect = CustomEmojiGlyphMetrics::aspect(
                    item.width,
                    item.height,
                    item.asset_type == StickerAssetType::Lottie,
                )
                .min(PremiumBadgeMetrics::MAXIMUM_ASPECT);
                return div()
                    .w(px((size * aspect.max(1.0)).round()))
                    .h(px(size))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        CustomEmojiGlyph::new(item, size * PremiumBadgeMetrics::CUSTOM_GLYPH_CANVAS_RATIO)
                            .maximum_aspect(PremiumBadgeMetrics::MAXIMUM_ASPECT)
                            .is_playing(self.animated)
                            .loops(false),
                    )
                    .into_any_element();
            }
            let warmup = CustomEmojiResourceWarmup::shared(cx);
            let revision = warmup.read(cx).resource_revision(cx);
            let unresolved = warmup.read(cx).is_unresolved_custom_emoji(custom_emoji_id);
            let id = custom_emoji_id.to_string();
            use_task(
                ElementId::Name(SharedString::from(format!("premium_badge_warmup/{id}"))),
                revision,
                window,
                cx,
                async move |cx| {
                    let _ = warmup.update(cx, |warmup, cx| warmup.ensure_custom_emoji(&id, cx));
                },
            );
            if unresolved {
                return PremiumDiamond::new().size(size).looping(false).into_any_element();
            }
            return div().w(px(size)).h(px(size)).into_any_element();
        }
        div()
            .w(px(size))
            .h(px(size))
            .flex()
            .items_center()
            .justify_center()
            .child(
                MainText::new(value.to_string())
                    .px(size * 0.86)
                    .minimum_scale_factor(0.5),
            )
            .into_any_element()
    }
}

impl RenderOnce for PremiumBadge {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let source = PremiumBadgeSource::emoji(self.user_id.as_deref(), self.emoji.as_deref(), cx).unwrap_or_default();
        let value = trimming_whitespaces_and_newlines(&source).to_string();
        let content = self.content(&value, window, cx);
        div()
            .id(ElementId::Name(SharedString::from(format!("premium_badge/{value}"))))
            .h(px(self.size))
            .flex_none()
            .flex()
            .items_center()
            .child(content)
    }
}

/// name in its premium color then the official and premium marks, one look in every people list
#[derive(IntoElement)]
pub struct UserNameWithBadges {
    user_id: Option<SharedString>,
    name: SharedString,
    is_official: bool,
    is_premium: bool,
    emoji: Option<SharedString>,
    name_color: Option<SharedString>,
    font_size: f32,
    font_weight: FontWeight,
    badge_size: f32,
}

impl UserNameWithBadges {
    pub fn new(user_id: Option<SharedString>, name: impl Into<SharedString>) -> Self {
        Self {
            user_id,
            name: name.into(),
            is_official: false,
            is_premium: false,
            emoji: None,
            name_color: None,
            font_size: 16.0,
            font_weight: FontWeight::SEMIBOLD,
            badge_size: 14.0,
        }
    }

    pub fn official(mut self, is_official: bool) -> Self {
        self.is_official = is_official;
        self
    }

    pub fn premium(mut self, is_premium: bool, emoji: Option<String>, name_color: Option<String>) -> Self {
        self.is_premium = is_premium;
        self.emoji = emoji.map(SharedString::from);
        self.name_color = name_color.map(SharedString::from);
        self
    }

    pub fn font(mut self, size: f32, weight: FontWeight) -> Self {
        self.font_size = size;
        self.font_weight = weight;
        self
    }

    pub fn badge_size(mut self, size: f32) -> Self {
        self.badge_size = size;
        self
    }
}

impl RenderOnce for UserNameWithBadges {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let user_id = self.user_id.as_deref().filter(|user_id| !user_id.is_empty());
        let is_premium = PremiumBadgeSource::is_premium(user_id, self.is_premium, cx);
        let is_own = user_id.is_some_and(|user_id| PremiumAuthHooks::is_current_user(user_id, cx));
        let color = if is_own {
            PremiumNameColor::color_for_user(user_id, None, cx)
        } else if is_premium {
            PremiumNameColor::parse(self.name_color.as_deref())
        } else {
            None
        };
        let badge_size = self.badge_size;
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .min_w_0()
            .max_w_full()
            .child(
                div().min_w_0().overflow_hidden().child(
                    MainText::new(self.name)
                        .px(self.font_size)
                        .font_weight(self.font_weight)
                        .tac(TextAlignT::Left)
                        .number_of_lines(1)
                        .color(color),
                ),
            )
            .when(self.is_official, |line| {
                line.child(div().flex_none().child(OfficialBadge::new().size(px(badge_size))))
            })
            .when(is_premium, |line| {
                line.child(PremiumBadge::new().user_id(self.user_id.clone()).emoji(self.emoji.clone()).size(badge_size))
            })
    }
}
