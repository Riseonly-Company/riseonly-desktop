use std::sync::Arc;

use gpui::{App, Context, ElementId, Entity, FontWeight, Hsla, SharedString, Subscription, Window, rgb};
use rise_i18n::number_format::decimal_number_formatter::DecimalNumberFormatter;
use rise_i18n::{I18n, tr};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::rise_symbol::RiseSymbol;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_types::ModerationQueueMetricsModel;
use crate::modules::music::stores::music_actions::music_wire_types::WireModerationQueueMetricsDTOExt;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

const LIKED_COLLECTION_ID: &str = "liked_songs";
const LIKED_COLOR: u32 = 0xE84393;
const LIBRARY_COLOR: u32 = 0x6C5CE7;
const LOCAL_TRACKS_COLOR: u32 = 0x0984E3;
const ARTIST_COLOR: u32 = 0xFF6B6B;
const ARTIST_REQUEST_COLOR: u32 = 0xF19A1A;
const STUDIO_COLOR: u32 = 0x00B894;
const TRACK_REQUESTS_COLOR: u32 = 0x6C5CE7;
const ALBUM_REQUESTS_COLOR: u32 = 0xFDCB6E;
const ADMIN_ARTIST_COLOR: u32 = 0xE17055;
const ADMIN_TRACK_COLOR: u32 = 0x0984E3;
const ADMIN_ALBUM_COLOR: u32 = 0xA29BFE;
const PENDING_TRACKS_COLOR: u32 = 0xFDCB6E;
const PENDING_ALBUMS_COLOR: u32 = 0xA29BFE;
const APPROVED_TODAY_COLOR: u32 = 0x00B894;

fn named_child(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

/// int with thousands separators
fn formatted(value: i64) -> String {
    DecimalNumberFormatter::new(I18n::shared().code()).string(value.max(0) as u64)
}

struct MusicSettingsState {
    is_covered: bool,
    _subscriptions: Vec<Subscription>,
}

impl MusicSettingsState {
    fn new(nav_store: Option<Entity<NavigationStore>>, cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let services = MusicServicesStore::shared(cx);
        let socket = SaiWebSocketManager::shared(cx);
        let mut subscriptions = vec![
            cx.observe(&actions, |_, _, cx| cx.notify()),
            cx.observe(&services, |_, _, cx| cx.notify()),
            cx.observe(&socket, |_, _, cx| cx.notify()),
        ];
        if let Some(nav_store) = nav_store {
            subscriptions.push(cx.observe(&nav_store, |state, nav_store, cx| state.stack_did_change(&nav_store, cx)));
        }
        Self { is_covered: false, _subscriptions: subscriptions }
    }

    fn stack_did_change(&mut self, nav_store: &Entity<NavigationStore>, cx: &mut Context<Self>) {
        let path = &nav_store.read(cx).path;
        let is_top = path.last() == Some(&RootRoute::MusicSettings);
        let is_in_stack = is_top || path.contains(&RootRoute::MusicSettings);
        tracing::warn!(target: "riseonly::X1379", is_top, is_in_stack, covered = self.is_covered, "music settings stack change");
        if !is_top {
            self.is_covered = is_in_stack;
            return;
        }
        if !self.is_covered {
            return;
        }
        self.is_covered = false;
        MusicSettings::page_did_appear(cx);
    }
}

#[derive(IntoElement)]
pub struct MusicSettings {
    id: ElementId,
}

impl Default for MusicSettings {
    fn default() -> Self {
        Self::new()
    }
}

impl MusicSettings {
    pub fn new() -> Self {
        Self { id: ElementId::Name(SharedString::new_static("music-settings")) }
    }
}

impl RenderOnce for MusicSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let state_nav_store = nav_store.clone();
        let state_id = named_child(&self.id, "state");
        let _state = window.use_keyed_state(state_id, cx, move |_, cx| MusicSettingsState::new(state_nav_store, cx));
        let is_connected = SaiWebSocketManager::shared(cx).read(cx).is_connected();

        on_change(named_child(&self.id, "connected"), is_connected, false, window, cx, |_, connected, _, cx| {
            let is_staff = MusicServicesStore::shared(cx).read(cx).music_is_staff(cx);
            if !*connected || !is_staff {
                return;
            }
            MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_moderation_queue_metrics(cx));
        });

        let back_store = nav_store.clone();
        SettingsWrapperUi::new(tr("settings_music_title"), move |_, cx| {
            if let Some(nav_store) = &back_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(MusicSettingsBody { id: named_child(&self.id, "body"), nav_store, is_connected })
        .on_appear(named_child(&self.id, "appear"), |_, cx| Self::page_did_appear(cx))
    }
}

impl MusicSettings {
    /// body of the on appear hook
    fn page_did_appear(cx: &mut App) {
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.load_artist_status_if_needed(cx);
            store.load_moderation_queue_metrics_if_needed(true, cx);
        });
    }
}

#[derive(IntoElement)]
struct MusicSettingsBody {
    id: ElementId,
    nav_store: Option<Entity<NavigationStore>>,
    is_connected: bool,
}

impl MusicSettingsBody {
    fn route_button(
        &self,
        id: &'static str,
        icon: &'static str,
        icon_background: u32,
        text: String,
        route: RootRoute,
    ) -> GroupedBtn {
        let nav_store = self.nav_store.clone();
        let background: Hsla = rgb(icon_background).into();
        GroupedBtn::new(id, text).icon(icon).icon_background(background).callback(move |_, cx| {
            if let Some(nav_store) = &nav_store {
                let route = route.clone();
                nav_store.update(cx, |nav_store, cx| {
                    nav_store.push(route, NavigationStoreSettlingPush::Drop, cx);
                });
            }
        })
    }

    fn moderation_metrics_section(
        &self,
        theme: &AppTheme,
        metrics: Option<ModerationQueueMetricsModel>,
        is_loading: bool,
        is_stale: bool,
    ) -> Div {
        let mut section = vstack(10.0).w_full().align(HorizontalAlignment::Leading).child(
            div()
                .px(px(16.0))
                .child(MainText::new(tr("music_moderation_metrics_title")).px(15.0).font_weight(FontWeight::SEMIBOLD)),
        );
        if is_loading {
            section = section.child(
                hstack(0.0)
                    .w_full()
                    .py(px(20.0))
                    .child(spacer())
                    .child(progress_view().tint(theme.primary._100))
                    .child(spacer()),
            );
        } else if let Some(metrics) = metrics {
            section = section
                .child(
                    div()
                        .w_full()
                        .px(px(16.0))
                        .grid()
                        .grid_cols(2)
                        .gap(px(10.0))
                        .child(Self::moderation_metric_card(
                            formatted(metrics.pending_tracks),
                            tr("music_moderation_pending_tracks"),
                            rgb(PENDING_TRACKS_COLOR).into(),
                            theme,
                        ))
                        .child(Self::moderation_metric_card(
                            formatted(metrics.pending_albums),
                            tr("music_moderation_pending_albums"),
                            rgb(PENDING_ALBUMS_COLOR).into(),
                            theme,
                        ))
                        .child(Self::moderation_metric_card(
                            formatted(metrics.approved_today),
                            tr("music_moderation_approved_today"),
                            rgb(APPROVED_TODAY_COLOR).into(),
                            theme,
                        ))
                        .child(Self::moderation_metric_card(
                            formatted(metrics.rejected_today),
                            tr("music_moderation_rejected_today"),
                            theme.semantic.error_100,
                            theme,
                        )),
                )
                .child(
                    div().w_full().px(px(16.0)).child(
                        hstack(0.0)
                            .w_full()
                            .px(px(16.0))
                            .py(px(10.0))
                            .bg(theme.bg._200)
                            .rounded(theme.radius._300)
                            .child(SecondaryText::new(tr("music_moderation_avg_review_hours")).px(13.0))
                            .child(spacer())
                            .child(
                                MainText::new(metrics.avg_review_time_hours_text())
                                    .px(14.0)
                                    .font_weight(FontWeight::SEMIBOLD),
                            ),
                    ),
                );
            if is_stale {
                section = section.child(self.moderation_metrics_error(theme));
            }
        } else {
            section = section.child(self.moderation_metrics_error(theme));
        }
        section
    }

    fn moderation_metrics_error(&self, theme: &AppTheme) -> Div {
        vstack(8.0)
            .w_full()
            .px(px(16.0))
            .py(px(12.0))
            .child(SecondaryText::new(tr("music_moderation_metrics_error")).px(13.0).tac(TextAlignT::Center))
            .child(
                plain(named_child(&self.id, "metrics-retry"))
                    .on_press(|_, cx| {
                        MusicInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.refresh_moderation_queue_metrics(cx));
                    })
                    .child(
                        MainText::new(tr("music_moderation_metrics_retry"))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.primary._100),
                    ),
            )
    }

    fn moderation_metric_card(value: String, label: String, color: Hsla, theme: &AppTheme) -> Div {
        vstack(4.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .p(px(12.0))
            .bg(color.opacity(0.1))
            .rounded(theme.radius._300)
            .child(MainText::new(value).px(22.0).font_weight(FontWeight::BOLD))
            .child(SecondaryText::new(label).px(12.0).line_limit(2))
    }
}

impl RenderOnce for MusicSettingsBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let is_staff = MusicServicesStore::shared(cx).read(cx).music_is_staff(cx);
        let is_artist = MusicServicesStore::shared(cx).update(cx, |service, cx| service.music_is_approved_artist(cx));
        let actions = MusicActionsStore::shared(cx);
        let (moderation_metrics, is_metrics_pending, has_metrics_error) = {
            let slice = &actions.read(cx).get_moderation_queue_metrics_sai;
            (slice.data().map(|data| data.as_model()), slice.is_pending(), slice.error().is_some())
        };
        let is_loading = is_metrics_pending && moderation_metrics.is_none() && self.is_connected;
        let is_stale = has_metrics_error || !self.is_connected;

        scroll_view(named_child(&self.id, "scroll"))
            .shows_indicators(false)
            .size_full()
            .bg(theme.bg._100)
            .list_shell_scroll_under_chrome(cx)
            .child(
                vstack(16.0)
                    .w_full()
                    .child(
                        div().w_full().px(px(16.0)).pt(px(12.0)).child(
                            hstack(12.0)
                                .w_full()
                                .p(px(14.0))
                                .bg(theme.primary._100.opacity(0.08))
                                .rounded(theme.radius._300)
                                .children(IconUi::sized("music.note", px(20.0), theme.primary._100, false))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(SecondaryText::new(tr("music_settings_info")).px(13.0).line_limit(4)),
                                ),
                        ),
                    )
                    .child(GroupedBtns::new(
                        named_child(&self.id, "listening"),
                        vec![
                            self.route_button(
                                "liked",
                                "heart.fill",
                                LIKED_COLOR,
                                tr("music_liked_songs"),
                                RootRoute::MusicPlaylistDetail { collection_id: LIKED_COLLECTION_ID.to_owned() },
                            ),
                            self.route_button(
                                "library",
                                "music.note.list",
                                LIBRARY_COLOR,
                                tr("music_library"),
                                RootRoute::MusicLibrary,
                            ),
                            self.route_button(
                                "local_tracks",
                                "externaldrive.fill",
                                LOCAL_TRACKS_COLOR,
                                tr("music_local_tracks_title"),
                                RootRoute::MusicLocalTracks,
                            ),
                        ],
                    ))
                    .child(GroupedBtns::new(
                        named_child(&self.id, "becoming-artist"),
                        vec![
                            self.route_button(
                                "artist",
                                RiseSymbol::ARTIST,
                                ARTIST_COLOR,
                                tr("music_become_artist"),
                                RootRoute::BecomeArtistSettings,
                            ),
                            self.route_button(
                                "my_artist_req",
                                "doc.text.fill",
                                ARTIST_REQUEST_COLOR,
                                tr("music_my_artist_request"),
                                RootRoute::MyMusicArtistRequest,
                            ),
                        ],
                    ))
                    .when(is_artist, |column| {
                        column.child(GroupedBtns::new(
                            named_child(&self.id, "artist"),
                            vec![
                                self.route_button(
                                    "studio",
                                    "waveform.path.ecg",
                                    STUDIO_COLOR,
                                    tr("music_artist_studio"),
                                    RootRoute::ArtistStudio,
                                ),
                                self.route_button(
                                    "my_track_req",
                                    "music.note",
                                    TRACK_REQUESTS_COLOR,
                                    tr("music_my_track_requests"),
                                    RootRoute::MyMusicTrackRequests,
                                ),
                                self.route_button(
                                    "my_album_req",
                                    "square.stack.fill",
                                    ALBUM_REQUESTS_COLOR,
                                    tr("music_album_requests"),
                                    RootRoute::MyMusicAlbumRequests,
                                ),
                            ],
                        ))
                    })
                    .when(is_staff, |column| {
                        column
                            .child(self.moderation_metrics_section(&theme, moderation_metrics, is_loading, is_stale))
                            .child(GroupedBtns::new(
                                named_child(&self.id, "admin"),
                                vec![
                                    self.route_button(
                                        "admin_artist",
                                        "person.badge.shield.checkmark.fill",
                                        ADMIN_ARTIST_COLOR,
                                        tr("music_admin_artist_requests"),
                                        RootRoute::MusicArtistRequestsAdmin,
                                    ),
                                    self.route_button(
                                        "admin_track",
                                        "waveform.badge.plus",
                                        ADMIN_TRACK_COLOR,
                                        tr("music_admin_track_requests"),
                                        RootRoute::MusicTrackRequestsAdmin,
                                    ),
                                    self.route_button(
                                        "admin_album",
                                        "square.stack.fill",
                                        ADMIN_ALBUM_COLOR,
                                        tr("music_admin_album_requests"),
                                        RootRoute::MusicAlbumRequestsAdmin,
                                    ),
                                ],
                            ))
                    })
                    .child(div().w_full().h(px(80.0))),
            )
    }
}
