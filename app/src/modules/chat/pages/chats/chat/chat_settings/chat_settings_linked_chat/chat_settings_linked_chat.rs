use std::collections::HashSet;

use gpui::{App, Context, Entity, FontWeight, SharedString, Subscription, Window};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::tooltip_label::TooltipLabelPageMotion;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::chat::components::chat::chat_logo::ChatLogo;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType, LinkedChatInfo};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;

const PREFETCH_THRESHOLD: usize = 8;
const TITLE_PLACEHOLDER: &str = "{title}";
const EMPTY_ICON_SIZE: f32 = 36.0;
const EMPTY_ICON_OPACITY: f32 = 0.6;
const EMPTY_TEXT_MAX_WIDTH: f32 = 300.0;
const TAKEN_GROUP_OPACITY: f32 = 0.5;
const ROW_SEPARATOR_INSET: f32 = 68.0;

type NavStore = Option<Entity<NavigationStore>>;

fn page_id(chat_id: &str, part: &str) -> SharedString {
    SharedString::from(format!("chat-settings-linked-{part}-{chat_id}"))
}

pub struct ChatSettingsLinkedChat {
    chat_id: Option<String>,
    services: Entity<ChatServicesStore>,
    _subscriptions: [Subscription; 3],
}

impl ChatSettingsLinkedChat {
    pub fn new(chat_id: Option<String>, cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        let chat_store = ChatActionsStore::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        let subscriptions = [
            cx.observe(&services, |_, _, cx| cx.notify()),
            cx.observe(&chat_store, |_, _, cx| cx.notify()),
            cx.observe(&sandbox, |_, _, cx| cx.notify()),
        ];
        Self { chat_id, services, _subscriptions: subscriptions }
    }

    fn resolved_chat_id(&self) -> &str {
        self.chat_id.as_deref().unwrap_or_default()
    }

    fn chat(&self, cx: &App) -> Option<ChatInfo> {
        self.services.read(cx).chat_info(self.resolved_chat_id(), cx)
    }

    /// chat-service unlinks for the owner only, an admin sees the link without the button
    fn can_unlink(&self, chat: Option<&ChatInfo>, cx: &mut App) -> bool {
        if GuideSandbox::shared(cx).read(cx).owns_discussion(Some(self.resolved_chat_id())) {
            return true;
        }
        chat.map(ChatProfilePermissions::is_owner).unwrap_or(false)
    }

    fn is_linked_elsewhere(&self, group_id: &str, cx: &App) -> bool {
        let Some(other) = self.services.read(cx).resolved_linked_chat(group_id, cx).map(|linked| linked.id) else {
            return false;
        };
        !other.is_empty() && other != self.resolved_chat_id()
    }

    fn candidate_groups(&self, cx: &mut App) -> Vec<ChatInfo> {
        if let Some(invented) = GuideSandbox::shared(cx).read(cx).discussion_candidates(self.resolved_chat_id()) {
            return invented;
        }
        let (inbox, archived) =
            self.services.update(cx, |services, cx| (services.current_chat_infos(cx), services.archived_chats(cx)));
        let mut seen: HashSet<String> = HashSet::new();
        inbox
            .into_iter()
            .chain(archived)
            .filter(|chat| chat.r#type == ChatType::Group && ChatProfilePermissions::can_manage(chat))
            .filter(|chat| seen.insert(chat.id.clone()))
            .collect()
    }

    fn linked_card(
        &self,
        info: &LinkedChatInfo,
        chat: Option<&ChatInfo>,
        is_channel: bool,
        can_unlink: bool,
        nav_store: &NavStore,
        theme: &AppTheme,
    ) -> Div {
        let hint = tr(if is_channel { "chat_linked_comments_hint" } else { "chat_linked_channel_hint" })
            .replace(TITLE_PLACEHOLDER, &info.title);
        let open_store = nav_store.clone();
        let open_id = info.id.clone();
        let chat_id = self.resolved_chat_id();
        let logo_chat = ChatInfo {
            id: info.id.clone(),
            title: info.title.clone(),
            r#type: if is_channel { ChatType::Group } else { ChatType::Channel },
            logo_url: info.logo_url.clone(),
            ..ChatInfo::default()
        };
        vstack(14.0)
            .w_full()
            .items_stretch()
            .child(
                div()
                    .w_full()
                    .px(px(24.0))
                    .child(MainText::new(hint).px(14.0).tac(TextAlignT::Center).color(theme.text.secondary)),
            )
            .child(
                div().w_full().px(px(12.0)).child(
                    plain(page_id(chat_id, "open"))
                        .w_full()
                        .on_press(move |_: &mut Window, cx: &mut App| {
                            if let Some(nav_store) = &open_store {
                                let route = RootRoute::Chat { chat_id: Some(open_id.clone()) };
                                nav_store.update(cx, |nav_store, cx| {
                                    nav_store.push(route, NavigationStoreSettlingPush::Drop, cx);
                                });
                            }
                        })
                        .child(
                            Shape::continuous(16.0).round(
                                hstack(12.0)
                                    .w_full()
                                    .p(px(14.0))
                                    .bg(theme.bg._200)
                                    .child(
                                        ChatLogo::new(page_id(chat_id, "linked-logo"), logo_chat)
                                            .size(44.0)
                                            .icon_size(20.0)
                                            .show_online_indicator(false),
                                    )
                                    .child(
                                        vstack(2.0)
                                            .align(HorizontalAlignment::Leading)
                                            .child(
                                                MainText::new(info.title.clone())
                                                    .px(16.0)
                                                    .font_weight(FontWeight::SEMIBOLD),
                                            )
                                            .child(
                                                MainText::new(tr(if is_channel {
                                                    "chat_linked_kind_group"
                                                } else {
                                                    "chat_linked_kind_channel"
                                                }))
                                                .px(13.0)
                                                .color(theme.text.secondary),
                                            ),
                                    )
                                    .child(spacer()),
                            ),
                        ),
                ),
            )
            .when(is_channel, |column| {
                column.child(
                    div().w_full().px(px(24.0)).child(
                        MainText::new(tr("chat_linked_autoforward_hint"))
                            .px(13.0)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary),
                    ),
                )
            })
            .when(can_unlink, |column| {
                let unlink_info = info.clone();
                let unlink_message = chat.map(|chat| chat.title.clone()).unwrap_or_default();
                let unlink_chat_id = chat_id.to_owned();
                let unlink_store = nav_store.clone();
                column.child(GroupedBtns::new(
                    page_id(chat_id, "unlink"),
                    vec![
                        GroupedBtn::new("unlink", tr(Self::unlink_key(is_channel)))
                            .icon("link.badge.plus")
                            .icon_background(ChatTileColor::red())
                            .destructive(true)
                            .show_chevron(false)
                            .callback(move |_: &mut Window, cx: &mut App| {
                                Self::confirm_unlink(
                                    &unlink_info,
                                    &unlink_message,
                                    is_channel,
                                    unlink_chat_id.clone(),
                                    unlink_store.clone(),
                                    cx,
                                );
                            }),
                    ],
                ))
            })
    }

    fn unlink_key(is_channel: bool) -> &'static str {
        if is_channel { "chat_settings_linked_group_unlink" } else { "chat_settings_linked_channel_unlink" }
    }

    fn confirm_unlink(
        info: &LinkedChatInfo,
        message: &str,
        is_channel: bool,
        chat_id: String,
        nav_store: NavStore,
        cx: &mut App,
    ) {
        NativeConfirmAlert::present_delete(
            &info.title,
            message,
            Some(tr(Self::unlink_key(is_channel))),
            None,
            false,
            None,
            None,
            move |cx: &mut App| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.unlink_discussion(&chat_id, cx));
                if let Some(nav_store) = &nav_store {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            },
            cx,
        );
    }

    fn group_picker(&self, window: &mut Window, theme: &AppTheme, cx: &mut App) -> Div {
        let chat_id = self.resolved_chat_id().to_owned();
        let candidate_groups = self.candidate_groups(cx);
        let services = self.services.read(cx);
        let cache_key = SharedString::from(services.chats_cache_key(cx).unwrap_or_default());
        let relative_id = services.chats_relative_id(cx);
        let has_more = services.chats_has_more(cx);
        let (archive_cache_key, archive_relative_id, archive_has_more) = self.services.update(cx, |services, cx| {
            (
                SharedString::from(services.archived_chats_cache_key(cx).unwrap_or_default()),
                services.archived_chats_relative_id(cx),
                services.archived_chats_has_more(cx),
            )
        });
        let count = candidate_groups.len();

        let body = if candidate_groups.is_empty() {
            vstack(10.0)
                .w_full()
                .items_center()
                .pt(px(24.0))
                .children(IconUi::sized(
                    "bubble.left.and.bubble.right",
                    px(EMPTY_ICON_SIZE),
                    theme.text.secondary.opacity(EMPTY_ICON_OPACITY),
                    false,
                ))
                .child(
                    MainText::new(tr("chat_linked_no_groups"))
                        .max_w(px(EMPTY_TEXT_MAX_WIDTH))
                        .px(14.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary),
                )
                .into_any_element()
        } else {
            let rows: Vec<AnyElement> = candidate_groups
                .iter()
                .enumerate()
                .map(|(index, group)| {
                    let taken = self.is_linked_elsewhere(&group.id, cx);
                    self.group_row(
                        group,
                        index,
                        count,
                        taken,
                        cache_key.clone(),
                        relative_id.clone(),
                        has_more,
                        window,
                        theme,
                        cx,
                    )
                })
                .collect();
            div()
                .w_full()
                .px(px(12.0))
                .child(Shape::continuous(20.0).round(vstack(0.0).w_full().items_stretch().bg(theme.bg._200).children(rows)))
                .into_any_element()
        };

        vstack(14.0)
            .w_full()
            .items_stretch()
            .child(
                div().w_full().px(px(24.0)).child(
                    MainText::new(tr("chat_linked_pick_hint"))
                        .px(14.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary),
                ),
            )
            .child(
                body.sai_pagination_drain_filtered_empty(
                    page_id(&chat_id, "drain"),
                    SaiPaginationPrefetchOptions::drain_filtered_empty(0, cache_key, relative_id, has_more),
                    |_, cx| {
                        ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_chats(cx));
                    },
                )
                .sai_pagination_drain_filtered_empty(
                    page_id(&chat_id, "drain-archive"),
                    SaiPaginationPrefetchOptions::drain_filtered_empty(
                        0,
                        archive_cache_key,
                        archive_relative_id,
                        archive_has_more,
                    ),
                    |_, cx| {
                        ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_archived_chats(cx));
                    },
                ),
            )
    }
}

impl Render for ChatSettingsLinkedChat {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store: NavStore = env::<NavigationStoreKey>(cx);
        let resolved_chat_id = self.resolved_chat_id().to_owned();
        let chat = self.chat(cx);
        let is_channel = chat.as_ref().is_some_and(|chat| chat.r#type == ChatType::Channel);
        let linked = self.services.read(cx).resolved_linked_chat(&resolved_chat_id, cx);

        let task_chat_id = resolved_chat_id.clone();
        use_task(page_id(&resolved_chat_id, "task"), resolved_chat_id.clone(), window, cx, async move |cx| {
            if task_chat_id.is_empty() {
                return;
            }
            let settle = cx.update(|_, cx| TooltipLabelPageMotion::remaining(cx)).unwrap_or_default();
            if !settle.is_zero() {
                cx.background_executor().timer(settle).await;
            }
            let _ = cx.update(|_, cx| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_initial_chats(cx);
                    store.load_initial_archived_chats(cx);
                    store.load_chat_settings(&task_chat_id, cx);
                    store.load_linked_chat(&task_chat_id, cx);
                });
            });
        });

        let state_view = if let Some(linked) = &linked {
            let can_unlink = self.can_unlink(chat.as_ref(), cx);
            self.linked_card(linked, chat.as_ref(), is_channel, can_unlink, &nav_store, &theme)
        } else if is_channel {
            self.group_picker(window, &theme, cx)
        } else {
            Self::empty_group_state(&theme)
        };

        let content = scroll_view(page_id(&resolved_chat_id, "scroll"))
            .list_shell_scroll_under_chrome(cx)
            .size_full()
            .bg(theme.bg._100)
            .child(vstack(16.0).w_full().items_stretch().pt(px(12.0)).child(state_view).child(spacer_min_h(80.0)));

        SettingsWrapperUi::new(
            tr(if is_channel { "chat_settings_discussion" } else { "chat_settings_linked_channel" }),
            move |_, cx| {
                if let Some(nav_store) = &nav_store {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            },
        )
        .content(content)
    }
}

impl ChatSettingsLinkedChat {
    #[allow(clippy::too_many_arguments)]
    fn group_row(
        &self,
        group: &ChatInfo,
        index: usize,
        count: usize,
        taken: bool,
        cache_key: SharedString,
        relative_id: Option<String>,
        has_more: bool,
        window: &mut Window,
        theme: &AppTheme,
        cx: &mut App,
    ) -> AnyElement {
        let chat_id = self.resolved_chat_id().to_owned();
        let group_id = group.id.clone();

        let load_group_id = group.id.clone();
        use_task(page_id(&group.id, "group-task"), group.id.clone(), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_linked_chat(&load_group_id, cx));
            });
        });

        let row = plain(page_id(&group.id, "group"))
            .w_full()
            .disabled(taken)
            .on_press(move |_: &mut Window, cx: &mut App| {
                ChatInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.link_discussion_group(&chat_id, &group_id, None, cx));
            })
            .child(
                hstack(12.0)
                    .w_full()
                    .opacity(if taken { TAKEN_GROUP_OPACITY } else { 1.0 })
                    .px(px(16.0))
                    .py(px(10.0))
                    .bg(theme.bg._200)
                    .child(
                        ChatLogo::new(page_id(&group.id, "group-logo"), group.clone())
                            .size(40.0)
                            .icon_size(18.0)
                            .show_online_indicator(false),
                    )
                    .child(
                        vstack(2.0)
                            .flex_1()
                            .min_w_0()
                            .align(HorizontalAlignment::Leading)
                            .child(
                                MainText::new(group.title.clone())
                                    .px(15.0)
                                    .font_weight(FontWeight::MEDIUM)
                                    .number_of_lines(1),
                            )
                            .child(
                                MainText::new(if taken {
                                    tr("chat_linked_group_taken")
                                } else {
                                    tr(if group.is_public { "chat_linked_group_public" } else { "chat_linked_group_private" })
                                })
                                .px(12.0)
                                .color(theme.text.secondary),
                            ),
                    ),
            )
            .guide_anchor((index == 0).then_some(RiseGuideAnchor::LinkedChatFirstGroup))
            .sai_pagination_prefetch(
                page_id(&group.id, "group-prefetch"),
                SaiPaginationPrefetchOptions::new(index, count, cache_key, relative_id, has_more)
                    .threshold(PREFETCH_THRESHOLD),
                |_, cx| {
                    ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_chats(cx));
                },
            );

        div()
            .w_full()
            .child(row)
            .when(index + 1 < count, |entry| {
                entry.child(
                    div()
                        .w_full()
                        .pl(px(ROW_SEPARATOR_INSET))
                        .child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55))),
                )
            })
            .into_any_element()
    }

    fn empty_group_state(theme: &AppTheme) -> Div {
        vstack(10.0)
            .w_full()
            .items_center()
            .pt(px(32.0))
            .children(IconUi::sized(
                "megaphone",
                px(EMPTY_ICON_SIZE),
                theme.text.secondary.opacity(EMPTY_ICON_OPACITY),
                false,
            ))
            .child(
                MainText::new(tr("chat_linked_group_empty"))
                    .max_w(px(EMPTY_TEXT_MAX_WIDTH))
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary),
            )
    }
}
