use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, ElementId, Entity, IntoElement, ParentElement, RenderOnce, SharedString, Styled,
    Window, div, px,
};
use rise_chat::ChatType;
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactStyle, NotificationKind, impact, notification};
use rise_platform::pasteboard::UIPasteboard;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonCircle, SkeletonShimmerExt};
use rise_ui::swiftui::environment::{env, env_scope};
use rise_ui::swiftui::stacks::{hstack, spacer_min_w, vstack};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::media::media_save_coordinator::MediaSaveCoordinator;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::ChatMessageRenderEntryGroupFlags;
use crate::modules::chat::components::chat::media_viewer::chat_media_presentation::{
    ChatGalleryMessagesKey, ChatGalleryPaginatesKey,
};
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::components::chat::message_row::MessageRow;
use crate::modules::chat::components::chat::message_system_pill::MessageSystemPill;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_message_jump::ChatProfileMessageJump;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_pagination::ChatProfilePaginationPrefetchModifiers;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::{
    ChatProfileTabBottomSpacer, ChatProfileTabDataRender, ChatProfileTabScrollContainer,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_source::ChatProfileSource;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tab_window::{
    ChatProfileTabHost, ChatProfileWindowedRow, ChatProfileWindowedRows,
};
use crate::modules::chat::shared::chat_saved_message_presentation::{
    ChatMessageGrouping, ChatQuickReactionPolicy, RiseMessageSenderIdentity,
};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::shared::message_permissions::{ChatMessageAction, MessagePermissions};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::music::widgets::bottomsheets::playlists::message_mp3_import_sheet::MessageMusicImportPresenter;

const CACHE_ID: &str = "get_fav_messages";

const ESTIMATED_ROW_HEIGHT: f32 = 56.0;

#[derive(IntoElement)]
pub struct ChatProfileFavTab {
    chat_id: String,
}

impl ChatProfileFavTab {
    pub fn new(chat_id: impl Into<String>) -> Self {
        Self { chat_id: chat_id.into() }
    }

    fn favourites_chat(chat_id: &str, favourites_chat_id: Option<&str>, cx: &mut App) -> ChatInfo {
        let info = favourites_chat_id
            .and_then(|id| ChatServicesStore::shared(cx).update(cx, |services, cx| services.chat_info(id, cx)));
        info.unwrap_or_else(|| {
            let mut fallback = ChatInfo::new(favourites_chat_id.unwrap_or(chat_id).to_owned());
            fallback.r#type = ChatType::Favourites;
            fallback
        })
    }
}

impl RenderOnce for ChatProfileFavTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let chat_id = self.chat_id;
        ChatProfileTabHost::hosted(
            SharedString::from(format!("chat-profile-fav-host-{chat_id}")),
            window,
            cx,
            |_| Vec::new(),
            move |_, cx| ChatProfileFavTab::body(&chat_id, cx),
        )
    }
}

impl ChatProfileFavTab {
    fn body(chat_id: &str, cx: &mut App) -> AnyElement {
        let favourites_chat_id =
            ChatServicesStore::shared(cx).update(cx, |services, cx| services.favourites_chat_id(cx));
        let messages: Vec<GetMessageMessage> =
            ChatProfileSource::favourite_items(chat_id, cx).into_iter().map(|entry| entry.message).collect();
        let host = Self::favourites_chat(chat_id, favourites_chat_id.as_deref(), cx);
        let flags = ChatMessageGrouping::flags(&messages, &host, cx);
        let is_empty = messages.is_empty();
        let actions = Rc::new(ChatProfileFavActions {
            chat_id: chat_id.to_owned(),
            favourites_chat_id,
            nav_store: env::<NavigationStoreKey>(cx),
        });
        let rows = ChatProfileFavRows {
            id: SharedString::from(format!("chat-profile-fav-{chat_id}")),
            chat_id: chat_id.to_owned(),
            messages: Arc::new(messages.into_iter().map(Arc::new).collect()),
            flags,
            host: Arc::new(host),
            actions,
        };
        let retry_chat_id = chat_id.to_owned();
        let content_id = ElementId::Name(format!("chat-profile-fav-scroll-{chat_id}").into());
        let loading_id = ElementId::Name(format!("chat-profile-fav-loading-{chat_id}").into());
        ChatProfileTabDataRender::new(
            CACHE_ID,
            chat_id.to_owned(),
            is_empty,
            "bookmark.fill",
            tr("chat_profile_empty_fav"),
            move |_, cx| {
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_initial_fav_tab(&retry_chat_id, None, true, cx);
                });
            },
            move |_, _| {
                ChatProfileTabScrollContainer::new(content_id, move |_, _| {
                    vstack(0.0).w_full().child(rows).child(ChatProfileTabBottomSpacer)
                })
            },
            move |_, _| {
                ChatProfileTabScrollContainer::new(loading_id.clone(), |_, _| ChatProfileFavSkeleton)
                    .scroll_enabled(false)
                    .shows_indicators(false)
            },
        )
        .into_any_element()
    }
}

#[derive(IntoElement)]
struct ChatProfileFavRows {
    id: SharedString,
    chat_id: String,
    messages: Arc<Vec<Arc<GetMessageMessage>>>,
    flags: Vec<ChatMessageRenderEntryGroupFlags>,
    host: Arc<ChatInfo>,
    actions: Rc<ChatProfileFavActions>,
}

impl ChatProfileFavRows {
    fn child_id(&self, name: &str) -> ElementId {
        ElementId::Name(format!("{}-{name}", self.id).into())
    }

    fn row(
        &self,
        message: &Arc<GetMessageMessage>,
        group: ChatMessageRenderEntryGroupFlags,
        cx: &mut App,
    ) -> MessageRow {
        let is_own = MessageList::is_own_bubble(message, &self.host, cx);
        let raw = ChatQuickReactionPolicy::default_raw(cx);
        let quick_reaction = ChatQuickReactionPolicy::resolved(message, &self.host, &raw, cx);
        let can_double_tap = quick_reaction.is_some();

        let reaction_actions = self.actions.clone();
        let reaction_message = message.clone();
        let action_actions = self.actions.clone();
        let action_message = message.clone();
        let action_host = self.host.clone();
        let profile_actions = self.actions.clone();
        let profile_message = message.clone();
        let profile_host = self.host.clone();
        let tap_actions = self.actions.clone();
        let tap_message = message.clone();

        MessageRow::new(message.clone(), self.host.clone(), is_own, group.is_first, group.is_last, true)
            .id(self.child_id(&format!("row-{}", message.id)))
            .draws_inline_avatar(true)
            .allows_reply(false)
            .interactions_enabled(true)
            .allows_composer_actions(false)
            .leading_context_actions(self.actions.leading_actions(message))
            .on_reaction(Some(Rc::new(move |emoji: &str, _: &mut Window, cx: &mut App| {
                reaction_actions.toggle_reaction(&reaction_message, emoji, cx);
            })))
            .on_action(Some(Rc::new(move |action: &str, _: &mut Window, cx: &mut App| {
                action_actions.handle_action(&action_message, &action_host, action, cx);
            })))
            .on_open_sender_profile(Some(Rc::new(move |_: &mut Window, cx: &mut App| {
                profile_actions.open_sender_profile(&profile_message, &profile_host, cx);
            })))
            .can_double_tap(Rc::new(move || can_double_tap))
            .on_double_tap(Some(Rc::new(move |_: &mut Window, cx: &mut App| {
                let Some(reaction) = &quick_reaction else {
                    return;
                };
                let _ = impact(ImpactStyle::Light);
                tap_actions.toggle_reaction(&tap_message, reaction, cx);
            })))
    }

    fn row_cell(&self, index: usize, cx: &mut App) -> AnyElement {
        let message = self.messages[index].clone();
        let group = self
            .flags
            .get(index)
            .copied()
            .unwrap_or(ChatMessageRenderEntryGroupFlags { is_first: true, is_last: true });
        let top_padding = if group.is_first {
            MessageSizes::list_first_in_group_top_padding()
        } else {
            MessageSizes::list_continuation_top_padding()
        };
        let bottom_padding = if group.is_last { MessageSizes::list_last_in_group_bottom_padding() } else { 0.0 };
        let load_chat_id = self.chat_id.clone();
        let row = self.row(&message, group, cx);
        div().w_full().pt(px(top_padding)).pb(px(bottom_padding)).child(row).chat_profile_pagination_prefetch(
            self.child_id(&format!("prefetch-{}", message.id)),
            index as i64,
            self.messages.len(),
            CACHE_ID,
            &self.chat_id,
            None,
            move |_, cx| {
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_more_fav_tab(&load_chat_id, cx);
                });
            },
            cx,
        )
    }

    fn day_separator(messages: &[Arc<GetMessageMessage>], index: usize, calendar: &Calendar) -> Option<String> {
        let day = calendar.start_of_day(&Date::from_time_interval_since_1970(messages[index].created_at));
        if index > 0 {
            let previous_day =
                calendar.start_of_day(&Date::from_time_interval_since_1970(messages[index - 1].created_at));
            if previous_day == day {
                return None;
            }
        }
        Some(RiseFormat::day_label(&day, &Date::now(), calendar, &RiseFormat::app_locale(), false))
    }
}

impl RenderOnce for ChatProfileFavRows {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let stack_id = self.child_id("rows");
        let gallery_messages = self.messages.clone();
        let calendar = Calendar::current();
        let this = Rc::new(self);
        let mut rows = Vec::with_capacity(this.messages.len());
        for index in 0..this.messages.len() {
            let message_id = &this.messages[index].id;
            if let Some(label) = Self::day_separator(&this.messages, index, &calendar) {
                rows.push(ChatProfileWindowedRow::new(format!("{}-day-{message_id}", this.id), move |_, _| {
                    div().w_full().pt(px(6.0)).pb(px(2.0)).child(MessageSystemPill::new(label.clone()))
                }));
            }
            let row_this = this.clone();
            rows.push(ChatProfileWindowedRow::new(format!("{}-row-{message_id}", this.id), move |_, cx| {
                row_this.row_cell(index, cx)
            }));
        }
        env_scope::<ChatGalleryMessagesKey>(Some(gallery_messages)).child(
            env_scope::<ChatGalleryPaginatesKey>(false).child(
                div().w_full().pt(px(8.0)).child(ChatProfileWindowedRows::new(stack_id, ESTIMATED_ROW_HEIGHT, rows)),
            ),
        )
    }
}

/// what the row closures see of the view
struct ChatProfileFavActions {
    chat_id: String,
    favourites_chat_id: Option<String>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl ChatProfileFavActions {
    /// show in chat goes to the original in the viewed chat, found by forward_info
    /// without it the item shows disabled
    fn origin_message_id(message: &GetMessageMessage) -> Option<String> {
        let forward_info = message.forward_info.as_ref()?;
        [forward_info.from_message_id.as_ref(), forward_info.original_message_id.as_ref()]
            .into_iter()
            .flatten()
            .find(|id| {
                let trimmed = trimming_whitespaces_and_newlines(id);
                !trimmed.is_empty() && !GetSaiServerId::is_temp_id(Some(trimmed))
            })
            .cloned()
    }

    fn leading_actions(&self, message: &GetMessageMessage) -> Vec<HoldContextMenuAction> {
        let mut actions = vec![
            HoldContextMenuAction::new(ChatMessageAction::SHOW_IN_CHAT, tr("ctx_show_in_chat"))
                .system_image("bubble.left.and.text.bubble.right")
                .disabled(Self::origin_message_id(message).is_none()),
        ];
        if let Some(favourites_chat_id) = &self.favourites_chat_id
            && !favourites_chat_id.is_empty()
            && *favourites_chat_id != self.chat_id
        {
            actions.push(
                HoldContextMenuAction::new(ChatMessageAction::SHOW_IN_FAVOURITES, tr("ctx_show_in_favourites"))
                    .system_image("bookmark"),
            );
        }
        actions
    }

    fn open_in_source_chat(&self, message: &GetMessageMessage, cx: &mut App) {
        let (Some(origin_id), Some(nav_store)) = (Self::origin_message_id(message), &self.nav_store) else {
            return;
        };
        ChatProfileMessageJump::open(&self.chat_id, None, &origin_id, nav_store, cx);
    }

    fn open_in_favourites(&self, message: &GetMessageMessage, cx: &mut App) {
        let Some(nav_store) = &self.nav_store else {
            return;
        };
        let chat_id = self.favourites_chat_id.as_deref().unwrap_or(&self.chat_id);
        ChatProfileMessageJump::open(chat_id, None, &message.sai_server_id(), nav_store, cx);
    }

    fn open_sender_profile(&self, message: &GetMessageMessage, host: &ChatInfo, cx: &mut App) {
        let participant = RiseMessageSenderIdentity::participant(message, host, cx);
        if participant.id.is_empty() || participant.is_deleted {
            return;
        }
        let Some(nav_store) = &self.nav_store else {
            return;
        };
        let route = RootRoute::UserPage {
            user_id: Some(participant.id),
            tag: (!participant.tag.is_empty()).then_some(participant.tag),
        };
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::default(), cx));
    }

    fn toggle_reaction(&self, message: &GetMessageMessage, emoji: &str, cx: &mut App) {
        let Some(favourites_chat_id) = self.favourites_chat_id.as_deref().filter(|id| !id.is_empty()) else {
            return;
        };
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.toggle_reaction(favourites_chat_id, &message.id, emoji, None, None, Some(message), cx);
        });
    }

    fn handle_action(&self, message: &GetMessageMessage, host: &ChatInfo, action: &str, cx: &mut App) {
        let target_chat_id = self.favourites_chat_id.as_deref().unwrap_or(&self.chat_id);
        if target_chat_id.is_empty() {
            return;
        }
        let is_own = MessageList::is_own_bubble(message, host, cx);
        let perms = MessagePermissions::new(message, host, is_own);

        match action {
            ChatMessageAction::COPY => {
                if !perms.can_copy() {
                    return;
                }
                UIPasteboard::general().set_string(perms.copy_text(), cx);
                let _ = notification(NotificationKind::Success);
            }
            ChatMessageAction::FORWARD => {
                if !perms.can_forward(cx) {
                    return;
                }
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_messages(vec![message.clone()], cx));
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
                MessageMusicImportPresenter::shared(cx).update(cx, |presenter, cx| presenter.present(message, cx));
            }
            ChatMessageAction::PIN
            | ChatMessageAction::PIN_EVERYONE
            | ChatMessageAction::PIN_EVERYONE_NOTIFY
            | ChatMessageAction::PIN_EVERYONE_SILENT
            | ChatMessageAction::PIN_ME => {
                if !perms.can_pin(cx) {
                    return;
                }
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.pin_message(&message.id, target_chat_id, None, None, None, None, cx);
                });
            }
            ChatMessageAction::UNPIN => {
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.unpin_message(&message.id, target_chat_id, None, None, cx);
                });
            }
            ChatMessageAction::DELETE
            | ChatMessageAction::DELETE_EVERYONE
            | ChatMessageAction::DELETE_ME
            | ChatMessageAction::DELETE_MODERATE
            | ChatMessageAction::DELETE_FROM_FAVOURITES => {
                if !perms.can_delete(cx) && !perms.can_delete_for_me(cx) {
                    return;
                }
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.delete_messages(std::slice::from_ref(&message.id), target_chat_id, None, None, true, cx);
                });
            }
            ChatMessageAction::SHOW_IN_CHAT => self.open_in_source_chat(message, cx),
            ChatMessageAction::SHOW_IN_FAVOURITES => self.open_in_favourites(message, cx),
            _ => {}
        }
    }
}

#[derive(IntoElement)]
struct ChatProfileFavSkeleton;

impl ChatProfileFavSkeleton {
    const PLAN: [(bool, f32, f32); 8] = [
        (false, 184.0, 44.0),
        (false, 132.0, 44.0),
        (true, 156.0, 44.0),
        (false, 220.0, 66.0),
        (true, 118.0, 44.0),
        (false, 168.0, 44.0),
        (true, 196.0, 66.0),
        (false, 140.0, 44.0),
    ];
}

impl RenderOnce for ChatProfileFavSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        vstack(MessageSizes::list_row_spacing())
            .w_full()
            .pt(px(8.0))
            .children(Self::PLAN.iter().map(|(is_own, width, height)| {
                let is_own = *is_own;
                hstack(0.0)
                    .items_end()
                    .w_full()
                    .px(px(MessageSizes::row_side_inset()))
                    .pt(px(MessageSizes::list_first_in_group_top_padding()))
                    .pb(px(MessageSizes::list_last_in_group_bottom_padding()))
                    .children(is_own.then(|| spacer_min_w(MessageSizes::opposite_side_min_width())))
                    .children((!is_own).then(|| {
                        div()
                            .flex_none()
                            .ml(px((MessageSizes::avatar_leading_inset() - MessageSizes::row_side_inset()).max(0.0)))
                            .mr(px(MessageSizes::avatar_gap()))
                            .child(SkeletonCircle::new(MessageSizes::avatar_size()))
                    }))
                    .child(SkeletonBlock::new(*width, *height, MessageSizes::bubble_corner_radius()))
                    .children((!is_own).then(|| spacer_min_w(MessageSizes::opposite_side_min_width())))
            }))
            .skeleton_shimmer(None)
    }
}
