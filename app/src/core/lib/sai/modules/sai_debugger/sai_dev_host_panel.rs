#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::rc::Rc;

use gpui::{App, Context, Entity, FontWeight, Hsla, SharedString, Svg, Window, rgb};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_platform::haptics::{ImpactStyle, impact};
use rise_theme::AppTheme;
use rise_ui::swiftui::font::{Font, FontDesign, SwiftUiFont};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::text_selection::selectable_text;
use rise_ui::{IconUi, InputUi, InputUiState};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::sai_dev_host_override::{
    SaiDevHostCustomEndpoint, SaiDevHostEnvironment, SaiDevHostOverride, SaiDevHostSlot,
};
use crate::core::lib::sai::core::websocket::sai_web_socket_environment::SaiWebSocketEnvironment;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::sai::Sai;
use crate::core::lib::sai::sai_auth_hooks::sai_auth_hooks;
use crate::core::stores::http::http_service::HttpService;

const ACTIVE_GREEN: u32 = 0x4ade80;
const WARNING_AMBER: u32 = 0xf59e0b;

pub enum SaiDevHostSwitcher {}

impl SaiDevHostSwitcher {
    /// applying a slot re-points the http base url and recycles the socket
    /// reconnect is the whole switch, each account is stamped with its backend
    /// switch picks that backends latest account or parks the current, nobody is signed out
    pub fn apply(slot: SaiDevHostSlot, cx: &mut App) {
        let hooks = sai_auth_hooks();
        let current = HttpService::resolved_backend_id();
        let target = Self::backend_id(slot);

        if target == current {
            Self::repoint(slot, cx);
            Self::reconnect_if_possible(cx);
            return;
        }

        let next = hooks.and_then(|hooks| (hooks.debug_first_account)(&target, cx));
        if next.is_none() {
            if let Some(hooks) = hooks {
                (hooks.debug_detach_active_account)(cx);
            }
        }

        Self::repoint(slot, cx);

        let (Some(next), Some(hooks)) = (next, hooks) else { return };
        if (hooks.active_account_id)(cx).as_deref() == Some(next.as_str()) {
            Self::reconnect_if_possible(cx);
        } else {
            (hooks.switch_account)(&next, cx);
        }
    }

    /// the profile menu lists accounts from every backend
    /// picking one issued elsewhere moves the host first so activation connects where the token is valid
    pub fn repoint_if_needed(for_account_user_id: &str, cx: &mut App) {
        if !SaiDevHostOverride::is_effective() {
            return;
        }
        let Some(hooks) = sai_auth_hooks() else { return };
        let Some(backend_id) = (hooks.load_account_backend_id)(for_account_user_id) else { return };
        if backend_id == HttpService::resolved_backend_id() {
            return;
        }
        let Some(slot) = Self::slot(&backend_id) else { return };
        Self::repoint(slot, cx);
    }

    pub fn repoint(slot: SaiDevHostSlot, cx: &mut App) {
        SaiDevHostOverride::select(slot);
        HttpService::invalidate_resolved_api_base_url();
        HttpService::shared(cx).update(cx, |service, cx| service.configure(cx));

        Sai::log(
            "DevHost",
            || {
                format!(
                    "switched to {} \u{2192} {}",
                    slot.raw_value(),
                    HttpService::resolved_api_base_url().absolute_string()
                )
            },
            SaiLogKind::Network,
            cx,
        );
    }

    pub fn backend_id(of_slot: SaiDevHostSlot) -> String {
        if of_slot.environment() == Some(SaiDevHostEnvironment::Dev) {
            return HttpService::LOCAL_BACKEND_ID.to_owned();
        }
        HttpService::canonical_backend_id(&HttpService::origin(&HttpService::debug_api_base_url(
            of_slot,
        )))
    }

    /// both local slots answer for local, the one used last wins
    /// so the address doesnt change under an account that was picked
    pub fn slot(for_backend_id: &str) -> Option<SaiDevHostSlot> {
        if for_backend_id == HttpService::LOCAL_BACKEND_ID {
            return Some(SaiDevHostOverride::last_local_slot());
        }
        SaiDevHostSlot::ALL_CASES
            .into_iter()
            .find(|slot| Self::is_reachable(*slot) && Self::backend_id(*slot) == for_backend_id)
    }

    pub fn is_reachable(slot: SaiDevHostSlot) -> bool {
        if slot != SaiDevHostSlot::Custom {
            return true;
        }
        SaiDevHostOverride::custom_endpoint().api_url().is_some()
    }

    fn reconnect_if_possible(cx: &mut App) {
        let Some(hooks) = sai_auth_hooks() else { return };
        let token = (hooks.access_token)(cx).unwrap_or_default();
        let token = trimming_whitespaces_and_newlines(&token).to_owned();
        if token.is_empty() {
            return;
        }
        SaiWebSocketManager::shared(cx)
            .update(cx, |manager, cx| manager.reconnect_with_access_token(&token, cx));
    }
}

pub type SaiDevHostRequestSwitch = Rc<dyn Fn(SaiDevHostSlot, &mut Window, &mut App)>;

pub struct SaiDevHostPanel {
    theme: AppTheme,
    revision: i64,
    on_request_switch: SaiDevHostRequestSwitch,
    custom_api_text: Entity<InputUiState>,
    custom_web_socket_text: Entity<InputUiState>,
}

impl SaiDevHostPanel {
    pub fn new(
        theme: AppTheme,
        revision: i64,
        on_request_switch: SaiDevHostRequestSwitch,
        cx: &mut Context<Self>,
    ) -> Self {
        let stored = SaiDevHostOverride::custom_endpoint();
        let custom_api_text = Self::make_field(&stored.api_text, "https://staging.riseonly.net/api", cx);
        let custom_web_socket_text =
            Self::make_field(&stored.web_socket_text, "wss://staging.riseonly.net/ws", cx);
        cx.observe(&custom_api_text, |_, _, cx| cx.notify()).detach();
        cx.observe(&custom_web_socket_text, |_, _, cx| cx.notify()).detach();
        Self { theme, revision, on_request_switch, custom_api_text, custom_web_socket_text }
    }

    pub fn set_props(&mut self, theme: AppTheme, revision: i64, cx: &mut Context<Self>) {
        self.theme = theme;
        self.revision = revision;
        cx.notify();
    }

    fn make_field(text: &str, placeholder: &'static str, cx: &mut Context<Self>) -> Entity<InputUiState> {
        let text = text.to_owned();
        cx.new(move |cx| {
            let mut field = InputUiState::single_line(cx);
            field.set_placeholder(placeholder, cx);
            field.set_text(&text, cx);
            field
        })
    }

    fn selected(&self) -> SaiDevHostSlot {
        SaiDevHostOverride::slot()
    }

    fn typed_endpoint(&self, cx: &App) -> SaiDevHostCustomEndpoint {
        SaiDevHostCustomEndpoint {
            api_text: self.custom_api_text.read(cx).text().to_owned(),
            web_socket_text: self.custom_web_socket_text.read(cx).text().to_owned(),
        }
    }

    fn notice(&self) -> Div {
        div()
            .child("Not active in this build \u{2014} the host switch only applies to the DEBUG dev environment.")
            .swiftui_font(Font::caption2())
            .text_color(rgb(WARNING_AMBER))
            .w_full()
            .p(px(10.))
            .bg(Hsla::from(rgb(WARNING_AMBER)).opacity(0.12))
            .clip_shape(Shape::continuous(10.))
    }

    fn active_card(&self, cx: &mut App) -> Div {
        let theme = &self.theme;
        let socket = SaiWebSocketManager::shared(cx).read(cx).active_url_display_string();
        vstack(6.)
            .align(HorizontalAlignment::Leading)
            .child(
                hstack(6.)
                    .child(
                        div()
                            .child("Active")
                            .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                            .text_color(theme.text.secondary),
                    )
                    .child(
                        div()
                            .child(self.selected().title())
                            .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                            .text_color(rgb(ACTIVE_GREEN)),
                    ),
            )
            .child(self.url_line("active", "API", HttpService::resolved_api_base_url().absolute_string()))
            .child(self.url_line(
                "active",
                "WS",
                SaiWebSocketEnvironment::resolved_web_socket_url().absolute_string(),
            ))
            .child(self.url_line("active", "Socket", &socket))
            .w_full()
            .p(px(10.))
            .bg(theme.bg._100)
            .clip_shape(Shape::continuous(10.))
    }

    fn slot_row(&self, slot: SaiDevHostSlot) -> impl IntoElement {
        let theme = &self.theme;
        let selected = self.selected();
        let active = selected == slot;
        let on_request_switch = self.on_request_switch.clone();

        let badge = slot.is_live().then(|| {
            div()
                .child("LIVE")
                .swiftui_font(Font::system(9., FontWeight::EXTRA_BOLD, FontDesign::Default))
                .text_color(rgb(WARNING_AMBER))
                .px(px(5.))
                .py(px(2.))
                .bg(Hsla::from(rgb(WARNING_AMBER)).opacity(0.16))
                .clip_shape(Shape::continuous(4.))
        });

        plain(SharedString::from(format!("sai-dev-host-slot-{}", slot.raw_value())))
            .on_press(move |window, cx| {
                if SaiDevHostOverride::slot() == slot {
                    return;
                }
                on_request_switch(slot, window, cx);
                let _ = impact(ImpactStyle::Light);
            })
            .disabled(!SaiDevHostOverride::is_effective())
            .child(
                hstack(10.)
                    .align(VerticalAlignment::Top)
                    .children(Self::radio(theme, active))
                    .child(
                        vstack(4.)
                            .align(HorizontalAlignment::Leading)
                            .child(
                                hstack(6.)
                                    .child(
                                        div()
                                            .child(slot.title())
                                            .swiftui_font(Font::subheadline().weight(FontWeight::SEMIBOLD))
                                            .text_color(theme.text.primary),
                                    )
                                    .children(badge),
                            )
                            .child(
                                div()
                                    .child(slot.subtitle())
                                    .swiftui_font(Font::caption2())
                                    .text_color(theme.text.secondary),
                            )
                            .child(self.url_line(
                                slot.raw_value(),
                                "API",
                                HttpService::debug_api_base_url(slot).absolute_string(),
                            ))
                            .child(self.url_line(
                                slot.raw_value(),
                                "WS",
                                SaiWebSocketEnvironment::debug_web_socket_url(slot).absolute_string(),
                            )),
                    )
                    .child(spacer_min_w(0.))
                    .p(px(10.))
                    .w_full()
                    .bg(if active { theme.bg._300 } else { theme.bg._100 })
                    .clip_shape(Shape::continuous(10.)),
            )
            .w_full()
    }

    fn custom_editor(&self, cx: &App) -> Div {
        let theme = &self.theme;
        let endpoint = self.typed_endpoint(cx);
        let active = self.selected() == SaiDevHostSlot::Custom;
        let is_effective = SaiDevHostOverride::is_effective();
        let api_url = endpoint.api_url();
        let can_apply = is_effective && api_url.is_some();

        let preview: Option<AnyElement> = match api_url.as_ref() {
            Some(api) => Some(
                vstack(6.)
                    .align(HorizontalAlignment::Leading)
                    .child(self.url_line("typed", "\u{2192} API", api.absolute_string()))
                    .child(self.url_line(
                        "typed",
                        "\u{2192} WS",
                        endpoint
                            .web_socket_url()
                            .as_ref()
                            .map(|url| url.absolute_string())
                            .unwrap_or("\u{2014}"),
                    ))
                    .into_any_element(),
            ),
            None if !trimming_whitespaces_and_newlines(&endpoint.api_text).is_empty() => Some(
                div()
                    .child("Not a URL the app can resolve.")
                    .swiftui_font(Font::caption2())
                    .text_color(rgb(WARNING_AMBER))
                    .into_any_element(),
            ),
            None => None,
        };

        let on_request_switch = self.on_request_switch.clone();
        let stored_endpoint = endpoint.clone();
        let apply_button = plain("sai-dev-host-custom-apply")
            .on_press(move |window, cx| {
                SaiDevHostOverride::store_custom_endpoint(stored_endpoint.clone());
                on_request_switch(SaiDevHostSlot::Custom, window, cx);
                let _ = impact(ImpactStyle::Light);
            })
            .disabled(!can_apply)
            .opacity(if can_apply { 1. } else { 0.45 })
            .w_full()
            .child(
                div()
                    .child(if active { "Apply" } else { "Use this host" })
                    .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                    .text_color(theme.text.primary)
                    .w_full()
                    .flex()
                    .justify_center()
                    .py(px(9.))
                    .bg(theme.bg._300)
                    .clip_shape(Shape::continuous(9.)),
            );

        vstack(8.)
            .align(HorizontalAlignment::Leading)
            .child(
                hstack(6.)
                    .children(Self::radio(theme, active))
                    .child(
                        div()
                            .child(SaiDevHostSlot::Custom.title())
                            .swiftui_font(Font::subheadline().weight(FontWeight::SEMIBOLD))
                            .text_color(theme.text.primary),
                    ),
            )
            .child(
                div()
                    .child(SaiDevHostSlot::Custom.subtitle())
                    .swiftui_font(Font::caption2())
                    .text_color(theme.text.secondary),
            )
            .child(self.field("API", &self.custom_api_text))
            .child(self.field("WS", &self.custom_web_socket_text))
            .children(preview)
            .child(apply_button)
            .p(px(10.))
            .w_full()
            .bg(if active { theme.bg._300 } else { theme.bg._100 })
            .clip_shape(Shape::continuous(10.))
    }

    fn radio(theme: &AppTheme, active: bool) -> Option<Svg> {
        let symbol = if active { "largecircle.fill.circle" } else { "circle" };
        let color = if active { Hsla::from(rgb(ACTIVE_GREEN)) } else { theme.text.secondary };
        IconUi::sized(symbol, px(17.), color, false)
    }

    /// url style text field with no autocaps and no autocorrect
    /// the desktop field does neither so only the look is kept
    fn field(&self, label: &'static str, text: &Entity<InputUiState>) -> Div {
        let theme = &self.theme;
        hstack(6.)
            .child(
                div()
                    .child(label)
                    .swiftui_font(Font::caption2().weight(FontWeight::SEMIBOLD))
                    .text_color(theme.text.secondary)
                    .w(px(42.))
                    .flex_none(),
            )
            .child(
                div()
                    .flex_1()
                    .swiftui_font(Font::caption2().monospaced())
                    .text_color(theme.text.primary)
                    .px(px(8.))
                    .py(px(7.))
                    .bg(theme.bg._200)
                    .clip_shape(Shape::continuous(8.))
                    .child(InputUi::new(text.clone())),
            )
    }

    fn url_line(&self, scope: &str, label: &'static str, value: &str) -> Div {
        let theme = &self.theme;
        hstack(6.)
            .align(VerticalAlignment::FirstTextBaseline)
            .child(
                div()
                    .child(label)
                    .swiftui_font(Font::caption2().weight(FontWeight::SEMIBOLD))
                    .text_color(theme.text.secondary)
                    .w(px(42.))
                    .flex_none(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .swiftui_font(Font::caption2().monospaced())
                    .text_color(theme.text.primary)
                    .line_clamp(2)
                    .child(selectable_text(
                        SharedString::from(format!("sai-dev-host-url-{scope}-{label}")),
                        value.to_owned(),
                    )),
            )
    }
}

impl Render for SaiDevHostPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme.clone();
        let notice = (!SaiDevHostOverride::is_effective()).then(|| self.notice());
        let active_card = self.active_card(cx);
        let rows: Vec<AnyElement> = SaiDevHostSlot::ALL_CASES
            .into_iter()
            .filter(|slot| *slot != SaiDevHostSlot::Custom)
            .map(|slot| self.slot_row(slot).into_any_element())
            .collect();
        let custom_editor = self.custom_editor(cx);

        div().id("sai-dev-host-panel").size_full().overflow_y_scroll().child(
            vstack(14.)
                .align(HorizontalAlignment::Leading)
                .children(notice)
                .child(active_card)
                .child(
                    vstack(8.)
                        .align(HorizontalAlignment::Leading)
                        .w_full()
                        .child(
                            div()
                                .child("Hosts")
                                .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                                .text_color(theme.text.secondary),
                        )
                        .children(rows),
                )
                .child(custom_editor)
                .child(
                    div()
                        .child("Switching applies immediately: HTTP is re-pointed, the socket reconnects, and the choice survives a relaunch. Accounts remember the backend that issued them, so a switch activates the most recent account of that backend, or opens sign-in when there is none while keeping the current session parked for the way back. Picking an account from another backend in the profile menu moves the host with it. Deep links, share URLs and StoreKit still follow the compiled APP_ENV.")
                        .swiftui_font(Font::caption2())
                        .text_color(theme.text.secondary),
                )
                .p(px(12.)),
        )
    }
}
