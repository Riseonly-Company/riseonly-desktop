use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Hsla, ListAlignment, SharedString, Subscription, Window,
};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_errors::ErrorCode;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::{Locale, tr};
use rise_theme::AppTheme;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::IconUi;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::official_badge::OfficialBadge;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::ui::bottom_sheet_ui::sheet_floating_header::{SheetFloatingHeader, SheetFloatingHeaderMetrics};
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::chat::components::chat::join_requests::chat_join_requests_banner::ChatJoinRequestsCountText;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::JoinRequest;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::{
    ChatInteractionsBoolResult, ChatInteractionsStore,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;

const ROW_SHIFT_TAP_GUARD: f64 = 0.45;
const PREFETCH_THRESHOLD: usize = 8;
const CARD_RADIUS: f32 = 22.0;
const CARD_OPACITY: f32 = 0.94;

/// what sits above the rows, a change of it changes the height of the first list item
#[derive(Clone, PartialEq, Eq)]
enum ChatJoinRequestsSheetPhase {
    RightsLost,
    Loading,
    Failed(String),
    Empty,
    Rows,
}

pub struct ChatJoinRequestsSheetContent {
    chat_id: String,
    is_channel: bool,
    on_open_profile: Rc<dyn Fn(String, String, &mut Window, &mut App)>,
    on_close: Rc<dyn Fn(&mut Window, &mut App)>,
    did_load: bool,
    processing_request_ids: HashSet<String>,
    row_shift_tap_guard_deadline: f64,
    list: ListUi,
    list_phase: ChatJoinRequestsSheetPhase,
    list_rows: Vec<JoinRequest>,
    row_shift: Animated<f32>,
    row_shift_start: usize,
    _subscriptions: Vec<Subscription>,
}

impl ChatJoinRequestsSheetContent {
    pub fn new(
        chat_id: impl Into<String>,
        is_channel: bool,
        on_open_profile: impl Fn(String, String, &mut Window, &mut App) + 'static,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let services = ChatServicesStore::shared(cx);
        let actions = ChatActionsStore::shared(cx);
        Self {
            chat_id: chat_id.into(),
            is_channel,
            on_open_profile: Rc::new(on_open_profile),
            on_close: Rc::new(on_close),
            did_load: false,
            processing_request_ids: HashSet::new(),
            row_shift_tap_guard_deadline: 0.0,
            list: ListUi::new(rise_ui::theme(cx), 2, ListAlignment::Top),
            list_phase: ChatJoinRequestsSheetPhase::Empty,
            list_rows: Vec::new(),
            row_shift: Animated::new(0.0),
            row_shift_start: 0,
            _subscriptions: vec![
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
            ],
        }
    }

    /// follows the chat row so buttons go when the owner demotes the viewer
    /// or switches off adding members while the sheet is open. unknown row keeps them
    fn can_review(chat_id: &str, cx: &mut App) -> bool {
        let services = ChatServicesStore::shared(cx);
        let services = services.read(cx);
        match services.chat_info(chat_id, cx) {
            Some(chat) => services.can_review_join_requests(&chat),
            None => true,
        }
    }

    fn requests_emptiness_changed(this: &Entity<Self>, is_empty: bool, window: &mut Window, cx: &mut App) {
        if !is_empty {
            this.update(cx, |this, _| this.did_load = true);
            return;
        }
        let (did_load, chat_id, on_close) = {
            let this = this.read(cx);
            (this.did_load, this.chat_id.clone(), this.on_close.clone())
        };
        if !did_load {
            return;
        }
        let has_more =
            ChatServicesStore::shared(cx).update(cx, |services, cx| services.join_requests_have_more(&chat_id, cx));
        if has_more {
            ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_join_requests(&chat_id, cx));
        } else {
            on_close(window, cx);
        }
    }

    fn has_more_changed(this: &Entity<Self>, has_more: bool, window: &mut Window, cx: &mut App) {
        let (did_load, chat_id, on_close) = {
            let this = this.read(cx);
            (this.did_load, this.chat_id.clone(), this.on_close.clone())
        };
        if has_more || !did_load {
            return;
        }
        let is_empty =
            ChatServicesStore::shared(cx).update(cx, |services, cx| services.join_requests(&chat_id, cx)).is_empty();
        if is_empty {
            on_close(window, cx);
        }
    }

    /// a resolved row leaves at once and the next one slides up
    /// so a quick second tap would resolve someone else, rows ignore taps until the shift settles
    fn process(this: &Entity<Self>, request_id: &str, approve: bool, cx: &mut App) {
        let now = RiseClock::monotonic().as_secs_f64();
        let chat_id = this.update(cx, |this, cx| {
            if now < this.row_shift_tap_guard_deadline {
                return None;
            }
            if !this.processing_request_ids.insert(request_id.to_owned()) {
                return None;
            }
            this.row_shift_tap_guard_deadline = now + ROW_SHIFT_TAP_GUARD;
            cx.notify();
            Some(this.chat_id.clone())
        });
        let Some(chat_id) = chat_id else {
            return;
        };
        let weak = this.downgrade();
        let finished_id = request_id.to_owned();
        let completion: ChatInteractionsBoolResult = Box::new(move |_: bool, cx: &mut App| {
            let _ = weak.update(cx, |this, cx| {
                this.processing_request_ids.remove(&finished_id);
                cx.notify();
            });
        });
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            if approve {
                store.approve_join_request(&chat_id, request_id, Some(completion), cx);
            } else {
                store.reject_join_request(&chat_id, request_id, Some(completion), cx);
            }
        });
    }

    fn sync_list(&mut self, phase: ChatJoinRequestsSheetPhase, rows: &[JoinRequest], cx: &mut Context<Self>) {
        if phase == self.list_phase && rows == self.list_rows.as_slice() {
            return;
        }
        let removed = single_removal(&self.list_rows, rows);
        if phase != self.list_phase {
            self.list.replaced(rows.len() + 2);
        } else if let Some(index) = removed {
            let height = self.list.state().bounds_for_item(index + 1).map(|bounds| f32::from(bounds.size.height));
            self.list.removed(index + 1..index + 2);
            if let Some(height) = height {
                self.row_shift_start = index;
                self.row_shift.jump(height);
                self.row_shift.set_in(0.0, spring(0.3, 0.86), RiseClock::monotonic(), cx);
            }
        } else {
            for (range, count) in row_splices(&self.list_rows, rows) {
                self.list.state().splice(range.start + 1..range.end + 1, count);
            }
        }
        self.list_phase = phase;
        self.list_rows = rows.to_vec();
    }

    fn head(
        phase: &ChatJoinRequestsSheetPhase,
        is_channel: bool,
        count_label: (i64, bool),
        chat_id: &str,
        theme: &AppTheme,
    ) -> AnyElement {
        let content = match phase {
            ChatJoinRequestsSheetPhase::RightsLost => Self::rights_lost_state(theme),
            ChatJoinRequestsSheetPhase::Loading => Self::loading_state(theme),
            ChatJoinRequestsSheetPhase::Failed(error) => Self::error_state(error, chat_id, theme),
            ChatJoinRequestsSheetPhase::Empty => Self::empty_state(theme),
            ChatJoinRequestsSheetPhase::Rows => Self::section_label(count_label, theme),
        };
        vstack(0.0)
            .w_full()
            .items_start()
            .child(div().h(px(SheetFloatingHeaderMetrics::HEIGHT)))
            .child(Self::hint(is_channel, theme))
            .child(content)
            .into_any_element()
    }

    fn hint(is_channel: bool, theme: &AppTheme) -> AnyElement {
        let key = if is_channel { "chat_join_requests_hint_channel" } else { "chat_join_requests_hint_group" };
        div()
            .px(px(18.0))
            .pt(px(2.0))
            .pb(px(14.0))
            .child(MainText::new(tr(key)).px(13.0).color(theme.text.secondary))
            .into_any_element()
    }

    fn section_label(count_label: (i64, bool), theme: &AppTheme) -> AnyElement {
        let (count, is_exact) = count_label;
        div()
            .px(px(18.0))
            .pb(px(6.0))
            .child(
                MainText::new(ChatJoinRequestsCountText::title(count, !is_exact).to_uppercase())
                    .px(12.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.secondary),
            )
            .into_any_element()
    }

    fn loading_state(theme: &AppTheme) -> AnyElement {
        div()
            .w_full()
            .px(px(14.0))
            .child(
                div()
                    .w_full()
                    .py(px(6.0))
                    .bg(theme.bg._200.opacity(CARD_OPACITY))
                    .rounded(px(CARD_RADIUS))
                    .child(UserListSkeleton::new().rows(6)),
            )
            .into_any_element()
    }

    fn error_state(error: &str, chat_id: &str, theme: &AppTheme) -> AnyElement {
        let chat_id = chat_id.to_owned();
        vstack(14.0)
            .w_full()
            .items_center()
            .py(px(34.0))
            .children(IconUi::sized(
                "exclamationmark.arrow.triangle.2.circlepath",
                px(34.0),
                theme.semantic.error_100,
                false,
            ))
            .child(
                div().px(px(28.0)).child(
                    MainText::new(error.to_owned()).px(14.0).tac(TextAlignT::Center).color(theme.text.secondary),
                ),
            )
            .child(
                LiquidGlassButton::new("chat-join-requests-retry", LiquidGlassButtonShape::Capsule)
                    .horizontal_padding(18.0)
                    .vertical_padding(9.0)
                    .tint(Some(theme.primary._100.opacity(0.82)))
                    .on_press(move |_, cx| {
                        ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_join_requests(&chat_id, cx));
                    })
                    .child(
                        MainText::new(tr("chat_join_requests_retry"))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.content_on(theme.primary._100)),
                    ),
            )
            .into_any_element()
    }

    fn rights_lost_state(theme: &AppTheme) -> AnyElement {
        let text = CanonErrors::text(Some(ErrorCode::ChatPermissionDenied.wire()), &HashMap::new())
            .unwrap_or_else(|| tr("rtc_error_missing_permission"));
        vstack(10.0)
            .w_full()
            .items_center()
            .py(px(40.0))
            .children(IconUi::sized("lock.fill", px(34.0), theme.text.secondary, true))
            .child(
                div()
                    .px(px(28.0))
                    .child(MainText::new(text).px(15.0).tac(TextAlignT::Center).color(theme.text.secondary)),
            )
            .into_any_element()
    }

    fn empty_state(theme: &AppTheme) -> AnyElement {
        vstack(10.0)
            .w_full()
            .items_center()
            .py(px(40.0))
            .children(IconUi::sized("person.crop.circle.badge.checkmark", px(38.0), theme.text.secondary, false))
            .child(MainText::new(tr("chat_join_requests_empty")).px(15.0).color(theme.text.secondary))
            .into_any_element()
    }
}

impl Render for ChatJoinRequestsSheetContent {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Rc::new(ThemeStore::shared(cx).read(cx).current_theme().clone());
        let chat_id = self.chat_id.clone();
        let is_channel = self.is_channel;
        let services = ChatServicesStore::shared(cx);
        let requests = services.update(cx, |services, cx| services.join_requests(&chat_id, cx));
        let has_more = services.update(cx, |services, cx| services.join_requests_have_more(&chat_id, cx));
        let relative_id = services.update(cx, |services, cx| services.join_requests_next_relative_id(&chat_id, cx));
        let count_label = services.update(cx, |services, cx| services.join_requests_count_label(&chat_id, cx));
        let slice = ChatActionsStore::shared(cx).read(cx).get_join_requests_sai.clone();
        let this = cx.entity();

        let observer = this.clone();
        on_change(
            "chat-join-requests-is-empty",
            requests.is_empty(),
            false,
            window,
            cx,
            move |_, is_empty, window, cx| Self::requests_emptiness_changed(&observer, *is_empty, window, cx),
        );
        let observer = this.clone();
        on_change("chat-join-requests-has-more", has_more, false, window, cx, move |_, has_more, window, cx| {
            Self::has_more_changed(&observer, *has_more, window, cx);
        });

        let phase = if !Self::can_review(&chat_id, cx) {
            ChatJoinRequestsSheetPhase::RightsLost
        } else if !requests.is_empty() {
            ChatJoinRequestsSheetPhase::Rows
        } else if slice.is_pending() {
            ChatJoinRequestsSheetPhase::Loading
        } else if let Some(error) = slice.error().filter(|error| !error.is_empty()) {
            ChatJoinRequestsSheetPhase::Failed(error)
        } else {
            ChatJoinRequestsSheetPhase::Empty
        };
        let had_rows = !requests.is_empty();
        let rows: Vec<JoinRequest> =
            if phase == ChatJoinRequestsSheetPhase::Rows { requests } else { Vec::new() };
        self.sync_list(phase.clone(), &rows, cx);

        let row_shift = self.row_shift.frame_value(RiseClock::monotonic(), window);
        let row_shift_start = self.row_shift_start;
        let processing = self.processing_request_ids.clone();
        let cache_key: SharedString = ChatServicesStore::join_requests_cache_key(&chat_id).into();
        let accept_label: SharedString = if is_channel {
            tr("chat_join_request_accept_channel").into()
        } else {
            tr("chat_join_request_accept").into()
        };
        let on_open_profile = self.on_open_profile.clone();
        let on_close = self.on_close.clone();
        let list_chat_id = chat_id.clone();
        let list_theme = theme.clone();
        let list_this = this.clone();
        let item_count = rows.len();

        let list = self.list.element(move |index, _, _| {
            if index == 0 {
                return Self::head(&phase, is_channel, count_label, &list_chat_id, &list_theme);
            }
            let Some(request) = rows.get(index - 1) else {
                return div().h(px(18.0)).into_any_element();
            };
            let row_index = index - 1;
            let is_last = row_index + 1 == item_count;
            let approve_this = list_this.clone();
            let approve_id = request.id.clone();
            let reject_this = list_this.clone();
            let reject_id = request.id.clone();
            let open_profile = on_open_profile.clone();
            let profile_user = request.user_id.clone();
            let profile_tag = request.user_tag.clone();
            let prefetch_chat_id = list_chat_id.clone();
            let row = ChatJoinRequestRow {
                request: request.clone(),
                theme: list_theme.clone(),
                accept_label: accept_label.clone(),
                is_processing: processing.contains(&request.id),
                on_approve: Rc::new(move |_: &mut Window, cx: &mut App| Self::process(&approve_this, &approve_id, true, cx)),
                on_reject: Rc::new(move |_: &mut Window, cx: &mut App| Self::process(&reject_this, &reject_id, false, cx)),
                on_tap: Rc::new(move |window: &mut Window, cx: &mut App| {
                    open_profile(profile_user.clone(), profile_tag.clone(), window, cx);
                }),
            };
            div()
                .w_full()
                .px(px(14.0))
                .when(row_index >= row_shift_start && row_shift != 0.0, |wrapper| wrapper.relative().top(px(row_shift)))
                .child(
                    div()
                        .w_full()
                        .bg(list_theme.bg._200.opacity(CARD_OPACITY))
                        .when(row_index == 0, |card| card.rounded_t(px(CARD_RADIUS)))
                        .when(is_last, |card| card.rounded_b(px(CARD_RADIUS)))
                        .child(row)
                        .when(!is_last, |card| {
                            card.child(
                                div()
                                    .w_full()
                                    .px(px(14.0))
                                    .child(div().w_full().h(px(0.5)).bg(list_theme.border._200.opacity(0.55))),
                            )
                        }),
                )
                .sai_pagination_prefetch(
                    SharedString::from(format!("chat-join-request-prefetch-{}", request.id)),
                    SaiPaginationPrefetchOptions::new(
                        row_index,
                        item_count,
                        cache_key.clone(),
                        relative_id.clone(),
                        has_more,
                    )
                    .threshold(PREFETCH_THRESHOLD),
                    move |_, cx| {
                        ChatInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.load_more_join_requests(&prefetch_chat_id, cx));
                    },
                )
                .into_any_element()
        });

        let appear_this = this;
        let appear_chat_id = chat_id;
        div()
            .relative()
            .size_full()
            .child(list)
            .child(
                div().absolute().top_0().left_0().right_0().child(SheetFloatingHeader::new(
                    "chat-join-requests-header",
                    tr("chat_join_requests_title"),
                    move |window, cx| on_close(window, cx),
                )),
            )
            .on_appear("chat-join-requests-sheet", move |_, cx| {
                if had_rows {
                    appear_this.update(cx, |this, _| this.did_load = true);
                }
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_join_requests(&appear_chat_id, cx));
            })
    }
}

fn single_removal(old: &[JoinRequest], new: &[JoinRequest]) -> Option<usize> {
    if old.len() != new.len() + 1 {
        return None;
    }
    let index = old.iter().zip(new).position(|(old, new)| old.id != new.id).unwrap_or(new.len());
    same_ids(&old[index + 1..], &new[index..]).then_some(index)
}

fn row_splices(old: &[JoinRequest], new: &[JoinRequest]) -> Vec<(Range<usize>, usize)> {
    let old_ids: HashSet<_> = old.iter().map(|row| &row.id).collect();
    let new_ids: HashSet<_> = new.iter().map(|row| &row.id).collect();
    let (mut i, mut j) = (old.len(), new.len());
    let mut splices = Vec::new();
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old[i - 1].id == new[j - 1].id {
            if old[i - 1] != new[j - 1] {
                splices.push((i - 1..i, 1));
            }
            i -= 1;
            j -= 1;
        } else if i > 0 && (j == 0 || !new_ids.contains(&old[i - 1].id)) {
            splices.push((i - 1..i, 0));
            i -= 1;
        } else if j > 0 && (i == 0 || !old_ids.contains(&new[j - 1].id)) {
            splices.push((i..i, 1));
            j -= 1;
        } else {
            splices.push((i - 1..i, 1));
            i -= 1;
            j -= 1;
        }
    }
    splices
}

fn same_ids(old: &[JoinRequest], new: &[JoinRequest]) -> bool {
    old.len() == new.len() && old.iter().zip(new).all(|(old, new)| old.id == new.id)
}

type ChatJoinRequestRowAction = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
struct ChatJoinRequestRow {
    request: JoinRequest,
    theme: Rc<AppTheme>,
    accept_label: SharedString,
    is_processing: bool,
    on_approve: ChatJoinRequestRowAction,
    on_reject: ChatJoinRequestRowAction,
    on_tap: ChatJoinRequestRowAction,
}

impl ChatJoinRequestRow {
    fn display_name(&self) -> String {
        if !self.request.user_name.is_empty() {
            return self.request.user_name.clone();
        }
        if !self.request.user_tag.is_empty() {
            return format!("@{}", self.request.user_tag);
        }
        tr("chat_deleted_account")
    }

    fn request_action_button(
        &self,
        name: &str,
        title: SharedString,
        tint: Hsla,
        foreground: Hsla,
        action: ChatJoinRequestRowAction,
    ) -> AnyElement {
        let label = if self.is_processing {
            progress_view().tint(foreground).into_any_element()
        } else {
            MainText::new(title)
                .px(14.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(foreground)
                .line_limit(1)
                .minimum_scale_factor(0.82)
                .into_any_element()
        };
        div()
            .flex_1()
            .min_w_0()
            .child(
                LiquidGlassButton::new(
                    SharedString::from(format!("chat-join-request-{name}-{}", self.request.id)),
                    LiquidGlassButtonShape::Capsule,
                )
                .horizontal_padding(12.0)
                .vertical_padding(9.0)
                .tint(Some(tint))
                .disabled(self.is_processing)
                .on_press(move |window, cx| action(window, cx))
                .child(div().w_full().flex().items_center().justify_center().child(label)),
            )
            .into_any_element()
    }
}

impl RenderOnce for ChatJoinRequestRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme.clone();
        let request = &self.request;
        let name_color = request
            .is_premium
            .then(|| PremiumNameColor::parse(request.name_color.as_deref()))
            .flatten()
            .unwrap_or(theme.text.primary);
        let on_tap = self.on_tap.clone();
        let decline = self.request_action_button(
            "decline",
            tr("chat_join_request_decline").into(),
            theme.button.bg_400,
            theme.text.primary,
            self.on_reject.clone(),
        );
        let accept = self.request_action_button(
            "accept",
            self.accept_label.clone(),
            theme.primary._100.opacity(0.86),
            theme.content_on(theme.primary._100),
            self.on_approve.clone(),
        );

        vstack(0.0)
            .w_full()
            .child(
                plain(SharedString::from(format!("chat-join-request-open-{}", request.id)))
                    .w_full()
                    .on_press(move |window, cx| on_tap(window, cx))
                    .child(
                        hstack(12.0)
                            .w_full()
                            .items_center()
                            .px(px(14.0))
                            .pt(px(12.0))
                            .pb(px(10.0))
                            .child(
                                UserLogoUi::new(SharedString::from(format!("chat-join-request-logo-{}", request.id)))
                                    .source(SharedString::from(request.user_logo.clone()))
                                    .name(SharedString::from(request.user_name.clone()))
                                    .user_id(SharedString::from(request.user_id.clone()))
                                    .size(46.0),
                            )
                            .child(
                                vstack(3.0)
                                    .min_w_0()
                                    .items_start()
                                    .child(
                                        hstack(6.0)
                                            .min_w_0()
                                            .items_center()
                                            .child(
                                                div().min_w_0().child(
                                                    MainText::new(self.display_name())
                                                        .px(16.0)
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .color(name_color)
                                                        .line_limit(1),
                                                ),
                                            )
                                            .when(request.is_official, |names| {
                                                names.child(OfficialBadge::new().size(px(14.0)))
                                            })
                                            .when(request.is_premium, |names| {
                                                names.child(
                                                    PremiumBadge::new()
                                                        .user_id(Some(SharedString::from(request.user_id.clone())))
                                                        .emoji(
                                                            request.premium_badge_emoji.clone().map(SharedString::from),
                                                        )
                                                        .size(14.0),
                                                )
                                            }),
                                    )
                                    .when(!request.user_tag.is_empty(), |column| {
                                        column.child(
                                            MainText::new(format!("@{}", request.user_tag))
                                                .px(13.0)
                                                .color(theme.text.secondary)
                                                .line_limit(1),
                                        )
                                    })
                                    .when_some(request.message.clone(), |column, message| {
                                        column.child(
                                            MainText::new(message).px(13.0).color(theme.text.secondary).line_limit(2),
                                        )
                                    }),
                            )
                            .child(spacer_min_w(8.0))
                            .child(
                                MainText::new(JoinRequestTime::short(request.requested_at))
                                    .px(12.0)
                                    .color(theme.text.secondary),
                            )
                            .children(IconUi::sized(
                                rise_ui::IconUi::trailing_chevron(),
                                px(11.0),
                                theme.text.secondary.opacity(0.72),
                                false,
                            )),
                    ),
            )
            .child(hstack(10.0).w_full().px(px(12.0)).py(px(10.0)).child(decline).child(accept))
    }
}

pub enum JoinRequestTime {}

impl JoinRequestTime {
    pub fn short(seconds: f64) -> String {
        if seconds <= 0.0 {
            return String::new();
        }
        let mut formatter = DateFormatter::new();
        formatter.set_locale(Locale::app_locale().identifier());
        formatter.set_date_format("HH:mm");
        formatter.string_from(&Date::from_time_interval_since_1970(seconds))
    }
}
