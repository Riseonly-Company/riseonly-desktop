use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, Entity, FontWeight, IntoElement, Pixels, Point, RenderOnce, SharedString, Window,
    black, div, prelude::*, px, white,
};
use rise_canon::rise_wire_methods;
use rise_i18n::tr;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::ThemeStore;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::skeleton_ui::SkeletonListUi;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::chat::components::chat::bubble::chat_audio_track_adapter::ChatAudioTrackAdapter;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_media_save_actions::ChatProfileMediaSaveActions;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_message_jump::ChatProfileMessageJump;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_pagination::ChatProfilePaginationPrefetchModifiers;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_section_list::ChatProfileSectionList;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::{
    ChatProfileFormatter, ChatProfileTabBottomSpacer, ChatProfileTabDataRender, ChatProfileTabScrollContainer,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_source::{
    ChatProfileMediaEntry, ChatProfileSource,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tab_window::ChatProfileTabHost;
use crate::modules::chat::shared::chat_audio_queue_continuation::ChatAudioQueueContinuationOrder;
use crate::modules::chat::shared::message_permissions::ChatMessageAction;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_player_state::{MusicPlayOptions, MusicPlayerState};
use crate::modules::music::widgets::bottomsheets::playlists::message_mp3_import_sheet::MessageMusicImportPresenter;

const CACHE_ID: &str = rise_wire_methods::message::GET_MUSIC_MESSAGES;
const ROW_HEIGHT: f32 = 68.5;

fn row_element_id(entry: &ChatProfileMediaEntry, part: &str) -> ElementId {
    ElementId::Name(format!("chat-profile-music-{part}-{}", entry.id).into())
}

struct ChatProfileMusicRow {
    entry: ChatProfileMediaEntry,
    chat_id: String,
    queue: Rc<Vec<ChatProfileMediaEntry>>,
    nav_store: Option<Entity<NavigationStore>>,
}

#[derive(IntoElement)]
pub(crate) struct ChatProfileMusicTab {
    chat_id: String,
}

impl ChatProfileMusicTab {
    pub(crate) fn new(chat_id: impl Into<String>) -> Self {
        Self { chat_id: chat_id.into() }
    }

    /// same track as in the message bubble, so the tab and the chat share one player state
    fn track(entry: &ChatProfileMediaEntry, chat_id: &str) -> TrackModel {
        ChatAudioTrackAdapter::track(&entry.message_id, Some(entry.message_id.as_str()), chat_id, &entry.item, "")
    }

    fn toggle(row: &ChatProfileMusicRow, cx: &mut App) {
        let track = Self::track(&row.entry, &row.chat_id);
        let player = MusicPlayerState::shared(cx);
        if ChatAudioTrackAdapter::is_same_audio(player.read(cx).current_track.as_ref(), &track) {
            player.update(cx, |player, cx| player.toggle_play_pause(cx));
            return;
        }
        let options = MusicPlayOptions {
            queue: row.queue.iter().map(|entry| Self::track(entry, &row.chat_id)).collect(),
            scope_id: Some(format!("chat:{}", row.chat_id)),
            record_play_on_success: false,
            ..MusicPlayOptions::default()
        };
        player.update(cx, |player, cx| player.play(track, options, cx));
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.continue_chat_audio_queue(&row.chat_id, ChatAudioQueueContinuationOrder::TabOrder, cx);
        });
    }

    fn row(
        entry: &ChatProfileMediaEntry,
        chat_id: &str,
        queue: &Rc<Vec<ChatProfileMediaEntry>>,
        entry_index_by_id: &HashMap<String, i64>,
        nav_store: Option<&Entity<NavigationStore>>,
        cx: &mut App,
    ) -> AnyElement {
        let separator = ThemeStore::shared(cx).read(cx).current_theme().border._200.opacity(0.4);
        let row = Rc::new(ChatProfileMusicRow {
            entry: entry.clone(),
            chat_id: chat_id.to_owned(),
            queue: queue.clone(),
            nav_store: nav_store.cloned(),
        });
        let actions = Self::audio_hold_actions(entry, chat_id, cx);
        let content_row = row.clone();
        let action_row = row.clone();
        let tap_row = row;
        let hold = HoldContextMenuContainer::new(
            row_element_id(entry, "hold"),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, cx: &mut App| Self::row_content(&content_row, cx)),
        )
        .show_emoji_list(false)
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            Self::handle_audio_hold(&action_row, &id, cx);
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| Self::toggle(&tap_row, cx)));

        let scope = chat_id.to_owned();
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(hold.chat_profile_pagination_prefetch(
                row_element_id(entry, "prefetch"),
                entry_index_by_id.get(&entry.id).copied().unwrap_or(0),
                queue.len(),
                CACHE_ID,
                chat_id,
                None,
                move |_, cx| {
                    MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_music_tab(&scope, cx));
                },
                cx,
            ))
            .child(div().w_full().pl(px(76.0)).child(div().w_full().h(px(0.5)).bg(separator)))
            .into_any_element()
    }

    fn row_content(row: &Rc<ChatProfileMusicRow>, cx: &mut App) -> AnyElement {
        let entry = &row.entry;
        let item = &entry.item;
        let (title, artist) = ChatAudioTrackAdapter::title_and_artist(item, "");
        let track = Self::track(entry, &row.chat_id);
        let (is_current, is_playing, is_loading) = {
            let player = MusicPlayerState::shared(cx);
            let player = player.read(cx);
            let is_current = ChatAudioTrackAdapter::is_same_audio(player.current_track.as_ref(), &track);
            (is_current, is_current && player.is_playing, is_current && player.is_preparing_playback)
        };
        let (primary, text_primary, text_secondary, page) = {
            let theme = ThemeStore::shared(cx).read(cx).current_theme();
            (theme.primary._100, theme.text.primary, theme.text.secondary, theme.bg._100)
        };

        let cover = match item.thumbnail_url.as_ref().filter(|cover| !cover.is_empty()) {
            Some(cover) => div()
                .size(px(48.0))
                .child(
                    CleverImageUi::new(cover.clone())
                        .id(row_element_id(entry, "cover"))
                        .content_mode(ContentMode::Fill)
                        .corner_radius(8.0)
                        .persist_to_disk(true)
                        .maximum_decode_pixel_dimension(144_i64),
                )
                .into_any_element(),
            None => div()
                .size(px(48.0))
                .rounded(px(8.0))
                .bg(primary.opacity(0.16))
                .flex()
                .items_center()
                .justify_center()
                .children(IconUi::sized("music.note", px(22.0), primary, false))
                .into_any_element(),
        };

        let glyph = if is_loading {
            progress_view().tint(white()).control_size(ControlSize::Small).into_any_element()
        } else {
            div()
                .relative()
                .left(px(if is_playing { 0.0 } else { 1.0 }))
                .flex()
                .children(IconUi::sized(if is_playing { "pause.fill" } else { "play.fill" }, px(14.0), white(), true))
                .into_any_element()
        };
        let button = div().child(
            plain(row_element_id(entry, "play"))
                .child(zstack(Alignment::Center).size(px(26.0)).rounded_full().bg(black().opacity(0.45)).child(glyph)),
        );

        let duration = item.duration.map(ChatProfileFormatter::duration);
        hstack(12.0_f32)
            .w_full()
            .padding_edges(EdgeSet::HORIZONTAL, 16.0)
            .padding_edges(EdgeSet::VERTICAL, 10.0)
            .child(
                div()
                    .relative()
                    .flex_none()
                    .size(px(48.0))
                    .child(
                        div()
                            .size_full()
                            .rounded(px(10.0))
                            .overflow_hidden()
                            .child(zstack(Alignment::Center).size_full().child(cover).child(button)),
                    )
                    .child(
                        div()
                            .absolute()
                            .right(px(-4.0))
                            .bottom(px(-4.0))
                            .size(px(18.0))
                            .child(
                                UserLogoUi::new(row_element_id(entry, "sender"))
                                    .source(entry.sender_logo.clone().map(SharedString::from))
                                    .name(Some(SharedString::from(entry.sender_name.clone())))
                                    .user_id(Some(SharedString::from(entry.sender_id.clone())))
                                    .size(18.0)
                                    .show_border(false)
                                    .show_story_ring(false)
                                    .open_story_on_press(false),
                            )
                            .child(div().absolute().inset_0().rounded_full().border(px(1.5)).border_color(page)),
                    ),
            )
            .child(
                vstack(2.0_f32)
                    .items_start()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        MainText::new(title)
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(if is_current { primary } else { text_primary })
                            .number_of_lines(1),
                    )
                    .child(
                        hstack(6.0_f32)
                            .w_full()
                            .child(MainText::new(artist).px(12.0).color(text_secondary).number_of_lines(1))
                            .when_some(duration, |meta, duration| {
                                meta.child(div().flex_none().size(px(2.0)).rounded_full().bg(text_secondary.opacity(0.4)))
                                    .child(MainText::new(duration).px(12.0).color(text_secondary))
                            })
                            .child(spacer()),
                    ),
            )
            .child(
                vstack(2.0_f32)
                    .items_end()
                    .flex_none()
                    .max_w(px(110.0))
                    .when(!entry.sender_name.is_empty(), |column| {
                        column.child(
                            MainText::new(entry.sender_name.clone())
                                .px(11.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(text_secondary)
                                .number_of_lines(1),
                        )
                    })
                    .child(
                        MainText::new(ChatProfileFormatter::relative_date(entry.created_at)).px(11.0).color(text_secondary),
                    ),
            )
            .into_any_element()
    }

    fn handle_audio_hold(row: &ChatProfileMusicRow, id: &str, cx: &mut App) {
        let entry = &row.entry;
        let chat_id = row.chat_id.as_str();
        if ChatProfileMediaSaveActions::handle(id, entry, chat_id, cx) {
            return;
        }
        if let Some(for_everyone) = ChatProfileSource::deletes_for_everyone(id) {
            let ids = [entry.message_id.clone()];
            MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.delete_messages(&ids, chat_id, None, None, for_everyone, cx);
            });
            return;
        }
        match id {
            ChatMessageAction::ADD_TO_LOCAL_PLAYLIST => {
                MessageMusicImportPresenter::shared(cx).update(cx, |presenter, cx| {
                    presenter.present_item(
                        &entry.item,
                        &entry.message_id,
                        chat_id,
                        "",
                        !entry.has_protected_content,
                        cx,
                    );
                });
            }
            "copy" => {
                if entry.has_protected_content {
                    return;
                }
                let text = if entry.item.url.is_empty() {
                    RiseonlyPublicLink::Chat { id: chat_id.to_owned() }
                        .web_url(AppEnvironment::current())
                        .map(|link| link.absolute_string().to_string())
                } else {
                    Some(entry.item.url.clone())
                };
                if let Some(text) = text {
                    UIPasteboard::general().set_string(text.as_str(), cx);
                }
            }
            "goto" => {
                if let Some(nav_store) = &row.nav_store {
                    ChatProfileMessageJump::open(chat_id, None, &entry.message_id, nav_store, cx);
                }
            }
            "forward" => {
                let Some(message) = ChatProfileSource::forwardable_message(entry, chat_id, cx) else {
                    return;
                };
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_messages(vec![message], cx));
            }
            _ => {}
        }
    }

    fn audio_hold_actions(entry: &ChatProfileMediaEntry, chat_id: &str, cx: &mut App) -> Vec<HoldContextMenuAction> {
        let mut actions: Vec<HoldContextMenuAction> = Vec::new();
        if !entry.has_protected_content {
            actions.push(HoldContextMenuAction::new("forward", tr("ctx_forward")).system_image("arrowshape.turn.up.right"));
            actions.push(HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"));
        }
        if entry.item.is_importable_mp3() && !entry.has_protected_content {
            actions.insert(
                actions.len().min(1),
                HoldContextMenuAction::new(ChatMessageAction::ADD_TO_LOCAL_PLAYLIST, tr("ctx_add_mp3_to_local_playlist"))
                    .system_image("text.badge.plus"),
            );
        }
        actions.extend(ChatProfileMediaSaveActions::actions(entry));
        actions.push(HoldContextMenuAction::new("goto", tr("ctx_go_to_message")).system_image("arrow.turn.up.left"));
        if let Some(delete) = ChatProfileSource::delete_action(&entry.message_id, &entry.sender_id, chat_id, cx) {
            actions.push(delete);
        }
        actions
    }
}

impl ChatProfileMusicTab {
    fn body(chat_id: &str, cx: &mut App) -> AnyElement {
        let chat_id = chat_id.to_string();
        let nav_store = env::<NavigationStoreKey>(cx);
        let entries = ChatProfileSource::audio_items(&chat_id, cx);
        let services = MessageServicesStore::shared(cx);
        let sections =
            services.update(cx, |services, cx| services.message_tab_media_sections(CACHE_ID, &chat_id, &entries, cx));
        let entry_index_by_id = Rc::new(services.update(cx, |services, cx| {
            services.message_tab_entry_index_by_id(
                CACHE_ID,
                &chat_id,
                || entries.iter().map(|entry| entry.id.clone()).collect(),
                cx,
            )
        }));
        let is_empty = entries.is_empty();
        let queue = Rc::new(entries);

        let retry_chat_id = chat_id.clone();
        let content_chat_id = chat_id.clone();
        let loading_id = ElementId::Name(format!("chat-profile-music-loading-{chat_id}").into());
        let scroll_id = ElementId::Name(format!("chat-profile-music-scroll-{chat_id}").into());
        let list_id = ElementId::Name(format!("chat-profile-music-list-{chat_id}").into());
        ChatProfileTabDataRender::new(
            CACHE_ID,
            chat_id,
            is_empty,
            "music.note",
            tr("chat_profile_empty_music"),
            move |_, cx| {
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_initial_music_tab(&retry_chat_id, None, true, cx);
                });
            },
            move |_, _| {
                ChatProfileTabScrollContainer::new(scroll_id, move |_, _| {
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .child(ChatProfileSectionList::new(list_id, sections, ROW_HEIGHT, move |entry, _, cx| {
                            Self::row(entry, &content_chat_id, &queue, &entry_index_by_id, nav_store.as_ref(), cx)
                        }))
                        .child(ChatProfileTabBottomSpacer)
                })
            },
            move |_, _| {
                ChatProfileTabScrollContainer::new(loading_id.clone(), |_, _| SkeletonListUi {
                    rows: 8,
                    ..SkeletonListUi::default()
                })
                .scroll_enabled(false)
                .shows_indicators(false)
            },
        )
        .into_any_element()
    }
}

impl RenderOnce for ChatProfileMusicTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let chat_id = self.chat_id;
        ChatProfileTabHost::hosted(
            SharedString::from(format!("chat-profile-music-host-{chat_id}")),
            window,
            cx,
            |cx| {
                let player = MusicPlayerState::shared(cx);
                vec![cx.observe(&player, |_, _, cx| cx.notify())]
            },
            move |_, cx| Self::body(&chat_id, cx),
        )
    }
}
