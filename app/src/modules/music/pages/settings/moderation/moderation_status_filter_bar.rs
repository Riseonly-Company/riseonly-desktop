use std::rc::Rc;

use gpui::{App, ElementId, FontWeight, SharedString, Window, transparent_black};
use rise_canon::rise_service_text::music_service;
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::swiftui::prelude::*;

const ALL_STATUSES: &str = "all";

#[derive(IntoElement)]
pub struct ModerationStatusFilterBar {
    selected: Binding<String>,
    theme: AppTheme,
    track_mode: bool,
    on_change: Rc<dyn Fn(&str, &mut App)>,
}

impl ModerationStatusFilterBar {
    pub fn new(selected: Binding<String>, theme: AppTheme, on_change: impl Fn(&str, &mut App) + 'static) -> Self {
        Self { selected, theme, track_mode: false, on_change: Rc::new(on_change) }
    }

    pub fn track_mode(mut self, track_mode: bool) -> Self {
        self.track_mode = track_mode;
        self
    }

    fn options(&self) -> [(&'static str, &'static str); 4] {
        if self.track_mode {
            return [
                (RiseMusicTrackStatus::Pending.wire(), "music_status_pending"),
                (RiseMusicTrackStatus::Published.wire(), "music_status_published"),
                (RiseMusicTrackStatus::Rejected.wire(), "music_status_rejected"),
                (ALL_STATUSES, "music_filter_all"),
            ];
        }
        [
            (music_service::ALBUM_REQUEST_STATUS_PENDING, "music_status_pending"),
            (music_service::ALBUM_REQUEST_STATUS_APPROVED, "music_status_approved"),
            (music_service::ALBUM_REQUEST_STATUS_REJECTED, "music_status_rejected"),
            (ALL_STATUSES, "music_filter_all"),
        ]
    }
}

impl RenderOnce for ModerationStatusFilterBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let selected = self.selected.get(cx);
        let theme = &self.theme;
        let mut chips = hstack(8.0).flex_none().px(px(16.0)).py(px(8.0));
        for (key, label_key) in self.options() {
            let is_selected = selected == key;
            let binding = self.selected.clone();
            let on_change = self.on_change.clone();
            chips = chips.child(
                plain(ElementId::from(SharedString::from(format!("moderation-status-filter-{key}"))))
                    .on_press(move |window, cx| {
                        if binding.get(cx) == key {
                            return;
                        }
                        binding.set(key.to_owned(), window, cx);
                        on_change(key, cx);
                    })
                    .child(
                        div()
                            .px(px(14.0))
                            .py(px(7.0))
                            .capsule()
                            .swiftui_font(Font::system_weight(
                                13.0,
                                if is_selected { FontWeight::SEMIBOLD } else { FontWeight::NORMAL },
                            ))
                            .text_color(if is_selected { theme.primary._100 } else { theme.text.secondary })
                            .bg(if is_selected { theme.primary._100.opacity(0.15) } else { theme.bg._200 })
                            .stroke_border(if is_selected { theme.primary._100 } else { transparent_black() }, 1.0)
                            .child(tr(label_key)),
                    ),
            );
        }
        div()
            .id("moderation-status-filter-bar")
            .w_full()
            .flex()
            .overflow_x_scroll()
            .restrict_scroll_to_axis()
            .child(chips)
    }
}
