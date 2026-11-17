use gpui::prelude::*;
use gpui::{
    App, Div, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, MouseButton, SharedString, Stateful, Window,
    WindowControlArea, div, hsla, px,
};
use rise_i18n::tr;
use rise_platform::HostOs;
use rise_platform::call_window::{IncomingCallResponse, incoming_window_takes_focus};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::button_style::button_chrome;
use rise_ui::swiftui::stacks::{hstack, vstack};
use rise_ui::swiftui::text::TruncationMode;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::{IconUi, theme};

use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::rtc::system::rise_rtc_call_system_coordinator::RiseRTCCallSystemCoordinator;

const AVATAR_SIZE: f32 = 56.0;
const BUTTON_SIZE: f32 = 44.0;
const BUTTON_ICON_SIZE: f32 = 18.0;
const WINDOW_PADDING: f32 = 14.0;
const NAME_FONT_SIZE: f32 = 15.0;
const STATUS_FONT_SIZE: f32 = 13.0;
const STATUS_ICON_SIZE: f32 = 13.0;

/// the call screen is dark in both themes, this small window follows it
fn window_fill() -> Hsla {
    hsla(0.0, 0.0, 0.11, 1.0)
}

pub struct RiseRTCIncomingCallWindow {
    server_call_id: String,
    display_name: String,
    avatar_url: Option<String>,
    remote_user_id: Option<String>,
    has_video: bool,
    focus_handle: FocusHandle,
}

impl RiseRTCIncomingCallWindow {
    pub fn build(
        server_call_id: String,
        display_name: String,
        avatar_url: Option<String>,
        remote_user_id: Option<String>,
        has_video: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| {
            let focus_handle = cx.focus_handle();
            if incoming_window_takes_focus(HostOs::current()) {
                window.focus(&focus_handle, cx);
            }
            Self { server_call_id, display_name, avatar_url, remote_user_id, has_video, focus_handle }
        })
    }

    pub fn set_caller(
        &mut self,
        display_name: String,
        avatar_url: Option<String>,
        remote_user_id: Option<String>,
        has_video: bool,
        cx: &mut Context<Self>,
    ) {
        self.display_name = display_name;
        self.avatar_url = avatar_url;
        self.remote_user_id = remote_user_id;
        self.has_video = has_video;
        cx.notify();
    }

    fn respond(server_call_id: String, response: IncomingCallResponse, cx: &mut App) {
        cx.defer(move |cx| {
            RiseRTCCallSystemCoordinator::shared(cx).update(cx, |coordinator, cx| {
                coordinator.respond_to_incoming_call(server_call_id, response, cx);
            });
        });
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) || event.keystroke.modifiers.modified() {
            return;
        }
        let Some(response) = IncomingCallResponse::from_key(&event.keystroke.key) else {
            return;
        };
        cx.stop_propagation();
        Self::respond(self.server_call_id.clone(), response, cx);
    }

    fn response_button(
        &self,
        id: &'static str,
        icon: &'static str,
        title: String,
        fill: Hsla,
        response: IncomingCallResponse,
        cx: &App,
    ) -> Stateful<Div> {
        let server_call_id = self.server_call_id.clone();
        button_chrome(div().id(id).occlude(), false, None)
            .flex_none()
            .size(px(BUTTON_SIZE))
            .rounded_full()
            .bg(fill)
            .flex()
            .items_center()
            .justify_center()
            .tooltip_label(title)
            .children(IconUi::sized(icon, px(BUTTON_ICON_SIZE), theme(cx).content_on(fill), true))
            .on_click(move |_, _, cx| Self::respond(server_call_id.clone(), response, cx))
    }
}

impl Render for RiseRTCIncomingCallWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text_color = theme(cx).content_on(window_fill());
        let semantic = &theme(cx).semantic;
        let decline_fill = semantic.error_100;
        let answer_fill = semantic.success_100;
        div()
            .id("rtc-incoming-call-window")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .size_full()
            .bg(window_fill())
            .child(
                hstack(12.0)
                    .size_full()
                    .p(px(WINDOW_PADDING))
                    .child(
                        div().flex_none().size(px(AVATAR_SIZE)).child(
                            UserLogoUi::new("rtc-incoming-call-logo")
                                .source(self.avatar_url.clone().map(SharedString::from))
                                .name(Some(SharedString::from(self.display_name.clone())))
                                .user_id(self.remote_user_id.clone().map(SharedString::from))
                                .size(AVATAR_SIZE)
                                .show_border(false)
                                .show_story_ring(false)
                                .image_cache_policy(CleverImageCachePolicy::MemoryOnly),
                        ),
                    )
                    .child(
                        vstack(2.0)
                            .flex_1()
                            .min_w_0()
                            .items_start()
                            .child(
                                MainText::new(self.display_name.clone())
                                    .px(NAME_FONT_SIZE)
                                    .font_weight(FontWeight::MEDIUM)
                                    .color(text_color)
                                    .number_of_lines(1)
                                    .truncation(TruncationMode::Tail),
                            )
                            .child(
                                hstack(4.0)
                                    .min_w_0()
                                    .items_center()
                                    .when(self.has_video, |row| {
                                        row.children(IconUi::sized("video.fill", px(STATUS_ICON_SIZE), text_color, true))
                                    })
                                    .child(
                                        MainText::new(tr("rtc_call_incoming"))
                                            .px(STATUS_FONT_SIZE)
                                            .color(text_color)
                                            .number_of_lines(1)
                                            .truncation(TruncationMode::Tail),
                                    ),
                            ),
                    )
                    .child(self.response_button(
                        "rtc-incoming-call-decline",
                        "phone.down.fill",
                        tr("rtc_decline_call"),
                        decline_fill,
                        IncomingCallResponse::Decline,
                        cx,
                    ))
                    .child(self.response_button(
                        "rtc-incoming-call-answer",
                        if self.has_video { "video.fill" } else { "phone.fill" },
                        tr("rtc_answer_call"),
                        answer_fill,
                        IncomingCallResponse::Answer,
                        cx,
                    )),
            )
    }
}
