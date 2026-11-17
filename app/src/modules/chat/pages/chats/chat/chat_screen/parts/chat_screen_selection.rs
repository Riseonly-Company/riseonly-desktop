use std::collections::HashSet;
use gpui::prelude::*;
use gpui::{AnyElement, App, AppContext, Context, FontWeight, Window, div, px};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::keyboard_dismiss::KeyboardDismissModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::Transition;
use rise_ui::{IconUi, MainText};
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::modules::chat::components::chat::composer::chat_composer_types::{ChatBottomBarMetrics, resign_chat_keyboard};
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::shared::message_permissions::MessagePermissions;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageForwardLimits};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::core::widgets::sheets::message_external_share_materializer::MessageExternalShareCoordinator;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use super::super::chat_screen::*;
use super::super::chat_screen_support::*;

impl ChatScreen {
    pub(in super::super) fn selected_messages(&self, cx: &mut App) -> Vec<GetMessageMessage> {
        let chat_id = self.resolved_chat_id(cx);
        let scope = self.message_scope(cx);
        let mut selected: Vec<GetMessageMessage> = self
            .state
            .selected_messages_by_server_id
            .iter()
            .filter_map(|(server_id, snapshot)| {
                message_services(cx, |store, cx| {
                    if store.is_confirmed_deleted(server_id, &chat_id) {
                        return None;
                    }
                    Some(store.message(server_id, &scope, cx).unwrap_or_else(|| snapshot.clone()))
                })
            })
            .collect();
        selected.sort_by(|left, right| {
            left.created_at
                .total_cmp(&right.created_at)
                .then_with(|| left.sai_server_id().cmp(&right.sai_server_id()))
        });
        selected
    }

    pub(in super::super) fn selected_message_ids(&self, cx: &mut App) -> HashSet<String> {
        self.selected_messages(cx).iter().map(|message| message.sai_server_id()).collect()
    }

    pub(in super::super) fn selected_messages_can_delete_for_everyone(&self, cx: &mut App) -> bool {
        let messages = self.selected_messages(cx);
        let chat = self.resolved_chat(cx);
        !messages.is_empty()
            && messages.iter().all(|message| {
                let is_own = MessageList::is_own_bubble(message, &chat, cx);
                MessagePermissions { message, chat: &chat, is_own }.can_delete(cx)
            })
    }

    pub(in super::super) fn selected_messages_can_delete_for_me(&self, cx: &mut App) -> bool {
        let messages = self.selected_messages(cx);
        let chat = self.resolved_chat(cx);
        !messages.is_empty()
            && messages.iter().all(|message| {
                let is_own = MessageList::is_own_bubble(message, &chat, cx);
                MessagePermissions { message, chat: &chat, is_own }.can_delete_for_me(cx)
            })
    }

    pub(in super::super) fn selected_forwardable_messages(&self, cx: &mut App) -> Vec<GetMessageMessage> {
        let chat = self.resolved_chat(cx);
        self.selected_messages(cx)
            .into_iter()
            .filter(|message| {
                let is_own = MessageList::is_own_bubble(message, &chat, cx);
                MessagePermissions { message, chat: &chat, is_own }.has_forwardable_content()
            })
            .collect()
    }

    pub(in super::super) fn selected_messages_can_forward(&self, cx: &mut App) -> bool {
        let messages = self.selected_forwardable_messages(cx);
        let chat = self.resolved_chat(cx);
        !messages.is_empty()
            && messages.len() as i64 <= MessageForwardLimits::max_messages()
            && messages.iter().all(|message| {
                let is_own = MessageList::is_own_bubble(message, &chat, cx);
                MessagePermissions { message, chat: &chat, is_own }.can_forward(cx)
            })
    }

    pub(in super::super) fn selected_messages_can_share(&self, cx: &mut App) -> bool {
        let messages = self.selected_forwardable_messages(cx);
        let chat = self.resolved_chat(cx);
        !messages.is_empty()
            && messages.iter().all(|message| {
                let is_own = MessageList::is_own_bubble(message, &chat, cx);
                MessagePermissions { message, chat: &chat, is_own }.can_share()
            })
    }

    pub(in super::super) fn message_selection_header_button(
        &self,
        id: &'static str,
        title: String,
        destructive: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let color = if destructive { theme.semantic.error_100 } else { theme.text.primary };
        LiquidGlassButton::new(id, LiquidGlassButtonShape::Capsule)
            .horizontal_padding(12.0)
            .vertical_padding(10.0)
            .accessibility_label(Some(title.clone()))
            .on_press(action)
            .child(
                MainText::new(title)
                    .px(14.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(color)
                    .line_limit(1)
                    .minimum_scale_factor(0.72),
            )
            .into_any_element()
    }

    pub(in super::super) fn begin_message_selection(&mut self, message: &GetMessageMessage, window: &mut Window, cx: &mut Context<Self>) {
        let chat = self.resolved_chat(cx);
        let is_own = MessageList::is_own_bubble(message, &chat, cx);
        if !MessagePermissions::new(message, &chat, is_own).can_select(cx) {
            return;
        }

        self.deactivate_message_search(window, cx);
        resign_chat_keyboard(window, cx);
        self.reset_composer_input_surface(cx);
        self.dismiss_inline_composer(true, window, cx);
        if self.show_bot_chat_menu {
            self.dismiss_bot_chat_menu(window, cx);
        }
        self.state.is_message_selection_active = true;
        self.state.selected_messages_by_server_id.insert(message.sai_server_id(), message.clone());
        let _ = selection_changed();
        cx.notify();
    }

    pub(in super::super) fn toggle_message_selection(&mut self, message: &GetMessageMessage, _window: &mut Window, cx: &mut Context<Self>) {
        let raw_server_id = message.sai_server_id();
        let server_id = trimming_whitespaces_and_newlines(&raw_server_id).to_owned();
        if !self.state.is_message_selection_active || server_id.is_empty() || GetSaiServerId::is_temp_id(Some(server_id.as_str())) {
            tracing::warn!(target: "riseonly::X1870", %server_id, active = self.state.is_message_selection_active, "selection toggle refused");
            return;
        }
        let removed = self.state.selected_messages_by_server_id.remove(&server_id).is_some();
        if !removed {
            self.state.selected_messages_by_server_id.insert(server_id.clone(), message.clone());
        }
        tracing::warn!(target: "riseonly::X1870", %server_id, removed, selected = self.state.selected_messages_by_server_id.len(), shown = self.selected_messages(cx).len(), "selection toggled");
        let _ = selection_changed();
        cx.notify();
    }

    pub(in super::super) fn cancel_message_selection(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.is_message_selection_active {
            return;
        }
        self.state.is_message_selection_active = false;
        self.state.selected_messages_by_server_id.clear();
        self.state.show_selected_messages_delete_dialog = false;
        cx.notify();
    }

    pub(in super::super) fn prune_confirmed_deleted_message_selection(&mut self, cx: &mut Context<Self>) {
        if !self.state.is_message_selection_active {
            return;
        }
        let chat_id = self.resolved_chat_id(cx);
        let services = MessageServicesStore::shared(cx);
        let services = services.read(cx);
        self.state.selected_messages_by_server_id.retain(|message_id, _| !services.is_confirmed_deleted(message_id, &chat_id));
        cx.notify();
    }

    pub(in super::super) fn delete_selected_messages(&mut self, for_everyone: bool, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.selected_messages(cx).iter().map(GetMessageMessage::sai_server_id).collect();
        if ids.is_empty() {
            return;
        }
        self.cancel_message_selection(window, cx);
        self.delete_message_ids(&ids, for_everyone, cx);
    }

    pub(in super::super) fn share_selected_messages_externally(&mut self, cx: &mut Context<Self>) {
        if !self.selected_messages_can_share(cx) {
            return;
        }
        let messages = self.selected_forwardable_messages(cx);
        MessageExternalShareCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.present(messages, cx));
    }

    pub(in super::super) fn present_selected_messages_for_forwarding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_messages_can_forward(cx) {
            return;
        }
        let messages = self.selected_forwardable_messages(cx);
        self.cancel_message_selection(window, cx);
        ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_messages(messages, cx));
    }

    pub(in super::super) fn message_selection_bar(&mut self, theme: &AppTheme, safe_bottom_inset: f32, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.weak_entity();
        let circle = ChatBottomBarMetrics::CIRCLE;
        let delete = Self::chat_circle_chrome_button(
            "chat-selection-delete",
            circle,
            self.selected_messages_can_delete_for_me(cx) || self.selected_messages_can_delete_for_everyone(cx),
            Self::act(this.clone(), |screen, _, cx| {
                screen.state.show_selected_messages_delete_dialog = true;
                cx.notify();
            }),
            IconUi::sized("trash", px(17.0), theme.semantic.error_100, false),
        );
        let share = Self::chat_circle_chrome_button(
            "chat-selection-share",
            circle,
            self.selected_messages_can_share(cx),
            Self::act(this.clone(), |screen, _, cx| screen.share_selected_messages_externally(cx)),
            IconUi::sized("square.and.arrow.up", px(17.0), theme.text.primary, false),
        );
        let forward = Self::chat_circle_chrome_button(
            "chat-selection-forward",
            circle,
            self.selected_messages_can_forward(cx),
            Self::act(this, |screen, window, cx| screen.present_selected_messages_for_forwarding(window, cx)),
            IconUi::sized("arrowshape.turn.up.right", px(18.0), theme.text.primary, false),
        );
        let bottom_padding = self.composer_bottom_padding(safe_bottom_inset, cx);
        let row = div()
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(20.0))
            .pt(px(ChatBottomBarMetrics::TOP_PADDING))
            .pb(px(bottom_padding))
            .child(delete)
            .child(share)
            .child(forward)
            .keyboard_dismiss_excluded();
        Self::appearing(
            "chat-selection-bar-presence",
            Transition::opacity(),
            ease_out(0.18),
            div().w_full().flex_none().child(row),
            window,
            cx,
        )
    }
}
