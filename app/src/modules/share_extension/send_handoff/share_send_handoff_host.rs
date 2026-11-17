use std::f32::consts::{FRAC_PI_2, TAU};

use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, Render, RenderOnce, Styled, Subscription, Window, black, canvas, div, px, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::{FormatArg, format_printf, tr};
use rise_theme::ui_color_system::UIColor;
use rise_theme::{Appearance, RiseThemeAppearance, ThemeStore};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::IconUi;
use rise_ui::loader_ui::{arc_points, round_cap, stroke_polyline};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::button_style::{BorderedStyle, system_button};
use rise_ui::swiftui::font::{Font, SwiftUiFont};
use rise_ui::swiftui::lifecycle::on_change;
use rise_ui::swiftui::motion::{ease_in_out, linear};
use rise_ui::swiftui::progress_view::{ControlSize, progress_view};
use rise_ui::swiftui::transition::{Presence, Transition};
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};

use crate::core::lib::byte_format::ByteFormat;
use crate::core::media_viewer::media_viewer_presentation_state::media_viewer_presentation;
use crate::core::widgets::navigations::bottom_chrome_insets::BottomChromeInsetsModifiers;
use crate::modules::share_extension::send_handoff::share_send_handoff_interactions::ShareSendHandoffInteractionsStore;
use crate::modules::share_extension::send_handoff::share_send_handoff_models::ShareSendHandoffPhase;
use crate::modules::share_extension::send_handoff::share_send_handoff_services::ShareSendHandoffServicesStore;

const PRESENCE_DURATION: f64 = 0.2;
const GAUGE_SIDE: f32 = 37.0;
const GAUGE_LINE_WIDTH: f32 = 4.0;
const GAUGE_TRACK_OPACITY: f32 = 0.3;
const GAUGE_ANIMATION_DURATION: f64 = 0.2;
const DIMMING_OPACITY: f32 = 0.92;
const CONTENT_MAXIMUM_WIDTH: f32 = 440.0;
const CONTENT_HORIZONTAL_PADDING: f32 = 28.0;
const STACK_SPACING: f32 = 18.0;
const SUCCESS_STACK_SPACING: f32 = 16.0;
const BUTTON_SPACING: f32 = 12.0;
const PROGRESS_BUTTON_MINIMUM_HEIGHT: f32 = 48.0;
const RESULT_BUTTON_MINIMUM_HEIGHT: f32 = 46.0;
const STATUS_SYMBOL_SIZE: f32 = 46.0;
const SUCCESS_SYMBOL_SIZE: f32 = 62.0;
const BUTTON_TINT_OPACITY: f32 = 0.9;
const PROGRESS_DETAIL_OPACITY: f32 = 0.62;
const FAILURE_DETAIL_OPACITY: f32 = 0.68;
const PROGRESS_DETAIL_LINE_LIMIT: usize = 4;
const FAILURE_DETAIL_LINE_LIMIT: usize = 8;

pub struct ShareSendHandoffHost {
    services: Entity<ShareSendHandoffServicesStore>,
    presence: Presence<ShareSendHandoffPhase>,
    focus_handle: FocusHandle,
    focused_session_id: Option<String>,
    _services_observation: Subscription,
}

impl ShareSendHandoffHost {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let services = ShareSendHandoffServicesStore::shared(cx);
        let phase = services.read(cx).phase().clone();
        let presence = Presence::new(Self::presented(phase)).with_transition(Transition::opacity());
        let services_observation = cx.observe(&services, |host, services, cx| {
            let phase = services.read(cx).phase().clone();
            host.presence
                .set_in(Self::presented(phase), ease_in_out(PRESENCE_DURATION), RiseClock::monotonic(), cx);
            cx.notify();
        });
        Self {
            services,
            presence,
            focus_handle: cx.focus_handle(),
            focused_session_id: None,
            _services_observation: services_observation,
        }
    }

    fn presented(phase: ShareSendHandoffPhase) -> Option<ShareSendHandoffPhase> {
        (phase != ShareSendHandoffPhase::Idle).then_some(phase)
    }

    fn interactions(cx: &mut App, action: impl FnOnce(&mut ShareSendHandoffInteractionsStore, &mut Context<ShareSendHandoffInteractionsStore>)) {
        ShareSendHandoffInteractionsStore::shared(cx).update(cx, action);
    }

    fn overlay(&self, content: AnyElement, opacity: f32, can_cancel: bool) -> AnyElement {
        div()
            .id("share-send-handoff-overlay")
            .track_focus(&self.focus_handle)
            .layer_keys(LayerCancel::dismiss(move |_, cx| {
                if can_cancel {
                    Self::interactions(cx, |interactions, cx| interactions.cancel_and_discard(cx));
                }
            }))
            .size_full()
            .occlude()
            .opacity(opacity)
            .bg(black().opacity(DIMMING_OPACITY))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div().px(px(CONTENT_HORIZONTAL_PADDING)).w_full().flex().justify_center().child(
                    div().w_full().max_w(px(CONTENT_MAXIMUM_WIDTH)).child(content),
                ),
            )
            .occludes_bottom_chrome()
            .into_any_element()
    }

    fn title(text: String) -> impl IntoElement {
        div()
            .swiftui_font(Font::title3().bold())
            .text_color(white())
            .text_center()
            .child(text)
    }

    fn detail(text: String, opacity: f32, line_limit: usize) -> impl IntoElement {
        div()
            .swiftui_font(Font::subheadline())
            .text_color(white().opacity(opacity))
            .text_center()
            .line_clamp(line_limit)
            .child(text)
    }

    fn symbol(name: &str, side: f32, color: Hsla) -> AnyElement {
        match IconUi::sized(name, px(side), color, name.ends_with(".fill")) {
            Some(icon) => icon.into_any_element(),
            None => div().size(px(side)).into_any_element(),
        }
    }

    fn system_color(color: UIColor, cx: &mut App) -> Hsla {
        let appearance = match ThemeStore::shared(cx).read(cx).resolved_appearance() {
            RiseThemeAppearance::Light => Appearance::Light,
            RiseThemeAppearance::Dark => Appearance::Dark,
        };
        color.resolved_color(appearance)
    }

    fn button_label(text: String, minimum_height: f32, font: Option<Font>) -> impl IntoElement {
        let label = div()
            .w_full()
            .min_h(px(minimum_height))
            .flex()
            .items_center()
            .justify_center()
            .text_center();
        match font {
            Some(font) => label.swiftui_font(font).child(text),
            None => label.child(text),
        }
    }

    fn progress_content(title: String, detail: String, progress: Option<f64>, allows_cancellation: bool) -> AnyElement {
        let indicator = match progress {
            Some(progress) => ShareSendHandoffGauge { fraction: progress.clamp(0.0, 1.0) }.into_any_element(),
            None => progress_view().control_size(ControlSize::Large).tint(white()).into_any_element(),
        };
        let mut stack = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(STACK_SPACING))
            .child(indicator)
            .child(Self::title(title))
            .child(Self::detail(detail, PROGRESS_DETAIL_OPACITY, PROGRESS_DETAIL_LINE_LIMIT));
        if allows_cancellation {
            stack = stack.child(
                div()
                    .id("share-send-handoff-cancel-hint")
                    .w_full()
                    .tooltip_label(tr("share_send_handoff_cancel_hint"))
                    .child(
                    system_button("share-send-handoff-cancel", BorderedStyle::Bordered)
                        .tint(white().opacity(BUTTON_TINT_OPACITY))
                        .on_press(|_, cx| {
                            Self::interactions(cx, |interactions, cx| interactions.cancel_and_discard(cx));
                        })
                        .child(Self::button_label(
                            tr("common_cancel"),
                            PROGRESS_BUTTON_MINIMUM_HEIGHT,
                            Some(Font::headline()),
                        )),
                ),
            );
        }
        stack.into_any_element()
    }

    fn retry_button() -> impl IntoElement {
        div().flex_1().child(
            system_button("share-send-handoff-retry", BorderedStyle::Prominent)
                .on_press(|_, cx| {
                    Self::interactions(cx, |interactions, cx| interactions.retry(cx));
                })
                .child(Self::button_label(
                    tr("share_send_handoff_retry"),
                    RESULT_BUTTON_MINIMUM_HEIGHT,
                    Some(Font::body().weight(FontWeight::SEMIBOLD)),
                )),
        )
    }

    fn failure_content(message: String, commit_started: bool, cx: &mut App) -> AnyElement {
        let leading = if !commit_started {
            system_button("share-send-handoff-failure-cancel", BorderedStyle::Bordered)
                .tint(white().opacity(BUTTON_TINT_OPACITY))
                .on_press(|_, cx| {
                    Self::interactions(cx, |interactions, cx| interactions.cancel_and_discard(cx));
                })
                .child(Self::button_label(tr("common_cancel"), RESULT_BUTTON_MINIMUM_HEIGHT, None))
        } else {
            system_button("share-send-handoff-later", BorderedStyle::Bordered)
                .tint(white().opacity(BUTTON_TINT_OPACITY))
                .on_press(|_, cx| {
                    Self::interactions(cx, |interactions, cx| interactions.dismiss_committed_until_next_launch(cx));
                })
                .child(Self::button_label(
                    tr("share_send_handoff_later"),
                    RESULT_BUTTON_MINIMUM_HEIGHT,
                    None,
                ))
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(STACK_SPACING))
            .child(Self::symbol(
                "exclamationmark.arrow.triangle.2.circlepath",
                STATUS_SYMBOL_SIZE,
                Self::system_color(UIColor::system_orange(), cx),
            ))
            .child(Self::title(tr("share_send_handoff_failed_title")))
            .child(Self::detail(message, FAILURE_DETAIL_OPACITY, FAILURE_DETAIL_LINE_LIMIT))
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap(px(BUTTON_SPACING))
                    .child(div().flex_1().child(leading))
                    .child(Self::retry_button()),
            )
            .into_any_element()
    }

    fn partial_content(message: String, cx: &mut App) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(STACK_SPACING))
            .child(Self::symbol(
                "checkmark.circle.badge.exclamationmark",
                STATUS_SYMBOL_SIZE,
                Self::system_color(UIColor::system_orange(), cx),
            ))
            .child(Self::title(tr("share_send_handoff_partial_title")))
            .child(Self::detail(message, FAILURE_DETAIL_OPACITY, FAILURE_DETAIL_LINE_LIMIT))
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap(px(BUTTON_SPACING))
                    .child(
                        div().flex_1().child(
                            system_button("share-send-handoff-done", BorderedStyle::Bordered)
                                .tint(white().opacity(BUTTON_TINT_OPACITY))
                                .on_press(|_, cx| {
                                    Self::interactions(cx, |interactions, cx| {
                                        interactions.finish_acknowledged_partial(cx);
                                    });
                                })
                                .child(Self::button_label(tr("common_done"), RESULT_BUTTON_MINIMUM_HEIGHT, None)),
                        ),
                    )
                    .child(Self::retry_button()),
            )
            .into_any_element()
    }

    fn success_content(destination_count: i64, cx: &mut App) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(SUCCESS_STACK_SPACING))
            .child(Self::symbol(
                "checkmark.circle.fill",
                SUCCESS_SYMBOL_SIZE,
                Self::system_color(UIColor::system_green(), cx),
            ))
            .child(Self::title(tr("share_send_handoff_sent")))
            .child(
                div()
                    .swiftui_font(Font::subheadline())
                    .text_color(white().opacity(PROGRESS_DETAIL_OPACITY))
                    .child(format_printf(
                        &tr("share_send_handoff_destination_count"),
                        &[FormatArg::from(destination_count)],
                    )),
            )
            .into_any_element()
    }

    fn uploading_detail(filename: &str, item: i64, total: i64, sent_bytes: i64, file_bytes: i64) -> String {
        let position = format!("{filename}\n{item} / {total}");
        if file_bytes <= 0 {
            return position;
        }
        let transferred = format_printf(
            &tr("chat_media_upload_progress"),
            &[
                FormatArg::Text(ByteFormat::telegram_transferred(sent_bytes, file_bytes).into()),
                FormatArg::Text(ByteFormat::telegram(file_bytes).into()),
            ],
        );
        position + "\n" + &transferred
    }

    fn content(&self, phase: &ShareSendHandoffPhase, can_cancel: bool, cx: &mut App) -> Option<AnyElement> {
        Some(match phase {
            ShareSendHandoffPhase::Idle => return None,
            ShareSendHandoffPhase::Preparing { .. } => Self::progress_content(
                tr("share_send_handoff_preparing"),
                tr("share_send_handoff_keep_open"),
                None,
                can_cancel,
            ),
            ShareSendHandoffPhase::Uploading { filename, item, total, fraction, sent_bytes, file_bytes, .. } => {
                Self::progress_content(
                    tr("share_send_handoff_uploading"),
                    Self::uploading_detail(filename, *item, *total, *sent_bytes, *file_bytes),
                    Some(*fraction),
                    can_cancel,
                )
            }
            ShareSendHandoffPhase::Sending { .. } => Self::progress_content(
                tr("share_send_handoff_sending"),
                tr("share_send_handoff_finishing"),
                None,
                false,
            ),
            ShareSendHandoffPhase::Failed { message, commit_started, .. } => {
                Self::failure_content(message.clone(), *commit_started, cx)
            }
            ShareSendHandoffPhase::Partial { message, .. } => Self::partial_content(message.clone(), cx),
            ShareSendHandoffPhase::Succeeded { destination_count, .. } => {
                Self::success_content(*destination_count, cx)
            }
        })
    }
}

impl Focusable for ShareSendHandoffHost {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ShareSendHandoffHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let services = self.services.read(cx);
        let can_cancel = services.can_cancel_current();
        let active_session_id = services.phase().active_session_id().map(str::to_owned);

        let viewer = media_viewer_presentation(cx);
        on_change(
            "share-send-handoff-active-session",
            active_session_id.clone(),
            true,
            window,
            cx,
            move |_, session_id, _, cx| {
                if session_id.is_some() && viewer.read(cx).is_presented() {
                    viewer.update(cx, |viewer, cx| viewer.dismiss(cx));
                }
            },
        );

        if active_session_id.is_some() && self.focused_session_id != active_session_id {
            self.focused_session_id = active_session_id;
            window.focus(&self.focus_handle, cx);
        }

        let now = RiseClock::monotonic();
        let opacity = self.presence.effect(now).opacity;
        let Some(phase) = self.presence.value(now).cloned() else {
            return div().into_any_element();
        };
        if self.presence.is_running(now) {
            window.request_animation_frame();
        }
        match self.content(&phase, can_cancel, cx) {
            Some(content) => self.overlay(content, opacity, can_cancel),
            None => div().into_any_element(),
        }
    }
}

#[derive(IntoElement)]
struct ShareSendHandoffGauge {
    fraction: f64,
}

impl RenderOnce for ShareSendHandoffGauge {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let target = self.fraction as f32;
        let state = window.use_keyed_state("share-send-handoff-gauge", cx, |_, _| Animated::new(target));
        let now = RiseClock::monotonic();
        let reduce = reduce_motion(cx);
        let fraction = state.update(cx, |animated, _| {
            animated.set(target, linear(GAUGE_ANIMATION_DURATION), now, reduce);
            animated.frame_value(now, window)
        });
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                let center = bounds.center();
                let radius = GAUGE_SIDE / 2.0;
                let track = arc_points(center, radius, 0.0, TAU);
                stroke_polyline(&track, GAUGE_LINE_WIDTH, true, white().opacity(GAUGE_TRACK_OPACITY), window);
                if fraction <= 0.0 {
                    return;
                }
                let arc = arc_points(center, radius, -FRAC_PI_2, TAU * fraction);
                stroke_polyline(&arc, GAUGE_LINE_WIDTH, false, white(), window);
                if let (Some(first), Some(last)) = (arc.first(), arc.last()) {
                    round_cap(*first, GAUGE_LINE_WIDTH, white(), window);
                    round_cap(*last, GAUGE_LINE_WIDTH, white(), window);
                }
            },
        )
        .size(px(GAUGE_SIDE))
        .flex_none()
    }
}
