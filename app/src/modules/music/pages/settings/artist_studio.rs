use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, ElementId, Entity, FontWeight, Hsla, SharedString, Subscription, Window, list, rgb};
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_i18n::number_format::decimal_number_formatter::DecimalNumberFormatter;
use rise_i18n::{I18n, tr};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::animated_tabs_ui::animated_tabs_types::{AnimatedTabsHaptics, AnimatedTabsMetrics, TabHeaderConfig};
use rise_ui::context_menu_ui::{ContextMenuItem, ContextMenuUi};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::PageHeaderToolbarSlot;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::ui::animated_tabs_ui::animated_tabs_header::AnimatedTabsLiveHeader;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::{AnimatedTabsPager, AnimatedTabsPagerLiveState};
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, ListShellUi, list_shell_floating_top_inset,
};
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::music::pages::track::track_context_menu_row::{TrackContextMenuRow, TrackContextSource};
use crate::modules::music::shared::utils::music::music_models::{TrackModel, display_genre};
use crate::modules::music::shared::utils::music::music_pagination_prefetch::MusicPaginationPrefetchModifiers;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_types::ArtistAnalyticsModel;
use crate::modules::music::stores::music_actions::music_wire_types::{
    ProtoTrackRecordExt, WireAlbumDTO, WireArtistAnalyticsDTOExt,
};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::bottomsheets::studio::create_album_sheet::CreateAlbumSheet;
use crate::modules::music::widgets::bottomsheets::studio::publish_schedule_sheet::{
    PublishScheduleSheet, PublishScheduleSheetLayout,
};
use crate::modules::music::widgets::bottomsheets::studio::upload_track_sheet::UploadTrackSheet;
use crate::modules::music::widgets::bottomsheets::track_picker::track_picker_shared::TrackPickerListModel;
use crate::modules::music::widgets::music_mini_player_bottom_padding::MusicMiniPlayerBottomPaddingModifiers;
use crate::modules::music::widgets::music_skeletons::{MusicLibraryListSkeleton, TrackListSkeleton};
use crate::modules::music::widgets::music_track_cover_image::MusicCoverImage;
use crate::modules::post::components::moderation_hidden_banner::ModerationHiddenBanner;

const TITLE_PLACEHOLDER: &str = "\u{2014}";
const TAB_TRACKS: i64 = 0;
const TAB_ALBUMS: i64 = 1;
const TAB_ANALYTICS: i64 = 2;
const TABS_COUNT: i64 = 3;
const TRACK_PREFETCH_THRESHOLD: usize = 8;
const UPLOAD_TRACK_SHEET_FRACTION: f32 = 0.93;
const CREATE_ALBUM_SHEET_FRACTION: f32 = 0.85;
const LISTENERS_COLOR: u32 = 0x00B894;
const LIKES_COLOR: u32 = 0xE84393;
const FOLLOWERS_COLOR: u32 = 0xFDCB6E;
const RETENTION_WEEK_COLOR: u32 = 0x6C5CE7;
const RETENTION_MONTH_COLOR: u32 = 0x0984E3;
const PENDING_STATUS_COLOR: u32 = 0xFDCB6E;

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

fn hairline(leading: f32, color: Hsla) -> Div {
    div().w_full().pl(px(leading)).child(div().w_full().h(px(0.5)).bg(color))
}

/// int with thousands separators
fn formatted(value: i64) -> String {
    DecimalNumberFormatter::new(I18n::shared().code()).string(value.max(0) as u64)
}

fn is_approved_artist(cx: &mut App) -> bool {
    MusicServicesStore::shared(cx).update(cx, |service, cx| service.music_is_approved_artist(cx))
}

/// state of the screen
/// drag progress and tap suppression live in pager_live_state, a write
/// per swipe frame would re-read the track and album lists
struct ArtistStudioState {
    selected_tab: i64,
    page_width: f32,
    pager_live_state: Entity<AnimatedTabsPagerLiveState>,
    show_upload_track: bool,
    show_create_album: bool,
    show_publish_sheet: bool,
    publish_sheet_fraction: f32,
    album_to_publish: Option<String>,
    tracks_list: TrackPickerListModel,
    albums_list: TrackPickerListModel,
    _subscriptions: [Subscription; 2],
}

impl ArtistStudioState {
    fn new(cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let services = MusicServicesStore::shared(cx);
        Self {
            selected_tab: TAB_TRACKS,
            page_width: 0.0,
            pager_live_state: cx.new(|_| AnimatedTabsPagerLiveState::new()),
            show_upload_track: false,
            show_create_album: false,
            show_publish_sheet: false,
            publish_sheet_fraction: PublishScheduleSheetLayout::RESTING_FRACTION,
            album_to_publish: None,
            tracks_list: TrackPickerListModel::new(cx),
            albums_list: TrackPickerListModel::new(cx),
            _subscriptions: [
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn select_tab(&mut self, id: i64, cx: &mut Context<Self>) {
        if id == self.selected_tab {
            return;
        }
        AnimatedTabsHaptics::selection_changed();
        self.selected_tab = id;
        cx.notify();
    }
}

#[derive(IntoElement)]
pub struct ArtistStudio {
    id: ElementId,
}

impl Default for ArtistStudio {
    fn default() -> Self {
        Self::new()
    }
}

impl ArtistStudio {
    pub fn new() -> Self {
        Self { id: ElementId::Name(SharedString::new_static("artist-studio")) }
    }

    fn enforce_artist_access(
        state: &Entity<ArtistStudioState>,
        nav_store: &Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) {
        let status = MusicActionsStore::slices(cx).get_artist_status_sai.status();
        if status != SaiWsSliceStatus::Fulfilled || is_approved_artist(cx) {
            return;
        }
        state.update(cx, |state, cx| {
            state.show_upload_track = false;
            state.show_create_album = false;
            cx.notify();
        });
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }

    fn publish_album_from_sheet(state: &Entity<ArtistStudioState>, scheduled_at_ms: Option<i64>, cx: &mut App) {
        let album_id = state.update(cx, |state, _| state.album_to_publish.take());
        let Some(album_id) = album_id.filter(|album_id| !album_id.is_empty()) else {
            return;
        };
        let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(|message: String, cx: &mut App| {
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
            store.publish_approved_album(&album_id, scheduled_at_ms, None, Some(on_error), cx);
        });
    }

    fn on_tab_changed(tab: i64, cx: &mut App) {
        if !is_approved_artist(cx) {
            return;
        }
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| match tab {
            TAB_TRACKS => store.load_my_tracks_if_needed(cx),
            TAB_ALBUMS => store.refresh_my_albums(cx),
            _ => store.load_analytics_if_needed(cx),
        });
    }

    fn add_menu(&self, state: &Entity<ArtistStudioState>, theme: &AppTheme) -> ContextMenuUi {
        let upload = state.clone();
        let create = state.clone();
        ContextMenuUi::new(
            named_child(&self.id, "add-menu"),
            vec![
                ContextMenuItem::new(0, tr("music_upload_track"), None, "upload-track", move |_, cx| {
                    upload.update(cx, |state, cx| {
                        state.show_upload_track = true;
                        cx.notify();
                    });
                }),
                ContextMenuItem::new(1, tr("music_create_album"), None, "create-album", move |_, cx| {
                    create.update(cx, |state, cx| {
                        state.show_create_album = true;
                        cx.notify();
                    });
                }),
            ],
            PageHeaderToolbarSlot::new(div().children(IconUi::sized("plus", px(17.0), theme.text.primary, false))),
        )
    }

    fn below_header(&self, state: &Entity<ArtistStudioState>, active_tab: Binding<i64>, cx: &App) -> Div {
        let live_state = state.read(cx).pager_live_state.clone();
        let tap_state = state.clone();
        div().w_full().px(px(10.0)).py(px(4.0)).child(
            div().w_full().h(AnimatedTabsMetrics::BAR_HEIGHT).overflow_hidden().child(
                AnimatedTabsLiveHeader::new(
                    named_child(&self.id, "tabs"),
                    vec![
                        TabHeaderConfig::new(TAB_TRACKS, tr("music_studio_tracks")),
                        TabHeaderConfig::new(TAB_ALBUMS, tr("music_studio_albums")),
                        TabHeaderConfig::new(TAB_ANALYTICS, tr("music_studio_analytics")),
                    ],
                    active_tab,
                    live_state,
                    Rc::new(move |id, _, cx| {
                        tap_state.update(cx, |state, cx| state.select_tab(id, cx));
                    }),
                )
                .horizontal_padding(0.0)
                .force_scrollable(true),
            ),
        )
    }

    fn pager(
        &self,
        state: &Entity<ArtistStudioState>,
        active_tab: Binding<i64>,
        nav_store: &Option<Entity<NavigationStore>>,
        cx: &App,
    ) -> AnimatedTabsPager {
        let live_state = state.read(cx).pager_live_state.clone();
        let width_reader = state.clone();
        let width_writer = state.clone();
        let page_width = Binding::new(
            move |cx: &App| width_reader.read(cx).page_width,
            move |width, _, cx| {
                width_writer.update(cx, |state, _| state.page_width = width);
            },
        );
        let suppressed_state = live_state.clone();
        AnimatedTabsPager::new(
            named_child(&self.id, "pager"),
            TABS_COUNT,
            active_tab,
            Binding::constant(0.0),
            page_width,
        )
        .live_drag_state(Some(live_state))
        .yields_first_tab_back_swipe_to_navigation(true)
        .on_siblings_tap_suppressed(Some(Rc::new(move |suppressed, _, cx| {
            suppressed_state.update(cx, |live, cx| {
                live.set_tab_taps_suppressed(suppressed, AnimatedTabsMetrics::TAB_TAP_RESTORE_DELAY, cx);
            });
        })))
        .child(div().id(named_child(&self.id, "page-tracks")).size_full().child(ArtistStudioTracksPage {
            id: named_child(&self.id, "tracks"),
            state: state.clone(),
            nav_store: nav_store.clone(),
        }))
        .child(div().id(named_child(&self.id, "page-albums")).size_full().child(ArtistStudioAlbumsPage {
            id: named_child(&self.id, "albums"),
            state: state.clone(),
            nav_store: nav_store.clone(),
        }))
        .child(
            div()
                .id(named_child(&self.id, "page-analytics"))
                .size_full()
                .child(ArtistStudioAnalyticsPage { id: named_child(&self.id, "analytics") }),
        )
    }

    fn upload_track_sheet(&self, state: &Entity<ArtistStudioState>) -> BottomSheetUi {
        let sheet_id = named_child(&self.id, "upload-track");
        let dismiss = state.clone();
        BottomSheetUi::new(
            named_child(&self.id, "upload-track-sheet"),
            Binding::entity(
                state.clone(),
                |state: &ArtistStudioState| state.show_upload_track,
                |state, value, _| state.show_upload_track = value,
            ),
            move |_, _| {
                let dismiss = dismiss.clone();
                UploadTrackSheet::new(sheet_id.clone()).on_dismiss(move |cx| {
                    dismiss.update(cx, |state, cx| {
                        state.show_upload_track = false;
                        cx.notify();
                    });
                })
            },
            |_, _| div(),
        )
        .snap_fractions(vec![UPLOAD_TRACK_SHEET_FRACTION])
    }

    fn create_album_sheet(&self, state: &Entity<ArtistStudioState>) -> BottomSheetUi {
        let sheet_id = named_child(&self.id, "create-album");
        let dismiss = state.clone();
        BottomSheetUi::new(
            named_child(&self.id, "create-album-sheet"),
            Binding::entity(
                state.clone(),
                |state: &ArtistStudioState| state.show_create_album,
                |state, value, _| state.show_create_album = value,
            ),
            move |_, _| {
                let dismiss = dismiss.clone();
                CreateAlbumSheet::new(sheet_id.clone()).on_dismiss(move |cx| {
                    dismiss.update(cx, |state, cx| {
                        state.show_create_album = false;
                        cx.notify();
                    });
                })
            },
            |_, _| div(),
        )
        .snap_fractions(vec![CREATE_ALBUM_SHEET_FRACTION])
    }

    fn publish_sheet(&self, state: &Entity<ArtistStudioState>, theme: &AppTheme) -> BottomSheetUi {
        let sheet_id = named_child(&self.id, "publish");
        let content_state = state.clone();
        let fraction = Binding::entity(
            state.clone(),
            |state: &ArtistStudioState| state.publish_sheet_fraction,
            |state, value, _| state.publish_sheet_fraction = value,
        );
        let sheet_fraction = fraction.clone();
        BottomSheetUi::new(
            named_child(&self.id, "publish-sheet"),
            Binding::entity(
                state.clone(),
                |state: &ArtistStudioState| state.show_publish_sheet,
                |state, value, _| state.show_publish_sheet = value,
            ),
            move |_, _| {
                let publish_now = content_state.clone();
                let schedule = content_state.clone();
                let dismiss = content_state.clone();
                PublishScheduleSheet::new(
                    sheet_id.clone(),
                    move |_, cx| Self::publish_album_from_sheet(&publish_now, None, cx),
                    move |date, _, cx| {
                        let scheduled_at_ms = (date.time_interval_since_1970() * 1000.0) as i64;
                        Self::publish_album_from_sheet(&schedule, Some(scheduled_at_ms), cx);
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

impl RenderOnce for ArtistStudio {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(named_child(&self.id, "state"), cx, |_, cx| ArtistStudioState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let theme = rise_ui::theme(cx).clone();
        let is_approved = is_approved_artist(cx);
        let artist_status = MusicActionsStore::slices(cx).get_artist_status_sai.data().map(|data| data.status);
        let selected_tab = state.read(cx).selected_tab;

        let status_state = state.clone();
        let status_nav = nav_store.clone();
        on_change(named_child(&self.id, "artist-status"), artist_status, false, window, cx, move |_, _, _, cx| {
            Self::enforce_artist_access(&status_state, &status_nav, cx);
        });
        on_change(named_child(&self.id, "selected-tab"), selected_tab, false, window, cx, |_, tab, _, cx| {
            Self::on_tab_changed(*tab, cx);
        });

        let active_reader = state.clone();
        let active_writer = state.clone();
        let active_tab = Binding::new(
            move |cx: &App| active_reader.read(cx).selected_tab,
            move |value: i64, _, cx| {
                active_writer.update(cx, |state, cx| state.select_tab(value, cx));
            },
        );

        let back_store = nav_store.clone();
        let mut shell = ListShellUi::new()
            .title(SharedString::from(tr("music_artist_studio")))
            .shows_back_button(true)
            .on_back(move |_, cx| {
                if let Some(nav_store) = &back_store {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            })
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()))
            .below_header(self.below_header(&state, active_tab.clone(), cx))
            .content(div().size_full().child(self.pager(&state, active_tab, &nav_store, cx)));
        if is_approved {
            shell = shell.right_jsx(self.add_menu(&state, &theme));
        }

        let appear_state = state.clone();
        let appear_nav = nav_store;
        div()
            .relative()
            .size_full()
            .child(shell)
            .child(self.upload_track_sheet(&state))
            .child(self.create_album_sheet(&state))
            .child(self.publish_sheet(&state, &theme))
            .on_appear(named_child(&self.id, "appear"), move |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_artist_status_if_needed(cx));
                Self::enforce_artist_access(&appear_state, &appear_nav, cx);
                if !is_approved_artist(cx) {
                    return;
                }
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_my_tracks_if_needed(cx);
                    store.refresh_my_albums(cx);
                    store.load_my_album_requests_if_needed(cx);
                    store.load_analytics_if_needed(cx);
                });
            })
    }
}

fn studio_status_badge(status: &str, theme: &AppTheme) -> Option<Div> {
    if status.is_empty() || status == RiseMusicTrackStatus::Published.wire() {
        return None;
    }
    let color: Hsla = if status == RiseMusicTrackStatus::Approved.wire() {
        theme.primary._100
    } else if status == RiseMusicTrackStatus::Rejected.wire() || status == RiseMusicTrackStatus::ModerationDisabled.wire() {
        theme.semantic.error_100
    } else {
        rgb(PENDING_STATUS_COLOR).into()
    };
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

fn empty_state(icon: &'static str, title: String, theme: &AppTheme) -> Div {
    vstack(16.0)
        .w_full()
        .pt(px(80.0))
        .children(IconUi::sized(icon, px(48.0), theme.text.secondary.opacity(0.4), false))
        .child(MainText::new(title).px(16.0).tac(TextAlignT::Center).color(theme.text.secondary))
}

/// one entry of the tracks column, gpui lays out a long column only through list
#[derive(Clone, Copy)]
enum StudioTracksRow {
    TopInset,
    Skeleton,
    Empty,
    Track(usize),
    Foot,
}

/// a row of the raw tracks page that asTrackModel accepted
/// index stays the position in the raw page which is what the prefetch counts
struct StudioTrackEntry {
    index: usize,
    track: TrackModel,
    status: String,
}

#[derive(IntoElement)]
struct ArtistStudioTracksPage {
    id: ElementId,
    state: Entity<ArtistStudioState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl RenderOnce for ArtistStudioTracksPage {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let top_inset = list_shell_floating_top_inset(cx);
        let theme = rise_ui::theme(cx).clone();
        let actions = MusicActionsStore::shared(cx);
        let (raw, is_pending) = {
            let store = actions.read(cx);
            (
                store.list_my_tracks_sai.data().map(|data| data.tracks).unwrap_or_default(),
                store.list_my_tracks_sai.is_pending(),
            )
        };
        let raw_count = raw.len();
        let entries: Vec<StudioTrackEntry> = raw
            .iter()
            .enumerate()
            .filter_map(|(index, wire)| {
                wire.as_track_model().map(|track| StudioTrackEntry { index, track, status: wire.stored().status.unwrap_or_default() })
            })
            .collect();

        let mut rows = vec![StudioTracksRow::TopInset];
        if is_pending && raw.is_empty() {
            rows.push(StudioTracksRow::Skeleton);
        } else if raw.is_empty() {
            rows.push(StudioTracksRow::Empty);
        } else {
            rows.extend((0..entries.len()).map(StudioTracksRow::Track));
        }
        rows.push(StudioTracksRow::Foot);

        let ids: Vec<SharedString> = rows
            .iter()
            .map(|row| match row {
                StudioTracksRow::TopInset => SharedString::new_static("top-inset"),
                StudioTracksRow::Skeleton => SharedString::new_static("skeleton"),
                StudioTracksRow::Empty => SharedString::new_static("empty"),
                StudioTracksRow::Track(index) => SharedString::from(entries[*index].track.id.clone()),
                StudioTracksRow::Foot => SharedString::new_static("foot"),
            })
            .collect();
        let list_state = self.state.update(cx, |state, _| {
            state.tracks_list.sync(ids);
            state.tracks_list.list().state().clone()
        });

        let id = self.id;
        let nav_store = self.nav_store;
        let cache_key = MusicServicesStore::my_tracks_cache_key();
        list(list_state, move |index, _, cx| match rows.get(index) {
            Some(StudioTracksRow::TopInset) => div().w_full().h(px(top_inset)).into_any_element(),
            Some(StudioTracksRow::Skeleton) => {
                div().w_full().pt(px(8.0)).child(TrackListSkeleton::new().rows(8).horizontal_padding(16.0)).into_any_element()
            }
            Some(StudioTracksRow::Empty) => {
                empty_state("music.note", tr("music_studio_no_tracks"), &theme).into_any_element()
            }
            Some(StudioTracksRow::Track(position)) => {
                let entry = &entries[*position];
                let track_id = entry.track.id.clone();
                let tap_store = nav_store.clone();
                let row = TrackContextMenuRow::new(named_child(&id, format!("track-{track_id}")), entry.track.clone())
                    .context(TrackContextSource::OwnArtistStudio)
                    .shows_like_button(false)
                    .on_tap(move |_, cx| {
                        if let Some(nav_store) = &tap_store {
                            let route = RootRoute::StudioTrackDetail { track_id: track_id.clone() };
                            nav_store.update(cx, |nav_store, cx| {
                                nav_store.push(route, NavigationStoreSettlingPush::Drop, cx);
                            });
                        }
                    });
                vstack(0.0)
                    .w_full()
                    .child(div().w_full().px(px(16.0)).child(row.music_pagination_prefetch(
                        named_child(&id, format!("track-prefetch-{}", entry.track.id)),
                        entry.index,
                        raw_count,
                        &cache_key,
                        true,
                        TRACK_PREFETCH_THRESHOLD,
                        |_, cx| {
                            MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_my_tracks(cx));
                        },
                        cx,
                    )))
                    .when(entry.status == RiseMusicTrackStatus::ModerationDisabled.wire(), |column| {
                        column.child(
                            div()
                                .w_full()
                                .pl(px(76.0))
                                .pr(px(16.0))
                                .pb(px(10.0))
                                .child(ModerationHiddenBanner::new(tr("music_status_moderation_disabled"))),
                        )
                    })
                    .child(hairline(76.0, theme.border._100))
                    .into_any_element()
            }
            Some(StudioTracksRow::Foot) => div().w_full().music_mini_player_bottom_padding(40.0, 0.0).into_any_element(),
            None => div().into_any_element(),
        })
        .size_full()
    }
}

#[derive(Clone, Copy)]
enum StudioAlbumsRow {
    Head,
    Skeleton,
    Empty,
    Album(usize),
    Foot,
}

#[derive(IntoElement)]
struct ArtistStudioAlbumsPage {
    id: ElementId,
    state: Entity<ArtistStudioState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl ArtistStudioAlbumsPage {
    fn requests_row(
        id: ElementId,
        top_inset: f32,
        nav_store: Option<Entity<NavigationStore>>,
        theme: &AppTheme,
    ) -> AnyElement {
        vstack(0.0)
            .w_full()
            .pt(px(top_inset))
            .child(
                plain(id)
                    .w_full()
                    .on_press(move |_, cx| {
                        if let Some(nav_store) = &nav_store {
                            nav_store.update(cx, |nav_store, cx| {
                                nav_store.push(RootRoute::MyMusicAlbumRequests, NavigationStoreSettlingPush::Drop, cx);
                            });
                        }
                    })
                    .child(
                        hstack(12.0)
                            .w_full()
                            .px(px(16.0))
                            .py(px(14.0))
                            .child(div().w(px(32.0)).flex().justify_center().children(IconUi::sized(
                                "tray.full",
                                px(16.0),
                                theme.primary._100,
                                false,
                            )))
                            .child(MainText::new(tr("music_album_requests")).px(14.0).color(theme.text.primary))
                            .child(spacer())
                            .children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(13.0), theme.text.secondary, false)),
                    ),
            )
            .child(hairline(0.0, theme.border._100))
            .into_any_element()
    }

    fn album_row(
        id: &ElementId,
        album: &WireAlbumDTO,
        is_publish_disabled: bool,
        state: Entity<ArtistStudioState>,
        nav_store: Option<Entity<NavigationStore>>,
        theme: &AppTheme,
    ) -> AnyElement {
        let album_id = album.id.clone();
        let title = if album.title.is_empty() { TITLE_PLACEHOLDER.to_owned() } else { album.title.clone() };
        let cover_url = RemoteMediaURL::optional_normalized(Some(album.cover_url.as_str())).unwrap_or_default();
        let trailing: AnyElement = if album.status == RiseMusicTrackStatus::Approved.wire() {
            let publish_album_id = album_id.clone();
            div()
                .id(named_child(id, format!("publish-wrap-{album_id}")))
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    plain(named_child(id, format!("publish-{album_id}")))
                        .disabled(is_publish_disabled)
                        .on_press(move |_, cx| {
                            let album_id = publish_album_id.clone();
                            state.update(cx, |state, cx| {
                                state.album_to_publish = Some(album_id);
                                state.show_publish_sheet = true;
                                cx.notify();
                            });
                        })
                        .child(
                            div()
                                .px(px(10.0))
                                .py(px(5.0))
                                .capsule()
                                .bg(theme.primary._100.opacity(0.12))
                                .whitespace_nowrap()
                                .swiftui_font(Font::system_weight(12.0, FontWeight::SEMIBOLD))
                                .text_color(theme.primary._100)
                                .child(tr("music_publish")),
                        ),
                )
                .into_any_element()
        } else {
            div().children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(13.0), theme.text.secondary, false)).into_any_element()
        };

        vstack(0.0)
            .w_full()
            .child(
                plain(named_child(id, format!("album-{album_id}")))
                    .w_full()
                    .on_press(move |_, cx| {
                        if album_id.is_empty() {
                            return;
                        }
                        if let Some(nav_store) = &nav_store {
                            let route = RootRoute::StudioAlbumDetail { album_id: album_id.clone() };
                            nav_store.update(cx, |nav_store, cx| {
                                nav_store.push(route, NavigationStoreSettlingPush::Drop, cx);
                            });
                        }
                    })
                    .child(
                        hstack(14.0)
                            .w_full()
                            .px(px(16.0))
                            .py(px(12.0))
                            .child(
                                MusicCoverImage::new(named_child(id, format!("cover-{}", album.id)), cover_url)
                                    .size(56.0)
                                    .corner_radius(10.0),
                            )
                            .child(
                                vstack(4.0)
                                    .flex_1()
                                    .min_w_0()
                                    .align(HorizontalAlignment::Leading)
                                    .child(
                                        MainText::new(title)
                                            .px(15.0)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .color(theme.text.primary)
                                            .line_limit(1),
                                    )
                                    .child(
                                        hstack(6.0)
                                            .when(!album.genre.is_empty(), |line| {
                                                line.child(SecondaryText::new(display_genre(&album.genre)).px(12.0))
                                            })
                                            .children(studio_status_badge(&album.status, theme)),
                                    ),
                            )
                            .child(trailing),
                    ),
            )
            .child(hairline(86.0, theme.border._100))
            .into_any_element()
    }
}

impl RenderOnce for ArtistStudioAlbumsPage {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let top_inset = list_shell_floating_top_inset(cx);
        let theme = rise_ui::theme(cx).clone();
        let actions = MusicActionsStore::shared(cx);
        let (albums, is_pending, is_publish_pending) = {
            let store = actions.read(cx);
            (
                store.list_my_albums_sai.data().map(|data| data.albums).unwrap_or_default(),
                store.list_my_albums_sai.is_pending(),
                store.publish_album_sai.is_pending(),
            )
        };
        let is_publish_disabled = is_publish_pending || self.state.read(cx).show_publish_sheet;

        let mut rows = vec![StudioAlbumsRow::Head];
        if is_pending && albums.is_empty() {
            rows.push(StudioAlbumsRow::Skeleton);
        } else if albums.is_empty() {
            rows.push(StudioAlbumsRow::Empty);
        } else {
            rows.extend((0..albums.len()).map(StudioAlbumsRow::Album));
        }
        rows.push(StudioAlbumsRow::Foot);

        let ids: Vec<SharedString> = rows
            .iter()
            .map(|row| match row {
                StudioAlbumsRow::Head => SharedString::new_static("head"),
                StudioAlbumsRow::Skeleton => SharedString::new_static("skeleton"),
                StudioAlbumsRow::Empty => SharedString::new_static("empty"),
                StudioAlbumsRow::Album(index) => SharedString::from(albums[*index].id.clone()),
                StudioAlbumsRow::Foot => SharedString::new_static("foot"),
            })
            .collect();
        let list_state = self.state.update(cx, |state, _| {
            state.albums_list.sync(ids);
            state.albums_list.list().state().clone()
        });

        let id = self.id;
        let state = self.state;
        let nav_store = self.nav_store;
        list(list_state, move |index, _, _| match rows.get(index) {
            Some(StudioAlbumsRow::Head) => {
                Self::requests_row(named_child(&id, "requests"), top_inset, nav_store.clone(), &theme)
            }
            Some(StudioAlbumsRow::Skeleton) => {
                div().w_full().pt(px(8.0)).child(MusicLibraryListSkeleton::new().rows(6)).into_any_element()
            }
            Some(StudioAlbumsRow::Empty) => {
                empty_state("square.stack", tr("music_studio_no_albums"), &theme).into_any_element()
            }
            Some(StudioAlbumsRow::Album(position)) => Self::album_row(
                &id,
                &albums[*position],
                is_publish_disabled,
                state.clone(),
                nav_store.clone(),
                &theme,
            ),
            Some(StudioAlbumsRow::Foot) => div().w_full().music_mini_player_bottom_padding(40.0, 0.0).into_any_element(),
            None => div().into_any_element(),
        })
        .size_full()
    }
}

#[derive(IntoElement)]
struct ArtistStudioAnalyticsPage {
    id: ElementId,
}

impl ArtistStudioAnalyticsPage {
    fn analytics_card(icon: &'static str, value: String, label: String, color: Hsla, theme: &AppTheme) -> Div {
        hstack(16.0)
            .w_full()
            .px(px(16.0))
            .py(px(8.0))
            .child(
                div()
                    .size(px(48.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(12.0))
                    .bg(color.opacity(0.12))
                    .children(IconUi::sized(icon, px(20.0), color, false)),
            )
            .child(
                vstack(2.0)
                    .align(HorizontalAlignment::Leading)
                    .child(MainText::new(value).px(20.0).font_weight(FontWeight::BOLD).color(theme.text.primary))
                    .child(SecondaryText::new(label).px(13.0)),
            )
            .child(spacer())
    }

    fn analytics_cards(analytics: &ArtistAnalyticsModel, theme: &AppTheme) -> Div {
        vstack(12.0)
            .w_full()
            .child(Self::analytics_card(
                "ear",
                formatted(analytics.monthly_listeners),
                tr("music_analytics_listeners"),
                theme.primary._100,
                theme,
            ))
            .child(Self::analytics_card(
                "play.circle",
                formatted(analytics.total_plays),
                tr("music_analytics_plays"),
                rgb(LISTENERS_COLOR).into(),
                theme,
            ))
            .child(Self::analytics_card(
                "heart.fill",
                formatted(analytics.total_likes),
                tr("music_analytics_likes"),
                rgb(LIKES_COLOR).into(),
                theme,
            ))
            .child(Self::analytics_card(
                "person.2",
                formatted(analytics.total_followers),
                tr("music_analytics_followers"),
                rgb(FOLLOWERS_COLOR).into(),
                theme,
            ))
            .child(Self::analytics_card(
                "arrow.triangle.2.circlepath",
                analytics.retention7d_percent_text(),
                tr("music_analytics_retention_7d"),
                rgb(RETENTION_WEEK_COLOR).into(),
                theme,
            ))
            .child(Self::analytics_card(
                "calendar",
                analytics.retention30d_percent_text(),
                tr("music_analytics_retention_30d"),
                rgb(RETENTION_MONTH_COLOR).into(),
                theme,
            ))
    }
}

impl RenderOnce for ArtistStudioAnalyticsPage {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let actions = MusicActionsStore::shared(cx);
        let (analytics, is_pending) = {
            let store = actions.read(cx);
            (store.get_artist_analytics_sai.data().map(|data| data.as_model()), store.get_artist_analytics_sai.is_pending())
        };
        scroll_view(self.id)
            .shows_indicators(false)
            .size_full()
            .list_shell_scroll_under_chrome(cx)
            .child(
                vstack(20.0)
                    .w_full()
                    .pt(px(16.0))
                    .when(is_pending && analytics.is_none(), |column| {
                        column.child(
                            hstack(0.0)
                                .w_full()
                                .py(px(32.0))
                                .child(spacer())
                                .child(progress_view().tint(theme.primary._100))
                                .child(spacer()),
                        )
                    })
                    .when_some(analytics.as_ref(), |column, analytics| {
                        column.child(Self::analytics_cards(analytics, &theme))
                    })
                    .child(
                        div()
                            .w_full()
                            .px(px(32.0))
                            .child(SecondaryText::new(tr("music_analytics_hint")).px(13.0).tac(TextAlignT::Center)),
                    )
                    .music_mini_player_bottom_padding(40.0, 0.0),
            )
    }
}
