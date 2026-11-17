use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, ElementId, Entity, FontWeight, ListState, RenderOnce, SharedString, Subscription, Window, black,
    list, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::{FormatArg, tr, tr_plural_with};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::PageHeaderToolbarSlot;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonCircle, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::blur::BlurModifiers;
use rise_ui::swiftui::animated::{Animated, Lerp};
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::UnitPoint;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::music::pages::music_home::album_card::AlbumCard;
use crate::modules::music::pages::track::track_context_menu_row::{TrackContextMenuRow, TrackContextSource};
use crate::modules::music::shared::utils::music::music_models::{AlbumModel, ArtistModel, TrackModel, display_genre};
use crate::modules::music::shared::utils::music::music_pagination_prefetch::MusicPaginationPrefetchModifiers;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::{
    ProtoTrackRecordExt, WireAlbumDTOExt, WireArtistDTOExt,
};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::bottomsheets::track_picker::track_picker_shared::TrackPickerListModel;
use crate::modules::music::widgets::music_mini_player_bottom_padding::{
    MusicMiniPlayerBottomPaddingModifiers, observed_music_player,
};
use crate::modules::music::widgets::music_scope_playback_button::MusicScopePlaybackButton;
use crate::modules::music::widgets::music_skeletons::{TrackListSkeleton, TrackRowSkeleton};
use crate::modules::music::widgets::music_cover_placeholder::{MusicCoverMedia, MusicCoverPlaceholder};
use crate::modules::music::widgets::music_track_cover_image::MusicCoverImage;

const ARTIST_TITLE_PLACEHOLDER: &str = "\u{2014}";
const ARTIST_HERO_HEIGHT: f32 = 240.0;
const ARTIST_PAGE_PADDING: f32 = 20.0;
const ARTIST_SECTION_SPACING: f32 = 24.0;
const ARTIST_TRACK_SPACING: f32 = 8.0;
const TOP_TRACKS_SHOWN: usize = 5;

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

struct ArtistProfileState {
    list_model: TrackPickerListModel,
    is_artist_menu_presented: bool,
    _subscriptions: [Subscription; 3],
}

impl ArtistProfileState {
    fn new(cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let services = MusicServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        Self {
            list_model: TrackPickerListModel::new(cx),
            is_artist_menu_presented: false,
            _subscriptions: [
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&auth, |_, _, cx| cx.notify()),
            ],
        }
    }
}

#[derive(IntoElement)]
pub struct ArtistProfile {
    id: ElementId,
    artist_id: String,
}

impl ArtistProfile {
    pub fn new(artist_id: impl Into<String>) -> Self {
        let artist_id = artist_id.into();
        Self { id: ElementId::Name(SharedString::from(format!("artist-profile-{artist_id}"))), artist_id }
    }

    fn artist(&self, store: &MusicActionsStore) -> Option<ArtistModel> {
        let own = store.get_artist_sai.data().and_then(|data| data.artist);
        if let Some(own) = own.filter(|artist| artist.id == self.artist_id) {
            return own.as_artist_model();
        }
        let in_list = store
            .list_artists_sai
            .data()
            .map(|data| data.artists)
            .unwrap_or_default()
            .iter()
            .filter_map(WireArtistDTOExt::as_artist_model)
            .find(|artist| artist.id == self.artist_id);
        if in_list.is_some() {
            return in_list;
        }
        let in_search = store
            .search_sai
            .data()
            .and_then(|data| data.artists)
            .unwrap_or_default()
            .iter()
            .filter_map(WireArtistDTOExt::as_artist_model)
            .find(|artist| artist.id == self.artist_id);
        if in_search.is_some() {
            return in_search;
        }
        store
            .search_artists_sai
            .data()
            .and_then(|data| data.artists)
            .and_then(|artists| artists.into_iter().find(|artist| artist.id == self.artist_id))
            .and_then(|artist| artist.as_artist_model())
    }

    fn has_full_profile(&self, store: &MusicActionsStore) -> bool {
        store.get_artist_sai.data().and_then(|data| data.artist).is_some_and(|artist| artist.id == self.artist_id)
    }

    fn top_tracks(&self, store: &MusicActionsStore) -> Vec<TrackModel> {
        if !self.has_full_profile(store) {
            return Vec::new();
        }
        store
            .get_artist_sai
            .data()
            .map(|data| data.top_tracks)
            .unwrap_or_default()
            .iter()
            .filter_map(ProtoTrackRecordExt::as_track_model)
            .collect()
    }

    fn all_tracks(&self, store: &MusicActionsStore) -> Vec<TrackModel> {
        if !self.has_full_profile(store) {
            return Vec::new();
        }
        store
            .list_artist_tracks_sai
            .data()
            .map(|data| data.tracks)
            .unwrap_or_default()
            .iter()
            .filter_map(ProtoTrackRecordExt::as_track_model)
            .filter(|track| track.artist_id == self.artist_id)
            .collect()
    }

    fn albums(&self, store: &MusicActionsStore) -> Vec<AlbumModel> {
        if !self.has_full_profile(store) {
            return Vec::new();
        }
        store
            .get_artist_sai
            .data()
            .map(|data| data.albums)
            .unwrap_or_default()
            .iter()
            .filter_map(WireAlbumDTOExt::as_album_model)
            .collect()
    }

    fn is_following(&self, store: &MusicActionsStore) -> bool {
        if !self.has_full_profile(store) {
            return false;
        }
        store.get_artist_sai.data().and_then(|data| data.artist).is_some_and(|artist| artist.is_following)
    }

    fn is_own_artist(&self, store: &MusicActionsStore, viewer_id: Option<&str>) -> bool {
        if !self.has_full_profile(store) {
            return false;
        }
        let owner_id = store.get_artist_sai.data().and_then(|data| data.artist).map(|artist| artist.user_id);
        match owner_id {
            Some(owner_id) if !owner_id.is_empty() => viewer_id == Some(owner_id.as_str()),
            _ => false,
        }
    }

    fn is_loading(artist: Option<&ArtistModel>, store: &MusicActionsStore) -> bool {
        artist.is_none() && store.get_artist_sai.is_pending()
    }

    fn artist_menu_glyph(theme: &AppTheme) -> AnyElement {
        PageHeaderToolbarSlot::new(div().children(IconUi::sized("ellipsis", px(17.0), theme.text.primary, false)))
            .into_any_element()
    }
}

impl RenderOnce for ArtistProfile {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(named_child(&self.id, "state"), cx, |_, cx| ArtistProfileState::new(cx));
        let viewer_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let actions = MusicActionsStore::shared(cx);
        let parts = {
            let store = actions.read(cx);
            let artist = self.artist(store);
            ArtistProfileParts {
                id: self.id.clone(),
                artist_id: self.artist_id.clone(),
                is_loading: Self::is_loading(artist.as_ref(), store),
                is_get_artist_pending: store.get_artist_sai.is_pending(),
                has_full_profile: self.has_full_profile(store),
                top_tracks: Rc::new(self.top_tracks(store)),
                all_tracks: Rc::new(self.all_tracks(store)),
                albums: self.albums(store),
                is_following: self.is_following(store),
                is_own_artist: self.is_own_artist(store, viewer_id.as_deref()),
                artist,
                nav_store: nav_store.clone(),
                top_inset: 0.0,
            }
        };

        let theme = rise_ui::theme(cx).clone();
        let title = parts.artist.as_ref().map(|artist| artist.name.clone());
        let has_artist = parts.artist.is_some();
        let (artist_id, menu_id) = (self.artist_id.clone(), named_child(&self.id, "menu"));
        let appear_artist_id = self.artist_id.clone();

        let mut wrapper = SettingsWrapperUi::new(title.unwrap_or_else(|| ARTIST_TITLE_PLACEHOLDER.to_owned()), move |_, cx| {
            if let Some(nav_store) = nav_store.as_ref() {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        });
        if has_artist {
            let is_presented = Binding::entity(
                state.clone(),
                |state: &ArtistProfileState| state.is_artist_menu_presented,
                |state: &mut ArtistProfileState, value: bool, cx| {
                    state.is_artist_menu_presented = value;
                    cx.notify();
                },
            );
            let actions: HoldContextMenuPresentationActions = Rc::new(|_: &App| {
                vec![
                    HoldContextMenuAction::new("report", tr("ctx_report"))
                        .system_image("exclamationmark.bubble")
                        .destructive(true),
                ]
            });
            let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |_: String, _: &mut Window, cx: &mut App| {
                let target = ReportTarget::Artist(artist_id.clone());
                ReportSheetStore::shared(cx).update(cx, |store, cx| store.present(target, None, None, cx));
            });
            let source_theme = theme.clone();
            let source: HoldContextMenuTapSource =
                Rc::new(move |_: &mut Window, _: &mut App| Self::artist_menu_glyph(&source_theme));
            let opener = state.clone();
            wrapper = wrapper.right_jsx(
                plain(named_child(&self.id, "menu-button"))
                    .on_press(move |_, cx| {
                        opener.update(cx, |state, cx| {
                            state.is_artist_menu_presented = true;
                            cx.notify();
                        });
                    })
                    .child(Self::artist_menu_glyph(&theme))
                    .tap_context_menu(
                        menu_id,
                        is_presented,
                        HoldContextMenuAlignment::Trailing,
                        false,
                        false,
                        false,
                        None,
                        actions,
                        on_action,
                        source,
                    ),
            );
        }

        let body: AnyElement = if has_artist {
            ArtistProfileContent { parts, state }.into_any_element()
        } else if parts.is_loading {
            ArtistProfileSkeleton.into_any_element()
        } else {
            ArtistProfileNotFound.into_any_element()
        };

        wrapper.content(body).on_appear(named_child(&self.id, "appear"), move |_, cx| {
            MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.open_artist(&appear_artist_id, cx));
        })
    }
}

/// one entry of the page list
/// gpui lays out a long column only through list
/// so everything above the track rows is the first entry of one scroller
#[derive(Clone, Copy)]
enum ArtistProfileRow {
    Head,
    TracksTitle,
    Track(usize),
    Foot,
}

impl ArtistProfileRow {
    fn rows(track_count: usize) -> Vec<ArtistProfileRow> {
        let mut rows = Vec::with_capacity(track_count + 3);
        rows.push(ArtistProfileRow::Head);
        if track_count > 0 {
            rows.push(ArtistProfileRow::TracksTitle);
            rows.extend((0..track_count).map(ArtistProfileRow::Track));
        }
        rows.push(ArtistProfileRow::Foot);
        rows
    }
}

/// what a row of the list needs, the list asks for rows from outside the pages render
struct ArtistProfileParts {
    id: ElementId,
    artist_id: String,
    artist: Option<ArtistModel>,
    is_loading: bool,
    is_get_artist_pending: bool,
    has_full_profile: bool,
    top_tracks: Rc<Vec<TrackModel>>,
    all_tracks: Rc<Vec<TrackModel>>,
    albums: Vec<AlbumModel>,
    is_following: bool,
    is_own_artist: bool,
    nav_store: Option<Entity<NavigationStore>>,
    top_inset: f32,
}

impl ArtistProfileParts {
    fn child_id(&self, name: impl Into<SharedString>) -> ElementId {
        named_child(&self.id, name)
    }

    fn scope_id(&self) -> String {
        format!("artist:{}", self.artist_id)
    }

    fn show_now_playing(cx: &mut App) {
        MusicPlayerState::shared(cx).update(cx, |player, cx| player.set_show_now_playing(true, cx));
    }

    fn row(&self, row: ArtistProfileRow, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        match row {
            ArtistProfileRow::Head => self.head(theme, window, cx),
            ArtistProfileRow::TracksTitle => div()
                .w_full()
                .px(px(ARTIST_PAGE_PADDING))
                .pt(px(ARTIST_SECTION_SPACING))
                .child(
                    MainText::new(tr("music_tracks"))
                        .px(16.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.primary),
                )
                .into_any_element(),
            ArtistProfileRow::Track(index) => self.all_tracks_row(index, cx),
            ArtistProfileRow::Foot => div()
                .w_full()
                .h(px(ARTIST_PAGE_PADDING))
                .music_mini_player_bottom_padding(40.0, 0.0)
                .into_any_element(),
        }
    }

    fn head(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let Some(artist) = self.artist.as_ref() else {
            return div().into_any_element();
        };
        let body = vstack(ARTIST_SECTION_SPACING)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .px(px(ARTIST_PAGE_PADDING))
            .pt(px(ARTIST_PAGE_PADDING))
            .child(self.actions(theme, window, cx))
            .when(self.has_full_profile && !self.top_tracks.is_empty(), |body| body.child(self.top_tracks_section(theme, cx)))
            .when(self.has_full_profile && self.albums.iter().any(|album| !album.is_scheduled()), |body| {
                body.child(self.discography(theme))
            })
            .when(!self.has_full_profile && self.is_get_artist_pending, |body| {
                body.child(TrackListSkeleton::new().rows(4).horizontal_padding(0.0))
            });

        vstack(0.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .pt(px(self.top_inset))
            .child(self.hero(artist))
            .child(body)
            .into_any_element()
    }

    fn hero(&self, artist: &ArtistModel) -> AnyElement {
        let has_cover = MusicCoverMedia::has_remote_cover(&artist.cover_image_url);
        let gradient = LinearGradient::colors(
            [black().opacity(0.0), black().opacity(0.7)],
            UnitPoint::TOP,
            UnitPoint::BOTTOM,
        );
        let genres = (!artist.genres.is_empty()).then(|| {
            hstack(6.0).children(artist.genres.iter().map(|genre| {
                div()
                    .px(px(10.0))
                    .py(px(4.0))
                    .bg(white().opacity(0.2))
                    .capsule()
                    .swiftui_font(Font::system_weight(11.0, FontWeight::MEDIUM))
                    .text_color(white().opacity(0.7))
                    .child(display_genre(genre))
            }))
        });
        let monthly_listeners = tr_plural_with(
            "music_monthly_listeners",
            artist.monthly_listeners,
            FormatArg::Text(RiseFormat::compact_count(artist.monthly_listeners).into()),
        );

        div()
            .relative()
            .w_full()
            .h(px(ARTIST_HERO_HEIGHT))
            .overflow_hidden()
            .when(has_cover, |hero| {
                hero.child(
                    div().absolute().inset_0().overflow_hidden().child(
                        div()
                            .size_full()
                            .child(
                                MusicCoverImage::new(self.child_id("cover-backdrop"), artist.cover_image_url.clone())
                                    .fallback_colors(artist.cover_colors.clone())
                                    .size(None::<f32>)
                                    .corner_radius(0.0)
                                    .shows_placeholder_note(false),
                            )
                            .blur(40.),
                    ),
                )
                .child(
                    div().absolute().top_0().right_0().size(px(ARTIST_HERO_HEIGHT)).child(
                        MusicCoverImage::new(self.child_id("cover"), artist.cover_image_url.clone())
                            .fallback_colors(artist.cover_colors.clone())
                            .size(ARTIST_HERO_HEIGHT)
                            .corner_radius(0.0),
                    ),
                )
            })
            .when(!has_cover, |hero| {
                hero.child(
                    div().absolute().inset_0().child(
                        MusicCoverPlaceholder::new().colors(artist.cover_colors.clone()).shows_note(false),
                    ),
                )
            })
            .child(div().absolute().inset_0().child(gradient.view()))
            .when(!has_cover, |hero| {
                hero.child(
                    div()
                        .absolute()
                        .top_0()
                        .right_0()
                        .size(px(ARTIST_HERO_HEIGHT))
                        .child(MusicCoverPlaceholder::note(ARTIST_HERO_HEIGHT, 0.34)),
                )
            })
            .child(
                vstack(6.0)
                    .absolute()
                    .left_0()
                    .bottom_0()
                    .align(HorizontalAlignment::Leading)
                    .p(px(ARTIST_PAGE_PADDING))
                    .child(MainText::new(artist.name.clone()).px(28.0).font_weight(FontWeight::BOLD).color(white()))
                    .child(SecondaryText::new(monthly_listeners).px(13.0).color(white().opacity(0.8)))
                    .children(genres),
            )
            .into_any_element()
    }

    fn actions(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        hstack(12.0)
            .when(!self.is_own_artist, |row| row.child(self.follow_button(theme, window, cx)))
            .child(
                MusicScopePlaybackButton::new(self.scope_id(), (*self.top_tracks).clone())
                    .size(56.0)
                    .on_play_started(Self::show_now_playing),
            )
            .into_any_element()
    }

    fn follow_button(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let target = if self.is_following { 1_f32 } else { 0_f32 };
        let following = window.use_keyed_state(self.child_id("follow-motion"), cx, move |_, _| Animated::new(target));
        let now = RiseClock::monotonic();
        let progress = following.update(cx, |animated, cx| {
            animated.set_in(target, ease_in_out(0.15), now, cx);
            animated.frame_value(now, window)
        });
        let accent = theme.primary._100;
        let ink = theme.text.primary.lerp(&accent, progress);
        let background = theme.bg._200.lerp(&accent.opacity(0.12), progress);
        let border = theme.border._100.lerp(&accent, progress);
        let artist_id = self.artist_id.clone();

        plain(self.child_id("follow"))
            .disabled(!self.has_full_profile)
            .on_press(move |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_artist_follow(&artist_id, cx));
            })
            .child(
                div()
                    .px(px(24.0))
                    .py(px(10.0))
                    .bg(background)
                    .capsule()
                    .border_1()
                    .border_color(border)
                    .swiftui_font(Font::system_weight(14.0, FontWeight::SEMIBOLD))
                    .text_color(ink)
                    .child(if self.is_following { tr("music_unfollow") } else { tr("music_follow") }),
            )
            .into_any_element()
    }

    fn track_row(&self, track: &TrackModel, queue: &Rc<Vec<TrackModel>>, cx: &mut App) -> TrackContextMenuRow {
        let is_playing = {
            let player = MusicPlayerState::shared(cx);
            let player = player.read(cx);
            player.current_track.as_ref().is_some_and(|current| current.id == track.id) && player.is_playing
        };
        let tapped = track.clone();
        let queue = queue.clone();
        let scope_id = self.scope_id();
        TrackContextMenuRow::new(self.child_id(format!("track-{}", track.id)), track.clone())
            .context(TrackContextSource::External)
            .is_playing(is_playing)
            .on_tap(move |_, cx| {
                let (track, queue) = (tapped.clone(), (*queue).clone());
                let scope_id = scope_id.clone();
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.play(track, queue, Some(scope_id.as_str()), true, Some(Box::new(Self::show_now_playing)), cx);
                });
            })
    }

    fn top_tracks_section(&self, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let mut section = vstack(ARTIST_TRACK_SPACING).w_full().align(HorizontalAlignment::Leading).child(
            MainText::new(tr("music_top_tracks")).px(16.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary),
        );
        for track in self.top_tracks.iter().take(TOP_TRACKS_SHOWN) {
            section = section.child(self.track_row(track, &self.top_tracks, cx));
        }
        section.into_any_element()
    }

    fn discography(&self, theme: &AppTheme) -> AnyElement {
        let cards = self.albums.iter().filter(|album| !album.is_scheduled()).map(|album| {
            let nav_store = self.nav_store.clone();
            let album_id = album.id.clone();
            AlbumCard::new(self.child_id(format!("album-{}", album.id)), album.title.clone(), album.year.to_string(), album.cover_colors.clone())
                .cover_image_url(Some(album.cover_image_url.clone()))
                .on_tap(move |_, cx| {
                    if let Some(nav_store) = nav_store.as_ref() {
                        let route = RootRoute::MusicAlbumDetail { album_id: album_id.clone() };
                        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
                    }
                })
        });
        vstack(12.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(
                MainText::new(tr("music_discography")).px(16.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary),
            )
            .child(div().id(self.child_id("albums")).flex().w_full().overflow_x_scroll().child(hstack(16.0).children(cards)))
            .into_any_element()
    }

    fn all_tracks_row(&self, index: usize, cx: &mut App) -> AnyElement {
        let Some(track) = self.all_tracks.get(index) else {
            return div().into_any_element();
        };
        let cache_key = MusicServicesStore::tracks_cache_key(None, Some(self.artist_id.as_str()), None);
        let artist_id = self.artist_id.clone();
        let row = self.track_row(track, &self.all_tracks, cx);
        div()
            .w_full()
            .px(px(ARTIST_PAGE_PADDING))
            .pt(px(ARTIST_TRACK_SPACING))
            .child(row.music_pagination_prefetch(
                self.child_id(format!("track-prefetch-{}", track.id)),
                index,
                self.all_tracks.len(),
                &cache_key,
                true,
                8,
                move |_, cx| {
                    MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_artist_tracks(&artist_id, cx));
                },
                cx,
            ))
            .into_any_element()
    }
}

#[derive(IntoElement)]
struct ArtistProfileContent {
    parts: ArtistProfileParts,
    state: Entity<ArtistProfileState>,
}

impl RenderOnce for ArtistProfileContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut parts = self.parts;
        parts.top_inset = list_shell_floating_top_inset(cx);
        let _player = observed_music_player(parts.child_id("player"), window, cx);
        let theme = rise_ui::theme(cx).clone();

        let rows = ArtistProfileRow::rows(parts.all_tracks.len());
        let ids: Vec<SharedString> = rows
            .iter()
            .map(|row| match row {
                ArtistProfileRow::Head => SharedString::new_static("head"),
                ArtistProfileRow::TracksTitle => SharedString::new_static("tracks-title"),
                ArtistProfileRow::Track(index) => SharedString::from(parts.all_tracks[*index].id.clone()),
                ArtistProfileRow::Foot => SharedString::new_static("foot"),
            })
            .collect();
        let list_state: ListState = self.state.update(cx, |state, _| {
            state.list_model.sync(ids);
            state.list_model.list().state().clone()
        });

        let background = theme.bg._100;
        let parts = Rc::new(parts);
        div().size_full().bg(background).child(
            list(list_state, move |index, window, cx| match rows.get(index) {
                Some(row) => parts.row(*row, &theme, window, cx),
                None => div().into_any_element(),
            })
            .size_full(),
        )
    }
}

#[derive(IntoElement)]
struct ArtistProfileNotFound;

impl RenderOnce for ArtistProfileNotFound {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        vstack(0.0)
            .w_full()
            .h_full()
            .pt(px(list_shell_floating_top_inset(cx)))
            .child(
                div()
                    .w_full()
                    .pt(px(60.0))
                    .child(SecondaryText::new(tr("music_artist_not_found")).px(15.0).tac(TextAlignT::Center)),
            )
            .child(spacer())
    }
}

#[derive(IntoElement)]
struct ArtistProfileSkeleton;

impl RenderOnce for ArtistProfileSkeleton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        div().size_full().overflow_hidden().pt(px(list_shell_floating_top_inset(cx))).child(
            vstack(0.0)
                .w_full()
                .align(HorizontalAlignment::Leading)
                .child(div().w_full().child(SkeletonBlock::new(None::<f32>, ARTIST_HERO_HEIGHT, 0.0)))
                .child(
                    vstack(ARTIST_SECTION_SPACING)
                        .w_full()
                        .align(HorizontalAlignment::Leading)
                        .p(px(ARTIST_PAGE_PADDING))
                        .child(
                            vstack(ARTIST_TRACK_SPACING)
                                .align(HorizontalAlignment::Leading)
                                .child(SkeletonLine::new(160.0, 15.0))
                                .child(SkeletonLine::new(110.0, 8.0)),
                        )
                        .child(hstack(12.0).child(SkeletonBlock::new(108.0, 38.0, 19.0)).child(SkeletonCircle::new(56.0)))
                        .child(
                            vstack(ARTIST_TRACK_SPACING)
                                .w_full()
                                .align(HorizontalAlignment::Leading)
                                .child(SkeletonLine::new(120.0, 9.0))
                                .child(
                                    vstack(0.0)
                                        .w_full()
                                        .children((0..3).map(|index| TrackRowSkeleton::new().alternation_index(index))),
                                ),
                        ),
                )
                .skeleton_shimmer(None),
        )
    }
}
