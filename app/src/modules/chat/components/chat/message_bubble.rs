use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, BoxShadow, ElementId, Entity, FontWeight, Hsla, Pixels, Point, SharedString, Subscription,
    Window, black, canvas, linear_color_stop, linear_gradient, point,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::plural_word::plural_word;
use rise_i18n::tr;
use rise_platform::open_url::open_url;
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{
    HoldContextMenuAction, HoldContextMenuAlignment, HoldSourceBubbleClip,
};
use rise_ui::main_text::MainText;
use rise_ui::message_bubble_geometry::{MessageBubbleDescriptor, MessageBubbleShape, MessageBubbleTailSide};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::environment::LayoutDirection;
use rise_ui::swiftui::prelude::*;

use crate::app::router::in_app_link_browser::InAppLinkBrowser;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::ui::formatted_text_ui::FormattedTextInlinePreparation;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::{
    HoldContextMenuContainer, HoldContextMenuPointHandler,
};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierRetryOptions, NotifierSendOptions, NotifierType,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::shared::bot_ui_validation::BotMiniAppURLPolicy;
use crate::modules::bot::stores::bot::bot_actions::bot_types::{
    BotCallbackAnswerModel, BotCallbackEnqueueResult, BotMiniAppLaunchData, BotTagRules,
};
use crate::modules::bot::stores::bot::bot_interactions::bot_interactions::BotInteractionsStore;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::chat::components::chat::bubble::bot_message_inline_keyboard::BotMessageInlineKeyboard;
use crate::modules::chat::components::chat::bubble::bubble_corners::BubbleCorners;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::{BubbleTapRouter, BubbleTapRouterModifiers};
use crate::modules::chat::components::chat::bubble::message_content::{
    BubbleColumnLayout, ChatMessageViewportWidthKey, MessageContent, MessageContentAction, MessageContentKind,
    MessageContentStringAction, chat_message_viewport_width,
};
use crate::modules::chat::components::chat::bubble::message_sticker_view::MessageStickerFavourite;
use crate::modules::chat::components::chat::message_read_receipt_menu::MessageReadReceiptMenuAccessory;
use crate::modules::chat::shared::chat_search_highlight::{ChatSearchHighlightWordsKey, chat_search_highlight_words};
use crate::modules::chat::shared::config::message_sizes::{MessageAccessoryStackGeometry, MessageSizes};
use crate::modules::chat::shared::message_permissions::{ChatMessageAction, MessagePermissions};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageBotKeyboardButton, MessageContentType, MessageDeliveryStatus,
};
use crate::modules::music::widgets::bottomsheets::playlists::message_mp3_import_sheet::MessageMusicImportPresenter;

const COMMENTERS_SHOWN: usize = 3;

pub enum ChatMessageHoldPreviewPolicy {}

impl ChatMessageHoldPreviewPolicy {
    pub const RASTERIZED_TEXT_UTF16_THRESHOLD: usize = 256;

    pub fn prefers_rasterized_preview(kind: MessageContentKind, text: &str) -> bool {
        match kind {
            MessageContentKind::Text => {
                text.encode_utf16().count() >= Self::RASTERIZED_TEXT_UTF16_THRESHOLD || text.contains("```")
            }
            _ => false,
        }
    }
}

pub type MessageBubbleStringAction = Rc<dyn Fn(&str, &mut Window, &mut App)>;
pub type MessageBubbleAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub type MessageBubbleTapAction = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;
pub type MessageBubbleCanDoubleTap = Rc<dyn Fn() -> bool>;

#[derive(IntoElement, Clone)]
pub struct MessageBubble {
    id: ElementId,
    message: Arc<GetMessageMessage>,
    chat: Arc<ChatInfo>,
    is_own: bool,
    is_first_in_group: bool,
    is_last_in_group: bool,
    interactions_enabled: bool,
    allows_message_selection: bool,
    allows_composer_actions: bool,
    leading_context_actions: Vec<HoldContextMenuAction>,
    on_reaction: Option<MessageBubbleStringAction>,
    on_action: Option<MessageBubbleStringAction>,
    on_tap: Option<MessageBubbleTapAction>,
    on_open_sender_profile: Option<MessageBubbleAction>,
    can_double_tap: MessageBubbleCanDoubleTap,
    on_double_tap: Option<MessageBubbleAction>,
    prepared_text: Option<FormattedTextInlinePreparation>,
}

impl MessageBubble {
    pub fn new(
        id: impl Into<ElementId>,
        message: impl Into<Arc<GetMessageMessage>>,
        chat: impl Into<Arc<ChatInfo>>,
    ) -> Self {
        Self {
            id: id.into(),
            message: message.into(),
            chat: chat.into(),
            is_own: false,
            is_first_in_group: true,
            is_last_in_group: true,
            interactions_enabled: true,
            allows_message_selection: false,
            allows_composer_actions: true,
            leading_context_actions: Vec::new(),
            on_reaction: None,
            on_action: None,
            on_tap: None,
            on_open_sender_profile: None,
            can_double_tap: Rc::new(|| true),
            on_double_tap: None,
            prepared_text: None,
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

    pub fn is_last_in_group(mut self, is_last_in_group: bool) -> Self {
        self.is_last_in_group = is_last_in_group;
        self
    }

    pub fn interactions_enabled(mut self, interactions_enabled: bool) -> Self {
        self.interactions_enabled = interactions_enabled;
        self
    }

    pub fn allows_message_selection(mut self, allows_message_selection: bool) -> Self {
        self.allows_message_selection = allows_message_selection;
        self
    }

    pub fn allows_composer_actions(mut self, allows_composer_actions: bool) -> Self {
        self.allows_composer_actions = allows_composer_actions;
        self
    }

    pub fn leading_context_actions(mut self, leading_context_actions: Vec<HoldContextMenuAction>) -> Self {
        self.leading_context_actions = leading_context_actions;
        self
    }

    pub fn on_reaction(mut self, on_reaction: Option<MessageBubbleStringAction>) -> Self {
        self.on_reaction = on_reaction;
        self
    }

    pub fn on_action(mut self, on_action: Option<MessageBubbleStringAction>) -> Self {
        self.on_action = on_action;
        self
    }

    pub fn on_tap(mut self, on_tap: Option<MessageBubbleTapAction>) -> Self {
        self.on_tap = on_tap;
        self
    }

    pub fn on_open_sender_profile(mut self, on_open_sender_profile: Option<MessageBubbleAction>) -> Self {
        self.on_open_sender_profile = on_open_sender_profile;
        self
    }

    pub fn can_double_tap(mut self, can_double_tap: MessageBubbleCanDoubleTap) -> Self {
        self.can_double_tap = can_double_tap;
        self
    }

    pub fn on_double_tap(mut self, on_double_tap: Option<MessageBubbleAction>) -> Self {
        self.on_double_tap = on_double_tap;
        self
    }

    pub fn prepared_text(mut self, prepared_text: Option<FormattedTextInlinePreparation>) -> Self {
        self.prepared_text = prepared_text;
        self
    }

    pub fn bubble_max_width() -> f32 {
        MessageSizes::bubble_max_width()
    }

    fn child_id(&self, name: &str) -> ElementId {
        ElementId::Name(SharedString::from(format!("{}-{name}", self.id)))
    }

    fn shows_comments_footer(&self) -> bool {
        self.chat.r#type == ChatType::Channel
            && !self.message.is_system()
            && self.message.discussion_chat_id.as_deref().is_some_and(|id| !id.is_empty())
    }

    fn comments_footer(&self, nav_store: Option<Entity<NavigationStore>>, cx: &App) -> AnyElement {
        let theme = rise_ui::theme(cx);
        let (separator, primary, backdrop, secondary) =
            (theme.border._200.opacity(0.5), theme.primary._100, theme.bg._300, theme.text.secondary.opacity(0.7));
        let count = self.message.comments_count.unwrap_or(0);
        let title = if count > 0 {
            let (one, few, many) = (tr("chat_thread_title_one"), tr("chat_thread_title_few"), tr("chat_thread_title_many"));
            plural_word(count, &one, &few, &many).replace("{count}", &count.to_string())
        } else {
            tr("chat_post_comment_cta")
        };
        let commenters = self.message.recent_commenters.iter().take(COMMENTERS_SHOWN).enumerate().map(|(index, commenter)| {
            div()
                .relative()
                .when(index > 0, |this| this.ml(px(-8.0)))
                .child(
                    UserLogoUi::new(self.child_id(&format!("commenter-{index}")))
                        .source((!commenter.logo.is_empty()).then(|| SharedString::from(commenter.logo.clone())))
                        .name(Some(SharedString::from(commenter.name.clone())))
                        .size(24.0)
                        .show_story_ring(false),
                )
                .child(centred_stroke(backdrop, 1.5, Shape::Circle))
        });
        let leading = if self.message.recent_commenters.is_empty() {
            div().children(IconUi::sized("bubble.left.and.bubble.right", px(14.0), primary, false)).into_any_element()
        } else {
            div().flex().flex_row().children(commenters).into_any_element()
        };
        let discussion_chat_id = self.message.discussion_chat_id.clone();
        let discussion_message_id = self.message.discussion_message_id.clone();
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(div().w_full().h(px(0.5)).bg(separator))
            .child(
                div()
                    .w_full()
                    .min_h(px(42.0))
                    .px(px(12.0))
                    .py(px(9.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(leading)
                    .child(MainText::new(title).px(14.0).font_weight(FontWeight::MEDIUM).color(primary))
                    .child(spacer_min_w(8.0))
                    .children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(12.0), secondary, false)),
            )
            .bubble_tap_target_priority(self.child_id("comments-footer"), move |_, cx| {
                let Some(group_id) = discussion_chat_id.clone().filter(|id| !id.is_empty()) else {
                    return;
                };
                let Some(nav_store) = &nav_store else {
                    return;
                };
                let route = match discussion_message_id.clone().filter(|id| !id.is_empty()) {
                    Some(root_id) => RootRoute::ChatThread { chat_id: Some(group_id), root_id: Some(root_id) },
                    None => RootRoute::Chat { chat_id: Some(group_id) },
                };
                nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
            })
            .into_any_element()
    }

    fn content(
        &self,
        kind: MessageContentKind,
        corners: BubbleCorners,
        nav_store: Option<Entity<NavigationStore>>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let on_reaction = Self::content_string_action(self.on_reaction.clone(), window);
        let on_action = Self::content_string_action(self.on_action.clone(), window);
        let on_open_sender_profile: Option<MessageContentAction> = self.on_open_sender_profile.clone();
        let content = MessageContent::new((*self.message).clone(), (*self.chat).clone())
            .is_own(self.is_own)
            .is_first_in_group(self.is_first_in_group)
            .bubble_corners(corners)
            .hide_name(matches!(kind, MessageContentKind::Sticker))
            .on_reaction(on_reaction)
            .on_message_action(on_action)
            .on_open_sender_profile(on_open_sender_profile)
            .prepared_text(self.prepared_text.clone())
            .can_double_tap(Some(self.can_double_tap.clone()));
        if matches!(kind, MessageContentKind::Sticker | MessageContentKind::VideoNote) {
            return content.into_any_element();
        }

        let shows_comments_footer = self.shows_comments_footer();
        let shape = corners.bubble_shape();
        let theme = rise_ui::theme(cx);
        let is_own = self.is_own;
        let (fill_start, fill_end) = if is_own {
            (theme.primary._100.opacity(0.96), theme.primary._200.opacity(0.92))
        } else {
            let incoming = theme.bg._300.opacity(0.96);
            (incoming, incoming)
        };
        let incoming_stroke = theme.border._200.opacity(0.45);
        let radii = corners.uneven_shape(LayoutDirection::current());
        let tail_inset = px(MessageSizes::bubble_tail_inset());
        let footer = shows_comments_footer.then(|| self.comments_footer(nav_store, cx));
        div()
            .relative()
            .min_w(px(MessageSizes::bubble_minimum_width()))
            .min_h(px(MessageSizes::min_message_row_height()))
            .rounded_tl(radii.top_left)
            .rounded_tr(radii.top_right)
            .rounded_bl(radii.bottom_left)
            .rounded_br(radii.bottom_right)
            .shadow(vec![BoxShadow {
                color: black().opacity(if is_own { 0.07 } else { 0.04 }),
                offset: point(px(0.0), px(1.0)),
                blur_radius: px(1.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .child(Self::bubble_background(shape, is_own, fill_start, fill_end))
            .child(
                div()
                    .relative()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .when(corners.tail_side == MessageBubbleTailSide::Leading, |this| this.pl(tail_inset))
                    .when(corners.tail_side == MessageBubbleTailSide::Trailing, |this| this.pr(tail_inset))
                    .child(div().when(shows_comments_footer, |this| this.w_full()).child(content))
                    .children(footer),
            )
            .when(!is_own, |this| this.child(Self::bubble_stroke(shape, incoming_stroke)))
            .into_any_element()
    }

    fn bubble_background(shape: MessageBubbleShape, is_own: bool, start: Hsla, end: Hsla) -> impl IntoElement {
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                if is_own {
                    let angle = f32::from(bounds.size.width)
                        .atan2(-f32::from(bounds.size.height))
                        .to_degrees()
                        .rem_euclid(360.0);
                    shape.paint(
                        bounds,
                        linear_gradient(angle, linear_color_stop(start, 0.0), linear_color_stop(end, 1.0)),
                        window,
                    );
                } else {
                    shape.paint(bounds, start, window);
                }
            },
        )
        .absolute()
        .inset_0()
    }

    fn bubble_stroke(shape: MessageBubbleShape, color: Hsla) -> impl IntoElement {
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let path = shape.path(bounds);
                if path.is_empty() {
                    return;
                }
                let mut builder = gpui::PathBuilder::stroke(px(0.5));
                path.replay_into(&mut builder);
                if let Ok(stroke) = builder.build() {
                    window.paint_path(stroke, color);
                }
            },
        )
        .absolute()
        .inset_0()
    }

    /// message content reports without a window but callers want one
    /// window is handed out mid update so the call waits for the end of the current effect cycle
    fn content_string_action(
        action: Option<MessageBubbleStringAction>,
        window: &Window,
    ) -> Option<MessageContentStringAction> {
        let action = action?;
        let handle = window.window_handle();
        Some(Rc::new(move |value: &str, cx: &mut App| {
            let action = action.clone();
            let value = value.to_owned();
            cx.defer(move |cx| {
                handle.update(cx, |_, window, cx| action(&value, window, cx)).ok();
            });
        }))
    }

    fn hold_source_bubble_clip(kind: MessageContentKind, corners: &BubbleCorners) -> Option<HoldSourceBubbleClip> {
        if matches!(kind, MessageContentKind::Sticker | MessageContentKind::VideoNote) {
            return None;
        }
        Some(HoldSourceBubbleClip::with_tail(
            corners.top_leading,
            corners.top_trailing,
            corners.bottom_leading,
            corners.bottom_trailing,
            corners.tail_side,
            corners.draws_tail,
        ))
    }

    fn bubble_corners(&self) -> BubbleCorners {
        let descriptor = MessageBubbleDescriptor::grouped(
            self.is_own,
            self.is_first_in_group,
            self.is_last_in_group,
            px(MessageSizes::bubble_corner_radius()),
            px(MessageSizes::bubble_stacked_corner_radius()),
        );
        BubbleCorners::new(
            descriptor.radii.top_leading,
            descriptor.radii.top_trailing,
            descriptor.radii.bottom_leading,
            descriptor.radii.bottom_trailing,
            descriptor.tail_side,
            descriptor.draws_tail,
        )
    }

    fn context_menu_actions(&self, cx: &mut App) -> Vec<HoldContextMenuAction> {
        let mut actions = MessagePermissions::new(&self.message, &self.chat, self.is_own).context_menu_actions(
            self.allows_message_selection,
            self.allows_composer_actions,
            cx,
        );
        actions.splice(0..0, self.leading_context_actions.iter().cloned());
        actions.extend(MessageStickerFavourite::menu_actions(&self.message, &self.chat, cx));
        actions
    }

    fn has_inline_keyboard(&self) -> bool {
        self.message.reply_markup.as_ref().is_some_and(|markup| markup.has_inline_keyboard())
    }

    fn pending_button_key(state: &Entity<MessageBubbleCallbackState>, cx: &mut App) -> Option<String> {
        let (scope, key) = {
            let state = state.read(cx);
            (state.active_callback_scope.clone()?, state.active_callback_button_key.clone())
        };
        let services = BotServicesStore::shared(cx);
        services.read(cx).pending_callback_scopes.contains(&scope).then_some(key).flatten()
    }

    fn bubble_guide_anchor(&self, kind: MessageContentKind) -> Option<RiseGuideAnchor> {
        match kind {
            MessageContentKind::Audio => Some(RiseGuideAnchor::ChatAudioMessage),
            MessageContentKind::MediaGrid if self.is_own => Some(RiseGuideAnchor::ChatOwnPhotoMessage),
            _ => None,
        }
    }

    fn handle_bot_button(
        context: &MessageBubbleBotContext,
        button: &MessageBotKeyboardButton,
        key: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(callback_data) = &button.callback_data {
            Self::enqueue_callback(context, key, callback_data, cx);
            return;
        }
        if let Some(web_app_url) = &button.web_app_url {
            Self::open_mini_app(context.clone(), button.clone(), web_app_url.clone(), cx);
            return;
        }
        if let Some(url) = button.url.as_deref().and_then(Self::secure_url) {
            BotLinkBrowser::open(&url, cx);
            return;
        }
        if let (Some(query), Some(on_action)) = (&button.switch_inline_query_current_chat, &context.on_action) {
            on_action(&ChatMessageAction::bot_inline_query_action(query), window, cx);
            return;
        }
        Notifier::send(
            tr("bot_button_not_supported"),
            NotifierSendOptions {
                r#type: NotifierType::Warning,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                ..Default::default()
            },
            cx,
        );
    }

    fn enqueue_callback(context: &MessageBubbleBotContext, key: &str, data: &str, cx: &mut App) {
        let message = &context.message;
        let bot_identifier =
            trimming_whitespaces_and_newlines(message.via_bot.as_deref().unwrap_or(&message.sender_id)).to_owned();
        if bot_identifier.is_empty() {
            return;
        }
        let server_id = message.sai_server_id();
        let scope = format!("{}:{}:{}:{}", message.chat_id, server_id, key, Uuid::new_v4().uuid_string());
        context.state.update(cx, |state, cx| state.begin(scope.clone(), key.to_owned(), cx));
        let services = BotServicesStore::shared(cx);
        services.update(cx, |services, cx| services.begin_callback(&scope, cx));

        let (result_services, error_services) = (services.clone(), services);
        let (result_scope, error_scope) = (scope.clone(), scope);
        BotInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.enqueue_callback_query(
                &bot_identifier,
                "",
                Some(message.chat_id.as_str()),
                Some(server_id.as_str()),
                None,
                Some(data),
                None,
                Some(Rc::new(move |result: BotCallbackEnqueueResult, cx: &mut App| {
                    result_services.update(cx, |services, cx| {
                        services.bind_callback(&result_scope, &result.callback_query_id, cx);
                    });
                })),
                Some(Rc::new(move |_: &anyhow::Error, cx: &mut App| {
                    error_services.update(cx, |services, cx| services.fail_callback(&error_scope, cx));
                })),
                cx,
            );
        });
    }

    fn open_mini_app(
        context: MessageBubbleBotContext,
        button: MessageBotKeyboardButton,
        fallback_url: String,
        cx: &mut App,
    ) {
        let owner_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        let participant = &context.chat.participant;
        let tag = BotTagRules::sanitize(&participant.tag, false, false);
        let owned_bot = BotServicesStore::bot_by_tag(&tag, cx).is_some_and(|bot| bot.is_owned_by_requester);
        let allows_development_http =
            owned_bot || (!owner_id.is_empty() && participant.bot_owner_id.as_deref() == Some(owner_id.as_str()));
        let Some(fallback) = Url::from_string(&fallback_url)
            .filter(|url| BotMiniAppURLPolicy::allows(url, allows_development_http))
        else {
            return;
        };
        let interactions = BotInteractionsStore::shared(cx);
        let title = BotInteractionsStore::mini_app_title(
            button.bot_id.as_deref(),
            button.mini_app_id.as_deref(),
            None,
            Some(tag.as_str()),
            Some(participant.id.as_str()),
            cx,
        )
        .unwrap_or_else(|| button.text.clone());
        let chat_id = context.message.chat_id.clone();
        let launch_ids = button
            .bot_id
            .clone()
            .filter(|id| !id.is_empty())
            .zip(button.mini_app_id.clone().filter(|id| !id.is_empty()));
        let Some((bot_id, mini_app_id)) = launch_ids else {
            context.push_mini_app(fallback, title, allows_development_http, chat_id, button.bot_id.clone(), cx);
            return;
        };
        let result_context = context.clone();
        let result_chat_id = chat_id.clone();
        let result_bot_id = bot_id.clone();
        interactions.update(cx, |interactions, cx| {
            interactions.get_mini_app_launch(
                &bot_id,
                &mini_app_id,
                Some(chat_id.as_str()),
                None,
                allows_development_http,
                None,
                Some(Rc::new(move |launch: BotMiniAppLaunchData, cx: &mut App| {
                    let Some(url) = Url::from_string(&launch.launch_url) else {
                        return;
                    };
                    result_context.push_mini_app(
                        url,
                        title.clone(),
                        allows_development_http,
                        result_chat_id.clone(),
                        Some(result_bot_id.clone()),
                        cx,
                    );
                })),
                Some(Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                    let content = BotInteractionsStore::shared(cx).read(cx).mini_app_launch_failure_text(error);
                    let (context, button, fallback_url) = (context.clone(), button.clone(), fallback_url.clone());
                    Notifier::send_retry(
                        tr("bot_mini_app_open_failed"),
                        NotifierRetryOptions { content: Some(content), ..Default::default() },
                        |_| {},
                        move |cx| Self::open_mini_app(context, button, fallback_url, cx),
                        cx,
                    );
                })),
                cx,
            );
        });
    }

    fn secure_url(raw: &str) -> Option<Url> {
        Url::from_string(raw).filter(|url| BotMiniAppURLPolicy::allows(url, false))
    }
}

/// what the bot flows need after the element has been drawn
#[derive(Clone)]
struct MessageBubbleBotContext {
    message: Arc<GetMessageMessage>,
    chat: Arc<ChatInfo>,
    nav_store: Option<Entity<NavigationStore>>,
    state: Entity<MessageBubbleCallbackState>,
    on_action: Option<MessageBubbleStringAction>,
}

impl MessageBubbleBotContext {
    fn push_mini_app(
        &self,
        url: Url,
        title: String,
        allows_development_http: bool,
        chat_id: String,
        bot_id: Option<String>,
        cx: &mut App,
    ) {
        let Some(nav_store) = &self.nav_store else {
            return;
        };
        let route = RootRoute::BotMiniApp {
            url,
            title,
            allows_development_http,
            chat_id: Some(chat_id),
            bot_id,
            is_web_link: false,
        };
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }
}

/// state of the active callback scope and button key
/// the alert itself is the app wide confirm alert so no field for it here
struct MessageBubbleCallbackState {
    active_callback_scope: Option<String>,
    active_callback_button_key: Option<String>,
    callback_answer_observer: Option<Subscription>,
}

impl MessageBubbleCallbackState {
    fn begin(&mut self, scope: String, key: String, cx: &mut gpui::Context<Self>) {
        self.active_callback_scope = Some(scope);
        self.active_callback_button_key = Some(key);
        let services = BotServicesStore::shared(cx);
        self.callback_answer_observer = Some(cx.observe(&services, |state, services, cx| {
            state.bot_services_changed(services, cx);
        }));
        cx.notify();
    }

    /// a slot reused for another message must not carry a pending callback
    /// the scope keys the highlighted inline button and the url the answer opens
    /// none of it derives from the message
    fn reset(&mut self, cx: &mut gpui::Context<Self>) {
        self.active_callback_scope = None;
        self.active_callback_button_key = None;
        self.callback_answer_observer = None;
        cx.notify();
    }

    fn bot_services_changed(&mut self, services: Entity<BotServicesStore>, cx: &mut gpui::Context<Self>) {
        let Some(scope) = self.active_callback_scope.clone() else {
            return;
        };
        match services.read(cx).callback_answer(&scope) {
            Some(answer) => self.handle_callback_answer(&scope, answer, &services, cx),
            None => cx.notify(),
        }
    }

    fn handle_callback_answer(
        &mut self,
        scope: &str,
        answer: BotCallbackAnswerModel,
        services: &Entity<BotServicesStore>,
        cx: &mut gpui::Context<Self>,
    ) {
        services.update(cx, |services, cx| services.consume_callback_answer(scope, cx));
        if let Some(url) = MessageBubble::secure_url(&answer.url) {
            BotLinkBrowser::open(&url, cx);
        }
        if !answer.text.is_empty() {
            if answer.show_alert {
                let title = if answer.text.is_empty() { tr("bot_callback_answer") } else { answer.text.clone() };
                NativeConfirmAlert::present_notice(&title, "", None, None, |_| {}, cx);
            } else {
                Notifier::send(
                    answer.text.clone(),
                    NotifierSendOptions {
                        r#type: NotifierType::Info,
                        position: NotifierPosition::Bottom,
                        icon: NotifierIcon::System("checkmark.circle.fill".to_owned()),
                        ..Default::default()
                    },
                    cx,
                );
            }
        }
        self.reset(cx);
    }
}

impl RenderOnce for MessageBubble {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let kind = MessageContent::bubble_kind(&self.message, Some(self.chat.as_ref()));
        let corners = self.bubble_corners();
        let is_own = self.is_own;
        let frame_max_width =
            MessageSizes::available_bubble_width(chat_message_viewport_width(cx), &self.chat, is_own);
        let expands_to_proposal = self.message.link_preview.is_some() && kind.needs_bubble_padding();
        let reaction_menu =
            ChatServicesStore::shared(cx).read(cx).reaction_config(&self.chat.id, &self.chat.allowed_reactions);
        let offers_hold_menu = self.interactions_enabled && !self.message.is_disabled_by_moderation();
        let is_community_preview = self.chat.is_community_preview();
        let is_call = self.message.content_type == MessageContentType::Call;
        let show_emoji_list = self.interactions_enabled
            && !is_community_preview
            && !is_call
            && !self.message.is_pending_outgoing()
            && self.message.delivery_status != MessageDeliveryStatus::Failed
            && reaction_menu.enabled
            && MessagePermissions::new(&self.message, &self.chat, is_own).can_add_reactions(cx);
        let accessory = (offers_hold_menu && !is_community_preview && !is_call)
            .then(|| MessageReadReceiptMenuAccessory::configuration(&self.message, &self.chat, is_own, cx))
            .flatten();
        let actions = Rc::new(if offers_hold_menu { self.context_menu_actions(cx) } else { Vec::new() });
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(self.child_id("callback"), cx, |_, _| MessageBubbleCallbackState {
            active_callback_scope: None,
            active_callback_button_key: None,
            callback_answer_observer: None,
        });

        let reset_state = state.clone();
        on_change(
            self.child_id("server-id"),
            self.message.sai_server_id(),
            false,
            window,
            cx,
            move |_, _, _, cx| reset_state.update(cx, |state, cx| state.reset(cx)),
        );

        let on_emoji = {
            let on_reaction = self.on_reaction.clone();
            Rc::new(move |emoji: String, window: &mut Window, cx: &mut App| {
                if is_community_preview || is_call {
                    return;
                }
                if let Some(on_reaction) = &on_reaction {
                    on_reaction(&emoji, window, cx);
                }
            })
        };
        let on_action = {
            let (message, chat, on_action) = (self.message.clone(), self.chat.clone(), self.on_action.clone());
            Rc::new(move |id: String, window: &mut Window, cx: &mut App| {
                if id == MessageStickerFavourite::FAVOURITE_ACTION_ID {
                    MessageStickerFavourite::handle(&id, &message, &chat, cx);
                    return;
                }
                if id == ChatMessageAction::ADD_TO_LOCAL_PLAYLIST {
                    MessageMusicImportPresenter::shared(cx).update(cx, |presenter, cx| presenter.present(&message, cx));
                    return;
                }
                if let Some(on_action) = &on_action {
                    on_action(&id, window, cx);
                }
            })
        };
        let on_tap = {
            let (message, on_action, on_tap) = (self.message.clone(), self.on_action.clone(), self.on_tap.clone());
            let interactions_enabled = self.interactions_enabled;
            let nav_store = nav_store.clone();
            Rc::new(move |location: Point<Pixels>, window: &mut Window, cx: &mut App| {
                let is_sender =
                    is_own || AuthServicesStore::shared(cx).read(cx).is_current_user(&message.sender_id);
                if is_sender
                    && interactions_enabled
                    && !is_community_preview
                    && message.delivery_status == MessageDeliveryStatus::Failed
                {
                    if let Some(on_action) = &on_action {
                        on_action(ChatMessageAction::FAILED_SEND_OPTIONS, window, cx);
                    }
                    return;
                }
                let shared_post = message.shared_post.as_ref().filter(|shared| !shared.post_id.is_empty());
                if let (MessageContentKind::PostShort, Some(shared)) = (kind, shared_post) {
                    if BubbleTapRouter::dispatch(location, None, window, cx) {
                        return;
                    }
                    if let Some(nav_store) = &nav_store {
                        let route = shared.open_route();
                        nav_store.update(cx, |nav_store, cx| {
                            nav_store.push(route, NavigationStoreSettlingPush::Drop, cx);
                        });
                    }
                    return;
                }
                BubbleTapRouter::dispatch(location, None, window, cx);
                if let Some(on_tap) = &on_tap {
                    on_tap(location, window, cx);
                }
            })
        };
        let can_double_tap = {
            let can_double_tap = self.can_double_tap.clone();
            Rc::new(move |_: &App| !is_community_preview && !is_call && can_double_tap())
        };
        let on_double_tap = self.on_double_tap.clone().map(|action| -> HoldContextMenuPointHandler {
            Rc::new(move |_: Point<Pixels>, window: &mut Window, cx: &mut App| action(window, cx))
        });

        let source = {
            let this = self.clone();
            let nav_store = nav_store.clone();
            let viewport_width = chat_message_viewport_width(cx);
            let highlight_words = chat_search_highlight_words(cx);
            Rc::new(move |window: &mut Window, cx: &mut App| {
                env_scope::<ChatMessageViewportWidthKey>(viewport_width)
                    .child(env_scope::<ChatSearchHighlightWordsKey>(highlight_words.clone()).child(
                        this.content(kind, corners, nav_store.clone(), window, cx)
                            .guide_anchor(this.bubble_guide_anchor(kind))
                            .into_any_element(),
                    ))
                    .into_any_element()
            })
        };
        let container = HoldContextMenuContainer::new(
            self.child_id("hold"),
            if is_own { HoldContextMenuAlignment::Trailing } else { HoldContextMenuAlignment::Leading },
            Rc::new(move |_: &App| (*actions).clone()),
            source,
        )
        .show_emoji_list(show_emoji_list)
        .emojis(reaction_menu.quick_bar_emojis())
        .allows_custom_emoji(reaction_menu.allows_custom_emoji)
        .expanded_emojis(reaction_menu.restricted_emojis.clone())
        .source_bubble_clip(Self::hold_source_bubble_clip(kind, &corners))
        .accessory(accessory)
        .capture_snapshot(ChatMessageHoldPreviewPolicy::prefers_rasterized_preview(kind, &self.message.text))
        .lifts_live_source(true)
        .disabled(!self.interactions_enabled)
        .source_identity(Some(self.message.id.clone()))
        .on_emoji(on_emoji)
        .on_action(on_action)
        .on_tap(on_tap)
        .can_double_tap(can_double_tap)
        .on_double_tap(on_double_tap)
        .context_click_across_row(true);

        let inline_keyboard = self.has_inline_keyboard().then(|| {
            let allows_hit_testing = self.interactions_enabled && !is_community_preview;
            let context = MessageBubbleBotContext {
                message: self.message.clone(),
                chat: self.chat.clone(),
                nav_store: nav_store.clone(),
                state: state.clone(),
                on_action: self.on_action.clone(),
            };
            let rows = self.message.reply_markup.as_ref().map(|markup| markup.inline_keyboard.clone()).unwrap_or_default();
            let pending_key = Self::pending_button_key(&state, cx);
            let tail_inset = !matches!(kind, MessageContentKind::Sticker | MessageContentKind::VideoNote);
            let inset = px(MessageSizes::bubble_tail_inset());
            div()
                .w_full()
                .flex()
                .flex_col()
                .when(is_own, |this| this.items_end())
                .when(!is_own, |this| this.items_start())
                .pt(px(MessageAccessoryStackGeometry::INLINE_KEYBOARD_SPACING))
                .pb(px(MessageAccessoryStackGeometry::INLINE_KEYBOARD_BOTTOM_RESERVE))
                .when(tail_inset && !is_own, |this| this.pl(inset))
                .when(tail_inset && is_own, |this| this.pr(inset))
                .child(
                    BotMessageInlineKeyboard::new(
                        self.child_id("inline-keyboard"),
                        rows,
                        is_own,
                        move |button: &MessageBotKeyboardButton, key: &str, window: &mut Window, cx: &mut App| {
                            if allows_hit_testing {
                                Self::handle_bot_button(&context, button, key, window, cx);
                            }
                        },
                    )
                    .pending_key(pending_key),
                )
        });

        div().w_full().max_w(px(frame_max_width)).child(
            BubbleColumnLayout::new(0.0)
                .expands_to_proposal(true)
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .when(is_own, |this| this.items_end())
                        .when(!is_own, |this| this.items_start())
                        .child(div().max_w_full().when(expands_to_proposal, |this| this.w_full()).child(container)),
                )
                .children(inline_keyboard),
        )
    }
}

/// a bot link button opens inside riseonly in the same sheet as every outside link
/// its menu takes the page to the browser
enum BotLinkBrowser {}

impl BotLinkBrowser {
    fn open(url: &Url, cx: &mut App) {
        if InAppLinkBrowser::open(url, cx) {
            return;
        }
        open_url(url, cx);
    }
}
