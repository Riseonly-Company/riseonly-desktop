use std::path::PathBuf;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, ClipboardItem, Context, ElementId, Empty, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Task,
    Window, black, div, prelude::*, px, white,
};
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription};
use rise_i18n::tr;
use rise_ui::error_text_ui::system_red;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_theme::ThemeStore;
use rise_ui::IconUi;
use rise_ui::page_header_toolbar_button::{PageHeaderMetrics, PageHeaderToolbarSlot};
use rise_ui::swiftui::progress_view::progress_view;
use rise_ui::swiftui::stacks::hstack;

use crate::modules::user::components::profile_content::profile_performance_trace::{
    PROFILE_PERFORMANCE_TRACE_STATE_DID_CHANGE, ProfilePerformanceTrace,
};

const CONTROL_SIDE: f32 = 36.0;
const ICON_SIDE: f32 = 15.0;

pub struct ProfilePerformanceTraceControlState {
    scope: SharedString,
    is_recording: bool,
    is_finishing: bool,
    export_url: Option<PathBuf>,
    _state_observer: RiseSubscription,
    _state_pump: Task<()>,
}

impl ProfilePerformanceTraceControlState {
    fn new(scope: SharedString, cx: &mut Context<Self>) -> Self {
        let (sender, mut inbox) = mpsc::unbounded::<()>();
        let observer =
            RiseAppEvents::default_center().observe(PROFILE_PERFORMANCE_TRACE_STATE_DID_CHANGE, move |_| {
                let _ = sender.unbounded_send(());
            });
        let pump = cx.spawn(async move |this, cx| {
            while inbox.next().await.is_some() {
                let updated = this.update(cx, |this, cx| this.synchronize_state(cx));
                if updated.is_err() {
                    break;
                }
            }
        });
        let trace = ProfilePerformanceTrace::shared();
        Self {
            is_recording: trace.is_recording(&scope),
            is_finishing: trace.is_finishing(),
            export_url: trace.latest_trace_url(),
            scope,
            _state_observer: observer,
            _state_pump: pump,
        }
    }

    fn synchronize_state(&mut self, cx: &mut Context<Self>) {
        let trace = ProfilePerformanceTrace::shared();
        self.is_recording = trace.is_recording(&self.scope);
        self.is_finishing = trace.is_finishing();
        self.export_url = trace.latest_trace_url();
        cx.notify();
    }

    fn toggle_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let trace = ProfilePerformanceTrace::shared();
        if trace.is_recording(&self.scope) {
            self.is_recording = false;
            self.is_finishing = true;
            cx.notify();
            trace.stop("manual", None);
            return;
        }

        if !trace.start(&self.scope, window, cx) {
            self.synchronize_state(cx);
            return;
        }

        self.export_url = None;
        self.is_finishing = false;
        self.is_recording = true;
        cx.notify();
    }
}

/// trace control, sharing shows the file in its folder and copies its path
#[derive(IntoElement)]
pub struct ProfilePerformanceTraceControl {
    scope: SharedString,
    header_chrome: bool,
}

impl ProfilePerformanceTraceControl {
    pub fn new(scope: impl Into<SharedString>) -> Self {
        Self { scope: scope.into(), header_chrome: false }
    }

    /// in a page header every control gets its own theme chrome circle like its neighbours
    pub fn header_chrome(mut self, header_chrome: bool) -> Self {
        self.header_chrome = header_chrome;
        self
    }
}

impl RenderOnce for ProfilePerformanceTraceControl {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if !ProfilePerformanceTrace::is_control_available() {
            return Empty.into_any_element();
        }
        let scope = self.scope;
        let header_chrome = self.header_chrome;
        let text_primary = ThemeStore::shared(cx).read(cx).current_theme().text.primary;
        let on_backing = if header_chrome { text_primary } else { white() };
        let key = ElementId::Name(format!("profile-performance-trace-control-{scope}").into());
        let state = window.use_keyed_state(key, cx, {
            let scope = scope.clone();
            move |_, cx| ProfilePerformanceTraceControlState::new(scope, cx)
        });
        let (is_recording, is_finishing, export_url) = {
            let state = state.read(cx);
            (state.is_recording, state.is_finishing, state.export_url.clone())
        };

        let primary = if is_finishing {
            div()
                .id(ElementId::Name(format!("profile-performance-trace-finishing-{scope}").into()))
                .size(px(CONTROL_SIDE))
                .flex()
                .items_center()
                .justify_center()
                .aria_label(tr("profile_performance_trace_finishing"))
                .tooltip_label(tr("profile_performance_trace_finishing"))
                .child(progress_view().tint(on_backing))
        } else {
            let icon_color = if is_recording { on_backing } else { system_red(cx) };
            let label = if is_recording { "profile_performance_trace_stop" } else { "profile_performance_trace_start" };
            let toggle_state = state.clone();
            div()
                .id(ElementId::Name(format!("profile-performance-trace-toggle-{scope}").into()))
                .size(px(CONTROL_SIDE))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .aria_label(tr(label))
                .tooltip_label(tr(label))
                .on_click(move |_, window, cx| {
                    toggle_state.update(cx, |state, cx| state.toggle_recording(window, cx));
                })
                .children(IconUi::sized(
                    if is_recording { "stop.fill" } else { "waveform.path.ecg" },
                    px(ICON_SIDE),
                    icon_color,
                    is_recording,
                ))
        };

        let export = export_url.filter(|_| !is_recording && !is_finishing).map(|export_url| {
            div()
                .id(ElementId::Name(format!("profile-performance-trace-export-{scope}").into()))
                .size(px(CONTROL_SIDE))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .aria_label(tr("profile_performance_trace_export"))
                .tooltip_label(tr("profile_performance_trace_export"))
                .on_click(move |_, _window, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(export_url.to_string_lossy().into_owned()));
                    cx.reveal_path(&export_url);
                })
                .children(IconUi::sized("square.and.arrow.up", px(ICON_SIDE), on_backing, false))
        });

        if header_chrome {
            return hstack(6.0)
                .child(PageHeaderToolbarSlot::new(primary).size(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
                .children(export.map(|export| {
                    PageHeaderToolbarSlot::new(export).size(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE)
                }))
                .into_any_element();
        }
        hstack(6.0)
            .px(px(6.0))
            .rounded_full()
            .bg(black().opacity(0.86))
            .child(primary)
            .children(export)
            .into_any_element()
    }
}

