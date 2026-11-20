use gpui::{
    AnyView, App, AppContext, Bounds, Context, ElementId, Empty, Entity, FocusHandle, FontWeight, ListAlignment, Pixels,
    SharedString, StyleRefinement, Subscription, Window,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::input_ui::input_actions::Cancel;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::motion::{MotionSpec, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};

use crate::app::router::main_tab_chrome::main_tab_bar_underlap;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::modules::music::pages::track::track_context_menu_row::{TrackContextMenuRow, TrackContextSource};
use crate::modules::music::shared::utils::music::music_local_state::MusicLocalState;
use crate::modules::music::shared::utils::music::music_models::{AlbumModel, ArtistModel, TrackModel};
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::{ProtoTrackRecordExt, WireAlbumDTOExt, WireArtistDTOExt};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::music_mini_player_bottom_padding::MusicMiniPlayerBottomPaddingModifiers;
use crate::modules::music::widgets::music_skeletons::TrackListSkeleton;
use crate::modules::music::widgets::music_track_cover_image::MusicCoverImage;

const CLOSE_BUTTON_SIDE: f32 = 42.0;
const SCOPE_ID_SEARCH: &str = "search";

const CONTENT_PADDING: f32 = 16.0;
const SECTION_SPACING: f32 = 24.0;
const ROW_SPACING: f32 = 8.0;

fn overlay_id(suffix: impl Into<String>) -> ElementId {
    ElementId::Name(format!("music-search-{}", suffix.into()).into())
}

pub enum MusicSearchHero {}

impl MusicSearchHero {
    pub const FIELD_ID: &'static str = "musicSearchField";

    pub fn animation() -> MotionSpec {
        spring(0.42, 0.86)
    }
}

/// hero namespace is one number here
/// the field frame is computed from it and the two measured frames
pub struct MusicSearchHeroNamespace {
    pub(crate) motion: Animated<f32>,
    pub collapsed_field_frame: Option<Bounds<Pixels>>,
    pub(crate) overlay_field_frame: Option<Bounds<Pixels>>,
}

impl MusicSearchHeroNamespace {
    pub fn new() -> Self {
        Self { motion: Animated::new(0.0), collapsed_field_frame: None, overlay_field_frame: None }
    }

    pub fn set_presented(&mut self, presented: bool, now: MotionTime, cx: &mut Context<Self>) {
        self.motion.set_in(if presented { 1.0 } else { 0.0 }, MusicSearchHero::animation(), now, cx);
        cx.notify();
    }
}

impl Default for MusicSearchHeroNamespace {
    fn default() -> Self {
        Self::new()
    }
}

/// one row of the results, tracks page in so rows are laid out on demand
#[derive(Clone)]
enum MusicSearchRowKind {
    Prompt,
    Pending,
    SectionTitle(&'static str),
    Artist(ArtistModel),
    ShowMoreArtists,
    Album(AlbumModel),
    Track(usize, TrackModel),
    Failure,
    NoResults,
    Footer,
}

#[derive(Clone)]
struct MusicSearchRow {
    kind: MusicSearchRowKind,
    top_spacing: f32,
}

impl MusicSearchRow {
    fn id(&self) -> String {
        match &self.kind {
            MusicSearchRowKind::Prompt => "prompt".to_owned(),
            MusicSearchRowKind::Pending => "pending".to_owned(),
            MusicSearchRowKind::SectionTitle(key) => format!("section-{key}"),
            MusicSearchRowKind::Artist(artist) => format!("artist-{}", artist.id),
            MusicSearchRowKind::ShowMoreArtists => "more-artists".to_owned(),
            MusicSearchRowKind::Album(album) => format!("album-{}", album.id),
            MusicSearchRowKind::Track(_, track) => format!("track-{}", track.id),
            MusicSearchRowKind::Failure => "failure".to_owned(),
            MusicSearchRowKind::NoResults => "no-results".to_owned(),
            MusicSearchRowKind::Footer => "footer".to_owned(),
        }
    }
}

#[derive(Clone)]
struct MusicSearchRowParts {
    query: String,
    theme: AppTheme,
    nav_store: Option<Entity<NavigationStore>>,
    is_presented: Option<Binding<bool>>,
    search_tracks: Vec<TrackModel>,
    playing_track_id: Option<String>,
    next_cursor: Option<String>,
    has_more: bool,
    is_loading_more_artists: bool,
    tab_underlap: f32,
}

impl MusicSearchRowParts {
    fn add_search(&self, cx: &mut App) {
        let query = self.query.clone();
        MusicLocalState::shared(cx).update(cx, |state, cx| state.add_search(&query, cx));
    }

    fn open(&self, route: RootRoute, window: &mut Window, cx: &mut App) {
        self.add_search(cx);
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
        if let Some(is_presented) = &self.is_presented {
            MusicSearchOverlay::close(is_presented, window, cx);
        }
    }

    fn section_title(&self, key: &'static str) -> impl IntoElement {
        MainText::new(tr(key)).px(14.0).font_weight(FontWeight::SEMIBOLD).color(self.theme.text.secondary)
    }

    fn artist_row(&self, artist: &ArtistModel) -> impl IntoElement {
        let parts = self.clone();
        let artist_id = artist.id.clone();
        plain(overlay_id(format!("artist-{}", artist.id)))
            .w_full()
            .py(px(4.0))
            .on_press(move |window, cx| parts.open(RootRoute::MusicArtistProfile { artist_id: artist_id.clone() }, window, cx))
            .child(
                hstack(12.0)
                    .w_full()
                    .child(
                        MusicCoverImage::new(overlay_id(format!("artist-cover-{}", artist.id)), artist.cover_image_url.clone())
                            .fallback_colors(artist.cover_colors.clone())
                            .size(44.0)
                            .corner_radius(22.0),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(MainText::new(artist.name.clone()).px(15.0).color(self.theme.text.primary)),
                    ),
            )
    }

    fn show_more_artists_button(&self) -> impl IntoElement {
        let query = self.query.clone();
        let is_loading = self.is_loading_more_artists;
        div().opacity(if is_loading { 0.5 } else { 1.0 }).child(
            plain(overlay_id("show-more-artists"))
                .disabled(is_loading)
                .py(px(6.0))
                .on_press(move |_, cx| {
                    MusicInteractionsStore::shared(cx)
                        .update(cx, |interactions, cx| interactions.show_more_search_artists(&query, cx));
                })
                .child(
                    MainText::new(tr("common_show_more"))
                        .px(14.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(self.theme.primary._100),
                ),
        )
    }

    fn album_row(&self, album: &AlbumModel) -> impl IntoElement {
        let parts = self.clone();
        let album_id = album.id.clone();
        let artist_name = (!album.artist_name.is_empty()).then(|| {
            div().w_full().min_w_0().child(SecondaryText::new(album.artist_name.clone()).px(13.0).line_limit(1))
        });
        plain(overlay_id(format!("album-{}", album.id)))
            .w_full()
            .py(px(4.0))
            .on_press(move |window, cx| parts.open(RootRoute::MusicAlbumDetail { album_id: album_id.clone() }, window, cx))
            .child(
                hstack(12.0)
                    .w_full()
                    .child(
                        MusicCoverImage::new(overlay_id(format!("album-cover-{}", album.id)), album.cover_image_url.clone())
                            .fallback_colors(album.cover_colors.clone())
                            .size(44.0)
                            .corner_radius(8.0),
                    )
                    .child(
                        vstack(2.0)
                            .align(HorizontalAlignment::Leading)
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .child(MainText::new(album.title.clone()).px(15.0).color(self.theme.text.primary).line_limit(1)),
                            )
                            .children(artist_name),
                    ),
            )
    }

    fn track_row(&self, index: usize, track: &TrackModel) -> impl IntoElement {
        let parts = self.clone();
        let tapped = track.clone();
        let load_query = self.query.clone();
        TrackContextMenuRow::new(overlay_id(format!("track-{}", track.id)), track.clone())
            .context(TrackContextSource::External)
            .is_playing(self.playing_track_id.as_deref() == Some(track.id.as_str()))
            .on_tap(move |_, cx| {
                parts.add_search(cx);
                let track = tapped.clone();
                let queue = parts.search_tracks.clone();
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.play(
                        track,
                        queue,
                        Some(SCOPE_ID_SEARCH),
                        true,
                        Some(Box::new(|cx: &mut App| {
                            MusicPlayerState::shared(cx).update(cx, |player, cx| player.set_show_now_playing(true, cx));
                        })),
                        cx,
                    );
                });
            })
            .sai_pagination_prefetch(
                overlay_id(format!("track-prefetch-{}", track.id)),
                SaiPaginationPrefetchOptions::new(
                    index,
                    self.search_tracks.len(),
                    MusicServicesStore::search_cache_key(&self.query, None),
                    self.next_cursor.clone(),
                    self.has_more,
                )
                .threshold(8),
                move |_, cx| {
                    MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.load_more_search(&load_query, cx));
                },
            )
    }

    fn state_icon(symbol: &str, side: f32, tint: gpui::Hsla) -> Option<gpui::Svg> {
        IconUi::sized(symbol, px(side), tint, symbol.ends_with(".fill"))
    }

    fn failure(&self) -> impl IntoElement {
        let query = self.query.clone();
        vstack(12.0)
            .w_full()
            .pt(px(60.0))
            .children(Self::state_icon("exclamationmark.triangle.fill", 30.0, self.theme.semantic.error_100))
            .child(SecondaryText::new(tr("error_fetching_data")).px(14.0).tac(TextAlignT::Center))
            .child(
                ButtonUi::new(overlay_id("retry"))
                    .background_color(self.theme.primary._100)
                    .height(42.0)
                    .fit_content(true)
                    .padding_horizontal(18.0)
                    .on_press(move |_, cx| {
                        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.search(&query, None, cx));
                    })
                    .child(MainText::new(tr("music_retry")).px(14.0).font_weight(FontWeight::SEMIBOLD)),
            )
    }

    fn row(&self, row: &MusicSearchRow) -> AnyElement {
        let content: AnyElement = match &row.kind {
            MusicSearchRowKind::Prompt => vstack(12.0)
                .w_full()
                .pt(px(72.0))
                .children(Self::state_icon("magnifyingglass", 44.0, self.theme.text.secondary.opacity(0.35)))
                .child(SecondaryText::new(tr("music_search_placeholder")).px(14.0).tac(TextAlignT::Center))
                .into_any_element(),
            MusicSearchRowKind::Pending => TrackListSkeleton::new().rows(6).horizontal_padding(0.0).into_any_element(),
            MusicSearchRowKind::SectionTitle(key) => self.section_title(*key).into_any_element(),
            MusicSearchRowKind::Artist(artist) => self.artist_row(artist).into_any_element(),
            MusicSearchRowKind::ShowMoreArtists => self.show_more_artists_button().into_any_element(),
            MusicSearchRowKind::Album(album) => self.album_row(album).into_any_element(),
            MusicSearchRowKind::Track(index, track) => self.track_row(*index, track).into_any_element(),
            MusicSearchRowKind::Failure => self.failure().into_any_element(),
            MusicSearchRowKind::NoResults => vstack(12.0)
                .w_full()
                .pt(px(60.0))
                .children(Self::state_icon("magnifyingglass", 48.0, self.theme.text.secondary.opacity(0.4)))
                .child(MainText::new(tr("music_no_results")).px(15.0).tac(TextAlignT::Center).color(self.theme.text.secondary))
                .into_any_element(),
            MusicSearchRowKind::Footer => {
                div().w_full().h(px(CONTENT_PADDING)).music_mini_player_bottom_padding(24.0, self.tab_underlap).into_any_element()
            }
        };
        div().w_full().px(px(CONTENT_PADDING)).pt(px(row.top_spacing)).child(content).into_any_element()
    }
}

struct MusicSearchRows {
    rows: Vec<MusicSearchRow>,
}

impl MusicSearchRows {
    fn new() -> Self {
        Self { rows: Vec::new() }
    }

    fn begin_section(&mut self, kind: MusicSearchRowKind) {
        let top_spacing = if self.rows.is_empty() { CONTENT_PADDING } else { SECTION_SPACING };
        self.rows.push(MusicSearchRow { kind, top_spacing });
    }

    fn add_row(&mut self, kind: MusicSearchRowKind) {
        self.rows.push(MusicSearchRow { kind, top_spacing: ROW_SPACING });
    }
}

/// results under the field as their own cached view
/// each keystroke and caret blink renders the field view and inline the rows laid out again too
/// now only when the query or their slices change
struct MusicSearchResultsView {
    query: String,
    is_presented: Option<Binding<bool>>,
    nav_store: Option<Entity<NavigationStore>>,
    tab_underlap: f32,
    playing_track_id: Option<String>,
    list: ListUi,
    list_ids: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl MusicSearchResultsView {
    fn new(cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let interactions = MusicInteractionsStore::shared(cx);
        let services = MusicServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let player = MusicPlayerState::shared(cx);
        let playing_track_id = Self::playing_track_id(player.read(cx));
        Self {
            query: String::new(),
            is_presented: None,
            nav_store: None,
            tab_underlap: 0.0,
            playing_track_id,
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            list_ids: Vec::new(),
            _subscriptions: vec![
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&interactions, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
                cx.observe(&player, |this, player, cx| {
                    let next = Self::playing_track_id(player.read(cx));
                    if next != this.playing_track_id {
                        this.playing_track_id = next;
                        cx.notify();
                    }
                }),
            ],
        }
    }

    fn playing_track_id(player: &MusicPlayerState) -> Option<String> {
        player.current_track.as_ref().filter(|_| player.is_playing).map(|track| track.id.clone())
    }

    fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        if self.query != query {
            self.query = query;
            cx.notify();
        }
    }

    fn sync(
        &mut self,
        is_presented: Binding<bool>,
        nav_store: Option<Entity<NavigationStore>>,
        tab_underlap: f32,
        cx: &mut Context<Self>,
    ) {
        self.is_presented = Some(is_presented);
        if nav_store.as_ref().map(Entity::entity_id) != self.nav_store.as_ref().map(Entity::entity_id)
            || (self.tab_underlap - tab_underlap).abs() > f32::EPSILON
        {
            self.nav_store = nav_store;
            self.tab_underlap = tab_underlap;
            cx.notify();
        }
    }
    fn sync_list(&mut self, ids: Vec<String>) {
        if ids == self.list_ids {
            return;
        }
        if !self.list_ids.is_empty()
            && ids.len() > self.list_ids.len()
            && ids.last() == self.list_ids.last()
            && ids[..self.list_ids.len() - 1] == self.list_ids[..self.list_ids.len() - 1]
        {
            let added = ids.len() - self.list_ids.len();
            let at = self.list_ids.len() - 1;
            self.list.removed(at..at + 1);
            self.list.appended(added + 1);
        } else {
            self.list.replaced(ids.len());
        }
        self.list_ids = ids;
    }

    /// which rows are shown for the current query and slices
    fn rows(
        query: &str,
        search_tracks: &[TrackModel],
        search_artists: Vec<ArtistModel>,
        search_albums: Vec<AlbumModel>,
        can_show_more_artists: bool,
        is_pending: bool,
        has_error: bool,
    ) -> Vec<MusicSearchRow> {
        let mut rows = MusicSearchRows::new();
        if query.is_empty() {
            rows.begin_section(MusicSearchRowKind::Prompt);
        } else {
            if is_pending {
                rows.begin_section(MusicSearchRowKind::Pending);
            }
            let nothing_found = search_tracks.is_empty() && search_artists.is_empty() && search_albums.is_empty();
            if !search_artists.is_empty() {
                rows.begin_section(MusicSearchRowKind::SectionTitle("music_artists"));
                rows.rows.extend(
                    search_artists
                        .into_iter()
                        .map(|artist| MusicSearchRow { kind: MusicSearchRowKind::Artist(artist), top_spacing: ROW_SPACING }),
                );
                if can_show_more_artists {
                    rows.add_row(MusicSearchRowKind::ShowMoreArtists);
                }
            }
            if !search_albums.is_empty() {
                rows.begin_section(MusicSearchRowKind::SectionTitle("music_filter_albums"));
                rows.rows.extend(
                    search_albums
                        .into_iter()
                        .map(|album| MusicSearchRow { kind: MusicSearchRowKind::Album(album), top_spacing: ROW_SPACING }),
                );
            }
            if !search_tracks.is_empty() {
                rows.begin_section(MusicSearchRowKind::SectionTitle("music_tracks"));
                for (index, track) in search_tracks.iter().enumerate() {
                    rows.add_row(MusicSearchRowKind::Track(index, track.clone()));
                }
            }
            if !is_pending && nothing_found {
                rows.begin_section(if has_error { MusicSearchRowKind::Failure } else { MusicSearchRowKind::NoResults });
            }
        }
        rows.add_row(MusicSearchRowKind::Footer);
        rows.rows
    }
}

impl Render for MusicSearchResultsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let actions = MusicActionsStore::shared(cx);
        let interactions = MusicInteractionsStore::shared(cx);
        let query = self.query.clone();
        let trimmed = trimming_whitespaces_and_newlines(&query).to_owned();

        let expanded_query_matches = interactions.read(cx).expanded_search_artists_query() == Some(trimmed.as_str());
        let (search_tracks, search_artists, search_albums, can_show_more, next_cursor, has_more, is_pending, has_error) = {
            let store = actions.read(cx);
            let search = store.search_sai.data();
            let expanded_page = store.search_artists_sai.data();
            let expanded = expanded_page
                .as_ref()
                .filter(|_| expanded_query_matches)
                .and_then(|page| page.artists.as_ref())
                .filter(|artists| !artists.is_empty());
            let plain_artists = search.as_ref().and_then(|page| page.artists.as_ref());
            let tracks: Vec<TrackModel> = search
                .as_ref()
                .map(|page| page.tracks.iter().filter_map(|track| track.as_track_model()).collect())
                .unwrap_or_default();
            let artists: Vec<ArtistModel> = expanded
                .or(plain_artists)
                .map(|artists| artists.iter().filter_map(|artist| artist.as_artist_model()).collect())
                .unwrap_or_default();
            let albums: Vec<AlbumModel> = search
                .as_ref()
                .and_then(|page| page.albums.as_ref())
                .map(|albums| albums.iter().filter_map(|album| album.as_album_model()).collect())
                .unwrap_or_default();
            let can_show_more = if expanded.is_some() {
                expanded_page.as_ref().is_some_and(|page| page.has_more)
            } else {
                plain_artists.map_or(0, Vec::len) as i64 >= MusicInteractionsStore::SEARCH_ARTISTS_PAGE_SIZE
            };
            (
                tracks,
                artists,
                albums,
                can_show_more,
                search.as_ref().and_then(|page| page.next_cursor.clone()),
                search.as_ref().is_some_and(|page| page.has_more),
                store.search_sai.is_pending(),
                store.search_sai.error().is_some(),
            )
        };
        let is_loading_more_artists = interactions.update(cx, |interactions, cx| interactions.is_loading_more_search_artists(&query, cx));

        let rows = Self::rows(&query, &search_tracks, search_artists, search_albums, can_show_more, is_pending, has_error);
        self.sync_list(rows.iter().map(MusicSearchRow::id).collect());

        let parts = MusicSearchRowParts {
            query,
            theme: theme.clone(),
            nav_store: self.nav_store.clone(),
            is_presented: self.is_presented.clone(),
            search_tracks,
            playing_track_id: self.playing_track_id.clone(),
            next_cursor,
            has_more,
            is_loading_more_artists,
            tab_underlap: self.tab_underlap,
        };
        let list = self.list.element(move |index, _, _| {
            let Some(row) = rows.get(index) else {
                return Empty.into_any_element();
            };
            parts.row(row)
        });
        div().size_full().child(list)
    }
}

/// view behind the search overlay, keyed by the mount of the layer
/// query state dies with it so closing search forgets the text
pub struct MusicSearchOverlayView {
    query: String,
    is_presented: Option<Binding<bool>>,
    hero_namespace: Option<Entity<MusicSearchHeroNamespace>>,
    focus: FocusHandle,
    results: Entity<MusicSearchResultsView>,
    _subscriptions: Vec<Subscription>,
}

impl MusicSearchOverlayView {
    fn new(cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        Self {
            query: String::new(),
            is_presented: None,
            hero_namespace: None,
            focus: cx.focus_handle(),
            results: cx.new(MusicSearchResultsView::new),
            _subscriptions: vec![cx.observe(&theme_store, |_, _, cx| cx.notify())],
        }
    }

    fn sync(
        &mut self,
        is_presented: Binding<bool>,
        hero_namespace: Entity<MusicSearchHeroNamespace>,
        nav_store: Option<Entity<NavigationStore>>,
        tab_underlap: f32,
        cx: &mut Context<Self>,
    ) {
        self.is_presented = Some(is_presented.clone());
        if self.hero_namespace.as_ref().map(Entity::entity_id) != Some(hero_namespace.entity_id()) {
            self._subscriptions.push(cx.observe(&hero_namespace, |_, _, cx| cx.notify()));
            self.hero_namespace = Some(hero_namespace);
            cx.notify();
        }
        self.results.update(cx, |results, cx| results.sync(is_presented, nav_store, tab_underlap, cx));
    }
    fn header(&self, entity: &Entity<Self>, theme: &AppTheme, now: MotionTime, window: &Window, cx: &App) -> Div {
        let query_binding = Binding::entity(
            entity.clone(),
            |view: &MusicSearchOverlayView| view.query.clone(),
            |view, value: String, cx| {
                view.results.update(cx, |results, cx| results.set_query(value.clone(), cx));
                view.query = value;
                cx.notify();
            },
        );
        let cancel_presented = self.is_presented.clone();
        let field = SearchFieldUi::new(overlay_id(MusicSearchHero::FIELD_ID), query_binding)
            .placeholder(SharedString::from(tr("music_search_placeholder")))
            .on_change(|value, _, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.search(value, None, cx));
            })
            .on_cancel(move |window, cx| {
                if let Some(is_presented) = &cancel_presented {
                    MusicSearchOverlay::close(is_presented, window, cx);
                }
            })
            .uses_inline_text_field(true)
            .auto_focus(true);

        let mut hero = div().relative().w_full();
        if let Some(namespace) = self.hero_namespace.as_ref().map(|namespace| namespace.read(cx)) {
            let progress = namespace.motion.frame_value(now, window);
            if let (Some(collapsed), Some(slot)) = (namespace.collapsed_field_frame, namespace.overlay_field_frame)
                && (1.0 - progress).abs() > f32::EPSILON
            {
                let remaining = 1.0 - progress;
                let collapsed_width = f32::from(collapsed.size.width);
                let slot_width = f32::from(slot.size.width);
                hero = hero
                    .left(px((f32::from(collapsed.origin.x) - f32::from(slot.origin.x)) * remaining))
                    .top(px((f32::from(collapsed.origin.y) - f32::from(slot.origin.y)) * remaining))
                    .w(px(slot_width + (collapsed_width - slot_width) * remaining));
            }
        }

        let measured = self.hero_namespace.clone();
        let is_presented = self.is_presented.clone();
        hstack(10.0)
            .w_full()
            .px(px(12.0))
            .pt(px(10.0))
            .pb(px(10.0))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .child(hero.child(field))
                    .child(measure(overlay_id("field-frame"), move |bounds, _, cx| {
                        if let Some(measured) = measured {
                            measured.update(cx, |namespace, cx| {
                                if namespace.overlay_field_frame != Some(bounds) {
                                    namespace.overlay_field_frame = Some(bounds);
                                    cx.notify();
                                }
                            });
                        }
                    })),
            )
            .child(
                LiquidGlassButton::new(overlay_id("close"), LiquidGlassButtonShape::Circle)
                    .size(CLOSE_BUTTON_SIDE)
                    .accessibility_label(Some(tr("cancel")))
                    .on_press(move |window, cx| {
                        if let Some(is_presented) = &is_presented {
                            MusicSearchOverlay::close(is_presented, window, cx);
                        }
                    })
                    .children(IconUi::sized("xmark", px(15.0), theme.text.secondary, false)),
            )
    }
}

impl Render for MusicSearchOverlayView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let entity = cx.entity();
        let now = RiseClock::monotonic();
        let header = self.header(&entity, &theme, now, window, cx);
        let action_presented = self.is_presented.clone();
        let key_presented = self.is_presented.clone();
        div()
            .id(overlay_id("root"))
            .size_full()
            .occlude()
            .bg(theme.bg._100)
            .track_focus(&self.focus)
            .capture_action(move |_: &Cancel, window, cx| {
                cx.stop_propagation();
                if let Some(is_presented) = &action_presented {
                    MusicSearchOverlay::close(is_presented, window, cx);
                }
            })
            .layer_keys(LayerCancel::dismiss(move |window, cx| {
                if let Some(is_presented) = &key_presented {
                    MusicSearchOverlay::close(is_presented, window, cx);
                }
            }))
            .child(
                div().size_full().flex().justify_center().wheel_into_children().child(
                    vstack(0.0).h_full().w_full().max_w(theme.shell.music_max_width).child(header).child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .child(AnyView::from(self.results.clone()).cached(StyleRefinement::default().size_full())),
                    ),
                ),
            )
    }
}

#[derive(IntoElement)]
pub struct MusicSearchOverlay {
    id: ElementId,
    is_presented: Binding<bool>,
    hero_namespace: Entity<MusicSearchHeroNamespace>,
}

impl MusicSearchOverlay {
    pub fn new(
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        hero_namespace: Entity<MusicSearchHeroNamespace>,
    ) -> Self {
        Self { id: id.into(), is_presented, hero_namespace }
    }

    /// owner moves focus off the field when the binding goes false
    fn close(is_presented: &Binding<bool>, window: &mut Window, cx: &mut App) {
        ScreenFocus::release_text_field(window, cx);
        is_presented.set(false, window, cx);
    }
}

impl RenderOnce for MusicSearchOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let tab_underlap = main_tab_bar_underlap(cx);
        let view = window.use_keyed_state(self.id, cx, |_, cx| MusicSearchOverlayView::new(cx));
        let is_presented = self.is_presented;
        let hero_namespace = self.hero_namespace;
        view.update(cx, |view, cx| view.sync(is_presented, hero_namespace, nav_store, tab_underlap, cx));
        AnyView::from(view).cached(StyleRefinement::default().size_full())
    }
}
