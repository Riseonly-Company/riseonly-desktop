use gpui::{App, ElementId, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, px, white};
use rise_canon::rise_service_text::music_service;
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::app_date_formatter::AppDateFormatter;
use rise_i18n::date_format::date_formatter::Style;
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::{ButtonAction, plain};
use rise_ui::swiftui::progress_view::progress_view;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke, pass_through};
use rise_ui::swiftui::stacks::{HorizontalAlignment, StackAlign, hstack, spacer_min_w, vstack};
use rise_ui::switch_ui::SwitchUi;

use crate::modules::music::stores::music_actions::music_types::MusicUserReference;
use crate::modules::music::widgets::music_cover_placeholder::MusicCoverPlaceholder;
use crate::modules::music::widgets::music_track_cover_image::MusicCoverImage;

const COVER_CORNER_FRACTION: f32 = 0.22;
const PENDING_BADGE_LIMIT: i64 = 99;

#[derive(IntoElement)]
pub struct PlaylistCollaborationCover {
    id: ElementId,
    source: String,
    size: f32,
}

impl PlaylistCollaborationCover {
    /// title only labels the cover for accessibility, gpui has no tree for that
    pub fn new(id: impl Into<ElementId>, source: impl Into<String>) -> Self {
        Self { id: id.into(), source: source.into(), size: 88.0 }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for PlaylistCollaborationCover {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let corner_radius = self.size * COVER_CORNER_FRACTION;
        let cover = if trimming_whitespaces_and_newlines(&self.source).is_empty() {
            MusicCoverPlaceholder::new().icon_scale(0.34).corner_radius(corner_radius).into_any_element()
        } else {
            MusicCoverImage::new((self.id.clone(), "cover"), self.source)
                .size(None::<f32>)
                .corner_radius(corner_radius)
                .fills_container(true)
                .into_any_element()
        };
        div()
            .relative()
            .size(px(self.size))
            .child(div().absolute().inset_0().child(cover))
            .child(centred_stroke(theme.border._100.opacity(0.72), 0.5, Shape::rounded_rect(corner_radius)))
    }
}

#[derive(IntoElement)]
pub struct PlaylistCollaborationErrorState {
    id: ElementId,
    message: SharedString,
    retry: Option<ButtonAction>,
}

impl PlaylistCollaborationErrorState {
    pub fn new(id: impl Into<ElementId>, message: impl Into<SharedString>, retry: Option<ButtonAction>) -> Self {
        Self { id: id.into(), message: message.into(), retry }
    }
}

impl RenderOnce for PlaylistCollaborationErrorState {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let mut column = vstack(14.0)
            .size_full().justify_center()
            .py(px(44.0))
            .children(IconUi::sized(
                "exclamationmark.arrow.triangle.2.circlepath",
                px(38.0),
                theme.semantic.error_100,
                false,
            ))
            .child(
                div()
                    .px(px(28.0))
                    .child(MainText::new(self.message).px(15.0).tac(TextAlignT::Center).color(theme.text.secondary)),
            );
        if let Some(retry) = self.retry {
            column = column.child(
                plain(self.id).on_press(move |window, cx| retry(window, cx)).child(
                    div()
                        .px(px(20.0))
                        .min_h(px(42.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme.primary._100)
                        .capsule()
                        .child(
                            MainText::new(tr("music_playlist_retry"))
                                .px(15.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.content_on(theme.primary._100)),
                        ),
                ),
            );
        }
        column
    }
}

#[derive(IntoElement, Default)]
pub struct PlaylistCollaborationLoadingState;

impl PlaylistCollaborationLoadingState {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for PlaylistCollaborationLoadingState {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        vstack(18.0)
            .size_full().justify_center()
            .py(px(44.0))
            .child(div().size(px(104.0)).rounded_rect(22.0).bg(theme.bg._200))
            .child(
                vstack(9.0)
                    .child(div().w(px(190.0)).h(px(20.0)).capsule().bg(theme.bg._200))
                    .child(div().w(px(126.0)).h(px(14.0)).capsule().bg(theme.bg._200)),
            )
            .child(div().pt(px(8.0)).child(progress_view().tint(theme.primary._100)))
    }
}

#[derive(IntoElement)]
pub struct PlaylistPendingBadge {
    count: i64,
}

impl PlaylistPendingBadge {
    pub fn new(count: i64) -> Self {
        Self { count }
    }
}

impl RenderOnce for PlaylistPendingBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.count <= 0 {
            return div().into_any_element();
        }
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let text = if self.count > PENDING_BADGE_LIMIT { "99+".to_owned() } else { self.count.to_string() };
        div()
            .px(px(7.0))
            .min_w(px(24.0))
            .min_h(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.semantic.error_100)
            .capsule()
            .child(MainText::new(text).px(11.0).font_weight(FontWeight::BOLD).color(white()))
            .into_any_element()
    }
}

#[derive(IntoElement)]
pub struct PlaylistMemberStatusBadge {
    role: String,
    status: String,
}

impl PlaylistMemberStatusBadge {
    pub fn new(role: impl Into<String>, status: impl Into<String>) -> Self {
        Self { role: role.into(), status: status.into() }
    }
}

impl RenderOnce for PlaylistMemberStatusBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let is_owner = self.role.to_lowercase() == "owner";
        let is_suspended = self.status.to_lowercase() == music_service::PLAYLIST_MEMBER_STATUS_SUSPENDED_PLAN;
        let tint = if is_suspended { theme.semantic.error_100 } else { theme.primary._100 };
        let title = if is_owner {
            tr("music_playlist_owner")
        } else if is_suspended {
            tr("music_playlist_status_suspended")
        } else {
            tr("music_playlist_member")
        };
        div()
            .px(px(9.0))
            .py(px(5.0))
            .bg(tint.opacity(0.13))
            .capsule()
            .child(MainText::new(title).px(11.0).font_weight(FontWeight::SEMIBOLD).color(tint))
    }
}

/// permission toggle row, a press anywhere on the row flips it
#[derive(IntoElement)]
pub struct PlaylistPermissionToggleRow {
    id: ElementId,
    title: SharedString,
    subtitle: SharedString,
    is_on: Binding<bool>,
    disabled: bool,
}

impl PlaylistPermissionToggleRow {
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        subtitle: impl Into<SharedString>,
        is_on: Binding<bool>,
    ) -> Self {
        Self { id: id.into(), title: title.into(), subtitle: subtitle.into(), is_on, disabled: false }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for PlaylistPermissionToggleRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let toggled = self.is_on.clone();
        plain(self.id.clone()).w_full().disabled(self.disabled).on_press(move |window, cx| toggled.toggle(window, cx)).child(
            hstack(12.0)
                .px(px(16.0))
                .py(px(11.0))
                .child(
                    vstack(3.0)
                        .align(HorizontalAlignment::Leading)
                        .child(MainText::new(self.title).px(15.0).color(theme.text.primary))
                        .child(MainText::new(self.subtitle).px(12.0).color(theme.text.secondary).number_of_lines(2)),
                )
                .child(spacer_min_w(0.0))
                .child(pass_through().child(SwitchUi::new((self.id, "switch"), self.is_on))),
        )
    }
}

pub enum PlaylistCollaborationTime {}

impl PlaylistCollaborationTime {
    pub fn short(milliseconds: i64) -> String {
        if milliseconds <= 0 {
            return String::new();
        }
        let date = Date::from_time_interval_since_1970(milliseconds as f64 / 1000.0);
        let mut formatter = AppDateFormatter::make(None, None, None);
        if Calendar::current().is_date_in_today(&date) {
            formatter.set_time_style(Style::Short);
        } else {
            formatter.set_localized_date_format_from_template("dMMM");
        }
        formatter.string_from(&date)
    }
}

pub enum PlaylistCollaborationUserLabel {}

impl PlaylistCollaborationUserLabel {
    pub fn display_name(user: &MusicUserReference) -> String {
        if !user.name.is_empty() {
            return user.name.clone();
        }
        if !user.tag.is_empty() {
            return format!("@{}", user.tag);
        }
        tr("chat_deleted_account")
    }
}
