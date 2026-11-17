use std::sync::Arc;

use gpui::{App, ElementId, FontWeight, Hsla, SharedString, Window};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::swiftui::minimum_scale_factor::MinimumScaleText;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::music::stores::music_actions::music_types::MusicPlaylistListener;

/// the playlist whose live listeners a track row inside this subtree shows
pub struct MusicPlaylistLiveScopeIdKey;

impl EnvKey for MusicPlaylistLiveScopeIdKey {
    type Value = Option<String>;

    fn default_value() -> Self::Value {
        None
    }
}

/// stack of avatars of the people listening to a playlist, viewer left out
#[derive(IntoElement)]
pub struct MusicPlaylistListenerCluster {
    id: ElementId,
    listeners: Vec<MusicPlaylistListener>,
    total_count: i64,
    size: f32,
    is_loading: bool,
}

impl MusicPlaylistListenerCluster {
    pub fn new(id: impl Into<ElementId>, listeners: Vec<MusicPlaylistListener>, total_count: i64) -> Self {
        Self { id: id.into(), listeners, total_count, size: 34., is_loading: false }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn is_loading(mut self, is_loading: bool) -> Self {
        self.is_loading = is_loading;
        self
    }

    fn filtered_listeners(&self, current_user_id: Option<&str>) -> Vec<MusicPlaylistListener> {
        self.listeners.iter().filter(|listener| Some(listener.user_id.as_str()) != current_user_id).cloned().collect()
    }

    fn resolved_total_count(&self, filtered_listeners: &[MusicPlaylistListener], current_user_id: Option<&str>) -> i64 {
        let includes_current_user = self.listeners.iter().any(|listener| Some(listener.user_id.as_str()) == current_user_id);
        let external_total = 0_i64.max(self.total_count - i64::from(includes_current_user));
        external_total.max(filtered_listeners.len() as i64)
    }

    fn visible_cell_count(filtered_listeners: &[MusicPlaylistListener], resolved_total_count: i64) -> usize {
        if resolved_total_count <= 0 {
            return 0;
        }
        let wanted = if resolved_total_count > 3 { 3 } else { resolved_total_count as usize };
        3_usize.min(filtered_listeners.len().max(wanted))
    }

    #[allow(clippy::too_many_arguments)]
    fn cell(
        &self,
        index: usize,
        cell_size: f32,
        ring: Hsla,
        filtered_listeners: &[MusicPlaylistListener],
        resolved_total_count: i64,
        visible_cell_count: usize,
        theme: &AppTheme,
    ) -> AnyElement {
        if resolved_total_count > 3 && index == 2 {
            return zstack(Alignment::Center)
                .relative()
                .size_full()
                .child(div().size_full().circle().bg(theme.primary._100))
                .child(
                    div()
                        .swiftui_font(Font::caption2().weight(FontWeight::BOLD))
                        .text_color(theme.content_on(theme.primary._100))
                        .child(MinimumScaleText::new(format!("+{}", 1_i64.max(resolved_total_count - 2)), 0.6).line_limit(1)),
                )
                .child(centred_stroke(ring, 1.5, Shape::Circle))
                .into_any_element();
        }
        if let Some(listener) = filtered_listeners.get(index) {
            let logo_id = ElementId::NamedChild(
                Arc::new(self.id.clone()),
                SharedString::from(format!("logo-{}", listener.user_id)),
            );
            let line_width = if index == 0 && visible_cell_count == 1 { 1. } else { 1.5 };
            return div()
                .relative()
                .size_full()
                .child(
                    div().size_full().circle().overflow_hidden().child(
                        UserLogoUi::new(logo_id)
                            .source(SharedString::from(listener.user.logo.clone()))
                            .name(SharedString::from(listener.user.name.clone()))
                            .user_id(SharedString::from(listener.user_id.clone()))
                            .size(cell_size)
                            .show_border(false)
                            .show_story_ring(false),
                    ),
                )
                .child(centred_stroke(ring, line_width, Shape::Circle))
                .into_any_element();
        }
        div()
            .relative()
            .size_full()
            .circle()
            .bg(theme.bg._300)
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized("person.fill", px(Font::caption2().size), theme.text.secondary, true))
            .child(centred_stroke(ring, 1.5, Shape::Circle))
            .into_any_element()
    }
}

impl RenderOnce for MusicPlaylistListenerCluster {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let ring = theme.bg._100;
        let size = self.size;
        let current_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let current_user_id = current_user_id.as_deref();
        let filtered_listeners = self.filtered_listeners(current_user_id);
        let resolved_total_count = self.resolved_total_count(&filtered_listeners, current_user_id);
        let visible_cell_count = Self::visible_cell_count(&filtered_listeners, resolved_total_count);

        let mut stack = zstack(Alignment::Center).flex_none().size(px(size));
        match visible_cell_count {
            0 => {
                let glyph: AnyElement = if self.is_loading {
                    progress_view().control_size(ControlSize::Small).tint(theme.text.secondary).into_any_element()
                } else {
                    div()
                        .children(IconUi::sized("person.2.fill", px(size * 0.34), theme.text.secondary, true))
                        .into_any_element()
                };
                stack = stack.child(
                    div()
                        .relative()
                        .size_full()
                        .circle()
                        .bg(theme.bg._300)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(glyph)
                        .child(centred_stroke(ring, 1., Shape::Circle)),
                );
            }
            1 => {
                stack = stack.child(div().flex_none().size(px(size)).child(self.cell(
                    0,
                    size,
                    ring,
                    &filtered_listeners,
                    resolved_total_count,
                    visible_cell_count,
                    &theme,
                )));
            }
            2 => {
                let cell_size = size * 0.68;
                for (index, alignment) in [(0, Alignment::BottomLeading), (1, Alignment::TopTrailing)] {
                    stack = stack.child(
                        div()
                            .flex_none()
                            .size(px(cell_size))
                            .child(self.cell(
                                index,
                                cell_size,
                                ring,
                                &filtered_listeners,
                                resolved_total_count,
                                visible_cell_count,
                                &theme,
                            ))
                            .frame_max(alignment),
                    );
                }
            }
            _ => {
                let cell_size = size * 0.6;
                for (index, alignment) in
                    [(0, Alignment::Top), (1, Alignment::BottomLeading), (2, Alignment::BottomTrailing)]
                {
                    stack = stack.child(
                        div()
                            .flex_none()
                            .size(px(cell_size))
                            .child(self.cell(
                                index,
                                cell_size,
                                ring,
                                &filtered_listeners,
                                resolved_total_count,
                                visible_cell_count,
                                &theme,
                            ))
                            .frame_max(alignment),
                    );
                }
            }
        }
        stack
    }
}
