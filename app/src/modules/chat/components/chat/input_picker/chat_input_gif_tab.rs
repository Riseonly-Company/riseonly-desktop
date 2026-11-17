use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, ElementId, Entity, IntoElement, RenderOnce, ScrollHandle, SharedString, Subscription, Window, white};
use rise_i18n::tr;
use rise_platform::haptics::{ImpactStyle, impact, selection_changed};
use rise_theme::ThemeStore;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::MainText;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::swiftui::prelude::*;

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit::{
    ChatInputPickerFlatChromeKey, ChatInputPickerMetrics, ChatInputSectionHeader, chat_input_picker_bar_reserve,
};
use crate::modules::chat::components::chat::input_picker::gif_grid::GifGrid;
use crate::modules::chat::components::chat::input_picker::sticker_hold_preview::StickerPreviewAction;
use crate::modules::stickers::engine::rise_sticker_models::RiseStickerResourceID;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::GifItem;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

type GifHandler = Rc<dyn Fn(GifItem, &mut Window, &mut App)>;
type OpenHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// runs action while the end of its section is within one viewport of the scroller bottom
/// runs after the frame on each section frame change, so action must be idempotent
pub(crate) fn gif_reach_end_probe(
    id: impl Into<ElementId>,
    scroll: ScrollHandle,
    action: impl FnOnce(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    measure(id, move |bounds, window, cx| {
        let viewport = scroll.bounds();
        if f32::from(viewport.size.height) <= 0.0 {
            return;
        }
        if bounds.bottom() <= viewport.bottom() + viewport.size.height {
            action(window, cx);
        }
    })
}

fn user_id(cx: &mut App) -> String {
    AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default()
}

fn trending_next_cursor(services: &StickersServicesStore) -> Option<String> {
    ChatInputGifPaginationGate::next_cursor(services.gif_search_has_more(""), services.gif_search_next_relative_id(""))
}

struct ChatInputGifTabState {
    trending_pagination: ChatInputGifPaginationGate,
    scroll: ScrollHandle,
    _subscription: Subscription,
}

impl ChatInputGifTabState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = StickersServicesStore::shared(cx);
        Self {
            trending_pagination: ChatInputGifPaginationGate::default(),
            scroll: ScrollHandle::new(),
            _subscription: cx.observe(&services, |_, _, cx| cx.notify()),
        }
    }
}

#[derive(IntoElement)]
pub struct ChatInputGifTab {
    id: ElementId,
    on_send_gif: GifHandler,
    on_open_gif_search: OpenHandler,
    is_active: bool,
    gif_search_active: bool,
    uses_flat_chrome: bool,
}

impl ChatInputGifTab {
    pub fn new(
        id: impl Into<ElementId>,
        on_send_gif: impl Fn(GifItem, &mut Window, &mut App) + 'static,
        on_open_gif_search: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            on_send_gif: Rc::new(on_send_gif),
            on_open_gif_search: Rc::new(on_open_gif_search),
            is_active: true,
            gif_search_active: false,
            uses_flat_chrome: false,
        }
    }

    pub fn is_active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }

    /// no matched geometry in gpui so the namespace has nothing to key
    pub fn gif_search_namespace(self, _gif_search_namespace: Option<ElementId>) -> Self {
        self
    }

    pub fn gif_search_active(mut self, gif_search_active: bool) -> Self {
        self.gif_search_active = gif_search_active;
        self
    }

    pub fn uses_flat_chrome(mut self, uses_flat_chrome: bool) -> Self {
        self.uses_flat_chrome = uses_flat_chrome;
        self
    }

    fn child_id(&self, part: impl Into<SharedString>) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), part.into())
    }

    fn search_bar(&self) -> AnyElement {
        let bar = div().w_full().flex_none().px(px(10.)).pt(px(14.)).pb(px(6.));
        if self.gif_search_active {
            return bar.child(div().h(px(42.))).into_any_element();
        }
        let on_open = self.on_open_gif_search.clone();
        bar.child(
            SearchFieldUi::new(self.child_id("search"), Binding::constant(String::new()))
                .placeholder(SharedString::from(tr("gif_search_placeholder")))
                .on_activate(move |window, cx| {
                    let _ = selection_changed();
                    on_open(window, cx);
                })
                .uses_flat_chrome(self.uses_flat_chrome),
        )
        .into_any_element()
    }

    fn send_handler(&self) -> impl Fn(&GifItem, &mut Window, &mut App) + 'static + use<> {
        let on_send = self.on_send_gif.clone();
        move |gif: &GifItem, window: &mut Window, cx: &mut App| {
            let _ = impact(ImpactStyle::Light);
            on_send(gif.clone(), window, cx);
        }
    }

    fn grid(
        &self,
        state: &Entity<ChatInputGifTabState>,
        saved: Vec<GifItem>,
        popular: Vec<GifItem>,
        user_id: String,
        cx: &mut App,
    ) -> AnyElement {
        let scroll = state.read(cx).scroll.clone();
        let picker_bar_reserve = chat_input_picker_bar_reserve(cx);
        let mut content = div()
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .px(px(ChatInputPickerMetrics::GRID_SPACING))
            .pb(px(ChatInputPickerMetrics::GRID_BOTTOM_INSET + picker_bar_reserve));

        if !saved.is_empty() {
            if !popular.is_empty() {
                content = content.child(ChatInputSectionHeader::new(self.child_id("saved-header"), tr("stickers_recent_header")));
            }
            let remove_user = user_id.clone();
            content = content.child(
                div()
                    .relative()
                    .w_full()
                    .child(
                        GifGrid::new(self.child_id("saved"), saved, self.send_handler())
                            .spacing(ChatInputPickerMetrics::GRID_SPACING)
                            .is_playing(self.is_active)
                            .uses_flat_chrome(self.uses_flat_chrome)
                            .menu_for(move |gif: &GifItem, _: &App| {
                                let user_id = remove_user.clone();
                                let gif = gif.clone();
                                vec![
                                    StickerPreviewAction::new(
                                        "remove",
                                        tr("gif_unsave"),
                                        "bookmark.slash",
                                        Rc::new(move |_: &mut Window, cx: &mut App| {
                                            if user_id.is_empty() {
                                                return;
                                            }
                                            StickersInteractionsStore::shared(cx)
                                                .update(cx, |store, cx| store.toggle_gif_favourite(&user_id, &gif, cx));
                                        }),
                                    )
                                    .destructive(true),
                                ]
                            }),
                    )
                    .child(gif_reach_end_probe(self.child_id("saved-end"), scroll.clone(), move |_, cx| {
                        Self::load_more(cx);
                    })),
            );
        }

        if !popular.is_empty() {
            content = content.child(ChatInputSectionHeader::new(self.child_id("popular-header"), tr("contextMenu_popular")));
            let save_user = user_id;
            content = content.child(
                GifGrid::new(self.child_id("popular"), popular, self.send_handler())
                    .spacing(ChatInputPickerMetrics::GRID_SPACING)
                    .is_playing(self.is_active)
                    .uses_flat_chrome(self.uses_flat_chrome)
                    .menu_for(move |gif: &GifItem, _: &App| {
                        let user_id = save_user.clone();
                        let gif = gif.clone();
                        vec![StickerPreviewAction::new(
                            "save",
                            tr("gif_save"),
                            "bookmark",
                            Rc::new(move |_: &mut Window, cx: &mut App| {
                                if user_id.is_empty() {
                                    return;
                                }
                                StickersInteractionsStore::shared(cx)
                                    .update(cx, |store, cx| store.add_gif_favourite(&user_id, &gif, cx));
                            }),
                        )]
                    }),
            );
        }

        content = content.children(self.trending_continuation(state, cx));
        let probe_state = state.clone();
        let is_active = self.is_active;
        content = content.child(gif_reach_end_probe(self.child_id("trending-end"), scroll.clone(), move |_, cx| {
            Self::load_more_trending(&probe_state, is_active, true, cx);
        }));

        scroll_view(self.child_id("scroll"))
            .shows_indicators(false)
            .scroll_handle(scroll)
            .flex_1()
            .min_h_0()
            .w_full()
            .child(content)
            .into_any_element()
    }

    fn trending_continuation(&self, state: &Entity<ChatInputGifTabState>, cx: &mut App) -> Option<AnyElement> {
        let services = StickersServicesStore::shared(cx);
        trending_next_cursor(services.read(cx))?;
        let is_loading = services.read(cx).is_loading(RiseStickerResourceID::GifSearch(String::new()));
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let row = div().w_full().flex().justify_center().py(px(12.));
        if is_loading {
            return Some(row.child(progress_view().tint(theme.text.secondary)).into_any_element());
        }
        let press_state = state.clone();
        let is_active = self.is_active;
        Some(
            row.child(
                ButtonUi::new(self.child_id("more"))
                    .disabled(!is_active)
                    .fit_content(true)
                    .padding_horizontal(20.)
                    .on_press(move |_, cx| Self::load_more_trending(&press_state, is_active, false, cx))
                    .child(MainText::new(tr("more")).color(theme.text.primary)),
            )
            .into_any_element(),
        )
    }

    fn excluding_saved(items: Vec<GifItem>, saved: &[GifItem]) -> Vec<GifItem> {
        if saved.is_empty() || items.is_empty() {
            return items;
        }
        let saved_ids: HashSet<&str> = saved.iter().map(|gif| gif.id.as_str()).collect();
        let saved_urls: HashSet<&str> =
            saved.iter().map(|gif| gif.file_url.as_str()).filter(|url| !url.is_empty()).collect();
        items
            .into_iter()
            .filter(|gif| !saved_ids.contains(gif.id.as_str()) && !saved_urls.contains(gif.file_url.as_str()))
            .collect()
    }

    fn empty_state(&self, cx: &mut App) -> AnyElement {
        let color = if self.uses_flat_chrome {
            let store = ThemeStore::shared(cx);
            store.read(cx).current_theme().text.secondary
        } else {
            white().opacity(0.4)
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(
                div()
                    .px(px(32.))
                    .text_center()
                    .swiftui_font(Font::system_size(15.))
                    .text_color(color)
                    .child(SharedString::from(tr("gif_favourites_empty"))),
            )
            .into_any_element()
    }

    fn load_more(cx: &mut App) {
        let user_id = user_id(cx);
        let services = StickersServicesStore::shared(cx);
        if !services.read(cx).gif_favourites_has_more(&user_id) {
            return;
        }
        let Some(next) = services.read(cx).gif_favourites_next_relative_id(&user_id).filter(|next| !next.is_empty()) else {
            return;
        };
        if services.read(cx).is_loading(RiseStickerResourceID::GifFavourites) {
            return;
        }
        StickersInteractionsStore::shared(cx).update(cx, |store, cx| store.load_gif_favourites(&user_id, Some(next), cx));
    }

    /// the repository skips a fresh or in flight feed so reopening the tab doesnt refetch
    fn load_trending(is_active: bool, cx: &mut App) {
        if !is_active {
            return;
        }
        StickersInteractionsStore::shared(cx).update(cx, |store, cx| store.search_gifs("", None, cx));
    }

    fn load_more_trending(state: &Entity<ChatInputGifTabState>, is_active: bool, automatically: bool, cx: &mut App) {
        let user_id = user_id(cx);
        let services = StickersServicesStore::shared(cx);
        let cursor = trending_next_cursor(services.read(cx));
        let is_loading = services.read(cx).is_loading(RiseStickerResourceID::GifSearch(String::new()));
        let next = state.update(cx, |state, _| {
            state.trending_pagination.take(&user_id, cursor.as_deref(), is_active, is_loading, automatically)
        });
        if let Some(next) = next {
            StickersInteractionsStore::shared(cx).update(cx, |store, cx| store.search_gifs("", Some(next), cx));
        }
    }
}

impl RenderOnce for ChatInputGifTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, cx| ChatInputGifTabState::new(cx));
        let is_active = self.is_active;
        let user_id = user_id(cx);

        let active_state = state.clone();
        on_change(self.child_id("is-active"), is_active, false, window, cx, move |_, active, _, cx| {
            if !*active {
                return;
            }
            Self::load_trending(true, cx);
            let near_end = {
                let scroll = &active_state.read(cx).scroll;
                scroll.max_offset().y + scroll.offset().y <= scroll.bounds().size.height
            };
            if near_end {
                Self::load_more_trending(&active_state, true, true, cx);
            }
        });
        let reset_state = state.clone();
        on_change(self.child_id("user"), user_id.clone(), false, window, cx, move |_, _, _, cx| {
            reset_state.update(cx, |state, cx| {
                state.trending_pagination = ChatInputGifPaginationGate::default();
                cx.notify();
            });
            Self::load_trending(is_active, cx);
        });

        let services = StickersServicesStore::shared(cx);
        let saved = services.read(cx).gif_favourites(&user_id);
        let popular = Self::excluding_saved(services.read(cx).gif_search_results(""), &saved);
        let has_next = trending_next_cursor(services.read(cx)).is_some();
        let content = if saved.is_empty() && popular.is_empty() && !has_next {
            self.empty_state(cx)
        } else {
            self.grid(&state, saved, popular, user_id, cx)
        };

        env_scope::<ChatInputPickerFlatChromeKey>(self.uses_flat_chrome).child(
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(self.search_bar())
                .child(content)
                .on_appear(self.child_id("appear"), move |_, cx| Self::load_trending(is_active, cx)),
        )
    }
}

/// automatic path asks for each cursor once per account
/// a failed page is retried only by the visible button
#[derive(Clone, Debug, Default)]
pub struct ChatInputGifPaginationGate {
    last_request: Option<ChatInputGifPaginationRequest>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ChatInputGifPaginationRequest {
    account_id: String,
    cursor: String,
}

impl ChatInputGifPaginationGate {
    pub fn next_cursor(has_more: bool, cursor: Option<String>) -> Option<String> {
        if !has_more {
            return None;
        }
        cursor.filter(|cursor| !cursor.is_empty())
    }

    pub fn take(
        &mut self,
        account_id: &str,
        cursor: Option<&str>,
        is_active: bool,
        is_loading: bool,
        automatically: bool,
    ) -> Option<String> {
        if !is_active || is_loading || account_id.is_empty() {
            return None;
        }
        let cursor = cursor.filter(|cursor| !cursor.is_empty())?;
        let request = ChatInputGifPaginationRequest { account_id: account_id.to_owned(), cursor: cursor.to_owned() };
        if automatically && self.last_request.as_ref() == Some(&request) {
            return None;
        }
        self.last_request = Some(request);
        Some(cursor.to_owned())
    }
}
