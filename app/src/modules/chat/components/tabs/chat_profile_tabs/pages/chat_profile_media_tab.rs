use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{
    AnyView, App, Bounds, ElementId, Entity, FontWeight, PinchEvent, Pixels, Point, SharedString, StyleRefinement,
    Subscription, TouchPhase, Window, black, white,
};
use rise_core::foundation::foundation_date::TimeInterval;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{tr, tr_plural};
use rise_platform::haptics;
use rise_platform::pasteboard::UIPasteboard;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::prelude::*;
use rise_ui::window_screen_frame_reader::{MediaCellScreenFrameRegistry, WindowScreenFrameReader};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::media_viewer::media_viewer_open::MediaViewerOpen;
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerGalleryScope, MediaViewerPresentationHandlers, media_viewer_presentation,
};
use crate::core::media_viewer::media_viewer_types::{
    MediaViewerChrome, MediaViewerDeleteMode, MediaViewerFrameKeys, MediaViewerGalleryItem, MediaViewerHeroSkin,
};
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::profile_grid_date_scrubber::ProfileGridDateScrubberLayer;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::chat::components::chat::media_viewer::chat_media_gallery_builder::ChatMediaGalleryBuilder;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_grid_return_positioner::{
    ChatProfileGridLayoutProbe, ChatProfileGridReturnPositioner,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_media_grid_prefs::ChatProfileMediaGridPrefs;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_media_save_actions::ChatProfileMediaSaveActions;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_message_jump::ChatProfileMessageJump;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_pagination::ChatProfilePaginationPrefetchModifiers;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui_hooks::ChatProfileSharedUiHooks;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::{
    ChatProfileFormatter, ChatProfileTabBottomSpacer, ChatProfileTabDataRender, ChatProfileTabScrollContainer,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_source::{
    ChatProfileMediaEntry, ChatProfileSource,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tab_window::{
    ChatProfileSquareGrid, ChatProfileTabHost, chat_profile_grid_cell_lifted,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageMediaItem, MessageMediaKind};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::chat::stores::message::message_services::message_tabs_services::MessageTabPaginationState;
use crate::modules::post::components::post::post_skeleton::PostGridSkeleton;
use crate::modules::user::components::profile_content::profile_scroll_coordinator::ProfileScrollCoordinator;
use crate::modules::user::components::profile_content::shared_element_return_scroller_environment::SharedElementReturnScrollerKey;

const MEDIA_CACHE_ID: &str = "getMediaMessages";
const GRID_SPACING: f32 = 2.0;
const GRID_PADDING: f32 = 2.0;
const CELL_DECODE_DIMENSION: i64 = 512;
const GIF_BADGE_TEXT: &str = "GIF";
const CELL_HOVER_OPACITY: f32 = 0.85;

struct ChatProfileMediaPrefsObserver {
    _prefs: Subscription,
}

/// pinch scale the current column count was reached at, next step is measured from it
struct ChatProfileMediaPinch {
    base: f32,
    magnification: f32,
}

#[derive(Clone)]
struct ChatProfileMediaTabParts {
    chat_id: String,
    topic_id: Option<String>,
    nav_store: Option<Entity<NavigationStore>>,
    positioner: Rc<ChatProfileGridReturnPositioner>,
}

#[derive(IntoElement)]
pub(crate) struct ChatProfileMediaTab {
    chat_id: String,
    topic_id: Option<String>,
}

impl ChatProfileMediaTab {
    pub(crate) fn new(chat_id: impl Into<String>) -> Self {
        Self { chat_id: chat_id.into(), topic_id: None }
    }

    pub(crate) fn topic_id(mut self, topic_id: Option<String>) -> Self {
        self.topic_id = topic_id;
        self
    }

    /// the grid after the show photos and show videos filters
    fn shown_entries(chat_id: &str, topic_id: Option<&str>, cx: &mut App) -> Vec<ChatProfileMediaEntry> {
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, None);
        let all = ChatProfileSource::media_items(chat_id, topic_id, cx);
        let prefs = ChatProfileMediaGridPrefs::shared(cx);
        let prefs = prefs.read(cx);
        if prefs.shows_photos(&scope) && prefs.shows_videos(&scope) {
            return all;
        }
        all.into_iter().filter(|entry| prefs.shows(entry.item.kind, &scope)).collect()
    }

    fn handle_pinch(pinch: &Entity<ChatProfileMediaPinch>, event: &PinchEvent, cx: &mut App) {
        if matches!(event.phase, TouchPhase::Ended) {
            pinch.update(cx, |pinch, _| {
                pinch.base = 1.0;
                pinch.magnification = 1.0;
            });
            return;
        }
        let (relative, magnification) = pinch.update(cx, |pinch, _| {
            pinch.magnification *= 1.0 + event.delta;
            (pinch.magnification / pinch.base.max(0.01), pinch.magnification)
        });
        let step = if relative >= 1.25 {
            -1
        } else if relative <= 0.8 {
            1
        } else {
            return;
        };
        let prefs = ChatProfileMediaGridPrefs::shared(cx);
        let before = prefs.read(cx).columns();
        prefs.update(cx, |prefs, cx| prefs.set_columns(before + step, cx));
        pinch.update(cx, |pinch, _| pinch.base = magnification);
        if prefs.read(cx).columns() != before {
            let _ = haptics::selection_changed();
        }
    }

    fn chat(chat_id: &str, cx: &mut App) -> ChatInfo {
        ChatServicesStore::shared(cx)
            .update(cx, |services, cx| services.chat_info(chat_id, cx))
            .unwrap_or_else(|| {
                let mut chat = ChatInfo::new(chat_id.to_owned());
                chat.title = chat_id.to_owned();
                chat
            })
    }

    /// every message and chat store value the body and its cells read
    fn shown(chat_id: &str, topic_id: Option<&str>, cx: &mut App) -> ChatProfileMediaShown {
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, None);
        let entries = ChatProfileSource::media_items(chat_id, topic_id, cx);
        let services = MessageServicesStore::shared(cx);
        let (pagination, revision) = services.update(cx, |services, cx| {
            (
                services.tab_pagination(MEDIA_CACHE_ID, &scope, cx),
                services.message_tab_presentation_revision(MEDIA_CACHE_ID, &scope, cx),
            )
        });
        let status = ChatProfileSharedUiHooks::message_tab_render_status_or_default(
            MEDIA_CACHE_ID,
            &scope,
            !entries.is_empty(),
            cx,
        );
        let error = ChatProfileSharedUiHooks::message_tab_error_or_default(MEDIA_CACHE_ID, &scope, cx);
        ChatProfileMediaShown { entries, pagination, revision, status, error }
    }

    fn body(chat_id: &str, topic_id: Option<&str>, window: &mut Window, cx: &mut App) -> AnyElement {
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, None);
        let id_base = format!("chat-profile-media-{scope}");
        let positioner = window
            .use_keyed_state(SharedString::from(format!("{id_base}-positioner")), cx, |_, _| {
                ChatProfileGridReturnPositioner::new()
            })
            .read(cx)
            .clone();
        let return_scroller = env::<SharedElementReturnScrollerKey>(cx);
        let configure_positioner = positioner.clone();
        on_change(
            SharedString::from(format!("{id_base}-return-scroller")),
            return_scroller,
            true,
            window,
            cx,
            move |_, scroller, _, _| configure_positioner.configure(scroller.clone()),
        );

        let parts = ChatProfileMediaTabParts {
            chat_id: chat_id.to_owned(),
            topic_id: topic_id.map(str::to_owned),
            nav_store: env::<NavigationStoreKey>(cx),
            positioner,
        };
        let _ = window.use_keyed_state(SharedString::from(format!("{id_base}-prefs")), cx, |_, cx| {
            let prefs = ChatProfileMediaGridPrefs::shared(cx);
            ChatProfileMediaPrefsObserver { _prefs: cx.observe(&prefs, |_, _, cx| cx.notify()) }
        });
        let pinch = window.use_keyed_state(SharedString::from(format!("{id_base}-pinch")), cx, |_, _| {
            ChatProfileMediaPinch { base: 1.0, magnification: 1.0 }
        });
        let columns = ChatProfileMediaGridPrefs::shared(cx).read(cx).columns();
        let entries = Self::shown_entries(chat_id, topic_id, cx);
        let all_entries = ChatProfileSource::media_items(chat_id, topic_id, cx);
        let all_count = all_entries.len();
        let is_filtered = entries.len() != all_count;
        let entry_index_by_id = if is_filtered {
            let mut indexes = HashMap::new();
            for (index, entry) in entries.iter().enumerate() {
                indexes.entry(entry.id.clone()).or_insert(index as i64);
            }
            indexes
        } else {
            MessageServicesStore::shared(cx).update(cx, |services, cx| {
                services.message_tab_entry_index_by_id(
                    MEDIA_CACHE_ID,
                    &scope,
                    || entries.iter().map(|entry| entry.id.clone()).collect(),
                    cx,
                )
            })
        };
        let album_message_ids = Self::album_message_ids(&all_entries);
        let is_empty = all_count == 0;
        let retry_parts = parts.clone();
        let scroll_id = SharedString::from(id_base.clone());
        let grid_id = SharedString::from(format!("{id_base}-grid"));
        let loading_id = SharedString::from(format!("{id_base}-loading"));
        let grid_scope = scope.clone();

        ChatProfileTabDataRender::new(
            MEDIA_CACHE_ID,
            scope,
            is_empty,
            "photo.on.rectangle",
            tr("chat_profile_empty_media"),
            move |_, cx| {
                let parts = retry_parts.clone();
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_initial_media_tab(&parts.chat_id, parts.topic_id.as_deref(), None, true, cx);
                });
            },
            move |_, _| {
                ChatProfileTabScrollContainer::new(scroll_id, move |_, _| {
                    Self::grid(
                        parts,
                        entries,
                        entry_index_by_id,
                        album_message_ids,
                        grid_scope,
                        grid_id,
                        columns,
                        all_count,
                        pinch,
                    )
                })
            },
            move |_, _| {
                ChatProfileTabScrollContainer::new(loading_id.clone(), move |_, _| {
                    div()
                        .w_full()
                        .px(px(GRID_PADDING))
                        .child(PostGridSkeleton::default().columns(columns).spacing(GRID_SPACING))
                })
                .scroll_enabled(false)
                .shows_indicators(false)
            },
        )
        .into_any_element()
    }

    fn grid(
        parts: ChatProfileMediaTabParts,
        entries: Vec<ChatProfileMediaEntry>,
        entry_index_by_id: HashMap<String, i64>,
        album_message_ids: HashSet<String>,
        scope: String,
        grid_id: SharedString,
        columns: i64,
        all_count: usize,
        pinch: Entity<ChatProfileMediaPinch>,
    ) -> impl IntoElement {
        let entries = Rc::new(entries);
        let item_count = entries.len();
        let sentinel_parts = parts.clone();
        let sentinel_id = SharedString::from(format!("{grid_id}-filtered-{all_count}"));
        let cell_parts = parts.clone();
        let cell = Rc::new(move |index: usize, side: f32, _: &mut Window, cx: &mut App| {
            let entry = entries[index].clone();
            let is_album = album_message_ids.contains(&entry.message_id);
            let position = entry_index_by_id.get(&entry.id).copied().unwrap_or(0);
            let load_parts = cell_parts.clone();
            let prefetch_id = SharedString::from(format!("chat-profile-media-prefetch-{}", entry.id));
            Self::cell(&cell_parts, entry, is_album, side, cx)
                .chat_profile_pagination_prefetch(
                    prefetch_id,
                    position,
                    item_count,
                    MEDIA_CACHE_ID,
                    &scope,
                    None,
                    move |_, cx| {
                        let parts = load_parts.clone();
                        MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                            store.load_more_media_tab(&parts.chat_id, parts.topic_id.as_deref(), cx);
                        });
                    },
                    cx,
                )
        });
        let probe_positioner = parts.positioner;
        div()
            .w_full()
            .child(
                div()
                    .w_full()
                    .min_h(px(if item_count == 0 { 120.0 } else { 0.0 }))
                    .on_pinch(move |event: &PinchEvent, _, cx| Self::handle_pinch(&pinch, event, cx))
                    .child(
                        ChatProfileSquareGrid::new(grid_id, item_count, columns, GRID_SPACING, GRID_PADDING, cell)
                            .layout_probe(Rc::new(move |width: f32, _: &mut Window, _: &mut App| {
                                ChatProfileGridLayoutProbe::new(probe_positioner.clone(), width)
                                    .columns(columns.max(1) as usize)
                                    .into_any_element()
                            })),
                    ),
            )
            .when(item_count == 0, |grid| {
                grid.child(div().w_full().h(px(1.0)).on_appear(sentinel_id, move |_, cx| {
                    let parts = sentinel_parts.clone();
                    MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.load_more_media_tab(&parts.chat_id, parts.topic_id.as_deref(), cx);
                    });
                }))
            })
            .child(ChatProfileTabBottomSpacer)
    }

    fn flyback_target(
        parts: &ChatProfileMediaTabParts,
        registry_id: &str,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        let entries = Self::shown_entries(&parts.chat_id, parts.topic_id.as_deref(), cx);
        let index = entries.iter().position(|entry| MediaViewerFrameKeys::profile_grid(&entry.id) == registry_id)?;
        parts.positioner.prepare_flyback(index as i64, cx)
    }

    /// messages with more than one entry in the grid
    /// forward and delete hit the whole message so the cell says so
    fn album_message_ids(entries: &[ChatProfileMediaEntry]) -> HashSet<String> {
        let mut seen = HashSet::new();
        let mut albums = HashSet::new();
        for entry in entries {
            if !seen.insert(entry.message_id.as_str()) {
                albums.insert(entry.message_id.clone());
            }
        }
        albums
    }

    fn cell(
        parts: &ChatProfileMediaTabParts,
        entry: ChatProfileMediaEntry,
        is_album: bool,
        side: f32,
        cx: &mut App,
    ) -> HoldContextMenuContainer {
        let registry_id = MediaViewerFrameKeys::profile_grid(&entry.id);
        let is_lifted = chat_profile_grid_cell_lifted(&registry_id, cx);
        let actions = Self::media_entry_hold_actions(&entry, is_album, &parts.chat_id, cx);
        let entry = Rc::new(entry);
        let content_entry = entry.clone();
        let action_entry = entry.clone();
        let action_parts = parts.clone();
        let tap_entry = entry.clone();
        let tap_parts = parts.clone();

        HoldContextMenuContainer::new(
            ElementId::Name(format!("chat-profile-media-hold-{}", entry.id).into()),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let props = ChatProfileMediaCellProps {
                    item: content_entry.item.clone(),
                    side,
                    is_lifted,
                };
                let cell_id = ElementId::Name(format!("chat-profile-media-cell-{}", content_entry.id).into());
                let image_id = SharedString::from(format!("chat-profile-media-image-{}", content_entry.id));
                div()
                    .relative()
                    .w(px(side))
                    .h(px(side))
                    .child(div().absolute().inset_0().child(WindowScreenFrameReader::new(registry_id.clone())))
                    .child(ChatProfileMediaCellView::element(cell_id, image_id, props, window, cx))
                    .into_any_element()
            }),
        )
        .show_emoji_list(false)
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            Self::handle_media_entry_action(&action_parts, &action_entry, &id, cx);
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            Self::open_media_viewer(&tap_parts, &tap_entry, cx);
        }))
    }

    fn open_media_viewer(parts: &ChatProfileMediaTabParts, entry: &ChatProfileMediaEntry, cx: &mut App) {
        let chat = Self::chat(&parts.chat_id, cx);
        let entries = ChatProfileSource::media_items(&parts.chat_id, parts.topic_id.as_deref(), cx);
        let items = ChatMediaGalleryBuilder::items_profile_entries(&chat, &entries, cx);
        let registry_id = MediaViewerFrameKeys::profile_grid(&entry.id);
        let Some(open_index) = ChatMediaGalleryBuilder::open_index(&items, &registry_id) else { return };

        let skin = MediaViewerHeroSkin::new(entry.item.display_source(), false);
        let frame = MediaCellScreenFrameRegistry::frame(&registry_id).unwrap_or_default();
        let presenter = media_viewer_presentation(cx);
        let anchor_id = ChatMediaGalleryBuilder::window_identity_profile_entry(entry);
        let seed = ChatMediaGalleryBuilder::window_seed_profile_entries(&entries);

        let delete_chat_id = parts.chat_id.clone();
        let forward_chat_id = parts.chat_id.clone();
        let jump_parts = parts.clone();
        let jump_entries = entries.clone();
        let flyback_parts = parts.clone();
        let gallery_chat_id = parts.chat_id.clone();
        let gallery_topic_id = parts.topic_id.clone();
        let handlers = MediaViewerPresentationHandlers {
            on_delete: Some(Rc::new(
                move |item: &MediaViewerGalleryItem, mode: MediaViewerDeleteMode, cx: &mut App| {
                    let ids = [item.message_id.clone()];
                    MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.delete_messages(
                            &ids,
                            &delete_chat_id,
                            None,
                            None,
                            mode == MediaViewerDeleteMode::ForEveryone,
                            cx,
                        );
                    });
                },
            )),
            on_forward: Some(Rc::new(move |item: &MediaViewerGalleryItem, cx: &mut App| {
                let Some(source) = entries.iter().find(|entry| entry.message_id == item.message_id) else {
                    return;
                };
                let Some(forwardable) = ChatProfileSource::forwardable_message(source, &forward_chat_id, cx) else {
                    return;
                };
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_messages(vec![forwardable], cx));
            })),
            on_show_in_context: Some(Rc::new(move |item: &MediaViewerGalleryItem, cx: &mut App| {
                let topic_id = jump_entries
                    .iter()
                    .find(|entry| entry.message_id == item.message_id)
                    .and_then(|entry| entry.topic_id.clone());
                Self::jump(&jump_parts, &item.message_id, topic_id.as_deref(), cx);
            })),
            flyback_target_provider: Some(Rc::new(move |registry_id: &str, cx: &mut App| {
                Self::flyback_target(&flyback_parts, registry_id, cx)
            })),
            ..MediaViewerPresentationHandlers::default()
        };

        cx.spawn(async move |cx| {
            MediaViewerOpen::present(
                presenter,
                items,
                open_index,
                frame,
                registry_id,
                skin,
                MediaViewerChrome::CHAT,
                MediaViewerGalleryScope::Paged,
                handlers,
                cx,
            )
            .await;
            let Some(anchor_id) = anchor_id else { return };
            cx.update(|cx| {
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.open_media_gallery(
                        Uuid::new_v4(),
                        &gallery_chat_id,
                        gallery_topic_id.as_deref(),
                        seed,
                        &anchor_id,
                        cx,
                    );
                });
            });
        })
        .detach();
    }

    fn media_entry_hold_actions(
        entry: &ChatProfileMediaEntry,
        is_album: bool,
        chat_id: &str,
        cx: &mut App,
    ) -> Vec<HoldContextMenuAction> {
        let mut items = Vec::new();
        if !entry.has_protected_content {
            items.push(
                HoldContextMenuAction::new(
                    "forward",
                    if is_album { tr("ctx_forward_album") } else { tr("ctx_forward") },
                )
                .system_image("arrowshape.turn.up.right"),
            );
            items.push(HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"));
            items.extend(ChatProfileMediaSaveActions::actions(entry));
        }
        items.push(HoldContextMenuAction::new("goto", tr("ctx_go_to_message")).system_image("arrow.turn.up.left"));
        if let Some(delete) = ChatProfileSource::delete_action(&entry.message_id, &entry.sender_id, chat_id, cx) {
            items.push(if is_album {
                HoldContextMenuAction { title: tr("ctx_delete_album"), ..delete }
            } else {
                delete
            });
        }
        items
    }

    fn handle_media_entry_action(
        parts: &ChatProfileMediaTabParts,
        entry: &ChatProfileMediaEntry,
        id: &str,
        cx: &mut App,
    ) {
        if ChatProfileMediaSaveActions::handle(id, entry, &parts.chat_id, cx) {
            return;
        }
        if let Some(for_everyone) = ChatProfileSource::deletes_for_everyone(id) {
            let ids = [entry.message_id.clone()];
            let chat_id = parts.chat_id.clone();
            let topic_id = parts.topic_id.clone();
            let delete = move |cx: &mut App| {
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.delete_messages(&ids, &chat_id, topic_id.as_deref(), None, for_everyone, cx);
                });
            };
            let album_count = ChatProfileSource::media_items(&parts.chat_id, parts.topic_id.as_deref(), cx)
                .iter()
                .filter(|other| other.message_id == entry.message_id)
                .count();
            if album_count <= 1 {
                delete(cx);
                return;
            }
            NativeConfirmAlert::present_delete(
                &tr("media_viewer_delete_album_title"),
                &tr_plural("media_viewer_delete_album_message", album_count as i64),
                None,
                None,
                false,
                None,
                None,
                delete,
                cx,
            );
            return;
        }
        match id {
            "forward" => {
                let Some(message) = ChatProfileSource::forwardable_message(entry, &parts.chat_id, cx) else {
                    return;
                };
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_messages(vec![message], cx));
            }
            "copy" => {
                if entry.has_protected_content {
                    return;
                }
                let link = RiseonlyPublicLink::Chat { id: parts.chat_id.clone() }
                    .web_url(AppEnvironment::current())
                    .map(|url| url.absolute_string().to_owned());
                if let Some(value) = Self::item_url_string(&entry.item).or(link) {
                    UIPasteboard::general().set_string(&value, cx);
                }
            }
            "goto" => Self::jump(parts, &entry.message_id, entry.topic_id.as_deref(), cx),
            _ => {}
        }
    }

    /// profile own topic wins, a forum wide profile falls back to the topic of the message
    fn jump(parts: &ChatProfileMediaTabParts, message_id: &str, message_topic_id: Option<&str>, cx: &mut App) {
        if let Some(nav_store) = parts.nav_store.as_ref() {
            let topic_id = parts.topic_id.as_deref().or(message_topic_id);
            ChatProfileMessageJump::open(&parts.chat_id, topic_id, message_id, nav_store, cx);
        }
    }

    fn item_url_string(item: &MessageMediaItem) -> Option<String> {
        let url = trimming_whitespaces_and_newlines(&item.url);
        if url.is_empty() { None } else { Some(url.to_owned()) }
    }

    fn play_badge(duration: Option<TimeInterval>) -> impl IntoElement {
        hstack(3.0)
            .px(px(5.0))
            .py(px(2.0))
            .rounded_full()
            .bg(black().opacity(0.55))
            .children(IconUi::sized("play.fill", px(9.0), white(), true))
            .children(duration.map(|duration| {
                MainText::new(ChatProfileFormatter::duration(duration))
                    .px(11.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(white())
            }))
    }

    fn gif_badge() -> impl IntoElement {
        div()
            .px(px(5.0))
            .py(px(2.0))
            .rounded_full()
            .bg(black().opacity(0.55))
            .child(MainText::new(GIF_BADGE_TEXT).px(9.0).font_weight(FontWeight::BOLD).color(white()))
    }
}

impl RenderOnce for ChatProfileMediaTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let chat_id = self.chat_id;
        let topic_id = self.topic_id;
        let shown_chat_id = chat_id.clone();
        let shown_topic_id = topic_id.clone();
        ChatProfileTabHost::hosted_gated(
            SharedString::from(format!("chat-profile-media-host-{chat_id}-{}", topic_id.as_deref().unwrap_or_default())),
            window,
            cx,
            |_| Vec::new(),
            move |cx| Self::shown(&shown_chat_id, shown_topic_id.as_deref(), cx),
            move |window, cx| Self::body(&chat_id, topic_id.as_deref(), window, cx),
        )
    }
}

/// data the media tab shows, host rebuilds only when it changes
#[derive(PartialEq)]
struct ChatProfileMediaShown {
    entries: Vec<ChatProfileMediaEntry>,
    pagination: MessageTabPaginationState,
    revision: u64,
    status: SaiWsSliceStatus,
    error: Option<String>,
}

/// what one media cell draws, equal props keep the last frame
#[derive(Clone, PartialEq)]
struct ChatProfileMediaCellProps {
    item: MessageMediaItem,
    side: f32,
    is_lifted: bool,
}

/// one media cell as its own cached view, so a loaded image doesnt rebuild the whole grid
struct ChatProfileMediaCellView {
    image_id: SharedString,
    props: Option<ChatProfileMediaCellProps>,
}

struct ChatProfileMediaCellHolder {
    cell: Entity<ChatProfileMediaCellView>,
}

impl ChatProfileMediaCellView {
    fn element(
        id: ElementId,
        image_id: SharedString,
        props: ChatProfileMediaCellProps,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let style = StyleRefinement::default().w(px(props.side)).h(px(props.side));
        let host = window
            .use_keyed_state(id, cx, move |_, cx| ChatProfileMediaCellHolder {
                cell: cx.new(|_| Self { image_id, props: None }),
            })
            .read(cx)
            .cell
            .clone();
        let stale = host.update(cx, |host, _| host.set_props(props));
        if stale {
            AnyView::from(host).into_any_element()
        } else {
            host.cached(style).replay_when_moved().into_any_element()
        }
    }

    fn set_props(&mut self, props: ChatProfileMediaCellProps) -> bool {
        if self.props.as_ref() == Some(&props) {
            return false;
        }
        let drawn_before = self.props.is_some();
        self.props = Some(props);
        drawn_before
    }
}

impl Render for ChatProfileMediaCellView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let Some(props) = self.props.as_ref() else {
            return div().into_any_element();
        };
        let item = &props.item;
        div()
            .relative()
            .w(px(props.side))
            .h(px(props.side))
            .overflow_hidden()
            .opacity(if props.is_lifted { 0.0 } else { 1.0 })
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .hover(|style| style.opacity(CELL_HOVER_OPACITY))
                    .child(
                        CleverImageUi::new(item.display_source())
                            .id(self.image_id.clone())
                            .content_mode(ContentMode::Fill)
                            .corner_radius(0.0)
                            .persist_to_disk(true)
                            .maximum_decode_pixel_dimension(Some(CELL_DECODE_DIMENSION)),
                    ),
            )
            .children((item.kind == MessageMediaKind::Video).then(|| {
                div().absolute().left(px(6.0)).bottom(px(6.0)).child(ChatProfileMediaTab::play_badge(item.duration))
            }))
            .children((item.kind == MessageMediaKind::Animation).then(|| {
                div().absolute().left(px(6.0)).bottom(px(6.0)).child(ChatProfileMediaTab::gif_badge())
            }))
            .into_any_element()
    }
}

/// scrubber of the media tab, shares the driver with the stories grid but the grid here is square
#[derive(IntoElement)]
pub(crate) struct ChatProfileMediaDateScrubberLayer {
    id: ElementId,
    chat_id: String,
    topic_id: Option<String>,
    tab_key: i64,
    scroll_coordinator: Entity<ProfileScrollCoordinator>,
    top_inset: f32,
    bottom_inset: f32,
}

impl ChatProfileMediaDateScrubberLayer {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        chat_id: impl Into<String>,
        topic_id: Option<String>,
        tab_key: i64,
        scroll_coordinator: Entity<ProfileScrollCoordinator>,
        top_inset: f32,
        bottom_inset: f32,
    ) -> Self {
        Self {
            id: id.into(),
            chat_id: chat_id.into(),
            topic_id,
            tab_key,
            scroll_coordinator,
            top_inset,
            bottom_inset,
        }
    }
}

impl RenderOnce for ChatProfileMediaDateScrubberLayer {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let scope = MessageServicesStore::message_scope(&self.chat_id, self.topic_id.as_deref(), None);
        let prefs = ChatProfileMediaGridPrefs::shared(cx);
        let columns = prefs.read(cx).columns();
        let all = ChatProfileSource::media_items(&self.chat_id, self.topic_id.as_deref(), cx);
        let prefs = prefs.read(cx);
        let timestamps =
            all.iter().filter(|entry| prefs.shows(entry.item.kind, &scope)).map(|entry| entry.created_at).collect();
        ProfileGridDateScrubberLayer::new(
            self.id,
            timestamps,
            columns,
            1.0,
            GRID_SPACING,
            GRID_PADDING,
            self.tab_key,
            self.scroll_coordinator,
            self.top_inset,
            self.bottom_inset,
        )
    }
}
