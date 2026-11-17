use std::rc::Rc;
use std::time::Duration;

use gpui::{App, ElementId, Entity, Hsla, SharedString, Window, container_query, relative, rgb};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::shared::try_read;
use rise_ui::swiftui::environment::{env, env_scope};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{TransitionModifiers, UnitPoint};
use rise_ui::switch_ui::SwitchUi;
use rise_ui::MainText;

use crate::app::router::navigation_store::{NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::guides::guide_facts::GuideFacts;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_sheet_hooks::ChatWallpaperSheetHooks;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperStore;
use crate::modules::chat::components::chat::bubble::message_content::ChatMessageViewportWidthKey;
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType, User};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageContentType};
use crate::modules::theme::stores::theme_wallpapers::theme_wallpapers_interactions::theme_wallpapers_interactions::ThemeWallpapersInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

/// who may pin a wallpaper for everyone, server has the same rule so this only picks which rows to show
pub enum ChatWallpaperPermissions {}

impl ChatWallpaperPermissions {
    pub fn can_set(chat: &ChatInfo, _cx: &App) -> bool {
        match chat.r#type {
            ChatType::Direct | ChatType::Favourites => !UserServicesStore::is_contact_join_virtual_chat_id(&chat.id),
            ChatType::Group | ChatType::Channel => ChatWallpaperSheetHooks::resolved_is_owner(chat),
        }
    }
}

const WALLPAPER_PICKER_PUSH_DELAY: Duration = Duration::from_millis(320);

/// per chat wallpaper sheet, the switch says if this chat shows me its wallpaper
/// what it writes depends on the mode
#[derive(IntoElement)]
pub struct ChatWallpaperSheet {
    chat_id: String,
    topic_id: Option<String>,
    initial_chat: ChatInfo,
}

impl ChatWallpaperSheet {
    pub fn new(chat_id: impl Into<String>, initial_chat: ChatInfo) -> Self {
        Self {
            chat_id: chat_id.into(),
            topic_id: None,
            initial_chat,
        }
    }

    pub fn topic_id(mut self, topic_id: Option<String>) -> Self {
        self.topic_id = topic_id;
        self
    }

    fn set_shows_chat(shows: bool, chat_id: &str, cx: &mut App) {
        let _ = selection_changed();
        ThemeWallpapersInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.set_shows_chat_wallpaper(shows, chat_id, cx));
    }
}

impl RenderOnce for ChatWallpaperSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let chat_id = self.chat_id;
        let resolved_topic_id = self.topic_id.filter(|topic_id| !topic_id.is_empty());
        let wallpaper_store = ChatWallpaperStore::shared(cx);
        let _watch: Entity<gpui::Subscription> = window.use_keyed_state(
            ElementId::from(SharedString::from(format!("chat-wallpaper-sheet-watch-{chat_id}"))),
            cx,
            |_, cx| {
                let store = ChatWallpaperStore::shared(cx);
                cx.observe(&store, |_, _, cx| cx.notify())
            },
        );
        let shows_chat = wallpaper_store.read(cx).shows_chat_wallpaper(&chat_id, cx);
        let chat_services = ChatServicesStore::shared(cx);
        let chat = chat_services.read(cx).chat_info(&chat_id, cx).unwrap_or(self.initial_chat);
        let has_wallpaper = match resolved_topic_id.as_deref() {
            None => chat.wallpaper.is_some(),
            Some(topic_id) => chat_services
                .update(cx, |services, cx| services.topic_from_any_cache(&chat_id, topic_id, cx))
                .is_some_and(|topic| topic.wallpaper.is_some()),
        };
        let can_set = ChatWallpaperPermissions::can_set(&chat, cx);

        let sheet_id = SharedString::from(format!("chat-wallpaper-sheet-{chat_id}"));
        let remove_error: Entity<Option<String>> = window.use_keyed_state(ElementId::from(sheet_id.clone()), cx, |_, _| None);
        let remove_error_text = remove_error.read(cx).clone().filter(|message| !message.is_empty());

        let switch_chat_id = chat_id.clone();
        let row_chat_id = chat_id.clone();
        let mut items = vec![
            GroupedBtn::new("wallpaper_scope", tr("wallpaper_chat_show_this"))
                .icon("photo.on.rectangle")
                .icon_background(SettingsIconPalette::customization())
                .trailing_slot(SwitchUi::new(
                    "wallpaper-scope-switch",
                    Binding::new(move |_| shows_chat, move |shows, _, cx| Self::set_shows_chat(shows, &switch_chat_id, cx)),
                ))
                .callback(move |_, cx| Self::set_shows_chat(!shows_chat, &row_chat_id, cx)),
        ];
        if can_set {
            let dismiss_action = dismiss(cx);
            let navigation_store = env::<NavigationStoreKey>(cx);
            let picker_chat_id = chat_id.clone();
            let picker_topic_id = resolved_topic_id.clone();
            items.push(
                GroupedBtn::new(
                    "wallpaper_change",
                    if has_wallpaper { tr("wallpaper_chat_change") } else { tr("wallpaper_chat_pick_title") },
                )
                .icon("photo.on.rectangle.angled")
                .icon_background(Hsla::from(rgb(0x0083E0)))
                .guide_anchor(RiseGuideAnchor::WallpaperSheetChange)
                .callback(move |window, cx| {
                    dismiss_action.call_as_function(window, cx);
                    let Some(navigation_store) = navigation_store.clone() else {
                        return;
                    };
                    let route = RootRoute::ChatWallpaperPicker {
                        chat_id: Some(picker_chat_id.clone()),
                        topic_id: picker_topic_id.clone(),
                    };
                    cx.spawn(async move |cx| {
                        cx.background_executor().timer(WALLPAPER_PICKER_PUSH_DELAY).await;
                        cx.update(|cx| {
                            navigation_store
                                .update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
                        });
                    })
                    .detach();
                }),
            );
            if has_wallpaper {
                let remove_chat_id = chat_id.clone();
                let remove_topic_id = resolved_topic_id.clone();
                let remove_error = remove_error.clone();
                items.push(
                    GroupedBtn::new("wallpaper_remove", tr("wallpaper_chat_remove"))
                        .icon("trash")
                        .icon_background(SettingsIconPalette::delete_account())
                        .destructive(true)
                        .show_chevron(false)
                        .callback(move |_, cx| {
                            remove_error.update(cx, |error, cx| {
                                *error = None;
                                cx.notify();
                            });
                            let result_error = remove_error.clone();
                            ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                                store.clear_chat_wallpaper(
                                    &remove_chat_id,
                                    remove_topic_id.as_deref(),
                                    Some(Box::new(move |ok: bool, message: Option<String>, cx: &mut App| {
                                        if !ok {
                                            result_error.update(cx, |error, cx| {
                                                *error = Some(message.unwrap_or_else(|| tr("wallpaper_chat_apply_failed")));
                                                cx.notify();
                                            });
                                        }
                                    })),
                                    cx,
                                );
                            });
                        }),
                );
            }
        }

        let title = if resolved_topic_id.is_none() { tr("wallpaper_chat_sheet_title") } else { tr("wallpaper_topic_title") };
        let primary = theme.primary._100;

        let content = div()
            .id(sheet_id.clone())
            .w_full()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .pt(px(8.0))
            .pb(px(24.0))
            .child(GroupedBtns::new("wallpaper-sheet-rows", items))
            .children(remove_error_text.map(|message| {
                div()
                    .w_full()
                    .px(px(28.0))
                    .child(MainText::new(message).px(13.0).color(primary).number_of_lines(3))
            }));

        let close_action = dismiss(cx);
        div()
            .w_full()
            .flex()
            .flex_col()
            .bg(theme.bg._100)
            .child(
                SheetHeaderBar::new(SharedString::from(format!("{sheet_id}-header")), title, move |window, cx| {
                    close_action.call_as_function(window, cx);
                })
                .right_system_image(None),
            )
            .child(content)
            .on_appear(SharedString::from(format!("{sheet_id}-appear")), |_, cx| {
                ThemeWallpapersInteractionsStore::shared(cx).update(cx, |store, cx| store.load(false, cx));
                GuideFacts::set(RiseGuideSignal::ChatWallpaperSheetOpen, true, cx);
            })
            .on_disappear(SharedString::from(format!("{sheet_id}-disappear")), |_, cx| {
                GuideFacts::set(RiseGuideSignal::ChatWallpaperSheetOpen, false, cx);
            })
    }
}

/// the two sample messages in every wallpaper preview, drawn by real message rows
pub enum WallpaperPreviewMessages {}

impl WallpaperPreviewMessages {
    pub const PEER_ID: &'static str = "wallpaper-preview-peer";

    pub fn chat(real: Option<ChatInfo>) -> ChatInfo {
        if let Some(real) = real {
            return real;
        }
        let mut stand = ChatInfo::new("wallpaper-preview".to_owned());
        stand.title = tr("wallpapers_title");
        stand.r#type = ChatType::Direct;
        stand.participant = User {
            id: Self::PEER_ID.to_owned(),
            ..User::default()
        };
        stand
    }

    pub fn messages(cx: &App) -> Vec<GetMessageMessage> {
        let now = RiseClock::now().time_interval_since_1970();
        let mut incoming = GetMessageMessage::new("wallpaper-preview-1".to_owned());
        incoming.sender_id = Self::PEER_ID.to_owned();
        incoming.text = tr("wallpaper_preview_hi");
        incoming.content_type = MessageContentType::Text;
        incoming.created_at = now - 60.0;
        incoming.is_read = true;

        let mut outgoing = GetMessageMessage::new("wallpaper-preview-2".to_owned());
        outgoing.sender_id =
            try_read::<AuthServicesStore>(cx).and_then(AuthServicesStore::effective_user_id_for_wire).unwrap_or_default();
        outgoing.text = tr("wallpaper_preview_how_are_you");
        outgoing.content_type = MessageContentType::Text;
        outgoing.created_at = now;
        outgoing.is_read = true;
        vec![incoming, outgoing]
    }
}

/// real message rows inside a tile or preview
/// scale 1 is life size, smaller lays rows out wider then shrinks them
#[derive(IntoElement, Default)]
pub struct WallpaperTileMessages {
    chat: Option<ChatInfo>,
    scale: Option<f32>,
}

impl WallpaperTileMessages {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn chat(mut self, chat: Option<ChatInfo>) -> Self {
        self.chat = chat;
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = Some(scale);
        self
    }
}

impl RenderOnce for WallpaperTileMessages {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let scale = self.scale.unwrap_or(1.0).clamp(0.05, 1.0);
        div().size_full().relative().overflow_hidden().child(
            div()
                .absolute()
                .left_0()
                .bottom_0()
                .w(relative(1.0 / scale))
                .h(relative(1.0 / scale))
                .child({
                    let chat = self.chat;
                    container_query(move |size, _, _| {
                        env_scope::<ChatMessageViewportWidthKey>(f32::from(size.width).max(1.0)).child(
                            div()
                                .size_full()
                                .flex()
                                .flex_col()
                                .justify_end()
                                .child(WallpaperSampleBubbles::new().chat(chat)),
                        )
                    })
                })
                .scale_effect_anchored(scale, UnitPoint::BOTTOM_LEADING),
        )
    }
}

/// real message rows over a wallpaper for the full screen editor
#[derive(IntoElement, Default)]
pub struct WallpaperSampleBubbles {
    chat: Option<ChatInfo>,
}

impl WallpaperSampleBubbles {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn chat(mut self, chat: Option<ChatInfo>) -> Self {
        self.chat = chat;
        self
    }
}

impl RenderOnce for WallpaperSampleBubbles {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        MessageList::new(
            "wallpaper-sample-bubbles",
            WallpaperPreviewMessages::messages(cx),
            WallpaperPreviewMessages::chat(self.chat),
        )
        .interactions_enabled(false)
        .allows_swipe_reply(false)
    }
}
