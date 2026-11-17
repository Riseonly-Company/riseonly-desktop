use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, RenderOnce, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, black, div, px,
    white,
};
use rise_canon::rise_service_text::music_service;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::haptics::{NotificationKind, notification};
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::button_style::{ButtonAction, plain};
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::progress_view::progress_view;
use rise_ui::swiftui::shapes::{SwiftUiShapes, pass_through};
use rise_ui::swiftui::stacks::{hstack, vstack};
use rise_ui::swiftui::transition::{Presence, Transition, UnitPoint};
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};

use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatCountFormatter;
use crate::modules::music::pages::playlist_collaboration::playlist_collaboration_components::{
    PlaylistCollaborationCover, PlaylistCollaborationUserLabel,
};
use crate::modules::music::stores::music_actions::music_types::{
    MusicPlaylistInvitePreview, MusicPlaylistInviteSheetPhase, MusicPlaylistInviteSheetState,
};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

const SCRIM_OPACITY: f32 = 0.5;
const SHEET_CORNER_RADIUS: f32 = 34.0;
const SHEET_VERTICAL_PADDING: f32 = 14.0;
const SHEET_MAX_WIDTH: f32 = 560.0;
const SHEET_WINDOW_MARGIN: f32 = 16.0;
const CLOSE_BUTTON_SIZE: f32 = 30.0;
const CLOSE_BUTTON_INSET: f32 = 14.0;
const SHEET_PRESENT_SCALE: f32 = 0.94;
const PRESENTATION_RESPONSE: f64 = 0.36;
const PRESENTATION_DAMPING: f64 = 0.88;

/// callback with the playlist id
pub type PlaylistInviteOpenPlaylist = Rc<dyn Fn(String, &mut Window, &mut App)>;
/// callback with user id and tag
pub type PlaylistInviteOpenProfile = Rc<dyn Fn(String, String, &mut Window, &mut App)>;
pub type PlaylistInviteRequest = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// mounted once over the root, shows while the store holds a sheet
pub struct PlaylistInviteSheetHost {
    on_open_playlist: PlaylistInviteOpenPlaylist,
    on_open_profile: PlaylistInviteOpenProfile,
    services: Entity<MusicServicesStore>,
    scrim: Presence<()>,
    panel: Presence<MusicPlaylistInviteSheetState>,
    focus_handle: FocusHandle,
    focus_before_presentation: Option<FocusHandle>,
    _services_observation: Subscription,
}

impl PlaylistInviteSheetHost {
    pub fn new(
        on_open_playlist: impl Fn(String, &mut Window, &mut App) + 'static,
        on_open_profile: impl Fn(String, String, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let services = MusicServicesStore::shared(cx);
        let services_observation = cx.observe_in(&services, window, |this, _, window, cx| this.services_changed(window, cx));
        let mut host = Self {
            on_open_playlist: Rc::new(on_open_playlist),
            on_open_profile: Rc::new(on_open_profile),
            services,
            scrim: Presence::new(None).with_transition(Transition::opacity()),
            panel: Presence::new(None)
                .with_transition(Transition::scale_anchored(SHEET_PRESENT_SCALE, UnitPoint::CENTER).combined(Transition::opacity())),
            focus_handle: cx.focus_handle(),
            focus_before_presentation: None,
            _services_observation: services_observation,
        };
        host.services_changed(window, cx);
        host
    }

    fn services_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.services.read(cx).playlist_invite_sheet.clone();
        let presentable = state.is_some();
        let was_presentable = self.panel.is_present();
        let now = RiseClock::monotonic();
        let animation = spring(PRESENTATION_RESPONSE, PRESENTATION_DAMPING);
        self.scrim.set_in(presentable.then_some(()), animation, now, cx);
        self.panel.set_in(state, animation, now, cx);
        if presentable != was_presentable {
            if presentable {
                self.focus_before_presentation = window.focused(cx);
                window.focus(&self.focus_handle, cx);
            } else if let Some(focus) = self.focus_before_presentation.take() {
                window.focus(&focus, cx);
            }
        }
        cx.notify();
    }

    fn dismiss(cx: &mut App) {
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.dismiss_playlist_invite_sheet(cx));
    }

    fn retry(cx: &mut App) {
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.retry_playlist_invite_preview(cx));
    }

    fn handle_request(token: &str, cx: &mut App) {
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.request_playlist_join_from_sheet(
                token,
                |ok, cx| {
                    if !ok {
                        return;
                    }
                    Self::dismiss(cx);
                    notification(NotificationKind::Success);
                    Notifier::send(
                        tr("music_playlist_preview_request_sent"),
                        NotifierSendOptions {
                            content: Some(tr("music_playlist_preview_request_sent_hint")),
                            r#type: NotifierType::Success,
                            position: NotifierPosition::Bottom,
                            icon: NotifierIcon::Emoji("\u{2705}".to_owned()),
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                },
                cx,
            );
        });
    }

    fn panel_content(
        &self,
        state: &MusicPlaylistInviteSheetState,
        presentable: bool,
        theme: &AppTheme,
    ) -> AnyElement {
        let on_open_playlist = Rc::clone(&self.on_open_playlist);
        let on_open_profile = Rc::clone(&self.on_open_profile);
        vstack(0.0)
            .id("playlist-invite-sheet-panel")
            .when(presentable, |panel| panel.occlude())
            .w_full()
            .max_w(px(SHEET_MAX_WIDTH))
            .relative()
            .py(px(SHEET_VERTICAL_PADDING))
            .bg(theme.bg._100)
            .rounded(px(SHEET_CORNER_RADIUS))
            .border_1()
            .border_color(theme.text.secondary.opacity(0.18))
            .child(PlaylistInviteSheetContent::new(
                state.clone(),
                Rc::new(|_: &mut Window, cx: &mut App| Self::retry(cx)),
                Rc::new(|token: String, _: &mut Window, cx: &mut App| Self::handle_request(&token, cx)),
                Rc::new(move |playlist_id: String, window: &mut Window, cx: &mut App| {
                    Self::dismiss(cx);
                    on_open_playlist(playlist_id, window, cx);
                }),
                Rc::new(move |user_id: String, tag: String, window: &mut Window, cx: &mut App| {
                    Self::dismiss(cx);
                    on_open_profile(user_id, tag, window, cx);
                }),
            ))
            .child(Self::close_button(theme))
            .when(!presentable, |panel| panel.child(div().absolute().inset_0().occlude()))
            .into_any_element()
    }

    fn close_button(theme: &AppTheme) -> AnyElement {
        div()
            .absolute()
            .top(px(CLOSE_BUTTON_INSET))
            .right(px(CLOSE_BUTTON_INSET))
            .child(
                plain("playlist-invite-close").on_press(|_, cx| Self::dismiss(cx)).child(
                    div()
                        .size(px(CLOSE_BUTTON_SIZE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme.bg._200)
                        .capsule()
                        .children(IconUi::sized("xmark", px(13.0), theme.text.secondary, false)),
                ),
            )
            .into_any_element()
    }
}

impl Render for PlaylistInviteSheetHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let presentable = self.panel.is_present();

        let scrim = self.scrim.render_transitioned(now, window, |_| {
            div()
                .id("playlist-invite-sheet-scrim")
                .absolute()
                .inset_0()
                .bg(black().opacity(SCRIM_OPACITY))
                .when(presentable, |scrim| {
                    scrim.occlude().on_click(|_, _, cx| Self::dismiss(cx))
                })
        });
        let panel = self.panel.render_transitioned(now, window, |state| {
            div()
                .absolute()
                .inset_0()
                .p(px(SHEET_WINDOW_MARGIN))
                .flex()
                .items_center()
                .justify_center()
                .child(self.panel_content(state, presentable, &theme))
        });

        div()
            .id("playlist-invite-sheet-host")
            .absolute()
            .inset_0()
            .size_full()
            .when(presentable, |host| {
                host.track_focus(&self.focus_handle).layer_keys(LayerCancel::dismiss(|_, cx| Self::dismiss(cx)))
            })
            .children(scrim)
            .children(panel)
    }
}

#[derive(IntoElement)]
pub struct PlaylistInviteSheetContent {
    state: MusicPlaylistInviteSheetState,
    on_retry: ButtonAction,
    on_request: PlaylistInviteRequest,
    on_open_playlist: PlaylistInviteOpenPlaylist,
    on_open_profile: PlaylistInviteOpenProfile,
}

impl PlaylistInviteSheetContent {
    pub fn new(
        state: MusicPlaylistInviteSheetState,
        on_retry: ButtonAction,
        on_request: PlaylistInviteRequest,
        on_open_playlist: PlaylistInviteOpenPlaylist,
        on_open_profile: PlaylistInviteOpenProfile,
    ) -> Self {
        Self { state, on_retry, on_request, on_open_playlist, on_open_profile }
    }

    fn preview_state(&self, preview: &MusicPlaylistInvitePreview, theme: &AppTheme) -> AnyElement {
        let description = (!preview.description.is_empty()).then(|| {
            div().px(px(4.0)).child(
                MainText::new(preview.description.clone())
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary)
                    .number_of_lines(4),
            )
        });
        vstack(14.0)
            .w_full()
            .child(div().pt(px(6.0)).child(PlaylistCollaborationCover::new("playlist-invite-cover", preview.cover_url.clone()).size(88.0)))
            .child(
                vstack(7.0)
                    .child(
                        div().px(px(10.0)).py(px(5.0)).bg(theme.primary._100.opacity(0.13)).capsule().child(
                            MainText::new(tr("music_playlist_preview_type"))
                                .px(12.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.primary._100),
                        ),
                    )
                    .child(
                        MainText::new(preview.title.clone())
                            .px(21.0)
                            .font_weight(FontWeight::BOLD)
                            .color(theme.text.primary)
                            .number_of_lines(2),
                    )
                    .child(MainText::new(Self::participants_text(preview)).px(14.0).color(theme.text.secondary)),
            )
            .children(description)
            .child(self.owner_row(preview, theme))
            .child(div().w_full().pt(px(2.0)).child(self.action_area(preview, theme)))
            .into_any_element()
    }

    fn owner_row(&self, preview: &MusicPlaylistInvitePreview, theme: &AppTheme) -> AnyElement {
        let on_open_profile = Rc::clone(&self.on_open_profile);
        let owner_id = preview.owner.id.clone();
        let owner_tag = preview.owner.tag.clone();
        plain("playlist-invite-owner")
            .on_press(move |window, cx| on_open_profile(owner_id.clone(), owner_tag.clone(), window, cx))
            .child(
                hstack(8.0)
                    .px(px(12.0))
                    .py(px(7.0))
                    .bg(theme.bg._200)
                    .capsule()
                    .child(
                        UserLogoUi::new("playlist-invite-owner-logo")
                            .source(SharedString::from(preview.owner.logo.clone()))
                            .name(SharedString::from(preview.owner.name.clone()))
                            .user_id(SharedString::from(preview.owner.id.clone()))
                            .size(26.0)
                            .show_border(false),
                    )
                    .child(MainText::new(tr("music_playlist_preview_by_owner")).px(13.0).color(theme.text.secondary))
                    .child(
                        MainText::new(PlaylistCollaborationUserLabel::display_name(&preview.owner))
                            .px(13.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .number_of_lines(1),
                    )
                    .children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(10.0), theme.text.secondary.opacity(0.7), false)),
            )
            .into_any_element()
    }

    fn action_area(&self, preview: &MusicPlaylistInvitePreview, theme: &AppTheme) -> AnyElement {
        let submission_error = self.state.submission_error.as_ref().filter(|error| !error.is_empty()).map(|error| {
            div().py(px(6.0)).child(
                MainText::new(error.clone()).px(14.0).tac(TextAlignT::Center).color(theme.semantic.error_100),
            )
        });
        let viewer_state = preview.viewer_state.as_str();
        let action = if viewer_state == music_service::PLAYLIST_VIEWER_STATE_CAN_REQUEST {
            self.request_action(preview, theme)
        } else if viewer_state == music_service::PLAYLIST_VIEWER_STATE_MEMBER
            || viewer_state == music_service::PLAYLIST_VIEWER_STATE_OWNER
        {
            let on_open_playlist = Rc::clone(&self.on_open_playlist);
            let playlist_id = preview.playlist_id.clone();
            plain("playlist-invite-open")
                .w_full()
                .on_press(move |window, cx| on_open_playlist(playlist_id.clone(), window, cx))
                .child(
                    div()
                        .w_full()
                        .h(px(52.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_rect(15.0)
                        .bg(theme.primary._100)
                        .child(
                            MainText::new(tr("music_playlist_preview_open"))
                                .px(16.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.content_on(theme.primary._100)),
                        ),
                )
                .into_any_element()
        } else {
            div()
                .w_full()
                .py(px(10.0))
                .child(
                    MainText::new(Self::reason_text(viewer_state))
                        .px(14.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary)
                        .number_of_lines(4),
                )
                .into_any_element()
        };
        vstack(16.0).w_full().children(submission_error).child(action).into_any_element()
    }

    fn request_action(&self, preview: &MusicPlaylistInvitePreview, theme: &AppTheme) -> AnyElement {
        let is_submitting = self.state.is_submitting();
        let capacity = (preview.active_participant_count >= preview.effective_participant_limit).then(|| {
            MainText::new(tr("music_playlist_preview_capacity"))
                .px(13.0)
                .tac(TextAlignT::Center)
                .color(theme.text.secondary)
                .number_of_lines(3)
        });
        let on_request = Rc::clone(&self.on_request);
        let token = self.state.token.clone();
        let label = if is_submitting {
            progress_view().tint(white()).into_any_element()
        } else {
            MainText::new(tr("music_playlist_preview_join"))
                .px(16.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(white())
                .into_any_element()
        };
        vstack(10.0)
            .w_full()
            .children(capacity)
            .child(
                plain("playlist-invite-join")
                    .w_full()
                    .opacity(if is_submitting { 0.85 } else { 1.0 })
                    .disabled(is_submitting)
                    .on_press(move |window, cx| on_request(token.clone(), window, cx))
                    .child(
                        div()
                            .w_full()
                            .h(px(52.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_rect(15.0)
                            .bg(theme.primary._100)
                            .child(label),
                    ),
            )
            .into_any_element()
    }

    fn loading_state() -> AnyElement {
        pass_through()
            .w_full()
            .min_h(px(220.0))
            .flex()
            .flex_col()
            .justify_center()
            .pt(px(6.0))
            .child(
                vstack(14.0)
                    .w_full()
                    .child(SkeletonBlock::new(88.0, 88.0, 20.0))
                    .child(
                        vstack(9.0)
                            .child(SkeletonLine::new(170.0, 16.0))
                            .child(SkeletonLine::new(108.0, 11.0)),
                    )
                    .child(SkeletonLine::new(236.0, 10.0))
                    .child(div().pt(px(2.0)).child(SkeletonBlock::new(150.0, 40.0, 20.0)))
                    .child(div().w_full().pt(px(4.0)).child(SkeletonBlock::new(None::<f32>, 52.0, 15.0)))
                    .skeleton_shimmer(None),
            )
            .into_any_element()
    }

    fn error_state(&self, message: &str, retryable: bool, theme: &AppTheme) -> AnyElement {
        let on_retry = Rc::clone(&self.on_retry);
        vstack(15.0)
            .w_full()
            .min_h(px(220.0))
            .justify_center()
            .children(IconUi::sized(
                "exclamationmark.arrow.triangle.2.circlepath",
                px(38.0),
                theme.text.secondary,
                false,
            ))
            .child(MainText::new(message.to_owned()).px(15.0).tac(TextAlignT::Center).color(theme.text.secondary))
            .when(retryable, |column| {
                column.child(
                    plain("playlist-invite-retry").w_full().on_press(move |window, cx| on_retry(window, cx)).child(
                        div()
                            .w_full()
                            .h(px(48.0))
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
                )
            })
            .into_any_element()
    }

    fn participants_text(preview: &MusicPlaylistInvitePreview) -> String {
        let members = ChatCountFormatter::members(preview.active_participant_count);
        if preview.owner_is_premium || preview.effective_participant_limit <= 0 {
            return members;
        }
        format!("{members} \u{b7} {}/{}", preview.active_participant_count, preview.effective_participant_limit)
    }

    fn reason_text(viewer_state: &str) -> String {
        if viewer_state == music_service::PLAYLIST_VIEWER_STATE_PENDING {
            return tr("music_playlist_preview_pending");
        }
        if viewer_state == music_service::PLAYLIST_VIEWER_STATE_SUSPENDED_PLAN {
            return tr("music_playlist_preview_suspended");
        }
        tr("music_playlist_preview_unavailable")
    }
}

impl RenderOnce for PlaylistInviteSheetContent {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let content = match &self.state.phase {
            MusicPlaylistInviteSheetPhase::Loading => Self::loading_state(),
            MusicPlaylistInviteSheetPhase::Preview(preview) => self.preview_state(preview, &theme),
            MusicPlaylistInviteSheetPhase::Failed { message, retryable } => self.error_state(message, *retryable, &theme),
        };
        vstack(0.0).w_full().px(px(22.0)).pt(px(10.0)).pb(px(8.0)).child(content)
    }
}
