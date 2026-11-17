use std::rc::Rc;

use gpui::{App, Entity, FontWeight, Pixels, Point, SharedString, Window};
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_platform::haptics::{NotificationKind, notification};
use rise_platform::pasteboard::UIPasteboard;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::skeleton_ui::SkeletonListUi;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::chat::components::chat::bubble::chat_mention_tap_layer::ChatLinkOpen;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_message_jump::ChatProfileMessageJump;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_pagination::ChatProfilePaginationPrefetchModifiers;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::{
    ChatProfileFormatter, ChatProfileSectionHeader, ChatProfileTabBottomSpacer, ChatProfileTabDataRender,
    ChatProfileTabScrollContainer,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_source::{
    ChatProfileLinkEntry, ChatProfileSource,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tab_window::{
    ChatProfileTabHost, ChatProfileWindowedRow, ChatProfileWindowedRows,
};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

const LINK_CACHE_ID: &str = "getLinkMessages";

#[derive(Clone)]
struct ChatProfileLinkTabParts {
    chat_id: String,
    topic_id: Option<String>,
    nav_store: Option<Entity<NavigationStore>>,
}

#[derive(IntoElement)]
pub(crate) struct ChatProfileLinkTab {
    chat_id: String,
    topic_id: Option<String>,
}

impl ChatProfileLinkTab {
    const ESTIMATED_ROW_HEIGHT: f32 = 90.0;
    const SKELETON_ROWS: usize = 8;
    const THUMB_SIZE: f32 = 56.0;
    const THUMB_RADIUS: f32 = 12.0;
    const THUMB_ICON_SIZE: f32 = 20.0;
    const THUMB_DECODE_DIMENSION: i64 = 168;
    const SEPARATOR_LEADING: f32 = 84.0;

    pub(crate) fn new(chat_id: impl Into<String>) -> Self {
        Self { chat_id: chat_id.into(), topic_id: None }
    }

    pub(crate) fn topic_id(mut self, topic_id: Option<String>) -> Self {
        self.topic_id = topic_id;
        self
    }

    fn body(chat_id: &str, topic_id: Option<&str>, cx: &mut App) -> AnyElement {
        let parts = ChatProfileLinkTabParts {
            chat_id: chat_id.to_owned(),
            topic_id: topic_id.map(str::to_owned),
            nav_store: env::<NavigationStoreKey>(cx),
        };
        let entries = ChatProfileSource::link_items(chat_id, topic_id, cx);
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, None);
        let services = MessageServicesStore::shared(cx);
        let sections = services
            .update(cx, |services, cx| services.message_tab_link_sections(LINK_CACHE_ID, &scope, &entries, cx));
        let entry_index_by_id = services.update(cx, |services, cx| {
            services.message_tab_entry_index_by_id(
                LINK_CACHE_ID,
                &scope,
                || entries.iter().map(|entry| entry.id.clone()).collect(),
                cx,
            )
        });
        let item_count = entries.len();
        let retry_parts = parts.clone();
        let id_base = format!("chat-profile-link-{scope}");
        let scroll_id = SharedString::from(id_base.clone());
        let rows_id = SharedString::from(format!("{id_base}-rows"));
        let loading_id = SharedString::from(format!("{id_base}-loading"));
        let content_scope = scope.clone();

        ChatProfileTabDataRender::new(
            LINK_CACHE_ID,
            scope,
            entries.is_empty(),
            "link",
            tr("chat_profile_empty_links"),
            move |_, cx| {
                let parts = retry_parts.clone();
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_initial_link_tab(&parts.chat_id, parts.topic_id.as_deref(), None, true, cx);
                });
            },
            move |_, _| {
                ChatProfileTabScrollContainer::new(scroll_id, move |_, _| {
                    let mut rows = Vec::with_capacity(item_count + sections.len());
                    for section in sections {
                        let title = section.title();
                        rows.push(
                            ChatProfileWindowedRow::new(format!("{id_base}-section-{}", section.id), move |_, _| {
                                ChatProfileSectionHeader::new(title.clone())
                            })
                            .pinned(),
                        );
                        for entry in section.items {
                            let index = entry_index_by_id.get(&entry.id).copied().unwrap_or(0);
                            let row_id = format!("{id_base}-row-{}", entry.id);
                            let parts = parts.clone();
                            let scope = content_scope.clone();
                            let entry = Rc::new(entry);
                            rows.push(ChatProfileWindowedRow::new(row_id, move |_, cx| {
                                Self::row(&parts, entry.clone(), index, item_count, &scope, cx)
                            }));
                        }
                    }
                    div()
                        .w_full()
                        .child(ChatProfileWindowedRows::new(rows_id, Self::ESTIMATED_ROW_HEIGHT, rows))
                        .child(ChatProfileTabBottomSpacer)
                })
            },
            move |_, _| {
                ChatProfileTabScrollContainer::new(loading_id.clone(), |_, _| SkeletonListUi {
                    rows: Self::SKELETON_ROWS,
                    ..SkeletonListUi::default()
                })
                .scroll_enabled(false)
                .shows_indicators(false)
            },
        )
        .into_any_element()
    }

    fn row(
        parts: &ChatProfileLinkTabParts,
        entry: Rc<ChatProfileLinkEntry>,
        index: i64,
        item_count: usize,
        scope: &str,
        cx: &mut App,
    ) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let actions = Self::hold_actions(&entry, &parts.chat_id, cx);
        let content_entry = entry.clone();
        let action_entry = entry.clone();
        let action_parts = parts.clone();
        let tap_url = entry.preview.url.clone();
        let load_parts = parts.clone();
        vstack(0.0)
            .w_full()
            .child(
                div().w_full().child(
                    HoldContextMenuContainer::new(
                        SharedString::from(format!("chat-profile-link-hold-{}", entry.id)),
                        HoldContextMenuAlignment::Leading,
                        Rc::new(move |_: &App| actions.clone()),
                        Rc::new(move |_: &mut Window, cx: &mut App| {
                            Self::row_content(&content_entry, cx).into_any_element()
                        }),
                    )
                    .show_emoji_list(false)
                    .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                        Self::handle_action(&action_parts, &action_entry, &id, cx);
                    }))
                    .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
                        ChatLinkOpen::open(&tap_url, cx);
                    }))
                    .chat_profile_pagination_prefetch(
                        SharedString::from(format!("chat-profile-link-prefetch-{}", entry.id)),
                        index,
                        item_count,
                        LINK_CACHE_ID,
                        scope,
                        None,
                        move |_, cx| {
                            let parts = load_parts.clone();
                            MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                                store.load_more_link_tab(&parts.chat_id, parts.topic_id.as_deref(), cx);
                            });
                        },
                        cx,
                    ),
                ),
            )
            .child(
                div().w_full().pl(px(Self::SEPARATOR_LEADING)).child(
                    div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.4)),
                ),
            )
            .into_any_element()
    }

    fn hold_actions(entry: &ChatProfileLinkEntry, chat_id: &str, cx: &mut App) -> Vec<HoldContextMenuAction> {
        let mut actions = vec![
            HoldContextMenuAction::new("open", tr("ctx_open_link")).system_image("safari"),
            HoldContextMenuAction::new("copy", tr("ctx_copy_link")).system_image("doc.on.doc"),
            HoldContextMenuAction::new("share", tr("ctx_share")).system_image("square.and.arrow.up"),
            HoldContextMenuAction::new("goto", tr("ctx_go_to_message")).system_image("arrow.turn.up.left"),
        ];
        if let Some(delete) = ChatProfileSource::delete_action(&entry.message_id, &entry.sender_id, chat_id, cx) {
            actions.push(delete);
        }
        actions
    }

    fn handle_action(parts: &ChatProfileLinkTabParts, entry: &ChatProfileLinkEntry, id: &str, cx: &mut App) {
        if let Some(for_everyone) = ChatProfileSource::deletes_for_everyone(id) {
            let ids = [entry.message_id.clone()];
            MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.delete_messages(&ids, &parts.chat_id, parts.topic_id.as_deref(), None, for_everyone, cx);
            });
            return;
        }
        match id {
            "open" => ChatLinkOpen::open(&entry.preview.url, cx),
            "copy" => {
                UIPasteboard::general().set_string(&entry.preview.url, cx);
                notification(NotificationKind::Success);
            }
            "share" => {
                let Some(url) = ChatLinkOpen::url(&entry.preview.url) else { return };
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_link(url, cx));
            }
            "goto" => {
                if let Some(nav_store) = parts.nav_store.as_ref() {
                    ChatProfileMessageJump::open(
                        &parts.chat_id,
                        parts.topic_id.as_deref(),
                        &entry.message_id,
                        nav_store,
                        cx,
                    );
                }
            }
            _ => {}
        }
    }

    fn row_content(entry: &ChatProfileLinkEntry, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let preview = &entry.preview;
        let domain = Self::host(&preview.url);
        let thumbnail = match preview.image_url.as_deref().filter(|image| !image.is_empty()) {
            Some(image) => div()
                .size_full()
                .child(
                    CleverImageUi::new(image.to_owned())
                        .id(SharedString::from(format!("chat-profile-link-thumb-{}", entry.id)))
                        .content_mode(ContentMode::Fill)
                        .corner_radius(Self::THUMB_RADIUS)
                        .persist_to_disk(true)
                        .maximum_decode_pixel_dimension(Some(Self::THUMB_DECODE_DIMENSION)),
                )
                .into_any_element(),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.primary._100.opacity(0.16))
                .children(IconUi::sized("link", px(Self::THUMB_ICON_SIZE), theme.primary._100, false))
                .into_any_element(),
        };

        let headline = preview.title.clone().unwrap_or_else(|| preview.url.clone());
        hstack(12.0)
            .w_full()
            .items_start()
            .px(px(16.0))
            .py(px(12.0))
            .hover(|style| style.bg(theme.bg._200))
            .child(
                div()
                    .flex_none()
                    .size(px(Self::THUMB_SIZE))
                    .rounded(px(Self::THUMB_RADIUS))
                    .overflow_hidden()
                    .child(thumbnail),
            )
            .child(
                vstack(3.0)
                    .flex_1()
                    .min_w_0()
                    .align(HorizontalAlignment::Leading)
                    .child(MainText::new(headline).px(14.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(2))
                    .children(preview.description.clone().map(|description| {
                        MainText::new(description).px(12.0).color(theme.text.secondary).number_of_lines(2)
                    }))
                    .child(MainText::new(domain).px(12.0).color(theme.primary._100).number_of_lines(1))
                    .child(
                        hstack(6.0)
                            .pt(px(2.0))
                            .child(MainText::new(entry.sender_name.clone()).px(11.0).color(theme.text.secondary))
                            .child(div().size(px(2.0)).rounded_full().bg(theme.text.secondary.opacity(0.4)))
                            .child(
                                MainText::new(ChatProfileFormatter::relative_date(entry.created_at))
                                    .px(11.0)
                                    .color(theme.text.secondary),
                            ),
                    ),
            )
    }

    fn host(url_str: &str) -> String {
        match Url::from_string(url_str).and_then(|url| url.host()) {
            Some(host) => host.replace("www.", ""),
            None => url_str.to_owned(),
        }
    }
}

impl RenderOnce for ChatProfileLinkTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let chat_id = self.chat_id;
        let topic_id = self.topic_id;
        ChatProfileTabHost::hosted(
            SharedString::from(format!("chat-profile-link-host-{chat_id}-{}", topic_id.as_deref().unwrap_or_default())),
            window,
            cx,
            |_| Vec::new(),
            move |_, cx| Self::body(&chat_id, topic_id.as_deref(), cx),
        )
    }
}
