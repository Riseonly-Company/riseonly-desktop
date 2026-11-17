use gpui::{App, Entity, FontWeight, Hsla, SharedString, Window};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::i18n::tr_args;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::font::{Font, FontDesign, SwiftUiFont};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::TransitionModifiers;

use crate::modules::chat::components::chat_item::chat_list_preview_row::ChatListPreviewFactory;
use crate::modules::chat::pages::chats::chats::ChatsScreenState;
use crate::modules::chat::pages::chats::chats_archive_reveal::{ChatsArchiveRevealPhase, ChatsArchiveRevealPolicy};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;

const ICON_SIZE: f32 = 50.0;
const ICON_GLYPH_SIZE: f32 = 21.0;

#[derive(IntoElement)]
pub struct ChatsArchiveRevealRow {
    state: Entity<ChatsScreenState>,
    pinned: bool,
    chats: Vec<ChatInfo>,
    total_count: i64,
    unread_chats_count: i64,
    action: ButtonAction,
}

impl ChatsArchiveRevealRow {
    pub fn new(
        state: Entity<ChatsScreenState>,
        pinned: bool,
        chats: Vec<ChatInfo>,
        total_count: i64,
        unread_chats_count: i64,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { state, pinned, chats, total_count, unread_chats_count, action: std::rc::Rc::new(action) }
    }
}

impl RenderOnce for ChatsArchiveRevealRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = self.state.read(cx);
        let phase = if self.pinned { ChatsArchiveRevealPhase::Revealed } else { state.archive_reveal_phase };
        let marker_y = if phase == ChatsArchiveRevealPhase::Pulling { state.archive_marker_y } else { 0.0 };
        let progress = ChatsArchiveRevealPolicy::visibility_progress(phase, marker_y);
        div()
            .w_full()
            .h(px(ChatsArchiveRevealPolicy::ROW_HEIGHT))
            .opacity(progress)
            .child(ChatsArchiveSummaryRow {
                chats: self.chats,
                total_count: self.total_count,
                unread_chats_count: self.unread_chats_count,
                is_enabled: phase == ChatsArchiveRevealPhase::Revealed,
                action: self.action,
            })
            .scale_effect(0.98 + 0.02 * progress)
    }
}

#[derive(IntoElement)]
pub struct ChatsArchiveSummaryRow {
    pub chats: Vec<ChatInfo>,
    pub total_count: i64,
    pub unread_chats_count: i64,
    pub is_enabled: bool,
    pub action: ButtonAction,
}

impl ChatsArchiveSummaryRow {
    pub fn subtitle(&self, cx: &mut App) -> String {
        let titles = self
            .chats
            .iter()
            .take(3)
            .map(|chat| {
                let title = ChatListPreviewFactory::chat_title(chat, cx);
                let title = trimming_whitespaces_and_newlines(&title);
                if title.is_empty() { tr("chat_untitled") } else { title.to_string() }
            })
            .collect::<Vec<_>>();
        if !titles.is_empty() {
            return titles.join(", ");
        }
        if self.total_count == 0 {
            return tr("chat_archive_empty");
        }
        tr_args("chat_archive_count", &[self.total_count.into()])
    }

    fn archive_icon(theme: &AppTheme) -> Div {
        div()
            .flex_none()
            .w(px(ICON_SIZE))
            .h(px(ICON_SIZE))
            .circle()
            .bg(theme.input.bg_100)
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized("archivebox.fill", px(ICON_GLYPH_SIZE), theme.text.primary, true))
    }

    fn unread_badge(count: i64, background: Hsla) -> Div {
        let text = if count > 99 { "99+".to_owned() } else { count.to_string() };
        div()
            .flex_none()
            .min_w(px(25.0))
            .min_h(px(22.0))
            .px(px(7.0))
            .capsule()
            .bg(background)
            .flex()
            .items_center()
            .justify_center()
            .swiftui_font(Font::system(12.0, FontWeight::SEMIBOLD, FontDesign::Rounded))
            .text_color(gpui::white())
            .child(SharedString::from(text))
    }
}

impl RenderOnce for ChatsArchiveSummaryRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let subtitle = self.subtitle(cx);
        let badge = (self.unread_chats_count > 0)
            .then(|| Self::unread_badge(self.unread_chats_count, theme.text.secondary.opacity(0.46)));
        button_chrome(div().id("chats-archive-summary-row"), !self.is_enabled, Some(self.action))
            .w_full()
            .min_h(px(ChatsArchiveRevealPolicy::ROW_HEIGHT))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(Self::archive_icon(&theme))
            .child(
                vstack(3.0)
                    .flex_1()
                    .min_w_0()
                    .items_start()
                    .child(
                        MainText::new(tr("chat_archive_title"))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .number_of_lines(1),
                    )
                    .child(MainText::new(subtitle).px(13.0).color(theme.text.secondary).number_of_lines(1)),
            )
            .children(badge)
    }
}
