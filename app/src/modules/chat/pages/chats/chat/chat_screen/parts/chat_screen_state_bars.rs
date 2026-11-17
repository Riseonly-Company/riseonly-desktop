use std::rc::Rc;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use gpui::prelude::*;
use gpui::{AnyElement, App, AppContext, Context, Entity, FontWeight, Window, div, px};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::swiftui::prelude::*;
use rise_ui::{IconUi, MainText};
use crate::app::router::navigation_store::NavigationStore;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;
use rise_canon::ChatMessageRequestAction;
use rise_ui::swiftui::progress_view::progress_view;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatPublicJoinOutcome;
use crate::modules::chat::stores::chats::chat_interactions::chat_invites_interactions::ChatInteractionsPublicJoinResult;
use rise_ui::main_text::TextAlignT;
use super::super::chat_screen::*;
use super::super::chat_screen_support::*;

impl ChatScreen {
    /// full width call to action that stands in for the composer
    pub(in super::super) fn composer_action_row(&self, button: LiquidGlassButton, safe_bottom_inset: f32, cx: &App) -> AnyElement {
        div()
            .w_full()
            .flex_none()
            .flex()
            .flex_col()
            .px(px(10.0))
            .pt(px(6.0))
            .pb(px(self.composer_bottom_padding(safe_bottom_inset, cx)))
            .child(button)
            .into_any_element()
    }

    pub(in super::super) fn composer_icon_label_row(icon: &'static str, icon_size: f32, title: String, color: gpui::Hsla) -> gpui::Div {
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(8.0))
            .children(IconUi::sized(icon, px(icon_size), color, false))
            .child(MainText::new(title).px(16.0).font_weight(FontWeight::SEMIBOLD).color(color))
    }

    pub(in super::super) fn restricted_by_peer_bar(&mut self, text: String, theme: &AppTheme, safe_bottom_inset: f32, cx: &mut Context<Self>) -> AnyElement {
        let secondary = theme.text.secondary;
        let plate = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(7.0))
            .px(px(16.0))
            .min_h(px(ChatComposerPlateMetrics::ACTION_HEIGHT))
            .children(IconUi::sized("hand.raised.fill", px(12.0), secondary, false))
            .child(
                MainText::new(text)
                    .px(13.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::MEDIUM)
                    .color(secondary)
                    .number_of_lines(2),
            )
            .liquid_chrome_capsule_with(0.12, true, None, false);
        self.composer_plate_row(plate, safe_bottom_inset, cx)
    }

    pub(in super::super) fn community_preview_bar(&mut self, chat: &ChatInfo, theme: &AppTheme, safe_bottom_inset: f32, cx: &mut Context<Self>) -> AnyElement {
        let requires_approval = chat.join_approval_required;
        let title = if self.public_join_request_sent {
            tr("chat_public_preview_request_sent")
        } else if requires_approval {
            tr("chat_public_preview_request")
        } else if chat.r#type == ChatType::Channel {
            tr("chat_public_preview_subscribe")
        } else {
            tr("chat_public_preview_join")
        };
        let hint = self.shows_thread_members_only_hint(cx).then(|| {
            div().w_full().h(px(ChatComposerPlateMetrics::HINT_HEIGHT)).px(px(8.0)).child(
                MainText::new(tr("chat_thread_members_only"))
                    .px(13.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::MEDIUM)
                    .color(theme.text.secondary)
                    .number_of_lines(1),
            )
        });
        let icon = if self.public_join_request_sent {
            "clock.fill"
        } else if chat.r#type == ChatType::Channel {
            "bell.badge.fill"
        } else {
            "person.badge.plus"
        };
        let mut content = div().w_full().flex().flex_row().items_center().justify_center().gap(px(8.0));
        content = if self.public_join_submitting {
            content.child(progress_view().tint(theme.content_on(theme.primary._100)))
        } else {
            content.children(IconUi::sized(icon, px(16.0), theme.content_on(theme.primary._100), false))
        };
        let content = content.child(MainText::new(title.clone()).px(16.0).font_weight(FontWeight::SEMIBOLD).color(theme.content_on(theme.primary._100)));
        let join_chat = chat.clone();
        let button = LiquidGlassButton::new("chat-public-join", LiquidGlassButtonShape::Capsule)
            .horizontal_padding(18.0)
            .vertical_padding(13.0)
            .tint(Some(theme.primary._100.opacity(0.9)))
            .disabled(self.public_join_submitting || self.public_join_request_sent)
            .accessibility_label(Some(title))
            .on_press(Self::act(cx.weak_entity(), move |screen, _, cx| {
                if screen.public_join_submitting {
                    return;
                }
                screen.public_join_submitting = true;
                cx.notify();
                let this = cx.weak_entity();
                let on_result: ChatInteractionsPublicJoinResult = Box::new(move |outcome, cx: &mut App| {
                    Self::deliver(&this, cx, move |screen, cx| {
                        screen.public_join_submitting = false;
                        match outcome {
                            ChatPublicJoinOutcome::Joined => {
                                screen.public_join_request_sent = requires_approval;
                                if !requires_approval
                                    && (screen.resolved_topic_id(cx).is_some() || screen.resolved_thread_root_id().is_some())
                                {
                                    screen.retry_initial_messages(cx);
                                }
                            }
                            ChatPublicJoinOutcome::Banned => screen.public_join_banned = true,
                            ChatPublicJoinOutcome::Failed => {}
                        }
                        cx.notify();
                    });
                });
                let chat = join_chat.clone();
                chat_interactions(cx, |store, cx| store.join_public_chat(&chat, requires_approval, on_result, cx));
            }))
            .child(content);
        let bottom_padding = self.composer_bottom_padding(safe_bottom_inset, cx);
        div()
            .w_full()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(ChatComposerPlateMetrics::HINT_SPACING))
            .px(px(10.0))
            .pt(px(6.0))
            .pb(px(bottom_padding))
            .children(hint)
            .child(button)
            .into_any_element()
    }

    pub(in super::super) fn community_banned_bar(&mut self, chat: &ChatInfo, theme: &AppTheme, safe_bottom_inset: f32, cx: &mut Context<Self>) -> AnyElement {
        let text = tr(if chat.r#type == ChatType::Channel {
            "chat_public_preview_banned_channel"
        } else {
            "chat_public_preview_banned_group"
        });
        self.composer_plate_row(Self::locked_pill("lock.fill", text, theme), safe_bottom_inset, cx)
    }

    pub(in super::super) fn blocked_peer_bar(&mut self, chat: &ChatInfo, theme: &AppTheme, safe_bottom_inset: f32, cx: &mut Context<Self>) -> AnyElement {
        let peer_id = chat.participant.id.clone();
        let pending = UserServicesStore::shared(cx).read(cx).contact_block_pending_targets.contains(&peer_id);
        let restarts_bot = chat.is_bot_chat();
        let title = tr(if restarts_bot { "bot_start" } else { "contact_unblock_action" });
        let chat_id = self.resolved_chat_id(cx);
        let button = LiquidGlassButton::new("chat-unblock-peer", LiquidGlassButtonShape::Capsule)
            .horizontal_padding(18.0)
            .vertical_padding(13.0)
            .tint(Some(theme.primary._100.opacity(0.9)))
            .disabled(pending)
            .accessibility_label(Some(title.clone()))
            .on_press(move |_, cx| {
                if pending {
                    return;
                }
                let peer_id = peer_id.clone();
                let chat_id = chat_id.clone();
                let on_result = move |result: anyhow::Result<bool>, cx: &mut App| {
                    if !restarts_bot || result.is_err() {
                        return;
                    }
                    message_interactions(cx, |store, cx| {
                        let _ = store.send_text(&chat_id, "/start", None, None, None, None, None, None, None, None, None, None, cx);
                    });
                };
                UserInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.set_user_block_scope(&peer_id, None, on_result, cx);
                });
            })
            .child(Self::composer_icon_label_row(
                if restarts_bot { "arrow.clockwise" } else { "hand.raised.slash.fill" },
                16.0,
                title,
                theme.content_on(theme.primary._100),
            ));
        self.composer_action_row(button, safe_bottom_inset, cx)
    }

    pub(in super::super) fn channel_subscriber_bar(&mut self, chat: &ChatInfo, theme: &AppTheme, safe_bottom_inset: f32, cx: &mut Context<Self>) -> AnyElement {
        let muted = ChatServicesStore::shared(cx).read(cx).effective_is_muted(chat, cx);
        let title = tr(if muted { "ctx_unmute" } else { "ctx_mute" });
        let chat_id = chat.id.clone();
        let button = LiquidGlassButton::new("chat-channel-mute", LiquidGlassButtonShape::Capsule)
            .horizontal_padding(18.0)
            .vertical_padding(13.0)
            .accessibility_label(Some(title.clone()))
            .on_press(move |_, cx| chat_interactions(cx, |store, cx| store.toggle_chat_muted(&chat_id, cx)))
            .child(Self::composer_icon_label_row(
                if muted { "bell.fill" } else { "bell.slash.fill" },
                16.0,
                title,
                theme.primary._100,
            ));
        self.composer_action_row(button, safe_bottom_inset, cx)
    }

    pub(in super::super) fn message_request_secondary_button(
        id: &'static str,
        title: String,
        icon: &'static str,
        is_pending: bool,
        theme: &AppTheme,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> gpui::Div {
        let error = theme.semantic.error_100;
        let button = LiquidGlassButton::new(id, LiquidGlassButtonShape::Capsule)
            .horizontal_padding(12.0)
            .vertical_padding(9.0)
            .tint(Some(error.opacity(0.12)))
            .disabled(is_pending)
            .accessibility_label(Some(title.clone()))
            .on_press(action)
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .gap(px(5.0))
                    .children(IconUi::sized(icon, px(13.0), error, false))
                    .child(MainText::new(title).px(13.0).font_weight(FontWeight::MEDIUM).color(error).number_of_lines(1)),
            );
        div().flex_1().flex().flex_col().child(button)
    }

    pub(in super::super) fn resolve_message_request_and_leave(chat_id: String, action: ChatMessageRequestAction, nav_store: Option<Entity<NavigationStore>>, cx: &mut App) {
        chat_interactions(cx, |store, cx| store.resolve_message_request(&chat_id, action, None, cx));
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |store, cx| store.pop(cx));
        }
    }

    pub(in super::super) fn message_request_decision_bar(&mut self, theme: &AppTheme, safe_bottom_inset: f32, cx: &mut Context<Self>) -> AnyElement {
        let is_pending = ChatActionsStore::shared(cx).read(cx).resolve_message_request_sai.is_pending();
        let chat_id = self.resolved_chat_id(cx);
        let nav_store = self.nav_store();
        let accept_title = tr("message_request_accept");
        let mut accept_content = div().w_full().flex().flex_row().items_center().justify_center().gap(px(7.0));
        accept_content = if is_pending {
            accept_content.child(progress_view().tint(theme.content_on(theme.primary._100)))
        } else {
            accept_content.children(IconUi::sized("checkmark.circle.fill", px(16.0), theme.content_on(theme.primary._100), true))
        };
        let accept_chat_id = chat_id.clone();
        let accept = LiquidGlassButton::new("chat-message-request-accept", LiquidGlassButtonShape::Capsule)
            .horizontal_padding(16.0)
            .vertical_padding(12.0)
            .tint(Some(theme.primary._100.opacity(0.84)))
            .disabled(is_pending)
            .accessibility_label(Some(accept_title.clone()))
            .on_press(move |_, cx| {
                let read_chat_id = accept_chat_id.clone();
                let on_complete: Rc<dyn Fn(bool, Option<ChatInfo>, &mut App)> =
                    Rc::new(move |success: bool, _: Option<ChatInfo>, cx: &mut App| {
                        if !success {
                            return;
                        }
                        if let Some(task) = message_interactions(cx, |store, cx| store.mark_chat_as_read(&read_chat_id, None, false, cx)) {
                            task.detach();
                        }
                    });
                let chat_id = accept_chat_id.clone();
                chat_interactions(cx, |store, cx| {
                    store.resolve_message_request(&chat_id, ChatMessageRequestAction::Accept, Some(on_complete), cx);
                });
            })
            .child(accept_content.child(MainText::new(accept_title).px(16.0).font_weight(FontWeight::SEMIBOLD).color(theme.content_on(theme.primary._100))));
        let delete_chat_id = chat_id.clone();
        let delete_nav = nav_store.clone();
        let delete = Self::message_request_secondary_button(
            "chat-message-request-delete",
            tr("message_request_delete"),
            "trash",
            is_pending,
            theme,
            move |_, cx| {
                Self::resolve_message_request_and_leave(delete_chat_id.clone(), ChatMessageRequestAction::Delete, delete_nav.clone(), cx);
            },
        );
        let block = Self::message_request_secondary_button(
            "chat-message-request-block",
            tr("message_request_block"),
            "hand.raised.fill",
            is_pending,
            theme,
            move |_, cx| {
                Self::resolve_message_request_and_leave(chat_id.clone(), ChatMessageRequestAction::DeleteAndBlock, nav_store.clone(), cx);
            },
        );
        let bottom_padding = self.composer_bottom_padding(safe_bottom_inset, cx);
        let card = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(8.0))
            .child(accept)
            .child(div().w_full().flex().flex_row().gap(px(8.0)).child(delete).child(block))
            .liquid_chrome_rounded_rectangle(24.0);
        div()
            .w_full()
            .flex_none()
            .px(px(10.0))
            .pt(px(6.0))
            .pb(px(bottom_padding))
            .child(card)
            .into_any_element()
    }
}
