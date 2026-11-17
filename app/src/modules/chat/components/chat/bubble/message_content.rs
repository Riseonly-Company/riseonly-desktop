use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AvailableSpace, BoxShadow, Bounds, Element, ElementId, Entity, Font as GpuiFont, FontFeatures, FontStyle,
    FontWeight, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, ParentElement, Pixels, Point, RenderOnce, SharedString,
    Size, Style, Styled, Subscription, TextRun, TextSystem, Window, WindowTextSystem, black, div, point, px, rgb, size, white,
};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::foundation::ns_range::NSRange;
use rise_i18n::tr;
use rise_theme::FONT_FAMILY;
use rise_theme::theme::app_theme::AppTheme;
use rise_theme::theme::theme_environment::theme_store;
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::message_media_tail_presentation::MessageMediaTailGeometry;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::rich_text::document::rise_document::RiseDocument;
use rise_ui::rich_text::document::rise_document_codec::RiseDocumentCodec;
use rise_ui::rich_text::rich_text_markup::{RichBlock, RichTextMarkup, RiseIndexedRichBlock};
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::button_style::ButtonAction;
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::environment_values::{ColorScheme, ColorSchemeKey};
use rise_ui::swiftui::material::Material;
use rise_ui::swiftui::motion::{MotionSpec, ease_out, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Presence, Transition, TransitionModifiers, UnitPoint};
use rise_ui::window_screen_frame_reader::MediaCellScreenFrameRegistry;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{MainTab, RootRoute};
use crate::core::lib::canon_format::RiseFormat;
use crate::core::media_viewer::media_viewer_open::MediaViewerOpen;
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerGalleryScope, MediaViewerPresentationHandlers, chat_media_presentation,
};
use crate::core::media_viewer::media_viewer_types::{
    ChatMediaHeroSkin, MediaViewerChrome, MediaViewerDeleteMode, MediaViewerFrameKeys, MediaViewerGalleryItem,
};
use crate::core::storage::rise_chat_media_ledger::RiseChatMediaLedger;
use crate::core::ui::custom_emoji_glyph::CustomEmojiGlyphMetrics;
use crate::core::ui::custom_emoji_resource_warmup::CustomEmojiResourceWarmup;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::modules::chat::components::chat::async_render::rise_native_text_message_cell::OWN_SELECTION_OPACITY;
use crate::core::ui::formatted_text_ui::{
    FormattedText, FormattedTextInlinePreparation, FormattedTextOptions, FormattedTextPreparedGeometry, FormattedTextRenderSeed,
    FormattedTextSelection, FormattedTextStyle, FormattedTextUi, RoleMentionSpan,
};
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::reaction_emoji_glyph::ReactionEmojiPlaceholder;
use crate::core::ui::rich_text::code_block_view::CodeBlockView;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::async_render::rise_message_layout_measurement::is_message_layout_measurement;
use crate::modules::chat::components::chat::bubble::bubble_corners::{BubbleCorners, TelegramReactionPalette};
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::components::chat::bubble::chat_mention_tap_layer::{ChatMentionTap, ChatSpoilerFormattedText};
use crate::modules::chat::components::chat::bubble::chat_prepared_text_layout::{
    ChatPreparedTextLayoutCache, ChatPreparedTextLayoutMeasurement, ChatTextHitLayoutSeed,
};
use crate::modules::chat::components::chat::bubble::inline_custom_emoji_run::InlineCustomEmojiRun;
use crate::modules::chat::components::chat::bubble::inline_custom_emoji_text::InlineCustomEmojiText;
use crate::modules::chat::components::chat::bubble::media_letterbox_photo::MessageMediaSideFill;
use crate::modules::chat::components::chat::bubble::message_audio_view::AudioMessagePlaybackRow;
use crate::modules::chat::components::chat::bubble::message_call_view::{MessageCallPresentationFactory, MessageCallView};
use crate::modules::chat::components::chat::bubble::message_contact_view::MessageContactView;
use crate::modules::chat::components::chat::bubble::message_content_hooks::message_content_hooks;
use crate::modules::chat::components::chat::bubble::message_content_member_tag_hooks::message_content_member_tag_hooks;
use crate::modules::chat::components::chat::bubble::message_document_view::MessageDocumentView;
use crate::modules::chat::components::chat::bubble::message_forward_header::MessageForwardHeader;
use crate::modules::chat::components::chat::bubble::message_image_grid_layout::MessageGridCell;
use crate::modules::chat::components::chat::bubble::message_image_grid_view::MessageImageGridView;
use crate::modules::chat::components::chat::bubble::message_link_preview_view::MessageLinkPreviewView;
use crate::modules::chat::components::chat::bubble::message_location_view::MessageLocationView;
use crate::modules::chat::components::chat::bubble::message_meta::{MessageMeta, MessageMetaStyle};
use crate::modules::chat::components::chat::bubble::message_poll_view::MessagePollView;
use crate::modules::chat::components::chat::bubble::message_reply_header::MessageReplyHeader;
use crate::modules::chat::components::chat::bubble::message_shared_post_card::MessageSharedPostCard;
use crate::modules::chat::components::chat::bubble::message_sticker_view::{MessageStickerLayout, MessageStickerView};
use crate::modules::chat::components::chat::bubble::message_story_reply_header::MessageStoryReplyHeader;
use crate::modules::chat::components::chat::bubble::message_voice_view::VoiceMessagePlaybackRow;
use crate::modules::chat::components::chat::bubble::rise_document_bubble_view::{RiseDocumentBubbleView, RiseDocumentMediaContent};
use crate::modules::chat::components::chat::media_viewer::chat_media_gallery_builder::{
    ChatMediaGalleryBuilder, ChatViewLimitedOpenings, ChatViewLimitedViewerGuard,
};
use crate::modules::chat::components::chat::media_viewer::chat_media_presentation::{ChatGalleryMessagesKey, ChatGalleryPaginatesKey};
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::components::chat::video_note::video_note_message_view::VideoNoteMessageView;
use crate::modules::chat::components::chat::video_note::video_note_player_store::VideoNotePlayerStore;
use crate::modules::chat::pages::chats::chat::chat_roles::role_mention_composer::RoleMentionComposer;
use crate::modules::chat::pages::chats::chat::chat_roles::topic_mention_composer::TopicMentionComposer;
use crate::modules::chat::shared::chat_saved_message_presentation::{ChatSavedMessageAuthor, ChatSavedMessagePresentation};
use crate::modules::chat::shared::chat_search_highlight::chat_search_highlight_words;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::shared::message_permissions::{ChatMessageAction, MessagePermissions};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatMember, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageCustomEmojiEntity, MessageForwardInfo, MessageMediaItem, MessageMediaKind,
    MessageMediaViewPhase, MessageReaction, MessageReplyTo, MessageSharedPost, MessageStoryReply, ReactionReactor, RemoteMediaURL,
};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_poll_voters_services::PollVotersStore;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::sparks::components::sparks_icon_view::SparksBurstIcon;
use crate::modules::sparks::shared::sparks_message_policy::SparksMessagePolicy;
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_types::SparksTarget;
use crate::modules::sparks::stores::sparks::sparks_interactions::sparks_interactions::SparksInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::user::shared::mention_resolver::{MentionTapDestination, MentionTapResolver};

pub type MessageContentAction = Rc<dyn Fn(&mut Window, &mut App)>;
/// message action and reaction callbacks, the media viewer calls them too with no window in hand
pub type MessageContentStringAction = Rc<dyn Fn(&str, &mut App)>;

const DEFAULT_MESSAGE_VIEWPORT_WIDTH: f32 = 393.0;
const COUNTER_ESTIMATED_GLYPH_WIDTH: f32 = 6.5;
const DETACHED_PLATE_INSET_X: f32 = 8.0;

pub struct ChatMessageViewportWidthKey;

impl EnvKey for ChatMessageViewportWidthKey {
    type Value = f32;

    fn default_value() -> Self::Value {
        DEFAULT_MESSAGE_VIEWPORT_WIDTH
    }
}

/// off by default so other hosts of a bubble like pinned or search
/// draw the reply quote as decoration
pub struct ChatAllowsReplyJumpKey;

impl EnvKey for ChatAllowsReplyJumpKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        false
    }
}

/// viewport width the message bubbles lay out against
pub fn chat_message_viewport_width(cx: &App) -> f32 {
    env::<ChatMessageViewportWidthKey>(cx)
}

pub fn chat_allows_reply_jump(cx: &App) -> bool {
    env::<ChatAllowsReplyJumpKey>(cx)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageForwardSourceTarget {
    ChatProfile { chat_id: String },
    User { user_id: Option<String>, tag: Option<String> },
}

impl MessageForwardSourceTarget {
    pub fn resolve(info: &MessageForwardInfo) -> Option<Self> {
        let chat_id = Self::normalized(info.from_chat_id.as_deref());
        let sender_id = Self::normalized(info.sender_id.as_deref());
        let sender_tag = Self::normalized(info.sender_tag.as_deref()).map(|tag| tag.trim_matches('@').to_owned());

        if let Some(sender_id) = sender_id {
            if let Some(chat_id) = chat_id.filter(|chat_id| *chat_id == sender_id) {
                return Some(Self::ChatProfile { chat_id });
            }
            return Some(Self::User { user_id: Some(sender_id), tag: sender_tag.filter(|tag| !tag.is_empty()) });
        }

        if let Some(sender_tag) = sender_tag.filter(|tag| !tag.is_empty()) {
            return Some(Self::User { user_id: None, tag: Some(sender_tag) });
        }

        if let Some(chat_id) = chat_id
            && (Self::normalized(info.from_chat_title.as_deref()).is_some()
                || Self::normalized(info.from_chat_logo.as_deref()).is_some())
        {
            return Some(Self::ChatProfile { chat_id });
        }
        None
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(value?);
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MessageContentKind {
    Text,
    MediaGrid,
    Voice,
    Audio,
    Document,
    Contact,
    Location,
    Poll,
    Call,
    Sticker,
    VideoNote,
    PostShort,
}

impl MessageContentKind {
    pub const ALL_CASES: [Self; 12] = [
        Self::Text,
        Self::MediaGrid,
        Self::Voice,
        Self::Audio,
        Self::Document,
        Self::Contact,
        Self::Location,
        Self::Poll,
        Self::Call,
        Self::Sticker,
        Self::VideoNote,
        Self::PostShort,
    ];

    pub fn needs_bubble_padding(self) -> bool {
        !matches!(self, Self::Sticker | Self::MediaGrid | Self::VideoNote | Self::Call)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MessageContentTerminalSection {
    LinkPreview,
    Meta,
    Reactions,
}

/// what the bubble reads from environment and stores, once per render
struct MessageContentEnvironment {
    theme: AppTheme,
    nav_store: Option<Entity<NavigationStore>>,
    search_highlight_words: Vec<String>,
    message_viewport_width: f32,
    allows_reply_jump: bool,
    is_layout_measurement: bool,
    saved_author: Option<ChatSavedMessageAuthor>,
    sender_member: Option<ChatMember>,
    sender_premium_color: Option<Hsla>,
    chat_gallery_messages: Option<Arc<Vec<Arc<GetMessageMessage>>>>,
    chat_gallery_paginates: bool,
    animates_entrance: bool,
}

#[derive(Default)]
struct MessageContentMountState {
    did_mount: bool,
}

struct VideoNoteDisplayState {
    size: Animated<f32>,
    _player_observer: Subscription,
}

#[derive(IntoElement)]
pub struct MessageContent {
    id: ElementId,
    message: Rc<GetMessageMessage>,
    chat: Rc<ChatInfo>,
    is_own: bool,
    is_first_in_group: bool,
    bubble_corners: BubbleCorners,
    hide_name: bool,
    on_reaction: Option<MessageContentStringAction>,
    on_message_action: Option<MessageContentStringAction>,
    on_open_sender_profile: Option<MessageContentAction>,
    prepared_text: Option<FormattedTextInlinePreparation>,
    can_double_tap: Option<Rc<dyn Fn() -> bool>>,
}

impl MessageContent {
    pub const DOCUMENT_ROW_GAP: f32 = 4.0;

    pub fn new(message: GetMessageMessage, chat: ChatInfo) -> Self {
        Self {
            id: ElementId::Name(SharedString::from(format!("message-content-{}", message.id))),
            message: Rc::new(message),
            chat: Rc::new(chat),
            is_own: false,
            is_first_in_group: true,
            bubble_corners: BubbleCorners::uniform(px(MessageSizes::bubble_corner_radius())),
            hide_name: false,
            on_reaction: None,
            on_message_action: None,
            on_open_sender_profile: None,
            prepared_text: None,
            can_double_tap: None,
        }
    }

    pub fn is_own(mut self, is_own: bool) -> Self {
        self.is_own = is_own;
        self
    }

    pub fn is_first_in_group(mut self, is_first_in_group: bool) -> Self {
        self.is_first_in_group = is_first_in_group;
        self
    }

    pub fn bubble_corners(mut self, bubble_corners: BubbleCorners) -> Self {
        self.bubble_corners = bubble_corners;
        self
    }

    pub fn hide_name(mut self, hide_name: bool) -> Self {
        self.hide_name = hide_name;
        self
    }

    pub fn on_reaction(mut self, on_reaction: impl Into<Option<MessageContentStringAction>>) -> Self {
        self.on_reaction = on_reaction.into();
        self
    }

    pub fn on_message_action(mut self, on_message_action: impl Into<Option<MessageContentStringAction>>) -> Self {
        self.on_message_action = on_message_action.into();
        self
    }

    pub fn on_open_sender_profile(mut self, on_open_sender_profile: impl Into<Option<MessageContentAction>>) -> Self {
        self.on_open_sender_profile = on_open_sender_profile.into();
        self
    }

    pub fn prepared_text(mut self, prepared_text: impl Into<Option<FormattedTextInlinePreparation>>) -> Self {
        self.prepared_text = prepared_text.into();
        self
    }

    pub fn media_max_width() -> f32 {
        MessageSizes::media_max_width()
    }

    pub fn media_inset() -> f32 {
        MessageSizes::media_inset()
    }

    pub fn bubble_corner_radius() -> f32 {
        MessageSizes::bubble_corner_radius()
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn indexed_id(&self, name: &str, index: impl std::fmt::Display) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::from(format!("{name}-{index}")))
    }

    fn is_channel(&self) -> bool {
        self.chat.r#type == ChatType::Channel
    }

    /// ties this bubble media and chat title to the chat for the per chat storage list
    fn note_chat_media(message: &GetMessageMessage, chat: &ChatInfo, cx: &mut App) {
        let chat_id = if message.chat_id.is_empty() { chat.id.as_str() } else { message.chat_id.as_str() };
        let chat_name = if chat_id == chat.id {
            message_content_hooks().map_or_else(|| chat.title.clone(), |hooks| (hooks.chat_title)(chat, cx))
        } else {
            String::new()
        };
        RiseChatMediaLedger::note(chat_id, &chat_name, &message.media_items);
    }

    fn content_insets(kind: MessageContentKind, viewport_width: f32) -> (f32, f32, f32, f32) {
        if !kind.needs_bubble_padding() {
            return (0.0, 0.0, 0.0, 0.0);
        }
        let horizontal_padding = MessageSizes::horizontal_bubble_padding(viewport_width);
        match kind {
            MessageContentKind::Voice | MessageContentKind::Audio | MessageContentKind::Document => (15.0, 9.0, 15.0, 12.0),
            _ => (
                MessageSizes::bubble_inner_top_padding(),
                horizontal_padding,
                MessageSizes::bubble_inner_bottom_padding(),
                horizontal_padding,
            ),
        }
    }

    fn inline_meta(&self) -> MessageMeta {
        MessageMeta::new((*self.message).clone()).is_own(self.is_own).is_channel(self.is_channel()).style(MessageMetaStyle::Inline)
    }

    fn overlay_meta(&self) -> MessageMeta {
        MessageMeta::new((*self.message).clone()).is_own(self.is_own).is_channel(self.is_channel()).style(MessageMetaStyle::Overlay)
    }

    /// double tap the gesture plate answers
    /// while its on offer the second click is the plates, not a word selection
    pub fn can_double_tap(mut self, can_double_tap: Option<Rc<dyn Fn() -> bool>>) -> Self {
        self.can_double_tap = can_double_tap;
        self
    }

    /// mouse text selection of the bubble, same guard as copy in the hold menu
    /// a message you cant copy you cant select either
    fn text_selection(&self, env: &MessageContentEnvironment) -> Option<FormattedTextSelection> {
        if !MessagePermissions::new(&self.message, &self.chat, self.is_own).can_copy() {
            return None;
        }
        let single_click_when = self.can_double_tap.clone().map(|can_double_tap| -> Rc<dyn Fn(&mut App) -> bool> {
            Rc::new(move |_: &mut App| can_double_tap())
        });
        Some(FormattedTextSelection {
            color: if self.is_own { env.theme.content_on(env.theme.primary._100).opacity(OWN_SELECTION_OPACITY) } else { env.theme.input.selection },
            press_gate: Rc::new(|cx: &mut App| HoldContextMenuPresenter::has_active_request(cx)),
            single_click_when,
        })
    }

    fn highlight_options(env: &MessageContentEnvironment) -> FormattedTextOptions {
        FormattedTextOptions { highlight_words: env.search_highlight_words.clone(), ..FormattedTextOptions::default() }
    }

    fn text_with_meta(&self, env: &MessageContentEnvironment, cx: &mut App) -> AnyElement {
        let rich_style = Self::formatted_text_style(&env.theme, self.is_own);
        let normalized_entities = self.message.resolved_entities().clone();
        if let Some(document) = RiseDocumentCodec::decode(self.message.document_raw.as_deref()) {
            return self.document_with_meta(document, rich_style, env);
        }
        if !normalized_entities.custom_emojis.is_empty() {
            return self.custom_emoji_text_with_meta(rich_style, normalized_entities.custom_emojis, env);
        }
        let mut role_spans = RoleMentionComposer::render_spans(
            &normalized_entities.role_mentions,
            &self.message.chat_id,
            self.is_own,
            cx,
        );
        role_spans.extend(TopicMentionComposer::render_spans(&normalized_entities.topic_mentions, &self.message.chat_id));
        if RichTextMarkup::has_code_block(&self.message.text) {
            return self.rich_blocks_with_meta(rich_style, &role_spans, env);
        }
        MessageFormattedTextWithMeta {
            id: self.child_id("formatted-text"),
            message: self.message.clone(),
            is_own: self.is_own,
            is_channel: self.is_channel(),
            style: rich_style,
            role_spans,
            options: Self::highlight_options(env),
            prepared: self.prepared_text.clone(),
            text_selection: self.text_selection(env),
            on_mention_tap: Self::handle_mention_tap(self.message.chat_id.clone(), env.nav_store.clone()),
        }
        .into_any_element()
    }

    fn document_with_meta(&self, document: RiseDocument, style: FormattedTextStyle, env: &MessageContentEnvironment) -> AnyElement {
        let media_content: RiseDocumentMediaContent = {
            let message = self.message.clone();
            let chat = self.chat.clone();
            let context = self.viewer_context(env);
            let id = self.id.clone();
            let viewport_width = env.message_viewport_width;
            let is_own = self.is_own;
            Rc::new(move |index: usize, _window: &mut Window, _cx: &mut App| {
                let item = message.media_items.get(index)?.clone();
                Some(Self::document_media(&id, item, index, viewport_width, is_own, &chat, &context))
            })
        };
        let mut column = BubbleColumnLayout::new(6.0).expands_to_proposal(true).child(
            RiseDocumentBubbleView::new(self.child_id("document"), document, style)
                .media_content(Some(media_content))
                .text_selection(self.text_selection(env)),
        );
        if self.message.link_preview.is_none() && !self.message.has_reaction_row() {
            column = column.child(hstack(0.0).child(spacer_min_w(0.0)).child(self.inline_meta()));
        }
        column.into_any_element()
    }

    fn document_media(
        id: &ElementId,
        item: MessageMediaItem,
        index: usize,
        viewport_width: f32,
        is_own: bool,
        chat: &ChatInfo,
        context: &MessageContentViewerContext,
    ) -> AnyElement {
        let maximum = MessageSizes::media_maximum_size(viewport_width);
        let container_width = maximum
            .width
            .min(
                MessageSizes::available_bubble_width(viewport_width, chat, is_own)
                    - MessageSizes::bubble_tail_inset()
                    - MessageSizes::horizontal_bubble_padding(viewport_width) * 2.0,
            )
            .max(1.0);
        let tap_context = context.clone();
        let cancel_context = context.clone();
        MessageImageGridView::new(
            ElementId::NamedChild(Arc::new(id.clone()), SharedString::from(format!("document-media-{index}"))),
            context.message.id.clone(),
            vec![item],
            container_width,
            maximum.height,
        )
        .corners(BubbleCorners::uniform(px(MessageSizes::bubble_corner_radius())))
        .on_tap(Rc::new(move |_: usize, frame: Bounds<Pixels>, cell: &MessageGridCell, layout_is_single: bool, _window: &mut Window, cx: &mut App| {
            tap_context.open_media_viewer(index, frame, cell, layout_is_single, cx);
        }))
        .on_cancel_upload(Rc::new(move |media_item: &MessageMediaItem, _window: &mut Window, cx: &mut App| {
            cancel_context.cancel_outgoing_media(media_item, cx);
        }))
        .into_any_element()
    }

    fn moderation_plate(&self, theme: &AppTheme) -> gpui::Div {
        let tint = if self.is_own { theme.content_on(theme.primary._100).opacity(0.9) } else { theme.semantic.error_100 };
        let plate = if self.is_own { theme.content_on(theme.primary._100).opacity(0.18) } else { theme.semantic.error_100.opacity(0.1) };
        hstack(6.0)
            .children(IconUi::sized("eye.slash", px(12.0), tint, false))
            .child(
                MainText::new(tr("message_disabled_by_moderation"))
                    .px(12.0)
                    .font_weight(FontWeight::MEDIUM)
                    .color(tint)
                    .number_of_lines(1),
            )
            .child(spacer_min_w(0.0))
            .px(px(10.0))
            .py(px(6.0))
            .bg(plate)
            .continuous_corners(8.0)
    }

    fn custom_emoji_text_with_meta(
        &self,
        style: FormattedTextStyle,
        entities: Vec<MessageCustomEmojiEntity>,
        env: &MessageContentEnvironment,
    ) -> AnyElement {
        let trailing_meta = (self.message.link_preview.is_none() && !self.message.has_reaction_row())
            .then(|| self.inline_meta().into_any_element());
        InlineCustomEmojiText::new(self.child_id("custom-emoji-text"), self.message.text.clone(), style, entities)
            .trailing_meta(trailing_meta)
            .text_selection(self.text_selection(env))
            .into_any_element()
    }

    fn rich_blocks_with_meta(
        &self,
        style: FormattedTextStyle,
        all_role_spans: &[RoleMentionSpan],
        env: &MessageContentEnvironment,
    ) -> AnyElement {
        let viewport_width = env.message_viewport_width;
        let blocks = RichTextMarkup::blocks(&self.message.text);
        let content_width = (MessageSizes::available_bubble_width(viewport_width, &self.chat, self.is_own)
            - MessageSizes::bubble_tail_inset()
            - MessageSizes::horizontal_bubble_padding(viewport_width) * 2.0)
            .max(1.0);
        let mut stack = div().flex().flex_col().items_start().gap(px(6.0)).w(px(content_width));
        for entry in RiseIndexedRichBlock::list(&blocks) {
            let entry_id = self.indexed_id("rich-block", &entry.id);
            stack = match entry.block {
                RichBlock::Paragraph(paragraph) => {
                    let paragraph_roles = self.role_spans(&paragraph, all_role_spans);
                    stack.child(
                        ChatSpoilerFormattedText::new(
                            entry_id,
                            paragraph,
                            style.clone(),
                            Self::handle_mention_tap(self.message.chat_id.clone(), env.nav_store.clone()),
                        )
                        .options(Self::highlight_options(env))
                        .role_mentions(paragraph_roles)
                        .width_hint(Some(content_width))
                        .text_selection(self.text_selection(env)),
                    )
                }
                RichBlock::Code { language, code } => {
                    stack.child(
                        CodeBlockView::new(entry_id, language, code)
                            .font_size(13.5)
                            .in_bubble(true)
                            .text_selection(self.text_selection(env)),
                    )
                }
            };
        }
        if self.message.link_preview.is_none() && !self.message.has_reaction_row() {
            stack = stack.child(hstack(0.0).w_full().child(spacer_min_w(0.0)).child(self.inline_meta()));
        }
        stack.into_any_element()
    }

    fn role_spans(&self, paragraph: &str, all: &[RoleMentionSpan]) -> Vec<RoleMentionSpan> {
        if all.is_empty() {
            return Vec::new();
        }
        let Some(start) = self.message.text.find(paragraph) else {
            return Vec::new();
        };
        let location = self.message.text[..start].encode_utf16().count();
        let length = paragraph.encode_utf16().count();
        all.iter()
            .filter(|span| span.range.location >= location && span.range.location + span.range.length <= location + length)
            .map(|span| RoleMentionSpan {
                range: NSRange { location: span.range.location - location, length: span.range.length },
                color: span.color,
                link: span.link.clone(),
            })
            .collect()
    }

    fn handle_mention_tap(chat_id: String, nav_store: Option<Entity<NavigationStore>>) -> ChatMentionTap {
        Rc::new(move |tag: &str, _window: &mut Window, cx: &mut App| {
            let needle = tag.to_lowercase();
            let is_role = ChatServicesStore::shared(cx)
                .update(cx, |store, cx| store.roles(&chat_id, cx))
                .iter()
                .any(|role| role.name.to_lowercase() == needle);
            if is_role {
                return;
            }
            let nav_store = nav_store.clone();
            MentionTapResolver::shared(cx).update(cx, |resolver, cx| {
                resolver.resolve(
                    tag,
                    move |destination, cx| {
                        let Some(nav_store) = nav_store.as_ref() else {
                            return;
                        };
                        nav_store.update(cx, |nav_store, cx| match &destination {
                            MentionTapDestination::User { tag } => nav_store.push(
                                RootRoute::UserPage { user_id: None, tag: Some(tag.clone()) },
                                NavigationStoreSettlingPush::Drop,
                                cx,
                            ),
                            MentionTapDestination::Chat { id } => {
                                nav_store.switch_tab(MainTab::Chats, cx);
                                nav_store.push(
                                    RootRoute::Chat { chat_id: Some(id.clone()) },
                                    NavigationStoreSettlingPush::Drop,
                                    cx,
                                );
                            }
                        });
                    },
                    cx,
                );
            });
        })
    }

    pub fn formatted_text_style(theme: &AppTheme, is_own: bool) -> FormattedTextStyle {
        let mut style = FormattedTextStyle::new(
            if is_own { theme.content_on(theme.primary._100).opacity(0.98) } else { theme.text.primary },
            if is_own { FormattedTextStyle::link_color_on_dark_fill() } else { FormattedTextStyle::standard_link_color() },
            if is_own { theme.content_on(theme.primary._100).opacity(0.72) } else { theme.text.secondary },
        );
        style.font_size = MessageSizes::text_font_size();
        style.subtitle_font_size = MessageSizes::text_subtitle_font_size();
        style.line_spacing = MessageSizes::text_line_spacing();
        style.code_foreground = Some(if is_own { theme.content_on(theme.primary._100) } else { theme.text.primary });
        style.code_background = Some(if is_own { theme.content_on(theme.primary._100).opacity(0.26) } else { theme.text.primary.opacity(0.18) });
        style.link_underline = is_own;
        style
    }

    fn view_phased_grid_items(&self, cx: &mut App) -> Vec<MessageMediaItem> {
        let items: Vec<MessageMediaItem> = self.message.grid_media_items().into_iter().cloned().collect();
        if !items.iter().any(|item| item.view_limit.is_some()) {
            return items;
        }
        let my_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default();
        let viewed_by_peer = self.message.played_by.iter().any(|id| !id.is_empty() && *id != my_id);
        let opening = {
            let openings = ChatViewLimitedOpenings::shared(cx);
            let openings = openings.read(cx);
            openings.contains(&self.message.id) && !openings.closed().contains(&self.message.id)
        };
        let now = Date::now();
        let mut phased = Vec::with_capacity(items.len());
        for mut item in items {
            let Some(limit) = item.view_limit.clone() else {
                phased.push(item);
                continue;
            };
            let opened = !self.is_own
                && !opening
                && ChatViewLimitedViewerGuard::is_opened_by_viewer(&self.message, &limit, &my_id, cx);
            if self.is_own
                && let (Some(server_ms), Some(seconds)) = (limit.sender_countdown_deadline_ms(), limit.seconds)
            {
                let message_id = self.message.id.clone();
                let deadline_ms = ChatViewLimitedOpenings::shared(cx).update(cx, |openings, cx| {
                    let deadline_ms = openings.sender_deadline(&message_id, server_ms, seconds);
                    openings.expire_sent(&message_id, deadline_ms, cx);
                    deadline_ms
                });
                item.view_phase = Some(MessageMediaViewPhase::Countdown { deadline_ms, seconds });
                phased.push(item);
                continue;
            }
            item.view_phase = Some(if self.is_own {
                MessageMediaViewPhase::Sent { viewed: viewed_by_peer }
            } else if opened || (!opening && limit.is_consumed(now)) {
                MessageMediaViewPhase::Consumed
            } else {
                MessageMediaViewPhase::Sealed
            });
            phased.push(item);
        }
        phased
    }

    fn media_with_caption_and_meta(&self, show_sender: bool, env: &MessageContentEnvironment, cx: &mut App) -> AnyElement {
        let items = self.view_phased_grid_items(cx);
        let has_context_header = self.has_message_context_header(env);
        let width_source_index = Some(if has_context_header { 1 } else { 0 });
        let horizontal_padding = MessageSizes::horizontal_bubble_padding(env.message_viewport_width);
        let media_inset = Self::media_inset();
        if items.is_empty() {
            return div().into_any_element();
        }
        if self.message.has_text() {
            let style = Self::formatted_text_style(&env.theme, self.is_own);
            let on_mention_tap = Self::handle_mention_tap(self.message.chat_id.clone(), env.nav_store.clone());
            let mut column = BubbleColumnLayout::new(6.0).width_source_index(width_source_index);
            let media_corners = if has_context_header {
                self.bubble_corners.media_middle()
            } else {
                self.bubble_corners.media_top_only(px(0.0))
            };
            let (grid, grid_width) = self.media_grid(items, media_corners, show_sender, false, env);
            if has_context_header {
                let header_width = grid_width + media_inset * 2.0 - horizontal_padding * 2.0;
                column = column.child(
                    self.context_header_content(false, true, Some(header_width), env)
                        .px(px(horizontal_padding))
                        .pt(px(MessageSizes::bubble_inner_top_padding())),
                );
            }
            column = column.fixed_width(Some(grid_width + media_inset * 2.0)).child(
                div()
                    .flex()
                    .flex_col()
                    .px(px(media_inset))
                    .when(!has_context_header, |this| this.pt(px(media_inset)))
                    .child(grid),
            );
            let caption_custom_emojis = self.message.resolved_entities().custom_emojis.clone();
            let caption = if caption_custom_emojis.is_empty() {
                ChatSpoilerFormattedText::new(self.child_id("caption"), self.message.text.clone(), style, on_mention_tap)
                    .options(Self::highlight_options(env))
                    .text_selection(self.text_selection(env))
                    .into_any_element()
            } else {
                InlineCustomEmojiText::new(self.child_id("caption"), self.message.text.clone(), style, caption_custom_emojis)
                    .options(Self::highlight_options(env))
                    .on_mention_tap(Some(on_mention_tap))
                    .text_selection(self.text_selection(env))
                    .into_any_element()
            };
            column = column.child(div().px(px(horizontal_padding)).child(caption)).child(
                self.trailing_meta(false)
                    .px(px(horizontal_padding))
                    .pb(px(MessageSizes::bubble_inner_bottom_padding())),
            );
            return column.into_any_element();
        }
        let mut column = BubbleColumnLayout::new(0.0).width_source_index(width_source_index);
        let media_corners = if has_context_header {
            self.bubble_corners.media_bottom_only(px(0.0))
        } else {
            self.bubble_corners.inset_all(px(0.0))
        };
        let (grid, grid_width) = self.media_grid(items, media_corners, show_sender, true, env);
        if has_context_header {
            let header_width = grid_width + media_inset * 2.0 - horizontal_padding * 2.0;
            column = column.child(
                self.context_header_content(false, true, Some(header_width), env)
                    .px(px(horizontal_padding))
                    .pt(px(MessageSizes::bubble_inner_top_padding()))
                    .pb(px(6.0)),
            );
        }
        column
            .fixed_width(Some(grid_width + media_inset * 2.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .px(px(media_inset))
                    .pb(px(media_inset))
                    .when(!has_context_header, |this| this.pt(px(media_inset)))
                    .child(grid),
            )
            .into_any_element()
    }

    fn hides_forward_header(env: &MessageContentEnvironment) -> bool {
        env.saved_author.is_some()
    }

    fn has_message_context_header(&self, env: &MessageContentEnvironment) -> bool {
        (self.message.forward_info.is_some() && !Self::hides_forward_header(env))
            || self.message.reply_to.is_some()
            || self.message.story_reply.is_some()
    }

    fn video_note_context_header(&self, display_size: f32, env: &MessageContentEnvironment) -> gpui::Div {
        let max_width = self.video_note_expanded_size(env).min(display_size).max(1.0);
        let plate = div().min_w_0().max_w(px(max_width)).child(self.detached_context_plate(false, true, Some(max_width), env));
        hstack(0.0)
            .w(px(display_size))
            .when(self.is_own, |this| this.child(spacer_min_w(0.0)))
            .child(plate)
            .when(!self.is_own, |this| this.child(spacer_min_w(0.0)))
    }

    /// plate beside a sticker hugs its text up to the room the sticker leaves
    /// never overlaps the sticker so the quote is a button like on a text bubble
    fn sticker_context_header(&self, sticker: &MessageMediaItem, env: &MessageContentEnvironment) -> gpui::Div {
        let max_width = MessageStickerLayout::context_header_max_width(
            f32::from(MessageStickerLayout::display_size(sticker).width),
            MessageSizes::available_bubble_width(env.message_viewport_width, &self.chat, self.is_own)
                - MessageSizes::bubble_tail_inset()
                - MessageStickerLayout::META_OVERHANG,
        );
        div().flex_none().max_w(px(max_width)).child(self.detached_context_plate(false, true, Some(max_width), env))
    }

    /// max_width is the outer cap of the plate
    /// a one line quote answers its whole unwrapped line so content gets the cap minus inset
    /// it ends in an ellipsis
    fn detached_context_plate(
        &self,
        fills_width: bool,
        allows_taps: bool,
        max_width: Option<f32>,
        env: &MessageContentEnvironment,
    ) -> gpui::Div {
        let content_max_width = max_width.map(|width| (width - DETACHED_PLATE_INSET_X * 2.0).max(1.0));
        let is_dark = env.theme.bg._100 == black();
        div()
            .relative()
            .when(fills_width, |this| this.w_full())
            .continuous_corners(8.0)
            .overflow_hidden()
            .shadow(vec![BoxShadow {
                color: black().opacity(0.14),
                offset: point(px(0.0), px(1.0)),
                blur_radius: px(2.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .child(div().absolute().inset_0().continuous_corners(8.0).bg(ultra_thin_material_fill(is_dark)))
            .child(div().absolute().inset_0().continuous_corners(8.0).bg(black().opacity(0.38)))
            .child(
                self.context_header_content(true, allows_taps, content_max_width, env)
                    .relative()
                    .px(px(DETACHED_PLATE_INSET_X))
                    .py(px(6.0)),
            )
    }

    fn context_header_content(
        &self,
        force_light_foreground: bool,
        allows_taps: bool,
        max_width: Option<f32>,
        env: &MessageContentEnvironment,
    ) -> gpui::Div {
        let is_own = force_light_foreground || self.is_own;
        let mut stack = div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(4.0))
            .w_full()
            .min_w_0()
            .when_some(max_width, |this, width| this.max_w(px(width)));
        if let Some(forward_info) = self.message.forward_info.clone().filter(|_| !Self::hides_forward_header(env)) {
            let action = allows_taps.then(|| Self::forward_source_action(&forward_info, env)).flatten();
            stack = stack.child(
                MessageForwardHeader::new(self.child_id("context-forward"), forward_info)
                    .is_own(is_own)
                    .on_open_source(action)
                    .max_width(max_width),
            );
        }
        if let Some(reply) = self.message.reply_to.clone() {
            let action = allows_taps.then(|| self.reply_source_action(&reply, env)).flatten();
            stack = stack.child(
                MessageReplyHeader::new(self.child_id("context-reply"), reply)
                    .is_own(is_own)
                    .on_open_source(action)
                    .max_width(max_width),
            );
        }
        if let Some(story) = self.message.story_reply.clone() {
            let action = allows_taps.then(|| Self::story_source_action(&story)).flatten();
            stack = stack.child(
                MessageStoryReplyHeader::new(self.child_id("context-story"), story)
                    .is_own(is_own)
                    .on_open_story(action)
                    .max_width(max_width),
            );
        }
        stack
    }

    /// only the chat itself can jump to a message so the quote is inert on other hosts
    /// the pinned page above all. a parent still sending has a temp id, quote stays a button
    fn reply_source_action(&self, reply: &MessageReplyTo, env: &MessageContentEnvironment) -> Option<MessageContentAction> {
        if !env.allows_reply_jump {
            return None;
        }
        let target_id = trimming_whitespaces_and_newlines(&reply.message_id).to_owned();
        if target_id.is_empty() {
            return None;
        }
        let on_message_action = self.on_message_action.clone();
        Some(Rc::new(move |_window: &mut Window, cx: &mut App| {
            if let Some(on_message_action) = on_message_action.as_ref() {
                on_message_action(&ChatMessageAction::jump_to_message_action(&target_id), cx);
            }
        }))
    }

    /// story player lives at the app root so this quote opens the story from any host
    /// unlike a reply jump
    fn story_source_action(story: &MessageStoryReply) -> Option<MessageContentAction> {
        if !story.is_openable() {
            return None;
        }
        let story_id = story.story_id;
        Some(Rc::new(move |_window: &mut Window, cx: &mut App| {
            StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.open_story(story_id, cx));
        }))
    }

    fn forward_source_action(info: &MessageForwardInfo, env: &MessageContentEnvironment) -> Option<MessageContentAction> {
        let target = MessageForwardSourceTarget::resolve(info)?;
        let nav_store = env.nav_store.clone();
        Some(Rc::new(move |_window: &mut Window, cx: &mut App| {
            let Some(nav_store) = nav_store.as_ref() else {
                return;
            };
            let route = match &target {
                MessageForwardSourceTarget::ChatProfile { chat_id } => RootRoute::ChatProfile { chat_id: Some(chat_id.clone()) },
                MessageForwardSourceTarget::User { user_id, tag } => {
                    RootRoute::UserPage { user_id: user_id.clone(), tag: tag.clone() }
                }
            };
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }))
    }

    fn open_shared_post_comments(post: &MessageSharedPost, cx: &mut App) {
        let feed_post = post.as_feed_post();
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.open_comments(&feed_post, false, cx));
    }

    fn media_draws_tail(corners: &BubbleCorners) -> bool {
        MessageMediaTailGeometry::eligible_side(corners.draws_tail, corners.tail_side, corners.bottom_leading, corners.bottom_trailing)
            != rise_ui::message_bubble_geometry::MessageBubbleTailSide::None
    }

    /// the grid and the width it lays itself out at
    fn media_grid(
        &self,
        items: Vec<MessageMediaItem>,
        media_corners: BubbleCorners,
        show_sender: bool,
        show_overlay_meta: bool,
        env: &MessageContentEnvironment,
    ) -> (AnyElement, f32) {
        let viewport_width = env.message_viewport_width;
        let bubble_width = MessageSizes::available_bubble_width(viewport_width, &self.chat, self.is_own);
        let maximum_media_size = MessageSizes::media_maximum_size(viewport_width);
        let container_width = maximum_media_size
            .width
            .min(bubble_width - MessageSizes::bubble_tail_inset() - Self::media_inset() * 2.0)
            .max(1.0);
        let meta_overlay = show_overlay_meta.then(|| self.overlay_meta().into_any_element());
        let sender_overlay = show_sender.then(|| {
            let on_open_sender_profile = self.on_open_sender_profile.clone();
            MediaSenderOverlay::new(self.sender_heading(env), self.sender_heading_color(env))
                .bubble_tap_target_priority(self.child_id("media-sender"), move |window, cx| {
                    if let Some(on_open_sender_profile) = on_open_sender_profile.as_ref() {
                        on_open_sender_profile(window, cx);
                    }
                })
                .into_any_element()
        });
        let draws_tail = Self::media_draws_tail(&media_corners);
        let side_fill = (!draws_tail && items.len() == 1).then(|| {
            let item = &items[0];
            let video_source = if item.kind == MessageMediaKind::Video { trimming_whitespaces_and_newlines(&item.url).to_owned() } else { String::new() };
            MessageMediaSideFill::new(self.child_id("media-side-fill"), trimming_whitespaces_and_newlines(&item.display_source()).to_owned(), media_corners.clone())
                .video_source(video_source)
        });
        let tap_context = self.viewer_context(env);
        let cancel_context = tap_context.clone();
        let grid = MessageImageGridView::new(
            self.child_id("media-grid"),
            self.message.id.clone(),
            items,
            container_width,
            maximum_media_size.height,
        )
        .corners(media_corners)
        .meta(meta_overlay)
        .sender_header(sender_overlay)
        .on_tap(Rc::new(move |index: usize, frame: Bounds<Pixels>, cell: &MessageGridCell, layout_is_single: bool, _window: &mut Window, cx: &mut App| {
            tap_context.open_media_viewer(index, frame, cell, layout_is_single, cx);
        }))
        .on_cancel_upload(Rc::new(move |media_item: &MessageMediaItem, _window: &mut Window, cx: &mut App| {
            cancel_context.cancel_outgoing_media(media_item, cx);
        }));
        let grid_width = grid.layout_width();
        let element = div()
            .relative()
            .flex()
            .flex_col()
            .when(!draws_tail, |this| this.w_full().items_center())
            .when_some(side_fill, |this, side_fill| this.child(div().absolute().inset_0().child(side_fill)))
            .child(grid)
            .into_any_element();
        (element, grid_width)
    }

    fn cancel_action(&self, item: MessageMediaItem) -> MessageContentAction {
        let message = self.message.clone();
        let chat = self.chat.clone();
        Rc::new(move |_window: &mut Window, cx: &mut App| {
            MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.cancel_outgoing_media(&message, &chat, &item, cx));
        })
    }

    fn voice_with_meta(&self, env: &MessageContentEnvironment, cx: &mut App) -> AnyElement {
        let Some(voice) = self.message.first_voice_item().cloned() else {
            return div().into_any_element();
        };
        let unread = self.media_unplayed(cx);
        BubbleColumnLayout::new(6.0)
            .expands_to_proposal(true)
            .child(
                VoiceMessagePlaybackRow::new(
                    self.child_id("voice"),
                    self.message.id.clone(),
                    self.chat.id.clone(),
                    self.chat.title.clone(),
                    self.message.sender_name.clone(),
                    voice.clone(),
                )
                .is_own(self.is_own)
                .unread(unread)
                .constrained_content_width(Some(self.interactive_file_content_width(env)))
                .on_cancel_upload(self.cancel_action(voice)),
            )
            .children(self.inline_terminal_meta())
            .into_any_element()
    }

    fn interactive_file_content_width(&self, env: &MessageContentEnvironment) -> f32 {
        (MessageSizes::available_bubble_width(env.message_viewport_width, &self.chat, self.is_own)
            - MessageSizes::bubble_tail_inset()
            - 9.0
            - 12.0)
            .max(1.0)
    }

    fn audio_with_meta(&self, env: &MessageContentEnvironment) -> AnyElement {
        let Some(audio) = self.message.first_audio_item().cloned() else {
            return div().into_any_element();
        };
        let chat_messages = env.chat_gallery_messages.clone().unwrap_or_else(|| Arc::new(vec![Arc::new((*self.message).clone())]));
        BubbleColumnLayout::new(6.0)
            .expands_to_proposal(true)
            .child(
                AudioMessagePlaybackRow::new(
                    self.child_id("audio"),
                    self.message.id.clone(),
                    self.chat.id.clone(),
                    audio.clone(),
                    chat_messages,
                )
                .cache_message_id(Some(self.message.sai_server_id()))
                .caption(self.message.text.clone())
                .is_own(self.is_own)
                .on_cancel_upload(self.cancel_action(audio)),
            )
            .children(self.inline_terminal_meta())
            .into_any_element()
    }

    fn documents_with_meta(&self) -> AnyElement {
        let documents = self.message.document_items();
        if documents.is_empty() {
            return div().into_any_element();
        }
        let last = documents.len() - 1;
        let mut column = BubbleColumnLayout::new(6.0).expands_to_proposal(true);
        for (index, document) in documents.into_iter().enumerate() {
            let caption = if index == last { self.message.text.clone() } else { String::new() };
            column = column.child(
                div().when(index != 0, |this| this.pt(px(Self::DOCUMENT_ROW_GAP))).child(
                    MessageDocumentView::new(
                        self.indexed_id("document-row", index),
                        document.clone(),
                        self.message.id.clone(),
                        self.chat.id.clone(),
                    )
                    .caption(caption)
                    .is_own(self.is_own)
                    .on_cancel_upload(Some(self.cancel_action(document.clone()))),
                ),
            );
        }
        column.children(self.inline_terminal_meta()).into_any_element()
    }

    fn video_note_item(&self) -> Option<&MessageMediaItem> {
        self.message.media_items.iter().find(|item| item.kind == MessageMediaKind::VideoNote)
    }

    fn video_note_expanded_size(&self, env: &MessageContentEnvironment) -> f32 {
        VideoNoteMessageView::expanded_size(
            env.message_viewport_width,
            MessageSizes::reserves_avatar_space(&self.chat, self.is_own),
        )
    }

    fn video_note_playing_size(&self, env: &MessageContentEnvironment) -> f32 {
        VideoNoteMessageView::playing_size(env.message_viewport_width, &self.chat, self.is_own)
    }

    fn is_video_note_playing(&self, cx: &mut App) -> bool {
        let store = VideoNotePlayerStore::shared(cx);
        let store = store.read(cx);
        store.is_active() && store.message_id() == self.message.id
    }

    fn video_note_with_meta(&self, env: &MessageContentEnvironment, window: &mut Window, cx: &mut App) -> AnyElement {
        let is_playing = self.is_video_note_playing(cx);
        let target_size = if is_playing && !env.is_layout_measurement {
            self.video_note_playing_size(env)
        } else {
            self.video_note_expanded_size(env)
        };
        let now = RiseClock::monotonic();
        let state = window.use_keyed_state(self.child_id("video-note-display"), cx, |_, cx| {
            let player = VideoNotePlayerStore::shared(cx);
            VideoNoteDisplayState {
                size: Animated::new(target_size),
                _player_observer: cx.observe(&player, |_, _, cx| cx.notify()),
            }
        });
        state.update(cx, |state, cx| {
            if *state.size.target() != target_size {
                state.size.set_in(target_size, VideoNoteMessageView::resize_animation(), now, cx);
            }
        });
        let display_size = state.read(cx).size.frame_value(now, window);

        let is_own = self.is_own;
        let mut stack = div().relative().w(px(display_size)).h(px(display_size));
        if is_playing {
            let playing_size = self.video_note_playing_size(env);
            let viewport_width = env.message_viewport_width;
            stack = stack.child(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, (), window, _| {
                        crate::modules::chat::components::chat::video_note::video_note_message_view::x767_mark(
                            &format!("content stack playing={playing_size} viewport_width={viewport_width}"),
                            bounds,
                            window,
                        );
                    },
                )
                .absolute()
                .inset_0(),
            );
        }
        if let Some(item) = self.video_note_item().cloned() {
            let video_note = VideoNoteMessageView::new(
                self.child_id("video-note"),
                item.clone(),
                (*self.message).clone(),
                self.chat.id.clone(),
                self.message.sender_name.clone(),
                self.video_note_expanded_size(env),
            )
            .playing_size(Some(self.video_note_playing_size(env)))
            .on_cancel_upload(self.cancel_action(item));
            stack = stack.child(
                div().absolute().bottom_0().map(|this| if is_own { this.right_0() } else { this.left_0() }).child(video_note),
            );
        }
        let duration = self.video_note_item().and_then(|item| item.duration).unwrap_or(0.0);
        let tail_inset = px(MessageSizes::bubble_tail_inset());
        div()
            .when(!is_own, |this| this.pl(tail_inset))
            .when(is_own, |this| this.pr(tail_inset))
            .child(
                BubbleColumnLayout::new(6.0)
                    .when(self.has_message_context_header(env), |this| {
                        this.child(self.video_note_context_header(display_size, env))
                    })
                    .child(stack)
                    .child(self.video_note_meta_row(duration, display_size, env, cx)),
            )
            .into_any_element()
    }

    fn media_unplayed(&self, cx: &mut App) -> bool {
        let viewer_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        MessageList::awaits_peer_playback(&self.message, &self.chat, self.is_own, &viewer_id)
    }

    fn inline_terminal_meta(&self) -> Option<gpui::Div> {
        self.message.link_preview.is_none().then(|| self.trailing_meta(false))
    }

    fn trailing_meta(&self, show_unplayed_dot: bool) -> gpui::Div {
        hstack(0.0).child(spacer_min_w(0.0)).child(self.inline_meta().show_unplayed_dot(show_unplayed_dot))
    }

    fn video_note_meta_row(&self, duration: f64, display_size: f32, env: &MessageContentEnvironment, cx: &mut App) -> gpui::Div {
        let unplayed = self.media_unplayed(cx);
        hstack(0.0)
            .w(px(display_size))
            .child(
                hstack(MessageSizes::meta_content_spacing())
                    .child(
                        MainText::new(RiseFormat::duration_time_interval(duration))
                            .px(MessageSizes::meta_font_size())
                            .color(white())
                            .monospaced_digits(true),
                    )
                    .when(unplayed, |this| this.child(div().circle().bg(white()).frame_size(6.0, 6.0)))
                    .px(px(MessageSizes::meta_overlay_horizontal_padding()))
                    .py(px(MessageSizes::meta_overlay_vertical_padding()))
                    .capsule()
                    .bg(MessageMeta::wallpaper_plate_fill(TelegramReactionPalette::is_dark_background(env.theme.bg._100))),
            )
            .child(spacer_min_w(0.0))
            .child(self.overlay_meta().on_wallpaper(true))
    }

    fn sticker_content(&self, env: &MessageContentEnvironment) -> AnyElement {
        let Some(sticker) = self.message.first_sticker_item().cloned() else {
            return div().into_any_element();
        };
        let is_own = self.is_own;
        let overhang = MessageStickerLayout::META_OVERHANG;
        let has_context_header = self.has_message_context_header(env);
        let sticker_view = div().relative().flex_none().child(MessageStickerView::new(self.child_id("sticker"), sticker.clone())).child(
            div()
                .absolute()
                .right(px(-overhang))
                .bottom(px(-overhang))
                .child(self.overlay_meta().on_wallpaper(true)),
        );
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(MessageStickerLayout::CONTEXT_HEADER_SPACING))
            .when(is_own && has_context_header, |this| this.child(self.sticker_context_header(&sticker, env)))
            .child(sticker_view)
            .when(!is_own && has_context_header, |this| this.child(self.sticker_context_header(&sticker, env)))
            .pb(px(overhang))
            .when(!is_own, |this| this.pl(px(MessageSizes::bubble_tail_inset())))
            .when(is_own, |this| this.pr(px(MessageSizes::bubble_tail_inset() + overhang)))
            .into_any_element()
    }

    fn terminal_reactions(&self, kind: MessageContentKind, env: &MessageContentEnvironment) -> impl IntoElement {
        let needs_padding = !kind.needs_bubble_padding();
        div().when(needs_padding, |this| this.px(px(10.0)).py(px(6.0))).child(
            ReactionsAndMetaRow::new(self.child_id("reactions"), self.message.reactions.clone(), self.is_own)
                .on_reaction(self.on_reaction.clone())
                .meta((kind == MessageContentKind::Text).then(|| self.inline_meta().into_any_element()))
                .sparks(self.sparks_chip())
                .hides_reactors(self.chat.r#type == ChatType::Channel)
                .animates_entrance(env.animates_entrance),
        )
    }

    fn sparks_chip(&self) -> Option<ReactionsSparksChip> {
        if self.message.sparks_total <= 0 {
            return None;
        }
        let server_id = self.message.sai_server_id();
        Some(ReactionsSparksChip {
            total: self.message.sparks_total,
            mine: self.message.my_sparks,
            target: SparksTarget::Message {
                chat_id: self.chat.id.clone(),
                message_id: if server_id.is_empty() { self.message.id.clone() } else { server_id },
            },
            can_send: SparksMessagePolicy::can_send(&self.message, &self.chat, self.is_own),
        })
    }

    fn should_show_sender_name(&self, env: &MessageContentEnvironment) -> bool {
        match self.chat.r#type {
            ChatType::Direct | ChatType::Channel => false,
            ChatType::Group => true,
            ChatType::Favourites => env.saved_author.is_some(),
        }
    }

    fn sender_heading(&self, env: &MessageContentEnvironment) -> String {
        if self.is_channel() {
            return self.chat.title.clone();
        }
        if let Some(saved_author) = env.saved_author.as_ref() {
            return saved_author.name.clone();
        }
        self.message.sender_name.clone()
    }

    fn sender_heading_color(&self, env: &MessageContentEnvironment) -> Hsla {
        if let Some(saved_author) = env.saved_author.as_ref() {
            return ChatColorPalette::color(&saved_author.id, false, &env.theme);
        }
        if let Some(role_color) = env.sender_member.as_ref().and_then(ChatMember::name_color) {
            return env.theme.legible(role_color);
        }
        if let Some(premium) = env.sender_premium_color {
            return premium;
        }
        ChatColorPalette::color(&self.message.sender_id, self.is_own, &env.theme)
    }

    fn viewer_context(&self, env: &MessageContentEnvironment) -> MessageContentViewerContext {
        MessageContentViewerContext {
            message: self.message.clone(),
            chat: self.chat.clone(),
            is_own: self.is_own,
            on_message_action: self.on_message_action.clone(),
            chat_gallery_messages: env.chat_gallery_messages.clone(),
            chat_gallery_paginates: env.chat_gallery_paginates,
        }
    }

    pub fn terminal_sections(
        kind: MessageContentKind,
        has_link_preview: bool,
        has_reactions: bool,
        link_preview_above_text: bool,
    ) -> Vec<MessageContentTerminalSection> {
        let mut sections = Vec::new();
        if has_link_preview && kind.needs_bubble_padding() {
            if !link_preview_above_text {
                sections.push(MessageContentTerminalSection::LinkPreview);
            }
            if !(has_reactions && kind == MessageContentKind::Text) {
                sections.push(MessageContentTerminalSection::Meta);
            }
        }
        if has_reactions && kind != MessageContentKind::Call {
            sections.push(MessageContentTerminalSection::Reactions);
        }
        sections
    }

    pub fn bubble_kind(m: &GetMessageMessage, chat: Option<&ChatInfo>) -> MessageContentKind {
        if m.document_raw.is_some() {
            return MessageContentKind::Text;
        }
        if m.content_type == MessageContentType::Call
            && m.call_info.is_some()
            && chat.is_none_or(|chat| chat.r#type == ChatType::Direct)
        {
            return MessageContentKind::Call;
        }
        if m.shared_post.is_some() {
            return MessageContentKind::PostShort;
        }
        if m.first_sticker_item().is_some() {
            return MessageContentKind::Sticker;
        }
        if m.content_type == MessageContentType::Voice || m.first_voice_item().is_some() {
            return MessageContentKind::Voice;
        }
        if m.first_audio_item().is_some() {
            return MessageContentKind::Audio;
        }
        if m.first_document_item().is_some() {
            return MessageContentKind::Document;
        }
        if m.contact.is_some() {
            return MessageContentKind::Contact;
        }
        if m.location.is_some() {
            return MessageContentKind::Location;
        }
        if m.poll.is_some() {
            return MessageContentKind::Poll;
        }
        if m.media_items.iter().any(|item| item.kind == MessageMediaKind::VideoNote) {
            return MessageContentKind::VideoNote;
        }
        if !m.grid_media_items().is_empty() {
            return MessageContentKind::MediaGrid;
        }
        MessageContentKind::Text
    }

    fn sender_row(&self, env: &MessageContentEnvironment) -> impl IntoElement {
        let chip = env.sender_member.as_ref().filter(|member| !member.custom_title.is_empty()).and_then(|member| {
            let hooks = message_content_member_tag_hooks()?;
            Some((hooks.member_tag_chip)(
                SharedString::from(member.custom_title.clone()),
                SharedString::from(member.custom_color.clone()),
            ))
        });
        let on_open_sender_profile = self.on_open_sender_profile.clone();
        let via_bot_label = self.message.via_bot_label_tag().map(|tag| self.via_bot_label(&tag, env));
        hstack(5.0)
            .child(
                hstack(5.0)
                    .child(
                        MainText::new(self.sender_heading(env))
                            .px(MessageSizes::sender_name_font_size())
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(self.sender_heading_color(env))
                            .number_of_lines(1),
                    )
                    .when(self.message.sender_is_official, |this| this.child(OfficialBadge::new().size(px(12.0))))
                    .children(chip)
                    .bubble_tap_target_priority(self.child_id("sender"), move |window, cx| {
                        if let Some(on_open_sender_profile) = on_open_sender_profile.as_ref() {
                            on_open_sender_profile(window, cx);
                        }
                    }),
            )
            .children(via_bot_label)
    }

    /// tag of the bot that composed the message for someone, shown as via @tag
    /// nothing for a bots own messages since they carry the author already
    fn via_bot_label(&self, tag: &str, env: &MessageContentEnvironment) -> AnyElement {
        let on_message_action = self.on_message_action.clone();
        let color = if self.is_own { env.theme.content_on(env.theme.primary._100).opacity(0.85) } else { env.theme.primary._100 };
        MainText::new(tr("message_via_bot").replace("{tag}", tag))
            .px(MessageSizes::sender_name_font_size())
            .font_weight(FontWeight::MEDIUM)
            .color(color)
            .number_of_lines(1)
            .bubble_tap_target_priority(self.child_id("via-bot"), move |_window, cx| {
                if let Some(on_message_action) = on_message_action.as_ref() {
                    on_message_action(ChatMessageAction::MENTION_VIA_BOT, cx);
                }
            })
            .into_any_element()
    }

    fn show_poll_voters(message: &GetMessageMessage, chat: &ChatInfo, cx: &mut App) {
        let Some(poll) = message.poll.clone() else {
            return;
        };
        let server_id = message.server_id.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or_default().to_owned();
        let chat_id = if message.chat_id.is_empty() { chat.id.clone() } else { message.chat_id.clone() };
        let message_id = if server_id.is_empty() { message.id.clone() } else { server_id };
        PollVotersStore::shared(cx).update(cx, |store, cx| store.present(&chat_id, &message_id, &poll, cx));
    }

    fn kind_content(&self, kind: MessageContentKind, show_sender: bool, env: &MessageContentEnvironment, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let is_own = self.is_own;
        let column = || BubbleColumnLayout::new(6.0).expands_to_proposal(true);
        Some(match kind {
            MessageContentKind::Text => self.text_with_meta(env, cx),
            MessageContentKind::MediaGrid => self.media_with_caption_and_meta(show_sender, env, cx),
            MessageContentKind::Voice => self.voice_with_meta(env, cx),
            MessageContentKind::Audio => self.audio_with_meta(env),
            MessageContentKind::Document => self.documents_with_meta(),
            MessageContentKind::Contact => column()
                .child(MessageContactView::new(self.child_id("contact"), self.message.contact.clone()?).is_own(is_own))
                .children(self.inline_terminal_meta())
                .into_any_element(),
            MessageContentKind::Location => column()
                .child(MessageLocationView::new(self.child_id("location"), self.message.location.clone()?).is_own(is_own))
                .children(self.inline_terminal_meta())
                .into_any_element(),
            MessageContentKind::Poll => {
                let message_id = self.message.id.clone();
                let poll = self.message.poll.clone()?;
                let mut poll_view = MessagePollView::new(self.child_id("poll"), poll.clone()).is_own(is_own).on_vote(
                    move |option_ids, cx| {
                        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.vote_poll(&message_id, option_ids, cx))
                    },
                );
                if PollVotersStore::can_show_voters(&poll, self.chat.r#type) {
                    let (message, chat) = (self.message.clone(), self.chat.clone());
                    poll_view = poll_view.on_show_voters(move |_window, cx| Self::show_poll_voters(&message, &chat, cx));
                }
                column().child(poll_view).children(self.inline_terminal_meta()).into_any_element()
            }
            MessageContentKind::Call => {
                if self.chat.r#type != ChatType::Direct {
                    return None;
                }
                let presentation = MessageCallPresentationFactory::make(&self.message, is_own, cx)?;
                MessageCallView::new(
                    self.child_id("call"),
                    (*self.message).clone(),
                    (*self.chat).clone(),
                    is_own,
                    presentation,
                    env.message_viewport_width,
                )
                .into_any_element()
            }
            MessageContentKind::VideoNote => self.video_note_with_meta(env, window, cx),
            MessageContentKind::PostShort => {
                let mut stack = column();
                if let Some(shared) = self.message.shared_post.clone() {
                    let opened = shared.clone();
                    let open_comments: ButtonAction =
                        Rc::new(move |_window: &mut Window, cx: &mut App| Self::open_shared_post_comments(&opened, cx));
                    stack = stack.child(
                        MessageSharedPostCard::new(self.child_id("shared-post"), shared)
                            .is_own(is_own)
                            .on_open_comments(open_comments),
                    );
                }
                stack.children(self.inline_terminal_meta()).into_any_element()
            }
            MessageContentKind::Sticker => self.sticker_content(env),
        })
    }
}

impl RenderOnce for MessageContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_layout_measurement = is_message_layout_measurement(cx);
        let mount_state = window.use_keyed_state(self.child_id("mounted"), cx, |_, _| MessageContentMountState::default());
        let did_mount = mount_state.read(cx).did_mount;
        if !did_mount {
            mount_state.update(cx, |state, _| state.did_mount = true);
        }
        let saved_author = ChatSavedMessagePresentation::author(&self.message, &self.chat, cx);
        let sender_member = {
            let chat_id = self.message.chat_id.clone();
            let sender_id = self.message.sender_id.clone();
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.member(&chat_id, &sender_id, cx))
        };
        let sender_premium_color = sender_member.as_ref().and_then(|member| {
            PremiumNameColor::color_for_user(Some(member.user_id.as_str()), Some(&member.to_author_info().more), cx)
        });
        let env = MessageContentEnvironment {
            theme: theme_store(cx).read(cx).current_theme().clone(),
            nav_store: env::<NavigationStoreKey>(cx),
            search_highlight_words: chat_search_highlight_words(cx),
            message_viewport_width: chat_message_viewport_width(cx),
            allows_reply_jump: chat_allows_reply_jump(cx),
            is_layout_measurement,
            saved_author,
            sender_member,
            sender_premium_color,
            chat_gallery_messages: env::<ChatGalleryMessagesKey>(cx),
            chat_gallery_paginates: env::<ChatGalleryPaginatesKey>(cx),
            animates_entrance: did_mount && !is_layout_measurement,
        };

        let kind = Self::bubble_kind(&self.message, Some(&self.chat));
        let chromeless = !kind.needs_bubble_padding();
        let show_sender = !self.is_own && self.is_first_in_group && !self.hide_name && self.should_show_sender_name(&env);

        let mut column = BubbleColumnLayout::new(MessageSizes::bubble_column_spacing())
            .expands_to_proposal(self.message.link_preview.is_some() && kind.needs_bubble_padding());
        if self.message.is_disabled_by_moderation() {
            column = column.child(self.moderation_plate(&env.theme));
        }
        if show_sender && !chromeless {
            column = column.child(self.sender_row(&env));
        } else if let Some(tag) = self.message.via_bot_label_tag().filter(|_| !chromeless) {
            column = column.child(self.via_bot_label(&tag, &env));
        }
        if let Some(forward_info) = self.message.forward_info.clone().filter(|_| !chromeless && !Self::hides_forward_header(&env)) {
            let action = Self::forward_source_action(&forward_info, &env);
            let header_width = MessageSizes::available_bubble_width(chat_message_viewport_width(cx), &self.chat, self.is_own)
                - MessageSizes::bubble_inner_horizontal_padding() * 2.0;
            column = column.child(
                MessageForwardHeader::new(self.child_id("forward"), forward_info)
                    .is_own(self.is_own)
                    .on_open_source(action)
                    .max_width(Some(header_width)),
            );
        }
        if let Some(reply) = self.message.reply_to.clone().filter(|_| !chromeless) {
            let action = self.reply_source_action(&reply, &env);
            let quote_width = MessageSizes::available_bubble_width(chat_message_viewport_width(cx), &self.chat, self.is_own)
                - MessageSizes::bubble_inner_horizontal_padding() * 2.0;
            column = column.child(
                MessageReplyHeader::new(self.child_id("reply"), reply)
                    .is_own(self.is_own)
                    .on_open_source(action)
                    .max_width(Some(quote_width)),
            );
        }
        if let Some(story) = self.message.story_reply.clone().filter(|_| !chromeless) {
            let action = Self::story_source_action(&story);
            let quote_width = MessageSizes::available_bubble_width(chat_message_viewport_width(cx), &self.chat, self.is_own)
                - MessageSizes::bubble_inner_horizontal_padding() * 2.0;
            column = column.child(
                MessageStoryReplyHeader::new(self.child_id("story"), story)
                    .is_own(self.is_own)
                    .on_open_story(action)
                    .max_width(Some(quote_width)),
            );
        }
        if let Some(link) = self.message.link_preview.clone().filter(|link| link.is_above_text && kind.needs_bubble_padding()) {
            column = column.child(MessageLinkPreviewView::new(self.child_id("link-above"), link).is_own(self.is_own));
        }

        if let Some(content) = self.kind_content(kind, show_sender, &env, window, cx) {
            column = column.child(content);
        }

        let sections = Self::terminal_sections(
            kind,
            self.message.link_preview.is_some(),
            self.message.has_reaction_row(),
            self.message.link_preview.as_ref().is_some_and(|link| link.is_above_text),
        );
        for section in sections {
            match section {
                MessageContentTerminalSection::LinkPreview => {
                    if let Some(link) = self.message.link_preview.clone().filter(|link| !link.is_above_text) {
                        column = column.child(MessageLinkPreviewView::new(self.child_id("link-below"), link).is_own(self.is_own));
                    }
                }
                MessageContentTerminalSection::Meta => column = column.child(self.trailing_meta(false)),
                MessageContentTerminalSection::Reactions => column = column.child(self.terminal_reactions(kind, &env)),
            }
        }

        let (top, leading, bottom, trailing) = Self::content_insets(kind, env.message_viewport_width);
        let color_scheme = if env.theme.bg._100 == black() { ColorScheme::Dark } else { ColorScheme::Light };

        let media_urls: Vec<String> = self.message.media_items.iter().map(|item| item.url.clone()).collect();
        let change_message = self.message.clone();
        let change_chat = self.chat.clone();
        on_change(self.child_id("media-urls"), media_urls, false, window, cx, move |_, _, _, cx| {
            if !is_layout_measurement {
                Self::note_chat_media(&change_message, &change_chat, cx);
            }
        });
        let appear_message = self.message.clone();
        let appear_chat = self.chat.clone();

        env_scope::<ColorSchemeKey>(Some(color_scheme))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .pt(px(top))
                    .pl(px(leading))
                    .pb(px(bottom))
                    .pr(px(trailing))
                    .child(column),
            )
            .on_appear(self.child_id("note-media"), move |_, cx| {
                if !is_layout_measurement {
                    Self::note_chat_media(&appear_message, &appear_chat, cx);
                }
            })
    }
}

/// what the viewer callbacks keep of the bubble after its render is gone
#[derive(Clone)]
struct MessageContentViewerContext {
    message: Rc<GetMessageMessage>,
    chat: Rc<ChatInfo>,
    is_own: bool,
    on_message_action: Option<MessageContentStringAction>,
    chat_gallery_messages: Option<Arc<Vec<Arc<GetMessageMessage>>>>,
    chat_gallery_paginates: bool,
}

impl MessageContentViewerContext {
    fn open_media_viewer(&self, index: usize, source_frame: Bounds<Pixels>, tapped_cell: &MessageGridCell, layout_is_single: bool, cx: &mut App) {
        let tapped_limit = self
            .message
            .grid_media_items()
            .into_iter()
            .filter(|item| !item.is_cancelled)
            .nth(index)
            .and_then(|item| item.view_limit.clone());
        if let Some(tapped_limit) = tapped_limit.clone().filter(|_| !self.is_own) {
            let my_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default();
            if tapped_limit.is_consumed(Date::now())
                || ChatViewLimitedViewerGuard::is_opened_by_viewer(&self.message, &tapped_limit, &my_id, cx)
            {
                return;
            }
            let chat_id = self.chat.id.clone();
            let message_id = self.message.id.clone();
            let openings = ChatViewLimitedOpenings::shared(cx);
            if !openings.update(cx, |openings, cx| openings.begin(&message_id, cx)) {
                return;
            }
            let viewed =
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.mark_media_viewed(&chat_id, &message_id, false, cx));
            let context = self.clone();
            let tapped_cell = tapped_cell.clone();
            cx.spawn(async move |cx| {
                let landed = viewed.await;
                cx.update(|cx| {
                    if !landed {
                        openings.update(cx, |openings, cx| openings.cancel(&message_id, cx));
                        return;
                    }
                    let presenter = chat_media_presentation(cx);
                    ChatViewLimitedViewerGuard::watch(&presenter, &tapped_limit, &chat_id, &message_id, cx);
                    context.open_media_viewer_contents(index, source_frame, &tapped_cell, layout_is_single, true, cx);
                });
            })
            .detach();
            return;
        }
        self.open_media_viewer_contents(index, source_frame, tapped_cell, layout_is_single, tapped_limit.is_some(), cx);
    }

    fn cancel_outgoing_media(&self, media_item: &MessageMediaItem, cx: &mut App) {
        let (message, chat) = (self.message.clone(), self.chat.clone());
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.cancel_outgoing_media(&message, &chat, media_item, cx));
    }

    fn open_media_viewer_contents(
        &self,
        index: usize,
        source_frame: Bounds<Pixels>,
        tapped_cell: &MessageGridCell,
        layout_is_single: bool,
        view_limited: bool,
        cx: &mut App,
    ) {
        let message = self.message.clone();
        let chat = self.chat.clone();
        let on_message_action = self.on_message_action.clone();
        let only_message = || Arc::new(vec![Arc::new((*message).clone())]);
        let gallery_messages: Arc<Vec<Arc<GetMessageMessage>>> = if view_limited {
            only_message()
        } else {
            self.chat_gallery_messages.clone().unwrap_or_else(only_message)
        };
        let source_messages: Option<&[Arc<GetMessageMessage>]> =
            if view_limited { Some(gallery_messages.as_slice()) } else { self.chat_gallery_messages.as_deref().map(Vec::as_slice) };
        let mut items = ChatMediaGalleryBuilder::items_messages(&chat, source_messages, &message, index, cx);
        if view_limited {
            items.retain(|item| item.message_id == message.id && item.index_in_message == index);
        }
        let open_index = items.iter().position(|item| item.message_id == message.id && item.index_in_message == index);
        tracing::warn!(
            target: "riseonly::X638",
            "open viewer message={} index={} items={} open_index={:?} single={} sizes={:?}",
            message.id,
            index,
            items.len(),
            open_index,
            layout_is_single,
            message.grid_media_items().iter().map(|item| (item.width, item.height)).collect::<Vec<_>>()
        );
        let Some(open_index) = open_index else {
            return;
        };
        let grid_media: Vec<&MessageMediaItem> = message.grid_media_items().into_iter().filter(|item| !item.is_cancelled).collect();
        let gallery_anchor = grid_media.get(index).and_then(|item| {
            if item.kind == MessageMediaKind::Image || item.kind == MessageMediaKind::Video {
                ChatMediaGalleryBuilder::window_identity_message(&message, item)
            } else {
                None
            }
        });
        let lift_id = MediaViewerFrameKeys::chat_bubble(&tapped_cell.render_id);
        let mut skin = ChatMediaHeroSkin::new(tapped_cell.display_source.clone(), layout_is_single);
        skin.media_tail = MediaCellScreenFrameRegistry::media_tail_presentation(&lift_id);
        skin.source_media_radii = MediaCellScreenFrameRegistry::source_media_radii(&lift_id);
        let paginates = self.chat_gallery_paginates;
        let uses_window = paginates && gallery_anchor.is_some();

        let handlers = MediaViewerPresentationHandlers {
            on_delete: Some(Rc::new({
                let message = message.clone();
                let chat = chat.clone();
                let on_message_action = on_message_action.clone();
                move |item: &MediaViewerGalleryItem, mode: MediaViewerDeleteMode, cx: &mut App| {
                    let for_everyone = mode == MediaViewerDeleteMode::ForEveryone;
                    if message.matches_message_id(&item.message_id) {
                        if let Some(on_message_action) = on_message_action.as_ref() {
                            on_message_action(
                                if for_everyone { ChatMessageAction::DELETE_EVERYONE } else { ChatMessageAction::DELETE_ME },
                                cx,
                            );
                        }
                        return;
                    }
                    let chat_id = ChatServicesStore::shared(cx).read(cx).real_chat_id(&chat.id, cx).unwrap_or_else(|| chat.id.clone());
                    if on_message_action.is_none() || chat_id.is_empty() {
                        return;
                    }
                    let ids = [item.message_id.clone()];
                    MessageInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.delete_messages(&ids, &chat_id, None, None, for_everyone, cx));
                }
            })),
            on_forward: Some(Rc::new({
                let chat = chat.clone();
                let gallery_messages = gallery_messages.clone();
                move |item: &MediaViewerGalleryItem, cx: &mut App| {
                    let captured = gallery_messages.iter().find(|message| message.matches_message_id(&item.message_id));
                    let topic_id = captured.and_then(|message| message.topic_id.as_deref());
                    let source = ChatMediaGalleryBuilder::live_message(&item.message_id, &chat.id, topic_id, cx).or_else(|| captured.map(|message| (**message).clone()));
                    let Some(source) = source else {
                        return;
                    };
                    let is_own = MessageList::is_own_bubble(&source, &chat, cx);
                    if !MessagePermissions::new(&source, &chat, is_own).can_forward(cx) {
                        return;
                    }
                    ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_messages(vec![source], cx));
                }
            })),
            on_show_in_context: Some(Rc::new({
                let on_message_action = on_message_action.clone();
                move |item: &MediaViewerGalleryItem, cx: &mut App| {
                    if let Some(on_message_action) = on_message_action.as_ref() {
                        on_message_action(&ChatMessageAction::jump_to_message_action(&item.message_id), cx);
                    }
                }
            })),
            on_reply: Some(Rc::new({
                let message = message.clone();
                let on_message_action = on_message_action.clone();
                move |item: &MediaViewerGalleryItem, cx: &mut App| {
                    if let Some(on_message_action) = on_message_action.as_ref() {
                        if message.matches_message_id(&item.message_id) {
                            on_message_action(ChatMessageAction::REPLY, cx);
                        } else {
                            on_message_action(&ChatMessageAction::reply_to_message_action(&item.message_id), cx);
                        }
                    }
                }
            })),
            ..MediaViewerPresentationHandlers::default()
        };
        let presenter = chat_media_presentation(cx);
        tracing::warn!(target: "riseonly::X638", "present starts hero_source={:?}", skin.display_source);
        cx.spawn(async move |cx| {
            MediaViewerOpen::present(
                presenter.clone(),
                items,
                open_index,
                source_frame,
                lift_id,
                skin,
                MediaViewerChrome::CHAT,
                if !view_limited && uses_window { MediaViewerGalleryScope::Paged } else { MediaViewerGalleryScope::Complete },
                handlers,
                cx,
            )
            .await;
            tracing::warn!(target: "riseonly::X638", "present returned (hero warm load finished)");
            cx.update(|cx| {
                let Some(gallery_anchor) = gallery_anchor.filter(|_| uses_window) else {
                    ChatMediaGalleryBuilder::follow_landing_rows(&presenter, &chat, gallery_messages.as_slice(), cx);
                    return;
                };
                let topic_id = ChatMediaGalleryBuilder::window_topic_id(gallery_messages.as_slice());
                let seed = ChatMediaGalleryBuilder::window_seed_messages(gallery_messages.as_slice());
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.open_media_gallery(Uuid::new_v4(), &chat.id, topic_id.as_deref(), seed, &gallery_anchor, cx);
                });
            });
        })
        .detach();
    }
}

fn ultra_thin_material_fill(is_dark: bool) -> Hsla {
    let opacity = Material::UltraThin.painted_opacity();
    if is_dark { black().opacity(opacity) } else { white().opacity(opacity) }
}

/// flex column that stretches each child to the widest one
/// taffy keeps the same per width measure cache
#[derive(IntoElement)]
pub struct BubbleColumnLayout {
    spacing: f32,
    expands_to_proposal: bool,
    width_source_index: Option<usize>,
    fixed_width: Option<f32>,
    children: Vec<AnyElement>,
}

impl BubbleColumnLayout {
    pub fn new(spacing: f32) -> Self {
        Self { spacing, expands_to_proposal: false, width_source_index: None, fixed_width: None, children: Vec::new() }
    }

    pub fn fixed_width(mut self, fixed_width: Option<f32>) -> Self {
        self.fixed_width = fixed_width.filter(|width| *width > 0.0);
        self
    }

    pub fn expands_to_proposal(mut self, expands_to_proposal: bool) -> Self {
        self.expands_to_proposal = expands_to_proposal;
        self
    }

    pub fn width_source_index(mut self, width_source_index: Option<usize>) -> Self {
        self.width_source_index = width_source_index;
        self
    }
}

impl ParentElement for BubbleColumnLayout {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for BubbleColumnLayout {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let width_source_index = self.width_source_index.filter(|index| *index < self.children.len());
        let fixed_width = self.fixed_width;
        let children = self.children.into_iter().enumerate().map(|(index, child)| match (width_source_index, fixed_width) {
            (Some(source), Some(width)) if source != index => div().w(px(width)).max_w_full().child(child).into_any_element(),
            (Some(source), None) if source != index => div().w(px(0.0)).min_w_full().child(child).into_any_element(),
            _ => child,
        });
        div()
            .flex()
            .flex_col()
            .gap(px(self.spacing))
            .max_w_full()
            .when(self.expands_to_proposal, |this| this.w_full())
            .when_some(fixed_width, |this, width| this.w(px(width)))
            .children(children)
    }
}

pub enum TelegramReactionMetrics {}

impl TelegramReactionMetrics {
    pub const HEIGHT: f32 = 30.0;
    pub const CORNER_RADIUS: f32 = 15.0;
    pub const ICON_SIZE: f32 = 18.0;
    pub const ICON_SLOT_SIZE: f32 = 22.0;
    pub const AVATAR_SIZE: f32 = 24.0;
    pub const HORIZONTAL_PADDING: f32 = 11.0;
    pub const CONTENT_SPACING: f32 = 2.0;
    pub const AVATAR_SPACING: f32 = 4.0;
    pub const COUNT_FONT_SIZE: f32 = 11.0;
    pub const AVATAR_OVERLAP: f32 = 10.0;

    pub fn custom_emoji_size() -> f32 {
        CustomEmojiGlyphMetrics::ink_height(Self::ICON_SIZE)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TelegramReactionLayoutFlowItem {
    pub index: usize,
    pub row: usize,
    pub frame: Bounds<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TelegramReactionLayoutFlowPlan {
    pub items: Vec<TelegramReactionLayoutFlowItem>,
    pub size: Size<f32>,
}

static COUNTER_TEXT_SYSTEM: OnceLock<Arc<TextSystem>> = OnceLock::new();

pub enum TelegramReactionLayout {}

impl TelegramReactionLayout {
    pub const ITEM_SPACING: f32 = 6.0;
    pub const LINE_SPACING: f32 = 6.0;
    pub const BOTTOM_INSET: f32 = 2.0;
    pub const EDGE_OVERFLOW: f32 = 1.0;

    /// hands the app text system to the off main height estimate, later calls do nothing
    pub fn install_text_system(cx: &App) {
        if COUNTER_TEXT_SYSTEM.get().is_none() {
            let _ = COUNTER_TEXT_SYSTEM.set(cx.text_system().clone());
        }
    }

    pub fn estimated_height(
        reactions: &[MessageReaction],
        maximum_width: f32,
        is_outgoing: bool,
        current_user_id: Option<&str>,
        sparks_total: i64,
        hides_reactors: bool,
    ) -> f32 {
        if reactions.is_empty() && sparks_total <= 0 {
            return 0.0;
        }
        let shows_avatars = !hides_reactors && Self::shows_complete_avatar_set(reactions, current_user_id);
        let mut sizes: Vec<Size<f32>> = reactions
            .iter()
            .map(|reaction| size(Self::pill_width(reaction, shows_avatars), TelegramReactionMetrics::HEIGHT))
            .collect();
        if sparks_total > 0 {
            sizes.insert(0, size(Self::sparks_pill_width(sparks_total), TelegramReactionMetrics::HEIGHT));
        }
        Self::flow_plan(&sizes, maximum_width, is_outgoing).size.height
    }

    pub fn pill_width(reaction: &MessageReaction, shows_avatars: bool) -> f32 {
        let avatar_count = reaction.reactors.len().min(3);
        if shows_avatars && avatar_count > 0 {
            return TelegramReactionMetrics::ICON_SLOT_SIZE
                + TelegramReactionMetrics::HORIZONTAL_PADDING * 2.0
                + TelegramReactionMetrics::AVATAR_SPACING
                + TelegramReactionMetrics::AVATAR_SIZE
                + if avatar_count == 1 { -2.0 } else { (avatar_count - 1) as f32 * 12.0 };
        }

        let count = Self::compact_count_string(reaction.count);
        let base_width = TelegramReactionMetrics::HORIZONTAL_PADDING * 2.0 + TelegramReactionMetrics::ICON_SLOT_SIZE;
        if count.is_empty() {
            return base_width - 1.0;
        }
        base_width + TelegramReactionMetrics::CONTENT_SPACING + Self::counter_width(&count)
    }

    /// sparks chip is an icon pill with a counter, never avatars
    /// same geometry as an emoji pill with count so the row wraps the same in estimate and on screen
    pub fn sparks_pill_width(total: i64) -> f32 {
        TelegramReactionMetrics::HORIZONTAL_PADDING * 2.0
            + TelegramReactionMetrics::ICON_SLOT_SIZE
            + TelegramReactionMetrics::CONTENT_SPACING
            + Self::counter_width(&RiseFormat::compact_count(total))
    }

    /// empty string at zero means no counter at all, the pill shrinks to one icon
    /// its width is computed from here too
    pub fn compact_count_string(count: i64) -> String {
        if count == 0 {
            return String::new();
        }
        RiseFormat::compact_count(count)
    }

    pub fn shows_complete_avatar_set(reactions: &[MessageReaction], current_user_id: Option<&str>) -> bool {
        if reactions.is_empty() {
            return false;
        }
        let maximum = rise_limits::signed(rise_limits::limit::reactions_per_message(false));
        let mut total_count = 0_i64;
        for reaction in reactions {
            if !(0..=maximum).contains(&reaction.count) {
                return false;
            }
            total_count += reaction.count;
            if total_count > maximum {
                return false;
            }
        }
        if total_count <= 0 {
            return false;
        }
        let normalized_current_user_id = current_user_id.map(trimming_whitespaces_and_newlines).filter(|id| !id.is_empty());

        for reaction in reactions {
            if reaction.count < 0 {
                return false;
            }
            let reactor_ids: Vec<&str> = reaction.reactors.iter().map(|reactor| trimming_whitespaces_and_newlines(&reactor.user_id)).collect();
            let unique: HashSet<&str> = reactor_ids.iter().copied().collect();
            if reactor_ids.iter().any(|id| id.is_empty())
                || unique.len() != reactor_ids.len()
                || reactor_ids.len() as i64 != reaction.count
            {
                return false;
            }
            let contains_current = normalized_current_user_id.is_some_and(|current| reactor_ids.contains(&current));
            if reaction.reacted_by_me {
                if !contains_current {
                    return false;
                }
            } else if contains_current {
                return false;
            }
        }
        true
    }

    pub fn displayed_reactors(reactors: &[ReactionReactor], current_user_id: Option<&str>) -> Vec<ReactionReactor> {
        let mut result = reactors.to_vec();
        let normalized_current_user_id = current_user_id.map(trimming_whitespaces_and_newlines).filter(|id| !id.is_empty());
        let current_index = normalized_current_user_id
            .and_then(|current| result.iter().position(|reactor| trimming_whitespaces_and_newlines(&reactor.user_id) == current))
            .filter(|index| *index != 0);
        if let Some(current_index) = current_index {
            let current = result.remove(current_index);
            result.insert(0, current);
        }
        result.truncate(3);
        result
    }

    pub fn flow_plan(item_sizes: &[Size<f32>], maximum_width: f32, is_outgoing: bool) -> TelegramReactionLayoutFlowPlan {
        if item_sizes.is_empty() {
            return TelegramReactionLayoutFlowPlan { items: Vec::new(), size: size(0.0, 0.0) };
        }
        let available_width = maximum_width.max(1.0);
        let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
        let mut row_widths: Vec<f32> = vec![0.0];
        let mut row_heights: Vec<f32> = vec![0.0];

        for (index, item) in item_sizes.iter().enumerate() {
            let row_index = rows.len() - 1;
            let spacing = if rows[row_index].is_empty() { 0.0 } else { Self::ITEM_SPACING };
            if !rows[row_index].is_empty() && row_widths[row_index] + spacing + item.width > available_width {
                rows.push(vec![index]);
                row_widths.push(item.width);
                row_heights.push(item.height);
            } else {
                rows[row_index].push(index);
                row_widths[row_index] += spacing + item.width;
                row_heights[row_index] = row_heights[row_index].max(item.height);
            }
        }

        let content_width = row_widths.iter().copied().fold(0.0_f32, f32::max);
        let mut result = Vec::with_capacity(item_sizes.len());
        let mut y = 0.0_f32;
        let row_count = rows.len();
        for (row_index, row) in rows.iter().enumerate() {
            let row_height = row_heights[row_index];
            let mut x = if is_outgoing { content_width + Self::EDGE_OVERFLOW } else { -Self::EDGE_OVERFLOW };
            for &index in row {
                let item = item_sizes[index];
                if is_outgoing {
                    x -= item.width;
                }
                result.push(TelegramReactionLayoutFlowItem {
                    index,
                    row: row_index,
                    frame: Bounds { origin: point(x, y + (row_height - item.height) / 2.0), size: item },
                });
                x += if is_outgoing { -Self::ITEM_SPACING } else { item.width + Self::ITEM_SPACING };
            }
            y += row_height;
            if row_index < row_count - 1 {
                y += Self::LINE_SPACING;
            }
        }

        TelegramReactionLayoutFlowPlan { items: result, size: size(content_width, y + Self::BOTTOM_INSET) }
    }

    fn counter_width(count: &str) -> f32 {
        let Some(text_system) = COUNTER_TEXT_SYSTEM.get() else {
            return (count.chars().count() as f32 * COUNTER_ESTIMATED_GLYPH_WIDTH).ceil();
        };
        let run = TextRun {
            len: count.len(),
            font: GpuiFont {
                family: SharedString::new_static(FONT_FAMILY),
                features: FontFeatures(Arc::new(vec![("tnum".to_string(), 1)])),
                fallbacks: None,
                weight: FontWeight::MEDIUM,
                style: FontStyle::Normal,
            },
            color: black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = WindowTextSystem::new(text_system.clone()).shape_line(
            SharedString::from(count.to_owned()),
            px(TelegramReactionMetrics::COUNT_FONT_SIZE),
            &[run],
            None,
        );
        f32::from(line.width()).ceil()
    }
}

/// plain values only so the chip compares exactly and no closure goes into the row
#[derive(Clone, Debug, PartialEq)]
pub struct ReactionsSparksChip {
    pub total: i64,
    pub mine: i64,
    pub target: SparksTarget,
    pub can_send: bool,
}

/// entrance animation of a pill or row appearing under the bubble reaction anim
/// inserted into a mounted bubble it runs, one that came with the bubble starts settled
fn entrance_progress(id: ElementId, animates_entrance: bool, window: &mut Window, cx: &mut App) -> f32 {
    let now = RiseClock::monotonic();
    let mut created = false;
    let state = window.use_keyed_state(id, cx, |_, _| {
        created = true;
        Animated::new(if animates_entrance { 0.0_f32 } else { 1.0_f32 })
    });
    if created && animates_entrance {
        state.update(cx, |progress, cx| progress.set_in(1.0, spring(0.34, 0.86), now, cx));
    }
    state.read(cx).frame_value(now, window)
}

const FLOW_SLOT_EPSILON: f32 = 0.01;
const REACTIONS_FLOW_SPARKS_KEY: &str = "sparks";
const REACTIONS_FLOW_META_KEY: &str = "meta";
const REACTION_PILL_TRANSITION_SCALE: f32 = 0.6;
const REACTIONS_ROW_TRANSITION_SCALE: f32 = 0.72;

fn reactions_change_motion() -> MotionSpec {
    spring(0.34, 0.86)
}

fn reaction_pill_transition() -> Transition {
    Transition::scale(REACTION_PILL_TRANSITION_SCALE).combined(Transition::opacity())
}

/// where each flow child is drawn
/// a staying child slides to its new place and a departing one holds its last place
#[derive(Default)]
struct ReactionsFlowMotion {
    spec: Option<MotionSpec>,
    slots: HashMap<SharedString, (Animated<f32>, Animated<f32>)>,
}

impl ReactionsFlowMotion {
    fn displayed(&mut self, key: SharedString, target: Point<f32>, window: &Window, cx: &App) -> Point<f32> {
        let now = RiseClock::monotonic();
        let spec = self.spec;
        let slot = self.slots.entry(key).or_insert_with(|| (Animated::new(target.x), Animated::new(target.y)));
        if (*slot.0.target() - target.x).abs() > FLOW_SLOT_EPSILON {
            slot.0.set_in(target.x, spec, now, cx);
        }
        if (*slot.1.target() - target.y).abs() > FLOW_SLOT_EPSILON {
            slot.1.set_in(target.y, spec, now, cx);
        }
        point(slot.0.frame_value(now, window), slot.1.frame_value(now, window))
    }

    fn last_place(&self, key: &SharedString, window: &Window) -> Point<f32> {
        let now = RiseClock::monotonic();
        self.slots
            .get(key)
            .map_or_else(|| point(0.0, 0.0), |slot| point(slot.0.frame_value(now, window), slot.1.frame_value(now, window)))
    }

    fn retain_keys<'a>(&mut self, keys: impl Iterator<Item = &'a SharedString>) {
        let keep: HashSet<&SharedString> = keys.collect();
        self.slots.retain(|key, _| keep.contains(key));
    }
}

struct LeavingReaction {
    reaction_id: String,
    presence: Presence<MessageReaction>,
}

struct ReactionsRowTransitions {
    row: Presence<()>,
    previous_reactions: Vec<MessageReaction>,
    leaving_reactions: Vec<LeavingReaction>,
    previous_sparks: Option<ReactionsSparksChip>,
    leaving_sparks: Option<Presence<ReactionsSparksChip>>,
    flow_motion: Rc<RefCell<ReactionsFlowMotion>>,
}

impl ReactionsRowTransitions {
    fn new(reactions: &[MessageReaction], sparks: Option<&ReactionsSparksChip>, anchor: UnitPoint) -> Self {
        let has_content = !reactions.is_empty() || sparks.is_some();
        Self {
            row: Presence::new(has_content.then_some(())).with_transition(
                Transition::scale_anchored(REACTIONS_ROW_TRANSITION_SCALE, anchor).combined(Transition::opacity()),
            ),
            previous_reactions: reactions.to_vec(),
            leaving_reactions: Vec::new(),
            previous_sparks: sparks.cloned(),
            leaving_sparks: None,
            flow_motion: Rc::new(RefCell::new(ReactionsFlowMotion::default())),
        }
    }

    fn reconcile(
        &mut self,
        reactions: &[MessageReaction],
        sparks: Option<&ReactionsSparksChip>,
        motion: Option<MotionSpec>,
        now: MotionTime,
        cx: &App,
    ) {
        let has_content = !reactions.is_empty() || sparks.is_some();
        self.row.set_in(has_content.then_some(()), motion, now, cx);
        self.flow_motion.borrow_mut().spec = motion;

        for previous in std::mem::take(&mut self.previous_reactions) {
            if motion.is_none() || reactions.iter().any(|reaction| reaction.id() == previous.id()) {
                continue;
            }
            let reaction_id = previous.id().to_owned();
            let mut presence = Presence::new(Some(previous)).with_transition(reaction_pill_transition());
            presence.set_in(None, motion, now, cx);
            self.leaving_reactions.push(LeavingReaction { reaction_id, presence });
        }
        self.previous_reactions = reactions.to_vec();
        self.leaving_reactions.retain(|leaving| {
            leaving.presence.value(now).is_some() && !reactions.iter().any(|reaction| reaction.id() == leaving.reaction_id)
        });

        if sparks.is_some() {
            self.leaving_sparks = None;
        } else if let Some(previous) = self.previous_sparks.take().filter(|_| motion.is_some()) {
            let mut presence = Presence::new(Some(previous)).with_transition(reaction_pill_transition());
            presence.set_in(None, motion, now, cx);
            self.leaving_sparks = Some(presence);
        }
        self.previous_sparks = sparks.cloned();
        if self.leaving_sparks.as_ref().is_some_and(|presence| presence.value(now).is_none()) {
            self.leaving_sparks = None;
        }
    }

    fn is_showing(&self, now: MotionTime) -> bool {
        self.row.value(now).is_some()
    }
}

#[derive(IntoElement)]
pub struct ReactionsAndMetaRow {
    id: ElementId,
    reactions: Vec<MessageReaction>,
    is_own: bool,
    on_reaction: Option<MessageContentStringAction>,
    meta: Option<AnyElement>,
    spacing: f32,
    ideal_maximum_width: f32,
    sparks: Option<ReactionsSparksChip>,
    hides_reactors: bool,
    animates_entrance: bool,
}

impl ReactionsAndMetaRow {
    pub fn new(id: impl Into<ElementId>, reactions: Vec<MessageReaction>, is_own: bool) -> Self {
        Self {
            id: id.into(),
            reactions,
            is_own,
            on_reaction: None,
            meta: None,
            spacing: 6.0,
            ideal_maximum_width: MessageSizes::bubble_max_width(),
            sparks: None,
            hides_reactors: false,
            animates_entrance: false,
        }
    }

    pub fn hides_reactors(mut self, hides_reactors: bool) -> Self {
        self.hides_reactors = hides_reactors;
        self
    }

    pub fn on_reaction(mut self, on_reaction: impl Into<Option<MessageContentStringAction>>) -> Self {
        self.on_reaction = on_reaction.into();
        self
    }

    pub fn meta(mut self, meta: impl Into<Option<AnyElement>>) -> Self {
        self.meta = meta.into();
        self
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn ideal_maximum_width(mut self, ideal_maximum_width: f32) -> Self {
        self.ideal_maximum_width = ideal_maximum_width;
        self
    }

    pub fn sparks(mut self, sparks: impl Into<Option<ReactionsSparksChip>>) -> Self {
        self.sparks = sparks.into();
        self
    }

    pub fn animates_entrance(mut self, animates_entrance: bool) -> Self {
        self.animates_entrance = animates_entrance;
        self
    }

    fn child_id(&self, name: &str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::from(name.to_owned()))
    }
}

impl RenderOnce for ReactionsAndMetaRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let anchor = if self.is_own { UnitPoint::TOP_TRAILING } else { UnitPoint::TOP_LEADING };
        let transitions = {
            let (reactions, sparks) = (&self.reactions, self.sparks.as_ref());
            window.use_keyed_state(self.child_id("transitions"), cx, |_, _| ReactionsRowTransitions::new(reactions, sparks, anchor))
        };
        let motion = self.animates_entrance.then(reactions_change_motion);
        transitions.update(cx, |transitions, cx| {
            transitions.reconcile(&self.reactions, self.sparks.as_ref(), motion, now, cx);
        });
        if !transitions.read(cx).is_showing(now) {
            return div()
                .when_some(self.meta, |this, meta| this.child(hstack(0.0).child(spacer_min_w(0.0)).child(meta)))
                .into_any_element();
        }
        TelegramReactionLayout::install_text_system(cx);
        let current_user_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone());
        let shows_avatars =
            !self.hides_reactors && TelegramReactionLayout::shows_complete_avatar_set(&self.reactions, current_user_id.as_deref());
        let theme_background = theme_store(cx).read(cx).current_theme().bg._100;
        let is_dark_theme = TelegramReactionPalette::is_dark_background(theme_background);

        let mut children: Vec<AnyElement> = Vec::with_capacity(self.reactions.len() + 2);
        let mut keys: Vec<SharedString> = Vec::with_capacity(self.reactions.len() + 2);
        if let Some(sparks) = self.sparks.clone() {
            keys.push(SharedString::new_static(REACTIONS_FLOW_SPARKS_KEY));
            children.push(
                SparksReactionPill {
                    id: self.child_id("sparks"),
                    chip: sparks,
                    is_own: self.is_own,
                    is_dark_theme,
                    animates_entrance: self.animates_entrance,
                }
                .into_any_element(),
            );
        }
        for reaction in &self.reactions {
            let emoji = reaction.emoji.clone();
            let on_reaction = self.on_reaction.clone();
            keys.push(SharedString::from(format!("pill-{}", reaction.id())));
            children.push(
                ReactionPill {
                    id: self.child_id(&format!("pill-{}", reaction.id())),
                    reaction: reaction.clone(),
                    is_own: self.is_own,
                    shows_avatars,
                    current_user_id: current_user_id.clone(),
                    is_dark_theme,
                    animates_entrance: self.animates_entrance,
                    on_tap: Rc::new(move |cx: &mut App| {
                        if let Some(on_reaction) = on_reaction.as_ref() {
                            on_reaction(&emoji, cx);
                        }
                    }),
                }
                .into_any_element(),
            );
        }
        let has_meta = self.meta.is_some();
        if has_meta {
            keys.push(SharedString::new_static(REACTIONS_FLOW_META_KEY));
        }

        let transitions = transitions.read(cx);
        let mut leaving: Vec<(SharedString, AnyElement)> = Vec::with_capacity(transitions.leaving_reactions.len() + 1);
        let leaving_sparks = transitions.leaving_sparks.as_ref().and_then(|presence| {
            presence.render_transitioned(now, window, |chip| {
                div().child(SparksReactionPill {
                    id: self.child_id("sparks"),
                    chip: chip.clone(),
                    is_own: self.is_own,
                    is_dark_theme,
                    animates_entrance: false,
                })
            })
        });
        if let Some(element) = leaving_sparks {
            leaving.push((SharedString::new_static(REACTIONS_FLOW_SPARKS_KEY), element.into_any_element()));
        }
        for departing in &transitions.leaving_reactions {
            let element = departing.presence.render_transitioned(now, window, |reaction| {
                div().child(ReactionPill {
                    id: self.child_id(&format!("pill-{}", departing.reaction_id)),
                    reaction: reaction.clone(),
                    is_own: self.is_own,
                    shows_avatars,
                    current_user_id: current_user_id.clone(),
                    is_dark_theme,
                    animates_entrance: false,
                    on_tap: Rc::new(|_: &mut App| {}),
                })
            });
            if let Some(element) = element {
                leaving.push((SharedString::from(format!("pill-{}", departing.reaction_id)), element.into_any_element()));
            }
        }

        children.extend(self.meta);
        let flow = ReactionsMetaFlow {
            spacing: self.spacing,
            line_spacing: TelegramReactionLayout::LINE_SPACING,
            has_meta,
            ideal_maximum_width: self.ideal_maximum_width,
            is_outgoing: self.is_own,
            children,
            keys,
            leaving,
            motion: transitions.flow_motion.clone(),
        };
        transitions
            .row
            .render_transitioned(now, window, move |_| div().child(flow))
            .map_or_else(|| div().into_any_element(), IntoElement::into_any_element)
    }
}

/// geometry is fixed by the sparks pill width and reaction metrics
/// list height estimate and cache rely on it. the burst only scales the icon
#[derive(IntoElement)]
struct SparksReactionPill {
    id: ElementId,
    chip: ReactionsSparksChip,
    is_own: bool,
    is_dark_theme: bool,
    animates_entrance: bool,
}

impl SparksReactionPill {
    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn activate(chip: &ReactionsSparksChip, taps: &Entity<i64>, cx: &mut App) {
        let store = SparksInteractionsStore::shared(cx);
        if !chip.can_send {
            store.update(cx, |store, cx| store.open_send_sheet(chip.target.clone(), true, None, cx));
            return;
        }
        if store.update(cx, |store, cx| store.tap(&chip.target, None, cx)) {
            taps.update(cx, |taps, cx| {
                *taps = taps.wrapping_add(1);
                cx.notify();
            });
        }
    }
}

impl RenderOnce for SparksReactionPill {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme_store(cx).read(cx).current_theme().clone();
        let is_layout_measurement = is_message_layout_measurement(cx);
        let taps = window.use_keyed_state(self.child_id("taps"), cx, |_, _| 0_i64);
        let is_mine = self.chip.mine > 0;
        let tint = SettingsIconPalette::sparks();
        let background = match (self.is_own, is_mine) {
            (true, true) => white(),
            (true, false) => white().opacity(0.12),
            (false, true) => tint.opacity(0.9),
            (false, false) => tint.opacity(if self.is_dark_theme { 0.2 } else { 0.16 }),
        };
        let foreground = match (self.is_own, is_mine) {
            (true, true) => theme.primary._100,
            (true, false) => white(),
            (false, true) => black().opacity(0.85),
            (false, false) => theme.text.primary,
        };
        let progress = entrance_progress(self.child_id("entrance"), self.animates_entrance, window, cx);
        let chip = self.chip.clone();
        let tap_id = self.child_id("tap");
        let tap_state = taps.clone();
        hstack(TelegramReactionMetrics::CONTENT_SPACING)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .frame_size(TelegramReactionMetrics::ICON_SLOT_SIZE, TelegramReactionMetrics::ICON_SLOT_SIZE)
                    .child(
                        SparksBurstIcon::new(TelegramReactionMetrics::ICON_SIZE, self.chip.target.clone(), *taps.read(cx))
                            .is_live(!is_layout_measurement),
                    ),
            )
            .child(
                MainText::new(RiseFormat::compact_count(self.chip.total))
                    .px(TelegramReactionMetrics::COUNT_FONT_SIZE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .monospaced_digits(true)
                    .color(foreground),
            )
            .pl(px(TelegramReactionMetrics::HORIZONTAL_PADDING))
            .frame_size(TelegramReactionLayout::sparks_pill_width(self.chip.total), TelegramReactionMetrics::HEIGHT)
            .continuous_corners(TelegramReactionMetrics::CORNER_RADIUS)
            .bg(background)
            .id(self.child_id("label"))
            .tooltip_label(format!("{}, {}", tr("sparks_currency_name"), self.chip.total))
            .opacity(progress)
            .scale_effect(0.6 + 0.4 * progress)
            .bubble_tap_target(tap_id, move |_window, cx| Self::activate(&chip, &tap_state, cx))
    }
}

#[derive(IntoElement)]
struct ReactionPill {
    id: ElementId,
    reaction: MessageReaction,
    is_own: bool,
    shows_avatars: bool,
    current_user_id: Option<String>,
    is_dark_theme: bool,
    animates_entrance: bool,
    on_tap: Rc<dyn Fn(&mut App)>,
}

impl ReactionPill {
    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn background(&self, is_mine: bool, theme: &AppTheme) -> Hsla {
        if self.is_own {
            return if is_mine { theme.content_on(theme.primary._100) } else { theme.content_on(theme.primary._100).opacity(0.12) };
        }
        if is_mine {
            return theme.primary._100;
        }
        if self.is_dark_theme { white().opacity(0.07) } else { theme.primary._100.opacity(0.10) }
    }

    fn foreground(&self, is_mine: bool, theme: &AppTheme) -> Hsla {
        if self.is_own {
            return if is_mine { theme.primary._100 } else { theme.content_on(theme.primary._100) };
        }
        if is_mine {
            return theme.content_on(theme.primary._100);
        }
        if self.is_dark_theme { white() } else { theme.primary._100 }
    }
}

impl RenderOnce for ReactionPill {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme_store(cx).read(cx).current_theme().clone();
        let is_mine = self.reaction.reacted_by_me;
        let bg = self.background(is_mine, &theme);
        let avatars = TelegramReactionLayout::displayed_reactors(&self.reaction.reactors, self.current_user_id.as_deref());
        let uses_avatars = self.shows_avatars && !avatars.is_empty();
        let count = TelegramReactionLayout::compact_count_string(self.reaction.count);
        let width = TelegramReactionLayout::pill_width(&self.reaction, self.shows_avatars);
        let progress = entrance_progress(self.child_id("entrance"), self.animates_entrance, window, cx);

        let glyph = match self.reaction.custom_emoji_id().filter(|_| self.reaction.is_custom_emoji()) {
            Some(id) => div()
                .flex()
                .items_center()
                .justify_center()
                .frame_size(TelegramReactionMetrics::ICON_SLOT_SIZE, TelegramReactionMetrics::ICON_SLOT_SIZE)
                .child(CustomEmojiReactionGlyph {
                    id: self.child_id("custom-glyph"),
                    custom_emoji_id: id,
                    size: TelegramReactionMetrics::custom_emoji_size(),
                })
                .into_any_element(),
            None => BuiltinReactionGlyph {
                emoji: self.reaction.emoji.clone(),
                size: TelegramReactionMetrics::ICON_SIZE,
                slot_size: TelegramReactionMetrics::ICON_SLOT_SIZE,
            }
            .into_any_element(),
        };
        let spacing = if uses_avatars { TelegramReactionMetrics::AVATAR_SPACING } else { TelegramReactionMetrics::CONTENT_SPACING };
        let mut row = hstack(spacing).child(glyph);
        if uses_avatars {
            row = row.child(ReactionAvatarStack { id: self.child_id("avatars"), reactors: avatars, ring_color: bg });
        } else if !count.is_empty() {
            row = row.child(
                MainText::new(count)
                    .px(TelegramReactionMetrics::COUNT_FONT_SIZE)
                    .font_weight(FontWeight::MEDIUM)
                    .monospaced_digits(true)
                    .color(self.foreground(is_mine, &theme)),
            );
        }
        let label = if self.reaction.is_custom_emoji() {
            self.reaction.count.to_string()
        } else {
            format!("{}, {}", self.reaction.emoji, self.reaction.count)
        };
        let on_tap = self.on_tap.clone();
        let tap_id = self.child_id("tap");
        row.pl(px(TelegramReactionMetrics::HORIZONTAL_PADDING))
            .frame_size(width, TelegramReactionMetrics::HEIGHT)
            .continuous_corners(TelegramReactionMetrics::CORNER_RADIUS)
            .bg(bg)
            .id(self.child_id("label"))
            .tooltip_label(label)
            .opacity(progress)
            .scale_effect(0.6 + 0.4 * progress)
            .bubble_tap_target(tap_id, move |_window, cx| on_tap(cx))
    }
}

#[derive(IntoElement)]
struct BuiltinReactionGlyph {
    emoji: String,
    size: f32,
    slot_size: f32,
}

impl RenderOnce for BuiltinReactionGlyph {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .frame_size(self.slot_size, self.slot_size)
            .child(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .text_size(px(self.size))
                    .line_height(px(self.slot_size))
                    .child(SharedString::from(self.emoji)),
            )
    }
}

#[derive(IntoElement)]
struct CustomEmojiReactionGlyph {
    id: ElementId,
    custom_emoji_id: String,
    size: f32,
}

impl RenderOnce for CustomEmojiReactionGlyph {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let stickers = StickersServicesStore::shared(cx);
        let item = stickers.read(cx).custom_emoji_item(&self.custom_emoji_id, cx).filter(|item| !item.file_url.is_empty());
        if let Some(item) = item {
            return InlineCustomEmojiRun::new(self.id, self.custom_emoji_id, item.file_url.clone(), item.asset_type.wire())
                .size(self.size)
                .canvas_width(item.width)
                .canvas_height(item.height)
                .maximum_width(Some(self.size))
                .into_any_element();
        }
        let warmup = CustomEmojiResourceWarmup::shared(cx);
        let revision = warmup.read(cx).resource_revision(cx);
        let custom_emoji_id = self.custom_emoji_id.clone();
        on_change(
            ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("warmup")),
            revision,
            true,
            window,
            cx,
            move |_, _, _, cx| {
                warmup.update(cx, |warmup, cx| warmup.ensure_custom_emoji(&custom_emoji_id, cx));
            },
        );
        ReactionEmojiPlaceholder::new(self.size).into_any_element()
    }
}

#[derive(IntoElement)]
struct ReactionAvatarStack {
    id: ElementId,
    reactors: Vec<ReactionReactor>,
    ring_color: Hsla,
}

impl RenderOnce for ReactionAvatarStack {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let avatar_size = TelegramReactionMetrics::AVATAR_SIZE;
        let advance = avatar_size - TelegramReactionMetrics::AVATAR_OVERLAP;
        let current_profile = AuthServicesStore::shared(cx).read(cx).profile().cloned();
        let placeholder_fill = theme_store(cx).read(cx).current_theme().text.secondary.opacity(0.18);
        let now = RiseClock::monotonic();
        let count = self.reactors.len();
        let width = if count == 0 { 0.0 } else { avatar_size + advance * (count - 1) as f32 };
        let mut stack = div().relative().frame_size(width, avatar_size);
        for (index, reactor) in self.reactors.into_iter().enumerate().rev() {
            let is_current_user = current_profile.as_ref().is_some_and(|profile| profile.id == reactor.user_id);
            let source = reactor.avatar_url.clone().or_else(|| {
                let profile = current_profile.as_ref().filter(|_| is_current_user)?;
                let raw = profile.avatar_url.as_deref().unwrap_or(profile.more.logo.as_str());
                RemoteMediaURL::optional_normalized(Some(raw))
            });
            let name = if reactor.name.is_empty() && is_current_user {
                current_profile.as_ref().map(|profile| profile.name.clone()).unwrap_or_default()
            } else {
                reactor.name.clone()
            };
            let is_identified = is_current_user || !reactor.name.is_empty() || reactor.avatar_url.is_some();
            let identified_target = if is_identified { 1.0_f32 } else { 0.0 };
            let fade = window.use_keyed_state(
                ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::from(format!("identified-{}", reactor.user_id))),
                cx,
                |_, _| Animated::new(identified_target),
            );
            fade.update(cx, |fade, cx| {
                if *fade.target() != identified_target {
                    fade.set_in(identified_target, ease_out(0.2), now, cx);
                }
            });
            let identified_opacity = fade.read(cx).frame_value(now, window);
            let placeholder = (identified_opacity < 1.0).then(|| {
                div().absolute().inset_0().circle().bg(placeholder_fill).opacity(1.0 - identified_opacity)
            });
            let logo = (identified_opacity > 0.0).then(|| {
                div().absolute().inset_0().opacity(identified_opacity).child(
                    UserLogoUi::new(ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::from(format!("avatar-{}", reactor.user_id))))
                        .source(source.map(SharedString::from))
                        .name(Some(SharedString::from(name)))
                        .user_id(Some(SharedString::from(reactor.user_id.clone())))
                        .size(avatar_size)
                        .show_border(false),
                )
            });
            stack = stack.child(
                div()
                    .absolute()
                    .top_0()
                    .left(px(advance * index as f32))
                    .frame_size(avatar_size, avatar_size)
                    .children(placeholder)
                    .children(logo)
                    .child(
                        div()
                            .absolute()
                            .top(px(-0.75))
                            .left(px(-0.75))
                            .frame_size(avatar_size + 1.5, avatar_size + 1.5)
                            .circle()
                            .stroke_border(self.ring_color, 1.5),
                    ),
            );
        }
        stack
    }
}

fn pixel_size(value: Size<f32>) -> Size<Pixels> {
    size(px(value.width), px(value.height))
}

fn proposed_width(known: Size<Option<Pixels>>, available: Size<AvailableSpace>) -> Option<f32> {
    known.width.map(f32::from).or(match available.width {
        AvailableSpace::Definite(width) => Some(f32::from(width)),
        AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
    })
}

const IDEAL_SPACE: Size<AvailableSpace> = Size { width: AvailableSpace::MaxContent, height: AvailableSpace::MaxContent };

fn measured_size(child: &mut AnyElement, window: &mut Window, cx: &mut App) -> Size<f32> {
    let measured = child.layout_as_root(IDEAL_SPACE, window, cx);
    size(f32::from(measured.width), f32::from(measured.height))
}

/// item of the reactions meta flow, index stands in for the layout subview
#[derive(Clone, Copy, Debug, PartialEq)]
struct ReactionsMetaFlowItem {
    index: usize,
    size: Size<f32>,
    is_meta: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct ReactionsMetaFlowLine {
    items: Vec<ReactionsMetaFlowItem>,
    width: f32,
    height: f32,
}

struct ReactionsMetaFlow {
    spacing: f32,
    line_spacing: f32,
    has_meta: bool,
    ideal_maximum_width: f32,
    is_outgoing: bool,
    children: Vec<AnyElement>,
    keys: Vec<SharedString>,
    leaving: Vec<(SharedString, AnyElement)>,
    motion: Rc<RefCell<ReactionsFlowMotion>>,
}

impl ReactionsMetaFlow {
    fn place(&mut self, index: usize, local_x: f32, local_y: f32, origin: Point<f32>, window: &mut Window, cx: &mut App) {
        let local = self.motion.borrow_mut().displayed(self.keys[index].clone(), point(local_x, local_y), window, cx);
        self.children[index].prepaint_at(point(px(origin.x + local.x), px(origin.y + local.y)), window, cx);
    }

    fn size_that_fits(
        proposal: Option<f32>,
        sizes: &[Size<f32>],
        spacing: f32,
        line_spacing: f32,
        has_meta: bool,
        ideal_maximum_width: f32,
        is_outgoing: bool,
    ) -> Size<f32> {
        if sizes.is_empty() {
            return size(0.0, 0.0);
        }
        let natural_width = sizes.iter().fold(0.0_f32, |result, item| {
            (if result > 0.0 { result + spacing } else { result }) + item.width
        });
        let maximum_width = proposal.unwrap_or(natural_width).min(ideal_maximum_width).max(1.0);
        if !has_meta {
            return TelegramReactionLayout::flow_plan(sizes, maximum_width, is_outgoing).size;
        }
        let lines = Self::layout(sizes, has_meta, spacing, maximum_width);
        let total_height = lines.iter().map(|line| line.height).sum::<f32>()
            + lines.len().saturating_sub(1) as f32 * line_spacing
            + TelegramReactionLayout::BOTTOM_INSET;
        let total_width = lines.iter().map(|line| line.width).fold(0.0_f32, f32::max);
        let filled_width = if proposal.is_some_and(f32::is_finite) { total_width.max(maximum_width) } else { total_width };
        size(filled_width.max(0.0), total_height)
    }

    fn row_width(items: &[ReactionsMetaFlowItem], spacing: f32) -> f32 {
        let widths: f32 = items.iter().map(|item| item.size.width).sum();
        widths + items.len().saturating_sub(1) as f32 * spacing
    }

    fn layout(sizes: &[Size<f32>], has_meta: bool, spacing: f32, maximum_width: f32) -> Vec<ReactionsMetaFlowLine> {
        if sizes.is_empty() {
            return Vec::new();
        }
        let meta_index = if has_meta { sizes.len() - 1 } else { sizes.len() };
        let mut lines = vec![ReactionsMetaFlowLine::default()];
        for (index, item_size) in sizes.iter().copied().enumerate().take(meta_index) {
            let cur_line = &lines[lines.len() - 1];
            let proposed = cur_line.width + if cur_line.items.is_empty() { 0.0 } else { spacing } + item_size.width;
            if proposed > maximum_width && !cur_line.items.is_empty() {
                lines.push(ReactionsMetaFlowLine::default());
            }
            let last = lines.len() - 1;
            let current = &mut lines[last];
            if !current.items.is_empty() {
                current.width += spacing;
            }
            current.width += item_size.width;
            current.height = current.height.max(item_size.height);
            current.items.push(ReactionsMetaFlowItem { index, size: item_size, is_meta: false });
        }
        if !has_meta {
            return lines;
        }
        let meta_size = sizes[meta_index];
        let meta_item = ReactionsMetaFlowItem { index: meta_index, size: meta_size, is_meta: true };
        let last_index = lines.len() - 1;
        let last = &mut lines[last_index];
        let needed = last.width + if last.items.is_empty() { 0.0 } else { spacing } + meta_size.width;
        if needed <= maximum_width {
            if !last.items.is_empty() {
                last.width += spacing;
            }
            last.width += meta_size.width;
            last.height = last.height.max(meta_size.height);
            last.items.push(meta_item);
        } else {
            let mut meta = ReactionsMetaFlowLine::default();
            if last.items.len() > 1
                && let Some(moved) = last.items.pop()
            {
                last.width = Self::row_width(&last.items, spacing);
                last.height = last.items.iter().map(|item| item.size.height).fold(0.0_f32, f32::max);
                meta.items.push(moved);
            }
            meta.items.push(meta_item);
            meta.width = Self::row_width(&meta.items, spacing);
            meta.height = meta.items.iter().map(|item| item.size.height).fold(0.0_f32, f32::max);
            lines.push(meta);
        }
        lines
    }
}

impl IntoElement for ReactionsMetaFlow {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ReactionsMetaFlow {
    type RequestLayoutState = Vec<Size<f32>>;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let sizes: Vec<Size<f32>> = self.children.iter_mut().map(|child| measured_size(child, window, cx)).collect();
        for (_, child) in &mut self.leaving {
            measured_size(child, window, cx);
        }
        let measure_sizes = sizes.clone();
        let (spacing, line_spacing, has_meta, ideal_maximum_width, is_outgoing) =
            (self.spacing, self.line_spacing, self.has_meta, self.ideal_maximum_width, self.is_outgoing);
        let layout_id = window.request_measured_layout(Style::default(), move |known, available, _window, _cx| {
            pixel_size(Self::size_that_fits(
                proposed_width(known, available),
                &measure_sizes,
                spacing,
                line_spacing,
                has_meta,
                ideal_maximum_width,
                is_outgoing,
            ))
        });
        (layout_id, sizes)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        sizes: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let maximum_width = f32::from(bounds.size.width);
        let (min_x, min_y) = (f32::from(bounds.left()), f32::from(bounds.top()));
        let origin = point(min_x, min_y);
        self.motion.borrow_mut().retain_keys(self.keys.iter().chain(self.leaving.iter().map(|(key, _)| key)));
        let motion = self.motion.clone();
        for (key, child) in &mut self.leaving {
            let local = motion.borrow().last_place(key, window);
            child.prepaint_at(point(px(min_x + local.x), px(min_y + local.y)), window, cx);
        }
        if !self.has_meta {
            let plan = TelegramReactionLayout::flow_plan(sizes, maximum_width, self.is_outgoing);
            for item in plan.items {
                self.place(item.index, item.frame.origin.x, item.frame.origin.y, origin, window, cx);
            }
            return;
        }
        let lines = Self::layout(sizes, self.has_meta, self.spacing, maximum_width);
        let mut y = 0.0_f32;
        let line_count = lines.len();
        for (line_index, line) in lines.iter().enumerate() {
            let is_last = line_index == line_count - 1;
            let mut x = 0.0_f32;
            for item in line.items.iter().filter(|item| !item.is_meta) {
                self.place(item.index, x, y + (line.height - item.size.height) / 2.0, origin, window, cx);
                x += item.size.width + self.spacing;
            }
            if is_last && let Some(meta) = line.items.iter().rev().find(|item| item.is_meta) {
                self.place(meta.index, maximum_width - meta.size.width, y + line.height - meta.size.height, origin, window, cx);
            }
            y += line.height + self.line_spacing;
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        for (_, child) in &mut self.leaving {
            child.paint(window, cx);
        }
        for child in &mut self.children {
            child.paint(window, cx);
        }
    }
}

#[derive(IntoElement)]
struct MessageFormattedTextWithMeta {
    id: ElementId,
    message: Rc<GetMessageMessage>,
    is_own: bool,
    is_channel: bool,
    style: FormattedTextStyle,
    role_spans: Vec<RoleMentionSpan>,
    options: FormattedTextOptions,
    prepared: Option<FormattedTextInlinePreparation>,
    text_selection: Option<FormattedTextSelection>,
    on_mention_tap: ChatMentionTap,
}

impl RenderOnce for MessageFormattedTextWithMeta {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let seed = FormattedTextRenderSeed::new(self.message.text.clone(), self.style, self.options, self.role_spans);
        let prepared = match self.prepared.filter(|prepared| prepared.geometry.cache_key == seed) {
            Some(prepared) => prepared,
            None => FormattedText::request_preparation(&seed).value_for_immediate_display(),
        };
        let has_tappable_runs =
            prepared.formatted.has_url || prepared.formatted.has_mention || !prepared.formatted.spoiler_ranges.is_empty();
        let inline_meta = MessageMeta::new((*self.message).clone()).is_own(self.is_own).is_channel(self.is_channel).style(MessageMetaStyle::Inline);
        if self.message.link_preview.is_some() || self.message.has_reaction_row() {
            return MessageTrailingMetaText {
                id: self.id,
                message: self.message,
                is_own: self.is_own,
                is_channel: self.is_channel,
                prepared,
                text_selection: self.text_selection,
                on_mention_tap: self.on_mention_tap,
            }
            .into_any_element();
        }
        TextWithInlineMeta {
            id: self.id,
            prepared,
            meta: inline_meta.into_any_element(),
            text_selection: self.text_selection,
            on_mention_tap: has_tappable_runs.then_some(self.on_mention_tap),
        }
        .into_any_element()
    }
}

#[derive(IntoElement)]
struct MessageTrailingMetaText {
    id: ElementId,
    message: Rc<GetMessageMessage>,
    is_own: bool,
    is_channel: bool,
    prepared: FormattedTextInlinePreparation,
    text_selection: Option<FormattedTextSelection>,
    on_mention_tap: ChatMentionTap,
}

impl RenderOnce for MessageTrailingMetaText {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let seed = self.prepared.geometry.cache_key.clone();
        let mut stack = div().flex().flex_col().items_start().gap(px(4.0)).child(
            ChatSpoilerFormattedText::new(self.id, seed.text, seed.style, self.on_mention_tap)
                .options(seed.options)
                .role_mentions(seed.role_mentions)
                .preformatted(Some(self.prepared.formatted.clone()))
                .text_selection(self.text_selection),
        );
        if self.message.link_preview.is_none() && !self.message.has_reaction_row() {
            stack = stack.child(
                hstack(0.0).w_full().child(spacer_min_w(0.0)).child(
                    MessageMeta::new((*self.message).clone()).is_own(self.is_own).is_channel(self.is_channel).style(MessageMetaStyle::Inline),
                ),
            );
        }
        stack
    }
}

#[derive(IntoElement)]
struct TextWithInlineMeta {
    id: ElementId,
    prepared: FormattedTextInlinePreparation,
    meta: AnyElement,
    text_selection: Option<FormattedTextSelection>,
    on_mention_tap: Option<ChatMentionTap>,
}

impl RenderOnce for TextWithInlineMeta {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let seed = self.prepared.geometry.cache_key.clone();
        let has_explicit_line_break = seed.text.contains(['\n', '\r', '\u{000B}', '\u{000C}', '\u{0085}', '\u{2028}', '\u{2029}']);
        let text = match self.on_mention_tap {
            Some(on_mention_tap) => ChatSpoilerFormattedText::new(self.id, seed.text, seed.style, on_mention_tap)
                .options(seed.options)
                .role_mentions(seed.role_mentions)
                .preformatted(Some(self.prepared.formatted.clone()))
                .text_selection(self.text_selection)
                .into_any_element(),
            None => FormattedTextUi::new(self.id, seed.text, seed.style)
                .options(seed.options)
                .shows_subtitle(false)
                .role_mentions(seed.role_mentions)
                .preformatted(Some(self.prepared.formatted.clone()))
                .text_selection(self.text_selection)
                .into_any_element(),
        };
        TextMetaFlow {
            spacing: MessageSizes::text_meta_flow_spacing(),
            geometry: self.prepared.geometry.clone(),
            has_explicit_line_break,
            text,
            meta: self.meta,
        }
    }
}

struct TextMetaFlow {
    spacing: f32,
    geometry: Arc<FormattedTextPreparedGeometry>,
    has_explicit_line_break: bool,
    text: AnyElement,
    meta: AnyElement,
}

/// the two intrinsic sizes the frame layout pass keeps
/// line measurements live in the bounded prepared text layout cache under the same geometry key
#[derive(Clone, Copy, Debug)]
struct TextMetaFlowCache {
    intrinsic_text_size: Size<f32>,
    intrinsic_meta_size: Size<f32>,
}

impl TextMetaFlow {
    const INLINE_SLACK: f32 = 3.0;

    fn constrained_text_height(geometry: &Arc<FormattedTextPreparedGeometry>, width: f32, one_line_height: f32) -> f32 {
        let total_height = TextLineMeasurer::measure(geometry, width).total_height;
        if total_height > 0.0 { total_height } else { one_line_height }
    }

    fn size_that_fits(
        proposal: Option<f32>,
        spacing: f32,
        geometry: &Arc<FormattedTextPreparedGeometry>,
        has_explicit_line_break: bool,
        cache: TextMetaFlowCache,
    ) -> Size<f32> {
        let meta = cache.intrinsic_meta_size;
        let one_line = cache.intrinsic_text_size;
        let natural_width = (one_line.width + spacing + meta.width).ceil() + Self::INLINE_SLACK;
        let constrained = proposal.filter(|width| width.is_finite() && *width < 100_000.0);
        let container_width = constrained.map_or_else(|| natural_width.max(1.0), |width| width.max(1.0));

        if !has_explicit_line_break && one_line.width + spacing + meta.width <= container_width {
            return size(container_width, one_line.height.max(meta.height));
        }

        let mut layout_width = container_width;
        if constrained.is_none() && has_explicit_line_break && one_line.width <= container_width {
            let natural_measurement = TextLineMeasurer::measure(geometry, container_width);
            let compact_width =
                one_line.width.max(natural_measurement.last_line_width + spacing + meta.width).ceil() + Self::INLINE_SLACK;
            layout_width = container_width.min(compact_width.max(1.0));
        }

        let text_height = Self::constrained_text_height(geometry, layout_width, one_line.height);
        let measured = TextLineMeasurer::measure(geometry, layout_width);
        let meta_fits_on_last_line = measured.last_line_width + spacing + meta.width <= layout_width;
        let height = if meta_fits_on_last_line { text_height } else { text_height + meta.height + 2.0 };
        size(layout_width, height)
    }
}

impl IntoElement for TextMetaFlow {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextMetaFlow {
    type RequestLayoutState = TextMetaFlowCache;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        ChatPreparedTextLayoutCache::install_text_system(cx);
        let cache = TextMetaFlowCache {
            intrinsic_text_size: measured_size(&mut self.text, window, cx),
            intrinsic_meta_size: measured_size(&mut self.meta, window, cx),
        };
        let spacing = self.spacing;
        let geometry = self.geometry.clone();
        let has_explicit_line_break = self.has_explicit_line_break;
        let layout_id = window.request_measured_layout(Style::default(), move |known, available, _window, _cx| {
            pixel_size(Self::size_that_fits(proposed_width(known, available), spacing, &geometry, has_explicit_line_break, cache))
        });
        (layout_id, cache)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        cache: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let container_width = f32::from(bounds.size.width);
        let (min_x, min_y, max_x) = (f32::from(bounds.left()), f32::from(bounds.top()), f32::from(bounds.right()));
        let meta = cache.intrinsic_meta_size;
        let one_line = cache.intrinsic_text_size;
        let definite_width = |width: f32| Size {
            width: AvailableSpace::Definite(px(width)),
            height: AvailableSpace::MaxContent,
        };

        if !self.has_explicit_line_break && one_line.width + self.spacing + meta.width <= container_width {
            let height = one_line.height.max(meta.height);
            self.text.layout_as_root(definite_width(one_line.width.max(container_width - self.spacing - meta.width)), window, cx);
            self.text.prepaint_at(point(px(min_x), px(min_y + (height - one_line.height) / 2.0)), window, cx);
            self.meta.prepaint_at(point(px(max_x - meta.width), px(min_y + height - meta.height - 1.0)), window, cx);
            return;
        }

        let text_size = self.text.layout_as_root(definite_width(container_width), window, cx);
        let text_height = f32::from(text_size.height);
        self.text.prepaint_at(point(px(min_x), px(min_y)), window, cx);
        let measured = TextLineMeasurer::measure(&self.geometry, container_width);
        let meta_fits_on_last_line = measured.last_line_width + self.spacing + meta.width <= container_width;
        let meta_y = if meta_fits_on_last_line { min_y + text_height - meta.height - 1.0 } else { min_y + text_height + 2.0 };
        self.meta.prepaint_at(point(px(max_x - meta.width), px(meta_y)), window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.text.paint(window, cx);
        self.meta.paint(window, cx);
    }
}

enum TextLineMeasurer {}

impl TextLineMeasurer {
    fn measure(geometry: &Arc<FormattedTextPreparedGeometry>, max_width: f32) -> ChatPreparedTextLayoutMeasurement {
        ChatPreparedTextLayoutCache::prepare(
            &ChatTextHitLayoutSeed::new(geometry.cache_key.clone(), max_width.max(1.0), None),
            Some(geometry.clone()),
        )
        .measurement
    }
}

#[derive(IntoElement)]
pub struct MediaSenderOverlay {
    name: String,
    color: Hsla,
}

impl MediaSenderOverlay {
    pub fn new(name: impl Into<String>, color: Hsla) -> Self {
        Self { name: name.into(), color }
    }
}

impl RenderOnce for MediaSenderOverlay {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div().flex().flex_row().items_start().p(px(8.0)).child(
            div()
                .relative()
                .flex_initial()
                .min_w_0()
                .capsule()
                .child(div().absolute().inset_0().capsule().bg(ultra_thin_material_fill(true)))
                .child(div().absolute().inset_0().capsule().bg(black().opacity(0.42)))
                .child(
                    div().relative().px(px(8.0)).py(px(3.0)).child(
                        MainText::new(self.name).px(12.0).font_weight(FontWeight::SEMIBOLD).color(self.color).number_of_lines(1),
                    ),
                ),
        )
    }
}

pub enum ChatColorPalette {}

impl ChatColorPalette {
    fn palette() -> [Hsla; 10] {
        [
            rgb(0x3F8EF7).into(),
            rgb(0x7E57C2).into(),
            rgb(0xEC407A).into(),
            rgb(0x26A69A).into(),
            rgb(0xFF7043).into(),
            rgb(0x5C6BC0).into(),
            rgb(0xFF8F00).into(),
            rgb(0x16A085).into(),
            rgb(0x9D27B0).into(),
            rgb(0x0288D1).into(),
        ]
    }

    pub fn color(sender_id: &str, is_own: bool, theme: &AppTheme) -> Hsla {
        if is_own {
            return theme.content_on(theme.primary._100);
        }
        if sender_id.is_empty() {
            return theme.primary._100;
        }
        let mut hash: u32 = 5381;
        for byte in sender_id.bytes() {
            hash = (hash << 5).wrapping_add(hash).wrapping_add(u32::from(byte));
        }
        let palette = Self::palette();
        palette[hash as usize % palette.len()]
    }
}

