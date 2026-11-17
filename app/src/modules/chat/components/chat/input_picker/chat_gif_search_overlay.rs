use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyView, App, AppContext, Context, ElementId, Entity, FocusHandle, FontWeight, IntoElement, KeyDownEvent, Pixels, Render, RenderOnce,
    SharedString, Size, Subscription, Window, black, relative, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::IconUi;
use rise_ui::keyboard_dismiss::KeyboardDismissModifiers;
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue};
use rise_ui::swiftui::motion::{MotionSpec, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::input_picker::gif_grid::GifGrid;
use crate::modules::chat::components::chat::input_picker::sticker_hold_preview::StickerPreviewAction;
use crate::modules::stickers::engine::rise_sticker_models::RiseStickerResourceID;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::GifItem;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

pub enum GifSearchHero {}

impl GifSearchHero {
    pub const FIELD_ID: &'static str = "gifSearchField";

    pub fn animation() -> MotionSpec {
        spring(0.42, 0.86)
    }
}

struct ChatGifSearchOverlayState {
    query: String,
    submitted_query: String,
    drag_offset: Animated<f32>,
    focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    _subscriptions: [Subscription; 2],
}

impl ChatGifSearchOverlayState {
    fn new(previous_focus: Option<FocusHandle>, cx: &mut Context<Self>) -> Self {
        let services = StickersServicesStore::shared(cx);
        let socket = SaiWebSocketManager::shared(cx);
        Self {
            query: String::new(),
            submitted_query: String::new(),
            drag_offset: Animated::new(0.0),
            focus: cx.focus_handle(),
            previous_focus,
            _subscriptions: [cx.observe(&services, |_, _, cx| cx.notify()), cx.observe(&socket, |_, _, cx| cx.notify())],
        }
    }
}

#[derive(IntoElement, Clone)]
pub struct ChatGifSearchOverlay {
    id: ElementId,
    is_presented: Binding<bool>,
    on_send_gif: Rc<dyn Fn(GifItem, &mut Window, &mut App)>,
    uses_flat_chrome: bool,
}

impl ChatGifSearchOverlay {
    pub fn new(
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        on_send_gif: impl Fn(GifItem, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), is_presented, on_send_gif: Rc::new(on_send_gif), uses_flat_chrome: false }
    }

    pub fn uses_flat_chrome(mut self, uses_flat_chrome: bool) -> Self {
        self.uses_flat_chrome = uses_flat_chrome;
        self
    }

    fn child_id(&self, part: impl Into<SharedString>) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), part.into())
    }

    /// the host animates the removal when the binding flips
    fn close(state: &Entity<ChatGifSearchOverlayState>, is_presented: &Binding<bool>, window: &mut Window, cx: &mut App) {
        match state.read(cx).previous_focus.clone() {
            Some(previous_focus) => window.focus(&previous_focus, cx),
            None => ScreenFocus::release_text_field(window, cx),
        }
        state.update(cx, |state, cx| {
            state.drag_offset.jump(0.0);
            cx.notify();
        });
        is_presented.set(false, window, cx);
    }

    fn handle_query_change(state: &Entity<ChatGifSearchOverlayState>, raw: &str, cx: &mut App) {
        let weak = state.downgrade();
        StickersInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.search_gifs_debounced(
                raw,
                move |fired: String, cx: &mut App| {
                    let _ = weak.update(cx, |state, cx| {
                        state.submitted_query = fired;
                        cx.notify();
                    });
                },
                cx,
            );
        });
    }

    fn run_search(state: &Entity<ChatGifSearchOverlayState>, raw: &str, cx: &mut App) {
        let trimmed = trimming_whitespaces_and_newlines(raw).to_owned();
        state.update(cx, |state, cx| {
            state.submitted_query = trimmed.clone();
            cx.notify();
        });
        StickersInteractionsStore::shared(cx).update(cx, |store, cx| store.search_gifs(&trimmed, None, cx));
    }

    fn load_more(state: &Entity<ChatGifSearchOverlayState>, cx: &mut App) {
        let active_query = state.read(cx).submitted_query.clone();
        let services = StickersServicesStore::shared(cx);
        let Some(next) = services.read(cx).gif_search_next_relative_id(&active_query).filter(|next| !next.is_empty()) else {
            return;
        };
        if services.read(cx).is_loading(RiseStickerResourceID::GifSearch(active_query.clone())) {
            return;
        }
        StickersInteractionsStore::shared(cx).update(cx, |store, cx| store.search_gifs(&active_query, Some(next), cx));
    }

    fn drag_handle(&self, state: &Entity<ChatGifSearchOverlayState>) -> impl IntoElement {
        let changed_state = state.clone();
        let ended_state = state.clone();
        let is_presented = self.is_presented.clone();
        div()
            .w_full()
            .flex_none()
            .flex()
            .justify_center()
            .pt(px(8.))
            .pb(px(4.))
            .child(div().w(px(38.)).h(px(5.)).rounded_full().bg(white().opacity(0.22)))
            .gesture(
                self.child_id("drag"),
                DragGesture::new(8.)
                    .on_changed(move |value: &DragGestureValue, _: &mut Window, cx: &mut App| {
                        let height = f32::from(value.translation.height).max(0.0);
                        changed_state.update(cx, |state, cx| {
                            state.drag_offset.jump(height);
                            cx.notify();
                        });
                    })
                    .on_ended(move |value: &DragGestureValue, window: &mut Window, cx: &mut App| {
                        if f32::from(value.translation.height) > 80.0 {
                            Self::close(&ended_state, &is_presented, window, cx);
                        } else {
                            ended_state.update(cx, |state, cx| {
                                state.drag_offset.set_in(0.0, spring(0.3, 0.85), RiseClock::monotonic(), cx);
                                cx.notify();
                            });
                        }
                    }),
            )
    }

    fn header(&self, state: &Entity<ChatGifSearchOverlayState>, theme: &AppTheme) -> impl IntoElement {
        let read_state = state.clone();
        let write_state = state.clone();
        let change_state = state.clone();
        let submit_state = state.clone();
        let close_state = state.clone();
        let is_presented = self.is_presented.clone();
        let close_action = move |window: &mut Window, cx: &mut App| Self::close(&close_state, &is_presented, window, cx);
        let close_icon = IconUi::sized("xmark", px(15.), theme.text.secondary, false);

        let close = if self.uses_flat_chrome {
            plain(self.child_id("close"))
                .on_press(close_action)
                .child(
                    div()
                        .id(self.child_id("close-label"))
                        .tooltip_label(tr("cancel"))
                        .frame_size(42_f32, 42_f32)
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(theme.bg._100)
                        .children(close_icon),
                )
                .into_any_element()
        } else {
            LiquidGlassButton::new(self.child_id("close"), LiquidGlassButtonShape::Circle)
                .size(42.)
                .accessibility_label(Some(tr("cancel")))
                .on_press(close_action)
                .children(close_icon)
                .into_any_element()
        };

        hstack(10_f32)
            .w_full()
            .flex_none()
            .px(px(12.))
            .pt(px(4.))
            .pb(px(10.))
            .child(
                div().flex_1().min_w_0().child(
                    SearchFieldUi::new(
                        self.child_id("field"),
                        Binding::new(
                            move |cx: &App| read_state.read(cx).query.clone(),
                            move |value: String, _: &mut Window, cx: &mut App| {
                                write_state.update(cx, |state, cx| {
                                    state.query = value;
                                    cx.notify();
                                });
                            },
                        ),
                    )
                    .placeholder(SharedString::from(tr("gif_search_placeholder")))
                    .on_change(move |raw: &str, _: &mut Window, cx: &mut App| Self::handle_query_change(&change_state, raw, cx))
                    .on_submit(move |_: &mut Window, cx: &mut App| {
                        let query = submit_state.read(cx).query.clone();
                        Self::run_search(&submit_state, &query, cx);
                    })
                    .uses_inline_text_field(true)
                    .auto_focus(true)
                    .uses_flat_chrome(self.uses_flat_chrome),
                ),
            )
            .child(close)
    }

    fn attribution(theme: &AppTheme) -> impl IntoElement {
        div()
            .w_full()
            .flex_none()
            .flex()
            .justify_center()
            .pb(px(8.))
            .swiftui_font(Font::system_weight(11., FontWeight::SEMIBOLD))
            .text_color(theme.text.secondary)
            .child(SharedString::from(tr("gif_powered_by_giphy")))
    }

    fn centered_fill(content: impl IntoElement) -> AnyElement {
        div().flex_1().min_h_0().w_full().flex().items_center().justify_center().child(content).into_any_element()
    }

    fn results_grid(
        &self,
        state: &Entity<ChatGifSearchOverlayState>,
        services: &Entity<StickersServicesStore>,
        is_connected: bool,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let (query, active_query) = {
            let state = state.read(cx);
            (state.query.clone(), state.submitted_query.clone())
        };
        let results = services.read(cx).gif_search_results(&active_query);
        let typing_ahead = {
            let trimmed = trimming_whitespaces_and_newlines(&query).to_owned();
            !trimmed.is_empty() && trimmed != active_query
        };
        let is_loading_first_page = results.is_empty()
            && (services.read(cx).is_loading(RiseStickerResourceID::GifSearch(active_query.clone())) || typing_ahead);

        if is_loading_first_page {
            return Self::centered_fill(progress_view().tint(theme.text.secondary));
        }
        if results.is_empty() {
            return Self::centered_fill(
                div()
                    .swiftui_font(Font::system_size(15.))
                    .text_color(theme.text.secondary)
                    .child(SharedString::from(tr(if is_connected { "picker_no_results" } else { "chats_status_waiting_for_network" }))),
            );
        }

        let on_send = self.on_send_gif.clone();
        let tap_state = state.clone();
        let is_presented = self.is_presented.clone();
        let menu_services = services.clone();
        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        let probe_state = state.clone();
        let box_size = window.use_keyed_state(self.child_id("results-box"), cx, |_, _| Size::<Pixels>::default());
        let measured = *box_size.read(cx);
        let list = GifGrid::new(self.child_id("results"), results, move |gif: &GifItem, window: &mut Window, cx: &mut App| {
            on_send(gif.clone(), window, cx);
            Self::close(&tap_state, &is_presented, window, cx);
        })
        .uses_flat_chrome(self.uses_flat_chrome)
        .menu_for(move |gif: &GifItem, cx: &App| {
            let is_favourite = menu_services.read(cx).is_gif_favourite(&user_id, gif);
            let user_id = user_id.clone();
            let gif = gif.clone();
            vec![StickerPreviewAction::new(
                if is_favourite { "unsave" } else { "save" },
                tr(if is_favourite { "gif_unsave" } else { "gif_save" }),
                if is_favourite { "bookmark.slash" } else { "bookmark" },
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    if user_id.is_empty() {
                        return;
                    }
                    StickersInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.toggle_gif_favourite(&user_id, &gif, cx));
                }),
            )]
        })
        .on_reach_end(move |_, cx| Self::load_more(&probe_state, cx))
        .virtual_list(measured, 2.0, 24.0);
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(list)
            .child(measure(self.child_id("results-measure"), move |bounds, _, cx| {
                box_size.update(cx, |size, cx| {
                    if *size != bounds.size {
                        *size = bounds.size;
                        cx.notify();
                    }
                });
            }))
            .into_any_element()
    }
}

/// the sheet as its own view
/// wheel, playing tiles and field notify it alone
struct ChatGifSearchOverlayView {
    overlay: ChatGifSearchOverlay,
}

impl Render for ChatGifSearchOverlayView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.overlay.clone().content(window, cx)
    }
}

struct ChatGifSearchOverlayHolder {
    view: Entity<ChatGifSearchOverlayView>,
}

impl RenderOnce for ChatGifSearchOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.sheet_view(window, cx)
    }
}

impl ChatGifSearchOverlay {
    pub fn into_view(self, window: &mut Window, cx: &mut App) -> AnyView {
        self.sheet_view(window, cx).into()
    }

    fn sheet_view(self, window: &mut Window, cx: &mut App) -> Entity<ChatGifSearchOverlayView> {
        let initial = self.clone();
        let view = window
            .use_keyed_state(self.child_id("view"), cx, move |_, cx| ChatGifSearchOverlayHolder {
                view: cx.new(|_| ChatGifSearchOverlayView { overlay: initial }),
            })
            .read(cx)
            .view
            .clone();
        view.update(cx, |view, _| view.overlay = self);
        view
    }
}

impl ChatGifSearchOverlay {
    fn content(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.child_id("state"), cx, |window, cx| {
            let previous_focus = window.focused(cx);
            ChatGifSearchOverlayState::new(previous_focus, cx)
        });
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let services = StickersServicesStore::shared(cx);
        let is_connected = SaiWebSocketManager::shared(cx).read(cx).is_connected();

        let reconnect_state = state.clone();
        on_change(self.child_id("connected"), is_connected, false, window, cx, move |_, connected, _, cx| {
            if !*connected {
                return;
            }
            let active_query = reconnect_state.read(cx).submitted_query.clone();
            if !StickersServicesStore::shared(cx).read(cx).gif_search_results(&active_query).is_empty() {
                Self::load_more(&reconnect_state, cx);
                return;
            }
            StickersInteractionsStore::shared(cx).update(cx, |store, cx| store.search_gifs(&active_query, None, cx));
        });

        let offset = state.read(cx).drag_offset.frame_value(RiseClock::monotonic(), window).max(0.0);
        let focus = state.read(cx).focus.clone();
        let results = self.results_grid(&state, &services, is_connected, &theme, window, cx);
        let appear_state = state.clone();

        let scrim_state = state.clone();
        let scrim_presented = self.is_presented.clone();
        let scrim = div()
            .id(self.child_id("scrim"))
            .absolute()
            .inset_0()
            .bg(if self.uses_flat_chrome { theme.bg._100 } else { black().opacity(0.32) })
            .on_click(move |_, window, cx| Self::close(&scrim_state, &scrim_presented, window, cx))
            .keyboard_dismiss_excluded();

        let sheet = div()
            .relative()
            .w_full()
            .h(relative(0.94))
            .top(px(offset))
            .flex()
            .flex_col()
            .bg(theme.bg._100)
            .rounded_t(px(20.))
            .occlude()
            .child(self.drag_handle(&state))
            .child(self.header(&state, &theme))
            .child(Self::attribution(&theme))
            .child(results);

        let key_state = state.clone();
        let key_presented = self.is_presented.clone();
        div()
            .absolute()
            .inset_0()
            .size_full()
            .flex()
            .flex_col()
            .justify_end()
            .occlude()
            .track_focus(&focus)
            .layer_keys(LayerCancel::dismiss(move |window, cx| Self::close(&key_state, &key_presented, window, cx)))
            .child(scrim)
            .child(sheet)
            .on_appear(self.child_id("appear"), move |_, cx| Self::run_search(&appear_state, "", cx))
    }
}
