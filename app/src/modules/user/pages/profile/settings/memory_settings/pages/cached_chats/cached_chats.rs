use std::rc::Rc;

use gpui::{App, Context, ElementId, FontWeight, Hsla, ListAlignment, ListState, SharedString, Subscription, Window, list, white};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::skeleton_ui::{SkeletonLine, SkeletonListUi, SkeletonShimmerExt};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::storage::rise_storage_usage_models::{RiseStorageChatUsage, RiseStorageSite};
use crate::core::storage::rise_storage_usage_presentation::RiseStorageUsagePresentation;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_under_chrome_box};
use crate::modules::post::pages::posts::post_detail::profile_post_list_body::splice_list_rows;
use crate::modules::user::stores::user_storage::user_storage_interactions::user_storage_interactions::UserStorageInteractionsStore;
use crate::modules::user::stores::user_storage::user_storage_services::user_storage_services::UserStorageServicesStore;

const LIST_OVERDRAW: f32 = 400.0;

struct CachedChatsState {
    list: ListState,
    rendered_rows: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl CachedChatsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseStorageUsagePresentation::shared(cx);
        Self {
            list: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
            rendered_rows: Vec::new(),
            _subscriptions: vec![cx.observe(&presentation, |_, _, cx| cx.notify())],
        }
    }
}

#[derive(Clone)]
enum CachedChatsRow {
    Header,
    Chat(RiseStorageChatUsage),
    SharedPayloads,
    BottomSpacer,
}

impl CachedChatsRow {
    fn id(&self) -> String {
        match self {
            Self::Header => "header".to_string(),
            Self::Chat(row) => format!("chat:{}", row.id()),
            Self::SharedPayloads => "shared".to_string(),
            Self::BottomSpacer => "spacer".to_string(),
        }
    }
}

fn row_id(name: &str, id: &str) -> ElementId {
    SharedString::from(format!("{name}-{id}")).into()
}

fn symbol(name: &str, side: f32, color: Hsla) -> AnyElement {
    let filled = name.ends_with(".fill");
    match IconUi::sized(name, px(side), color, filled).or_else(|| IconUi::sized(name, px(side), color, !filled)) {
        Some(icon) => icon.into_any_element(),
        None => div().size(px(side)).into_any_element(),
    }
}

#[derive(IntoElement)]
pub struct CachedChats;

impl CachedChats {
    pub fn new() -> Self {
        Self
    }

    fn screen_status(status: SaiWsSliceStatus, chats_status: SaiWsSliceStatus) -> SaiWsSliceStatus {
        if chats_status == SaiWsSliceStatus::Fulfilled { status } else { chats_status }
    }

    fn loading() -> AnyElement {
        scroll_view("cached-chats-loading")
            .size_full()
            .scroll_disabled(true)
            .child(
                vstack(0.0)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .child(
                        vstack(2.0)
                            .align(HorizontalAlignment::Leading)
                            .px(px(16.0))
                            .pt(px(12.0))
                            .pb(px(10.0))
                            .child(SkeletonLine::new(92.0, 16.0))
                            .child(SkeletonLine::new(190.0, 9.0))
                            .skeleton_shimmer(None),
                    )
                    .child(SkeletonListUi::new(8, SkeletonListUi::default().avatar_size)),
            )
            .into_any_element()
    }

    fn header(total_bytes: i64, theme: &AppTheme) -> AnyElement {
        vstack(2.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .px(px(16.0))
            .pt(px(12.0))
            .pb(px(10.0))
            .bg(theme.bg._100)
            .child(MainText::new(RiseFormat::file_size(total_bytes)).px(20.0).font_weight(FontWeight::SEMIBOLD))
            .child(SecondaryText::new(tr("memory_settings_chat_cache_hint")).px(12.0))
            .into_any_element()
    }

    fn chat_row(row: &RiseStorageChatUsage, is_clearing: bool, theme: &AppTheme) -> AnyElement {
        let chat_id = row.chat_id.clone();
        hstack(12.0)
            .w_full()
            .px(px(12.0))
            .py(px(10.0))
            .bg(theme.bg._100)
            .child(
                UserLogoUi::new(row_id("cached-chat-logo", &row.chat_id))
                    .name(SharedString::from(row.title.clone()))
                    .user_id(SharedString::from(row.chat_id.clone()))
                    .color_scope_id(SharedString::from(row.chat_id.clone()))
                    .size(40.0)
                    .show_story_ring(false),
            )
            .child(
                vstack(2.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w_0()
                    .child(MainText::new(row.title.clone()).px(14.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
                    .child(MainText::new(RiseFormat::file_size(row.total_bytes())).px(11.0).color(theme.text.secondary)),
            )
            .child(Self::trash_button(row_id("cached-chat-trash", &row.chat_id), is_clearing, theme, move |cx| {
                let chat_id = chat_id.clone();
                UserStorageInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_chat(chat_id, cx));
            }))
            .into_any_element()
    }

    fn shared_payloads_row(bytes: i64, is_clearing: bool, theme: &AppTheme) -> AnyElement {
        hstack(12.0)
            .w_full()
            .px(px(12.0))
            .py(px(10.0))
            .bg(theme.bg._100)
            .child(
                div()
                    .size(px(40.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(SettingsIconPalette::memory())
                    .child(symbol(RiseStorageSite::WsPayloads.icon_name(), 16.0, white())),
            )
            .child(
                vstack(2.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w_0()
                    .child(
                        MainText::new(tr(RiseStorageSite::WsPayloads.i18n_key()))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .number_of_lines(1),
                    )
                    .child(MainText::new(RiseFormat::file_size(bytes)).px(11.0).color(theme.text.secondary)),
            )
            .child(Self::trash_button("cached-chats-shared-trash".into(), is_clearing, theme, |cx| {
                UserStorageInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_site(RiseStorageSite::WsPayloads, cx));
            }))
            .into_any_element()
    }

    fn trash_button(id: ElementId, is_clearing: bool, theme: &AppTheme, action: impl Fn(&mut App) + 'static) -> AnyElement {
        plain(id)
            .disabled(is_clearing)
            .on_press(move |_, cx| action(cx))
            .size(px(34.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .child(symbol("trash", 17.0, theme.semantic.error_100))
            .into_any_element()
    }
}

impl Default for CachedChats {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for CachedChats {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("cached-chats-state", cx, |_, cx| CachedChatsState::new(cx));
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let storage = UserStorageServicesStore::shared(cx);
        let (total_bytes, shared_bytes, chats, status, chats_status, is_clearing) = storage.update(cx, |storage, cx| {
            (
                storage.chat_cache_bytes(cx),
                storage.unattributed_payload_bytes(cx),
                storage.chats(cx),
                storage.status(cx),
                storage.chats_status(cx),
                storage.is_clearing(cx),
            )
        });
        let is_empty = chats.is_empty() && total_bytes == 0;

        let mut rows = Vec::with_capacity(chats.len() + 3);
        rows.push(CachedChatsRow::Header);
        rows.extend(chats.into_iter().map(CachedChatsRow::Chat));
        if shared_bytes > 0 {
            rows.push(CachedChatsRow::SharedPayloads);
        }
        rows.push(CachedChatsRow::BottomSpacer);
        let row_ids: Vec<String> = rows.iter().map(CachedChatsRow::id).collect();
        let list_state = state.update(cx, |state, _| {
            splice_list_rows(&state.list, &mut state.rendered_rows, &row_ids);
            state.list.clone()
        });

        let rows = Rc::new(rows);
        let row_theme = theme.clone();
        let content = move |_: &mut Window, _: &mut App| {
            list(list_state, move |index, _, _| match rows.get(index) {
                Some(CachedChatsRow::Header) => div().w_full().pb(px(1.0)).child(Self::header(total_bytes, &row_theme)).into_any_element(),
                Some(CachedChatsRow::Chat(row)) => {
                    div().w_full().pb(px(1.0)).child(Self::chat_row(row, is_clearing, &row_theme)).into_any_element()
                }
                Some(CachedChatsRow::SharedPayloads) => {
                    div().w_full().pb(px(1.0)).child(Self::shared_payloads_row(shared_bytes, is_clearing, &row_theme)).into_any_element()
                }
                Some(CachedChatsRow::BottomSpacer) | None => div().w_full().h(px(80.0)).into_any_element(),
            })
            .size_full()
        };

        SettingsWrapperUi::new(tr("memory_settings_cached_chats"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(
            list_shell_under_chrome_box().size_full().bg(theme.bg._100).child(
                SaiDataRender::new(Self::screen_status(status, chats_status), content)
                    .id("cached-chats-data")
                    .is_empty(is_empty)
                    .no_data_text(tr("memory_settings_nothing_cached"))
                    .no_data_height_percent(40.0)
                    .sai_data_render_loading(|_, _| Self::loading()),
            ),
        )
        .on_appear("cached-chats-appear", |_, cx| {
            UserStorageInteractionsStore::shared(cx).update(cx, |store, cx| store.open_cached_chats(cx));
        })
    }
}
