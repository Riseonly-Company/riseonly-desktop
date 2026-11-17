use std::rc::Rc;

use gpui::{App, Entity, FontWeight, Hsla, SharedString, Subscription, Window, rgb};
use rise_canon::rise_service_text::music_service;
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::NavigationStore;
use crate::core::ui::input_ui::InputOutlinedRowUi;
use crate::core::widgets::wrappers::list_shell_ui::list_shell_floating_top_inset;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::bottomsheets::track_picker::track_picker_shared::TrackPickerListModel;

pub(crate) const MISSING_VALUE: &str = "\u{2014}";
pub(crate) const MUSIC_PREFETCH_THRESHOLD: usize = 8;
const CARD_RADIUS: f32 = 12.0;
const PUBLISHED_COLOR: u32 = 0x00B894;
const SUBMITTED_COLOR: u32 = 0xFDCB6E;
const REJECT_SHEET_INITIAL_HEIGHT: f32 = 200.0;
const REJECT_SHEET_HEIGHT_TOLERANCE: f32 = 0.5;

pub(crate) fn hex_color(value: u32) -> Hsla {
    Hsla::from(rgb(value))
}

pub(crate) fn music_status_title(status: &str) -> String {
    tr(&format!("music_status_{status}"))
}

pub(crate) fn pop_screen(nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
    }
}

pub(crate) fn labeled_card(label: impl Into<SharedString>, value: impl Into<SharedString>, theme: &AppTheme) -> Div {
    vstack(6.0)
        .align(HorizontalAlignment::Leading)
        .w_full()
        .p(px(16.0))
        .bg(theme.bg._200)
        .rounded(px(CARD_RADIUS))
        .overflow_hidden()
        .child(MainText::new(label).px(12.0).color(theme.text.secondary))
        .child(MainText::new(value).px(14.0))
}

pub(crate) fn header_card(theme: &AppTheme) -> Div {
    vstack(8.0).w_full().p(px(16.0)).bg(theme.bg._200).rounded(px(CARD_RADIUS)).overflow_hidden()
}

fn album_status_color(status: &str, theme: &AppTheme) -> Hsla {
    if status == RiseMusicTrackStatus::Published.wire() {
        hex_color(PUBLISHED_COLOR)
    } else if status == music_service::ALBUM_REQUEST_STATUS_APPROVED {
        theme.primary._100
    } else if status == music_service::ALBUM_REQUEST_STATUS_REJECTED {
        theme.semantic.error_100
    } else {
        hex_color(SUBMITTED_COLOR)
    }
}

/// status badge for both album request lists
pub(crate) fn album_status_badge(status: &str, theme: &AppTheme) -> Option<Div> {
    if status.is_empty() {
        return None;
    }
    let color = album_status_color(status, theme);
    Some(
        div()
            .flex_none()
            .px(px(8.0))
            .py(px(3.0))
            .bg(color.opacity(0.12))
            .capsule()
            .swiftui_font(Font::system_weight(11.0, FontWeight::SEMIBOLD))
            .text_color(color)
            .child(music_status_title(status)),
    )
}

pub(crate) struct RequestDetailState {
    pub(crate) show_reject_sheet: bool,
    pub(crate) reject_reason: String,
    pub(crate) reject_sheet_height: f32,
    _subscriptions: [Subscription; 2],
}

impl RequestDetailState {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            show_reject_sheet: false,
            reject_reason: String::new(),
            reject_sheet_height: REJECT_SHEET_INITIAL_HEIGHT,
            _subscriptions: [
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn set_show_reject_sheet(&mut self, show_reject_sheet: bool, cx: &mut Context<Self>) {
        self.show_reject_sheet = show_reject_sheet;
        cx.notify();
    }
}

/// moderation actions
/// approving leaves the screen at once, the review runs on its own
pub(crate) fn moderation_actions(
    id: &str,
    is_review_pending: bool,
    state: &Entity<RequestDetailState>,
    nav_store: Option<Entity<NavigationStore>>,
    approve: impl Fn(&mut App) + 'static,
    theme: &AppTheme,
) -> Div {
    let state = state.clone();
    vstack(10.0)
        .w_full()
        .child(
            ButtonUi::new(SharedString::from(format!("{id}-approve")))
                .disabled(is_review_pending)
                .on_press(move |_, cx| {
                    approve(cx);
                    pop_screen(nav_store.as_ref(), cx);
                })
                .child(MainText::new(tr("music_approve")).px(15.0).font_weight(FontWeight::SEMIBOLD)),
        )
        .child(
            ButtonUi::new(SharedString::from(format!("{id}-reject")))
                .disabled(is_review_pending)
                .background_color(theme.bg._200)
                .on_press(move |_, cx| state.update(cx, |state, cx| state.set_show_reject_sheet(true, cx)))
                .child(
                    MainText::new(tr("music_reject"))
                        .px(15.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.semantic.error_100),
                ),
        )
}

/// reject sheet
/// hugs its measured content
pub(crate) fn reject_sheet(
    id: &str,
    state: &Entity<RequestDetailState>,
    nav_store: Option<Entity<NavigationStore>>,
    is_review_pending: impl Fn() -> bool + 'static,
    reject: impl Fn(&str, &mut App) + 'static,
    theme: &AppTheme,
    cx: &App,
) -> NativeBottomSheetPresentationHost {
    let id = id.to_owned();
    let reader = state.clone();
    let writer = state.clone();
    let content_state = state.clone();
    let background = theme.bg._100;
    let height = state.read(cx).reject_sheet_height;
    let is_review_pending = Rc::new(is_review_pending);
    let reject = Rc::new(reject);

    NativeBottomSheetPresentationHost::new(
        SharedString::from(format!("{id}-sheet")),
        Binding::new(
            move |cx: &App| reader.read(cx).show_reject_sheet,
            move |presented, _window, cx| {
                writer.update(cx, |state, cx| state.set_show_reject_sheet(presented, cx));
            },
        ),
        move |_: &mut Window, cx: &mut App| {
            let theme = rise_ui::theme(cx).clone();
            let state = content_state.clone();
            let can_confirm = {
                let reason = state.read(cx).reject_reason.clone();
                !trimming_whitespaces_and_newlines(&reason).is_empty() && !is_review_pending()
            };
            let confirm: Rc<dyn Fn(&mut Window, &mut App)> = {
                let state = state.clone();
                let nav_store = nav_store.clone();
                let is_review_pending = is_review_pending.clone();
                let reject = reject.clone();
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    let reason = state.read(cx).reject_reason.clone();
                    if trimming_whitespaces_and_newlines(&reason).is_empty() || is_review_pending() {
                        return;
                    }
                    reject(&reason, cx);
                    state.update(cx, |state, cx| state.set_show_reject_sheet(false, cx));
                    pop_screen(nav_store.as_ref(), cx);
                })
            };
            let on_submit = confirm.clone();
            let on_press = confirm;
            let cancel_state = state.clone();
            let measured_state = state.clone();

            div().size_full().bg(theme.bg._100).child(
                div()
                    .w_full()
                    .relative()
                    .child(
                        vstack(16.0)
                            .w_full()
                            .p(px(20.0))
                            .child(MainText::new(tr("music_reject_reason_title")).px(17.0).font_weight(FontWeight::SEMIBOLD))
                            .child(div().w_full().child(
                                InputOutlinedRowUi::new(
                                    SharedString::from(format!("{id}-reason")),
                                    Binding::entity(
                                        state,
                                        |state: &RequestDetailState| state.reject_reason.clone(),
                                        |state, reason, cx| {
                                            state.reject_reason = reason;
                                            cx.notify();
                                        },
                                    ),
                                    tr("music_reject_reason_placeholder"),
                                    div(),
                                )
                                .on_submit(move |window, cx| on_submit(window, cx))
                                .on_cancel(move |_, cx| {
                                    cancel_state.update(cx, |state, cx| state.set_show_reject_sheet(false, cx));
                                }),
                            ))
                            .child(
                                ButtonUi::new(SharedString::from(format!("{id}-confirm")))
                                    .disabled(!can_confirm)
                                    .on_press(move |window, cx| on_press(window, cx))
                                    .child(MainText::new(tr("music_reject_confirm")).px(15.0).font_weight(FontWeight::SEMIBOLD)),
                            ),
                    )
                    .child(measure(SharedString::from(format!("{id}-height")), move |bounds, _, cx| {
                        let height = f32::from(bounds.size.height);
                        measured_state.update(cx, |state, cx| {
                            if (height - state.reject_sheet_height).abs() > REJECT_SHEET_HEIGHT_TOLERANCE {
                                state.reject_sheet_height = height;
                                cx.notify();
                            }
                        });
                    })),
            )
        },
    )
    .presentation_background(background)
    .presentation_detents([PresentationDetent::Height(height)])
}

/// scroll body for a request
/// the inset is read here because the shell provides it only to what renders inside it
#[derive(IntoElement)]
pub(crate) struct RequestScrollBody {
    id: SharedString,
    content: AnyElement,
}

impl RequestScrollBody {
    pub(crate) fn new(id: impl Into<SharedString>, content: impl IntoElement) -> Self {
        Self { id: id.into(), content: content.into_any_element() }
    }
}

impl RenderOnce for RequestScrollBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let top_inset = list_shell_floating_top_inset(cx);
        scroll_view(self.id).size_full().child(div().w_full().pt(px(top_inset)).child(self.content))
    }
}

/// disabled scroll around a loading skeleton
#[derive(IntoElement)]
pub(crate) struct RequestSkeletonBody {
    top_padding: f32,
    content: AnyElement,
}

impl RequestSkeletonBody {
    pub(crate) fn new(top_padding: f32, content: impl IntoElement) -> Self {
        Self { top_padding, content: content.into_any_element() }
    }
}

impl RenderOnce for RequestSkeletonBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let top_inset = list_shell_floating_top_inset(cx);
        div().size_full().overflow_hidden().pt(px(top_inset + self.top_padding)).child(self.content)
    }
}

/// state of a request list, gpui lays out a lazy column only through list
pub(crate) struct RequestListState {
    pub(crate) status_filter: String,
    pub(crate) list_model: TrackPickerListModel,
    _subscriptions: [Subscription; 3],
}

impl RequestListState {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let services = MusicServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            status_filter: music_service::ALBUM_REQUEST_STATUS_PENDING.to_owned(),
            list_model: TrackPickerListModel::new(cx),
            _subscriptions: [
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }

    pub(crate) fn status_filter_binding(state: &Entity<Self>) -> Binding<String> {
        Binding::entity(
            state.clone(),
            |state: &Self| state.status_filter.clone(),
            |state, status_filter, cx| {
                state.status_filter = status_filter;
                cx.notify();
            },
        )
    }
}

/// one entry of the list column
/// the spacing before it plus the column top and bottom padding
pub(crate) fn request_list_row(index: usize, item_count: usize, top: f32, spacing: f32, bottom: f32) -> Div {
    div()
        .w_full()
        .pt(px(if index == 0 { top } else { spacing }))
        .when(index + 1 == item_count, |row| row.pb(px(bottom)))
}
