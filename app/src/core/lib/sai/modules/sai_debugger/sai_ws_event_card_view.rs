use std::rc::Rc;

use gpui::{App, ElementId, Entity, FontWeight, Hsla, SharedString, Window, rgb, white};
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

use super::sai_debug_storage_panel_views::{SaiDebugBodyFormat, SaiDebugInspectorMetrics};
use super::sai_ws_event_store::SaiWsEventEntry;

const TIME_FORMAT: &str = "HH:mm:ss.SSS";

const HANDLED_BORDER: u32 = 0x2e7d32;
const NO_HANDLER_BORDER: u32 = 0xc62828;
const DESTRUCTIVE_RED: u32 = 0xe11d48;
const EVENT_PILL: u32 = 0x6a1b9a;
const HANDLED_PILL: u32 = 0x00a854;
const NO_HANDLER_PILL: u32 = 0xff7043;
const LINK_BLUE: u32 = 0x29b6f6;

thread_local! {
    static TIME_FORMATTER: DateFormatter = {
        let mut formatter = DateFormatter::new();
        formatter.set_date_format(TIME_FORMAT);
        formatter
    };
}

fn color(hex: u32) -> Hsla {
    Hsla::from(rgb(hex))
}

#[derive(Clone)]
struct SaiWsEventCardExpandState {
    expanded: bool,
    open: Presence<()>,
    closed: Presence<()>,
}

impl Default for SaiWsEventCardExpandState {
    fn default() -> Self {
        Self { expanded: false, open: Presence::new(None), closed: Presence::new(Some(())) }
    }
}

impl SaiWsEventCardExpandState {
    fn toggle(&mut self, cx: &App) {
        self.expanded = !self.expanded;
        let now = RiseClock::monotonic();
        self.open.set_in(self.expanded.then_some(()), ease_in_out(0.18), now, cx);
        self.closed.set_in((!self.expanded).then_some(()), ease_in_out(0.18), now, cx);
    }
}

#[derive(IntoElement)]
pub struct SaiWsEventCardView {
    entry: SaiWsEventEntry,
    theme: AppTheme,
    on_delete: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl SaiWsEventCardView {
    pub fn new(entry: SaiWsEventEntry, theme: AppTheme) -> Self {
        Self { entry, theme, on_delete: None }
    }

    /// called when the card is deleted
    pub fn on_delete(mut self, on_delete: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_delete = Some(Rc::new(on_delete));
        self
    }

    fn element_id(&self, part: &str) -> ElementId {
        ElementId::Name(SharedString::from(format!("sai-ws-event-{}-{}", self.entry.id, part)))
    }

    fn icon_button_label(&self, symbol: &'static str, tint: Hsla) -> Div {
        div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(36.))
            .h(px(32.))
            .bg(self.theme.bg._300.opacity(0.9))
            .clip_shape(Shape::continuous(8.))
            .children(IconUi::sized(symbol, px(15.), tint, false))
    }

    fn header_row(&self) -> Div {
        let theme = &self.theme;
        let time = TIME_FORMATTER.with(|formatter| formatter.string_from(&self.entry.created_at));
        let delete_button = self.on_delete.clone().map(|on_delete| {
            plain(self.element_id("delete"))
                .on_press(move |window, cx| on_delete(window, cx))
                .child(self.icon_button_label("trash", color(DESTRUCTIVE_RED)))
        });
        let copy_entry = self.entry.clone();

        hstack(6.)
            .align(VerticalAlignment::Top)
            .w_full()
            .child(
                hstack(6.)
                    .flex_1()
                    .min_w_0()
                    .child(self.event_pill())
                    .child(self.handled_pill()),
            )
            .child(
                div()
                    .child(time)
                    .swiftui_font(Font::caption2().weight(FontWeight::MEDIUM).monospaced_digit())
                    .text_color(theme.text.secondary),
            )
            .children(delete_button)
            .child(
                plain(self.element_id("copy"))
                    .on_press(move |_window, cx| {
                        UIPasteboard::general().set_string(&copy_entry.copy_block_text(), cx);
                    })
                    .child(self.icon_button_label("doc.on.doc", theme.text.primary)),
            )
            .px(px(10.))
            .py(px(8.))
            .bg(theme.bg._300.opacity(0.45))
    }

    fn event_pill(&self) -> Div {
        div()
            .child(self.entry.r#type.clone())
            .swiftui_font(Font::caption2().weight(FontWeight::EXTRA_BOLD))
            .text_color(white())
            .line_clamp(1)
            .px(px(7.))
            .py(px(3.))
            .bg(rgb(EVENT_PILL))
            .capsule()
    }

    fn handled_pill(&self) -> Div {
        let (title, fill) =
            if self.entry.handled { ("HANDLED", HANDLED_PILL) } else { ("NO HANDLER", NO_HANDLER_PILL) };
        div()
            .child(title)
            .swiftui_font(Font::caption2().weight(FontWeight::EXTRA_BOLD))
            .text_color(white())
            .px(px(7.))
            .py(px(3.))
            .bg(rgb(fill))
            .capsule()
    }

    fn payload_section(
        &self,
        state: &Entity<SaiWsEventCardExpandState>,
        current: &SaiWsEventCardExpandState,
        window: &mut Window,
    ) -> Div {
        let theme = &self.theme;
        let now = RiseClock::monotonic();
        let toggle_state = state.clone();
        let opened = current.open.render_transitioned(now, window, |_| {
            div()
                .id(self.element_id("payload-scroll"))
                .w_full()
                .max_h(px(SaiDebugInspectorMetrics::JSON_BODY_SCROLL_MAX_HEIGHT))
                .overflow_y_scroll()
                .child(
                    div()
                        .w_full()
                        .swiftui_font(Font::system(11., FontWeight::NORMAL, FontDesign::Monospaced))
                        .text_color(theme.text.primary)
                        .child(selectable_text(self.element_id("payload-text"), self.entry.payload_json.clone())),
                )
        });
        let closed = current.closed.render_transitioned(now, window, |_| {
            div()
                .child(SaiDebugBodyFormat::size_label_char_count(character_count(&self.entry.payload_json) as i64))
                .swiftui_font(Font::system(11., FontWeight::NORMAL, FontDesign::Monospaced))
                .text_color(theme.text.secondary.opacity(0.85))
        });

        vstack(6.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(
                hstack(8.)
                    .align(VerticalAlignment::FirstTextBaseline)
                    .w_full()
                    .child(
                        div()
                            .child("PAYLOAD")
                            .swiftui_font(Font::system(10., FontWeight::EXTRA_BOLD, FontDesign::Default))
                            .text_color(theme.text.secondary.opacity(0.85))
                            .tracking(0.5),
                    )
                    .child(spacer_min_w(0.))
                    .child(
                        plain(self.element_id("toggle"))
                            .on_press(move |_window, cx| {
                                toggle_state.update(cx, |state, cx| {
                                    state.toggle(cx);
                                    cx.notify();
                                });
                            })
                            .child(
                                div()
                                    .child(if current.expanded { "Hide" } else { "Show" })
                                    .swiftui_font(Font::caption2().weight(FontWeight::SEMIBOLD))
                                    .text_color(rgb(LINK_BLUE)),
                            ),
                    ),
            )
            .children(opened)
            .children(closed)
            .p(px(10.))
            .bg(theme.bg._100)
    }
}

impl RenderOnce for SaiWsEventCardView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.element_id("expand"), cx, |_, _| {
            SaiWsEventCardExpandState::default()
        });
        let current = state.read(cx).clone();
        let border = if self.entry.handled {
            color(HANDLED_BORDER).opacity(0.65)
        } else {
            color(NO_HANDLER_BORDER).opacity(0.7)
        };
        let theme = &self.theme;

        div()
            .relative()
            .w_full()
            .child(
                vstack(0.)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .child(self.header_row())
                    .child(div().w_full().h(px(1.)).bg(theme.border._200))
                    .child(self.payload_section(&state, &current, window))
                    .bg(theme.bg._200.opacity(0.92))
                    .clip_shape(Shape::continuous(12.)),
            )
            .child(centred_stroke(border, 1., Shape::continuous(12.)))
    }
}
