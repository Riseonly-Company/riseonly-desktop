use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, ElementId, FontWeight, Hsla, SharedString, Window, rgb, white};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::character_count;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::text_selection::selectable_text;
use rise_ui::swiftui::transition::Presence;

use crate::core::lib::sai::modules::sai_debugger::sai_debug_storage_panel_views::{
    SaiDebugBodyFormat, SaiDebugInspectorMetrics, SaiDebugJsonScrollBlock, line_limited,
};
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_entry::{
    SaiHttpTraceEntry, SaiTraceCacheSource, SaiTraceTag, SaiTraceTransport,
};
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_recorder::SaiHttpTraceRecorder;

const TIME_FORMAT: &str = "HH:mm:ss.SSS";
const EM_DASH: &str = "\u{2014}";

const LINK_BLUE: u32 = 0x29b6f6;
const DESTRUCTIVE_RED: u32 = 0xe11d48;
const PATH_TEAL: u32 = 0x26a69a;
const REQUEST_BLOCK: u32 = 0x1a3a52;
const RESPONSE_BLOCK: u32 = 0x1b5e20;
const CACHE_MEMORY: u32 = 0xffb020;
const CACHE_DISK: u32 = 0x9c27b0;
const STATUS_FAIL: u32 = 0xff4444;
const STATUS_NEUTRAL: u32 = 0x888888;

thread_local! {
    static TIME_FORMATTER: DateFormatter = {
        let mut formatter = DateFormatter::new();
        formatter.set_date_format(TIME_FORMAT);
        formatter
    };
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
enum Panel {
    #[default]
    Overview,
    Headers,
}

/// one expand flag with the two branches it swaps in an ease in out of 0.18s
#[derive(Clone, Debug)]
struct ExpandState {
    expanded: bool,
    open: Presence<()>,
    closed: Presence<()>,
}

impl Default for ExpandState {
    fn default() -> Self {
        Self {
            expanded: false,
            open: Presence::new(None),
            closed: Presence::new(Some(())),
        }
    }
}

impl ExpandState {
    fn toggle(&mut self, cx: &App) {
        self.expanded = !self.expanded;
        let now = RiseClock::monotonic();
        self.open.set_in(self.expanded.then_some(()), ease_in_out(0.18), now, cx);
        self.closed.set_in((!self.expanded).then_some(()), ease_in_out(0.18), now, cx);
    }
}

#[derive(Clone, Debug, Default)]
struct SaiHttpTraceCardState {
    panel: Panel,
    bodies: ExpandState,
    request_headers: ExpandState,
    response_headers: ExpandState,
}

#[derive(Clone, Copy)]
enum HeadersKind {
    Request,
    Response,
}

impl SaiHttpTraceCardState {
    fn headers_mut(&mut self, kind: HeadersKind) -> &mut ExpandState {
        match kind {
            HeadersKind::Request => &mut self.request_headers,
            HeadersKind::Response => &mut self.response_headers,
        }
    }

    fn headers(&self, kind: HeadersKind) -> &ExpandState {
        match kind {
            HeadersKind::Request => &self.request_headers,
            HeadersKind::Response => &self.response_headers,
        }
    }
}

fn pill(text: impl Into<SharedString>, fill: impl Into<Hsla>, horizontal: f32, vertical: f32) -> Div {
    let fill: Hsla = fill.into();
    div()
        .child(text.into())
        .swiftui_font(Font::caption2().weight(FontWeight::EXTRA_BOLD))
        .text_color(white())
        .line_clamp(1)
        .px(px(horizontal))
        .py(px(vertical))
        .bg(fill)
        .capsule()
}

fn section_title(text: impl Into<SharedString>, theme: &AppTheme) -> Div {
    div()
        .child(text.into())
        .swiftui_font(Font::system(10., FontWeight::EXTRA_BOLD, FontDesign::Default))
        .text_color(theme.text.secondary.opacity(0.85))
        .tracking(0.5)
}

fn pending_cluster(theme: &AppTheme) -> Div {
    hstack(4.)
        .child(progress_view().control_size(ControlSize::Mini))
        .child(
            div()
                .child("\u{b7}\u{b7}\u{b7}")
                .swiftui_font(Font::caption2().weight(FontWeight::EXTRA_BOLD))
                .text_color(theme.text.secondary),
        )
        .px(px(6.))
        .py(px(2.))
}

#[derive(IntoElement)]
pub struct SaiHttpTraceCardView {
    entry: SaiHttpTraceEntry,
    theme: AppTheme,
    on_delete: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl SaiHttpTraceCardView {
    pub fn new(entry: SaiHttpTraceEntry, theme: AppTheme) -> Self {
        Self { entry, theme, on_delete: None }
    }

    pub fn on_delete(mut self, on_delete: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_delete = Some(Rc::new(on_delete));
        self
    }

    fn element_id(&self, part: &str) -> ElementId {
        ElementId::Name(SharedString::from(format!("sai-http-trace-{}-{}", self.entry.id, part)))
    }

    fn panel_switcher(&self, state: &gpui::Entity<SaiHttpTraceCardState>, current: Panel) -> Div {
        hstack(8.)
            .w_full()
            .child(self.panel_chip("Overview", Panel::Overview, state, current))
            .child(self.panel_chip("Headers", Panel::Headers, state, current))
            .child(spacer_min_w(0.))
            .px(px(10.))
            .pb(px(6.))
    }

    fn panel_chip(
        &self,
        title: &'static str,
        panel: Panel,
        state: &gpui::Entity<SaiHttpTraceCardState>,
        current: Panel,
    ) -> PlainButton {
        let on = current == panel;
        let theme = &self.theme;
        let state = state.clone();
        plain(self.element_id(title))
            .on_press(move |_window, cx| {
                state.update(cx, |state, cx| {
                    state.panel = panel;
                    cx.notify();
                });
            })
            .child(
                div()
                    .child(title)
                    .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                    .text_color(if on { theme.text.primary } else { theme.text.secondary })
                    .px(px(12.))
                    .py(px(6.))
                    .bg(if on { theme.bg._300.opacity(0.95) } else { theme.bg._100.opacity(0.5) })
                    .clip_shape(Shape::continuous(8.)),
            )
    }

    fn overview_content(
        &self,
        state: &gpui::Entity<SaiHttpTraceCardState>,
        current: &SaiHttpTraceCardState,
        window: &Window,
    ) -> Div {
        let theme = self.theme.clone();
        let now = RiseClock::monotonic();
        let request_id = self.element_id("request-body");
        let request_body = self.entry.request_body_wire.clone();
        let block_theme = theme.clone();
        let open_blocks = current.bodies.expanded.then(|| {
            let request_block = current.bodies.open.render_transitioned(now, window, |_| {
                div().w_full().child(
                    SaiDebugJsonScrollBlock::new(request_id, "REQUEST BODY", request_body, block_theme)
                        .background(Hsla::from(rgb(REQUEST_BLOCK)).opacity(0.18)),
                )
            });
            let response_block =
                current.bodies.open.render_transitioned(now, window, |_| self.response_overview_block());
            vstack(12.)
                .align(HorizontalAlignment::Leading)
                .w_full()
                .flex_shrink_0()
                .children(request_block)
                .children(response_block)
        });

        vstack(12.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(self.overview_bodies_summary(state, current, window))
            .children(open_blocks)
            .p(px(10.))
            .bg(theme.bg._100)
    }

    fn response_overview_block(&self) -> Div {
        let theme = &self.theme;
        let entry = &self.entry;
        let is_waiting = entry.is_pending
            && entry.response_body_preview.is_empty()
            && entry.transport_error.as_deref().unwrap_or_default().is_empty();
        let body = if is_waiting {
            div()
                .flex()
                .justify_center()
                .w_full()
                .py(px(14.))
                .child(progress_view().control_size(ControlSize::Small))
        } else {
            div().w_full().child(
                div()
                    .id(self.element_id("response-scroll"))
                    .w_full()
                    .max_h(px(SaiDebugInspectorMetrics::JSON_BODY_SCROLL_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .w_full()
                            .swiftui_font(Font::system(11., FontWeight::NORMAL, FontDesign::Monospaced))
                            .text_color(theme.text.primary)
                            .child(selectable_text(
                                self.element_id("response-text"),
                                self.response_body_for_overview(),
                            )),
                    ),
            )
        };

        vstack(6.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(section_title("RESPONSE BODY", theme))
            .child(body)
            .p(px(10.))
            .bg(Hsla::from(rgb(RESPONSE_BLOCK)).opacity(0.12))
            .continuous_corners(10.)
    }

    fn overview_bodies_summary(
        &self,
        state: &gpui::Entity<SaiHttpTraceCardState>,
        current: &SaiHttpTraceCardState,
        window: &Window,
    ) -> Div {
        let theme = &self.theme;
        let toggle_state = state.clone();
        let toggle = plain(self.element_id("bodies-toggle"))
            .on_press(move |_window, cx| {
                toggle_state.update(cx, |state, cx| {
                    state.bodies.toggle(cx);
                    cx.notify();
                });
            })
            .child(
                div()
                    .child(if current.bodies.expanded { "Hide" } else { "Show" })
                    .swiftui_font(Font::caption2().weight(FontWeight::SEMIBOLD))
                    .text_color(rgb(LINK_BLUE)),
            );

        let now = RiseClock::monotonic();
        let secondary = theme.text.secondary.opacity(0.9);
        let summary_line = |text: String| {
            line_limited(
                div()
                    .child(text)
                    .swiftui_font(Font::system(11., FontWeight::NORMAL, FontDesign::Monospaced))
                    .text_color(secondary),
                2,
            )
        };
        let request_line = format!("Request \u{b7} {}", self.overview_request_summary());
        let response_line = format!("Response \u{b7} {}", self.overview_response_summary());
        let summary = if current.bodies.expanded {
            None
        } else {
            current.bodies.closed.render_transitioned(now, window, |_| {
                vstack(4.)
                    .align(HorizontalAlignment::Leading)
                    .child(summary_line(request_line))
                    .child(summary_line(response_line))
            })
        };

        vstack(8.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(
                hstack(8.)
                    .align(VerticalAlignment::FirstTextBaseline)
                    .w_full()
                    .child(section_title("Bodies", theme))
                    .child(spacer_min_w(0.))
                    .child(toggle),
            )
            .children(summary)
            .p(px(10.))
            .bg(theme.bg._300.opacity(0.25))
            .continuous_corners(10.)
            .flex_shrink_0()
    }

    fn overview_request_summary(&self) -> String {
        SaiDebugBodyFormat::size_label_char_count(character_count(&self.entry.request_body_wire) as i64)
    }

    fn overview_response_summary(&self) -> String {
        let entry = &self.entry;
        if let Some(transport_error) = entry.transport_error.as_deref().filter(|error| !error.is_empty()) {
            return SaiDebugBodyFormat::one_line_preview(
                transport_error,
                SaiDebugInspectorMetrics::COLLAPSED_PREVIEW_MAX_CHARS,
            );
        }
        if entry.response_body_preview.is_empty() && entry.has_status_code() {
            return EM_DASH.to_owned();
        }
        SaiDebugBodyFormat::size_label_char_count(character_count(&entry.response_body_preview) as i64)
    }

    fn response_body_for_overview(&self) -> String {
        let entry = &self.entry;
        if let Some(transport_error) = entry.transport_error.as_deref().filter(|error| !error.is_empty()) {
            return transport_error.to_owned();
        }
        if entry.response_body_preview.is_empty() && entry.has_status_code() {
            return EM_DASH.to_owned();
        }
        SaiHttpTraceRecorder::pretty_json_string_from_raw_utf8(&entry.response_body_preview)
    }

    fn headers_content(
        &self,
        state: &gpui::Entity<SaiHttpTraceCardState>,
        current: &SaiHttpTraceCardState,
        window: &Window,
    ) -> Div {
        vstack(12.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(self.headers_section(
                "REQUEST HEADERS",
                &self.entry.request_headers,
                HeadersKind::Request,
                state,
                current,
                window,
            ))
            .child(self.headers_section(
                "RESPONSE HEADERS",
                &self.entry.response_headers,
                HeadersKind::Response,
                state,
                current,
                window,
            ))
            .p(px(10.))
            .bg(self.theme.bg._100)
    }

    fn headers_section(
        &self,
        title: &'static str,
        headers: &HashMap<String, String>,
        kind: HeadersKind,
        state: &gpui::Entity<SaiHttpTraceCardState>,
        current: &SaiHttpTraceCardState,
        window: &Window,
    ) -> Div {
        let theme = &self.theme;
        let expand = current.headers(kind);
        let toggle = (!headers.is_empty()).then(|| {
            let toggle_state = state.clone();
            plain(self.element_id(&format!("{title}-toggle")))
                .on_press(move |_window, cx| {
                    toggle_state.update(cx, |state, cx| {
                        state.headers_mut(kind).toggle(cx);
                        cx.notify();
                    });
                })
                .child(
                    div()
                        .child(if expand.expanded { "Hide" } else { "Show" })
                        .swiftui_font(Font::caption2().weight(FontWeight::SEMIBOLD))
                        .text_color(rgb(LINK_BLUE)),
                )
        });

        let section = vstack(6.).align(HorizontalAlignment::Leading).w_full().child(
            hstack(8.)
                .align(VerticalAlignment::FirstTextBaseline)
                .w_full()
                .child(section_title(title, theme))
                .child(spacer_min_w(0.))
                .children(toggle),
        );

        if headers.is_empty() {
            return section.child(
                div()
                    .child("(none)")
                    .swiftui_font(Font::system(11., FontWeight::NORMAL, FontDesign::Monospaced))
                    .text_color(theme.text.secondary.opacity(0.7)),
            );
        }

        let now = RiseClock::monotonic();
        let mut keys: Vec<&String> = headers.keys().collect();
        keys.sort();
        let scroll_id = self.element_id(&format!("{title}-scroll"));
        let rows = keys
            .iter()
            .enumerate()
            .map(|(index, key)| {
                let value = headers.get(*key).map(String::as_str).unwrap_or_default();
                div()
                    .swiftui_font(Font::system(11., FontWeight::NORMAL, FontDesign::Monospaced))
                    .text_color(theme.text.primary)
                    .child(selectable_text(
                        self.element_id(&format!("{title}-row-{index}")),
                        format!("{key}: {value}"),
                    ))
            })
            .collect::<Vec<_>>();
        let count = headers.len();
        let body = if expand.expanded {
            expand.open.render_transitioned(now, window, |_| {
                div().w_full().child(
                    div()
                        .id(scroll_id)
                        .w_full()
                        .max_h(px(SaiDebugInspectorMetrics::JSON_BODY_SCROLL_MAX_HEIGHT))
                        .overflow_y_scroll()
                        .child(vstack(4.).align(HorizontalAlignment::Leading).w_full().children(rows)),
                )
            })
        } else {
            expand.closed.render_transitioned(now, window, |_| {
                div()
                    .child(format!("{count} header{}", if count == 1 { "" } else { "s" }))
                    .swiftui_font(Font::system(11., FontWeight::NORMAL, FontDesign::Monospaced))
                    .text_color(theme.text.secondary.opacity(0.85))
            })
        };
        section.children(body)
    }

    fn header_row(&self) -> Div {
        let theme = &self.theme;
        let entry = &self.entry;
        let time = TIME_FORMATTER.with(|formatter| formatter.string_from(&entry.created_at));
        let duration = if entry.is_pending { EM_DASH.to_owned() } else { format!("{} ms", entry.duration_ms) };
        let tags = (!entry.trace_tags.is_empty()).then(|| {
            div()
                .id(self.element_id("tags"))
                .max_h(px(entry.trace_tags.len() as f32 * 22.))
                .overflow_y_scroll()
                .child(
                    vstack(3.)
                        .align(HorizontalAlignment::Leading)
                        .children(entry.trace_tags.iter().map(|tag| Self::trace_tag_pill(tag))),
                )
        });
        let delete = self.on_delete.clone().map(|on_delete| {
            self.icon_button("delete", "trash", rgb(DESTRUCTIVE_RED).into(), move |window, cx| {
                on_delete(window, cx)
            })
        });
        let copy_entry = entry.clone();
        let copy = self.icon_button("copy", "doc.on.doc", theme.text.primary, move |_window, cx| {
            UIPasteboard::general().set_string(&copy_entry.copy_block_text(), cx);
        });

        hstack(6.)
            .align(VerticalAlignment::Top)
            .w_full()
            .child(
                vstack(4.)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w(px(0.))
                    .child(
                        hstack(6.)
                            .child(self.transport_pill())
                            .children(self.verb_or_rpc_pill())
                            .child(self.status_cluster()),
                    )
                    .children(tags),
            )
            .child(
                vstack(4.)
                    .align(HorizontalAlignment::Trailing)
                    .child(
                        div()
                            .child(time)
                            .swiftui_font(Font::caption2().weight(FontWeight::MEDIUM).monospaced_digit())
                            .text_color(theme.text.secondary),
                    )
                    .child(
                        div()
                            .child(duration)
                            .swiftui_font(Font::caption2().weight(FontWeight::SEMIBOLD))
                            .text_color(theme.text.secondary),
                    ),
            )
            .children(delete)
            .child(copy)
            .px(px(10.))
            .py(px(8.))
            .bg(theme.bg._300.opacity(0.45))
    }

    fn icon_button(
        &self,
        part: &str,
        symbol: &str,
        color: Hsla,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> PlainButton {
        plain(self.element_id(part)).on_press(action).child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .frame_size(36., 32.)
                .bg(self.theme.bg._300.opacity(0.9))
                .clip_shape(Shape::continuous(8.))
                .children(IconUi::sized(symbol, px(15.), color, false)),
        )
    }

    fn trace_tag_pill(tag: &str) -> Div {
        let fill: u32 = match tag {
            SaiTraceTag::SCREEN_REFRESH => 0x00897b,
            SaiTraceTag::SYSTEM => 0x546e7a,
            SaiTraceTag::RISE_ENGINE => 0x5e35b1,
            SaiTraceTag::COALESCED => 0x00838f,
            SaiTraceTag::FALLBACK => 0xef6c00,
            SaiTraceTag::STALE => 0x6d4c41,
            SaiTraceTag::NORMALIZED_STORE => 0x455a64,
            SaiTraceTag::DIRECT_RPC => 0x3949ab,
            "WsDebt" => 0x7c4dff,
            "Queued" => 0x1565c0,
            "Offline" => 0xe65100,
            "Cache" => 0xf9a825,
            "NoCache" => 0xc62828,
            _ => 0x607d8b,
        };
        pill(tag.to_owned(), rgb(fill), 7., 3.)
    }

    fn transport_pill(&self) -> Div {
        let entry = &self.entry;
        let label = if entry.is_rise_engine() { "RISE" } else { entry.transport.raw_value() };
        let fill: u32 = if entry.is_rise_engine() {
            0x5e35b1
        } else {
            match entry.transport {
                SaiTraceTransport::Http => 0x1565c0,
                SaiTraceTransport::Websocket => 0x2e7d32,
            }
        };
        pill(label, rgb(fill), 7., 3.)
    }

    fn verb_or_rpc_pill(&self) -> Option<Div> {
        let entry = &self.entry;
        match entry.transport {
            SaiTraceTransport::Http => {
                Some(pill(entry.method.clone(), rgb(Self::http_verb_color(&entry.method)), 7., 3.))
            }
            SaiTraceTransport::Websocket => {
                if entry.is_rise_engine() {
                    return Some(pill(entry.method.clone(), rgb(0x4527a0), 7., 3.));
                }
                entry
                    .fetch_id
                    .as_ref()
                    .filter(|rpc| !rpc.is_empty())
                    .map(|rpc| pill(rpc.clone(), rgb(0x6a1b9a), 7., 3.))
            }
        }
    }

    fn status_cluster(&self) -> Div {
        let entry = &self.entry;
        if entry.is_rise_engine() {
            return self.rise_status_cluster();
        }
        if let Some(cache) = entry.cache_source() {
            return match cache {
                SaiTraceCacheSource::Memory => pill("CACHE", rgb(CACHE_MEMORY), 8., 4.),
                SaiTraceCacheSource::Disk => pill("DISK", rgb(CACHE_DISK), 8., 4.),
            };
        }
        if entry.is_pending {
            return pending_cluster(&self.theme);
        }
        if entry.transport_error.as_deref().is_some_and(|error| !error.is_empty()) {
            return pill("NET", rgb(STATUS_FAIL), 8., 4.);
        }
        if entry.has_status_code() {
            let fill: u32 = if entry.is_success_status() { 0x00a854 } else { 0xff7043 };
            return pill(entry.response_status.to_string(), rgb(fill), 8., 4.);
        }
        pill(EM_DASH, rgb(STATUS_NEUTRAL), 8., 4.)
    }

    fn rise_status_cluster(&self) -> Div {
        if self.entry.is_pending {
            return pending_cluster(&self.theme);
        }
        let (label, fill) = self.rise_status_descriptor();
        pill(label, rgb(fill), 8., 4.)
    }

    fn rise_status_descriptor(&self) -> (String, u32) {
        let entry = &self.entry;
        if entry.transport_error.as_deref().is_some_and(|error| !error.is_empty()) {
            return ("FAIL".to_owned(), STATUS_FAIL);
        }
        if entry.response_headers.get("x-rise-coalesced").map(String::as_str) == Some("true") {
            return ("JOIN".to_owned(), 0x00838f);
        }
        let (label, fill): (&str, u32) = match entry.rise_resolution_source() {
            Some("resource-memory") => ("MEMORY", CACHE_MEMORY),
            Some("persistent-store") => ("RISESTORE", CACHE_DISK),
            Some("network") => ("NETWORK", 0x0277bd),
            Some("network-fallback-memory" | "network-fallback-persistent-store") => ("FALLBACK", 0xef6c00),
            Some("normalized-store") => ("STORE", 0x455a64),
            Some("stale-dropped") => ("STALE", 0x6d4c41),
            Some("failure") => ("FAIL", STATUS_FAIL),
            _ => {
                let label = if entry.has_status_code() {
                    entry.response_status.to_string()
                } else {
                    EM_DASH.to_owned()
                };
                return (label, STATUS_NEUTRAL);
            }
        };
        (label.to_owned(), fill)
    }

    fn http_verb_color(method: &str) -> u32 {
        match method.to_uppercase().as_str() {
            "GET" => 0x29b6f6,
            "POST" => 0xfb8c00,
            "PUT" => 0x7e57c2,
            "PATCH" => 0x00897b,
            "DELETE" => 0xe53935,
            "HEAD" => 0x78909c,
            "OPTIONS" => 0x5c6bc0,
            _ => 0x90a4ae,
        }
    }

    fn cache_border_overlay(&self) -> Div {
        let shape = Shape::continuous(12.);
        match self.entry.cache_source() {
            Some(SaiTraceCacheSource::Memory) => centred_stroke(rgb(CACHE_MEMORY), 2.5, shape),
            Some(SaiTraceCacheSource::Disk) => centred_stroke(rgb(CACHE_DISK), 2.5, shape),
            None => centred_stroke(self.theme.border._200.opacity(0.65), 1., shape),
        }
    }
}

impl RenderOnce for SaiHttpTraceCardView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.element_id("state"), cx, |_, _| SaiHttpTraceCardState::default());
        let current = state.read(cx).clone();
        let theme = &self.theme;

        let summary = div()
            .swiftui_font(Font::system(11., FontWeight::SEMIBOLD, FontDesign::Monospaced))
            .text_color(theme.text.primary)
            .child(selectable_text(self.element_id("summary"), self.entry.summary_line()));
        let path = self.entry.rise_path_line().map(|path| {
            div()
                .swiftui_font(Font::system(10., FontWeight::BOLD, FontDesign::Monospaced))
                .text_color(rgb(PATH_TEAL))
                .child(selectable_text(self.element_id("path"), path))
        });
        let content = match current.panel {
            Panel::Overview => self.overview_content(&state, &current, window),
            Panel::Headers => self.headers_content(&state, &current, window),
        };

        let body = vstack(0.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .bg(theme.bg._200.opacity(0.92))
            .clip_shape(Shape::continuous(12.))
            .child(self.header_row())
            .child(div().w_full().h(px(1.)).bg(theme.border._200))
            .child(
                vstack(5.)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .child(summary)
                    .children(path)
                    .px(px(10.))
                    .py(px(8.)),
            )
            .child(self.panel_switcher(&state, current.panel))
            .child(content);

        div().relative().w_full().child(body).child(self.cache_border_overlay())
    }
}
