use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Entity, FontWeight, SharedString, Window};
use rise_chat::ChatType;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactStyle, NotificationKind, impact, notification};
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::page_header_ui::PanelToggleHeaderReserveKey;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::media::media_save_coordinator::MediaSaveCoordinator;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::notifier::notifier_store::Notifier;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellUi, list_shell_safe_area_insets};
use crate::modules::chat::components::chat::async_render::rise_message_list_view::{
    RiseMessageInitialScrollPosition, RiseMessageListView, RiseMessageScrollDestination, RiseMessageScrollRequest,
};
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::{ChatBackdropLayers, ChatBackdropLayout};
use crate::modules::chat::components::chat::media_viewer::chat_media_presentation::{
    ChatGalleryMessagesKey, ChatGalleryPaginatesKey,
};
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::shared::message_permissions::{ChatMessageAction, ChatModerationRights, MessagePermissions};
use crate::modules::chat::shared::utils::message_preview::MessagePreview;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatPerm};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::music::widgets::bottomsheets::playlists::message_mp3_import_sheet::MessageMusicImportPresenter;
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_types::SparksTarget;
use crate::modules::sparks::stores::sparks::sparks_interactions::sparks_interactions::SparksInteractionsStore;

/// screen state kept as long as the route is drawn
struct PinnedMessagesState {
    query: String,
    show_search: bool,
    search_presence: Entity<Presence<()>>,
    scroll_request: Option<RiseMessageScrollRequest>,
}

impl PinnedMessagesState {
    fn new(cx: &mut Context<Self>) -> Self {
        let search_presence = cx.new(|_| {
            Presence::new(None).with_transition(Transition::move_edge(Edge::Top).combined(Transition::opacity()))
        });
        Self { query: String::new(), show_search: false, search_presence, scroll_request: None }
    }

    fn toggle_search(&mut self, cx: &mut Context<Self>) {
        self.show_search = !self.show_search;
        if !self.show_search {
            self.query.clear();
        }
        let show_search = self.show_search;
        let now = RiseClock::monotonic();
        self.search_presence.update(cx, |presence, cx| {
            presence.set_in(show_search.then_some(()), spring(0.34, 0.86), now, cx);
            cx.notify();
        });
        cx.notify();
    }
}

#[derive(IntoElement)]
pub struct PinnedMessages {
    chat_id: Option<String>,
    topic_id: Option<String>,
}

impl PinnedMessages {
    pub fn new(chat_id: Option<String>, topic_id: Option<String>) -> Self {
        Self { chat_id, topic_id }
    }

    fn resolved_chat_id(&self, cx: &mut App) -> String {
        let raw = self.chat_id.clone().unwrap_or_default();
        let service = ChatServicesStore::shared(cx);
        service.read(cx).real_chat_id(&raw, cx).unwrap_or(raw)
    }

    fn chat(resolved_chat_id: &str, cx: &mut App) -> ChatInfo {
        let service = ChatServicesStore::shared(cx);
        let live = service.read(cx).chat_info(resolved_chat_id, cx);
        live.unwrap_or_else(|| ChatInfo::new(resolved_chat_id.to_owned()))
    }

    /// count shows up only once every page is loaded
    /// a partial count looked like the total and then grew
    fn count_title(resolved_chat_id: &str, topic_id: Option<&str>, cx: &mut App) -> String {
        let service = MessageServicesStore::shared(cx);
        let is_complete = service.update(cx, |service, cx| {
            service.has_pinned_messages_cache(resolved_chat_id, cx)
                && !service.pinned_messages_pagination(resolved_chat_id, cx).has_more
        });
        if !is_complete {
            return tr("pinned_count_title");
        }
        let count = service.update(cx, |service, cx| service.pinned_messages_for_topic(resolved_chat_id, topic_id, cx).len());
        format!("{} {count}", tr("pinned_count_title"))
    }
}

impl RenderOnce for PinnedMessages {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let resolved_chat_id = self.resolved_chat_id(cx);
        let chat = Self::chat(&resolved_chat_id, cx);
        let state = window.use_keyed_state(
            SharedString::from(format!("pinned-messages-state-{resolved_chat_id}")),
            cx,
            |_, cx| PinnedMessagesState::new(cx),
        );
        let count_title = Self::count_title(&resolved_chat_id, self.topic_id.as_deref(), cx);
        let (show_search, query, search_presence) = {
            let screen = state.read(cx);
            (screen.show_search, screen.query.clone(), screen.search_presence.clone())
        };

        let binding_state = state.clone();
        let below_header = search_presence.read(cx).render_transitioned(RiseClock::monotonic(), window, move |_| {
            div().w_full().px(px(10.0)).pt(px(4.0)).pb(px(6.0)).child(
                SearchFieldUi::new(
                    "pinned-messages-search",
                    Binding::entity(
                        binding_state,
                        |screen: &PinnedMessagesState| screen.query.clone(),
                        |screen, query, cx| {
                            screen.query = query;
                            cx.notify();
                        },
                    ),
                )
                .placeholder(SharedString::from(tr("pinned_search_placeholder")))
                .uses_inline_text_field(true)
                .auto_focus(true),
            )
        });

        let nav_store = env::<NavigationStoreKey>(cx);
        let toggle_state = state.clone();
        let toggle_reserve = env::<PanelToggleHeaderReserveKey>(cx);
        ListShellUi::new()
            .center_slot(MainText::new(count_title).px(16.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary).line_limit(1))
            .use_safe_zone(true)
            .shows_back_button(true)
            .on_back(move |_, cx| {
                if let Some(nav_store) = &nav_store {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            })
            .right_jsx(
                div().pr(toggle_reserve).child(
                    PageHeaderIconButton::new(
                        "pinned-messages-search-toggle",
                        if show_search { "xmark" } else { "magnifyingglass" },
                        move |_, cx| toggle_state.update(cx, |state, cx| state.toggle_search(cx)),
                    )
                    .font_size(16.0),
                ),
            )
            .fills_viewport_background(false)
            .when_some(below_header, |shell, below_header| shell.below_header(below_header))
            .content(PinnedMessagesChat { resolved_chat_id, chat, query, topic_id: self.topic_id, state })
    }
}

#[derive(IntoElement)]
struct PinnedMessagesChat {
    resolved_chat_id: String,
    chat: ChatInfo,
    query: String,
    topic_id: Option<String>,
    state: Entity<PinnedMessagesState>,
}

impl PinnedMessagesChat {
    fn can_manage(chat: &ChatInfo, cx: &mut App) -> bool {
        if chat.r#type == ChatType::Direct || chat.r#type == ChatType::Favourites {
            return true;
        }
        ChatModerationRights::holds(chat, ChatPerm::PIN_MESSAGES, None, chat.can_pin_messages, cx)
    }

    fn render_source_version(&self, presentation_revision: u64) -> i64 {
        let mut hasher = DefaultHasher::new();
        presentation_revision.hash(&mut hasher);
        self.query.hash(&mut hasher);
        self.chat.id.hash(&mut hasher);
        self.chat.r#type.hash(&mut hasher);
        self.chat.allowed_reactions.hash(&mut hasher);
        self.chat.role.hash(&mut hasher);
        self.chat.forward_in_chat_enabled.hash(&mut hasher);
        self.chat.can_pin_messages.hash(&mut hasher);
        self.chat.can_manage_chat.hash(&mut hasher);
        RiseFormat::app_locale().hash(&mut hasher);
        hasher.finish() as i64
    }

    fn unpin_all_pill(actions: PinnedMessagesChatActions, theme: &AppTheme) -> impl IntoElement {
        LiquidGlassButton::new("pinned-messages-unpin-all", LiquidGlassButtonShape::Capsule)
            .horizontal_padding(16.0)
            .vertical_padding(11.0)
            .on_press(move |_, cx| {
                let _ = impact(ImpactStyle::Light);
                let actions = actions.clone();
                NativeConfirmAlert::present_delete(
                    &tr("pinned_unpin_all_confirm"),
                    "",
                    Some(tr("pinned_unpin_all")),
                    None,
                    false,
                    None,
                    None,
                    move |cx| actions.unpin_all(cx),
                    cx,
                );
            })
            .child(
                hstack(8.0)
                    .items_center()
                    .children(IconUi::sized("pin.slash.fill", px(15.0), theme.semantic.error_100, true))
                    .child(
                        MainText::new(tr("pinned_unpin_all"))
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.semantic.error_100),
                    ),
            )
    }
}

impl RenderOnce for PinnedMessagesChat {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let safe_insets = list_shell_safe_area_insets(cx);
        let actions = PinnedMessagesChatActions {
            resolved_chat_id: self.resolved_chat_id.clone(),
            chat: Arc::new(self.chat.clone()),
            topic_id: self.topic_id.clone(),
        };

        let pinned = actions.pinned(cx);
        let visible_messages = PinnedMessagesChatActions::filtered(&pinned, &self.query, cx);
        let service = MessageServicesStore::shared(cx);
        let chat_id = self.resolved_chat_id.clone();
        let (pagination, presentation_revision, has_initial_data, load_error) = service.update(cx, |service, cx| {
            (
                service.pinned_messages_pagination(&chat_id, cx),
                service.pinned_messages_presentation_revision(&chat_id, cx),
                service.has_pinned_messages_cache(&chat_id, cx),
                service.pinned_messages_error(&chat_id, cx),
            )
        });
        let source_version = self.render_source_version(presentation_revision);
        let can_manage = Self::can_manage(&self.chat, cx);
        let is_empty = visible_messages.is_empty();

        let task_chat_id = chat_id.clone();
        use_task(
            SharedString::from(format!("pinned-messages-task-{chat_id}")),
            chat_id.clone(),
            window,
            cx,
            async move |cx| {
                let _ = cx.update(|_, cx| {
                    MessageInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.load_initial_pinned_messages(&task_chat_id, cx));
                });
            },
        );

        let (query_state, query_actions) = (self.state.clone(), actions.clone());
        let query_text = self.query.clone();
        on_change(
            SharedString::from(format!("pinned-messages-query-{chat_id}")),
            self.query.clone(),
            false,
            window,
            cx,
            move |_, _, _, cx| {
                query_state.update(cx, |state, cx| {
                    state.scroll_request =
                        Some(RiseMessageScrollRequest::new(RiseMessageScrollDestination::Latest, false));
                    cx.notify();
                });
                query_actions.load_more_for_search_if_needed(&query_text, cx);
            },
        );
        let (revision_actions, revision_query) = (actions.clone(), self.query.clone());
        on_change(
            SharedString::from(format!("pinned-messages-revision-{chat_id}")),
            presentation_revision,
            false,
            window,
            cx,
            move |_, _, _, cx| revision_actions.load_more_for_search_if_needed(&revision_query, cx),
        );

        let messages = visible_messages.into_iter().map(Arc::new).collect::<Vec<_>>();
        let mut list = RiseMessageListView::new(
            SharedString::from(format!("pinned-messages-list-{chat_id}")),
            messages.clone(),
            actions.chat.clone(),
            format!("pinned-{chat_id}"),
            (if can_manage { 72.0 } else { 8.0 }) + safe_insets.bottom,
            source_version,
        );
        list.initial_scroll_position = RiseMessageInitialScrollPosition::Bottom;
        list.scroll_request = self.state.read(cx).scroll_request.clone();
        list.persists_scroll_position = false;
        list.gallery_paginates = false;
        list.initial_data_resolved = has_initial_data;
        list.can_load_older = pagination.has_more;
        list.is_scroll_enabled = HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_none();
        let reaction_actions = actions.clone();
        list.on_reaction = Some(Rc::new(move |message_id: &str, emoji: &str, _: &mut Window, cx: &mut App| {
            reaction_actions.toggle_reaction(message_id, emoji, cx);
        }));
        let action_actions = actions.clone();
        list.on_action = Some(Rc::new(move |message_id: &str, action_id: &str, _: &mut Window, cx: &mut App| {
            action_actions.handle_action(message_id, action_id, cx);
        }));
        let older_chat_id = chat_id.clone();
        list.on_load_older = Some(Rc::new(move |_: Option<&str>, _: &mut Window, cx: &mut App| {
            MessageInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.load_more_pinned_messages(&older_chat_id, cx));
        }));

        let overlay = if is_empty && has_initial_data {
            if load_error.is_some() {
                let retry_chat_id = chat_id.clone();
                Some(
                    PinnedMessagesErrorState {
                        theme: theme.clone(),
                        on_retry: Rc::new(move |cx: &mut App| {
                            MessageInteractionsStore::shared(cx)
                                .update(cx, |store, cx| store.load_initial_pinned_messages(&retry_chat_id, cx));
                        }),
                    }
                    .into_any_element(),
                )
            } else {
                Some(PinnedMessagesEmptyState { theme: theme.clone() }.into_any_element())
            }
        } else if !has_initial_data {
            Some(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(LoaderUi::new().size(LoaderUiSize::Large).color(Some(theme.primary._100)))
                    .into_any_element(),
            )
        } else {
            None
        };

        let pill = (can_manage && !pinned.is_empty()).then(|| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(safe_insets.bottom + 12.0))
                .flex()
                .justify_center()
                .child(div().occlude().child(Self::unpin_all_pill(actions.clone(), &theme)))
        });

        div()
            .relative()
            .size_full()
            .child(
                div().absolute().inset_0().child(
                    ChatBackdropLayers::new(ChatBackdropLayout::FullBleed)
                        .chat_id(Some(self.chat.id.clone()))
                        .chat_wallpaper(self.chat.wallpaper.clone()),
                ),
            )
            .child(
                div().absolute().inset_0().child(
                    env_scope::<ChatGalleryMessagesKey>(Some(Arc::new(messages)))
                        .child(env_scope::<ChatGalleryPaginatesKey>(false).child(list)),
                ),
            )
            .children(overlay)
            .children(pill)
    }
}

/// what the list closures can see of the view
#[derive(Clone)]
struct PinnedMessagesChatActions {
    resolved_chat_id: String,
    chat: Arc<ChatInfo>,
    topic_id: Option<String>,
}

impl PinnedMessagesChatActions {
    fn pinned(&self, cx: &mut App) -> Vec<GetMessageMessage> {
        MessageServicesStore::shared(cx).update(cx, |service, cx| {
            service.pinned_messages_for_topic(&self.resolved_chat_id, self.topic_id.as_deref(), cx)
        })
    }

    fn normalized_query(query: &str) -> String {
        trimming_whitespaces_and_newlines(query).to_lowercase()
    }

    fn filtered(pinned: &[GetMessageMessage], query: &str, cx: &mut App) -> Vec<GetMessageMessage> {
        let query = Self::normalized_query(query);
        if query.is_empty() {
            return pinned.to_vec();
        }
        pinned
            .iter()
            .filter(|message| {
                MessagePreview::snippet(message, true, cx).to_lowercase().contains(&query)
                    || message.sender_name.to_lowercase().contains(&query)
            })
            .cloned()
            .collect()
    }

    fn is_real(&self) -> bool {
        !self.resolved_chat_id.is_empty()
    }

    fn load_more_for_search_if_needed(&self, query: &str, cx: &mut App) {
        if Self::normalized_query(query).is_empty() && self.topic_id.is_none() {
            return;
        }
        let pinned = self.pinned(cx);
        if !Self::filtered(&pinned, query, cx).is_empty() {
            return;
        }
        let has_more = MessageServicesStore::shared(cx)
            .update(cx, |service, cx| service.pinned_messages_pagination(&self.resolved_chat_id, cx).has_more);
        if !has_more {
            return;
        }
        MessageInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.load_more_pinned_messages(&self.resolved_chat_id, cx));
    }

    fn pinned_message(&self, message_id: &str, cx: &mut App) -> Option<GetMessageMessage> {
        if !self.is_real() {
            return None;
        }
        self.pinned(cx).into_iter().find(|message| message.matches_message_id(message_id))
    }

    fn toggle_reaction(&self, message_id: &str, emoji: &str, cx: &mut App) {
        let Some(message) = self.pinned_message(message_id, cx) else {
            return;
        };
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.toggle_reaction(
                &self.resolved_chat_id,
                message_id,
                emoji,
                message.topic_id.as_deref(),
                None,
                Some(&message),
                cx,
            );
        });
    }

    fn handle_action(&self, message_id: &str, action_id: &str, cx: &mut App) {
        let Some(message) = self.pinned_message(message_id, cx) else {
            return;
        };
        let chat = self.chat.as_ref();
        let is_own = MessageList::is_own_bubble(&message, chat, cx);
        let perms = MessagePermissions::new(&message, chat, is_own);

        match action_id {
            ChatMessageAction::COPY => {
                UIPasteboard::general().set_string(perms.copy_text(), cx);
                let _ = notification(NotificationKind::Success);
            }
            ChatMessageAction::COPY_LINK => {
                let Some(link) = perms.message_link(cx) else {
                    return;
                };
                UIPasteboard::general().set_string(link.absolute_string(), cx);
                Notifier::copied(cx);
            }
            ChatMessageAction::FORWARD => {
                if !perms.can_forward(cx) {
                    return;
                }
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_messages(vec![message.clone()], cx));
            }
            ChatMessageAction::SEND_SPARKS => {
                let server_id = message.sai_server_id();
                let target = SparksTarget::Message {
                    chat_id: chat.id.clone(),
                    message_id: if server_id.is_empty() { message.id.clone() } else { server_id },
                };
                SparksInteractionsStore::shared(cx).update(cx, |store, cx| store.open_send_sheet(target, false, None, cx));
            }
            ChatMessageAction::SAVE_TO_GALLERY => {
                let items = perms.gallery_savable_items().into_iter().cloned().collect();
                MediaSaveCoordinator::shared(cx)
                    .update(cx, |coordinator, cx| coordinator.save_to_gallery(message.clone(), items, cx));
            }
            ChatMessageAction::SAVE => {
                let items = perms.file_savable_items().into_iter().cloned().collect();
                MediaSaveCoordinator::shared(cx)
                    .update(cx, |coordinator, cx| coordinator.save_to_files(message.clone(), items, cx));
            }
            ChatMessageAction::ADD_TO_LOCAL_PLAYLIST => {
                if !perms.can_add_mp3_to_local_playlist() {
                    return;
                }
                MessageMusicImportPresenter::shared(cx).update(cx, |presenter, cx| presenter.present(&message, cx));
            }
            ChatMessageAction::UNPIN => {
                let _ = impact(ImpactStyle::Light);
                MessageInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.unpin_message(message_id, &self.resolved_chat_id, None, None, cx));
            }
            ChatMessageAction::DELETE | ChatMessageAction::DELETE_EVERYONE | ChatMessageAction::DELETE_FROM_FAVOURITES => {
                self.delete_message(message_id, true, cx);
            }
            ChatMessageAction::DELETE_ME => self.delete_message(message_id, false, cx),
            ChatMessageAction::REPORT => {
                let target = ReportTarget::Message {
                    chat_id: self.resolved_chat_id.clone(),
                    message_id: message.sai_server_id(),
                };
                ReportSheetStore::shared(cx).update(cx, |store, cx| store.present(target, None, None, cx));
            }
            _ => {}
        }
    }

    fn delete_message(&self, message_id: &str, for_everyone: bool, cx: &mut App) {
        let ids = [message_id.to_owned()];
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.delete_messages(&ids, &self.resolved_chat_id, None, None, for_everyone, cx);
        });
    }

    fn unpin_all(&self, cx: &mut App) {
        if !self.is_real() {
            return;
        }
        if self.topic_id.is_none() {
            MessageInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.unpin_all_messages(&self.resolved_chat_id, cx));
            return;
        }
        for message in self.pinned(cx) {
            MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.unpin_message(&message.id, &self.resolved_chat_id, message.topic_id.as_deref(), None, cx);
            });
        }
    }
}

#[derive(IntoElement)]
struct PinnedMessagesEmptyState {
    theme: AppTheme,
}

impl RenderOnce for PinnedMessagesEmptyState {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        vstack(12.0)
            .absolute()
            .inset_0()
            .px(px(40.0))
            .items_center()
            .justify_center()
            .children(IconUi::sized("pin.slash", px(34.0), self.theme.text.secondary, false))
            .child(MainText::new(tr("pinned_empty")).px(15.0).tac(TextAlignT::Center).color(self.theme.text.secondary))
    }
}

#[derive(IntoElement)]
struct PinnedMessagesErrorState {
    theme: AppTheme,
    on_retry: Rc<dyn Fn(&mut App)>,
}

impl RenderOnce for PinnedMessagesErrorState {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_retry = self.on_retry;
        vstack(14.0)
            .absolute()
            .inset_0()
            .items_center()
            .justify_center()
            .child(MainText::new(tr("error_fetching_data")).px(15.0).color(self.theme.text.secondary))
            .child(
                LiquidGlassButton::new("pinned-messages-retry", LiquidGlassButtonShape::Capsule)
                    .accessibility_label(Some(tr("common_retry")))
                    .on_press(move |_, cx| on_retry(cx))
                    .child(MainText::new(tr("common_retry")).px(14.0).color(self.theme.text.primary)),
            )
    }
}
