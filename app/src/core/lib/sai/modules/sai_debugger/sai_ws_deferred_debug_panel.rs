use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, BoxShadow, Context, ElementId, Entity, FontWeight, Hsla, ListAlignment, PromptButton, PromptLevel,
    SharedString, Task, WeakEntity, Window, black, point, rgb, white,
};
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::{character_count, character_prefix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::minimum_scale_factor::MinimumScaleText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::rounded_gradient::{rounded_gradient_fill, rounded_gradient_stroke_border};
use rise_ui::swiftui::transition::UnitPoint;

use super::sai_debug_storage_panel_views::{SaiDebugBodyFormat, SaiDebugExpandableMonospaceBlock};
use super::sai_ws_deferred_debug_store::{
    SaiWsDeferredDebugDiskCard, SaiWsDeferredDebugFifoCard, SaiWsDeferredDebugHistoryRow, SaiWsDeferredDebugOutcome,
    SaiWsDeferredDebugStore,
};
use crate::core::lib::sai::core::websocket::sai_ws_deferred_outbound_coordinator::SaiWsDeferredOutboundCoordinator;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsSendError;

const SHORT_TIME_FORMAT: &str = "HH:mm:ss";
const EM_DASH: &str = "\u{2014}";
const ELLIPSIS: &str = "\u{2026}";

const LINK_BLUE: u32 = 0x29b6f6;
const DESTRUCTIVE_RED: u32 = 0xe11d48;
const FIFO_STRIP_TINT: u32 = 0x1a237e;
const ACK_STRIP_TINT: u32 = 0x33691e;
const WAIT_AMBER: u32 = 0xffb020;
const DATA_BLOCK: u32 = 0x1a3a52;
const OUTCOME_OK: u32 = 0x00db0f;
const OUTCOME_FAILED: u32 = 0xff4444;
const OUTCOME_NEUTRAL: u32 = 0x888888;

const FIFO_ACCENT: (u32, u32) = (0x7c4dff, 0x00bcd4);
const DISK_ACCENT: (u32, u32) = (0xff9800, 0xf4511e);
const ACK_ACCENT: (u32, u32) = (0x8bc34a, 0x00796b);

const FIFO_CARD_WIDTH: f32 = 168.;
const DISK_CARD_WIDTH: f32 = 158.;
const ACK_CARD_WIDTH: f32 = 168.;
const MINI_CARD_RADIUS: f32 = 14.;
const MINI_CARD_STROKE: f32 = 1.5;
const BODY_PREVIEW_MAX_CHARS: usize = 220;
const BODY_PREVIEW_KEPT_CHARS: usize = 217;
const HISTORY_MAX_HEIGHT: f32 = 420.;
const HISTORY_PADDING: f32 = 14.;
const HISTORY_ROW_SPACING: f32 = 12.;
const WAIT_TICK: Duration = Duration::from_millis(250);

thread_local! {
    static SHORT_TIME: DateFormatter = {
        let mut formatter = DateFormatter::new();
        formatter.set_date_format(SHORT_TIME_FORMAT);
        formatter
    };
}

fn short_time(date: &Date) -> String {
    SHORT_TIME.with(|formatter| formatter.string_from(date))
}

fn color(hex: u32) -> Hsla {
    Hsla::from(rgb(hex))
}

fn accent_gradient(accent: (u32, u32)) -> LinearGradient {
    LinearGradient::colors([color(accent.0), color(accent.1)], UnitPoint::TOP_LEADING, UnitPoint::BOTTOM_TRAILING)
}

fn body_preview(text: &str) -> String {
    let trimmed = trimming_whitespaces_and_newlines(text);
    if character_count(trimmed) <= BODY_PREVIEW_MAX_CHARS {
        return trimmed.to_owned();
    }
    format!("{}{ELLIPSIS}", character_prefix(trimmed, BODY_PREVIEW_KEPT_CHARS))
}

fn format_wait(wait: TimeInterval) -> String {
    if wait < 1. {
        return format!("{:.0} ms", wait * 1000.);
    }
    if wait < 60. {
        return format!("{wait:.1} s");
    }
    let whole = wait as i64;
    format!("{}m {}s", whole / 60, whole % 60)
}

fn history_accent(outcome: &SaiWsDeferredDebugOutcome) -> Hsla {
    match outcome {
        SaiWsDeferredDebugOutcome::Sent
        | SaiWsDeferredDebugOutcome::ColdReplaySent
        | SaiWsDeferredDebugOutcome::AckDebtAcknowledged
        | SaiWsDeferredDebugOutcome::AckDebtColdReplaySent => color(OUTCOME_OK),
        SaiWsDeferredDebugOutcome::SendFailed(_)
        | SaiWsDeferredDebugOutcome::ColdReplayFailed(_)
        | SaiWsDeferredDebugOutcome::AckDebtColdReplayFailed(_) => color(OUTCOME_FAILED),
        SaiWsDeferredDebugOutcome::Cancelled(_) | SaiWsDeferredDebugOutcome::AckDebtTimedOut => color(OUTCOME_NEUTRAL),
    }
}

fn caption_title(text: String, theme: &AppTheme) -> Div {
    div()
        .child(text)
        .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
        .text_color(theme.text.secondary)
}

fn count_label(count: usize, theme: &AppTheme) -> Div {
    div()
        .child(format!("({count})"))
        .swiftui_font(Font::caption2().monospaced_digit())
        .text_color(theme.text.secondary.opacity(0.75))
}

fn text_button(id: &'static str, title: String, tint: u32, on_press: impl Fn(&mut Window, &mut App) + 'static) -> impl IntoElement {
    plain(id).on_press(on_press).child(
        div()
            .child(title)
            .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
            .text_color(rgb(tint)),
    )
}

fn horizontal_strip(id: &'static str, tint: Hsla, theme: &AppTheme, cards: Vec<Div>) -> Div {
    let backdrop = LinearGradient::colors([theme.bg._100.opacity(0.35), tint], UnitPoint::LEADING, UnitPoint::TRAILING);
    div()
        .relative()
        .w_full()
        .child(div().absolute().inset_0().child(backdrop.view()))
        .child(
            div().id(id).relative().w_full().flex().overflow_x_scroll().child(
                hstack(10.)
                    .align(VerticalAlignment::Top)
                    .flex_none()
                    .children(cards)
                    .px(px(14.))
                    .py(px(10.)),
            ),
        )
}

/// inputs of the mini card chrome
struct SaiWsDeferredMiniCard {
    title: String,
    subtitle: String,
    wait: Option<TimeInterval>,
    accent: (u32, u32),
    badge: &'static str,
    body: String,
    cache_key: Option<String>,
    enqueued_label: String,
    width: f32,
}

fn mini_card_chrome(card: SaiWsDeferredMiniCard, theme: &AppTheme) -> Div {
    let half_stroke = MINI_CARD_STROKE / 2.;
    let cache_key = card.cache_key.filter(|cache_key| !cache_key.is_empty()).map(|cache_key| {
        div()
            .child(cache_key)
            .swiftui_font(Font::system(8., FontWeight::MEDIUM, FontDesign::Monospaced))
            .text_color(theme.text.secondary.opacity(0.65))
            .line_clamp(1)
    });
    let wait = card.wait.map(|wait| {
        div()
            .child(format_wait(wait))
            .swiftui_font(Font::system(13., FontWeight::EXTRA_BOLD, FontDesign::Rounded).monospaced_digit())
            .text_color(rgb(WAIT_AMBER))
    });

    div()
        .relative()
        .flex_none()
        .w(px(card.width))
        .child(
            div()
                .absolute()
                .inset_0()
                .bg(theme.bg._200.opacity(0.92))
                .clip_shape(Shape::continuous(MINI_CARD_RADIUS))
                .shadow(vec![BoxShadow {
                    color: black().opacity(0.12),
                    offset: point(px(0.), px(2.)),
                    blur_radius: px(6.),
                    spread_radius: px(0.),
                    inset: false,
                }]),
        )
        .child(
            vstack(6.)
                .relative()
                .align(HorizontalAlignment::Leading)
                .w_full()
                .child(
                    hstack(6.)
                        .w_full()
                        .child(
                            div()
                                .relative()
                                .child(div().absolute().inset_0().child(rounded_gradient_fill(
                                    accent_gradient(card.accent),
                                    f32::MAX,
                                )))
                                .child(
                                    div()
                                        .relative()
                                        .px(px(6.))
                                        .py(px(3.))
                                        .child(card.badge)
                                        .swiftui_font(Font::system(9., FontWeight::EXTRA_BOLD, FontDesign::Default))
                                        .text_color(white().opacity(0.95)),
                                ),
                        )
                        .child(spacer_min_w(0.)),
                )
                .child(
                    div()
                        .w_full()
                        .swiftui_font(Font::system(11., FontWeight::BOLD, FontDesign::Monospaced))
                        .text_color(theme.text.primary)
                        .child(MinimumScaleText::new(card.title, 0.85).line_limit(2usize)),
                )
                .child(
                    div()
                        .child(card.subtitle)
                        .swiftui_font(Font::system(9., FontWeight::MEDIUM, FontDesign::Default))
                        .text_color(theme.text.secondary)
                        .line_clamp(2),
                )
                .children(wait)
                .child(
                    div()
                        .child(tr_args("sai_ws_deferred_debug_in_queue", &[FormatArg::from(card.enqueued_label)]))
                        .swiftui_font(Font::caption2())
                        .text_color(theme.text.secondary.opacity(0.8)),
                )
                .children(cache_key)
                .child(
                    div()
                        .w_full()
                        .child(body_preview(&card.body))
                        .swiftui_font(Font::system(9., FontWeight::NORMAL, FontDesign::Monospaced))
                        .text_color(theme.text.secondary)
                        .line_clamp(5),
                )
                .p(px(10.)),
        )
        .child(
            div()
                .absolute()
                .inset(px(-half_stroke))
                .child(rounded_gradient_stroke_border(
                    accent_gradient(card.accent),
                    MINI_CARD_RADIUS + half_stroke,
                    MINI_CARD_STROKE,
                )),
        )
}

fn fifo_mini_card(card: &SaiWsDeferredDebugFifoCard, now: Date, theme: &AppTheme) -> Div {
    mini_card_chrome(
        SaiWsDeferredMiniCard {
            title: format!("{}.{}", card.service, card.method),
            subtitle: tr_args(
                "sai_ws_deferred_debug_position_to_wire",
                &[FormatArg::from(card.position_from_wire.to_string()), FormatArg::from(card.total_in_fifo.to_string())],
            ),
            wait: Some(now.time_interval_since(&card.enqueued_at)),
            accent: FIFO_ACCENT,
            badge: "RAM",
            body: card.body_json.clone(),
            cache_key: card.cache_key.clone(),
            enqueued_label: short_time(&card.enqueued_at),
            width: FIFO_CARD_WIDTH,
        },
        theme,
    )
}

fn disk_mini_card(card: &SaiWsDeferredDebugDiskCard, theme: &AppTheme) -> Div {
    mini_card_chrome(
        SaiWsDeferredMiniCard {
            title: format!("{}.{}", card.service, card.method),
            subtitle: tr("sai_ws_deferred_debug_disk_only"),
            wait: None,
            accent: DISK_ACCENT,
            badge: "DISK",
            body: card.body_json.clone(),
            cache_key: card.cache_key.clone(),
            enqueued_label: EM_DASH.to_owned(),
            width: DISK_CARD_WIDTH,
        },
        theme,
    )
}

fn ack_debt_mini_card(card: &SaiWsDeferredDebugDiskCard, theme: &AppTheme) -> Div {
    mini_card_chrome(
        SaiWsDeferredMiniCard {
            title: format!("{}.{}", card.service, card.method),
            subtitle: card.id.clone(),
            wait: None,
            accent: ACK_ACCENT,
            badge: "ACK",
            body: card.body_json.clone(),
            cache_key: card.cache_key.clone(),
            enqueued_label: EM_DASH.to_owned(),
            width: ACK_CARD_WIDTH,
        },
        theme,
    )
}

fn history_id(kind: &str, id: Uuid) -> ElementId {
    ElementId::Name(SharedString::from(format!("sai-ws-deferred-{kind}-{id}")))
}

fn history_icon_label(symbol: &'static str, tint: Hsla, theme: &AppTheme) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(34.))
        .h(px(30.))
        .bg(theme.bg._300.opacity(0.9))
        .clip_shape(Shape::continuous(8.))
        .children(IconUi::sized(symbol, px(14.), tint, false))
}

fn history_row(row: SaiWsDeferredDebugHistoryRow, theme: &AppTheme, panel: WeakEntity<SaiWsDeferredDebugPanel>) -> AnyElement {
    let accent = history_accent(&row.outcome);
    let wait_label = match row.wait_seconds {
        Some(wait) => hstack(4.)
            .children(IconUi::sized("clock", px(11.), color(WAIT_AMBER), false))
            .child(
                div()
                    .child(tr_args("sai_ws_deferred_debug_waited", &[FormatArg::from(format!("{wait:.3}"))]))
                    .swiftui_font(Font::caption2().weight(FontWeight::MEDIUM))
                    .text_color(rgb(WAIT_AMBER)),
            ),
        None => hstack(4.)
            .children(IconUi::sized("clock", px(11.), theme.text.secondary.opacity(0.7), false))
            .child(
                div()
                    .child(tr("sai_ws_deferred_debug_wait_missing"))
                    .swiftui_font(Font::caption2())
                    .text_color(theme.text.secondary.opacity(0.7)),
            ),
    };
    let enqueued_label = row.enqueued_at.as_ref().map(|enqueued_at| {
        div()
            .child(format!("enqueue {}", short_time(enqueued_at)))
            .swiftui_font(Font::caption2())
            .text_color(theme.text.secondary.opacity(0.75))
    });
    let cache_key = row.cache_key.as_deref().filter(|cache_key| !cache_key.is_empty()).map(|cache_key| {
        div()
            .child(format!("cache: {cache_key}"))
            .swiftui_font(Font::caption2())
            .text_color(theme.text.secondary.opacity(0.6))
            .line_clamp(2)
    });
    let id = row.id;
    let block_body = row.body_json.clone();
    let summary = SaiDebugBodyFormat::size_label_char_count(character_count(&row.body_json) as i64);
    let copy_row = row.clone();

    hstack(10.)
        .align(VerticalAlignment::Top)
        .w_full()
        .child(
            div()
                .self_stretch()
                .flex()
                .flex_col()
                .py(px(2.))
                .child(div().w(px(4.)).flex_1().bg(accent).clip_shape(Shape::continuous(2.))),
        )
        .child(
            vstack(6.)
                .align(HorizontalAlignment::Leading)
                .flex_1()
                .min_w_0()
                .child(
                    hstack(8.)
                        .w_full()
                        .child(
                            div()
                                .child(short_time(&row.processed_at))
                                .swiftui_font(Font::caption2().weight(FontWeight::MEDIUM).monospaced_digit())
                                .text_color(theme.text.secondary.opacity(0.85)),
                        )
                        .child(
                            div()
                                .child(format!("{}.{}", row.service, row.method))
                                .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                                .text_color(theme.text.primary),
                        )
                        .child(spacer_min_w(0.)),
                )
                .child(
                    div()
                        .child(row.outcome_label())
                        .swiftui_font(Font::caption2().weight(FontWeight::SEMIBOLD))
                        .text_color(white().opacity(0.95))
                        .px(px(8.))
                        .py(px(3.))
                        .bg(accent.opacity(0.55))
                        .capsule(),
                )
                .child(hstack(12.).child(wait_label).children(enqueued_label))
                .children(cache_key)
                .child(
                    SaiDebugExpandableMonospaceBlock::new(
                        history_id("data", id),
                        "DATA",
                        summary,
                        theme.clone(),
                        Rc::new(move |_cx: &mut App| block_body.clone()),
                    )
                    .background(color(DATA_BLOCK).opacity(0.18)),
                ),
        )
        .child(
            vstack(6.)
                .child(
                    plain(history_id("copy", id))
                        .on_press(move |_window, cx| {
                            UIPasteboard::general().set_string(&copy_row.copy_block_text(), cx);
                        })
                        .child(history_icon_label("doc.on.doc", theme.text.primary, theme)),
                )
                .child(
                    plain(history_id("delete", id))
                        .on_press(move |window, cx| {
                            let _ = panel.update(cx, |panel, cx| panel.confirm_history_delete(id, window, cx));
                        })
                        .child(history_icon_label("trash", color(DESTRUCTIVE_RED), theme)),
                ),
        )
        .p(px(10.))
        .bg(theme.bg._200.opacity(0.55))
        .clip_shape(Shape::continuous(10.))
        .into_any_element()
}

/// deferred debug panel
/// the confirm dialogs are the system prompt of the window so the flags presenting them arent kept
pub struct SaiWsDeferredDebugPanel {
    store: Entity<SaiWsDeferredDebugStore>,
    theme: AppTheme,
    history_list: ListUi,
    history_ids: Vec<Uuid>,
    _wait_ticker: Option<Task<()>>,
}

impl SaiWsDeferredDebugPanel {
    pub fn new(theme: AppTheme, cx: &mut Context<Self>) -> Self {
        let store = SaiWsDeferredDebugStore::shared(cx);
        let history_ids: Vec<Uuid> = store.read(cx).history().iter().map(|row| row.id).collect();
        let history_list = ListUi::new(&theme, history_ids.len(), ListAlignment::Top);
        cx.observe(&store, |panel, _store, cx| {
            panel.sync_history_list(cx);
            panel.sync_wait_ticker(cx);
            cx.notify();
        })
        .detach();
        let mut panel = Self { store, theme, history_list, history_ids, _wait_ticker: None };
        panel.sync_wait_ticker(cx);
        panel
    }

    pub fn set_theme(&mut self, theme: AppTheme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    fn sync_history_list(&mut self, cx: &mut Context<Self>) {
        let new_ids: Vec<Uuid> = self.store.read(cx).history().iter().map(|row| row.id).collect();
        let old_ids = &self.history_ids;
        let prefix = old_ids.iter().zip(&new_ids).take_while(|(old, new)| old == new).count();
        let suffix = old_ids[prefix..]
            .iter()
            .rev()
            .zip(new_ids[prefix..].iter().rev())
            .take_while(|(old, new)| old == new)
            .count();
        let old_end = old_ids.len() - suffix;
        let new_count = new_ids.len() - suffix - prefix;
        if old_end > prefix || new_count > 0 {
            self.history_list.state().splice(prefix..old_end, new_count);
        }
        self.history_ids = new_ids;
    }

    fn sync_wait_ticker(&mut self, cx: &mut Context<Self>) {
        if self.store.read(cx).fifo_cards().is_empty() {
            self._wait_ticker = None;
            return;
        }
        if self._wait_ticker.is_some() {
            return;
        }
        self._wait_ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(WAIT_TICK).await;
                if this.update(cx, |_panel, cx| cx.notify()).is_err() {
                    break;
                }
            }
        }));
    }

    fn confirm_cancel_queue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let answer = window.prompt(
            PromptLevel::Warning,
            &tr("sai_ws_deferred_debug_reset_dialog_title"),
            Some(tr("sai_ws_deferred_debug_reset_dialog_message").as_str()),
            &[
                PromptButton::Ok(tr("sai_ws_deferred_debug_reset_dialog_confirm").into()),
                PromptButton::Cancel(tr("common_cancel").into()),
            ],
            cx,
        );
        cx.spawn(async move |_this, cx| {
            if answer.await == Ok(0) {
                cx.update(|cx| {
                    SaiWsDeferredOutboundCoordinator::shared(cx).update(cx, |coordinator, cx| {
                        coordinator.fail_all_pending(anyhow::Error::new(SaiWsSendError::NotConnected), cx);
                    });
                });
            }
        })
        .detach();
    }

    fn confirm_history_delete(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let answer = window.prompt(
            PromptLevel::Warning,
            &tr("sai_ws_deferred_debug_delete_dialog_title"),
            Some(tr("sai_ws_deferred_debug_delete_dialog_message").as_str()),
            &[PromptButton::Ok(tr("ctx_delete").into()), PromptButton::Cancel(tr("common_cancel").into())],
            cx,
        );
        let store = self.store.clone();
        cx.spawn(async move |_this, cx| {
            if answer.await == Ok(0) {
                cx.update(|cx| store.update(cx, |store, cx| store.remove_history(id, cx)));
            }
        })
        .detach();
    }

    fn confirm_history_clear_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let answer = window.prompt(
            PromptLevel::Warning,
            &tr("sai_ws_deferred_debug_clear_dialog_title"),
            Some(tr("sai_ws_deferred_debug_clear_dialog_message").as_str()),
            &[
                PromptButton::Ok(tr("sai_ws_deferred_debug_clear_dialog_confirm").into()),
                PromptButton::Cancel(tr("common_cancel").into()),
            ],
            cx,
        );
        let store = self.store.clone();
        cx.spawn(async move |_this, cx| {
            if answer.await == Ok(0) {
                cx.update(|cx| store.update(cx, |store, cx| store.clear_history(cx)));
            }
        })
        .detach();
    }

    fn copy_queue_summary(store: &SaiWsDeferredDebugStore) -> String {
        let mut parts = vec!["WS deferred queue (newest \u{2192} oldest in RAM strip)".to_owned()];
        for card in store.fifo_cards().iter().rev() {
            parts.push(format!(
                "[RAM] {}.{} pos {}/{}\n{}",
                card.service, card.method, card.position_from_wire, card.total_in_fifo, card.body_json,
            ));
        }
        for card in store.disk_backlog_cards() {
            parts.push(format!("[DISK] {}.{}\n{}", card.service, card.method, card.body_json));
        }
        for card in store.ack_debt_disk_cards() {
            parts.push(format!("[ACK_DEBT] {} {}.{}\n{}", card.id, card.service, card.method, card.body_json));
        }
        parts.join("\n\n")
    }

    fn queue_section(&self, cx: &mut Context<Self>) -> Div {
        let theme = &self.theme;
        let store = self.store.read(cx);
        let count = store.fifo_cards().len() + store.disk_backlog_cards().len() + store.ack_debt_disk_cards().len();
        let has_fifo = !store.fifo_cards().is_empty();
        let has_disk = !store.disk_backlog_cards().is_empty();
        let has_ack_debt = !store.ack_debt_disk_cards().is_empty();
        let now = RiseClock::now();

        let content: Div = if !has_fifo && !has_disk && !has_ack_debt {
            div().w_full().flex().justify_center().py(px(20.)).child(
                div()
                    .child(tr("sai_ws_deferred_debug_queue_empty"))
                    .swiftui_font(Font::caption())
                    .text_color(theme.text.secondary),
            )
        } else {
            let queue_strip = (has_fifo || has_disk).then(|| {
                let mut cards: Vec<Div> =
                    store.fifo_cards().iter().rev().map(|card| fifo_mini_card(card, now, theme)).collect();
                cards.extend(store.disk_backlog_cards().iter().map(|card| disk_mini_card(card, theme)));
                horizontal_strip("sai-ws-deferred-queue-strip", color(FIFO_STRIP_TINT).opacity(0.06), theme, cards)
            });
            let ack_debt = has_ack_debt.then(|| {
                let cards: Vec<Div> =
                    store.ack_debt_disk_cards().iter().map(|card| ack_debt_mini_card(card, theme)).collect();
                vstack(10.)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .child(
                        div()
                            .child(tr("sai_ws_deferred_debug_ack_debt_hint"))
                            .swiftui_font(Font::caption2())
                            .text_color(theme.text.secondary.opacity(0.78))
                            .px(px(14.)),
                    )
                    .child(horizontal_strip(
                        "sai-ws-deferred-ack-strip",
                        color(ACK_STRIP_TINT).opacity(0.08),
                        theme,
                        cards,
                    ))
            });
            vstack(10.).align(HorizontalAlignment::Leading).w_full().children(queue_strip).children(ack_debt)
        };

        let copy_store = self.store.clone();
        let this = cx.weak_entity();
        vstack(0.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(
                hstack(8.)
                    .w_full()
                    .child(caption_title(tr("sai_ws_deferred_debug_queue_title"), theme))
                    .child(count_label(count, theme))
                    .child(spacer_min_w(0.))
                    .children((count > 0).then(|| {
                        text_button(
                            "sai-ws-deferred-copy-queue",
                            tr("sai_ws_deferred_debug_copy"),
                            LINK_BLUE,
                            move |_window, cx| {
                                let summary = Self::copy_queue_summary(copy_store.read(cx));
                                UIPasteboard::general().set_string(&summary, cx);
                            },
                        )
                    }))
                    .children(has_fifo.then(|| {
                        text_button(
                            "sai-ws-deferred-reset-queue",
                            tr("sai_ws_deferred_debug_reset"),
                            DESTRUCTIVE_RED,
                            move |window, cx| {
                                let _ = this.update(cx, |panel, cx| panel.confirm_cancel_queue(window, cx));
                            },
                        )
                    }))
                    .px(px(14.))
                    .py(px(8.))
                    .bg(theme.bg._300.opacity(0.35)),
            )
            .child(
                div()
                    .child(tr("sai_ws_deferred_debug_queue_hint"))
                    .swiftui_font(Font::caption2())
                    .text_color(theme.text.secondary.opacity(0.78))
                    .px(px(14.))
                    .pb(px(6.)),
            )
            .child(content)
    }

    fn history_section(&self, cx: &mut Context<Self>) -> Div {
        let theme = self.theme.clone();
        let count = self.store.read(cx).history().len();
        let this = cx.weak_entity();

        let content: AnyElement = if count == 0 {
            div()
                .w_full()
                .flex()
                .justify_center()
                .px(px(HISTORY_PADDING))
                .pt(px(HISTORY_PADDING + 24.))
                .child(
                    div()
                        .child(tr("sai_ws_deferred_debug_history_empty"))
                        .swiftui_font(Font::caption())
                        .text_color(theme.text.secondary),
                )
                .into_any_element()
        } else {
            let row_store = self.store.clone();
            let row_theme = theme.clone();
            let row_panel = this.clone();
            self.history_list
                .element(move |index, _window, cx| {
                    let history = row_store.read(cx).history();
                    let is_last = index + 1 == history.len();
                    let Some(row) = history.get(index).cloned() else {
                        return div().into_any_element();
                    };
                    div()
                        .w_full()
                        .px(px(HISTORY_PADDING))
                        .pt(px(if index == 0 { HISTORY_PADDING } else { HISTORY_ROW_SPACING }))
                        .pb(px(if is_last { HISTORY_PADDING } else { 0. }))
                        .child(history_row(row, &row_theme, row_panel.clone()))
                        .into_any_element()
                })
                .into_any_element()
        };

        let copy_store = self.store.clone();
        let header_buttons = (count > 0).then(|| {
            hstack(8.)
                .child(text_button(
                    "sai-ws-deferred-copy-history",
                    tr("sai_ws_deferred_debug_copy_all"),
                    LINK_BLUE,
                    move |_window, cx| {
                        let text = copy_store.read(cx).copy_all_history_text();
                        UIPasteboard::general().set_string(&text, cx);
                    },
                ))
                .child(text_button(
                    "sai-ws-deferred-clear-history",
                    tr("sai_ws_deferred_debug_delete_all"),
                    DESTRUCTIVE_RED,
                    move |window, cx| {
                        let _ = this.update(cx, |panel, cx| panel.confirm_history_clear_all(window, cx));
                    },
                ))
        });

        vstack(0.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(
                hstack(8.)
                    .w_full()
                    .child(caption_title(tr("sai_ws_deferred_debug_history_title"), &theme))
                    .child(count_label(count, &theme))
                    .child(spacer_min_w(0.))
                    .children(header_buttons)
                    .px(px(14.))
                    .py(px(8.))
                    .bg(theme.bg._300.opacity(0.35)),
            )
            .child(div().w_full().h(px(HISTORY_MAX_HEIGHT)).bg(theme.bg._100).child(content))
    }
}

impl Render for SaiWsDeferredDebugPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        vstack(0.)
            .w_full()
            .child(self.queue_section(cx))
            .child(div().w_full().h(px(1.)).bg(self.theme.border._200))
            .child(self.history_section(cx))
    }
}
