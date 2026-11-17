use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{
    AnyView, App, Context, ElementId, Entity, FontWeight, Hsla, ScrollHandle, SharedString, StyleRefinement, Subscription,
    Window, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::{tr, tr_plural};
use rise_theme::{AppTheme, ThemeStore, color};
use rise_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::rounded_gradient::rounded_gradient_fill;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::transition::UnitPoint;

use crate::app::router::main_tab_chrome::main_tab_bar_underlap;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::{SaiCacheRevisionBumped, SaiCacheRevisionTracker};
use crate::core::widgets::wrappers::list_shell_ui::list_shell_floating_top_inset;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::music::pages::music_home::album_card::AlbumCard;
use crate::modules::music::pages::music_tab::music_tab::{
    MusicHeaderCollapseContext, MusicHeaderCollapseKey, music_header_collapse_probe,
};
use crate::modules::music::pages::track::track_context_menu_row::{TrackContextMenuRow, TrackContextSource};
use crate::modules::music::shared::utils::music::music_models::{ArtistModel, PlaylistModel, TrackModel};
use crate::modules::music::shared::utils::music::music_pagination_prefetch::MusicPaginationPrefetchModifiers;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::{ProtoTrackRecordExt, WireArtistDTOExt};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::{MusicPaginationState, MusicServicesStore};
use crate::modules::music::widgets::music_cover_placeholder::MusicCoverPlaceholder;
use crate::modules::music::widgets::music_mini_player_bottom_padding::MusicMiniPlayerBottomPaddingModifiers;
use crate::modules::music::widgets::music_skeletons::{
    AlbumCardGridSkeleton, MusicArtistsStripSkeleton, MusicRecentGridSkeleton, TrackListSkeleton,
};
use crate::modules::music::widgets::music_track_cover_image::MusicCoverImage;
use crate::modules::user::engine::rise_user_block_presentation::RiseUserBlockPresentation;
use crate::modules::user::shared::rise_blocked_content::RiseBlockedContent;

const LIKED_SONGS_ID: &str = "liked_songs";
const LIKED_COLORS: [&str; 2] = ["#E84393", "#FD79A8"];
const RECENT_GRID_LIMIT: usize = 6;
const RECENT_TRACKS_SCANNED: usize = 12;
const TRENDING_TRACKS_SHOWN: usize = 8;
const PLAYLIST_GRID_SPACING: f32 = 16.0;
const PLAYLIST_TILE_MIN_WIDTH: f32 = 170.0;
const PLAYLIST_GRID_MIN_COLUMNS: u16 = 2;

fn playlist_grid_column_count(width: f32) -> u16 {
    if width <= 0.0 {
        return PLAYLIST_GRID_MIN_COLUMNS;
    }
    let fitting = ((width + PLAYLIST_GRID_SPACING) / (PLAYLIST_TILE_MIN_WIDTH + PLAYLIST_GRID_SPACING)).floor();
    (fitting as u16).max(PLAYLIST_GRID_MIN_COLUMNS)
}

fn liked_colors() -> Vec<String> {
    LIKED_COLORS.iter().map(|hex| (*hex).to_owned()).collect()
}

fn home_id(suffix: impl Into<String>) -> ElementId {
    ElementId::Name(format!("music-home-{}", suffix.into()).into())
}

fn push(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }
}

fn card_subtitle(playlist: &PlaylistModel, services: &MusicServicesStore, cx: &mut App) -> String {
    services
        .playlist(&playlist.id, cx)
        .map(|stored| stored.playlist_list_subtitle())
        .unwrap_or_else(|| playlist.list_subtitle())
}

fn card_cache_keys(playlist_id: &str) -> [String; 2] {
    [
        SaiCache::new(["getPlaylist", playlist_id]).key,
        MusicServicesStore::message_mp3_pending_playlist_cache_key(playlist_id),
    ]
}

fn show_now_playing(cx: &mut App) {
    MusicPlayerState::shared(cx).update(cx, |player, cx| player.set_show_now_playing(true, cx));
}

/// one cell of the recently played grid
#[derive(Clone)]
struct MusicRecentGridItem {
    id: String,
    title: String,
    subtitle: String,
    colors: Vec<String>,
    image_url: String,
    tracks: Vec<TrackModel>,
}

fn recent_grid_items(
    liked_tracks: &[TrackModel],
    recent_tracks: &[TrackModel],
    my_playlists: &[PlaylistModel],
    liked_subtitle: &str,
) -> Vec<MusicRecentGridItem> {
    let mut items = Vec::new();
    if !liked_tracks.is_empty() {
        items.push(MusicRecentGridItem {
            id: LIKED_SONGS_ID.to_owned(),
            title: tr("music_liked_songs"),
            subtitle: liked_subtitle.to_owned(),
            colors: liked_colors(),
            image_url: String::new(),
            tracks: liked_tracks.to_vec(),
        });
    }

    let mut seen_albums = HashSet::new();
    for track in recent_tracks.iter().take(RECENT_TRACKS_SCANNED) {
        let key = if track.album_id.is_empty() { format!("t_{}", track.id) } else { track.album_id.clone() };
        if !seen_albums.insert(key.clone()) {
            continue;
        }
        let bucket = if track.album_id.is_empty() {
            vec![track.clone()]
        } else {
            recent_tracks.iter().filter(|other| other.album_id == track.album_id).cloned().collect()
        };
        let title = if track.album_id.is_empty() || track.album_title.is_empty() {
            track.title.clone()
        } else {
            track.album_title.clone()
        };
        items.push(MusicRecentGridItem {
            id: key,
            title,
            subtitle: track.artist.clone(),
            colors: Vec::new(),
            image_url: track.cover_image_url.clone(),
            tracks: bucket,
        });
        if items.len() >= RECENT_GRID_LIMIT {
            break;
        }
    }

    if items.is_empty() {
        return Vec::new();
    }
    for playlist in my_playlists {
        if items.len() >= RECENT_GRID_LIMIT {
            break;
        }
        if playlist.tracks.is_empty() {
            continue;
        }
        items.push(MusicRecentGridItem {
            id: playlist.id.clone(),
            title: playlist.title.clone(),
            subtitle: playlist.subtitle.clone(),
            colors: playlist.cover_colors.clone(),
            image_url: playlist.cover_image_url.clone(),
            tracks: playlist.tracks.clone(),
        });
    }

    items.truncate(RECENT_GRID_LIMIT);
    items
}

fn combined_status(statuses: &[SaiWsSliceStatus]) -> SaiWsSliceStatus {
    if statuses.contains(&SaiWsSliceStatus::Pending) {
        return SaiWsSliceStatus::Pending;
    }
    if statuses.contains(&SaiWsSliceStatus::Fulfilled) {
        return SaiWsSliceStatus::Fulfilled;
    }
    if statuses.contains(&SaiWsSliceStatus::Rejected) {
        return SaiWsSliceStatus::Rejected;
    }
    SaiWsSliceStatus::Idle
}

/// what the feed shows, derived from the stores once per data change
/// decoding pages, block filter and artists payload used to run every render
/// and a scroll renders every frame the header collapse moves it
struct MusicHomeFeed {
    liked_subtitle: String,
    grid_items: Vec<MusicRecentGridItem>,
    artists: Vec<ArtistModel>,
    artists_page: MusicPaginationState,
    tracks: Vec<TrackModel>,
    recent_status: SaiWsSliceStatus,
    recent_error: Option<String>,
    playlists_status: SaiWsSliceStatus,
    artists_status: SaiWsSliceStatus,
    artists_error: Option<String>,
    tracks_status: SaiWsSliceStatus,
    tracks_error: Option<String>,
}

pub struct MusicHomeView {
    nav_store: Option<Entity<NavigationStore>>,
    tab_underlap: f32,
    top_inset: f32,
    header_collapse: Option<MusicHeaderCollapseContext>,
    scroll_handle: ScrollHandle,
    playing_track_id: Option<String>,
    playlist_grid_width: f32,
    playlist_cards: Option<Rc<(Vec<PlaylistModel>, i64)>>,
    card_subtitles: HashMap<String, (String, String)>,
    card_keys: HashMap<String, String>,
    feed: Option<Rc<MusicHomeFeed>>,
    seen_revisions: Option<[u64; 5]>,
    hidden: bool,
    stale: bool,
    stale_cards: bool,
    stale_subtitles: bool,
    seen_slice_revisions: [u64; 5],
    _subscriptions: Vec<Subscription>,
}

impl MusicHomeView {
    fn new(cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let services = MusicServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let player = MusicPlayerState::shared(cx);
        let blocks = RiseUserBlockPresentation::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        let playing_track_id = Self::playing_track_id(player.read(cx));
        Self {
            nav_store: None,
            tab_underlap: 0.0,
            top_inset: 0.0,
            header_collapse: None,
            scroll_handle: ScrollHandle::new(),
            playing_track_id,
            playlist_grid_width: 0.0,
            playlist_cards: None,
            card_subtitles: HashMap::new(),
            card_keys: HashMap::new(),
            feed: None,
            seen_revisions: None,
            hidden: false,
            stale: false,
            stale_cards: false,
            stale_subtitles: false,
            seen_slice_revisions: [0; 5],
            _subscriptions: vec![
                cx.observe(&actions, |this, _, cx| this.data_changed(true, false, cx)),
                cx.observe(&blocks, |this, _, cx| this.data_changed(false, false, cx)),
                cx.observe(&services, |this, _, cx| this.data_changed(true, true, cx)),
                cx.observe(&sandbox, |this, _, cx| this.data_changed(true, true, cx)),
                cx.subscribe(&cache_revisions, |this, _, event: &SaiCacheRevisionBumped, cx| {
                    let touched = match event.key.as_deref() {
                        None => {
                            this.card_subtitles.clear();
                            true
                        }
                        Some(key) => match this.card_keys.get(key) {
                            Some(playlist_id) => {
                                let playlist_id = playlist_id.clone();
                                this.card_subtitles.remove(&playlist_id);
                                true
                            }
                            None => false,
                        },
                    };
                    if !touched {
                        return;
                    }
                    if this.hidden {
                        this.stale_cards = true;
                        this.stale = true;
                    } else {
                        this.playlist_cards = None;
                        this.feed = None;
                        this.refresh(cx);
                        cx.notify();
                    }
                }),
                cx.observe(&theme_store, |this, _, cx| this.redraw(cx)),
                cx.observe(&player, |this, player, cx| {
                    let next = Self::playing_track_id(player.read(cx));
                    if next != this.playing_track_id {
                        this.playing_track_id = next;
                        this.redraw(cx);
                    }
                }),
            ],
        }
    }

    /// a store changed, the feed is rebuilt here outside the draw
    /// a render that reads a store subscribes the view to it
    /// so a hidden page would redraw under the library on every store change
    fn data_changed(&mut self, cards: bool, subtitles: bool, cx: &mut Context<Self>) {
        if self.hidden {
            self.stale_cards |= cards;
            self.stale_subtitles |= subtitles;
            self.stale = true;
            return;
        }
        if cards {
            self.playlist_cards = None;
        }
        if subtitles {
            self.card_subtitles.clear();
        }
        self.feed = None;
        self.refresh(cx);
        cx.notify();
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        if self.hidden {
            self.stale = true;
        } else {
            cx.notify();
        }
    }

    fn apply_stale(&mut self, cx: &mut Context<Self>) {
        if self.hidden || !self.stale {
            return;
        }
        if self.stale_cards {
            self.playlist_cards = None;
        }
        if self.stale_subtitles {
            self.card_subtitles.clear();
        }
        self.feed = None;
        self.stale = false;
        self.stale_cards = false;
        self.stale_subtitles = false;
        self.refresh(cx);
        cx.notify();
    }

    /// slice marks notify only views that read the slice in a draw
    /// read here every visible draw so a fetch answered after the build wakes this view
    fn slice_revisions(cx: &mut App) -> [u64; 5] {
        let slices = MusicActionsStore::slices(cx);
        [
            slices.get_playlists_mine_sai.revision(),
            slices.get_liked_sai.revision(),
            slices.get_recent_sai.revision(),
            slices.list_artists_sai.revision(),
            slices.list_tracks_sai.revision(),
        ]
    }

    /// drops what the changed slices feed, true when anything was dropped
    fn take_slice_changes(&mut self, cx: &mut App) -> bool {
        let revisions = Self::slice_revisions(cx);
        if revisions == self.seen_slice_revisions {
            return false;
        }
        if revisions[..2] != self.seen_slice_revisions[..2] {
            self.playlist_cards = None;
        }
        self.feed = None;
        self.seen_slice_revisions = revisions;
        true
    }

    /// builds what is missing of the cards and the feed
    fn refresh(&mut self, cx: &mut App) -> (Rc<(Vec<PlaylistModel>, i64)>, Rc<MusicHomeFeed>) {
        let services = MusicServicesStore::shared(cx);
        let playlist_cards = match self.playlist_cards.clone() {
            Some(cards) => cards,
            None => {
                let subtitles = &mut self.card_subtitles;
                let started = RiseClock::monotonic();
                let mut decoded = 0usize;
                let cards = Rc::new(services.update(cx, |services, cx| {
                    let playlists: Vec<PlaylistModel> =
                        services.playlists_mine(cx).iter().filter_map(|playlist| playlist.as_playlist_model()).collect();
                    let playlists: Vec<PlaylistModel> = playlists
                        .into_iter()
                        .map(|mut playlist| {
                            let caption = playlist.list_subtitle();
                            playlist.subtitle = match subtitles.get(&playlist.id) {
                                Some((seen, cached)) if *seen == caption => cached.clone(),
                                _ => {
                                    decoded += 1;
                                    let subtitle = card_subtitle(&playlist, services, cx);
                                    subtitles.insert(playlist.id.clone(), (caption, subtitle.clone()));
                                    subtitle
                                }
                            };
                            playlist
                        })
                        .collect();
                    (playlists, services.liked_tracks_total_count(cx))
                }));
                self.card_subtitles.retain(|id, _| cards.0.iter().any(|playlist| &playlist.id == id));
                self.card_keys = cards
                    .0
                    .iter()
                    .flat_map(|playlist| card_cache_keys(&playlist.id).map(|key| (key, playlist.id.clone())))
                    .collect();
                tracing::debug!(
                    target: "riseonly::X1961",
                    cards = cards.0.len(),
                    decoded,
                    ms = RiseClock::monotonic().saturating_sub(started).as_secs_f64() * 1000.0,
                    "music home cards rebuilt"
                );
                self.playlist_cards = Some(cards.clone());
                self.feed = None;
                cards
            }
        };
        let feed = match self.feed.clone() {
            Some(feed) => feed,
            None => {
                let actions = MusicActionsStore::shared(cx);
                let feed = Rc::new(Self::build_feed(&actions, &services, &playlist_cards, cx));
                self.feed = Some(feed.clone());
                feed
            }
        };
        (playlist_cards, feed)
    }

    fn playing_track_id(player: &MusicPlayerState) -> Option<String> {
        player.current_track.as_ref().filter(|_| player.is_playing).map(|track| track.id.clone())
    }

    fn sync(
        &mut self,
        nav_store: Option<Entity<NavigationStore>>,
        tab_underlap: f32,
        top_inset: f32,
        header_collapse: Option<MusicHeaderCollapseContext>,
        hidden: bool,
        cx: &mut Context<Self>,
    ) {
        if self.hidden != hidden {
            self.hidden = hidden;
            if !hidden {
                cx.notify();
            }
            if !hidden && self.stale {
                let view = cx.entity().downgrade();
                cx.defer(move |cx| {
                    let _ = view.update(cx, |view, cx| view.apply_stale(cx));
                });
            } else if !hidden {
                let view = cx.entity().downgrade();
                cx.defer(move |cx| {
                    let _ = view.update(cx, |view, cx| {
                        if view.take_slice_changes(cx) {
                            view.refresh(cx);
                            cx.notify();
                        }
                    });
                });
            }
        }
        let changed = nav_store.as_ref().map(Entity::entity_id) != self.nav_store.as_ref().map(Entity::entity_id)
            || (self.tab_underlap - tab_underlap).abs() > f32::EPSILON
            || (self.top_inset - top_inset).abs() > f32::EPSILON
            || header_collapse != self.header_collapse;
        if changed {
            self.nav_store = nav_store;
            self.tab_underlap = tab_underlap;
            self.top_inset = top_inset;
            self.header_collapse = header_collapse;
            cx.notify();
        }
    }

    fn music_section(title: String, theme: &AppTheme, content: impl IntoElement) -> Div {
        vstack(12.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(div().px(px(16.0)).child(MainText::new(title).px(17.0).font_weight(FontWeight::BOLD).color(theme.text.primary)))
            .child(content)
    }

    fn recent_card(
        item: &MusicRecentGridItem,
        my_playlists: &[PlaylistModel],
        nav_store: &Option<Entity<NavigationStore>>,
        theme: &AppTheme,
    ) -> impl IntoElement {
        let cover: AnyElement = if item.image_url.is_empty() {
            let mut cover = div()
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .flex_none()
                .size(px(52.0))
                .clip_shape(Shape::continuous(8.0));
            if item.colors.is_empty() {
                cover = cover.child(div().absolute().inset_0().child(MusicCoverPlaceholder::new().icon_scale(0.34).corner_radius(8.0)));
            } else {
                let gradient = LinearGradient::colors(
                    item.colors.iter().map(|hex| color(hex)),
                    UnitPoint::TOP_LEADING,
                    UnitPoint::BOTTOM_TRAILING,
                );
                let symbol = if item.id == LIKED_SONGS_ID { "heart.fill" } else { "music.note" };
                cover = cover
                    .child(div().absolute().inset_0().child(rounded_gradient_fill(gradient, 8.0)))
                    .child(div().relative().children(IconUi::sized(symbol, px(18.0), white(), symbol.ends_with(".fill"))));
            }
            cover.into_any_element()
        } else {
            MusicCoverImage::new(home_id(format!("recent-cover-{}", item.id)), item.image_url.clone())
                .fallback_colors(item.colors.clone())
                .size(52.0)
                .corner_radius(8.0)
                .into_any_element()
        };

        let target = item.clone();
        let is_playlist = my_playlists.iter().any(|playlist| playlist.id == item.id);
        let nav_store = nav_store.clone();
        plain(home_id(format!("recent-{}", item.id)))
            .w_full()
            .on_press(move |_, cx| Self::open_recent(&target, is_playlist, &nav_store, cx))
            .child(
                hstack(10.0)
                    .w_full()
                    .h(px(52.0))
                    .bg(theme.bg._200)
                    .clip_shape(Shape::continuous(8.0))
                    .child(cover)
                    .child(
                        div().flex_1().min_w_0().child(
                            MainText::new(item.title.clone())
                                .px(13.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.text.primary)
                                .line_limit(2),
                        ),
                    ),
            )
    }

    fn open_recent(item: &MusicRecentGridItem, is_playlist: bool, nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
        if item.id == LIKED_SONGS_ID {
            push(nav_store, RootRoute::MusicPlaylistDetail { collection_id: LIKED_SONGS_ID.to_owned() }, cx);
            return;
        }
        if is_playlist {
            push(nav_store, RootRoute::MusicPlaylistDetail { collection_id: item.id.clone() }, cx);
            return;
        }
        let interactions = MusicInteractionsStore::shared(cx);
        if let Some(album_id) = item.tracks.first().map(|track| track.album_id.clone()).filter(|id| !id.is_empty() && *id == item.id) {
            let fallback = item.tracks.clone();
            interactions.update(cx, |interactions, cx| interactions.play_album(&album_id, fallback, cx));
        } else if !item.tracks.is_empty() {
            let tracks = item.tracks.clone();
            interactions.update(cx, |interactions, cx| {
                interactions.play_collection(
                    tracks,
                    None,
                    Some(Box::new(MusicInteractionsStore::open_full_player_on_accept)),
                    cx,
                );
            });
        }
    }

    fn build_feed(
        actions: &Entity<MusicActionsStore>,
        services: &Entity<MusicServicesStore>,
        playlist_cards: &(Vec<PlaylistModel>, i64),
        cx: &mut App,
    ) -> MusicHomeFeed {
        let (liked_tracks, recent_tracks, artists, tracks) = {
            let store = actions.read(cx);
            let liked = store.get_liked_sai.data().map(|page| page.tracks).unwrap_or_default();
            let recent = store.get_recent_sai.data().map(|page| page.tracks).unwrap_or_default();
            let artists = store.list_artists_sai.data().map(|page| page.artists).unwrap_or_default();
            let tracks = store.list_tracks_sai.data().map(|page| page.tracks).unwrap_or_default();
            (
                liked.iter().filter_map(|track| track.as_track_model()).collect::<Vec<TrackModel>>(),
                recent.iter().filter_map(|track| track.as_track_model()).collect::<Vec<TrackModel>>(),
                artists,
                tracks,
            )
        };
        let (recent_status, recent_error, playlists_status, artists_status, artists_error, tracks_status, tracks_error) = {
            let store = actions.read(cx);
            let recent = store.get_recent_sai.status();
            let liked = store.get_liked_sai.status();
            let error = if recent == SaiWsSliceStatus::Rejected {
                store.get_recent_sai.error()
            } else if liked == SaiWsSliceStatus::Rejected {
                store.get_liked_sai.error()
            } else {
                None
            };
            (
                combined_status(&[recent, liked]),
                error,
                store.get_playlists_mine_sai.status(),
                store.list_artists_sai.status(),
                store.list_artists_sai.error(),
                store.list_tracks_sai.status(),
                store.list_tracks_sai.error(),
            )
        };
        let artists: Vec<ArtistModel> = artists
            .iter()
            .filter(|artist| !RiseBlockedContent::hides(Some(artist.user_id.as_str()), cx))
            .filter_map(|artist| artist.as_artist_model())
            .collect();
        let tracks: Vec<TrackModel> = tracks
            .iter()
            .filter(|track| !RiseBlockedContent::hides(track.artist.as_ref().map(|artist| artist.user_id.as_str()), cx))
            .filter_map(|track| track.as_track_model())
            .collect();
        let liked_subtitle = tr_plural("music_track_count", playlist_cards.1);
        let grid_items = recent_grid_items(&liked_tracks, &recent_tracks, &playlist_cards.0, &liked_subtitle);
        let cache_key = MusicServicesStore::artists_cache_key();
        let artists_page = services.update(cx, |service, cx| service.pagination_state(&cache_key, cx));
        MusicHomeFeed {
            liked_subtitle,
            grid_items,
            artists,
            artists_page,
            tracks,
            recent_status,
            recent_error,
            playlists_status,
            artists_status,
            artists_error,
            tracks_status,
            tracks_error,
        }
    }

    fn artist_cell(
        index: usize,
        artist: &ArtistModel,
        count: usize,
        artists_cache_key: &str,
        artists_page: &MusicPaginationState,
        nav_store: &Option<Entity<NavigationStore>>,
        theme: &AppTheme,
    ) -> AnyElement {
        let artist_id = artist.id.clone();
        let nav_store = nav_store.clone();
        plain(home_id(format!("artist-{}", artist.id)))
            .flex_none()
            .on_press(move |_, cx| push(&nav_store, RootRoute::MusicArtistProfile { artist_id: artist_id.clone() }, cx))
            .child(
                vstack(8.0)
                    .child(
                        MusicCoverImage::new(home_id(format!("artist-cover-{}", artist.id)), artist.cover_image_url.clone())
                            .fallback_colors(artist.cover_colors.clone())
                            .size(100.0)
                            .corner_radius(50.0),
                    )
                    .child(
                        div().w(px(100.0)).child(
                            MainText::new(artist.name.clone())
                                .px(12.0)
                                .font_weight(FontWeight::MEDIUM)
                                .color(theme.text.primary)
                                .tac(TextAlignT::Center)
                                .line_limit(1),
                        ),
                    ),
            )
            .music_pagination_prefetch_page(
                home_id(format!("artist-prefetch-{}", artist.id)),
                index,
                count,
                artists_cache_key,
                artists_page,
                true,
                8,
                |_, cx| {
                    MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.load_more_artists(cx));
                },
            )
            .into_any_element()
    }
}

impl Render for MusicHomeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        if !self.hidden {
            self.take_slice_changes(cx);
        }
        if !self.hidden {
            let revisions = Self::slice_revisions(cx);
            if self.seen_revisions != Some(revisions) {
                if self.seen_revisions.is_some_and(|seen| seen[0] != revisions[0]) {
                    self.playlist_cards = None;
                }
                self.seen_revisions = Some(revisions);
                self.feed = None;
            }
        }
        let (playlist_cards, feed) = self.refresh(cx);
        let my_playlists: &[PlaylistModel] = &playlist_cards.0;
        let liked_subtitle = feed.liked_subtitle.clone();
        let recent_status = feed.recent_status;
        let recent_error = feed.recent_error.clone();
        let playlists_status = feed.playlists_status;
        let artists_slice_status = feed.artists_status;
        let artists_error = feed.artists_error.clone();
        let tracks_status = feed.tracks_status;
        let tracks_error = feed.tracks_error.clone();

        let nav_store = self.nav_store.clone();

        let recent_section = {
            let grid_nav = nav_store.clone();
            let grid_theme = theme.clone();
            let grid_playlists = playlist_cards.clone();
            let grid_feed = feed.clone();
            let is_empty = feed.grid_items.is_empty();
            let empty_text = tr("music_home_recently_empty");
            let empty_theme = theme.clone();
            let empty_message = empty_text.clone();
            SaiDataRender::new(recent_status, move |_, _| {
                div().w_full().px(px(16.0)).child(div().grid().grid_cols(2).gap_x(px(8.0)).gap_y(px(10.0)).w_full().children(
                    grid_feed.grid_items.iter().map(|item| Self::recent_card(item, &grid_playlists.0, &grid_nav, &grid_theme)),
                ))
            })
            .is_empty(is_empty)
            .no_data_text(empty_text)
            .error_text(recent_error)
            .need_pending(is_empty)
            .message_height_percent(0.0)
            .no_data_height_percent(0.0)
            .sai_data_render_loading(|_, _| MusicRecentGridSkeleton::new().cells(4))
            .sai_data_render_empty(move |_, _| MusicHomeSectionChrome::empty(empty_message.clone(), &empty_theme))
        };

        let playlists_section = {
            let liked_nav = nav_store.clone();
            let columns = playlist_grid_column_count(self.playlist_grid_width);
            let grid_view = cx.entity().downgrade();
            let mut grid = div().grid().grid_cols(columns).gap(px(PLAYLIST_GRID_SPACING)).w_full().child(
                AlbumCard::new(home_id("liked-card"), tr("music_liked_songs"), liked_subtitle.clone(), liked_colors())
                    .icon_overlay(SharedString::new_static("heart.fill"))
                    .fills_width(true)
                    .on_tap(move |_, cx| {
                        push(&liked_nav, RootRoute::MusicPlaylistDetail { collection_id: LIKED_SONGS_ID.to_owned() }, cx);
                    }),
            );
            for playlist in my_playlists {
                let playlist_nav = nav_store.clone();
                let collection_id = playlist.id.clone();
                let cover = Some(playlist.cover_image_url.clone()).filter(|url| !url.is_empty());
                grid = grid.child(
                    AlbumCard::new(
                        home_id(format!("playlist-{}", playlist.id)),
                        playlist.title.clone(),
                        playlist.subtitle.clone(),
                        playlist.cover_colors.clone(),
                    )
                    .cover_image_url(cover)
                    .fills_width(true)
                    .on_tap(move |_, cx| {
                        push(&playlist_nav, RootRoute::MusicPlaylistDetail { collection_id: collection_id.clone() }, cx);
                    }),
                );
            }
            let footer: Option<AnyElement> = if playlists_status == SaiWsSliceStatus::Pending && my_playlists.is_empty() {
                Some(AlbumCardGridSkeleton::new().cells(2).into_any_element())
            } else if playlists_status == SaiWsSliceStatus::Fulfilled && my_playlists.is_empty() {
                Some(MusicHomeSectionChrome::empty(tr("music_home_playlists_empty"), &theme).into_any_element())
            } else {
                None
            };
            vstack(12.0)
                .align(HorizontalAlignment::Leading)
                .w_full()
                .child(
                    div().w_full().px(px(16.0)).child(div().relative().w_full().child(grid).child(measure(
                        home_id("playlist-grid-width"),
                        move |bounds, _, cx| {
                            let width = f32::from(bounds.size.width);
                            let _ = grid_view.update(cx, |view, cx| {
                                if (view.playlist_grid_width - width).abs() > f32::EPSILON {
                                    view.playlist_grid_width = width;
                                    cx.notify();
                                }
                            });
                        },
                    ))),
                )
                .children(footer)
        };

        let artists_section = {
            let is_empty = feed.artists.is_empty();
            let strip_feed = feed.clone();
            let empty_text = tr("music_home_artists_empty");
            let empty_message = empty_text.clone();
            let empty_theme = theme.clone();
            let strip_theme = theme.clone();
            let strip_nav = nav_store.clone();
            SaiDataRender::new(artists_slice_status, move |_, _| {
                let count = strip_feed.artists.len();
                let cache_key = MusicServicesStore::artists_cache_key();
                let page = &strip_feed.artists_page;
                let cells: Vec<AnyElement> = strip_feed
                    .artists
                    .iter()
                    .enumerate()
                    .map(|(index, artist)| Self::artist_cell(index, artist, count, &cache_key, page, &strip_nav, &strip_theme))
                    .collect();
                div()
                    .id(home_id("artists-strip"))
                    .flex()
                    .w_full()
                    .overflow_x_scroll()
                    .child(hstack(16.0).flex_none().px(px(16.0)).children(cells))
            })
            .error_text(artists_error)
            .is_empty(is_empty)
            .no_data_text(empty_text)
            .need_pending(is_empty)
            .message_height_percent(0.0)
            .no_data_height_percent(0.0)
            .sai_data_render_loading(|_, _| MusicArtistsStripSkeleton::new())
            .sai_data_render_empty(move |_, _| MusicHomeSectionChrome::empty(empty_message.clone(), &empty_theme))
        };

        let tracks_section = {
            let is_empty = feed.tracks.is_empty();
            let tracks_feed = feed.clone();
            let empty_text = tr("music_home_tracks_empty");
            let empty_message = empty_text.clone();
            let empty_theme = theme.clone();
            let playing_track_id = self.playing_track_id.clone();
            SaiDataRender::new(tracks_status, move |_, _| {
                let tracks = &tracks_feed.tracks;
                vstack(0.0).w_full().children(tracks.iter().take(TRENDING_TRACKS_SHOWN).map(|track| {
                    let tapped = track.clone();
                    let queue = tracks.clone();
                    div().w_full().px(px(16.0)).child(
                        TrackContextMenuRow::new(home_id(format!("track-{}", track.id)), track.clone())
                            .context(TrackContextSource::External)
                            .is_playing(playing_track_id.as_deref() == Some(track.id.as_str()))
                            .on_tap(move |_, cx| {
                                let track = tapped.clone();
                                let queue = queue.clone();
                                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                                    interactions.play(track, queue, None, true, Some(Box::new(show_now_playing)), cx);
                                });
                            }),
                    )
                }))
            })
            .error_text(tracks_error)
            .is_empty(is_empty)
            .no_data_text(empty_text)
            .need_pending(is_empty)
            .message_height_percent(0.0)
            .no_data_height_percent(0.0)
            .sai_data_render_loading(|_, _| TrackListSkeleton::new().rows(5))
            .sai_data_render_empty(move |_, _| MusicHomeSectionChrome::empty(empty_message.clone(), &empty_theme))
        };

        let sections = vstack(28.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .pt(px(12.0))
            .child(Self::music_section(tr("music_recently_played"), &theme, recent_section))
            .child(Self::music_section(tr("music_your_playlists"), &theme, playlists_section))
            .child(Self::music_section(tr("music_artists_for_you"), &theme, artists_section))
            .child(Self::music_section(tr("music_trending_tracks"), &theme, tracks_section))
            .music_mini_player_bottom_padding(80.0, self.tab_underlap);

        let probe = music_header_collapse_probe(home_id("collapse-probe"), self.header_collapse.as_ref())
            .map(|probe| probe.scroll_view(self.scroll_handle.clone()));
        div()
            .relative()
            .size_full()
            .bg(theme.bg._100)
            .child(
                scroll_view(home_id("scroll"))
                    .scroll_handle(self.scroll_handle.clone())
                    .shows_indicators(false)
                    .pt(px(self.top_inset))
                    .child(sections),
            )
            .children(probe)
    }
}

/// compact empty state per block
enum MusicHomeSectionChrome {}

impl MusicHomeSectionChrome {
    fn empty(text: String, theme: &AppTheme) -> Div {
        let faded: Hsla = theme.text.secondary.opacity(0.45);
        hstack(10.0)
            .w_full()
            .h(px(56.0))
            .px(px(16.0))
            .children(IconUi::sized("music.note.list", px(20.0), faded, false))
            .child(SecondaryText::new(text).px(14.0).color(theme.text.secondary))
    }
}

#[derive(IntoElement)]
pub struct MusicHome {
    id: ElementId,
    hidden: bool,
}

impl MusicHome {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), hidden: false }
    }

    /// desktop only, the pager shows another page
    /// the feed redraws when shown again
    pub fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }
}

impl RenderOnce for MusicHome {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let tab_underlap = main_tab_bar_underlap(cx);
        let top_inset = list_shell_floating_top_inset(cx);
        let header_collapse = env::<MusicHeaderCollapseKey>(cx);
        let view = window.use_keyed_state(self.id, cx, |_, cx| MusicHomeView::new(cx));
        view.update(cx, |view, cx| view.sync(nav_store, tab_underlap, top_inset, header_collapse, self.hidden, cx));
        AnyView::from(view).cached(StyleRefinement::default().size_full())
    }
}
