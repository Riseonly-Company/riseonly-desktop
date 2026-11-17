use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Context, Div, ElementId, Entity, FontWeight, IntoElement, ParentElement, RenderOnce, Rgba, SharedString,
    Styled, Subscription, Window, div, px, white,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_theme::AppTheme;
use rise_theme::avatar_palette::AvatarPalette;
use rise_ui::IconUi;
use rise_ui::live_voice_bars_ui::LiveVoiceBarsUi;
use rise_ui::skeleton_ui::{SkeletonCircle, SkeletonShimmerExt};
use rise_ui::swiftui::font::{Font, SwiftUiFont};
use rise_ui::swiftui::minimum_scale_factor::MinimumScaleText;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke};

use crate::core::ui::clever_image_disk_cache::CleverImagePrefetchItem;
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi, ContentMode};
use crate::core::ui::user_logo_ui::StoryRingUi;
pub use crate::core::widgets::sheets::chat_favourites_logo_hooks::ChatFavouritesLogo;
use crate::modules::chat::engine::forum::rise_chat_voice_presence::RiseChatVoicePresentation;
use crate::modules::chat::stores::chats::chat_actions::chat_avatar_palette::AvatarPaletteChat;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::gifts::components::gift_worn_cosmetics::GiftWornCosmeticsModifiers;
use crate::modules::presence::stores::presence::presence_services::presence_services::{
    PresenceIndicatorProjection, PresenceIndicatorValue, PresenceServicesStore,
};
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::stores::user::user_actions::user_contact_wire_types::SavedContactDtoExt;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

/// offsets the corner badge, gpui has no offset so the corner is computed from the centre
fn at_corner(element: Div, logo_size: f32, side: f32, offset: f32) -> Div {
    let origin = (logo_size - side) / 2.0 + offset;
    element.absolute().left(px(origin)).top(px(origin)).size(px(side))
}

pub struct ChatLogoImagePolicy;

impl ChatLogoImagePolicy {
    pub fn descriptor(chat: &ChatInfo, size: f32, cx: &mut App) -> Option<CleverImagePrefetchItem> {
        if chat.r#type == ChatType::Favourites {
            return None;
        }
        if chat.r#type == ChatType::Direct
            && let Some(contact) =
                UserServicesStore::observed_saved_contact(&chat.participant.id, cx)
            && let Some(url) = AvatarPalette::resolved_logo_url(contact.avatar_override_url.as_deref())
        {
            return Some(CleverImagePrefetchItem {
                maximum_pixel_dimension: Some((size * 3.0).ceil() as i64),
                persist_to_disk: false,
                cache_policy: CleverImageCachePolicy::PrivateEphemeral,
                ..CleverImagePrefetchItem::new(url)
            });
        }
        let url = AvatarPalette::resolved_logo_url_for_chat(chat)?;
        Some(CleverImagePrefetchItem {
            maximum_pixel_dimension: Some((size * 3.0).ceil() as i64),
            ..CleverImagePrefetchItem::new(url)
        })
    }
}

#[derive(IntoElement)]
pub struct ChatLogo {
    id: ElementId,
    chat: ChatInfo,
    size: f32,
    icon_size: f32,
    show_online_indicator: bool,
    show_story_ring: bool,
    shows_worn_frame: bool,
    worn_frame_loops: bool,
    letter_fallback: Option<String>,
}

impl ChatLogo {
    pub fn new(id: impl Into<ElementId>, chat: ChatInfo) -> Self {
        Self {
            id: id.into(),
            chat,
            size: 50.0,
            icon_size: 18.0,
            show_online_indicator: true,
            show_story_ring: true,
            shows_worn_frame: false,
            worn_frame_loops: false,
            letter_fallback: None,
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn icon_size(mut self, icon_size: f32) -> Self {
        self.icon_size = icon_size;
        self
    }

    pub fn show_online_indicator(mut self, show_online_indicator: bool) -> Self {
        self.show_online_indicator = show_online_indicator;
        self
    }

    pub fn show_story_ring(mut self, show_story_ring: bool) -> Self {
        self.show_story_ring = show_story_ring;
        self
    }

    pub fn shows_worn_frame(mut self, shows_worn_frame: bool) -> Self {
        self.shows_worn_frame = shows_worn_frame;
        self
    }

    pub fn worn_frame_loops(mut self, worn_frame_loops: bool) -> Self {
        self.worn_frame_loops = worn_frame_loops;
        self
    }

    pub fn letter_fallback(mut self, name: impl Into<String>) -> Self {
        self.letter_fallback = Some(name.into());
        self
    }

    fn story_user_id(&self) -> Option<&str> {
        (self.chat.r#type == ChatType::Direct).then_some(self.chat.participant.id.as_str())
    }

    fn is_deleted_partner(&self) -> bool {
        self.chat.r#type == ChatType::Direct && self.chat.participant.is_deleted
    }

    fn has_active_stories(&self, stories: ChatLogoStoryValues) -> bool {
        !self.is_deleted_partner() && self.show_story_ring && stories.has_active
    }

    fn saved_alias(&self, cx: &mut App) -> Option<String> {
        if self.chat.r#type != ChatType::Direct {
            return None;
        }
        let contact = UserServicesStore::observed_saved_contact(&self.chat.participant.id, cx);
        let alias = contact
            .map(|contact| trimming_whitespaces_and_newlines(&contact.resolved_display_name()).to_owned())
            .unwrap_or_default();
        if alias.is_empty() { None } else { Some(alias) }
    }

    fn worn_frame_user_id(&self) -> Option<String> {
        if self.chat.r#type != ChatType::Direct || self.chat.is_bot_chat() || self.is_deleted_partner() {
            return None;
        }
        let id = &self.chat.participant.id;
        if id.is_empty() { None } else { Some(id.clone()) }
    }

    fn logo_body(&self, theme: &AppTheme, cx: &mut App) -> AnyElement {
        if self.chat.r#type == ChatType::Favourites {
            return self.favourites_logo();
        }
        if let Some(image) = ChatLogoImagePolicy::descriptor(&self.chat, self.size, cx) {
            return div()
                .relative()
                .size(px(self.size))
                .child(
                    div().size_full().clip_shape(Shape::Circle).child(
                        CleverImageUi::new(image.url_string)
                            .id(child_id(&self.id, "image"))
                            .content_mode(ContentMode::Fill)
                            .corner_radius(self.size / 2.0)
                            .persist_to_disk(image.persist_to_disk)
                            .shows_progress_indicator(false)
                            .maximum_decode_pixel_dimension(image.maximum_pixel_dimension)
                            .cache_policy(image.cache_policy),
                    ),
                )
                .child(centred_stroke(theme.border._100, 0.5, Shape::Circle))
                .into_any_element();
        }
        if self.is_deleted_partner() {
            return self.deleted_avatar(theme);
        }
        self.letter_avatar(theme, cx)
    }

    fn deleted_avatar(&self, theme: &AppTheme) -> AnyElement {
        div()
            .relative()
            .size(px(self.size))
            .flex()
            .items_center()
            .justify_center()
            .child(div().absolute().inset_0().rounded_full().bg(theme.bg._300))
            .children(IconUi::sized(
                "person.fill.questionmark",
                px((self.size * 0.38).max(9.0)),
                theme.text.secondary,
                true,
            ))
            .child(centred_stroke(theme.border._100, 0.5, Shape::Circle))
            .into_any_element()
    }

    fn favourites_logo(&self) -> AnyElement {
        ChatFavouritesLogo::new(self.size).icon_size(self.icon_size).into_any_element()
    }

    /// no name yet means the chat card hasnt loaded (deep link to an unseen chat)
    /// so a skeleton circle and not a hash colour with the ? letter
    fn has_letter_source(&self, cx: &mut App) -> bool {
        if self.saved_alias(cx).is_some() {
            return true;
        }
        let source = if self.chat.r#type == ChatType::Direct && !self.chat.has_no_direct_peer() {
            if self.chat.participant.name.is_empty() { &self.chat.participant.tag } else { &self.chat.participant.name }
        } else {
            &self.chat.title
        };
        !trimming_whitespaces_and_newlines(source).is_empty()
    }

    fn letter_avatar(&self, theme: &AppTheme, cx: &mut App) -> AnyElement {
        if self.has_letter_source(cx) || self.letter_fallback.is_some() {
            self.letter_circle(theme, cx)
        } else {
            SkeletonCircle::new(self.size).skeleton_shimmer(None).into_any_element()
        }
    }

    fn letter_circle(&self, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let letter = match self.saved_alias(cx) {
            Some(alias) => AvatarPalette::display_letter(Some(&alias)),
            None if !self.has_letter_source(cx) && self.letter_fallback.is_some() => {
                AvatarPalette::display_letter(self.letter_fallback.as_deref())
            }
            None => AvatarPalette::display_letter_for_chat(&self.chat),
        };
        let color = AvatarPalette::background_color(&self.chat);
        let font_size = (self.size * 0.46).max(11.0);

        div()
            .relative()
            .size(px(self.size))
            .flex()
            .items_center()
            .justify_center()
            .child(div().absolute().inset_0().rounded_full().bg(color))
            .child(
                div()
                    .relative()
                    .size(px(self.size * 0.72))
                    .flex()
                    .items_center()
                    .justify_center()
                    .swiftui_font(Font::system_weight(font_size, FontWeight::SEMIBOLD))
                    .text_color(AvatarPalette::letter_color_on(color))
                    .child(MinimumScaleText::new(letter, 0.55).line_limit(1)),
            )
            .child(centred_stroke(theme.border._100, 0.5, Shape::Circle))
            .into_any_element()
    }
}

impl RenderOnce for ChatLogo {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let size = self.size;
        let is_direct = self.chat.r#type == ChatType::Direct;

        let mut stories = ChatLogoStoryValues::default();
        if is_direct {
            let user_id = self.chat.participant.id.clone();
            let state = window.use_keyed_state(child_id(&self.id, "story-ring"), cx, {
                let user_id = user_id.clone();
                move |_, cx| ChatLogoStoryState::new(user_id, cx)
            });
            if state.read(cx).user_id != user_id {
                state.update(cx, |state, cx| {
                    state.values = ChatLogoStoryValues::read(&user_id, cx);
                    state.user_id = user_id;
                });
            }
            stories = state.read(cx).values;
        }

        let has_active_stories = self.has_active_stories(stories);
        let mut logo = div().relative().size(px(size)).child(self.logo_body(&theme, cx));
        if has_active_stories {
            logo = logo.child(StoryRingUi::new(size, stories.has_unseen, theme.clone()));
        }
        let logo = if self.shows_worn_frame {
            logo.gift_worn_frame(child_id(&self.id, "worn-frame"), self.worn_frame_user_id(), size)
                .loops(self.worn_frame_loops)
                .into_any_element()
        } else {
            logo.into_any_element()
        };

        let presence = (self.show_online_indicator && is_direct).then(|| {
            let identity = ChatLogoPresenceIndicatorIdentity {
                user_id: self.chat.participant.id.clone(),
                chat_id: self.chat.id.clone(),
                fallback_online: self.chat.participant.is_online,
            };
            ChatLogoPresenceIndicator::new(identity.element_id(&self.id), identity, size)
        });
        let voice = (self.show_online_indicator && self.chat.r#type == ChatType::Group).then(|| ChatLogoVoiceIndicator {
            id: child_id(&self.id, "voice"),
            chat_id: self.chat.id.clone(),
            fallback_live: self.chat.has_live_voice,
            size,
        });

        div()
            .relative()
            .size(px(size))
            .flex_none()
            .child(logo)
            .when_some(presence, |logo, indicator| logo.child(indicator))
            .when_some(voice, |logo, indicator| logo.child(indicator))
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ChatLogoPresenceIndicatorIdentity {
    user_id: String,
    chat_id: String,
    fallback_online: bool,
}

impl ChatLogoPresenceIndicatorIdentity {
    /// element id of the logo
    /// another person or chat or fallback starts from a fresh projection
    fn element_id(&self, parent: &ElementId) -> ElementId {
        child_id(parent, format!("presence-{}-{}-{}", self.user_id, self.chat_id, self.fallback_online))
    }
}

/// story ring of one private chat logo
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ChatLogoStoryValues {
    has_active: bool,
    has_unseen: bool,
}

impl ChatLogoStoryValues {
    fn read(user_id: &str, cx: &mut App) -> Self {
        let store = StoryServicesStore::shared(cx);
        let stories = store.read(cx);
        Self {
            has_active: stories.has_active_stories(Some(user_id), cx),
            has_unseen: stories.has_unseen_stories(Some(user_id), cx),
        }
    }
}

struct ChatLogoStoryState {
    user_id: String,
    values: ChatLogoStoryValues,
    _observer: Subscription,
}

impl ChatLogoStoryState {
    fn new(user_id: String, cx: &mut Context<Self>) -> Self {
        let stories = StoryServicesStore::shared(cx);
        let observer = cx.observe(&stories, |this, _, cx| {
            let values = ChatLogoStoryValues::read(&this.user_id, cx);
            if this.values != values {
                this.values = values;
                cx.notify();
            }
        });
        let values = ChatLogoStoryValues::read(&user_id, cx);
        Self { user_id, values, _observer: observer }
    }
}

struct ChatLogoPresenceIndicatorState {
    projection: Entity<PresenceIndicatorProjection>,
    _observer: Subscription,
}

#[derive(IntoElement)]
struct ChatLogoPresenceIndicator {
    id: ElementId,
    identity: ChatLogoPresenceIndicatorIdentity,
    size: f32,
}

impl ChatLogoPresenceIndicator {
    fn new(id: ElementId, identity: ChatLogoPresenceIndicatorIdentity, size: f32) -> Self {
        Self { id, identity, size }
    }
}

impl RenderOnce for ChatLogoPresenceIndicator {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let host = window.current_view();
        let identity = self.identity;
        let state = window.use_keyed_state(self.id, cx, move |_, cx| {
            let projection = PresenceServicesStore::shared(cx).update(cx, |store, cx| {
                store.indicator_projection(&identity.user_id, &identity.chat_id, identity.fallback_online, cx)
            });
            let observer = App::observe(cx, &projection, move |_, cx: &mut App| cx.notify(host));
            ChatLogoPresenceIndicatorState { projection, _observer: observer }
        });

        let value = state.read(cx).projection.read(cx).value();
        let theme = rise_ui::theme(cx);
        let size = self.size;
        match value {
            PresenceIndicatorValue::InChat => {
                let chrome = at_corner(div(), size, size * 0.36, size * 0.34)
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(theme.bg._100);
                chrome
                    .children(IconUi::sized("bubble.left.fill", px(size * 0.2), theme.semantic.success_100, true))
                    .into_any_element()
            }
            PresenceIndicatorValue::Online => at_corner(div(), size, size * 0.24, size * 0.36)
                .rounded_full()
                .bg(theme.semantic.success_100)
                .child(centred_stroke(theme.bg._100, 2.0, Shape::Circle))
                .into_any_element(),
            PresenceIndicatorValue::Hidden => div().into_any_element(),
        }
    }
}

fn color_multiply(color: gpui::Hsla, white_level: f32) -> gpui::Hsla {
    let rgb = color.to_rgb();
    Rgba { r: rgb.r * white_level, g: rgb.g * white_level, b: rgb.b * white_level, a: rgb.a }.into()
}

#[derive(IntoElement)]
struct ChatLogoVoiceIndicator {
    id: ElementId,
    chat_id: String,
    fallback_live: bool,
    size: f32,
}

impl RenderOnce for ChatLogoVoiceIndicator {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let host = window.current_view();
        let presentation = RiseChatVoicePresentation::shared(cx);
        let observed = presentation.clone();
        let chat_id = self.chat_id.clone();
        let _observer = window.use_keyed_state(child_id(&self.id, "observer"), cx, move |_, cx| {
            let mut drawn = observed.read(cx).is_live(&chat_id);
            App::observe(cx, &observed, move |presentation, cx: &mut App| {
                let live = presentation.read(cx).is_live(&chat_id);
                if live != drawn {
                    tracing::warn!(target: "riseonly::X1913", chat = %chat_id, ?drawn, ?live, "list badge voice changed");
                    drawn = live;
                    cx.notify(host);
                }
            })
        });

        let live = presentation.read(cx).is_live(&self.chat_id).unwrap_or(self.fallback_live);
        if !live {
            return div().into_any_element();
        }
        let theme = rise_ui::theme(cx);
        let size = self.size;
        at_corner(div(), size, size * 0.36, size * 0.34)
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(color_multiply(theme.semantic.success_100, 0.98))
            .child(LiveVoiceBarsUi::new(child_id(&self.id, "bars"), white()).width(size * 0.22).height(size * 0.2))
            .child(centred_stroke(theme.bg._100, 1.5, Shape::Circle))
            .into_any_element()
    }
}
