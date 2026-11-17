use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, ElementId, Entity, IntoElement, RenderOnce, SharedString, Subscription, Window};
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::{tr, tr_plural};
use rise_ui::swiftui::lifecycle::on_change;
use rise_ui::swiftui::open_url::open_url;

use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseColdOpenTrace;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::push_notifications::universal_link_parser::UniversalLinkParser;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::ui::link_preview_card_ui::{LinkPreviewCardPresentation, LinkPreviewCardUi, LinkPreviewCardVariant};
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_invites_actions::ChatInviteActionResult;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatJoinPreview, ChatVoiceJoinPreview};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{GetChatPreviewGatewayDTO, GetChatPreviewGatewayDTOExt};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageLinkPreview, MessageLinkPreviewSize,
};

#[derive(IntoElement)]
pub struct MessageLinkPreviewView {
    id: ElementId,
    pub preview: MessageLinkPreview,
    pub is_own: bool,
}

struct MessageLinkPreviewGlimpseWatch {
    _glimpse: Subscription,
}

impl MessageLinkPreviewView {
    pub fn new(id: impl Into<ElementId>, preview: MessageLinkPreview) -> Self {
        Self {
            id: id.into(),
            preview,
            is_own: false,
        }
    }

    pub fn is_own(mut self, is_own: bool) -> Self {
        self.is_own = is_own;
        self
    }

    fn child_id(&self, suffix: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(suffix))
    }

    /// invite link of this app, the card may be a voice chat link whose live state is per viewer
    fn join_token(&self) -> Option<String> {
        MessageVoiceLinkGlimpse::join_token(&self.preview.url)
    }
}

impl RenderOnce for MessageLinkPreviewView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let join_token = self.join_token();
        let glimpse = if self.preview.feed_owns_voice_card {
            None
        } else {
            join_token.as_ref().and_then(|token| {
                let shared = MessageVoiceLinkGlimpse::shared(cx);
                window.use_keyed_state(self.child_id("glimpse-watch"), cx, |_, cx| MessageLinkPreviewGlimpseWatch {
                    _glimpse: cx.observe(&shared, |_, _, cx| cx.notify()),
                });
                shared.read(cx).entry(token).cloned()
            })
        };
        let voice = glimpse.as_ref().and_then(|glimpse| glimpse.voice.clone());

        on_change(
            self.child_id("glimpse-task"),
            join_token.clone(),
            true,
            window,
            cx,
            |_, token: &Option<String>, _, cx| {
                let Some(token) = token else {
                    return;
                };
                MessageVoiceLinkGlimpse::refresh(token, cx);
            },
        );

        let presentation = LinkPreviewCardPresentation {
            url: self.preview.url.clone(),
            title: match voice.as_ref() {
                Some(voice) => MessageVoiceLinkGlimpse::title(
                    voice,
                    glimpse.as_ref().and_then(|glimpse| glimpse.chat_title.as_deref()),
                    self.preview.title.as_deref(),
                ),
                None => self.preview.title.clone(),
            },
            summary: match voice.as_ref() {
                Some(voice) => Some(MessageVoiceLinkGlimpse::summary(voice)),
                None => self.preview.description.clone(),
            },
            site_name: self.preview.site_name.clone(),
            image_url: self.preview.image_url.clone(),
            image_aspect: self.preview.image_aspect,
            summary_is_skeleton: self.preview.voice_card_pending,
        };
        let variant = if self.preview.size == MessageLinkPreviewSize::Small {
            LinkPreviewCardVariant::Compact
        } else {
            LinkPreviewCardVariant::Expanded
        };
        let link = self.preview.url.clone();
        LinkPreviewCardUi::new(self.child_id("card"), presentation)
            .variant(variant)
            .is_on_accent_fill(self.is_own)
            .shows_card_chrome(false)
            .opens_url_on_tap(false)
            .compact_media_size(54.0)
            .reserves_compact_media_space(false)
            .expanded_media_height(180.0)
            .bubble_tap_target_priority(self.child_id("tap"), move |window: &mut Window, cx: &mut App| {
                let Some(url) = Url::from_string(&link) else {
                    return;
                };
                let is_web = url
                    .scheme()
                    .map(str::to_lowercase)
                    .is_some_and(|scheme| scheme == "https" || scheme == "http");
                if !is_web {
                    return;
                }
                open_url(cx).call_as_function(&url, window, cx);
            })
    }
}

#[derive(Clone, Debug)]
pub struct MessageVoiceLinkGlimpseEntry {
    pub voice: Option<ChatVoiceJoinPreview>,
    pub chat_title: Option<String>,
    pub stamp: Date,
}

/// live state of voice chat links in messages
/// the preview is shared so call status is asked per viewer with the invite preview
/// request and cached briefly
#[derive(Default)]
pub struct MessageVoiceLinkGlimpse {
    entries: HashMap<String, MessageVoiceLinkGlimpseEntry>,
    pending: HashSet<String>,
    revision: u64,
}

impl MessageVoiceLinkGlimpse {
    const FRESH_FOR: TimeInterval = 30.0;
    const RETRY_AFTER_FAILURE: TimeInterval = 60.0;
    const CARD_LINE_PLACEHOLDER: &'static str = "\u{00A0}\n\u{00A0}";

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self::default())
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn join_token(url: &str) -> Option<String> {
        let url = Url::from_string(url)?;
        match UniversalLinkParser::parse(&url, AppEnvironment::current()) {
            Some(PushDeepLink::JoinChat { token }) => Some(token),
            _ => None,
        }
    }

    pub fn baking(messages: Vec<Arc<GetMessageMessage>>, cx: &App) -> Vec<Arc<GetMessageMessage>> {
        match rise_ui::shared::try_read::<Self>(cx) {
            Some(shared) => shared.baking_into(messages),
            None => Self::default().baking_into(messages),
        }
    }

    fn baking_into(&self, messages: Vec<Arc<GetMessageMessage>>) -> Vec<Arc<GetMessageMessage>> {
        if !messages.iter().any(|message| message.link_preview.is_some()) {
            return messages;
        }
        messages
            .into_iter()
            .map(|message| {
                let baked = message.link_preview.as_ref().and_then(|preview| self.baked_preview(preview));
                match baked {
                    Some(baked) => {
                        let mut message = Arc::unwrap_or_clone(message);
                        message.link_preview = Some(baked);
                        Arc::new(message)
                    }
                    None => message,
                }
            })
            .collect()
    }

    fn baked_preview(&self, preview: &MessageLinkPreview) -> Option<MessageLinkPreview> {
        let token = Self::join_token(&preview.url)?;
        let mut baked = preview.clone();
        baked.feed_owns_voice_card = true;
        let entry = self.entries.get(&token);
        if let Some(entry) = entry
            && let Some(voice) = entry.voice.as_ref()
        {
            baked.title = Self::title(voice, entry.chat_title.as_deref(), preview.title.as_deref());
            baked.description = Some(Self::holding_card_lines(&Self::summary(voice)));
        } else if entry.is_none()
            && trimming_whitespaces_and_newlines(preview.description.as_deref().unwrap_or("")).is_empty()
        {
            baked.description = Some(Self::CARD_LINE_PLACEHOLDER.to_owned());
            baked.voice_card_pending = true;
        }
        RiseColdOpenTrace::mark(|| {
            format!(
                "b84c bake tok={} entry={} voice={} pending={} rev={}",
                token.chars().take(4).collect::<String>(),
                entry.is_some(),
                entry.is_some_and(|entry| entry.voice.is_some()),
                baked.voice_card_pending,
                self.revision
            )
        });
        (baked != *preview).then_some(baked)
    }

    fn holding_card_lines(summary: &str) -> String {
        if summary.contains('\n') {
            summary.to_owned()
        } else {
            format!("{summary}\n\u{00A0}")
        }
    }

    /// shared preview only has the group name, a hidden call keeps the neutral title
    /// a viewer who may see the group gets its name from his own preview
    pub fn title(voice: &ChatVoiceJoinPreview, chat_title: Option<&str>, server_title: Option<&str>) -> Option<String> {
        if voice.details_hidden {
            return Some(tr(if voice.is_text { "topic_invite_preview_title" } else { "voice_invite_preview_title" }));
        }
        let own = trimming_whitespaces_and_newlines(chat_title.unwrap_or(""));
        if !own.is_empty() {
            return Some(own.to_owned());
        }
        let server = trimming_whitespaces_and_newlines(server_title.unwrap_or(""));
        if !server.is_empty() {
            return Some(server.to_owned());
        }
        Some(
            voice
                .topic_title
                .clone()
                .filter(|topic| !topic.is_empty())
                .unwrap_or_else(|| tr(if voice.is_text { "chat_topics_topic" } else { "rtc_group_voice" })),
        )
    }

    pub fn summary(voice: &ChatVoiceJoinPreview) -> String {
        if voice.details_hidden {
            return tr(if voice.is_text { "topic_invite_preview_private" } else { "voice_invite_preview_private" });
        }
        if voice.is_text {
            let topic = trimming_whitespaces_and_newlines(voice.topic_title.as_deref().unwrap_or(""));
            return if topic.is_empty() { tr("chat_topics_topic") } else { topic.to_owned() };
        }
        let state = if voice.live {
            tr_plural("voice_invite_live", voice.participant_count)
        } else {
            tr("voice_invite_not_live")
        };
        let topic = trimming_whitespaces_and_newlines(voice.topic_title.as_deref().unwrap_or(""));
        if topic.is_empty() {
            return state;
        }
        format!("{topic}\n{state}")
    }

    pub fn voice(&self, token: &str) -> Option<&ChatVoiceJoinPreview> {
        self.entries.get(token).and_then(|entry| entry.voice.as_ref())
    }

    pub fn entry(&self, token: &str) -> Option<&MessageVoiceLinkGlimpseEntry> {
        self.entries.get(token)
    }

    pub fn refresh(token: &str, cx: &mut App) {
        let glimpse = Self::shared(cx);
        let starts = glimpse.update(cx, |glimpse, _| {
            if let Some(entry) = glimpse.entries.get(token)
                && Date::now().time_interval_since(&entry.stamp) < Self::FRESH_FOR
            {
                return false;
            }
            glimpse.pending.insert(token.to_owned())
        });
        if !starts {
            return;
        }
        let weak = glimpse.downgrade();
        let answered = token.to_owned();
        let on_result: ChatInviteActionResult<GetChatPreviewGatewayDTO> = Rc::new(
            move |result: Result<GetChatPreviewGatewayDTO, &anyhow::Error>, cx: &mut App| {
                let Some(glimpse) = weak.upgrade() else {
                    return;
                };
                RiseColdOpenTrace::mark(|| {
                    format!(
                        "b84c glimpse tok={} ok={}",
                        answered.chars().take(4).collect::<String>(),
                        matches!(&result, Ok(dto) if dto.success)
                    )
                });
                let outcome = match result {
                    Ok(dto) if dto.success => Some(dto.to_chat_join_preview()),
                    _ => None,
                };
                glimpse.update(cx, |glimpse, cx| glimpse.finish(&answered, outcome, cx));
            },
        );
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.get_chat_preview_by_link_action(token, on_result, cx);
        });
    }

    /// outcome is none when the request failed, inner none is a preview the server sent empty
    fn finish(&mut self, token: &str, outcome: Option<Option<ChatJoinPreview>>, cx: &mut Context<Self>) {
        self.pending.remove(token);
        let previous = self.entries.get(token).cloned();
        match outcome {
            Some(preview) => {
                let voice = preview.as_ref().and_then(|preview| preview.voice.clone());
                let chat_title = if voice.as_ref().is_some_and(|voice| !voice.details_hidden) {
                    preview.as_ref().map(|preview| preview.title.clone())
                } else {
                    None
                };
                let changed = previous
                    .as_ref()
                    .is_none_or(|previous| previous.voice != voice || previous.chat_title != chat_title);
                self.entries.insert(
                    token.to_owned(),
                    MessageVoiceLinkGlimpseEntry {
                        voice,
                        chat_title,
                        stamp: Date::now(),
                    },
                );
                if changed {
                    self.revision = self.revision.wrapping_add(1);
                }
            }
            None => {
                let first_failure = previous.is_none();
                let (voice, chat_title) = previous
                    .map(|previous| (previous.voice, previous.chat_title))
                    .unwrap_or_default();
                self.entries.insert(
                    token.to_owned(),
                    MessageVoiceLinkGlimpseEntry {
                        voice,
                        chat_title,
                        stamp: Date::now().adding_time_interval(Self::RETRY_AFTER_FAILURE - Self::FRESH_FOR),
                    },
                );
                if first_failure {
                    self.revision = self.revision.wrapping_add(1);
                }
            }
        }
        cx.notify();
    }
}
