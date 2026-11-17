use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, ElementId, Entity, FontWeight, Hsla, SharedString, Subscription, Window, list, rgb};
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_pagination_prefetch::MusicPaginationPrefetchModifiers;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::{
    ProtoTrackRecordExt, RiseStoredMusicTrack, WireAlbumDTO,
};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::bottomsheets::studio::publish_schedule_sheet::{
    PublishScheduleSheet, PublishScheduleSheetLayout,
};
use crate::modules::music::widgets::bottomsheets::studio::upload_track_sheet::UploadTrackSheet;
use crate::modules::music::widgets::bottomsheets::track_picker::track_picker_shared::TrackPickerListModel;
use crate::modules::music::widgets::music_mini_player_bottom_padding::MusicMiniPlayerBottomPaddingModifiers;
use crate::modules::music::widgets::music_skeletons::TrackListSkeleton;
use crate::modules::music::widgets::music_track_cover_image::{MusicCoverImage, MusicTrackCoverImage};

const TITLE_PLACEHOLDER: &str = "\u{2014}";
const UPLOAD_TRACK_SHEET_FRACTION: f32 = 0.93;
const TRACK_PREFETCH_THRESHOLD: usize = 8;
const PUBLISHED_COLOR: u32 = 0x00B894;
const PENDING_COLOR: u32 = 0xFDCB6E;

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

/// state of the screen, the music store and the column
struct StudioAlbumDetailState {
    show_upload_track: bool,
    show_publish_sheet: bool,
    publish_sheet_fraction: f32,
    is_publishing: bool,
    list_model: TrackPickerListModel,
    _subscriptions: [Subscription; 2],
}

impl StudioAlbumDetailState {
    fn new(cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let services = MusicServicesStore::shared(cx);
        Self {
            show_upload_track: false,
            show_publish_sheet: false,
            publish_sheet_fraction: PublishScheduleSheetLayout::RESTING_FRACTION,
            is_publishing: false,
            list_model: TrackPickerListModel::new(cx),
            _subscriptions: [
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
            ],
        }
    }
}

#[derive(IntoElement)]
pub struct StudioAlbumDetail {
    id: ElementId,
    album_id: String,
}

impl StudioAlbumDetail {
    pub fn new(album_id: impl Into<String>) -> Self {
        let album_id = album_id.into();
        Self { id: ElementId::Name(SharedString::from(format!("studio-album-detail-{album_id}"))), album_id }
    }

    fn album_wire(store: &MusicActionsStore, album_id: &str) -> Option<WireAlbumDTO> {
        store.list_my_albums_sai.data().and_then(|data| data.albums.into_iter().find(|album| album.id == album_id))
    }

    fn album_tracks(store: &MusicActionsStore, album_id: &str) -> Vec<RiseStoredMusicTrack> {
        store
            .list_my_tracks_sai
            .data()
            .map(|data| data.tracks)
            .unwrap_or_default()
            .iter()
            .map(ProtoTrackRecordExt::stored)
            .filter(|track| track.album.as_ref().map_or("", |album| album.id.as_str()) == album_id)
            .collect()
    }

    fn publish_action(
        album_id: &str,
        state: &Entity<StudioAlbumDetailState>,
        scheduled_at_ms: Option<i64>,
        cx: &mut App,
    ) {
        let has_publishable_tracks = {
            let actions = MusicActionsStore::shared(cx);
            !Self::album_tracks(actions.read(cx), album_id).is_empty()
        };
        if state.read(cx).is_publishing || !has_publishable_tracks {
            return;
        }
        state.update(cx, |state, cx| {
            state.is_publishing = true;
            cx.notify();
        });
        let success_state = state.clone();
        let on_success: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            success_state.update(cx, |state, cx| {
                state.is_publishing = false;
                cx.notify();
            });
        });
        let error_state = state.clone();
        let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(move |message: String, cx: &mut App| {
            error_state.update(cx, |state, cx| {
                state.is_publishing = false;
                cx.notify();
            });
            NativeConfirmAlert::present_notice(
                &tr("music_upload_error_title"),
                &message,
                None,
                None,
                |_: &mut App| {},
                cx,
            );
        });
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.publish_approved_album(album_id, scheduled_at_ms, Some(on_success), Some(on_error), cx);
        });
    }

    fn right_jsx(
        &self,
        state: &Entity<StudioAlbumDetailState>,
        is_approved: bool,
        has_publishable_tracks: bool,
        is_publishing: bool,
        theme: &AppTheme,
    ) -> AnyElement {
        let action_state = state.clone();
        if is_approved {
            plain(named_child(&self.id, "publish"))
                .disabled(is_publishing || !has_publishable_tracks)
                .on_press(move |_, cx| {
                    action_state.update(cx, |state, cx| {
                        state.show_publish_sheet = true;
                        cx.notify();
                    });
                })
                .child(
                    MainText::new(tr("music_publish"))
                        .px(14.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(if has_publishable_tracks { theme.primary._100 } else { theme.text.secondary }),
                )
                .liquid_glass_nav_pill(10.0, None)
        } else {
            PageHeaderIconButton::new(named_child(&self.id, "add-track"), "plus", move |_, cx| {
                action_state.update(cx, |state, cx| {
                    state.show_upload_track = true;
                    cx.notify();
                });
            })
            .into_any_element()
        }
    }

    fn upload_track_sheet(&self, state: &Entity<StudioAlbumDetailState>) -> BottomSheetUi {
        let sheet_id = named_child(&self.id, "upload-track");
        let album_id = self.album_id.clone();
        let dismiss = state.clone();
        BottomSheetUi::new(
            named_child(&self.id, "upload-track-sheet"),
            Binding::entity(
                state.clone(),
                |state: &StudioAlbumDetailState| state.show_upload_track,
                |state, value, _| state.show_upload_track = value,
            ),
            move |_, _| {
                let dismiss = dismiss.clone();
                UploadTrackSheet::new(sheet_id.clone()).preselected_album_id(Some(album_id.clone())).on_dismiss(
                    move |cx| {
                        dismiss.update(cx, |state, cx| {
                            state.show_upload_track = false;
                            cx.notify();
                        });
                    },
                )
            },
            |_, _| div(),
        )
        .snap_fractions(vec![UPLOAD_TRACK_SHEET_FRACTION])
    }

    fn publish_sheet(&self, state: &Entity<StudioAlbumDetailState>, theme: &AppTheme) -> BottomSheetUi {
        let sheet_id = named_child(&self.id, "publish-schedule");
        let album_id = self.album_id.clone();
        let content_state = state.clone();
        let fraction = Binding::entity(
            state.clone(),
            |state: &StudioAlbumDetailState| state.publish_sheet_fraction,
            |state, value, _| state.publish_sheet_fraction = value,
        );
        let sheet_fraction = fraction.clone();
        BottomSheetUi::new(
            named_child(&self.id, "publish-sheet"),
            Binding::entity(
                state.clone(),
                |state: &StudioAlbumDetailState| state.show_publish_sheet,
                |state, value, _| state.show_publish_sheet = value,
            ),
            move |_, _| {
                let (now_id, now_state) = (album_id.clone(), content_state.clone());
                let (later_id, later_state) = (album_id.clone(), content_state.clone());
                let dismiss = content_state.clone();
                PublishScheduleSheet::new(
                    sheet_id.clone(),
                    move |_, cx| Self::publish_action(&now_id, &now_state, None, cx),
                    move |date, _, cx| {
                        let scheduled_at_ms = (date.time_interval_since_1970() * 1000.0) as i64;
                        Self::publish_action(&later_id, &later_state, Some(scheduled_at_ms), cx);
                    },
                )
                .on_dismiss(move |_, cx| {
                    dismiss.update(cx, |state, cx| {
                        state.show_publish_sheet = false;
                        cx.notify();
                    });
                })
                .sheet_fraction(sheet_fraction.clone())
            },
            |_, _| div(),
        )
        .snap_fractions(PublishScheduleSheetLayout::SNAP_FRACTIONS.to_vec())
        .modal_sized_by_detent(true)
        .selected_snap_fraction(Some(fraction))
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .sheet_background(Some(theme.bg._100))
    }
}

impl RenderOnce for StudioAlbumDetail {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(named_child(&self.id, "state"), cx, |_, cx| StudioAlbumDetailState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let theme = rise_ui::theme(cx).clone();
        let actions = MusicActionsStore::shared(cx);
        let (album_wire, has_publishable_tracks) = {
            let store = actions.read(cx);
            (Self::album_wire(store, &self.album_id), !Self::album_tracks(store, &self.album_id).is_empty())
        };
        let album_status = album_wire.as_ref().map(|album| album.status.clone()).unwrap_or_default();
        let is_approved = album_status == RiseMusicTrackStatus::Approved.wire();
        let is_publishing = state.read(cx).is_publishing;
        let title = album_wire.as_ref().map_or_else(|| tr("music_studio_albums"), |album| album.title.clone());

        div()
            .relative()
            .size_full()
            .child(
                SettingsWrapperUi::new(title, move |_, cx| {
                    if let Some(nav_store) = &nav_store {
                        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                    }
                })
                .right_jsx(self.right_jsx(&state, is_approved, has_publishable_tracks, is_publishing, &theme))
                .content(StudioAlbumDetailBody {
                    id: named_child(&self.id, "body"),
                    album_id: self.album_id.clone(),
                    album: album_wire,
                    album_status,
                    is_approved,
                    state: state.clone(),
                }),
            )
            .child(self.upload_track_sheet(&state))
            .child(self.publish_sheet(&state, &theme))
            .on_appear(named_child(&self.id, "appear"), |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_my_albums_if_needed(cx);
                    store.load_my_tracks_if_needed(cx);
                });
            })
    }
}

/// one entry of the column, gpui lays out a long column only through list
#[derive(Clone, Copy)]
enum StudioAlbumDetailRow {
    Header,
    Skeleton,
    Empty,
    Track(usize),
    Foot,
}

#[derive(IntoElement)]
struct StudioAlbumDetailBody {
    id: ElementId,
    album_id: String,
    album: Option<WireAlbumDTO>,
    album_status: String,
    is_approved: bool,
    state: Entity<StudioAlbumDetailState>,
}

impl RenderOnce for StudioAlbumDetailBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let top_inset = list_shell_floating_top_inset(cx);
        let theme = rise_ui::theme(cx).clone();
        let actions = MusicActionsStore::shared(cx);
        let (visible_album_tracks, is_pending) = {
            let store = actions.read(cx);
            let visible: Vec<(RiseStoredMusicTrack, TrackModel)> = StudioAlbumDetail::album_tracks(store, &self.album_id)
                .into_iter()
                .filter_map(|wire| wire.as_track_model().map(|track| (wire, track)))
                .collect();
            (visible, store.list_my_tracks_sai.is_pending())
        };

        let mut rows = vec![StudioAlbumDetailRow::Header];
        if is_pending && visible_album_tracks.is_empty() {
            rows.push(StudioAlbumDetailRow::Skeleton);
        } else if visible_album_tracks.is_empty() {
            rows.push(StudioAlbumDetailRow::Empty);
        } else {
            rows.extend((0..visible_album_tracks.len()).map(StudioAlbumDetailRow::Track));
        }
        rows.push(StudioAlbumDetailRow::Foot);

        let ids: Vec<SharedString> = rows
            .iter()
            .map(|row| match row {
                StudioAlbumDetailRow::Header => SharedString::new_static("header"),
                StudioAlbumDetailRow::Skeleton => SharedString::new_static("skeleton"),
                StudioAlbumDetailRow::Empty => SharedString::new_static("empty"),
                StudioAlbumDetailRow::Track(index) => SharedString::from(visible_album_tracks[*index].1.id.clone()),
                StudioAlbumDetailRow::Foot => SharedString::new_static("foot"),
            })
            .collect();
        let list_state = self.state.update(cx, |state, _| {
            state.list_model.sync(ids);
            state.list_model.list().state().clone()
        });

        let id = self.id;
        let album = self.album;
        let album_status = self.album_status;
        let is_approved = self.is_approved;
        let cache_key = MusicServicesStore::my_tracks_cache_key();
        let background = theme.bg._100;
        div().size_full().bg(background).child(
            list(list_state, move |index, _, cx| match rows.get(index) {
                Some(StudioAlbumDetailRow::Header) => vstack(0.0)
                    .w_full()
                    .pt(px(top_inset))
                    .children(album.as_ref().map(|album| {
                        header_section(&id, album, &album_status, is_approved, &theme)
                    }))
                    .into_any_element(),
                Some(StudioAlbumDetailRow::Skeleton) => div()
                    .w_full()
                    .pt(px(8.0))
                    .child(TrackListSkeleton::new().rows(6).horizontal_padding(16.0))
                    .into_any_element(),
                Some(StudioAlbumDetailRow::Empty) => vstack(16.0)
                    .w_full()
                    .pt(px(60.0))
                    .children(IconUi::sized("music.note", px(48.0), theme.text.secondary.opacity(0.4), false))
                    .child(
                        MainText::new(tr("music_studio_no_tracks"))
                            .px(16.0)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary),
                    )
                    .into_any_element(),
                Some(StudioAlbumDetailRow::Track(position)) => {
                    let (wire, track) = &visible_album_tracks[*position];
                    vstack(0.0)
                        .w_full()
                        .child(track_row(&id, wire, track, &theme).music_pagination_prefetch(
                            named_child(&id, format!("track-prefetch-{}", track.id)),
                            *position,
                            visible_album_tracks.len(),
                            &cache_key,
                            true,
                            TRACK_PREFETCH_THRESHOLD,
                            |_, cx| {
                                MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_my_tracks(cx));
                            },
                            cx,
                        ))
                        .child(div().w_full().pl(px(78.0)).child(div().w_full().h(px(0.5)).bg(theme.border._100)))
                        .into_any_element()
                }
                Some(StudioAlbumDetailRow::Foot) => div()
                    .w_full()
                    .music_mini_player_bottom_padding(40.0, 0.0)
                    .music_pagination_drain_filtered_empty(
                        named_child(&id, "drain"),
                        visible_album_tracks.len(),
                        &cache_key,
                        true,
                        |_, cx| {
                            MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_my_tracks(cx));
                        },
                        cx,
                    )
                    .into_any_element(),
                None => div().into_any_element(),
            })
            .size_full(),
        )
    }
}

fn header_section(
    id: &ElementId,
    album: &WireAlbumDTO,
    album_status: &str,
    is_approved: bool,
    theme: &AppTheme,
) -> Div {
    let title = if album.title.is_empty() { TITLE_PLACEHOLDER.to_owned() } else { album.title.clone() };
    let cover_url = RemoteMediaURL::optional_normalized(Some(album.cover_url.as_str())).unwrap_or_default();
    vstack(12.0)
        .w_full()
        .px(px(16.0))
        .pt(px(16.0))
        .pb(px(8.0))
        .child(MusicCoverImage::new(named_child(id, "cover"), cover_url).size(140.0).corner_radius(16.0))
        .child(MainText::new(title).px(20.0).tac(TextAlignT::Center).font_weight(FontWeight::BOLD))
        .children(status_badge(album_status, theme))
        .when(is_approved, |column| {
            column.child(
                hstack(6.0)
                    .children(IconUi::sized("info.circle", px(12.0), theme.primary._100, false))
                    .child(SecondaryText::new(tr("music_approved_ready_to_publish")).px(12.0).color(theme.primary._100)),
            )
        })
        .child(div().w_full().h(px(0.5)).bg(theme.border._100))
}

fn track_row(id: &ElementId, wire: &RiseStoredMusicTrack, track: &TrackModel, theme: &AppTheme) -> Div {
    hstack(14.0)
        .w_full()
        .px(px(16.0))
        .py(px(12.0))
        .child(
            MusicTrackCoverImage::new(named_child(id, format!("cover-{}", track.id)), track.cover_image_url.clone())
                .size(48.0)
                .corner_radius(8.0),
        )
        .child(
            vstack(2.0)
                .flex_1()
                .min_w_0()
                .align(HorizontalAlignment::Leading)
                .child(
                    MainText::new(track.title.clone())
                        .px(15.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.primary)
                        .line_limit(1),
                )
                .child(
                    hstack(6.0)
                        .child(SecondaryText::new(track.formatted_duration()).px(12.0))
                        .children(status_badge(wire.status.as_deref().unwrap_or_default(), theme)),
                ),
        )
}

fn status_badge(status: &str, theme: &AppTheme) -> Option<Div> {
    if status.is_empty() {
        return None;
    }
    let color = status_color(status, theme);
    Some(
        div()
            .px(px(8.0))
            .py(px(3.0))
            .capsule()
            .bg(color.opacity(0.12))
            .whitespace_nowrap()
            .swiftui_font(Font::system_weight(11.0, FontWeight::SEMIBOLD))
            .text_color(color)
            .child(tr(&format!("music_status_{status}"))),
    )
}

fn status_color(status: &str, theme: &AppTheme) -> Hsla {
    if status == RiseMusicTrackStatus::Published.wire() {
        rgb(PUBLISHED_COLOR).into()
    } else if status == RiseMusicTrackStatus::Approved.wire() {
        theme.primary._100
    } else if status == RiseMusicTrackStatus::Rejected.wire() {
        theme.semantic.error_100
    } else {
        rgb(PENDING_COLOR).into()
    }
}
