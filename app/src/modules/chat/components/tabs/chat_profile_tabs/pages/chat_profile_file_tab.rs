use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, Entity, FontWeight, Hsla, IntoElement, ParentElement, Pixels, Point, RenderOnce,
    SharedString, Styled, Window, div, px, rgb,
};
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::MainText;
use rise_ui::skeleton_ui::SkeletonListUi;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::stacks::{HorizontalAlignment, StackAlign, hstack, spacer, vstack};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
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
use crate::modules::chat::shared::message_permissions::ChatMessageAction;
use crate::modules::chat::shared::utils::chat_document_open::ChatDocumentOpen;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::music::widgets::bottomsheets::playlists::message_mp3_import_sheet::MessageMusicImportPresenter;

const CACHE_ID: &str = "get_file_messages";

const ROW_HEIGHT: f32 = 64.5;

#[derive(IntoElement)]
pub(crate) struct ChatProfileFileTab {
    chat_id: String,
    topic_id: Option<String>,
}

impl ChatProfileFileTab {
    pub(crate) fn new(chat_id: impl Into<String>) -> Self {
        Self { chat_id: chat_id.into(), topic_id: None }
    }

    pub(crate) fn topic_id(mut self, topic_id: Option<String>) -> Self {
        self.topic_id = topic_id;
        self
    }

    fn body(chat_id: &str, topic_id: Option<&str>, cx: &mut App) -> AnyElement {
        let entries = ChatProfileSource::document_items(chat_id, topic_id, cx);
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, None);
        let (sections, entry_index_by_id) = MessageServicesStore::shared(cx).update(cx, |services, cx| {
            let sections = services.message_tab_media_sections(CACHE_ID, &scope, &entries, cx);
            let index = services.message_tab_entry_index_by_id(
                CACHE_ID,
                &scope,
                || entries.iter().map(|entry| entry.id.clone()).collect(),
                cx,
            );
            (sections, index)
        });
        let rows = Rc::new(ChatProfileFileRows {
            id: format!("chat-profile-file-{scope}"),
            chat_id: chat_id.to_owned(),
            topic_id: topic_id.map(str::to_owned),
            scope: scope.clone(),
            entry_index_by_id,
            item_count: entries.len(),
            nav_store: env::<NavigationStoreKey>(cx),
            theme: Rc::new(ThemeStore::shared(cx).read(cx).current_theme().clone()),
        });
        let retry_chat_id = chat_id.to_owned();
        let retry_topic_id = topic_id.map(str::to_owned);
        let list_id = rows.child_id("rows");
        let content_id = rows.child_id("scroll");
        let loading_id = rows.child_id("loading");
        ChatProfileTabDataRender::new(
            CACHE_ID,
            scope,
            entries.is_empty(),
            "doc.text",
            tr("chat_profile_empty_files"),
            move |_, cx| {
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_initial_files_tab(&retry_chat_id, retry_topic_id.as_deref(), None, true, cx);
                });
            },
            move |_, _| {
                ChatProfileTabScrollContainer::new(content_id, move |_, _| {
                    vstack(0.0)
                        .w_full()
                        .child(ChatProfileSectionList::new(list_id, sections, ROW_HEIGHT, move |entry, _, cx| {
                            rows.row(entry, cx)
                        }))
                        .child(ChatProfileTabBottomSpacer)
                })
            },
            move |_, _| {
                ChatProfileTabScrollContainer::new(loading_id.clone(), |_, _| SkeletonListUi::default())
                    .scroll_enabled(false)
                    .shows_indicators(false)
            },
        )
        .into_any_element()
    }
}

impl RenderOnce for ChatProfileFileTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let chat_id = self.chat_id;
        let topic_id = self.topic_id;
        let topic_key = topic_id.as_deref().unwrap_or_default();
        ChatProfileTabHost::hosted(
            SharedString::from(format!("chat-profile-file-host-{chat_id}-{topic_key}")),
            window,
            cx,
            |_| Vec::new(),
            move |_, cx| ChatProfileFileTab::body(&chat_id, topic_id.as_deref(), cx),
        )
    }
}

struct ChatProfileFileRows {
    id: String,
    chat_id: String,
    topic_id: Option<String>,
    scope: String,
    entry_index_by_id: HashMap<String, i64>,
    item_count: usize,
    nav_store: Option<Entity<NavigationStore>>,
    theme: Rc<AppTheme>,
}

impl ChatProfileFileRows {
    fn child_id(&self, name: &str) -> ElementId {
        ElementId::Name(format!("{}-{name}", self.id).into())
    }

    fn row(&self, entry: &ChatProfileMediaEntry, cx: &mut App) -> AnyElement {
        let ext = ChatProfileFileRow::file_ext(entry.item.file_name.as_deref().unwrap_or("FILE")).to_uppercase();
        let actions = self.menu_actions(entry, cx);
        let content_entry = entry.clone();
        let content_theme = self.theme.clone();
        let action_context = ChatProfileFileActions {
            entry: entry.clone(),
            chat_id: self.chat_id.clone(),
            topic_id: self.topic_id.clone(),
            nav_store: self.nav_store.clone(),
        };
        let tap_entry = entry.clone();
        let tap_chat_id = self.chat_id.clone();
        let hold = HoldContextMenuContainer::new(
            self.child_id(&format!("hold-{}", entry.id)),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, _: &mut App| {
                ChatProfileFileRow { entry: content_entry.clone(), ext: ext.clone(), theme: content_theme.clone() }
                    .into_any_element()
            }),
        )
        .show_emoji_list(false)
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| action_context.handle_action(&id, cx)))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            ChatDocumentOpen::open(&tap_entry.item, Some(&tap_entry.message_id), Some(&tap_chat_id), cx);
        }));
        let separator = self.theme.border._200.opacity(0.45);
        let load_chat_id = self.chat_id.clone();
        let load_topic_id = self.topic_id.clone();
        div()
            .w_full()
            .child(hold)
            .child(div().w_full().pl(px(72.0)).child(div().w_full().h(px(0.5)).bg(separator)))
            .chat_profile_pagination_prefetch(
                self.child_id(&format!("prefetch-{}", entry.id)),
                self.entry_index_by_id.get(&entry.id).copied().unwrap_or(0),
                self.item_count,
                CACHE_ID,
                &self.scope,
                None,
                move |_, cx| {
                    MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.load_more_files_tab(&load_chat_id, load_topic_id.as_deref(), cx);
                    });
                },
                cx,
            )
    }

    fn menu_actions(&self, entry: &ChatProfileMediaEntry, cx: &mut App) -> Vec<HoldContextMenuAction> {
        let mut actions =
            vec![HoldContextMenuAction::new("open", tr("ctx_open")).system_image("arrow.up.forward.app")];
        if ChatProfileSource::can_forward(entry, &self.chat_id, cx) {
            actions.push(HoldContextMenuAction::new("forward", tr("ctx_forward")).system_image("arrowshape.turn.up.right"));
        }
        if entry.item.is_importable_mp3() && !entry.has_protected_content {
            actions.push(
                HoldContextMenuAction::new(ChatMessageAction::ADD_TO_LOCAL_PLAYLIST, tr("ctx_add_mp3_to_local_playlist"))
                    .system_image("text.badge.plus"),
            );
        }
        actions.extend(ChatProfileMediaSaveActions::actions(entry));
        actions.push(HoldContextMenuAction::new("goto", tr("ctx_go_to_message")).system_image("arrow.turn.up.left"));
        if let Some(delete) =
            ChatProfileSource::delete_action(&entry.message_id, &entry.sender_id, &self.chat_id, cx)
        {
            actions.push(delete);
        }
        actions
    }
}

struct ChatProfileFileActions {
    entry: ChatProfileMediaEntry,
    chat_id: String,
    topic_id: Option<String>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl ChatProfileFileActions {
    fn handle_action(&self, id: &str, cx: &mut App) {
        let entry = &self.entry;
        let chat_id = self.chat_id.as_str();
        let topic_id = self.topic_id.as_deref();
        if ChatProfileMediaSaveActions::handle(id, entry, chat_id, cx) {
            return;
        }
        if let Some(for_everyone) = ChatProfileSource::deletes_for_everyone(id) {
            MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.delete_messages(
                    std::slice::from_ref(&entry.message_id),
                    chat_id,
                    topic_id,
                    None,
                    for_everyone,
                    cx,
                );
            });
            return;
        }
        match id {
            "open" => ChatDocumentOpen::open(&entry.item, Some(&entry.message_id), Some(chat_id), cx),
            "forward" => {
                let Some(message) = ChatProfileSource::forwardable_message(entry, chat_id, cx) else {
                    return;
                };
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_messages(vec![message], cx));
            }
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
            "goto" => {
                if let Some(nav_store) = &self.nav_store {
                    ChatProfileMessageJump::open(chat_id, topic_id, &entry.message_id, nav_store, cx);
                }
            }
            _ => {}
        }
    }
}

#[derive(IntoElement)]
struct ChatProfileFileRow {
    entry: ChatProfileMediaEntry,
    ext: String,
    theme: Rc<AppTheme>,
}

impl ChatProfileFileRow {
    fn file_ext(name: &str) -> &str {
        match name.rfind('.') {
            Some(dot) => &name[dot + 1..],
            None => "FILE",
        }
    }

    fn ext_color(ext: &str, theme: &AppTheme) -> Hsla {
        match ext {
            "PDF" => rgb(0xE03B3B).into(),
            "ZIP" | "RAR" | "7Z" => rgb(0x7E57C2).into(),
            "DOC" | "DOCX" | "TXT" | "RTF" => rgb(0x3D7BD9).into(),
            "XLS" | "XLSX" | "CSV" | "NUMBERS" => rgb(0x34A853).into(),
            "PPT" | "KEY" => rgb(0xF59E2B).into(),
            "FIG" | "FIGMA" => rgb(0xA259FF).into(),
            "PSD" | "AI" | "SKETCH" => rgb(0xFF7A00).into(),
            "MP4" | "MOV" | "AVI" => rgb(0x5E81F4).into(),
            "MP3" | "WAV" | "FLAC" | "M4A" => rgb(0x1DB954).into(),
            _ => theme.primary._100,
        }
    }
}

impl RenderOnce for ChatProfileFileRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let secondary = theme.text.secondary;
        let item = self.entry.item;
        let badge_text: String = self.ext.chars().take(4).collect();
        let size = item.file_size.map(|size| {
            hstack(6.0)
                .flex_none()
                .child(
                    MainText::new(ChatProfileFormatter::bytes(size)).px(11.0).color(secondary).number_of_lines(1),
                )
                .child(div().size(px(2.0)).rounded_full().bg(secondary.opacity(0.4)))
        });
        hstack(12.0)
            .w_full()
            .px(px(16.0))
            .py(px(10.0))
            .child(
                div()
                    .size(px(44.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(12.0))
                    .bg(Self::ext_color(&self.ext, &theme))
                    .child(
                        MainText::new(badge_text)
                            .px(11.0)
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .color(theme.content_on(Self::ext_color(&self.ext, &theme)))
                            .number_of_lines(1)
                            .minimum_scale_factor(0.5),
                    ),
            )
            .child(
                vstack(2.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w_0()
                    .child(
                        MainText::new(item.file_name.clone().unwrap_or_else(|| tr("msg_preview_document_fallback")))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .number_of_lines(1),
                    )
                    .child(
                        hstack(6.0)
                            .w_full()
                            .children(size)
                            .child(
                                div().min_w_0().child(
                                    MainText::new(self.entry.sender_name)
                                        .px(11.0)
                                        .color(secondary)
                                        .number_of_lines(1),
                                ),
                            )
                            .child(spacer())
                            .child(
                                div().flex_none().child(
                                    MainText::new(ChatProfileFormatter::relative_date(self.entry.created_at))
                                        .px(11.0)
                                        .color(secondary)
                                        .number_of_lines(1),
                                ),
                            ),
                    ),
            )
    }
}
