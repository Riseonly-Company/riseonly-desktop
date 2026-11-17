use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Bounds, Entity, Pixels, SharedString, Window};
use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;
use rise_ui::window_screen_frame_reader::{MediaCellScreenFrameRegistry, WindowScreenFrameReader};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::media_viewer::media_viewer_open::MediaViewerOpen;
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerGalleryScope, MediaViewerPresentationHandlers, media_viewer_presentation,
};
use crate::core::media_viewer::media_viewer_types::{
    MediaViewerChrome, MediaViewerDeleteMode, MediaViewerFrameKeys, MediaViewerGalleryItem, MediaViewerHeroSkin,
};
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::input_picker::gif_grid::GifGridCell;
use crate::modules::chat::components::chat::input_picker::sticker_hold_preview::{
    StickerHoldEntry, StickerHoldGridModifier, StickerHoldPreviewModifiers, StickerPreviewAction, StickerPreviewAsset,
};
use crate::modules::chat::components::chat::media_viewer::chat_media_gallery_builder::ChatMediaGalleryBuilder;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_grid_return_positioner::{
    ChatProfileGridLayoutProbe, ChatProfileGridReturnPositioner,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_media_save_actions::ChatProfileMediaSaveActions;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_message_jump::ChatProfileMessageJump;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_pagination::ChatProfilePaginationPrefetchModifiers;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::{
    ChatProfileTabBottomSpacer, ChatProfileTabDataRender, ChatProfileTabScrollContainer,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_source::{
    ChatProfileMediaEntry, ChatProfileSource,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tab_window::{
    ChatProfileSquareGrid, ChatProfileTabHost, chat_profile_grid_cell_lifted,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::post::components::post::post_skeleton::PostGridSkeleton;
use crate::modules::user::components::profile_content::shared_element_return_scroller_environment::SharedElementReturnScrollerKey;

const GIF_CACHE_ID: &str = "get_gif_messages";
const GRID_COLUMNS: i64 = 3;
const GRID_SPACING: f32 = 2.0;
const GRID_PADDING: f32 = 2.0;
const CELL_HOVER_OPACITY: f32 = 0.85;

#[derive(Clone)]
struct ChatProfileGifTabParts {
    chat_id: String,
    nav_store: Option<Entity<NavigationStore>>,
    positioner: Rc<ChatProfileGridReturnPositioner>,
}

#[derive(IntoElement)]
pub(crate) struct ChatProfileGifTab {
    chat_id: String,
}

impl ChatProfileGifTab {
    pub(crate) fn new(chat_id: impl Into<String>) -> Self {
        Self { chat_id: chat_id.into() }
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

    fn body(chat_id: &str, window: &mut Window, cx: &mut App) -> AnyElement {
        let id_base = format!("chat-profile-gif-{chat_id}");
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

        let parts = ChatProfileGifTabParts {
            chat_id: chat_id.to_owned(),
            nav_store: env::<NavigationStoreKey>(cx),
            positioner,
        };
        let entries = ChatProfileSource::gif_items(chat_id, cx);
        let entry_index_by_id = MessageServicesStore::shared(cx).update(cx, |services, cx| {
            services.message_tab_entry_index_by_id(
                GIF_CACHE_ID,
                chat_id,
                || entries.iter().map(|entry| entry.id.clone()).collect(),
                cx,
            )
        });
        let retry_chat_id = chat_id.to_owned();
        let scroll_id = SharedString::from(id_base.clone());
        let grid_id = SharedString::from(format!("{id_base}-grid"));
        let hold_id = SharedString::from(format!("{id_base}-hold"));
        let loading_id = SharedString::from(format!("{id_base}-loading"));
        let is_empty = entries.is_empty();

        ChatProfileTabDataRender::new(
            GIF_CACHE_ID,
            chat_id.to_owned(),
            is_empty,
            "photo.stack",
            tr("chat_profile_empty_gif"),
            move |_, cx| {
                MessageInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_initial_gif_tab(&retry_chat_id, None, true, cx));
            },
            move |_, _| {
                ChatProfileTabScrollContainer::new(scroll_id, move |_, _| {
                    Self::grid(parts, entries, entry_index_by_id, grid_id, hold_id)
                })
            },
            move |_, _| {
                ChatProfileTabScrollContainer::new(loading_id.clone(), |_, _| {
                    div()
                        .w_full()
                        .px(px(GRID_PADDING))
                        .pt(px(GRID_PADDING))
                        .child(PostGridSkeleton::default().columns(GRID_COLUMNS).spacing(GRID_SPACING))
                })
                .scroll_enabled(false)
                .shows_indicators(false)
            },
        )
        .into_any_element()
    }

    fn grid(
        parts: ChatProfileGifTabParts,
        entries: Vec<ChatProfileMediaEntry>,
        entry_index_by_id: HashMap<String, i64>,
        grid_id: SharedString,
        hold_id: SharedString,
    ) -> impl IntoElement {
        let entries = Rc::new(entries);
        let item_count = entries.len();
        let menus: Rc<RefCell<HashMap<String, Vec<StickerPreviewAction>>>> = Rc::new(RefCell::new(HashMap::new()));

        let cell_entries = entries.clone();
        let cell_parts = parts.clone();
        let cell_menus = menus.clone();
        let cell = Rc::new(move |index: usize, side: f32, _: &mut Window, cx: &mut App| {
            let entry = &cell_entries[index];
            cell_menus.borrow_mut().insert(entry.id.clone(), Self::gif_menu(&cell_parts, entry, cx));
            let load_chat_id = cell_parts.chat_id.clone();
            let position = entry_index_by_id.get(&entry.id).copied().unwrap_or(0);
            Self::cell(entry, side, cx)
                .chat_profile_pagination_prefetch(
                    SharedString::from(format!("chat-profile-gif-prefetch-{}", entry.id)),
                    position,
                    item_count,
                    GIF_CACHE_ID,
                    &cell_parts.chat_id,
                    None,
                    move |_, cx| {
                        MessageInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.load_more_gif_tab(&load_chat_id, cx));
                    },
                    cx,
                )
        });

        let probe_positioner = parts.positioner.clone();
        let grid = ChatProfileSquareGrid::new(grid_id, item_count, GRID_COLUMNS, GRID_SPACING, GRID_PADDING, cell)
            .top_padding(GRID_PADDING)
            .layout_probe(Rc::new(move |width: f32, _: &mut Window, _: &mut App| {
                ChatProfileGridLayoutProbe::new(probe_positioner.clone(), width).into_any_element()
            }));

        let primary_entries = entries;
        let primary_parts = parts;
        div()
            .w_full()
            .child(div().w_full().child(grid).sticker_hold_grid(
                StickerHoldGridModifier::new(
                    hold_id,
                    Rc::new(move |held: &StickerHoldEntry, _: &mut Window, cx: &mut App| {
                        if let Some(entry) = primary_entries.iter().find(|entry| entry.id == held.key) {
                            Self::open_media_viewer(&primary_parts, entry, cx);
                        }
                    }),
                )
                .menu_for(Rc::new(move |held: &StickerHoldEntry, _: &App| {
                    menus.borrow().get(&held.key).cloned().unwrap_or_default()
                })),
            ))
            .child(ChatProfileTabBottomSpacer)
    }

    fn flyback_target(
        chat_id: &str,
        positioner: &ChatProfileGridReturnPositioner,
        registry_id: &str,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        let entries = ChatProfileSource::gif_items(chat_id, cx);
        let index = entries.iter().position(|entry| MediaViewerFrameKeys::profile_grid(&entry.id) == registry_id)?;
        positioner.prepare_flyback(index as i64, cx)
    }

    fn cell(entry: &ChatProfileMediaEntry, side: f32, cx: &mut App) -> Div {
        let registry_id = MediaViewerFrameKeys::profile_grid(&entry.id);
        let is_lifted = chat_profile_grid_cell_lifted(&registry_id, cx);
        let url = Self::animation_url(entry);

        div().relative().w(px(side)).h(px(side)).opacity(if is_lifted { 0.0 } else { 1.0 }).child(
            div()
                .relative()
                .w(px(side))
                .h(px(side))
                .hover(|style| style.opacity(CELL_HOVER_OPACITY))
                .child(div().absolute().inset_0().child(WindowScreenFrameReader::new(registry_id)))
                .child(GifGridCell::new(SharedString::from(format!("chat-profile-gif-cell-{}", entry.id)), url.clone()))
                .sticker_hold_cell_frame(
                    entry.id.clone(),
                    StickerPreviewAsset::Gif(url, entry.item.aspect_ratio()),
                    None,
                ),
        )
    }

    fn gif_menu(
        parts: &ChatProfileGifTabParts,
        entry: &ChatProfileMediaEntry,
        cx: &mut App,
    ) -> Vec<StickerPreviewAction> {
        let jump_parts = parts.clone();
        let jump_message = entry.message_id.clone();
        let mut items = vec![StickerPreviewAction::new(
            "show_in_chat",
            tr("ctx_show_in_chat"),
            "bubble.left.and.bubble.right",
            Rc::new(move |_: &mut Window, cx: &mut App| {
                Self::jump(&jump_parts, &jump_message, cx);
            }),
        )];
        if let Some(forwardable) = ChatProfileSource::forwardable_message(entry, &parts.chat_id, cx) {
            items.push(StickerPreviewAction::new(
                "forward",
                tr("ctx_forward"),
                "arrowshape.turn.up.right",
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    let forwardable = forwardable.clone();
                    ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_messages(vec![forwardable], cx));
                }),
            ));
        }
        for action in ChatProfileMediaSaveActions::actions(entry) {
            let action_id = action.id.clone();
            let save_entry = entry.clone();
            let save_chat_id = parts.chat_id.clone();
            items.push(StickerPreviewAction::new(
                action.id,
                action.title,
                action.system_image.unwrap_or_else(|| "square.and.arrow.down".to_owned()),
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    ChatProfileMediaSaveActions::handle(&action_id, &save_entry, &save_chat_id, cx);
                }),
            ));
        }
        if AuthServicesStore::shared(cx).read(cx).is_current_user(&entry.sender_id) {
            let delete_ids = [entry.message_id.clone()];
            let delete_chat_id = parts.chat_id.clone();
            items.push(
                StickerPreviewAction::new(
                    "delete",
                    tr("ctx_delete"),
                    "trash",
                    Rc::new(move |_: &mut Window, cx: &mut App| {
                        MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                            store.delete_messages(&delete_ids, &delete_chat_id, None, None, false, cx);
                        });
                    }),
                )
                .destructive(true),
            );
        }
        items
    }

    fn jump(parts: &ChatProfileGifTabParts, message_id: &str, cx: &mut App) {
        if let Some(nav_store) = parts.nav_store.as_ref() {
            ChatProfileMessageJump::open(&parts.chat_id, None, message_id, nav_store, cx);
        }
    }

    fn animation_url(entry: &ChatProfileMediaEntry) -> String {
        let full = rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines(&entry.item.url);
        if full.is_empty() { entry.item.display_source() } else { full.to_owned() }
    }

    fn open_media_viewer(parts: &ChatProfileGifTabParts, entry: &ChatProfileMediaEntry, cx: &mut App) {
        let chat = Self::chat(&parts.chat_id, cx);
        let entries = ChatProfileSource::gif_items(&parts.chat_id, cx);
        let items = ChatMediaGalleryBuilder::items_profile_entries(&chat, &entries, cx);
        let registry_id = MediaViewerFrameKeys::profile_grid(&entry.id);
        let Some(open_index) = ChatMediaGalleryBuilder::open_index(&items, &registry_id) else { return };

        let skin = MediaViewerHeroSkin::new(entry.item.display_source(), false);
        let frame = MediaCellScreenFrameRegistry::frame(&registry_id).unwrap_or_default();
        let presenter = media_viewer_presentation(cx);

        let delete_chat_id = parts.chat_id.clone();
        let forward_chat_id = parts.chat_id.clone();
        let jump_parts = parts.clone();
        let flyback_parts = parts.clone();
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
                Self::jump(&jump_parts, &item.message_id, cx);
            })),
            flyback_target_provider: Some(Rc::new(move |registry_id: &str, cx: &mut App| {
                Self::flyback_target(&flyback_parts.chat_id, &flyback_parts.positioner, registry_id, cx)
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
        })
        .detach();
    }
}

impl RenderOnce for ChatProfileGifTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let chat_id = self.chat_id;
        ChatProfileTabHost::hosted(
            SharedString::from(format!("chat-profile-gif-host-{chat_id}")),
            window,
            cx,
            |_| Vec::new(),
            move |window, cx| Self::body(&chat_id, window, cx),
        )
    }
}
