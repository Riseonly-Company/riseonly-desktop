use std::rc::Rc;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use gpui::prelude::*;
use gpui::{AnyElement, App, AppContext, Context, WeakEntity, Window};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_range::NSRange;
use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::stores::bot::bot_actions::bot_actions::BotActionsStore;
use crate::modules::bot::stores::bot::bot_interactions::bot_interactions::BotInteractionsStore;
use crate::modules::bot::stores::bot::bot_actions::bot_inline_types::BotInlineIdentityModel;
use crate::modules::bot::stores::bot::bot_actions::bot_types::{BotCommandModel, BotMiniAppModel, BotModel, BotTagRules};
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::chat::components::chat::bot_chat_menu_sheet::BotChatMenuPresentationContract;
use crate::modules::chat::components::chat::composer::chat_composer_inline_context::ChatComposerInlineContext;
use crate::modules::chat::components::chat::composer::chat_composer_types::resign_chat_keyboard;
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageBotKeyboardButton};
use crate::modules::chat::stores::message::message_services::message_services::MessageScopePresentationSnapshot;
use rise_core::foundation::foundation_url::Url;
use crate::core::widgets::notifier::notifier_store::NotifierRetryOptions;
use crate::modules::bot::shared::bot_ui_validation::BotMiniAppURLPolicy;
use crate::modules::bot::stores::bot::bot_actions::bot_actions::{BotActionError, BotActionResult};
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotMiniAppLaunchData;
use std::time::Duration;
use gpui::AnyWindowHandle;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::chat::components::chat::bot_chat_menu_sheet::BotChatStartCard;
use super::super::chat_screen::*;
use super::super::chat_screen_support::*;

impl ChatScreen {
    pub(in super::super) fn inline_composer_bot_identity(&self, cx: &mut App) -> Option<BotInlineIdentityModel> {
        let ChatComposerInlineContext::Bot { bot_tag, .. } = &self.inline_composer_context else {
            return None;
        };
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        let language_code = self.inline_composer_language_code();
        if let Some(cached) = BotServicesStore::inline_commands(bot_tag, &chat_id, topic_id.as_deref(), &language_code, cx)
            .and_then(|commands| commands.bot)
        {
            return Some(cached);
        }
        self.inline_selected_bot_identity
            .as_ref()
            .filter(|identity| identity.tag.to_lowercase() == bot_tag.to_lowercase())
            .cloned()
    }

    pub(in super::super) fn bot_chat_tag(&self, cx: &mut App) -> String {
        BotTagRules::sanitize(&self.resolved_chat(cx).participant.tag, false, false)
    }

    pub(in super::super) fn bot_chat_details(&self, cx: &mut App) -> Option<BotModel> {
        let tag = self.bot_chat_tag(cx);
        if !self.resolved_chat(cx).is_bot_chat() || !BotTagRules::is_valid(&tag) {
            return None;
        }
        BotServicesStore::bot_by_tag(&tag, cx)
    }

    pub(in super::super) fn can_manage_bot_chat(&self, cx: &mut App) -> bool {
        let chat = self.resolved_chat(cx);
        if !chat.is_bot_chat() {
            return false;
        }
        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        self.bot_chat_details(cx).is_some_and(|bot| bot.is_owned_by_requester)
            || (!user_id.is_empty() && chat.participant.bot_owner_id.as_deref() == Some(user_id.as_str()))
    }

    pub(in super::super) fn bot_public_commands(&self, cx: &mut App) -> Vec<BotCommandModel> {
        let tag = self.bot_chat_tag(cx);
        let bot_user_id = self.resolved_chat(cx).participant.id;
        let chat_id = self.resolved_chat_id(cx);
        BotServicesStore::public_commands(None, Some(&tag), Some(&bot_user_id), Some(&chat_id), None, Some("private"), cx)
            .map(|model| model.commands)
            .unwrap_or_default()
    }

    pub(in super::super) fn bot_public_mini_apps(&self, cx: &mut App) -> Vec<BotMiniAppModel> {
        let tag = self.bot_chat_tag(cx);
        let bot_user_id = self.resolved_chat(cx).participant.id;
        BotServicesStore::public_mini_apps(None, Some(&tag), Some(&bot_user_id), cx)
    }

    pub(in super::super) fn bot_public_mini_apps_cache_key(&self, cx: &mut App) -> String {
        let tag = self.bot_chat_tag(cx);
        let bot_user_id = self.resolved_chat(cx).participant.id;
        BotServicesStore::public_mini_apps_cache_key(None, Some(&tag), Some(&bot_user_id))
    }

    pub(in super::super) fn bot_chat_load_key(&self, cx: &mut App) -> String {
        let chat = self.resolved_chat(cx);
        format!(
            "{}|{}|{}|{}",
            self.resolved_chat_id(cx),
            chat.participant.id,
            self.bot_chat_tag(cx),
            chat.participant.bot_is_active
        )
    }

    pub(in super::super) fn can_load_bot_chat_metadata(&self, cx: &mut App) -> bool {
        let chat = self.resolved_chat(cx);
        chat.is_active_bot_chat() && BotTagRules::is_valid(&self.bot_chat_tag(cx)) && !chat.participant.id.is_empty()
    }

    pub(in super::super) fn bot_chat_menu_status(&self, cx: &mut App) -> SaiWsSliceStatus {
        if !self.bot_public_commands(cx).is_empty() || !self.bot_public_mini_apps(cx).is_empty() {
            return SaiWsSliceStatus::Fulfilled;
        }
        if !self.can_load_bot_chat_metadata(cx) {
            return SaiWsSliceStatus::Fulfilled;
        }
        let actions = BotActionsStore::shared(cx);
        let actions = actions.read(cx);
        let command_status = actions.get_public_bot_commands_sai.status();
        let mini_app_status = actions.list_public_mini_apps_sai.status();
        if command_status == SaiWsSliceStatus::Pending || mini_app_status == SaiWsSliceStatus::Pending {
            return SaiWsSliceStatus::Pending;
        }
        if command_status == SaiWsSliceStatus::Rejected || mini_app_status == SaiWsSliceStatus::Rejected {
            return SaiWsSliceStatus::Rejected;
        }
        if command_status == SaiWsSliceStatus::Fulfilled && mini_app_status == SaiWsSliceStatus::Fulfilled {
            return SaiWsSliceStatus::Fulfilled;
        }
        SaiWsSliceStatus::Idle
    }

    pub(in super::super) fn bot_chat_menu_error(&self, cx: &mut App) -> Option<String> {
        let actions = BotActionsStore::shared(cx);
        let actions = actions.read(cx);
        actions.get_public_bot_commands_sai.error().or_else(|| actions.list_public_mini_apps_sai.error())
    }

    pub(in super::super) fn active_bot_reply_keyboard_message(
        &self,
        bounded_presentation: Option<&MessageScopePresentationSnapshot>,
        cx: &mut App,
    ) -> Option<GetMessageMessage> {
        let message = match bounded_presentation.and_then(|snapshot| snapshot.latest_reply_keyboard_message.clone()) {
            Some(message) => message,
            None => {
                let scope = self.message_scope(cx);
                message_services(cx, |store, cx| store.latest_reply_keyboard_message(&scope, cx))?
            }
        };
        let markup = message.reply_markup.as_ref()?;
        if markup.remove_keyboard {
            return None;
        }
        if markup.has_reply_keyboard() && self.dismissed_bot_reply_keyboard_message_id.as_deref() != Some(message.id.as_str()) {
            return Some(message);
        }
        None
    }

    pub(in super::super) fn bot_reply_keyboard_panel_height(&self, message: Option<&GetMessageMessage>, peer_blocked: bool, cx: &mut App) -> f32 {
        if !self.should_show_bot_reply_keyboard(peer_blocked, cx) {
            return 0.0;
        }
        let Some(rows) = message.and_then(|message| message.reply_markup.as_ref()).map(|markup| &markup.keyboard) else {
            return 0.0;
        };
        if rows.is_empty() {
            return 0.0;
        }
        (rows.len() as f32 * 48.0).min(210.0) + 50.0
    }

    pub(in super::super) fn should_show_bot_reply_keyboard(&self, peer_blocked: bool, cx: &mut App) -> bool {
        !self.is_any_capture_active(cx)
            && !self.is_message_request(cx)
            && !self.state.is_message_selection_active
            && !self.topic_posting_locked(cx)
            && !self.channel_posting_locked(cx)
            && !peer_blocked
            && !self.picker_visible
            && !self.inline_composer_context.is_presented()
    }

    pub(in super::super) fn bot_force_reply_message_id(&self, last_message: Option<&GetMessageMessage>, cx: &mut App) -> Option<String> {
        if self.is_message_request(cx) || self.channel_posting_locked(cx) {
            return None;
        }
        let message = last_message?;
        if !message.reply_markup.as_ref().is_some_and(|markup| markup.force_reply) {
            return None;
        }
        let chat = self.resolved_chat(cx);
        if MessageList::is_own_bubble(message, &chat, cx) {
            return None;
        }
        Some(message.id.clone())
    }

    pub(in super::super) fn bot_start_card(&mut self, chat: &ChatInfo, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.weak_entity();
        let details = self.bot_chat_details(cx);
        let bot_id = details.as_ref().map(|details| details.id.clone());
        BotChatStartCard {
            id: "chat-bot-start-card".into(),
            name: chat.direct_peer_profile_name(),
            tag: self.bot_chat_tag(cx),
            bot_user_id: chat.participant.id.clone(),
            description: details
                .map(|details| details.description)
                .or_else(|| chat.description.clone())
                .unwrap_or_default(),
            avatar_url: if chat.participant.more.logo.is_empty() {
                chat.logo_url.clone()
            } else {
                Some(chat.participant.more.logo.clone())
            },
            can_manage: self.can_manage_bot_chat(cx),
            on_start: Rc::new(Self::act(this.clone(), |screen, _, cx| {
                if screen.bot_start_sent {
                    return;
                }
                screen.bot_start_sent = true;
                cx.notify();
                let chat_id = screen.resolved_chat_id(cx);
                message_interactions(cx, |store, cx| {
                    let _ = store.send_text(&chat_id, "/start", None, None, None, None, None, None, None, None, None, None, cx);
                });
            })),
            on_manage: Rc::new(Self::act(this, move |screen, _, cx| {
                let Some(bot_id) = bot_id.clone() else {
                    return;
                };
                screen.push_route(RootRoute::BotSettings { bot_id }, cx);
            })),
        }
        .into_any_element()
    }

    pub(in super::super) fn toggle_bot_chat_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_bot_chat_menu {
            self.dismiss_bot_chat_menu(window, cx);
            return;
        }
        self.bot_menu_starts_expanded = BotChatMenuPresentationContract::starts_expanded(0.0);
        self.close_picker(cx);
        resign_chat_keyboard(window, cx);
        if matches!(self.bot_chat_menu_status(cx), SaiWsSliceStatus::Rejected | SaiWsSliceStatus::Idle) {
            self.load_bot_chat_metadata(cx);
        }
        self.show_bot_chat_menu = true;
        cx.notify();
    }

    pub(in super::super) fn dismiss_bot_chat_menu(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.show_bot_chat_menu = false;
        cx.notify();
    }

    pub(in super::super) fn insert_via_bot_mention(&mut self, message: &GetMessageMessage, cx: &mut Context<Self>) {
        let tag = BotTagRules::sanitize(message.via_bot_tag.as_deref().unwrap_or_default(), false, false);
        if !BotTagRules::is_valid(&tag) {
            return;
        }
        let text = format!("@{tag} ");
        let selection = NSRange::new(NSRange::full(&text).length, 0);
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        let language_code = self.inline_composer_language_code();
        if BotServicesStore::inline_commands(&tag, &chat_id, topic_id.as_deref(), &language_code, cx).is_some() {
            self.set_composer_text(text, selection, cx);
            return;
        }
        let text_before = self.state.input_text.clone();
        let scope = self.inline_composer_scope(cx);
        bot_interactions(cx, |store, cx| {
            store.load_inline_commands(&tag, &chat_id, topic_id.as_deref(), &language_code, &scope, cx);
        });
        cx.spawn(async move |this, cx| {
            let deadline = RiseClock::monotonic() + Duration::from_millis(700);
            while RiseClock::monotonic() < deadline {
                let cached = cx.update(|cx| {
                    BotServicesStore::inline_commands(&tag, &chat_id, topic_id.as_deref(), &language_code, cx).is_some()
                });
                if cached {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(16)).await;
            }
            let _ = this.update(cx, |screen, cx| {
                if screen.state.input_text != text_before {
                    return;
                }
                screen.set_composer_text(text, selection, cx);
            });
        })
        .detach();
    }

    pub(in super::super) fn insert_bot_inline_query(&mut self, query: &str, message: &GetMessageMessage, cx: &mut Context<Self>) {
        let sender_tag = BotTagRules::sanitize(&message.sender_tag, false, false);
        let tag = if BotTagRules::is_valid(&sender_tag) { sender_tag } else { self.bot_chat_tag(cx) };
        if !BotTagRules::is_valid(&tag) {
            return;
        }
        let text = format!("@{tag} {query}");
        let selection = NSRange::new(NSRange::full(&text).length, 0);
        self.set_composer_text(text, selection, cx);
    }

    pub(in super::super) fn handle_bot_reply_keyboard_button(
        &mut self,
        button: &MessageBotKeyboardButton,
        source_message: &GetMessageMessage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parsed_url = button.url.as_deref().and_then(Url::from_string);
        if let Some(web_app_url) = button.web_app_url.as_deref() {
            self.open_bot_mini_app_button(button, web_app_url, window, cx);
        } else if let Some(url) = parsed_url.as_ref().filter(|url| BotMiniAppURLPolicy::allows(url, false)) {
            let _ = rise_platform::open_url::open_url(url, cx);
        } else if let Some(callback_data) = button.callback_data.as_deref() {
            let raw_identifier = source_message.via_bot.as_deref().unwrap_or(source_message.sender_id.as_str());
            let bot_identifier = trimming_whitespaces_and_newlines(raw_identifier).to_owned();
            if !bot_identifier.is_empty() {
                let chat_id = self.resolved_chat_id(cx);
                let message_id = source_message.sai_server_id();
                let on_error: BotActionError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
                    send_failed_notice("bot_callback_failed", Some(error.to_string()), cx);
                });
                bot_interactions(cx, |store, cx| {
                    store.enqueue_callback_query(
                        &bot_identifier,
                        "",
                        Some(chat_id.as_str()),
                        Some(message_id.as_str()),
                        None,
                        Some(callback_data),
                        None,
                        None,
                        Some(on_error),
                        cx,
                    );
                });
            }
        } else if let Some(query) = button.switch_inline_query_current_chat.as_ref().or(button.switch_inline_query.as_ref()) {
            let selection = NSRange::new(NSRange::full(query).length, 0);
            self.set_composer_text(query.clone(), selection, cx);
        } else if button.is_plain_text() {
            let chat_id = self.resolved_chat_id(cx);
            let topic_id = self.resolved_topic_id(cx);
            let post_topic_id = self.post_topic_id(cx);
            let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
            message_interactions(cx, |store, cx| {
                let _ = store.send_text(
                    &chat_id,
                    &button.text,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    cx,
                );
            });
            self.follow_outgoing_message(window, cx);
        } else {
            Notifier::send(
                tr("bot_button_not_supported"),
                NotifierSendOptions {
                    r#type: NotifierType::Warning,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        }

        if source_message.reply_markup.as_ref().is_some_and(|markup| markup.one_time_keyboard) {
            self.dismissed_bot_reply_keyboard_message_id = Some(source_message.id.clone());
            cx.notify();
        }
    }

    /// launch url of a mini app was refused or failed, the person retries from the notice
    pub(in super::super) fn mini_app_launch_error(
        this: WeakEntity<Self>,
        handle: AnyWindowHandle,
        retry: Rc<dyn Fn(&mut Self, &mut Window, &mut Context<Self>)>,
    ) -> BotActionError {
        Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            let content = BotInteractionsStore::shared(cx).read(cx).mini_app_launch_failure_text(error);
            let (this, retry) = (this.clone(), retry.clone());
            Notifier::send_retry(
                tr("bot_mini_app_open_failed"),
                NotifierRetryOptions { content: Some(content), ..NotifierRetryOptions::default() },
                |_| {},
                move |cx: &mut App| {
                    Self::deliver_in_window(&this, handle, cx, move |screen, window, cx| retry(screen, window, cx));
                },
                cx,
            );
        })
    }

    pub(in super::super) fn mini_app_launch_opener(
        this: WeakEntity<Self>,
        title: String,
        allows_development_http: bool,
        bot_id: String,
    ) -> BotActionResult<BotMiniAppLaunchData> {
        Rc::new(move |launch: BotMiniAppLaunchData, cx: &mut App| {
            let Some(url) = Url::from_string(&launch.launch_url) else {
                return;
            };
            let title = title.clone();
            let bot_id = bot_id.clone();
            Self::deliver(&this, cx, move |screen, cx| {
                let chat_id = screen.resolved_chat_id(cx);
                let route = RootRoute::BotMiniApp {
                    url,
                    title,
                    allows_development_http,
                    chat_id: Some(chat_id),
                    bot_id: Some(bot_id),
                    is_web_link: false,
                };
                screen.push_route(route, cx);
            });
        })
    }

    pub(in super::super) fn open_bot_mini_app_button(
        &mut self,
        button: &MessageBotKeyboardButton,
        fallback_url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let allows_development_http = self.can_manage_bot_chat(cx);
        let Some(fallback) = Url::from_string(fallback_url) else {
            return;
        };
        if !BotMiniAppURLPolicy::allows(&fallback, allows_development_http) {
            return;
        }
        let tag = self.bot_chat_tag(cx);
        let bot_user_id = self.resolved_chat(cx).participant.id;
        let title = BotInteractionsStore::mini_app_title(
            button.bot_id.as_deref(),
            button.mini_app_id.as_deref(),
            None,
            Some(tag.as_str()),
            Some(bot_user_id.as_str()),
            cx,
        )
        .unwrap_or_else(|| button.text.clone());
        let chat_id = self.resolved_chat_id(cx);
        let (Some(bot_id), Some(mini_app_id)) = (
            button.bot_id.as_deref().filter(|id| !id.is_empty()),
            button.mini_app_id.as_deref().filter(|id| !id.is_empty()),
        ) else {
            self.push_route(
                RootRoute::BotMiniApp {
                    url: fallback,
                    title,
                    allows_development_http,
                    chat_id: Some(chat_id),
                    bot_id: button.bot_id.clone(),
                    is_web_link: false,
                },
                cx,
            );
            return;
        };
        let this = cx.weak_entity();
        let retry_button = button.clone();
        let retry_url = fallback_url.to_owned();
        let on_error = Self::mini_app_launch_error(
            this.clone(),
            window.window_handle(),
            Rc::new(move |screen: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
                screen.open_bot_mini_app_button(&retry_button, &retry_url, window, cx);
            }),
        );
        let on_result = Self::mini_app_launch_opener(this, title, allows_development_http, bot_id.to_owned());
        bot_interactions(cx, |store, cx| {
            store.get_mini_app_launch(
                bot_id,
                mini_app_id,
                Some(chat_id.as_str()),
                None,
                allows_development_http,
                None,
                Some(on_result),
                Some(on_error),
                cx,
            );
        });
    }

    pub(in super::super) fn load_bot_chat_metadata(&mut self, cx: &mut Context<Self>) {
        if !self.can_load_bot_chat_metadata(cx) {
            return;
        }
        let chat = self.resolved_chat(cx);
        let tag = self.bot_chat_tag(cx);
        let chat_id = self.resolved_chat_id(cx);
        let bot_user_id = chat.participant.id;
        bot_interactions(cx, |store, cx| {
            store.load_bot_by_tag(&tag, cx);
            store.load_public_commands(
                None,
                Some(tag.as_str()),
                Some(bot_user_id.as_str()),
                Some(chat_id.as_str()),
                None,
                Some("private"),
                cx,
            );
            store.load_public_mini_apps(None, Some(tag.as_str()), Some(bot_user_id.as_str()), None, cx);
        });
    }

    pub(in super::super) fn open_public_mini_app(&mut self, mini_app: &BotMiniAppModel, window: &mut Window, cx: &mut Context<Self>) {
        let allows_development_http = self.can_manage_bot_chat(cx);
        let chat_id = self.resolved_chat_id(cx);
        let this = cx.weak_entity();
        let retry_app = mini_app.clone();
        let on_error = Self::mini_app_launch_error(
            this.clone(),
            window.window_handle(),
            Rc::new(move |screen: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
                screen.open_public_mini_app(&retry_app, window, cx);
            }),
        );
        let on_result =
            Self::mini_app_launch_opener(this, mini_app.title.clone(), allows_development_http, mini_app.bot_id.clone());
        bot_interactions(cx, |store, cx| {
            store.get_mini_app_launch(
                &mini_app.bot_id,
                &mini_app.id,
                Some(chat_id.as_str()),
                None,
                allows_development_http,
                None,
                Some(on_result),
                Some(on_error),
                cx,
            );
        });
    }
}
