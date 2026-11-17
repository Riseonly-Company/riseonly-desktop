use std::rc::Rc;

use gpui::{AnyElement, App, Context, ElementId, Empty, Entity, ListAlignment, Window};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::skeleton_ui::SkeletonShimmerExt;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::chat::components::chat::chat_logo::ChatLogo;
use crate::modules::chat::components::chat_item::chat_item_skeleton::ChatItemSkeleton;
use crate::modules::chat::components::chat_item::chat_list_preview_row::{
    ChatListPreviewFactory, ChatListPreviewRow, ChatListRowMetrics,
};
use crate::modules::chat::components::chat_search::chat_search_section::chat_search_sync_list;
use crate::modules::chat::engine::messages::rise_message_search_session::{ChatMessageSearchPolicy, RiseMessageSearchHit};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;

const FOOTER_EXTRA_INSET: f32 = 20.0;
const EMPTY_SKELETON_ROWS: i64 = 4;
const SEPARATOR_HEIGHT: f32 = 0.5;

fn list_id(suffix: impl Into<String>) -> ElementId {
    ElementId::Name(format!("chat-message-search-list-{}", suffix.into()).into())
}

/// separators of a hit start where the chat row text starts
pub enum ChatMessageSearchRowMetrics {}

impl ChatMessageSearchRowMetrics {
    pub const SEPARATOR_INSET: f32 =
        ChatListRowMetrics::HORIZONTAL_PADDING + ChatListRowMetrics::AVATAR_SIZE + ChatListRowMetrics::LEADING_SPACING;
}

/// one search hit drawn by the chat list own row
/// found message stands where the last message does, thumbnail and ticks and highlights
/// with chat the row is that chat, without it the author
#[derive(IntoElement)]
pub struct ChatMessageSearchResultRow {
    id: ElementId,
    hit: RiseMessageSearchHit,
    chat: Option<ChatInfo>,
    is_current: bool,
    theme: AppTheme,
    on_tap: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl ChatMessageSearchResultRow {
    pub fn new(
        id: impl Into<ElementId>,
        hit: RiseMessageSearchHit,
        chat: Option<ChatInfo>,
        is_current: bool,
        theme: AppTheme,
        on_tap: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), hit, chat, is_current, theme, on_tap: Rc::new(on_tap) }
    }

    fn avatar(hit: &RiseMessageSearchHit, chat: Option<&ChatInfo>) -> AnyElement {
        match chat {
            Some(chat) => ChatLogo::new(list_id(format!("logo-{}", hit.id)), chat.clone())
                .size(ChatListRowMetrics::AVATAR_SIZE)
                .icon_size(22.0)
                .show_online_indicator(false)
                .show_story_ring(false)
                .into_any_element(),
            None => UserLogoUi::new(list_id(format!("author-{}", hit.id)))
                .source(hit.message.sender_logo.clone().map(gpui::SharedString::from))
                .name(gpui::SharedString::from(hit.message.sender_name.clone()))
                .user_id(gpui::SharedString::from(hit.message.sender_id.clone()))
                .size(ChatListRowMetrics::AVATAR_SIZE)
                .into_any_element(),
        }
    }
}

impl RenderOnce for ChatMessageSearchResultRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let presentation = ChatListPreviewFactory::search_hit(&self.hit, self.chat.as_ref(), cx);
        let avatar = Self::avatar(&self.hit, self.chat.as_ref());
        let on_tap = self.on_tap;
        let current_tint = self.theme.primary._100.opacity(0.10);
        plain(self.id)
            .relative()
            .w_full()
            .on_press(move |window, cx| on_tap(window, cx))
            .child(ChatListPreviewRow::new(list_id(format!("row-{}", self.hit.id)), presentation).leading(avatar))
            .children(self.is_current.then(|| div().absolute().inset_0().bg(current_tint)))
    }
}

enum ChatMessageSearchListRow {
    TopInset,
    Empty,
    Skeleton(i64),
    Hit(usize, RiseMessageSearchHit),
    LoadingMore,
    Footer,
}

impl ChatMessageSearchListRow {
    fn id(&self) -> String {
        match self {
            Self::TopInset => "top-inset".to_owned(),
            Self::Empty => "empty".to_owned(),
            Self::Skeleton(rows) => format!("skeleton-{rows}"),
            Self::Hit(_, hit) => format!("hit-{}", hit.id),
            Self::LoadingMore => "loading-more".to_owned(),
            Self::Footer => "footer".to_owned(),
        }
    }
}

struct ChatMessageSearchResultsListState {
    list: ListUi,
    list_ids: Vec<String>,
}

impl ChatMessageSearchResultsListState {
    fn new(cx: &mut Context<Self>) -> Self {
        Self { list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top), list_ids: Vec::new() }
    }
}

/// full screen list of search hits inside one chat (show as list)
/// rows are cheap, snippet and its runs come from the server so a row is two texts and a glyph
#[derive(IntoElement)]
pub struct ChatMessageSearchResultsList {
    id: ElementId,
    hits: Vec<RiseMessageSearchHit>,
    current_index: i64,
    is_loading: bool,
    has_more: bool,
    did_load: bool,
    top_inset: f32,
    bottom_inset: f32,
    on_select: Rc<dyn Fn(i64, &mut Window, &mut App)>,
    on_load_more: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl ChatMessageSearchResultsList {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        hits: Vec<RiseMessageSearchHit>,
        current_index: i64,
        is_loading: bool,
        has_more: bool,
        did_load: bool,
        on_select: impl Fn(i64, &mut Window, &mut App) + 'static,
        on_load_more: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            hits,
            current_index,
            is_loading,
            has_more,
            did_load,
            top_inset: 0.0,
            bottom_inset: 0.0,
            on_select: Rc::new(on_select),
            on_load_more: Rc::new(on_load_more),
        }
    }

    pub fn top_inset(mut self, top_inset: f32) -> Self {
        self.top_inset = top_inset;
        self
    }

    pub fn bottom_inset(mut self, bottom_inset: f32) -> Self {
        self.bottom_inset = bottom_inset;
        self
    }

    fn rows(&self) -> Vec<ChatMessageSearchListRow> {
        let mut rows = vec![ChatMessageSearchListRow::TopInset];
        if self.hits.is_empty() {
            rows.push(if self.did_load && !self.is_loading {
                ChatMessageSearchListRow::Empty
            } else {
                ChatMessageSearchListRow::Skeleton(EMPTY_SKELETON_ROWS)
            });
        } else {
            rows.extend(self.hits.iter().cloned().enumerate().map(|(index, hit)| ChatMessageSearchListRow::Hit(index, hit)));
            if self.is_loading {
                rows.push(ChatMessageSearchListRow::LoadingMore);
            }
        }
        rows.push(ChatMessageSearchListRow::Footer);
        rows
    }
}

fn skeleton(rows: i64) -> AnyElement {
    vstack(0.0)
        .w_full()
        .children((0..rows).map(ChatItemSkeleton::new))
        .skeleton_shimmer(None)
        .into_any_element()
}

fn empty_state(theme: &AppTheme) -> AnyElement {
    vstack(6.0)
        .w_full()
        .pt(px(60.0))
        .children(IconUi::sized("magnifyingglass", px(30.0), theme.text.secondary, false))
        .child(MainText::new(tr("chat_search_no_results")).px(13.0).tac(TextAlignT::Center).color(theme.text.secondary))
        .into_any_element()
}

impl RenderOnce for ChatMessageSearchResultsList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let rows = self.rows();
        let state: Entity<ChatMessageSearchResultsListState> =
            window.use_keyed_state(self.id.clone(), cx, |_, cx| ChatMessageSearchResultsListState::new(cx));
        state.update(cx, |state, _| {
            chat_search_sync_list(&state.list, &mut state.list_ids, rows.iter().map(ChatMessageSearchListRow::id).collect());
        });

        let hits_count = self.hits.len() as i64;
        let (current_index, is_loading, has_more) = (self.current_index, self.is_loading, self.has_more);
        let (top_inset, bottom_inset) = (self.top_inset, self.bottom_inset);
        let (on_select, on_load_more) = (self.on_select, self.on_load_more);
        let row_theme = theme.clone();
        let list = state.read(cx).list.element(move |index, _, _| {
            let Some(row) = rows.get(index) else {
                return Empty.into_any_element();
            };
            match row {
                ChatMessageSearchListRow::TopInset => div().w_full().h(px(top_inset)).into_any_element(),
                ChatMessageSearchListRow::Empty => empty_state(&row_theme),
                ChatMessageSearchListRow::Skeleton(rows) => skeleton(*rows),
                ChatMessageSearchListRow::LoadingMore => skeleton(1),
                ChatMessageSearchListRow::Footer => div().w_full().h(px(bottom_inset + FOOTER_EXTRA_INSET)).into_any_element(),
                ChatMessageSearchListRow::Hit(hit_index, hit) => {
                    let hit_index = *hit_index as i64;
                    let on_select = on_select.clone();
                    let on_load_more = on_load_more.clone();
                    vstack(0.0)
                        .w_full()
                        .child(
                            ChatMessageSearchResultRow::new(
                                list_id(format!("hit-{}", hit.id)),
                                hit.clone(),
                                None,
                                hit_index == current_index,
                                row_theme.clone(),
                                move |window, cx| on_select(hit_index, window, cx),
                            )
                            .on_appear(list_id(format!("hit-appear-{}", hit.id)), move |window, cx| {
                                if ChatMessageSearchPolicy::should_load_more(hit_index, hits_count, has_more, is_loading) {
                                    on_load_more(window, cx);
                                }
                            }),
                        )
                        .child(
                            div().w_full().pl(px(ChatMessageSearchRowMetrics::SEPARATOR_INSET)).child(
                                div().w_full().h(px(SEPARATOR_HEIGHT)).bg(row_theme.border._100),
                            ),
                        )
                        .into_any_element()
                }
            }
        });
        div().size_full().bg(theme.bg._100).child(list)
    }
}
