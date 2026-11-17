use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{AnyElement, App, Context, ElementId, Entity, FontWeight, Hsla, SharedString, Subscription, Window};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::MainText;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::{MotionSpec, ease_out, spring};
use rise_ui::swiftui::prelude::*;

use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::{SaiDebouncer, SaiDebouncerOptions};
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::chat_logo::ChatLogo;
use crate::modules::chat::components::folders::chat_folder_edit_sheet::{
    FolderPresenceAxis, FolderPresenceList, FolderPresenceValue, FolderSection, folder_chat_display_name,
    folder_presence_rows, folder_presence_slots,
};
use crate::modules::chat::components::folders::chat_folder_editor_store::{AddMode, ChatFolderEditorStore};
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_folder_types::{FolderChatType, FolderChatTypeExt};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::music::widgets::bottomsheets::track_picker::track_picker_shared::TrackPickerListModel;

const SEARCH_DEBOUNCE_ID: &str = "folderChatSearch";
const SEARCH_DEBOUNCE_MILLISECONDS: i64 = 400;
const PREFETCH_THRESHOLD: usize = 8;
const SECTION_CORNER_RADIUS: f32 = 22.0;

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

/// state of the content, plus the lazy stack of its rows
struct ChatFolderAddChatsState {
    query: String,
    list_model: TrackPickerListModel,
    selection_motion: MotionSpec,
    _subscriptions: Vec<Subscription>,
}

impl ChatFolderAddChatsState {
    fn new(store: &Entity<ChatFolderEditorStore>, cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        let subscriptions = vec![
            cx.observe(store, |_, _, cx| cx.notify()),
            cx.observe(&services, |_, _, cx| cx.notify()),
        ];
        Self {
            query: String::new(),
            list_model: TrackPickerListModel::new(cx),
            selection_motion: spring(0.3, 0.85),
            _subscriptions: subscriptions,
        }
    }
}

/// the chats section plus the sections above it, flattened
/// gpui list keeps only rows on screen so the whole scrolling column has to be one list
enum ChatFolderAddChatsRow {
    Types,
    ChatsTitle,
    Chat { chat: ChatInfo, index: usize, count: usize },
    NoChats,
    BottomSpace,
}

impl ChatFolderAddChatsRow {
    fn id(&self) -> SharedString {
        match self {
            Self::Types => SharedString::new_static("types"),
            Self::ChatsTitle => SharedString::new_static("chats-title"),
            Self::Chat { chat, .. } => SharedString::from(format!("chat-{}", chat.id)),
            Self::NoChats => SharedString::new_static("no-chats"),
            Self::BottomSpace => SharedString::new_static("bottom-space"),
        }
    }
}

#[derive(Clone)]
struct ChatFolderAddChatsPaging {
    is_searching: bool,
    cache_key: SharedString,
    relative_id: Option<String>,
    has_more: bool,
}

#[derive(IntoElement)]
pub struct ChatFolderAddChatsContent {
    id: ElementId,
}

impl ChatFolderAddChatsContent {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into() }
    }
}

impl RenderOnce for ChatFolderAddChatsContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let store = ChatFolderEditorStore::shared(cx);
        let observed = store.clone();
        let state = window.use_keyed_state(
            named_child(&self.id, "state"),
            cx,
            move |_, cx: &mut Context<ChatFolderAddChatsState>| ChatFolderAddChatsState::new(&observed, cx),
        );
        let parts = ChatFolderAddChatsParts { id: self.id.clone(), store, state };
        parts.content(&theme, window, cx)
    }
}

#[derive(Clone)]
struct ChatFolderAddChatsParts {
    id: ElementId,
    store: Entity<ChatFolderEditorStore>,
    state: Entity<ChatFolderAddChatsState>,
}

impl ChatFolderAddChatsParts {
    fn content(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let add_mode = self.store.read(cx).add_mode;
        let reset_query = self.state.clone();
        on_change(
            named_child(&self.id, "add-mode"),
            add_mode,
            false,
            window,
            cx,
            move |_, mode: &Option<AddMode>, _, cx| {
                if mode.is_some() {
                    reset_query.update(cx, |state, cx| {
                        state.query = String::new();
                        cx.notify();
                    });
                }
            },
        );

        let query = self.state.read(cx).query.clone();
        on_change(
            named_child(&self.id, "query"),
            query,
            false,
            window,
            cx,
            move |_, query: &String, _, cx| {
                let query = query.clone();
                let _ = SaiDebouncer::debounced_action_id(
                    SEARCH_DEBOUNCE_ID,
                    Box::new(move |cx: &mut App| {
                        ChatActionsStore::shared(cx)
                            .update(cx, |store, cx| store.search_folder_chats_action(&query, None, cx));
                    }),
                    SaiDebouncerOptions {
                        delay_milliseconds: SEARCH_DEBOUNCE_MILLISECONDS,
                        group_key: SEARCH_DEBOUNCE_ID.to_owned(),
                        ..SaiDebouncerOptions::default()
                    },
                    cx,
                );
            },
        );

        let title = tr(self.store.read(cx).add_mode_title_key());
        let left = self.store.clone();
        let right = self.store.clone();
        let header = SheetHeaderBar::new(named_child(&self.id, "header"), title, move |_, cx| {
            left.update(cx, |store, cx| store.close_add_chats(cx));
        })
        .on_right(move |_, cx| {
            right.update(cx, |store, cx| store.close_add_chats(cx));
        })
        .right_guide_anchor(Some(RiseGuideAnchor::FolderPickerDone));

        let binding = Binding::entity(
            self.state.clone(),
            |state: &ChatFolderAddChatsState| state.query.clone(),
            |state, query, _| state.query = query,
        );
        let search = div().w_full().px(px(16.0)).pb(px(8.0)).child(
            SearchFieldUi::new(named_child(&self.id, "search"), binding)
                .placeholder(Some(SharedString::from(tr("folder_search_chats_placeholder"))))
                .uses_inline_text_field(true),
        );

        vstack(0.0)
            .size_full()
            .bg(theme.bg._100)
            .child(header)
            .children(self.chips(theme, window, cx))
            .child(search)
            .child(div().w_full().flex_1().min_h_0().child(self.rows(theme, cx)))
            .on_appear(named_child(&self.id, "appear"), |_, cx| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_initial_chats(cx));
            })
            .into_any_element()
    }

    fn query(&self, cx: &App) -> String {
        self.state.read(cx).query.clone()
    }

    fn is_searching(&self, cx: &App) -> bool {
        !trimming_whitespaces_and_newlines(&self.query(cx)).is_empty()
    }

    fn chips(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let (types, ids) = {
            let store = self.store.read(cx);
            (store.active_selected_types(), store.active_selected_chat_ids())
        };
        let chats = self.resolved_chats(&ids, cx);
        let has_chips = !types.is_empty() || !chats.is_empty();
        let current = folder_presence_rows(types, chats);
        let bar_current = if has_chips {
            vec![(SharedString::new_static("bar"), FolderPresenceValue::Type(String::new()))]
        } else {
            Vec::new()
        };
        let now = RiseClock::monotonic();
        let spec = self.state.read(cx).selection_motion;
        let list = window.use_keyed_state(named_child(&self.id, "chips-presence"), cx, |_, _| {
            FolderPresenceList::default()
        });
        list.update(cx, |list, cx| list.sync(current, spec, now, cx));
        let bar = window.use_keyed_state(named_child(&self.id, "chips-bar-presence"), cx, |_, _| {
            FolderPresenceList::default()
        });
        bar.update(cx, |bar, cx| bar.sync(bar_current, spec, now, cx));

        let chips_id = named_child(&self.id, "chips-slots");
        let chips = folder_presence_slots(
            &chips_id,
            &list,
            FolderPresenceAxis::Horizontal,
            8.0,
            now,
            window,
            cx,
            |value| {
                let store = self.store.clone();
                match value {
                    FolderPresenceValue::Type(raw) => {
                        let title =
                            tr(FolderChatType::from_wire(raw).map_or(raw.as_str(), |value| value.title_key()));
                        let name = SharedString::from(format!("chip-type-{raw}"));
                        let raw = raw.clone();
                        self.chip(name, title, theme, move |cx| {
                            store.update(cx, |store, cx| store.toggle_type(&raw, cx));
                        })
                    }
                    FolderPresenceValue::Chat(chat) => {
                        let name = SharedString::from(format!("chip-chat-{}", chat.id));
                        let title = folder_chat_display_name(chat);
                        let chat_id = chat.id.clone();
                        self.chip(name, title, theme, move |cx| {
                            store.update(cx, |store, cx| store.toggle_chat(&chat_id, cx));
                        })
                    }
                }
            },
        );
        let mut strip = Some(
            div()
                .id(named_child(&self.id, "chips"))
                .w_full()
                .flex()
                .flex_none()
                .overflow_x_scroll()
                .child(hstack(0.0).flex_none().items_center().px(px(16.0)).py(px(10.0)).children(chips))
                .into_any_element(),
        );
        let bar_id = named_child(&self.id, "chips-bar-slots");
        folder_presence_slots(&bar_id, &bar, FolderPresenceAxis::Vertical, 0.0, now, window, cx, |_| {
            strip.take().unwrap_or_else(|| div().into_any_element())
        })
        .into_iter()
        .next()
    }

    fn chip(
        &self,
        name: SharedString,
        text: String,
        theme: &AppTheme,
        on_remove: impl Fn(&mut App) + 'static,
    ) -> AnyElement {
        let content = theme.content_on(theme.primary._100);
        let state = self.state.clone();
        plain(named_child(&self.id, name))
            .flex_none()
            .on_press(move |_, cx| {
                let _ = selection_changed();
                state.update(cx, |state, _| state.selection_motion = spring(0.3, 0.85));
                on_remove(cx);
            })
            .child(
                hstack(6.0)
                    .items_center()
                    .pl(px(5.0))
                    .pr(px(12.0))
                    .py(px(5.0))
                    .bg(theme.primary._100)
                    .capsule()
                    .child(
                        div()
                            .flex_none()
                            .w(px(18.0))
                            .h(px(18.0))
                            .circle()
                            .bg(content.opacity(0.25))
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized("xmark", px(11.0), content, false)),
                    )
                    .child(
                        MainText::new(text)
                            .px(14.0)
                            .font_weight(FontWeight::MEDIUM)
                            .color(content)
                            .number_of_lines(1),
                    ),
            )
            .into_any_element()
    }

    fn types_section(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let count = FolderChatType::ALL.len();
        let mut rows: Vec<AnyElement> = Vec::with_capacity(count * 2);
        for (index, folder_type) in FolderChatType::ALL.iter().copied().enumerate() {
            let raw = folder_type.wire();
            let store = self.store.clone();
            let selected = self.store.read(cx).is_selected_type(raw);
            let leading = div()
                .flex_none()
                .w(px(36.0))
                .h(px(36.0))
                .circle()
                .bg(folder_type.color())
                .flex()
                .items_center()
                .justify_center()
                .children(IconUi::sized(
                    folder_type.system_image(),
                    px(16.0),
                    theme.content_on(folder_type.color()),
                    folder_type.system_image().ends_with(".fill"),
                ))
                .into_any_element();
            rows.push(self.select_row(
                SharedString::from(format!("type-{raw}")),
                selected,
                leading,
                tr(folder_type.title_key()),
                theme,
                window,
                cx,
                move |cx| store.update(cx, |store, cx| store.toggle_type(raw, cx)),
            ));
            if index < count - 1 {
                rows.push(Self::divider(theme));
            }
        }
        FolderSection::new(
            tr("folder_types_section"),
            vstack(0.0)
                .w_full()
                .children(rows)
                .guide_anchor(RiseGuideAnchor::FolderPickerTypes)
                .into_any_element(),
        )
        .into_any_element()
    }

    fn rows(&self, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let chats = self.filtered_chats(cx);
        let is_searching = self.is_searching(cx);
        let query = self.query(cx);
        let trimmed_query = trimming_whitespaces_and_newlines(&query).to_owned();
        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        let services = ChatServicesStore::shared(cx);
        let paging = if is_searching {
            let page = services.update(cx, |services, cx| services.folder_chat_search_page(&trimmed_query, &user_id, cx));
            ChatFolderAddChatsPaging {
                is_searching,
                cache_key: SharedString::from(ChatServicesStore::folder_chat_search_cache_key(
                    &user_id,
                    &trimmed_query,
                )),
                relative_id: page.as_ref().and_then(|page| page.relative_id.clone()),
                has_more: page.as_ref().is_some_and(|page| page.is_have_more == Some(true)),
            }
        } else {
            let services = services.read(cx);
            ChatFolderAddChatsPaging {
                is_searching,
                cache_key: SharedString::from(services.chats_cache_key(cx).unwrap_or_default()),
                relative_id: services.chats_relative_id(cx),
                has_more: services.chats_has_more(cx),
            }
        };

        let mut rows: Vec<ChatFolderAddChatsRow> = Vec::with_capacity(chats.len() + 3);
        if !is_searching {
            rows.push(ChatFolderAddChatsRow::Types);
        }
        if chats.is_empty() {
            rows.push(ChatFolderAddChatsRow::NoChats);
        } else {
            rows.push(ChatFolderAddChatsRow::ChatsTitle);
            let count = chats.len();
            rows.extend(chats.into_iter().enumerate().map(|(index, chat)| ChatFolderAddChatsRow::Chat {
                chat,
                index,
                count,
            }));
        }
        rows.push(ChatFolderAddChatsRow::BottomSpace);

        let ids: Vec<SharedString> = rows.iter().map(ChatFolderAddChatsRow::id).collect();
        self.state.update(cx, |state, _| state.list_model.sync(ids));

        let parts = self.clone();
        let theme = theme.clone();
        self.state
            .read(cx)
            .list_model
            .list()
            .element(move |index, window, cx| match rows.get(index) {
                Some(row) => parts.row(row, &paging, &theme, window, cx),
                None => div().into_any_element(),
            })
            .into_any_element()
    }

    fn load_more_chats(&self, cx: &mut App) {
        let query = self.query(cx);
        let is_searching = self.is_searching(cx);
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            if is_searching {
                store.load_more_folder_chat_search(&query, cx);
            } else {
                store.load_more_chats(cx);
            }
        });
    }

    fn row(
        &self,
        row: &ChatFolderAddChatsRow,
        paging: &ChatFolderAddChatsPaging,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let element = match row {
            ChatFolderAddChatsRow::Types => {
                div().w_full().px(px(16.0)).pt(px(6.0)).pb(px(22.0)).child(self.types_section(theme, window, cx))
            }
            ChatFolderAddChatsRow::ChatsTitle => div().w_full().px(px(16.0)).when(paging.is_searching, |title| title.pt(px(6.0))).pb(px(8.0)).child(
                div().pl(px(14.0)).child(
                    MainText::new(tr("folder_chats_section").to_uppercase())
                        .px(12.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.secondary),
                ),
            ),
            ChatFolderAddChatsRow::NoChats => {
                let load = self.clone();
                return div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .pt(px(if paging.is_searching { 34.0 } else { 28.0 }))
                    .child(MainText::new(tr("no_chats")).px(14.0).color(theme.text.secondary))
                    .sai_pagination_drain_filtered_empty(
                        named_child(&self.id, "drain-filtered-empty"),
                        SaiPaginationPrefetchOptions::drain_filtered_empty(
                            0,
                            paging.cache_key.clone(),
                            paging.relative_id.clone(),
                            paging.has_more,
                        ),
                        move |_, cx| load.load_more_chats(cx),
                    )
                    .into_any_element();
            }
            ChatFolderAddChatsRow::BottomSpace => div().w_full().h(px(62.0)),
            ChatFolderAddChatsRow::Chat { chat, index, count } => {
                return self.chat_row(chat, *index, *count, paging, theme, window, cx);
            }
        };
        element.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn chat_row(
        &self,
        chat: &ChatInfo,
        index: usize,
        count: usize,
        paging: &ChatFolderAddChatsPaging,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let store = self.store.clone();
        let chat_id = chat.id.clone();
        let selected = self.store.read(cx).is_selected_chat(&chat.id);
        let leading = ChatLogo::new(named_child(&self.id, SharedString::from(format!("logo-{}", chat.id))), chat.clone())
            .size(36.0)
            .icon_size(16.0)
            .show_online_indicator(false)
            .into_any_element();
        let row = self.select_row(
            SharedString::from(format!("chat-{}", chat.id)),
            selected,
            leading,
            folder_chat_display_name(chat),
            theme,
            window,
            cx,
            move |cx| store.update(cx, |store, cx| store.toggle_chat(&chat_id, cx)),
        );
        let is_first = index == 0;
        let is_last = index + 1 == count;
        let radius = px(SECTION_CORNER_RADIUS);
        let load = self.clone();
        div()
            .w_full()
            .px(px(16.0))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .bg(theme.bg._200)
                    .overflow_hidden()
                    .when(is_first, |section| section.rounded_t(radius))
                    .when(is_last, |section| section.rounded_b(radius))
                    .child(row)
                    .when(!is_last, |section| section.child(Self::divider(theme))),
            )
            .sai_pagination_prefetch(
                named_child(&self.id, SharedString::from(format!("prefetch-{}", chat.id))),
                SaiPaginationPrefetchOptions::new(
                    index,
                    count,
                    paging.cache_key.clone(),
                    paging.relative_id.clone(),
                    paging.has_more,
                )
                .threshold(PREFETCH_THRESHOLD),
                move |_, cx| load.load_more_chats(cx),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn select_row(
        &self,
        name: SharedString,
        selected: bool,
        leading: AnyElement,
        title: String,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
        action: impl Fn(&mut App) + 'static,
    ) -> AnyElement {
        let motion_state = self.state.clone();
        let now = RiseClock::monotonic();
        let fraction = window.use_keyed_state(
            named_child(&self.id, SharedString::from(format!("{name}-fraction"))),
            cx,
            |_, _| Animated::new(if selected { 1.0_f32 } else { 0.0_f32 }),
        );
        fraction.update(cx, |fraction, cx| {
            fraction.set_in(if selected { 1.0 } else { 0.0 }, ease_out(0.16), now, cx);
        });
        let shown = fraction.read(cx).frame_value(now, window);
        let highlight: Hsla = theme.primary._100.opacity(0.12 * shown);
        let icon_color = if selected { theme.primary._100 } else { theme.text.secondary.opacity(0.4) };
        let icon = if selected { "checkmark.circle.fill" } else { "circle" };
        plain(named_child(&self.id, name))
            .w_full()
            .on_press(move |_, cx| {
                let _ = selection_changed();
                motion_state.update(cx, |state, _| state.selection_motion = ease_out(0.16));
                action(cx);
            })
            .child(
                hstack(12.0)
                    .w_full()
                    .items_center()
                    .px(px(14.0))
                    .py(px(9.0))
                    .bg(highlight)
                    .children(IconUi::sized(icon, px(22.0), icon_color, false))
                    .child(leading)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(MainText::new(title).px(16.0).color(theme.text.primary).number_of_lines(1)),
                    ),
            )
            .into_any_element()
    }

    fn divider(theme: &AppTheme) -> AnyElement {
        div().w_full().pl(px(84.0)).child(div().w_full().h(px(0.5)).bg(theme.border._100)).into_any_element()
    }

    fn filtered_chats(&self, cx: &mut App) -> Vec<ChatInfo> {
        let services = ChatServicesStore::shared(cx);
        let all = services.read(cx).current_chat_infos(cx);
        let query = self.query(cx);
        let needle = trimming_whitespaces_and_newlines(&query).to_lowercase();
        if needle.is_empty() {
            return all;
        }
        let matches = |chat: &ChatInfo| folder_chat_display_name(chat).to_lowercase().contains(&needle);
        let local_matches: Vec<ChatInfo> = all.into_iter().filter(|chat| matches(chat)).collect();
        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        let backend = services.update(cx, |services, cx| services.folder_chat_search_results(&needle, &user_id, cx));
        let mut seen: HashSet<String> = HashSet::new();
        local_matches
            .into_iter()
            .chain(backend.into_iter().filter(|chat| matches(chat)))
            .filter(|chat| seen.insert(chat.id.clone()))
            .collect()
    }

    fn resolved_chats(&self, ids: &[String], cx: &mut App) -> Vec<ChatInfo> {
        let services = ChatServicesStore::shared(cx);
        let mut all = services.read(cx).current_chat_infos(cx);
        all.extend(services.update(cx, |services, cx| services.cached_folder_chat_search_results(cx)));
        let mut by_id: HashMap<&str, &ChatInfo> = HashMap::with_capacity(all.len());
        for chat in &all {
            by_id.entry(chat.id.as_str()).or_insert(chat);
        }
        let cx: &App = cx;
        let store = self.store.read(cx);
        let services = services.read(cx);
        ids.iter()
            .filter_map(|id| {
                by_id
                    .get(id.as_str())
                    .map(|chat| (*chat).clone())
                    .or_else(|| store.selected_chat_models().get(id).cloned())
                    .or_else(|| services.chat_info(id, cx))
            })
            .collect()
    }
}
