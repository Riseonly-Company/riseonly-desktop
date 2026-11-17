use std::rc::Rc;
use std::sync::Arc;

use gpui::{AnyElement, App, ElementId, FontWeight, ListAlignment, SharedString, Window};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr_plural;
use rise_theme::AppTheme;
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::{IconUi, ListUi, MainText, SecondaryText, TextAlignT};

use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::modules::music::pages::track::track_row::TrackRow;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_pagination_prefetch::MusicPaginationPrefetchModifiers;
use crate::modules::music::stores::music_actions::music_wire_types::{RiseStoredMusicPlaylist, RiseStoredMusicTrack};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::{MusicPaginationState, MusicServicesStore};
use crate::modules::music::widgets::music_cover_placeholder::MusicCoverPlaceholder;
use crate::modules::music::widgets::music_skeletons::TrackListSkeleton;
use crate::modules::music::widgets::music_track_cover_image::MusicCoverImage;

pub(crate) type TrackPickHandler = Rc<dyn Fn(RiseStoredMusicTrack, &mut App)>;
pub(crate) type TrackPickerLoadPage = Rc<dyn Fn(&mut App)>;

const SEARCH_PREFETCH_THRESHOLD: usize = 8;
const MUSIC_PREFETCH_THRESHOLD: usize = 8;

/// list model for the sheet rows
/// gpui keys rows by index so the row ids of the last frame tell which stretch changed
/// expanding a playlist must not throw the scroll away
pub struct TrackPickerListModel {
    list: ListUi,
    ids: Vec<SharedString>,
    measures_all: bool,
}

impl TrackPickerListModel {
    pub fn new(cx: &App) -> Self {
        Self { list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top), ids: Vec::new(), measures_all: false }
    }

    /// for a short list of sections, measures every row and not only the visible ones
    /// gpui counts an unmeasured row as zero height so wheel steps stopped early or jumped to an edge
    pub(crate) fn measuring_all(cx: &App) -> Self {
        let model = Self::new(cx);
        let _ = model.list.state().clone().measure_all();
        Self { measures_all: true, ..model }
    }

    pub fn list(&self) -> &ListUi {
        &self.list
    }

    pub fn sync(&mut self, ids: Vec<SharedString>) {
        if ids == self.ids {
            return;
        }
        let prefix = self.ids.iter().zip(ids.iter()).take_while(|(old, new)| old == new).count();
        let longest_suffix = self.ids.len().min(ids.len()) - prefix;
        let suffix = self
            .ids
            .iter()
            .rev()
            .zip(ids.iter().rev())
            .take(longest_suffix)
            .take_while(|(old, new)| old == new)
            .count();
        let inserted = ids.len() - suffix - prefix;
        self.list.state().splice(prefix..self.ids.len() - suffix, inserted);
        if self.measures_all {
            self.list.state().remeasure_items(prefix..prefix + inserted);
        }
        tracing::warn!(
            target: "riseonly::X1562",
            old = self.ids.len(),
            new = ids.len(),
            prefix,
            suffix,
            measures_all = self.measures_all,
            count = self.list.state().item_count(),
            "track picker list spliced"
        );
        self.ids = ids;
    }
}

struct TrackPickerClosingPlaylist {
    id: String,
    rows: Vec<TrackPickerRow>,
    motion: Animated<f32>,
}

/// expand and collapse of a playlist
/// opened rows fade in, closed ones fade out before leaving the list
/// their rows are kept from the moment of closing since opening another playlist replaces them
pub(crate) struct TrackPickerExpansion {
    pub(crate) expanded_playlist_id: Option<String>,
    motion: Animated<f32>,
    closing: Vec<TrackPickerClosingPlaylist>,
}

impl TrackPickerExpansion {
    const DURATION: f64 = 0.18;

    pub(crate) fn new() -> Self {
        Self { expanded_playlist_id: None, motion: Animated::new(1.0), closing: Vec::new() }
    }

    /// current_rows are the rows of the playlist open until now
    /// returns whether id opened
    pub(crate) fn toggle(&mut self, id: &str, current_rows: Vec<TrackPickerRow>, now: MotionTime, cx: &App) -> bool {
        let opened = self.expanded_playlist_id.as_deref() != Some(id);
        if let Some(previous) = self.expanded_playlist_id.take() {
            let mut motion = Animated::new(self.motion.value(now));
            motion.set_in(0.0, ease_in_out(Self::DURATION), now, cx);
            self.closing.push(TrackPickerClosingPlaylist { id: previous, rows: current_rows, motion });
        }
        if opened {
            let reopened = self.closing.iter().position(|closing| closing.id == id);
            let start = reopened.map_or(0.0, |index| self.closing.remove(index).motion.value(now));
            self.motion.jump(start);
            self.motion.set_in(1.0, ease_in_out(Self::DURATION), now, cx);
            self.expanded_playlist_id = Some(id.to_owned());
        }
        opened
    }

    pub(crate) fn has_closing(&self) -> bool {
        !self.closing.is_empty()
    }

    /// faded rows leave the list, call before the rows of the frame are built
    pub(crate) fn settle(&mut self, now: MotionTime) {
        self.closing.retain(|closing| closing.motion.is_running(now));
    }

    pub(crate) fn closing_rows(&self, playlist_id: &str, now: MotionTime) -> Vec<TrackPickerRow> {
        let Some(closing) = self.closing.iter().find(|closing| closing.id == playlist_id) else {
            return Vec::new();
        };
        let opacity = closing.motion.value(now);
        closing
            .rows
            .iter()
            .map(|row| TrackPickerRow::Closing { row: Box::new(row.clone()), opacity })
            .collect()
    }

    /// opacity of the opened rows and the frames both fades need
    pub(crate) fn frame_value(&self, now: MotionTime, window: &Window) -> f32 {
        for closing in &self.closing {
            closing.motion.frame_value(now, window);
        }
        self.motion.frame_value(now, window)
    }
}

/// how a row asks for the next page
#[derive(Clone)]
pub(crate) enum TrackPickerPagination {
    Music { cache_key: String, page: MusicPaginationState, enabled: bool, load_page: Option<TrackPickerLoadPage> },
    Search { query: String, relative_id: Option<String>, has_more: bool },
}

impl TrackPickerPagination {
    /// music cache cursor for cache_key, read once for the whole section
    pub(crate) fn music(cache_key: String, load_page: TrackPickerLoadPage, cx: &mut App) -> Self {
        let page = MusicServicesStore::shared(cx).update(cx, |service, cx| service.pagination_state(&cache_key, cx));
        Self::Music { cache_key, page, enabled: true, load_page: Some(load_page) }
    }

    /// a section that never loads another page
    pub(crate) fn none() -> Self {
        Self::Music {
            cache_key: MusicServicesStore::liked_cache_key(),
            page: MusicPaginationState { cursor: None, has_more: false },
            enabled: false,
            load_page: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct TrackPickerTrackRow {
    row_id: SharedString,
    index: usize,
    item_count: usize,
    dto: RiseStoredMusicTrack,
    track: TrackModel,
    search_index: Option<usize>,
    pagination: TrackPickerPagination,
}

#[derive(Clone)]
pub(crate) struct TrackPickerPlaylistRow {
    pub(crate) id: String,
    pub(crate) playlist: RiseStoredMusicPlaylist,
    pub(crate) subtitle: String,
    pub(crate) is_expanded: bool,
}

/// one row of the lazily drawn column
/// nested sections are flat here so only the rows on screen get built
#[derive(Clone)]
pub(crate) enum TrackPickerRow {
    SectionTitle { id: &'static str, text: String, top_padding: f32 },
    Loading { id: SharedString, rows: usize },
    StateLabel { id: SharedString, text: String },
    Track(Box<TrackPickerTrackRow>),
    Playlist(Box<TrackPickerPlaylistRow>),
    Expanded(Box<TrackPickerRow>),
    Closing { row: Box<TrackPickerRow>, opacity: f32 },
    BottomPadding,
}

impl TrackPickerRow {
    pub(crate) fn id(&self) -> SharedString {
        match self {
            Self::SectionTitle { id, .. } => SharedString::new_static(id),
            Self::Loading { id, .. } | Self::StateLabel { id, .. } => id.clone(),
            Self::Track(row) => row.row_id.clone(),
            Self::Playlist(row) => format!("playlist/{}", row.id).into(),
            Self::Expanded(row) | Self::Closing { row, .. } => format!("expanded/{}", row.id()).into(),
            Self::BottomPadding => SharedString::new_static("bottom-padding"),
        }
    }
}

/// what every row reads at draw time
#[derive(Clone)]
pub(crate) struct TrackPickerRowContext {
    pub(crate) on_pick: TrackPickHandler,
    pub(crate) on_toggle_playlist: Rc<dyn Fn(&str, &mut App)>,
    pub(crate) highlighted_result: Option<usize>,
    pub(crate) expanded_opacity: f32,
}

pub(crate) fn track_list(
    section: &str,
    tracks: Vec<RiseStoredMusicTrack>,
    pagination: TrackPickerPagination,
) -> Vec<TrackPickerRow> {
    let item_count = tracks.len();
    let mut search_index = 0;
    tracks
        .into_iter()
        .enumerate()
        .filter_map(|(index, dto)| {
            let track = dto.as_track_model()?;
            let is_search = matches!(pagination, TrackPickerPagination::Search { .. });
            let row = TrackPickerTrackRow {
                row_id: format!("{section}/{index}/{}", dto.id.as_deref().unwrap_or_default()).into(),
                index,
                item_count,
                dto,
                track,
                search_index: is_search.then_some(search_index),
                pagination: pagination.clone(),
            };
            search_index += 1;
            Some(TrackPickerRow::Track(Box::new(row)))
        })
        .collect()
}

/// arrow keys step through the results and stop at both ends
pub fn stepped_highlight(current: Option<usize>, delta: isize, count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match current {
        Some(current) => current.saturating_add_signed(delta).min(count - 1),
        None if delta > 0 => 0,
        None => count - 1,
    })
}

pub(crate) fn render_row(
    row: &TrackPickerRow,
    context: &TrackPickerRowContext,
    theme: &AppTheme,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    match row {
        TrackPickerRow::SectionTitle { text, top_padding, .. } => section_title(text, theme, *top_padding),
        TrackPickerRow::Loading { rows, .. } => loading_row(*rows),
        TrackPickerRow::StateLabel { text, .. } => state_label(text),
        TrackPickerRow::Track(row) => track_row(row, context, theme),
        TrackPickerRow::Playlist(row) => playlist_row(row, context, theme, window, cx),
        TrackPickerRow::Expanded(row) => {
            let inner = render_row(row, context, theme, window, cx);
            div().w_full().pl(px(16.0)).opacity(context.expanded_opacity).child(inner).into_any_element()
        }
        TrackPickerRow::Closing { row, opacity } => {
            let inner = render_row(row, context, theme, window, cx);
            div().w_full().pl(px(16.0)).opacity(*opacity).child(inner).into_any_element()
        }
        TrackPickerRow::BottomPadding => div().w_full().h(px(24.0)).into_any_element(),
    }
}

fn track_row(row: &TrackPickerTrackRow, context: &TrackPickerRowContext, theme: &AppTheme) -> AnyElement {
    let on_pick = context.on_pick.clone();
    let dto = row.dto.clone();
    let highlighted = row.search_index.is_some() && row.search_index == context.highlighted_result;
    let content = div()
        .w_full()
        .px(px(16.0))
        .when(highlighted, |content| content.bg(theme.bg._200))
        .child(
            TrackRow::new(row.row_id.clone(), row.track.clone())
                .shows_like_button(false)
                .on_tap(move |_, cx| on_pick(dto.clone(), cx)),
        );
    let prefetch_id = ElementId::NamedChild(Arc::new(row.row_id.clone().into()), SharedString::new_static("prefetch"));
    match &row.pagination {
        TrackPickerPagination::Search { query, relative_id, has_more } => {
            let query = query.clone();
            let options = SaiPaginationPrefetchOptions::new(
                row.index,
                row.item_count,
                MusicServicesStore::search_cache_key(&query, None),
                relative_id.clone(),
                *has_more,
            )
            .threshold(SEARCH_PREFETCH_THRESHOLD);
            content
                .sai_pagination_prefetch(prefetch_id, options, move |_, cx| {
                    let query = query.clone();
                    MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_search(&query, cx));
                })
                .into_any_element()
        }
        TrackPickerPagination::Music { cache_key, page, enabled, load_page } => {
            let load_page = load_page.clone();
            content
                .music_pagination_prefetch_page(
                    prefetch_id,
                    row.index,
                    row.item_count,
                    cache_key,
                    page,
                    *enabled,
                    MUSIC_PREFETCH_THRESHOLD,
                    move |_, cx| {
                        if let Some(load_page) = load_page.as_ref() {
                            load_page(cx);
                        }
                    },
                )
                .into_any_element()
        }
    }
}

fn playlist_row(
    row: &TrackPickerPlaylistRow,
    context: &TrackPickerRowContext,
    theme: &AppTheme,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let target: f32 = if row.is_expanded { 180.0 } else { 0.0 };
    let base_id: ElementId = SharedString::from(format!("track-picker-playlist/{}", row.id)).into();
    let rotation = window.use_keyed_state(
        ElementId::NamedChild(Arc::new(base_id.clone()), SharedString::new_static("chevron")),
        cx,
        |_, _| Animated::new(target),
    );
    let now = RiseClock::monotonic();
    let degrees = rotation.update(cx, |rotation, cx| {
        rotation.set_in(target, ease_in_out(0.18), now, cx);
        rotation.frame_value(now, window)
    });

    let cover = match row.playlist.cover_url.as_deref().filter(|url| !url.is_empty()) {
        Some(url) => MusicCoverImage::new(format!("track-picker-playlist-cover/{}", row.id), url.to_owned())
            .size(44.0)
            .corner_radius(8.0)
            .into_any_element(),
        None => MusicCoverPlaceholder::new().icon_scale(0.4).corner_radius(8.0).into_any_element(),
    };
    let on_toggle = context.on_toggle_playlist.clone();
    let id = row.id.clone();
    plain(base_id)
        .on_press(move |_, cx| on_toggle(&id, cx))
        .w_full()
        .child(
            hstack(14.0)
                .w_full()
                .px(px(16.0))
                .py(px(10.0))
                .child(div().flex_none().size(px(44.0)).rounded(px(8.0)).overflow_hidden().child(cover))
                .child(
                    vstack(3.0)
                        .min_w_0()
                        .items_start()
                        .child(
                            MainText::new(row.playlist.display_title())
                                .px(15.0)
                                .font_weight(FontWeight::MEDIUM)
                                .color(theme.text.primary)
                                .line_limit(1),
                        )
                        .child(SecondaryText::new(row.subtitle.clone()).px(12.0)),
                )
                .child(spacer_min_w(0.0))
                .child(
                    div()
                        .flex_none()
                        .children(IconUi::sized("chevron.down", px(13.0), theme.text.secondary, false))
                        .rotation_effect(degrees),
                ),
        )
        .into_any_element()
}

pub(crate) fn section_title(text: &str, theme: &AppTheme, top_padding: f32) -> AnyElement {
    hstack(0.0)
        .w_full()
        .pt(px(top_padding))
        .px(px(16.0))
        .pb(px(4.0))
        .child(
            MainText::new(SharedString::from(text.to_owned()))
                .px(14.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(theme.text.secondary),
        )
        .child(spacer_min_w(0.0))
        .into_any_element()
}

fn loading_row(rows: usize) -> AnyElement {
    div()
        .w_full()
        .py(px(8.0))
        .child(TrackListSkeleton::new().rows(rows).horizontal_padding(16.0))
        .into_any_element()
}

fn state_label(text: &str) -> AnyElement {
    div()
        .w_full()
        .flex()
        .justify_center()
        .px(px(24.0))
        .py(px(20.0))
        .child(SecondaryText::new(SharedString::from(text.to_owned())).px(13.0).tac(TextAlignT::Center))
        .into_any_element()
}

/// plural track count label shared by both playlist subtitles
pub(crate) fn track_count_label(count: usize) -> String {
    tr_plural("music_track_count", i64::try_from(count).unwrap_or(i64::MAX))
}
