use std::rc::Rc;

use gpui::{App, FontWeight, SharedString, Window};
use rise_i18n::{plural_word, tr};
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::MainText;
use rise_ui::swiftui::button_style::{ButtonAction, plain};
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::avatar_facepile_ui::{AvatarFacepileUi, AvatarFacepileUiFace};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType, VoiceChannelState};

/// when a group voice chat shows in the header
/// own type since the same rule drives the profile tile, voice chat is for plain groups and not forums
pub enum ChatGroupVoiceBannerPolicy {}

impl ChatGroupVoiceBannerPolicy {
    pub fn is_visible(chat: &ChatInfo, topic_id: Option<&str>, participant_count: i64) -> bool {
        chat.r#type == ChatType::Group
            && !chat.topics_enabled
            && topic_id.unwrap_or("").is_empty()
            && !chat.is_community_preview()
            && participant_count > 0
    }
}

/// geometry of the rows sharing the chat header card
/// voice chat, mini player and pinned message. one source since separate numbers drifted apart
pub enum ChatHeaderAccessoryMetrics {}

impl ChatHeaderAccessoryMetrics {
    pub const LEADING_PADDING: f32 = 12.0;
    pub const TRAILING_PADDING: f32 = 10.0;
    pub const VERTICAL_PADDING: f32 = 8.5;

    pub const FACEPILE_AVATAR_SIZE: f32 = 28.0;
    pub const FACEPILE_STEP: f32 = 18.0;

    pub const ACTION_HEIGHT: f32 = 30.0;
    pub const ACTION_HORIZONTAL_PADDING: f32 = 18.0;

    pub const CARD_CORNER_RADIUS: f32 = 26.0;
}

/// n people in voice row with avatars and a join button
/// first in the header card since an active voice chat is the most urgent
/// but a row and not its own shape
#[derive(IntoElement)]
pub struct ChatGroupVoiceBanner {
    state: VoiceChannelState,
    on_join: ButtonAction,
}

impl ChatGroupVoiceBanner {
    pub fn new(state: VoiceChannelState, on_join: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self {
            state,
            on_join: Rc::new(on_join),
        }
    }

    fn faces(&self) -> Vec<AvatarFacepileUiFace> {
        self.state
            .participants
            .iter()
            .take(3)
            .map(|participant| {
                AvatarFacepileUiFace::new(
                    participant.user_id.clone(),
                    (!participant.avatar_url.is_empty()).then(|| participant.avatar_url.clone()),
                    participant.name.clone(),
                )
            })
            .collect()
    }

    fn subtitle(&self) -> String {
        let count = self.state.participant_count.max(self.state.participants.len() as i64);
        let one = tr("chat_count_member_one");
        let few = tr("chat_count_member_few");
        let many = tr("chat_count_member_many");
        format!("{} {}", RiseFormat::compact_count(count), plural_word(count, &one, &few, &many))
    }
}

impl RenderOnce for ChatGroupVoiceBanner {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let subtitle = self.subtitle();
        let faces = self.faces();
        let on_join = self.on_join;
        let spacer = || div().flex_1().min_w(px(8.0));

        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .pl(px(ChatHeaderAccessoryMetrics::LEADING_PADDING))
            .pr(px(ChatHeaderAccessoryMetrics::TRAILING_PADDING))
            .py(px(ChatHeaderAccessoryMetrics::VERTICAL_PADDING))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(1.0))
                    .child(
                        MainText::new(tr("rtc_voice_chat"))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .number_of_lines(1),
                    )
                    .child(MainText::new(subtitle).px(11.0).color(theme.text.secondary).number_of_lines(1)),
            )
            .child(spacer())
            .child(
                AvatarFacepileUi::new(faces)
                    .size(ChatHeaderAccessoryMetrics::FACEPILE_AVATAR_SIZE)
                    .step(ChatHeaderAccessoryMetrics::FACEPILE_STEP)
                    .ring_color(theme.bg._100),
            )
            .child(spacer())
            .child(
                plain(SharedString::from(format!("chat-group-voice-banner-join-{}", self.state.room_id)))
                    .on_press(move |window, cx| on_join(window, cx))
                    .child(
                        div()
                            .h(px(ChatHeaderAccessoryMetrics::ACTION_HEIGHT))
                            .px(px(ChatHeaderAccessoryMetrics::ACTION_HORIZONTAL_PADDING))
                            .flex()
                            .items_center()
                            .rounded_full()
                            .bg(theme.primary._100)
                            .child(
                                MainText::new(tr("join_action_join"))
                                    .px(14.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.content_on(theme.primary._100))
                                    .number_of_lines(1),
                            ),
                    ),
            )
    }
}
