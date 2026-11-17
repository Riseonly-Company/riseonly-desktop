#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use gpui::{App, Context, Entity, FontWeight, Hsla, Subscription, Window, white};
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::{AppTheme, color};
use rise_ui::context_menu_ui::{ContextMenuItem, ContextMenuUi};
use rise_ui::swiftui::font::{Font, FontDesign, SwiftUiFont};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::text_selection::selectable_text;
use rise_ui::{IconUi, InputUi, InputUiState};

use crate::core::lib::sai::modules::sai_debugger::sai_requester_store::{
    SaiRequesterHTTPMethod, SaiRequesterMode, SaiRequesterState, SaiRequesterStore,
};

const ACCENT_BLUE: &str = "#29b6f6";
const ERROR_RED: &str = "#e11d48";
const SUCCESS_GREEN: &str = "#4caf50";

pub struct SaiRequesterPanel {
    theme: AppTheme,
    store: Entity<SaiRequesterStore>,
    ws_service: Entity<InputUiState>,
    ws_method: Entity<InputUiState>,
    ws_body_json: Entity<InputUiState>,
    http_url: Entity<InputUiState>,
    http_headers: Entity<InputUiState>,
    http_body_json: Entity<InputUiState>,
    _subscriptions: Vec<Subscription>,
}

impl SaiRequesterPanel {
    pub fn new(theme: AppTheme, cx: &mut Context<Self>) -> Self {
        let store = SaiRequesterStore::shared(cx);
        let snapshot = store.read(cx);
        let ws_service_text = snapshot.ws_service().to_owned();
        let ws_method_text = snapshot.ws_method().to_owned();
        let ws_body_text = snapshot.ws_body_json().to_owned();
        let http_url_text = snapshot.http_url().to_owned();
        let http_headers_text = snapshot.http_headers().to_owned();
        let http_body_text = snapshot.http_body_json().to_owned();

        let ws_service = Self::make_shortcut_field(&ws_service_text, "chat", 12., &theme, cx);
        let ws_method = Self::make_shortcut_field(&ws_method_text, "get_messages", 12., &theme, cx);
        let ws_body_json = Self::make_body_editor(&ws_body_text, &theme, cx);
        let http_url = Self::make_shortcut_field(&http_url_text, "https://\u{2026}", 11., &theme, cx);
        let http_headers = cx.new(|cx| {
            let mut field = InputUiState::multi_line(cx);
            Self::configure_field(&mut field, 11., &theme, cx);
            field.set_placeholder("e.g. X-Custom-Header: value", cx);
            field.set_max_visible_lines(4, cx);
            field.set_text(&http_headers_text, cx);
            field
        });
        let http_body_json = Self::make_body_editor(&http_body_text, &theme, cx);

        let mut subscriptions = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        subscriptions.extend(Self::bind(&ws_service, &store, SaiRequesterStore::ws_service, SaiRequesterStore::set_ws_service, cx));
        subscriptions.extend(Self::bind(&ws_method, &store, SaiRequesterStore::ws_method, SaiRequesterStore::set_ws_method, cx));
        subscriptions.extend(Self::bind(&ws_body_json, &store, SaiRequesterStore::ws_body_json, SaiRequesterStore::set_ws_body_json, cx));
        subscriptions.extend(Self::bind(&http_url, &store, SaiRequesterStore::http_url, SaiRequesterStore::set_http_url, cx));
        subscriptions.extend(Self::bind(&http_headers, &store, SaiRequesterStore::http_headers, SaiRequesterStore::set_http_headers, cx));
        subscriptions.extend(Self::bind(&http_body_json, &store, SaiRequesterStore::http_body_json, SaiRequesterStore::set_http_body_json, cx));

        Self {
            theme,
            store,
            ws_service,
            ws_method,
            ws_body_json,
            http_url,
            http_headers,
            http_body_json,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_theme(&mut self, theme: AppTheme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    fn configure_field(field: &mut InputUiState, font_size: f32, theme: &AppTheme, cx: &mut Context<InputUiState>) {
        field.set_chromeless(true, cx);
        field.set_font_size(Some(px(font_size)), cx);
        field.set_text_color(Some(theme.text.primary), cx);
    }

    /// shortcut text field with no autocaps and no autocorrect, only the look is kept
    fn make_shortcut_field(
        text: &str,
        placeholder: &'static str,
        font_size: f32,
        theme: &AppTheme,
        cx: &mut Context<Self>,
    ) -> Entity<InputUiState> {
        cx.new(|cx| {
            let mut field = InputUiState::single_line(cx);
            Self::configure_field(&mut field, font_size, theme, cx);
            field.set_placeholder(placeholder, cx);
            field.set_text(text, cx);
            field
        })
    }

    /// monospace scrolling body editor
    fn make_body_editor(text: &str, theme: &AppTheme, cx: &mut Context<Self>) -> Entity<InputUiState> {
        cx.new(|cx| {
            let mut field = InputUiState::multi_line(cx);
            Self::configure_field(&mut field, 11., theme, cx);
            field.set_scroll_enabled(true, cx);
            field.set_text(text, cx);
            field
        })
    }

    /// binds a field to the store, each side writes only when the other differs
    /// so the caret stays put while typing
    fn bind(
        field: &Entity<InputUiState>,
        store: &Entity<SaiRequesterStore>,
        read: fn(&SaiRequesterStore) -> &str,
        write: fn(&mut SaiRequesterStore, String, &mut Context<SaiRequesterStore>),
        cx: &mut Context<Self>,
    ) -> [Subscription; 2] {
        let store_for_field = store.clone();
        let field_for_store = field.clone();
        [
            cx.observe(field, move |_, field, cx| {
                let text = field.read(cx).text().to_owned();
                if read(store_for_field.read(cx)) != text {
                    store_for_field.update(cx, |store, cx| write(store, text, cx));
                }
            }),
            cx.observe(store, move |_, store, cx| {
                let text = read(store.read(cx)).to_owned();
                if field_for_store.read(cx).text() != text {
                    field_for_store.update(cx, |field, cx| field.set_text(&text, cx));
                }
            }),
        ]
    }

    fn header_bar(&self, state: &SaiRequesterState) -> Div {
        let theme = &self.theme;
        let store = self.store.clone();
        hstack(None::<f32>)
            .w_full()
            .child(
                div()
                    .child("Requester")
                    .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                    .text_color(theme.text.secondary),
            )
            .child(spacer_min_w(0.))
            .children((*state != SaiRequesterState::Idle).then(|| {
                plain("sai-requester-clear")
                    .on_press(move |_, cx| store.update(cx, |store, cx| store.reset(cx)))
                    .child(
                        div()
                            .child("Clear")
                            .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                            .text_color(theme.text.secondary),
                    )
            }))
            .px(px(14.))
            .py(px(8.))
            .bg(theme.bg._300.opacity(0.35))
    }

    fn mode_segment(&self, mode: SaiRequesterMode) -> Div {
        let theme = &self.theme;
        hstack(0.)
            .w_full()
            .children(SaiRequesterMode::ALL.into_iter().map(|candidate| {
                let active = mode == candidate;
                let store = self.store.clone();
                plain(candidate.raw_value())
                    .flex_1()
                    .on_press(move |_, cx| {
                        store.update(cx, |store, cx| {
                            store.set_mode(candidate, cx);
                            store.reset(cx);
                        });
                    })
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .justify_center()
                            .py(px(7.))
                            .bg(if active { theme.bg._300 } else { theme.bg._200.opacity(0.5) })
                            .swiftui_font(Font::caption().weight(if active {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            }))
                            .text_color(if active { theme.text.primary } else { theme.text.secondary })
                            .child(candidate.raw_value()),
                    )
            }))
            .clip_shape(Shape::continuous(8.))
            .stroke_border(theme.border._200, 1.)
    }

    fn ws_service_method_bar(&self) -> Div {
        hstack(8.)
            .w_full()
            .child(self.shortcut_field("Service", &self.ws_service))
            .child(self.shortcut_field("Method", &self.ws_method))
    }

    fn shortcut_field(&self, label: &'static str, field: &Entity<InputUiState>) -> Div {
        let theme = &self.theme;
        let accent = color(ACCENT_BLUE);
        vstack(3.)
            .align(HorizontalAlignment::Leading)
            .flex_1()
            .child(
                hstack(4.)
                    .child(
                        div()
                            .child(label)
                            .swiftui_font(Font::system_weight(10., FontWeight::EXTRA_BOLD))
                            .text_color(theme.text.secondary.opacity(0.65))
                            .tracking(0.4),
                    )
                    .children(IconUi::sized("arrow.triangle.2.circlepath", px(8.), accent.opacity(0.7), false)),
            )
            .child(
                div()
                    .w_full()
                    .swiftui_font(Font::system(12., FontWeight::NORMAL, FontDesign::Monospaced))
                    .text_color(theme.text.primary)
                    .px(px(9.))
                    .py(px(7.))
                    .bg(theme.bg._300.opacity(0.6))
                    .clip_shape(Shape::continuous(7.))
                    .stroke_border(accent.opacity(0.35), 1.)
                    .child(InputUi::new(field.clone())),
            )
    }

    fn ws_body_section(&self) -> Div {
        let store = self.store.clone();
        self.body_editor(
            "Body  { service \u{b7} method \u{b7} data }",
            "type / id / timestamp / metadata are auto-added on send",
            &self.ws_body_json,
            "sai-requester-ws-format",
            move |cx| store.update(cx, |store, cx| store.format_ws_body(cx)),
        )
    }

    fn http_top_bar(&self, http_method: SaiRequesterHTTPMethod) -> Div {
        let theme = &self.theme;
        let items: Vec<ContextMenuItem> = SaiRequesterHTTPMethod::ALL
            .into_iter()
            .enumerate()
            .map(|(index, method)| {
                let store = self.store.clone();
                ContextMenuItem::new(index as i64, method.raw_value(), None, method.raw_value(), move |_, cx| {
                    store.update(cx, |store, cx| store.set_http_method(method, cx));
                })
            })
            .collect();
        let label = hstack(4.)
            .w_full()
            .child(
                div()
                    .child(http_method.raw_value())
                    .swiftui_font(Font::system(12., FontWeight::SEMIBOLD, FontDesign::Monospaced))
                    .text_color(Self::http_method_color(http_method)),
            )
            .children(IconUi::sized("chevron.up.chevron.down", px(9.), theme.text.secondary, false))
            .px(px(9.))
            .py(px(7.))
            .bg(theme.bg._300.opacity(0.6))
            .clip_shape(Shape::continuous(7.))
            .stroke_border(theme.border._200, 1.);

        hstack(8.)
            .w_full()
            .child(
                vstack(3.)
                    .align(HorizontalAlignment::Leading)
                    .child(self.section_label("Method"))
                    .child(div().w(px(92.)).child(ContextMenuUi::new("sai-requester-http-method", items, label))),
            )
            .child(
                vstack(3.)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .child(self.section_label("URL"))
                    .child(self.text_box(&self.http_url)),
            )
    }

    fn text_box(&self, field: &Entity<InputUiState>) -> Div {
        let theme = &self.theme;
        div()
            .w_full()
            .swiftui_font(Font::system(11., FontWeight::NORMAL, FontDesign::Monospaced))
            .text_color(theme.text.primary)
            .px(px(9.))
            .py(px(7.))
            .bg(theme.bg._300.opacity(0.6))
            .clip_shape(Shape::continuous(7.))
            .stroke_border(theme.border._200, 1.)
            .child(InputUi::new(field.clone()))
    }

    fn http_headers_section(&self) -> Div {
        vstack(3.)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(self.section_label("Headers  (Key: Value, one per line)"))
            .child(self.text_box(&self.http_headers).min_h(px(42.)))
    }

    fn http_body_section(&self) -> Div {
        let store = self.store.clone();
        self.body_editor(
            "Body",
            "Auth header auto-attached from current session",
            &self.http_body_json,
            "sai-requester-http-format",
            move |cx| store.update(cx, |store, cx| store.format_http_body(cx)),
        )
    }

    fn body_editor(
        &self,
        label: &'static str,
        hint: &'static str,
        field: &Entity<InputUiState>,
        format_id: &'static str,
        on_format: impl Fn(&mut App) + 'static,
    ) -> Div {
        let theme = &self.theme;
        vstack(4.)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(
                hstack(None::<f32>)
                    .w_full()
                    .items_start()
                    .child(
                        vstack(1.)
                            .align(HorizontalAlignment::Leading)
                            .child(self.section_label(label))
                            .child(
                                div()
                                    .child(hint)
                                    .swiftui_font(Font::system_size(9.))
                                    .text_color(theme.text.secondary.opacity(0.5)),
                            ),
                    )
                    .child(spacer_min_w(0.))
                    .child(
                        plain(format_id).on_press(move |_, cx| on_format(cx)).child(
                            div()
                                .child("Format")
                                .swiftui_font(Font::caption2().weight(FontWeight::SEMIBOLD))
                                .text_color(color(ACCENT_BLUE)),
                        ),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(156.))
                    .p(px(8.))
                    .swiftui_font(Font::system(11., FontWeight::NORMAL, FontDesign::Monospaced))
                    .bg(theme.bg._300.opacity(0.4))
                    .clip_shape(Shape::continuous(8.))
                    .stroke_border(theme.border._200, 1.)
                    .child(InputUi::new(field.clone())),
            )
    }

    fn send_button(&self, mode: SaiRequesterMode, state: &SaiRequesterState) -> PlainButton {
        let is_ws = mode == SaiRequesterMode::Websocket;
        let is_loading = *state == SaiRequesterState::Loading;
        let store = self.store.clone();
        let background = if is_loading {
            color("#9e9e9e")
        } else if is_ws {
            color("#00db0f").opacity(0.85)
        } else {
            color(ACCENT_BLUE)
        };
        plain("sai-requester-send")
            .w_full()
            .disabled(is_loading)
            .on_press(move |_, cx| store.update(cx, |store, cx| store.send(cx)))
            .child(
                hstack(8.)
                    .w_full()
                    .justify_center()
                    .when(is_loading, |row| row.child(progress_view().tint(white()).control_size(ControlSize::Small)))
                    .when(!is_loading, |row| {
                        row.children(IconUi::sized(
                            if is_ws { "bolt.fill" } else { "paperplane.fill" },
                            px(12.),
                            white(),
                            true,
                        ))
                    })
                    .child(
                        div()
                            .child(if is_loading { "Sending\u{2026}" } else { "Send" })
                            .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                            .text_color(white()),
                    )
                    .py(px(10.))
                    .bg(background)
                    .clip_shape(Shape::continuous(9.)),
            )
    }

    fn response_section(&self, state: &SaiRequesterState) -> Option<Div> {
        match state {
            SaiRequesterState::Idle => None,
            SaiRequesterState::Loading => Some(
                hstack(8.)
                    .w_full()
                    .justify_center()
                    .py(px(20.))
                    .child(progress_view().control_size(ControlSize::Small))
                    .child(
                        div()
                            .child("Waiting for response\u{2026}")
                            .swiftui_font(Font::caption())
                            .text_color(self.theme.text.secondary),
                    ),
            ),
            SaiRequesterState::Success(body) => Some(self.response_block(body, false)),
            SaiRequesterState::Failure(message) => Some(self.response_block(message, true)),
        }
    }

    fn response_block(&self, body: &str, is_error: bool) -> Div {
        let theme = &self.theme;
        let error_red = color(ERROR_RED);
        let success_green = color(SUCCESS_GREEN);
        let copied = body.to_owned();
        let shown = if body.is_empty() { "(empty)".to_owned() } else { body.to_owned() };
        vstack(6.)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(
                hstack(None::<f32>)
                    .w_full()
                    .child(
                        div()
                            .child(if is_error { "ERROR" } else { "RESPONSE" })
                            .swiftui_font(Font::system_weight(10., FontWeight::EXTRA_BOLD))
                            .text_color(if is_error { error_red } else { success_green })
                            .tracking(0.5),
                    )
                    .child(spacer_min_w(0.))
                    .child(
                        plain("sai-requester-copy")
                            .on_press(move |_, cx| UIPasteboard::general().set_string(&copied, cx))
                            .children(IconUi::sized("doc.on.doc", px(12.), theme.text.secondary, false)),
                    ),
            )
            .child(
                div()
                    .id("sai-requester-response")
                    .w_full()
                    .max_h(px(300.))
                    .overflow_y_scroll()
                    .bg(theme.bg._300.opacity(0.4))
                    .clip_shape(Shape::continuous(8.))
                    .stroke_border(
                        if is_error { error_red.opacity(0.3) } else { success_green.opacity(0.3) },
                        1.,
                    )
                    .child(
                        div()
                            .w_full()
                            .p(px(8.))
                            .swiftui_font(Font::system(10., FontWeight::NORMAL, FontDesign::Monospaced))
                            .text_color(if is_error { error_red } else { theme.text.primary })
                            .child(selectable_text("sai-requester-response-text", shown)),
                    ),
            )
    }

    fn section_label(&self, text: &'static str) -> Div {
        div()
            .child(text)
            .swiftui_font(Font::system_weight(10., FontWeight::EXTRA_BOLD))
            .text_color(self.theme.text.secondary.opacity(0.65))
            .tracking(0.4)
    }

    fn http_method_color(method: SaiRequesterHTTPMethod) -> Hsla {
        match method {
            SaiRequesterHTTPMethod::Get => color(SUCCESS_GREEN),
            SaiRequesterHTTPMethod::Post => color(ACCENT_BLUE),
            SaiRequesterHTTPMethod::Put => color("#ff9800"),
            SaiRequesterHTTPMethod::Patch => color("#ce93d8"),
            SaiRequesterHTTPMethod::Delete => color(ERROR_RED),
        }
    }
}

impl Render for SaiRequesterPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let mode = store.mode();
        let http_method = store.http_method();
        let state = store.state().clone();

        let form = vstack(14.)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .p(px(14.))
            .child(self.mode_segment(mode))
            .when(mode == SaiRequesterMode::Websocket, |form| {
                form.child(self.ws_service_method_bar()).child(self.ws_body_section())
            })
            .when(mode != SaiRequesterMode::Websocket, |form| {
                form.child(self.http_top_bar(http_method))
                    .child(self.http_headers_section())
                    .child(self.http_body_section())
            })
            .child(self.send_button(mode, &state))
            .children(self.response_section(&state));

        vstack(0.)
            .size_full()
            .child(self.header_bar(&state))
            .child(
                div()
                    .id("sai-requester-scroll")
                    .w_full()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .bg(self.theme.bg._100)
                    .child(form),
            )
    }
}
