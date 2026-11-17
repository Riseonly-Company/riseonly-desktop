use std::collections::HashSet;
use std::ops::{Range, RangeInclusive};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyView, App, Context, ElementId, Entity, FontWeight, Hsla, SharedString, StyleRefinement, Subscription, Window,
    linear_color_stop, linear_gradient, rgb, white,
};
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::{tr, tr_plural};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Presence, Transition};

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::modules::gifts::components::gift_shop_shelf_grid::GiftScrollWindowProbe;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::bottomsheets::playlists::create_playlist_sheet::CreatePlaylistSheet;
use crate::modules::music::widgets::bottomsheets::playlists::playlist_visibility::PlaylistVisibility;
use crate::modules::music::widgets::music_cover_placeholder::MusicCoverPlaceholder;
use crate::modules::music::widgets::music_skeletons::MusicLibraryListSkeleton;
use crate::modules::music::widgets::music_track_cover_image::{MusicCoverImage, MusicTrackCoverImage};

const ROW_SEPARATOR_LEADING: f32 = 72.;
const ROW_COVER_SIDE: f32 = 52.;
const ROW_COVER_RADIUS: f32 = 8.;
const PLAYLIST_ROW_HEIGHT: f32 = 10. + ROW_COVER_SIDE + 10. + 0.5;
const PLAYLIST_INITIAL_ROWS: usize = 12;
const PLAYLIST_OVERSCAN: f32 = 6. * PLAYLIST_ROW_HEIGHT;
const CHECKMARK_SPRING_RESPONSE: f64 = 0.2;
const CHECKMARK_SPRING_DAMPING: f64 = 0.8;
const LIKED_COVER_START: u32 = 0xE84393;
const LIKED_COVER_END: u32 = 0xFD79A8;
const LIKED_COVER_ANGLE: f32 = 135.;

type AddToPlaylistDismiss = Rc<dyn Fn(&mut Window, &mut App)>;

struct AddToPlaylistSheetState {
    show_create_sheet: bool,
    added_playlist_ids: HashSet<String>,
    add_error: Option<String>,
    rows_height: f32,
    _services_observation: Subscription,
    _actions_observation: Subscription,
}

impl AddToPlaylistSheetState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = MusicServicesStore::shared(cx);
        let actions = MusicActionsStore::shared(cx);
        Self {
            show_create_sheet: false,
            added_playlist_ids: HashSet::new(),
            add_error: None,
            rows_height: 0.,
            _services_observation: cx.observe(&services, |_, _, cx| cx.notify()),
            _actions_observation: cx.observe(&actions, |_, _, cx| cx.notify()),
        }
    }

    /// a server refusal shows only as a checkmark that never appears
    /// no toast no retry here, so a silent row reads as no such playlist
    fn add_track(&mut self, playlist_id: &str, track_id: &str, cx: &mut Context<Self>) {
        if playlist_id.is_empty() {
            return;
        }
        self.add_error = None;
        cx.notify();
        let added_state = cx.weak_entity();
        let failed_state = cx.weak_entity();
        let added_id = playlist_id.to_owned();
        let playlist_id = playlist_id.to_owned();
        let track_id = track_id.to_owned();
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.add_track_to_playlist(
                &playlist_id,
                &track_id,
                Some(Rc::new(move |cx: &mut App| {
                    let _ = added_state.update(cx, |state, cx| {
                        state.added_playlist_ids.insert(added_id.clone());
                        cx.notify();
                    });
                })),
                Some(Rc::new(move |message: String, cx: &mut App| {
                    let _ = failed_state.update(cx, |state, cx| {
                        state.add_error = Some(message);
                        cx.notify();
                    });
                })),
                cx,
            );
        });
    }
}

#[derive(Clone, PartialEq)]
struct PlaylistRowData {
    id: String,
    title: String,
    cover_url: Option<String>,
    subtitle: String,
}

/// one row of the lazy list
#[derive(Clone)]
enum AddToPlaylistRow {
    CreateNew,
    Liked,
    ServerSkeleton,
    ServerFailure(String),
    ServerPlaylist(PlaylistRowData),
    LocalSkeleton,
    LocalFailure(String),
    LocalDestination { data: PlaylistRowData, is_last: bool },
    LoadingMore,
    RetryLoadMore,
    Empty,
    BottomInset,
}

impl AddToPlaylistRow {
    /// fixed height playlist row, a run of them can be stood in for by a spacer
    fn is_playlist(&self) -> bool {
        matches!(self, Self::ServerPlaylist(_) | Self::LocalDestination { .. })
    }
}

/// picks the playlist for a track, the liked list or a new playlist
#[derive(IntoElement)]
pub struct AddToPlaylistSheet {
    id: ElementId,
    track: TrackModel,
    on_dismiss: Option<AddToPlaylistDismiss>,
}

impl AddToPlaylistSheet {
    pub fn new(id: impl Into<ElementId>, track: TrackModel) -> Self {
        Self { id: id.into(), track, on_dismiss: None }
    }

    pub fn on_dismiss(mut self, on_dismiss: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(on_dismiss));
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    /// a personal upload isnt in the server catalog so music-service refuses it in a server playlist
    /// it only lives in local playlists
    fn takes_server_playlists(&self) -> bool {
        self.track.status != RiseMusicTrackStatus::LocalLibrary.wire()
    }

    fn rows(&self, cx: &mut App) -> Vec<AddToPlaylistRow> {
        let takes_server_playlists = self.takes_server_playlists();
        let services = MusicServicesStore::shared(cx);
        let actions = MusicActionsStore::shared(cx);
        let my_playlists = if takes_server_playlists {
            services.update(cx, |services, cx| services.add_track_destination_playlists(cx))
        } else {
            Vec::new()
        };
        let server_playlists_pending = takes_server_playlists && actions.read(cx).get_playlists_mine_sai.is_pending();
        let server_playlists_error =
            if takes_server_playlists { actions.read(cx).get_playlists_mine_sai.error() } else { None };
        let (destinations, destinations_loading, destinations_error, loading_more, has_more) =
            services.update(cx, |services, cx| {
                (
                    services.local_playlist_destinations(cx),
                    services.local_playlist_destinations_loading(cx),
                    services.local_playlist_destinations_error_text(cx),
                    services.local_playlist_destinations_loading_more(cx),
                    services.local_playlist_destinations_has_more(cx),
                )
            });

        let mut rows = vec![AddToPlaylistRow::CreateNew];
        if self.track.transfer_phase.is_none() {
            rows.push(AddToPlaylistRow::Liked);
        }

        if my_playlists.is_empty() && server_playlists_pending {
            rows.push(AddToPlaylistRow::ServerSkeleton);
        } else if let (true, Some(error)) = (my_playlists.is_empty(), server_playlists_error.clone()) {
            rows.push(AddToPlaylistRow::ServerFailure(error));
        } else {
            rows.extend(my_playlists.iter().map(|playlist| {
                AddToPlaylistRow::ServerPlaylist(PlaylistRowData {
                    id: playlist.id.clone().unwrap_or_default(),
                    title: playlist.display_title(),
                    cover_url: playlist.cover_url.clone(),
                    subtitle: tr_plural(
                        "music_track_count",
                        playlist.tracks.as_ref().map_or(0, |tracks| tracks.len() as i64),
                    ),
                })
            }));
        }

        if destinations_loading {
            rows.push(AddToPlaylistRow::LocalSkeleton);
        } else if let (true, Some(error)) = (destinations.is_empty(), destinations_error.clone()) {
            rows.push(AddToPlaylistRow::LocalFailure(error));
        } else {
            let last_id = destinations.last().map(|destination| destination.id.clone());
            rows.extend(destinations.iter().map(|destination| AddToPlaylistRow::LocalDestination {
                data: PlaylistRowData {
                    id: destination.id.clone(),
                    title: destination.title.clone(),
                    cover_url: Some(destination.cover_url.clone()),
                    subtitle: if destination.viewer_role.to_lowercase() == "owner" {
                        tr("music_message_import_playlist_owner")
                    } else {
                        tr("music_message_import_playlist_member")
                    },
                },
                is_last: last_id.as_deref() == Some(destination.id.as_str()),
            }));
        }

        if loading_more {
            rows.push(AddToPlaylistRow::LoadingMore);
        } else if has_more && destinations_error.is_some() {
            rows.push(AddToPlaylistRow::RetryLoadMore);
        }

        if my_playlists.is_empty()
            && destinations.is_empty()
            && !server_playlists_pending
            && !destinations_loading
            && server_playlists_error.is_none()
            && destinations_error.is_none()
        {
            rows.push(AddToPlaylistRow::Empty);
        }
        rows.push(AddToPlaylistRow::BottomInset);
        rows
    }

    fn sheet_header(&self) -> impl IntoElement {
        let on_dismiss = self.on_dismiss.clone();
        div().w_full().flex_none().child(
            SheetHeaderBar::new(self.child_id("header"), tr("music_select_playlist"), move |window, cx| {
                if let Some(on_dismiss) = &on_dismiss {
                    on_dismiss(window, cx);
                }
            })
            .right_system_image(None),
        )
    }

    fn track_header(&self, theme: &AppTheme) -> impl IntoElement {
        hstack(12.)
            .w_full()
            .flex_none()
            .px(px(16.))
            .pt(px(8.))
            .pb(px(16.))
            .child(
                MusicTrackCoverImage::new(self.child_id("track-cover"), self.track.cover_image_url.clone())
                    .size(44.)
                    .corner_radius(8.),
            )
            .child(
                vstack(2.)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w_0()
                    .child(
                        MainText::new(self.track.title.clone())
                            .px(15.)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .line_limit(1),
                    )
                    .child(SecondaryText::new(self.track.artist.clone()).px(13.).line_limit(1)),
            )
    }
}

impl RenderOnce for AddToPlaylistSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, cx| AddToPlaylistSheetState::new(cx));

        let rows = self.rows(cx);
        let add_error = state.read(cx).add_error.clone();
        let rows_height = state.read(cx).rows_height;

        let parts = AddToPlaylistRowParts {
            id: self.id.clone(),
            track: self.track.clone(),
            state: state.clone(),
            rows: Rc::new(rows),
        };

        let show_create_sheet = Binding::entity(
            state.clone(),
            |state: &AddToPlaylistSheetState| state.show_create_sheet,
            |state, value, _| {
                state.show_create_sheet = value;
            },
        );
        let created_state = state.clone();
        let created_track_id = self.track.id.clone();
        let create_sheet = CreatePlaylistSheet::new(self.child_id("create-sheet"), show_create_sheet)
            .locked_visibility(if self.takes_server_playlists() { None } else { Some(PlaylistVisibility::Local) })
            .on_created(move |playlist, cx| {
                MusicInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.refresh_local_playlist_destinations(cx));
                if let Some(id) = playlist.and_then(|playlist| playlist.id) {
                    created_state.update(cx, |state, cx| state.add_track(&id, &created_track_id, cx));
                }
            });

        let mut sheet = vstack(0.)
            .relative()
            .size_full()
            .bg(theme.bg._100)
            .child(self.sheet_header())
            .child(self.track_header(&theme))
            .child(separator(&theme, 0.));

        if let Some(add_error) = add_error {
            sheet = sheet.child(
                div().w_full().flex_none().px(px(16.)).py(px(8.)).child(
                    SecondaryText::new(add_error).px(13.).color(theme.semantic.error_100),
                ),
            );
        }

        let row_count = parts.rows.len();
        let row_elements = parts.elements(window, cx);
        let measured_state = state.clone();
        sheet
            .child(
                scroll_view(self.child_id("rows"))
                    .w_full()
                    .flex_auto()
                    .min_h_0()
                    .when(rows_height > 0., |list| list.h(px(rows_height)))
                    .child(
                        vstack(0.).relative().w_full().children(row_elements).child(measure(
                            self.child_id("rows-size"),
                            move |bounds, _, cx| {
                                let height = f32::from(bounds.size.height);
                                tracing::warn!(target: "riseonly::X816", rows = row_count, height, "add to playlist rows measured");
                                measured_state.update(cx, |state, cx| {
                                    if (state.rows_height - height).abs() > 0.5 {
                                        state.rows_height = height;
                                        cx.notify();
                                    }
                                });
                            },
                        )),
                    ),
            )
            .child(pass_through().absolute().inset_0().child(create_sheet))
            .on_appear(self.child_id("appear"), |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.load_playlists_if_needed(cx);
                    interactions.load_liked_if_needed(cx);
                    interactions.load_local_playlist_destinations_if_needed(cx);
                });
            })
    }
}

struct AddToPlaylistRowParts {
    id: ElementId,
    track: TrackModel,
    state: Entity<AddToPlaylistSheetState>,
    rows: Rc<Vec<AddToPlaylistRow>>,
}

impl AddToPlaylistRowParts {
    fn row_id(&self, prefix: &str, id: &str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::from(format!("{prefix}-{id}")))
    }

    /// list rows, each run of playlist rows is windowed to the visible band
    fn elements(&self, window: &mut Window, cx: &mut App) -> Vec<AnyElement> {
        let mut elements = Vec::with_capacity(self.rows.len());
        let mut index = 0;
        while index < self.rows.len() {
            if !self.rows[index].is_playlist() {
                elements.push(self.row(index, window, cx));
                index += 1;
                continue;
            }
            let start = index;
            while index < self.rows.len() && self.rows[index].is_playlist() {
                index += 1;
            }
            elements.push(self.playlist_run(start..index, window, cx));
        }
        elements
    }

    fn playlist_run(&self, range: Range<usize>, window: &mut Window, cx: &mut App) -> AnyElement {
        let count = range.len();
        let run_key = range.start.to_string();
        let shown_state: Entity<Range<usize>> = window.use_keyed_state(self.row_id("run-shown", &run_key), cx, |_, _| {
            0..PLAYLIST_INITIAL_ROWS.min(count)
        });
        let shown = shown_state.read(cx).clone();
        let rows: Vec<AnyElement> = range
            .enumerate()
            .map(|(offset, index)| {
                if shown.contains(&offset) {
                    self.row(index, window, cx)
                } else {
                    div().w_full().flex_none().h(px(PLAYLIST_ROW_HEIGHT)).into_any_element()
                }
            })
            .collect();
        let probe = GiftScrollWindowProbe::new(self.row_id("run-probe", &run_key), move |band, _, cx| {
            let Some(band) = band else {
                return;
            };
            let needed = playlist_rows_in(&band, 0., count);
            if needed.is_empty() {
                return;
            }
            shown_state.update(cx, |shown, cx| {
                if shown.start <= needed.start && needed.end <= shown.end {
                    return;
                }
                *shown = playlist_rows_in(&band, PLAYLIST_OVERSCAN, count);
                cx.notify();
            });
        });
        div()
            .relative()
            .w_full()
            .child(vstack(0.).w_full().children(rows))
            .child(div().absolute().inset_0().child(probe))
            .into_any_element()
    }

    fn row(&self, index: usize, window: &mut Window, cx: &mut App) -> AnyElement {
        let Some(row) = self.rows.get(index) else {
            return div().into_any_element();
        };
        let theme = rise_ui::theme(cx).clone();
        match row {
            AddToPlaylistRow::CreateNew => vstack(0.)
                .w_full()
                .child(self.create_new_row(&theme))
                .child(separator(&theme, ROW_SEPARATOR_LEADING))
                .into_any_element(),
            AddToPlaylistRow::Liked => vstack(0.)
                .w_full()
                .child(self.liked_row(&theme, cx))
                .child(separator(&theme, ROW_SEPARATOR_LEADING))
                .into_any_element(),
            AddToPlaylistRow::ServerSkeleton => {
                MusicLibraryListSkeleton::new().rows(4).into_any_element()
            }
            AddToPlaylistRow::LocalSkeleton => {
                MusicLibraryListSkeleton::new().rows(4).into_any_element()
            }
            AddToPlaylistRow::ServerFailure(error) => self.load_failure_state(error, &theme, false).into_any_element(),
            AddToPlaylistRow::LocalFailure(error) => self.load_failure_state(error, &theme, true).into_any_element(),
            AddToPlaylistRow::ServerPlaylist(data) => self.playlist_row_view("server", data, &theme, window, cx),
            AddToPlaylistRow::LocalDestination { data, is_last } => {
                let is_last = *is_last;
                div()
                    .w_full()
                    .child(self.playlist_row_view("local", data, &theme, window, cx))
                    .on_appear(self.row_id("destination-appear", &data.id), move |_, cx| {
                        if is_last {
                            MusicInteractionsStore::shared(cx)
                                .update(cx, |interactions, cx| interactions.load_more_local_playlist_destinations(cx));
                        }
                    })
                    .into_any_element()
            }
            AddToPlaylistRow::LoadingMore => {
                div().w_full().flex().justify_center().py(px(16.)).child(progress_view()).into_any_element()
            }
            AddToPlaylistRow::RetryLoadMore => div()
                .w_full()
                .flex()
                .justify_center()
                .py(px(16.))
                .child(
                    plain(self.row_id("retry", "more"))
                        .on_press(|_, cx| {
                            MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                                interactions.retry_load_more_local_playlist_destinations(cx);
                            });
                        })
                        .text_color(theme.primary._100)
                        .child(tr("retry")),
                )
                .into_any_element(),
            AddToPlaylistRow::Empty => div()
                .w_full()
                .py(px(24.))
                .px(px(16.))
                .child(
                    SecondaryText::new(tr("music_home_playlists_empty"))
                        .px(14.)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary),
                )
                .into_any_element(),
            AddToPlaylistRow::BottomInset => div().w_full().h(px(24.)).into_any_element(),
        }
    }

    fn load_failure_state(&self, error: &str, theme: &AppTheme, local_destinations: bool) -> impl IntoElement {
        vstack(10.)
            .w_full()
            .py(px(24.))
            .px(px(16.))
            .child(SecondaryText::new(error.to_owned()).px(14.).tac(TextAlignT::Center).color(theme.text.secondary))
            .child(
                plain(self.row_id("retry", if local_destinations { "local" } else { "server" }))
                    .on_press(move |_, cx| {
                        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                            if local_destinations {
                                interactions.refresh_local_playlist_destinations(cx);
                            } else {
                                interactions.refresh_playlists(cx);
                            }
                        });
                    })
                    .swiftui_font(Font::system_size(14.).weight(FontWeight::SEMIBOLD))
                    .text_color(theme.primary._100)
                    .child(tr("retry")),
            )
    }

    fn create_new_row(&self, theme: &AppTheme) -> impl IntoElement {
        let state = self.state.clone();
        plain(self.row_id("row", "create"))
            .w_full()
            .on_press(move |_, cx| {
                state.update(cx, |state, cx| {
                    state.show_create_sheet = true;
                    cx.notify();
                });
            })
            .child(
                hstack(14.)
                    .w_full()
                    .px(px(16.))
                    .py(px(10.))
                    .child(
                        div()
                            .flex_none()
                            .size(px(ROW_COVER_SIDE))
                            .continuous_corners(ROW_COVER_RADIUS)
                            .bg(theme.bg._300)
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized("plus", px(22.), theme.primary._100, false)),
                    )
                    .child(
                        MainText::new(tr("music_create_new_playlist"))
                            .px(15.)
                            .font_weight(FontWeight::MEDIUM)
                            .color(theme.primary._100),
                    )
                    .child(spacer_min_w(0.)),
            )
    }

    fn liked_row(&self, theme: &AppTheme, cx: &mut App) -> impl IntoElement {
        let track = self.track.clone();
        let is_liked =
            MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.is_liked(&track, cx));
        let liked_count = MusicServicesStore::shared(cx).update(cx, |services, cx| services.liked_tracks_total_count(cx));
        let track_id = self.track.id.clone();
        plain(self.row_id("row", "liked"))
            .w_full()
            .on_press(move |_, cx| {
                MusicInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.toggle_like(&track_id, cx));
            })
            .child(
                hstack(14.)
                    .w_full()
                    .px(px(16.))
                    .py(px(10.))
                    .child(
                        div()
                            .flex_none()
                            .size(px(ROW_COVER_SIDE))
                            .continuous_corners(ROW_COVER_RADIUS)
                            .overflow_hidden()
                            .bg(linear_gradient(
                                LIKED_COVER_ANGLE,
                                linear_color_stop(rgb(LIKED_COVER_START), 0.),
                                linear_color_stop(rgb(LIKED_COVER_END), 1.),
                            ))
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized("heart.fill", px(22.), white(), true)),
                    )
                    .child(
                        vstack(3.)
                            .align(HorizontalAlignment::Leading)
                            .flex_1()
                            .min_w_0()
                            .child(
                                MainText::new(tr("music_liked_songs"))
                                    .px(15.)
                                    .font_weight(FontWeight::MEDIUM)
                                    .color(theme.text.primary)
                                    .line_limit(1),
                            )
                            .child(SecondaryText::new(tr_plural("music_track_count", liked_count)).px(12.).line_limit(1)),
                    )
                    .child(AddedCheckmark {
                        id: self.row_id("check", "liked"),
                        is_shown: is_liked,
                        color: theme.primary._100,
                    }),
            )
    }

    fn playlist_row_view(
        &self,
        kind: &'static str,
        data: &PlaylistRowData,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let props = PlaylistRowProps {
            id: self.row_id(kind, &data.id),
            data: data.clone(),
            is_added: self.state.read(cx).added_playlist_ids.contains(&data.id),
            state: self.state.clone(),
            track_id: self.track.id.clone(),
            colors: PlaylistRowColors {
                text_primary: theme.text.primary,
                primary: theme.primary._100,
                border: theme.border._100,
            },
        };
        PlaylistRowView::element(self.row_id("row-view", &format!("{kind}-{}", data.id)), props, window, cx)
    }
}

/// rows of a playlist run that meet the band widened by overscan
fn playlist_rows_in(band: &RangeInclusive<f32>, overscan: f32, count: usize) -> Range<usize> {
    let first = ((*band.start() - overscan) / PLAYLIST_ROW_HEIGHT).floor().max(0.) as usize;
    let last = (((*band.end() + overscan) / PLAYLIST_ROW_HEIGHT).ceil().max(0.) as usize).min(count);
    first.min(last)..last
}

/// theme colors a cached row paints with, a theme switch changes them and rerenders it
#[derive(Clone, Copy, PartialEq)]
struct PlaylistRowColors {
    text_primary: Hsla,
    primary: Hsla,
    border: Hsla,
}

#[derive(Clone, PartialEq)]
struct PlaylistRowProps {
    id: ElementId,
    data: PlaylistRowData,
    is_added: bool,
    state: Entity<AddToPlaylistSheetState>,
    track_id: String,
    colors: PlaylistRowColors,
}

struct PlaylistRowView {
    props: Option<PlaylistRowProps>,
}

impl PlaylistRowView {
    fn element(id: ElementId, props: PlaylistRowProps, window: &mut Window, cx: &mut App) -> AnyElement {
        let style = StyleRefinement::default().w_full().h(px(PLAYLIST_ROW_HEIGHT));
        let host = window.use_keyed_state(id, cx, |_, _| Self { props: None });
        let stale = host.update(cx, |host, _| host.set_props(props));
        if stale {
            div().w_full().h(px(PLAYLIST_ROW_HEIGHT)).child(AnyView::from(host)).into_any_element()
        } else {
            host.cached(style).replay_when_moved().into_any_element()
        }
    }

    fn set_props(&mut self, props: PlaylistRowProps) -> bool {
        if self.props.as_ref() == Some(&props) {
            return false;
        }
        self.props = Some(props);
        true
    }
}

impl Render for PlaylistRowView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let row = self.props.clone().map(|props| {
            let colors = props.colors;
            vstack(0.)
                .w_full()
                .child(playlist_row(props))
                .child(div().w_full().flex_none().pl(px(ROW_SEPARATOR_LEADING)).child(
                    div().w_full().h(px(0.5)).bg(colors.border),
                ))
        });
        div().w_full().h(px(PLAYLIST_ROW_HEIGHT)).children(row)
    }
}

fn playlist_row(props: PlaylistRowProps) -> impl IntoElement {
    let PlaylistRowProps { id, data, is_added, state, track_id, colors } = props;
    let child_id = |name: &str| ElementId::NamedChild(Arc::new(id.clone()), SharedString::from(name.to_owned()));
    let cover: AnyElement = match data.cover_url.as_deref().filter(|url| !url.is_empty()) {
        Some(url) => MusicCoverImage::new(child_id("cover"), url.to_owned())
            .size(ROW_COVER_SIDE)
            .corner_radius(ROW_COVER_RADIUS)
            .into_any_element(),
        None => MusicCoverPlaceholder::new()
            .icon_scale(0.34)
            .corner_radius(ROW_COVER_RADIUS)
            .into_any_element(),
    };
    let check_id = child_id("check");
    let playlist_id = data.id.clone();
    plain(child_id("row"))
        .w_full()
        .opacity(if is_added { 0.6 } else { 1. })
        .on_press(move |_, cx| {
            if is_added {
                return;
            }
            state.update(cx, |state, cx| state.add_track(&playlist_id, &track_id, cx));
        })
        .child(
            hstack(14.)
                .w_full()
                .px(px(16.))
                .py(px(10.))
                .child(
                    div()
                        .flex_none()
                        .size(px(ROW_COVER_SIDE))
                        .continuous_corners(ROW_COVER_RADIUS)
                        .overflow_hidden()
                        .child(cover),
                )
                .child(
                    vstack(3.)
                        .align(HorizontalAlignment::Leading)
                        .flex_1()
                        .min_w_0()
                        .child(
                            MainText::new(data.title.clone())
                                .px(15.)
                                .font_weight(FontWeight::MEDIUM)
                                .color(colors.text_primary)
                                .line_limit(1),
                        )
                        .child(SecondaryText::new(data.subtitle.clone()).px(12.).line_limit(1)),
                )
                .child(AddedCheckmark { id: check_id, is_shown: is_added, color: colors.primary }),
        )
}

fn separator(theme: &AppTheme, leading: f32) -> impl IntoElement {
    div().w_full().flex_none().pl(px(leading)).child(div().w_full().h(px(0.5)).bg(theme.border._100))
}

/// checkmark that pops in under the row spring
#[derive(IntoElement)]
struct AddedCheckmark {
    id: ElementId,
    is_shown: bool,
    color: Hsla,
}

impl RenderOnce for AddedCheckmark {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let shown = self.is_shown.then_some(());
        let presence = window.use_keyed_state(self.id.clone(), cx, |_, _| {
            Presence::new(shown).with_transition(Transition::scale_default().combined(Transition::opacity()))
        });
        let now = RiseClock::monotonic();
        let spec = spring(CHECKMARK_SPRING_RESPONSE, CHECKMARK_SPRING_DAMPING);
        presence.update(cx, |presence, cx| presence.set_in(shown, spec, now, cx));
        let color = self.color;
        div().flex_none().children(presence.read(cx).render_transitioned(now, window, |_| {
            div().flex_none().children(IconUi::sized("checkmark.circle.fill", px(22.), color, true))
        }))
    }
}
