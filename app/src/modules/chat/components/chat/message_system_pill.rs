use std::collections::{HashMap, HashSet};

use gpui::{App, ElementId, FontWeight, Hsla, SharedString, Size, Window, white};
use rise_core::foundation::foundation_text::{trimming_whitespaces, trimming_whitespaces_and_newlines};
use rise_i18n::app_language_catalog::AppPluralRules;
use rise_i18n::{AppPluralSelector, FormatArg, I18n, PluralCategory, format_printf, tr};
use rise_i18n_catalog::MISSING_PLURAL_CATEGORY_FALLBACK;
use rise_theme::AppTheme;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::shared::try_read;
use rise_ui::swiftui::button_style::{ButtonAction, plain};
use rise_ui::swiftui::prelude::*;
use rise_ui::{IconUi, MainText, TextAlignT};

use crate::core::ui::premium_diamond::PremiumDiamond;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatDecorativeBackdrop;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_sheet::WallpaperTileMessages;
use crate::modules::chat::components::chat::message_system_content_parser::MessageSystemContentParser;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatType;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageSystemGiftItem, MessageSystemKind, MessageSystemWallpaperPreview,
};
use crate::modules::gifts::components::gift_message_card::GiftMessageCard;

fn system_text_color(theme: &AppTheme) -> Hsla {
    let is_dark = theme.bg._100 == gpui::black();
    if is_dark { white().opacity(0.95) } else { theme.text.primary }
}

#[derive(IntoElement)]
pub struct MessageSystemPill {
    text: SharedString,
    wallpaper_preview: Option<MessageSystemWallpaperPreview>,
    shows_gift: bool,
    gift_item: Option<MessageSystemGiftItem>,
}

impl MessageSystemPill {
    pub const PREVIEW_SIZE: Size<f32> = Size { width: 112.0, height: 150.0 };
    pub const LINE_LIMIT: usize = 4;

    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            wallpaper_preview: None,
            shows_gift: false,
            gift_item: None,
        }
    }

    pub fn wallpaper_preview(mut self, wallpaper_preview: Option<MessageSystemWallpaperPreview>) -> Self {
        self.wallpaper_preview = wallpaper_preview;
        self
    }

    pub fn shows_gift(mut self, shows_gift: bool) -> Self {
        self.shows_gift = shows_gift;
        self
    }

    pub fn gift_item(mut self, gift_item: Option<MessageSystemGiftItem>) -> Self {
        self.gift_item = gift_item;
        self
    }
}

impl RenderOnce for MessageSystemPill {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let side = Self::PREVIEW_SIZE;
        let preview = self.wallpaper_preview.map(|preview| {
            let backdrop_id = SharedString::from(format!("message-system-wallpaper-{}", preview.image_url));
            div()
                .relative()
                .flex_none()
                .w(px(side.width))
                .h(px(side.height))
                .child(
                    div().absolute().inset_0().overflow_hidden().continuous_corners(14.0).child(
                        ChatDecorativeBackdrop::new()
                            .id(backdrop_id)
                            .ignores_safe_area(false)
                            .preview_image_url(Some(preview.image_url))
                            .preview_look(Some(preview.look))
                            .animates(false),
                    ),
                )
                .child(div().absolute().inset_0().p(px(4.0)).child(WallpaperTileMessages::new().scale(0.6)))
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .continuous_corners(14.0)
                        .border(px(1.0))
                        .border_color(white().opacity(0.18)),
                )
        });
        let gift_card_id = self.text.clone();
        let gift = self.gift_item.map(|item| {
            GiftMessageCard::new(SharedString::from(format!("message-system-gift-{}", item.instance_id)), item)
        });

        div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.0))
            .py(px(4.0))
            .child(
                div().w_full().flex().flex_row().justify_center().child(
                    div().min_w_0().max_w_full().child(
                        MainText::new(self.text)
                            .px(MessageSizes::system_pill_font_size())
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(system_text_color(&theme))
                            .number_of_lines(Self::LINE_LIMIT)
                            .tac(TextAlignT::Center)
                            .liquid_glass_nav_pill(12.0, Some(1.0)),
                    ),
                ),
            )
            .children(preview)
            .when(self.shows_gift, |column| {
                column.child(MessageSystemGiftCard::new(SharedString::from(format!(
                    "message-system-diamond-{gift_card_id}"
                ))))
            })
            .children(gift)
    }
}

/// looping diamond under a gifted premium pill
/// fixed size so the row height is known before anything mounts
#[derive(IntoElement)]
pub struct MessageSystemGiftCard {
    id: ElementId,
}

impl MessageSystemGiftCard {
    pub const SIZE: Size<f32> = Size { width: 96.0, height: 96.0 };

    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into() }
    }
}

impl RenderOnce for MessageSystemGiftCard {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .id(self.id)
            .w(px(Self::SIZE.width))
            .h(px(Self::SIZE.height))
            .child(PremiumDiamond::new().size(Self::SIZE.width).looping(true))
    }
}

#[derive(IntoElement)]
pub struct MessageTopicDivider {
    title: SharedString,
    on_tap: ButtonAction,
}

impl MessageTopicDivider {
    pub const LINE_LIMIT: usize = 3;

    pub fn new(title: impl Into<SharedString>, on_tap: ButtonAction) -> Self {
        Self {
            title: title.into(),
            on_tap,
        }
    }
}

impl RenderOnce for MessageTopicDivider {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let color = system_text_color(&theme);
        let on_tap = self.on_tap;
        let button_id = SharedString::from(format!("message-topic-divider-{}", self.title));

        div().w_full().flex().flex_row().justify_center().py(px(4.0)).child(
            plain(button_id).on_press(move |window, cx| on_tap(window, cx)).child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(5.0))
                    .children(IconUi::sized("bubble.left.fill", px(10.0), color, true))
                    .child(
                        MainText::new(self.title)
                            .px(MessageSizes::system_pill_font_size())
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(color)
                            .number_of_lines(Self::LINE_LIMIT)
                            .tac(TextAlignT::Center),
                    )
                    .child(div().opacity(0.55).children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(9.0), color, false)))
                    .liquid_glass_nav_pill(12.0, Some(1.0)),
            ),
        )
    }
}

const GIFT_MONTHS_KEY: &str = "premium_gift_months";

const MESSAGE_SYSTEM_FORMAT_KEYS: &[&str] = &[
    "msg_sys_user_joined",
    "msg_sys_user_joined_you",
    "msg_sys_user_left",
    "msg_sys_user_left_you",
    "msg_sys_user_added_generic",
    "msg_sys_user_added_generic_you",
    "msg_sys_user_added_target",
    "msg_sys_user_added_target_you",
    "msg_sys_user_removed_generic",
    "msg_sys_user_removed_generic_you",
    "msg_sys_user_removed_target",
    "msg_sys_user_removed_target_you",
    "msg_sys_user_banned_generic",
    "msg_sys_user_banned_generic_you",
    "msg_sys_user_banned_target",
    "msg_sys_user_banned_target_you",
    "group_created_message",
    "msg_sys_chat_renamed",
    "msg_sys_chat_renamed_you",
    "msg_sys_chat_renamed_to",
    "msg_sys_chat_renamed_to_you",
    "msg_sys_chat_photo_changed",
    "msg_sys_chat_photo_changed_you",
    "msg_sys_chat_photo_removed",
    "msg_sys_chat_photo_removed_you",
    "msg_sys_chat_wallpaper_changed",
    "msg_sys_chat_wallpaper_changed_you",
    "msg_sys_chat_wallpaper_removed",
    "msg_sys_chat_wallpaper_removed_you",
    "msg_sys_message_pinned_you",
    "msg_sys_message_pinned",
    "msg_sys_message_unpinned",
    "msg_sys_message_unpinned_you",
    "msg_sys_voice_call_started",
    "msg_sys_voice_call_ended",
    "msg_sys_video_call_started",
    "msg_sys_video_call_ended",
    "msg_sys_secret_chat_started",
    "msg_sys_secret_chat_ttl_changed",
    "chat_autodelete_off",
    "chat_autodelete_1d",
    "chat_autodelete_1w",
    "chat_autodelete_1m",
    "channel_created_message",
    "chat_topic_created_system",
    "contact_joined_system_message",
    "msg_sys_migrated_from_group",
    "msg_sys_score_reached",
    "msg_sys_premium_gifted_to_you",
    "msg_sys_premium_gifted_by_you",
    "msg_sys_premium_gifted",
    "msg_sys_premium_gifted_short",
    "msg_sys_gift_sent_by_you",
    "msg_sys_gift_sent_to_you",
    "msg_sys_gift_sent_short",
];

fn format_text(template: &str, value: &str) -> String {
    format_printf(template, &[FormatArg::Text(value.into())])
}

fn format_texts(template: &str, first: &str, second: &str) -> String {
    format_printf(template, &[FormatArg::Text(first.into()), FormatArg::Text(second.into())])
}

#[derive(Clone)]
pub struct MessageSystemFormatContext {
    pub current_user_ids: HashSet<String>,
    pub strings: HashMap<String, String>,
    pub plural_rules: AppPluralRules,
    pub gift_months_templates: HashMap<PluralCategory, String>,
    pub channel_title: Option<String>,
}

impl MessageSystemFormatContext {
    /// channel title of chat_id when that chat is a channel
    pub fn acting_channel(mut self, chat_id: &str, cx: &App) -> Self {
        self.channel_title = try_read::<ChatServicesStore>(cx)
            .and_then(|chats| chats.chat_info(chat_id, cx))
            .filter(|chat| chat.r#type == ChatType::Channel)
            .map(|chat| trimming_whitespaces_and_newlines(&chat.title).to_owned())
            .filter(|title| !title.is_empty());
        self
    }

    /// ids the signed in person goes by, without building the whole string table
    pub fn current_user_ids_now(cx: &App) -> HashSet<String> {
        let mut current_user_ids = HashSet::new();
        if let Some(auth) = try_read::<AuthServicesStore>(cx) {
            let ids = [auth.profile().map(|profile| profile.id.clone()), auth.effective_user_id_for_wire()];
            for value in ids.into_iter().flatten() {
                let normalized = trimming_whitespaces_and_newlines(&value);
                if !normalized.is_empty() {
                    current_user_ids.insert(normalized.to_owned());
                }
            }
        }
        current_user_ids
    }

    pub fn current(cx: &App) -> Self {
        let current_user_ids = Self::current_user_ids_now(cx);
        let mut gift_months_templates = HashMap::new();
        for category in PluralCategory::ALL {
            let key = format!("{GIFT_MONTHS_KEY}_{}", category.as_str());
            let template = tr(&key);
            if template != key {
                gift_months_templates.insert(category, template);
            }
        }
        Self {
            current_user_ids,
            strings: MESSAGE_SYSTEM_FORMAT_KEYS.iter().map(|key| ((*key).to_owned(), tr(key))).collect(),
            plural_rules: I18n::shared().plural_rules(),
            gift_months_templates,
            channel_title: None,
        }
    }

    pub fn string(&self, key: &str) -> String {
        self.strings.get(key).cloned().unwrap_or_else(|| key.to_owned())
    }

    pub fn is_current_user(&self, id: Option<&str>) -> bool {
        let Some(id) = id else {
            return false;
        };
        self.current_user_ids.contains(trimming_whitespaces_and_newlines(id))
    }

    pub fn gift_months(&self, months: i64) -> String {
        let category = AppPluralSelector::category(self.plural_rules, months);
        let template = self
            .gift_months_templates
            .get(&category)
            .or_else(|| self.gift_months_templates.get(&MISSING_PLURAL_CATEGORY_FALLBACK))
            .map_or("%d", String::as_str);
        format_printf(template, &[FormatArg::Int(months)])
    }

    pub fn localized_creation_content(&self, content: &str) -> Option<String> {
        Self::creation_key(content).map(|key| self.string(key))
    }

    /// locale key of the created plate an older notice spells as an english sentence
    pub fn creation_key(content: &str) -> Option<&'static str> {
        let trimmed = trimming_whitespaces_and_newlines(content);
        if trimmed.is_empty() {
            return None;
        }
        let parts: Vec<&str> = trimmed.split('\'').collect();
        if parts.len() >= 2 {
            let first = trimming_whitespaces(parts[0]);
            let last = trimming_whitespaces(parts[parts.len() - 1]);
            match format!("{first} {last}").as_str() {
                "Group was created" => return Some("group_created_message"),
                "Channel was created" => return Some("channel_created_message"),
                _ => {}
            }
        }
        if trimmed.contains("Group was created") || trimmed.starts_with("Group '") {
            return Some("group_created_message");
        }
        if trimmed.contains("Channel was created") || trimmed.starts_with("Channel '") {
            return Some("channel_created_message");
        }
        None
    }
}

/// a person named in a system pill (X joined the chat, X added Y)
/// clicking the name opens that profile
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageSystemPersonLink {
    pub name: String,
    pub user_id: String,
}

const EMPTY_USER_ID: &str = "00000000-0000-0000-0000-000000000000";
const ERASED_ACCOUNT_NAME: &str = "Deleted Account";

pub enum MessageSystemFormatter {}

impl MessageSystemFormatter {
    /// people the rendered label names by name
    /// the signed in person, a deleted account and a name not in the label are left out
    pub fn person_links(
        message: &GetMessageMessage,
        label: &str,
        current_user_ids: &HashSet<String>,
    ) -> Vec<MessageSystemPersonLink> {
        let Some(system) = message.system.as_ref() else {
            return Vec::new();
        };
        if label.is_empty() {
            return Vec::new();
        }
        let deleted = tr("chat_deleted_account");
        let target_name = match system.target_name.as_deref() {
            Some(name) if !name.is_empty() => Self::localized_person(name),
            _ => MessageSystemContentParser::moderation_target(&message.text).unwrap_or_default(),
        };
        let candidates = [
            (Self::display_actor(message), system.actor_id.as_deref()),
            (target_name, system.target_id.as_deref()),
        ];
        let mut links: Vec<MessageSystemPersonLink> = Vec::new();
        for (raw_name, raw_id) in &candidates {
            let name = trimming_whitespaces_and_newlines(raw_name);
            let user_id = raw_id.map(|id| trimming_whitespaces_and_newlines(id)).unwrap_or_default();
            if name.is_empty()
                || name == deleted
                || user_id.is_empty()
                || user_id == EMPTY_USER_ID
                || current_user_ids.contains(user_id)
                || !label.contains(name)
                || links.iter().any(|link| link.user_id == user_id)
            {
                continue;
            }
            links.push(MessageSystemPersonLink { name: name.to_owned(), user_id: user_id.to_owned() });
        }
        links
    }

    /// person name as the server sent it, with an erased account literal in the reader language
    fn localized_person(name: &str) -> String {
        if trimming_whitespaces_and_newlines(name) == ERASED_ACCOUNT_NAME { tr("chat_deleted_account") } else { name.to_owned() }
    }

    fn display_actor(message: &GetMessageMessage) -> String {
        if let Some(actor) = message.system.as_ref().and_then(|system| system.actor_name.as_ref())
            && !actor.is_empty()
        {
            return Self::localized_person(actor);
        }
        let sender = trimming_whitespaces_and_newlines(&message.sender_name);
        let lowered = sender.to_lowercase();
        if !sender.is_empty() && lowered != "unknown user" && lowered != "system" {
            return Self::localized_person(sender);
        }
        if let Some(actor_id) = message.system.as_ref().and_then(|system| system.actor_id.as_ref())
            && !actor_id.is_empty()
            && actor_id != EMPTY_USER_ID
        {
            return tr("chat_deleted_account");
        }
        trimming_whitespaces_and_newlines(&message.text)
            .split(' ')
            .find(|word| !word.is_empty())
            .map(str::to_owned)
            .unwrap_or_default()
    }

    fn renamed_title(text: &str) -> Option<String> {
        let title = trimming_whitespaces_and_newlines(text);
        if title.is_empty() || title == "The group was renamed" {
            return None;
        }
        if title.starts_with('@') && title.ends_with(" renamed the group") {
            return None;
        }
        Some(title.to_owned())
    }

    /// renders with the context of the moment, for a caller on the main thread
    pub fn render_current(message: &GetMessageMessage, cx: &App) -> String {
        Self::render(message, &MessageSystemFormatContext::current(cx))
    }

    pub fn render(message: &GetMessageMessage, context: &MessageSystemFormatContext) -> String {
        if let Some(custom) = message.system.as_ref().and_then(|system| system.text.as_ref())
            && !custom.is_empty()
        {
            return custom.clone();
        }

        if message.content_type == MessageContentType::System {
            let plain_text = trimming_whitespaces_and_newlines(&message.text);
            if let Some(localized) = context.localized_creation_content(plain_text) {
                return localized;
            }
        }

        let Some(system) = message.system.as_ref() else {
            return message.text.clone();
        };
        let channel_actor = context.channel_title.as_ref().filter(|_| {
            system.actor_id.as_deref().is_some_and(|id| !id.is_empty())
                && !matches!(
                    system.kind,
                    MessageSystemKind::UserJoined
                        | MessageSystemKind::MemberJoined
                        | MessageSystemKind::UserLeft
                        | MessageSystemKind::MemberLeft
                        | MessageSystemKind::ContactJoined
                        | MessageSystemKind::PremiumGifted
                        | MessageSystemKind::GiftSent
                )
        });
        let actor = channel_actor.cloned().unwrap_or_else(|| Self::display_actor(message));
        let target = system.target_name.as_deref().map(Self::localized_person).unwrap_or_default();
        let actor_is_me = channel_actor.is_none() && context.is_current_user(system.actor_id.as_deref());
        let recipient_is_me = system
            .gift_recipient_id
            .as_ref()
            .is_some_and(|recipient| context.current_user_ids.contains(recipient));
        let by_actor = |key: &str| {
            if actor_is_me {
                context.string(&format!("{key}_you"))
            } else {
                format_text(&context.string(key), &actor)
            }
        };
        let by_subject = |key: &str| {
            if target.is_empty() {
                return by_actor(key);
            }
            if context.is_current_user(system.target_id.as_deref()) {
                context.string(&format!("{key}_you"))
            } else {
                format_text(&context.string(key), &target)
            }
        };
        match system.kind {
            MessageSystemKind::UserJoined | MessageSystemKind::MemberJoined => by_subject("msg_sys_user_joined"),
            MessageSystemKind::UserLeft | MessageSystemKind::MemberLeft => by_subject("msg_sys_user_left"),
            MessageSystemKind::UserAdded => {
                if target.is_empty() {
                    return by_actor("msg_sys_user_added_generic");
                }
                if actor_is_me {
                    format_text(&context.string("msg_sys_user_added_target_you"), &target)
                } else {
                    format_texts(&context.string("msg_sys_user_added_target"), &actor, &target)
                }
            }
            MessageSystemKind::UserRemoved | MessageSystemKind::MemberKicked | MessageSystemKind::MemberBanned => {
                let prefix = if system.kind == MessageSystemKind::MemberBanned {
                    "msg_sys_user_banned"
                } else {
                    "msg_sys_user_removed"
                };
                let removed = if target.is_empty() {
                    MessageSystemContentParser::moderation_target(&message.text).unwrap_or_default()
                } else {
                    target.clone()
                };
                if removed.is_empty() {
                    return by_actor(&format!("{prefix}_generic"));
                }
                if actor_is_me {
                    format_text(&context.string(&format!("{prefix}_target_you")), &removed)
                } else {
                    format_texts(&context.string(&format!("{prefix}_target")), &actor, &removed)
                }
            }
            MessageSystemKind::ChatCreated => context.string("group_created_message"),
            MessageSystemKind::ChatRenamed => {
                let Some(title) = Self::renamed_title(&message.text) else {
                    return by_actor("msg_sys_chat_renamed");
                };
                if actor_is_me {
                    format_text(&context.string("msg_sys_chat_renamed_to_you"), &title)
                } else {
                    format_texts(&context.string("msg_sys_chat_renamed_to"), &actor, &title)
                }
            }
            MessageSystemKind::ChatPhotoChanged => by_actor("msg_sys_chat_photo_changed"),
            MessageSystemKind::ChatPhotoRemoved => by_actor("msg_sys_chat_photo_removed"),
            MessageSystemKind::ChatWallpaperChanged => by_actor("msg_sys_chat_wallpaper_changed"),
            MessageSystemKind::ChatWallpaperRemoved => by_actor("msg_sys_chat_wallpaper_removed"),
            MessageSystemKind::MessagePinned => by_actor("msg_sys_message_pinned"),
            MessageSystemKind::MessageUnpinned => by_actor("msg_sys_message_unpinned"),
            MessageSystemKind::VoiceCallStarted => context.string("msg_sys_voice_call_started"),
            MessageSystemKind::VoiceCallEnded => context.string("msg_sys_voice_call_ended"),
            MessageSystemKind::VideoCallStarted => context.string("msg_sys_video_call_started"),
            MessageSystemKind::VideoCallEnded => context.string("msg_sys_video_call_ended"),
            MessageSystemKind::SecretChatStarted => context.string("msg_sys_secret_chat_started"),
            MessageSystemKind::SecretChatTtlChanged => {
                let base = context.string("msg_sys_secret_chat_ttl_changed");
                let key = match trimming_whitespaces_and_newlines(&message.text).parse::<i64>() {
                    Ok(0) => "chat_autodelete_off",
                    Ok(86_400) => "chat_autodelete_1d",
                    Ok(604_800) => "chat_autodelete_1w",
                    Ok(2_592_000) => "chat_autodelete_1m",
                    _ => return base,
                };
                format!("{base}: {}", context.string(key))
            }
            MessageSystemKind::ChannelCreated => context.string("channel_created_message"),
            MessageSystemKind::TopicCreated => {
                let name = trimming_whitespaces_and_newlines(&message.text);
                context.string("chat_topic_created_system").replace("{topic}", name)
            }
            MessageSystemKind::ContactJoined => format_text(
                &context.string("contact_joined_system_message"),
                if target.is_empty() { &actor } else { &target },
            ),
            MessageSystemKind::MigratedFromGroup => context.string("msg_sys_migrated_from_group"),
            MessageSystemKind::ScoreReached => context.string("msg_sys_score_reached"),
            MessageSystemKind::PremiumGifted => {
                let Some(months) = system.gift_months else {
                    return format_text(&context.string("msg_sys_premium_gifted_short"), &actor);
                };
                let period = context.gift_months(months);
                if actor_is_me {
                    return format_text(&context.string("msg_sys_premium_gifted_by_you"), &period);
                }
                if recipient_is_me {
                    return format_texts(&context.string("msg_sys_premium_gifted_to_you"), &actor, &period);
                }
                format_texts(&context.string("msg_sys_premium_gifted"), &actor, &period)
            }
            MessageSystemKind::GiftSent => {
                if actor_is_me {
                    return context.string("msg_sys_gift_sent_by_you");
                }
                if recipient_is_me {
                    return format_text(&context.string("msg_sys_gift_sent_to_you"), &actor);
                }
                format_text(&context.string("msg_sys_gift_sent_short"), &actor)
            }
            MessageSystemKind::Generic => {
                if message.content_type == MessageContentType::System
                    && let Some(localized) = context.localized_creation_content(&message.text)
                {
                    return localized;
                }
                message.text.clone()
            }
        }
    }
}
