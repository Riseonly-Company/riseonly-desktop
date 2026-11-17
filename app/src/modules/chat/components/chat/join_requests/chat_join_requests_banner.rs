use std::rc::Rc;

use gpui::{App, FontWeight, Window};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::{plural_word, tr};
use rise_platform::haptics::{ImpactStyle, impact};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::MainText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::ui::avatar_facepile_ui::{AvatarFacepileUi, AvatarFacepileUiFace};
use crate::modules::chat::components::chat::chat_group_voice_banner::ChatHeaderAccessoryMetrics;
use crate::modules::chat::stores::chats::chat_actions::chat_types::JoinRequest;

/// n requests to join row, always last in the chat header pill
/// under voice, player and pins
/// no shape of its own
#[derive(IntoElement)]
pub struct ChatJoinRequestsBanner {
    requests: Vec<JoinRequest>,
    count_label: (i64, bool),
    theme: AppTheme,
    on_open: Rc<dyn Fn(&mut Window, &mut App)>,
    on_close: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl ChatJoinRequestsBanner {
    pub fn new(
        requests: Vec<JoinRequest>,
        count_label: (i64, bool),
        theme: AppTheme,
        on_open: impl Fn(&mut Window, &mut App) + 'static,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { requests, count_label, theme, on_open: Rc::new(on_open), on_close: Rc::new(on_close) }
    }

    fn faces(&self) -> Vec<AvatarFacepileUiFace> {
        self.requests
            .iter()
            .take(3)
            .map(|request| {
                AvatarFacepileUiFace::new(
                    request.user_id.clone(),
                    (!request.user_logo.is_empty()).then(|| request.user_logo.clone()),
                    if request.user_name.is_empty() { request.user_tag.clone() } else { request.user_name.clone() },
                )
            })
            .collect()
    }
}

impl RenderOnce for ChatJoinRequestsBanner {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let faces = self.faces();
        let (count, is_exact) = self.count_label;
        let theme = self.theme;
        let on_open = self.on_open;
        let on_close = self.on_close;

        let now = RiseClock::monotonic();
        let count_motion =
            window.use_keyed_state("chat-join-requests-banner-count-motion", cx, |_, _| Animated::new(1.0_f32));
        let motion = count_motion.clone();
        on_change("chat-join-requests-banner-count", count, false, window, cx, move |_, _, _, cx| {
            motion.update(cx, |motion, cx| {
                motion.jump(0.0);
                motion.set_in(1.0, spring(0.32, 0.86), RiseClock::monotonic(), cx);
                cx.notify();
            });
        });
        let count_opacity = count_motion.read(cx).frame_value(now, window).clamp(0.0, 1.0);

        hstack(6.0)
            .w_full()
            .items_center()
            .pl(px(ChatHeaderAccessoryMetrics::LEADING_PADDING))
            .pr(px(ChatHeaderAccessoryMetrics::TRAILING_PADDING))
            .py(px(ChatHeaderAccessoryMetrics::VERTICAL_PADDING))
            .child(
                plain("chat-join-requests-banner-open")
                    .flex_1()
                    .min_w_0()
                    .on_press(move |window, cx| on_open(window, cx))
                    .child(
                        hstack(10.0)
                            .w_full()
                            .items_center()
                            .child(
                                AvatarFacepileUi::new(faces)
                                    .size(ChatHeaderAccessoryMetrics::FACEPILE_AVATAR_SIZE)
                                    .step(ChatHeaderAccessoryMetrics::FACEPILE_STEP)
                                    .ring_color(theme.bg._100),
                            )
                            .child(
                                div().min_w_0().opacity(count_opacity).child(
                                    MainText::new(ChatJoinRequestsCountText::title(count, !is_exact))
                                        .px(14.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(theme.text.primary)
                                        .number_of_lines(1),
                                ),
                            )
                            .child(spacer_min_w(8.0)),
                    ),
            )
            .child(
                plain("chat-join-requests-banner-close")
                    .flex_none()
                    .on_press(move |window, cx| {
                        let _ = impact(ImpactStyle::Light);
                        on_close(window, cx);
                    })
                    .child(
                        div()
                            .id("chat-join-requests-banner-close-label")
                            .tooltip_label(tr("close"))
                            .size(px(30.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .children(IconUi::sized("xmark", px(14.0), theme.text.secondary, false)),
                    ),
            )
    }
}

pub enum ChatJoinRequestsCountText {}

impl ChatJoinRequestsCountText {
    /// 30 requests to join, or 30+ when only a loaded page is known
    /// a page must not read as the whole queue
    pub fn title(count: i64, has_more: bool) -> String {
        let one = tr("chat_join_request_one");
        let few = tr("chat_join_request_few");
        let many = tr("chat_join_request_many");
        let word = plural_word(count, &one, &few, &many);
        format!("{count}{} {word}", if has_more { "+" } else { "" })
    }
}
