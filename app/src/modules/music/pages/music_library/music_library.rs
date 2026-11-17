use gpui::{
    AnyView, App, Context, ElementId, Empty, Entity, FontWeight, ListAlignment, SharedString, StyleRefinement, Subscription,
    Window, white,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_i18n::{FormatArg, tr, tr_plural, tr_plural_with};
use rise_theme::{AppTheme, ThemeStore, color};
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::rounded_gradient::rounded_gradient_fill;
use rise_ui::swiftui::transition::UnitPoint;

use crate::app::router::main_tab_chrome::main_tab_bar_underlap;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::music::pages::music_tab::music_tab::{
    MusicHeaderCollapseContext, MusicHeaderCollapseKey, music_header_collapse_probe,
};
use crate::modules::music::shared::utils::music::music_models::{ArtistModel, PlaylistModel};
use crate::modules::music::shared::utils::music::music_pagination_prefetch::MusicPaginationPrefetchModifiers;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::WireArtistDTOExt;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::bottomsheets::playlists::create_playlist_sheet::CreatePlaylistSheet;
use crate::modules::music::widgets::bottomsheets::playlists::playlist_visibility::PlaylistVisibility;
use crate::modules::music::widgets::music_cover_placeholder::MusicCoverPlaceholder;
use crate::modules::music::widgets::music_mini_player_bottom_padding::MusicMiniPlayerBottomPaddingModifiers;
use crate::modules::music::widgets::music_skeletons::MusicLibraryListSkeleton;
use crate::modules::music::widgets::music_track_cover_image::MusicCoverImage;

const LIKED_SONGS_ID: &str = "liked_songs";
const LIKED_COLORS: [&str; 2] = ["#E84393", "#FD79A8"];
const LOCAL_COLORS: [&str; 2] = ["#6C5CE7", "#0984E3"];
const COVER_SIDE: f32 = 54.0;
const COVER_RADIUS: f32 = 8.0;
const CIRCLE_RADIUS: f32 = 27.0;

fn library_id(suffix: impl Into<String>) -> ElementId {
    ElementId::Name(format!("music-library-{}", suffix.into()).into())
}

fn hex_colors(colors: [&str; 2]) -> Vec<String> {
    colors.iter().map(|hex| (*hex).to_owned()).collect()
}

/// one row of the list
/// feed is paginated so rows are laid out on demand, not all at once
#[derive(Clone)]
enum MusicLibraryRow {
    Liked,
    Local,
    CreatePlaylist,
    Playlist(PlaylistModel),
    PlaylistsSkeleton,
    Artist(usize, ArtistModel),
    ArtistsSkeleton,
    Footer,
}

impl MusicLibraryRow {
    fn id(&self) -> String {
        match self {
            Self::Liked => "liked".to_owned(),
            Self::Local => "local".to_owned(),
            Self::CreatePlaylist => "create".to_owned(),
            Self::Playlist(playlist) => format!("playlist-{}", playlist.id),
            Self::PlaylistsSkeleton => "playlists-skeleton".to_owned(),
            Self::Artist(_, artist) => format!("artist-{}", artist.id),
            Self::ArtistsSkeleton => "artists-skeleton".to_owned(),
            Self::Footer => "footer".to_owned(),
        }
    }
}

#[derive(Clone)]
struct MusicLibraryRowParts {
    nav_store: Option<Entity<NavigationStore>>,
    theme: AppTheme,
    state: Entity<MusicLibraryView>,
    local_track_count: i64,
    liked_track_count: i64,
    artists_count: usize,
    tab_underlap: f32,
    top_inset: f32,
}

impl MusicLibraryRowParts {
    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn divider(&self) -> Div {
        div().w_full().pl(px(84.0)).child(div().w_full().h(px(0.5)).bg(self.theme.border._100))
    }

    #[allow(clippy::too_many_arguments)]
    fn library_row(
        &self,
        id: String,
        image_url: Option<String>,
        colors: Vec<String>,
        icon: Option<&'static str>,
        title: String,
        subtitle: Option<String>,
        is_circle: bool,
        action: impl Fn(&mut App) + 'static,
    ) -> PlainButton {
        let radius = if is_circle { CIRCLE_RADIUS } else { COVER_RADIUS };
        let mut cover = div()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .size(px(COVER_SIDE))
            .clip_shape(if is_circle { Shape::Circle } else { Shape::continuous(COVER_RADIUS) });
        if let Some(url) = image_url.filter(|url| !url.is_empty()) {
            cover = cover.child(
                div().absolute().inset_0().child(
                    MusicCoverImage::new(library_id(format!("cover-{id}")), url)
                        .fallback_colors(colors)
                        .size(COVER_SIDE)
                        .corner_radius(radius),
                ),
            );
        } else if colors.is_empty() {
            cover = cover.child(div().absolute().inset_0().child(MusicCoverPlaceholder::new().icon_scale(0.34).corner_radius(radius)));
        } else {
            let gradient =
                LinearGradient::colors(colors.iter().map(|hex| color(hex)), UnitPoint::TOP_LEADING, UnitPoint::BOTTOM_TRAILING);
            cover = cover.child(div().absolute().inset_0().child(rounded_gradient_fill(gradient, radius)));
        }
        if let Some(icon) = icon {
            cover = cover.child(div().relative().children(IconUi::sized(icon, px(22.0), white(), icon.ends_with(".fill"))));
        }

        let subtitle = subtitle.filter(|subtitle| !subtitle.is_empty());
        plain(library_id(format!("row-{id}"))).w_full().on_press(move |_, cx| action(cx)).child(
            hstack(14.0)
                .w_full()
                .px(px(16.0))
                .py(px(10.0))
                .child(cover)
                .child(
                    vstack(3.0)
                        .align(HorizontalAlignment::Leading)
                        .flex_1()
                        .min_w_0()
                        .child(
                            MainText::new(title)
                                .px(15.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(self.theme.text.primary)
                                .line_limit(1),
                        )
                        .children(subtitle.map(|subtitle| SecondaryText::new(subtitle).px(12.0).line_limit(1))),
                ),
        )
    }

    fn create_playlist_row(&self) -> impl IntoElement {
        let primary = self.theme.primary._100;
        let state = self.state.clone();
        plain(library_id("create-playlist"))
            .w_full()
            .on_press(move |_, cx| {
                state.update(cx, |state, cx| {
                    state.show_create_playlist = true;
                    cx.notify();
                });
            })
            .child(
                hstack(14.0)
                    .w_full()
                    .px(px(16.0))
                    .py(px(10.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .flex_none()
                            .size(px(COVER_SIDE))
                            .bg(primary.opacity(0.12))
                            .clip_shape(Shape::continuous(COVER_RADIUS))
                            .children(IconUi::sized("plus", px(22.0), primary, false)),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            MainText::new(tr("music_create_playlist"))
                                .px(15.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(primary),
                        ),
                    ),
            )
            .guide_anchor(RiseGuideAnchor::MusicCreatePlaylist)
    }

    fn row(&self, row: &MusicLibraryRow, is_first: bool, cx: &mut App) -> AnyElement {
        let content: AnyElement = match row {
            MusicLibraryRow::Liked => {
                let parts = self.clone();
                vstack(0.0)
                    .w_full()
                    .child(self.library_row(
                        "liked".to_owned(),
                        None,
                        hex_colors(LIKED_COLORS),
                        Some("heart.fill"),
                        tr("music_liked_songs"),
                        Some(tr_plural("music_track_count", self.liked_track_count)),
                        false,
                        move |cx| parts.push(RootRoute::MusicPlaylistDetail { collection_id: LIKED_SONGS_ID.to_owned() }, cx),
                    ))
                    .child(self.divider())
                    .into_any_element()
            }
            MusicLibraryRow::Local => {
                let parts = self.clone();
                let subtitle = (self.local_track_count != 0).then(|| tr_plural("music_track_count", self.local_track_count));
                vstack(0.0)
                    .w_full()
                    .child(
                        self.library_row(
                            "local".to_owned(),
                            None,
                            hex_colors(LOCAL_COLORS),
                            Some("music.note.house.fill"),
                            tr("music_local_tracks_title"),
                            subtitle,
                            false,
                            move |cx| parts.push(RootRoute::MusicLocalTracks, cx),
                        )
                        .guide_anchor(RiseGuideAnchor::MusicLocalTracks),
                    )
                    .child(self.divider())
                    .into_any_element()
            }
            MusicLibraryRow::CreatePlaylist => {
                vstack(0.0).w_full().child(self.create_playlist_row()).child(self.divider()).into_any_element()
            }
            MusicLibraryRow::Playlist(playlist) => {
                let parts = self.clone();
                let collection_id = playlist.id.clone();
                vstack(0.0)
                    .w_full()
                    .child(self.library_row(
                        format!("playlist-{}", playlist.id),
                        Some(playlist.cover_image_url.clone()),
                        playlist.cover_colors.clone(),
                        None,
                        playlist.title.clone(),
                        Some(playlist.list_subtitle()),
                        false,
                        move |cx| parts.push(RootRoute::MusicPlaylistDetail { collection_id: collection_id.clone() }, cx),
                    ))
                    .child(self.divider())
                    .into_any_element()
            }
            MusicLibraryRow::PlaylistsSkeleton | MusicLibraryRow::ArtistsSkeleton => {
                MusicLibraryListSkeleton::new().rows(3).into_any_element()
            }
            MusicLibraryRow::Artist(index, artist) => {
                let parts = self.clone();
                let artist_id = artist.id.clone();
                let subtitle = tr_plural_with(
                    "music_listeners_short",
                    artist.monthly_listeners,
                    FormatArg::Text(RiseFormat::compact_count(artist.monthly_listeners).into()),
                );
                vstack(0.0)
                    .w_full()
                    .child(self.library_row(
                        format!("artist-{}", artist.id),
                        Some(artist.cover_image_url.clone()),
                        artist.cover_colors.clone(),
                        None,
                        artist.name.clone(),
                        Some(subtitle),
                        true,
                        move |cx| parts.push(RootRoute::MusicArtistProfile { artist_id: artist_id.clone() }, cx),
                    ))
                    .child(self.divider())
                    .music_pagination_prefetch(
                        library_id(format!("artist-prefetch-{}", artist.id)),
                        *index,
                        self.artists_count,
                        &MusicServicesStore::artists_cache_key(),
                        true,
                        8,
                        |_, cx| {
                            MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.load_more_artists(cx));
                        },
                        cx,
                    )
                    .into_any_element()
            }
            MusicLibraryRow::Footer => {
                div().w_full().music_mini_player_bottom_padding(80.0, self.tab_underlap).into_any_element()
            }
        };
        div().w_full().when(is_first, |first| first.pt(px(self.top_inset))).child(content).into_any_element()
    }
}

/// view behind the music library, a cached view so a pager
/// swipe and a parent redraw dont rebuild the feed
/// environment input comes through sync
pub struct MusicLibraryView {
    nav_store: Option<Entity<NavigationStore>>,
    tab_underlap: f32,
    top_inset: f32,
    header_collapse: Option<MusicHeaderCollapseContext>,
    show_create_playlist: bool,
    list: ListUi,
    list_ids: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl MusicLibraryView {
    fn new(cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let services = MusicServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        Self {
            nav_store: None,
            tab_underlap: 0.0,
            top_inset: 0.0,
            header_collapse: None,
            show_create_playlist: false,
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            list_ids: Vec::new(),
            _subscriptions: vec![
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
                cx.observe(&sandbox, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn sync(
        &mut self,
        nav_store: Option<Entity<NavigationStore>>,
        tab_underlap: f32,
        top_inset: f32,
        header_collapse: Option<MusicHeaderCollapseContext>,
        cx: &mut Context<Self>,
    ) {
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

    fn sync_list(&mut self, ids: Vec<String>) {
        if ids == self.list_ids {
            return;
        }
        self.list.replaced(ids.len());
        self.list_ids = ids;
    }
}

impl Render for MusicLibraryView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let actions = MusicActionsStore::shared(cx);
        let services = MusicServicesStore::shared(cx);

        let (my_playlists, local_track_count, liked_track_count): (Vec<PlaylistModel>, i64, i64) =
            services.update(cx, |services, cx| {
                let playlists = services.playlists_mine(cx).iter().filter_map(|playlist| playlist.as_playlist_model()).collect();
                (playlists, services.local_tracks_total_count(cx), services.liked_tracks_total_count(cx))
            });
        let (artists, playlists_pending, artists_pending) = {
            let store = actions.read(cx);
            let artists: Vec<ArtistModel> = store
                .list_artists_sai
                .data()
                .map(|page| page.artists)
                .unwrap_or_default()
                .iter()
                .filter_map(|artist| artist.as_artist_model())
                .collect();
            (artists, store.get_playlists_mine_sai.is_pending(), store.list_artists_sai.is_pending())
        };

        let mut rows = vec![MusicLibraryRow::Liked, MusicLibraryRow::Local, MusicLibraryRow::CreatePlaylist];
        let playlists_empty = my_playlists.is_empty();
        rows.extend(my_playlists.into_iter().map(MusicLibraryRow::Playlist));
        if playlists_empty && playlists_pending {
            rows.push(MusicLibraryRow::PlaylistsSkeleton);
        }
        let artists_count = artists.len();
        rows.extend(artists.into_iter().enumerate().map(|(index, artist)| MusicLibraryRow::Artist(index, artist)));
        if artists_count == 0 && artists_pending {
            rows.push(MusicLibraryRow::ArtistsSkeleton);
        }
        rows.push(MusicLibraryRow::Footer);
        self.sync_list(rows.iter().map(MusicLibraryRow::id).collect());

        let state = cx.entity();
        let parts = MusicLibraryRowParts {
            nav_store: self.nav_store.clone(),
            theme: theme.clone(),
            state: state.clone(),
            local_track_count,
            liked_track_count,
            artists_count,
            tab_underlap: self.tab_underlap,
            top_inset: self.top_inset,
        };
        let list = self.list.element(move |index, _, cx| {
            let Some(row) = rows.get(index) else {
                return Empty.into_any_element();
            };
            parts.row(row, index == 0, cx)
        });

        let is_presented = Binding::entity(
            state,
            |view: &MusicLibraryView| view.show_create_playlist,
            |view, value: bool, _| view.show_create_playlist = value,
        );
        let locked_visibility = GuideSandbox::shared(cx).read(cx).is_armed().then_some(PlaylistVisibility::Local);
        let created_store = self.nav_store.clone();
        let probe = music_header_collapse_probe(library_id("collapse-probe"), self.header_collapse.as_ref())
            .map(|probe| probe.list_state(self.list.state().clone()));
        div().relative().size_full().bg(theme.bg._100).child(list).children(probe).child(
            CreatePlaylistSheet::new(library_id("create-sheet"), is_presented)
                .locked_visibility(locked_visibility)
                .on_created(move |new_playlist, cx| {
                    if let (Some(id), Some(nav_store)) = (new_playlist.and_then(|playlist| playlist.id), &created_store) {
                        nav_store.update(cx, |nav_store, cx| {
                            nav_store.push(RootRoute::MusicPlaylistDetail { collection_id: id }, NavigationStoreSettlingPush::Drop, cx);
                        });
                    }
                }),
        )
    }
}

#[derive(IntoElement)]
pub struct MusicLibrary {
    id: ElementId,
}

impl MusicLibrary {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into() }
    }
}

impl RenderOnce for MusicLibrary {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let tab_underlap = main_tab_bar_underlap(cx);
        let top_inset = list_shell_floating_top_inset(cx);
        let header_collapse = env::<MusicHeaderCollapseKey>(cx);
        let view = window.use_keyed_state(self.id, cx, |_, cx| MusicLibraryView::new(cx));
        view.update(cx, |view, cx| view.sync(nav_store, tab_underlap, top_inset, header_collapse, cx));
        AnyView::from(view).cached(StyleRefinement::default().size_full())
    }
}

/// library tab as its own page for the music settings row reachable without the music tab
#[derive(IntoElement, Default)]
pub struct MusicLibraryScreen;

impl MusicLibraryScreen {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for MusicLibraryScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        use_task(library_id("screen-bootstrap"), user_id, window, cx, async move |cx| {
            let _ = cx.update(|_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.bootstrap_music_tab(cx));
            });
        });
        SettingsWrapperUi::new(SharedString::from(tr("music_library")), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(MusicLibrary::new(library_id("screen-content")))
        .on_appear(library_id("screen-guide-fact"), |_, cx| {
            GuideFacts::set(RiseGuideSignal::MusicLibraryOpened, true, cx);
        })
        .and_on_disappear(|_, cx| {
            GuideFacts::set(RiseGuideSignal::MusicLibraryOpened, false, cx);
        })
    }
}
