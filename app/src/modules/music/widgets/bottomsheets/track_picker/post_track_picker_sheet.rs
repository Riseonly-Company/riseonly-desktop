use std::rc::Rc;
use std::sync::Arc;

use gpui::{AnyElement, App, Context, ElementId, Entity, FontWeight, SharedString, Subscription, Window, relative};
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::input_ui::input_actions::{Cancel, MoveDown, MoveUp};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::{MotionSpec, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::{IconUi, MainText};

use super::track_picker_shared::{
    TrackPickHandler, TrackPickerExpansion, TrackPickerListModel, TrackPickerPagination, TrackPickerPlaylistRow, TrackPickerRow,
    TrackPickerRowContext, render_row, stepped_highlight, track_count_label, track_list,
};
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::{ProtoTrackRecordExt, RiseStoredMusicTrack};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;

const LOADING_ROWS: usize = 5;
const HEADER_BUTTON_SIDE: f32 = 42.0;
const SEARCH_FIELD_CLOSE_GAP: f32 = 10.0;

enum PostTrackPickerSearchHero {}

impl PostTrackPickerSearchHero {
    fn animation() -> MotionSpec {
        spring(0.42, 0.86)
    }
}

#[derive(IntoElement)]
pub struct PostTrackPickerSheet {
    id: ElementId,
    on_pick: TrackPickHandler,
    on_dismiss: Option<SheetAction>,
}

impl PostTrackPickerSheet {
    pub fn new(id: impl Into<ElementId>, on_pick: impl Fn(RiseStoredMusicTrack, &mut App) + 'static) -> Self {
        Self { id: id.into(), on_pick: Rc::new(on_pick), on_dismiss: None }
    }

    pub fn on_dismiss(mut self, on_dismiss: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(on_dismiss));
        self
    }
}

/// view state, alive as long as the sheet is drawn
struct PostTrackPickerSheetState {
    store: Entity<MusicActionsStore>,
    query: String,
    expansion: TrackPickerExpansion,
    search_in_flight: bool,
    has_own_search_results: bool,
    is_searching: bool,
    search_motion: Animated<f32>,
    highlighted_result: Option<usize>,
    list_model: TrackPickerListModel,
    list_rows: Rc<Vec<TrackPickerRow>>,
    _observations: Vec<Subscription>,
}

impl PostTrackPickerSheetState {
    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = MusicActionsStore::shared(cx);
        let observations = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        Self {
            store,
            query: String::new(),
            expansion: TrackPickerExpansion::new(),
            search_in_flight: false,
            has_own_search_results: false,
            is_searching: false,
            search_motion: Animated::new(0.0),
            highlighted_result: None,
            list_model: TrackPickerListModel::measuring_all(cx),
            list_rows: Rc::new(Vec::new()),
            _observations: observations,
        }
    }
}

#[derive(Clone)]
struct PostTrackPickerSheetParts {
    state: Entity<PostTrackPickerSheetState>,
    on_pick: TrackPickHandler,
    on_dismiss: Option<SheetAction>,
}

impl RenderOnce for PostTrackPickerSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(self.id.clone(), cx, PostTrackPickerSheetState::new);
        let parts = PostTrackPickerSheetParts { state, on_pick: self.on_pick, on_dismiss: self.on_dismiss };

        let header = parts.sheet_header(&theme, window, cx);
        let list = parts.list(&theme, window, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg._100)
            .child(header)
            .child(div().w_full().flex_1().min_h_0().child(list))
            .on_appear(ElementId::NamedChild(Arc::new(self.id), SharedString::new_static("appear")), |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_liked_if_needed(cx);
                    store.load_playlists_if_needed(cx);
                });
            })
    }
}

impl PostTrackPickerSheetParts {
    fn sheet_header(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let now = RiseClock::monotonic();
        let (is_searching, progress) = {
            let state = self.state.read(cx);
            (state.is_searching, state.search_motion.frame_value(now, window))
        };
        let rest_opacity = (1.0 - progress).clamp(0.0, 1.0);
        let search_opacity = progress.clamp(0.0, 1.0);

        let mut header = div().relative().w_full().h(px(HEADER_BUTTON_SIDE));
        if rest_opacity > 0.0 {
            header = header.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(rest_opacity)
                    .child(
                        MainText::new(tr("audio_page_title"))
                            .px(17.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .line_limit(1),
                    ),
            );
        }
        if !is_searching || search_opacity > 0.0 {
            header = header.child(self.search_slot(is_searching, progress, rest_opacity, search_opacity, theme));
        }
        let close_parts = self.clone();
        let header = header.child(
            div()
                .absolute()
                .top_0()
                .left(relative(progress))
                .ml(px(-HEADER_BUTTON_SIDE * progress))
                .child(self.close_button(theme, move |window, cx| {
                    if close_parts.state.read(cx).is_searching {
                        close_parts.close_search(window, cx);
                    } else if let Some(on_dismiss) = close_parts.on_dismiss.as_ref() {
                        on_dismiss(window, cx);
                    }
                })),
        );
        div().w_full().flex_none().px(px(12.0)).pb(px(6.0)).child(header).into_any_element()
    }

    /// magnifier at rest and the field while searching share one frame
    /// it runs from the trailing button square to the header minus the close button
    fn search_slot(
        &self,
        is_searching: bool,
        progress: f32,
        rest_opacity: f32,
        search_opacity: f32,
        theme: &AppTheme,
    ) -> AnyElement {
        let trailing_inset = (HEADER_BUTTON_SIDE + SEARCH_FIELD_CLOSE_GAP) * progress;
        let leading = 1.0 - progress;
        let open_parts = self.clone();
        let mut slot = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(relative(leading))
            .ml(px(-HEADER_BUTTON_SIDE * leading))
            .right(px(trailing_inset));
        if !is_searching {
            slot = slot.child(
                div().absolute().inset_0().flex().justify_end().opacity(rest_opacity).child(
                    LiquidGlassButton::new("post-track-picker-open-search", LiquidGlassButtonShape::Circle)
                        .size(HEADER_BUTTON_SIDE)
                        .accessibility_label(Some(tr("post_music_picker_search")))
                        .on_press(move |_, cx| open_parts.open_search(cx))
                        .children(IconUi::sized("magnifyingglass", px(16.0), theme.text.primary, false)),
                ),
            );
        }
        if search_opacity > 0.0 {
            slot = slot.child(self.search_field(search_opacity));
        }
        slot.into_any_element()
    }

    fn search_field(&self, opacity: f32) -> AnyElement {
        let binding = Binding::entity(
            self.state.clone(),
            |state: &PostTrackPickerSheetState| state.query.clone(),
            |state, query, _| state.query = query,
        );
        let on_change_parts = self.clone();
        let on_submit_parts = self.clone();
        let up_parts = self.clone();
        let down_parts = self.clone();
        let cancel_parts = self.clone();
        div()
            .absolute()
            .inset_0()
            .opacity(opacity)
            .on_action(move |_: &MoveUp, _, cx| up_parts.move_search_highlight(-1, cx))
            .on_action(move |_: &MoveDown, _, cx| down_parts.move_search_highlight(1, cx))
            .capture_action(move |_: &Cancel, window, cx| {
                cancel_parts.close_search(window, cx);
                cx.stop_propagation();
            })
            .child(
                SearchFieldUi::new("post-track-picker-search", binding)
                    .placeholder(SharedString::from(tr("search_placeholder")))
                    .on_change(move |value, _, cx| on_change_parts.run_search(value, cx))
                    .on_submit(move |_, cx| on_submit_parts.pick_highlighted_result(cx))
                    .uses_inline_text_field(true)
                    .auto_focus(true),
            )
            .into_any_element()
    }

    fn list(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let settled_at = RiseClock::monotonic();
        self.state.update(cx, |state, _| state.expansion.settle(settled_at));
        let is_searching = self.state.read(cx).is_searching;
        let mut rows = if is_searching { self.search_section(cx) } else { self.my_music_section(cx) };
        rows.push(TrackPickerRow::BottomPadding);
        let ids: Vec<SharedString> = rows.iter().map(TrackPickerRow::id).collect();
        let now = RiseClock::monotonic();
        let (highlighted_result, expanded_opacity) = {
            let state = self.state.read(cx);
            (state.highlighted_result, state.expansion.frame_value(now, window))
        };
        let rows = Rc::new(rows);
        self.state.update(cx, |state, _| {
            state.list_model.sync(ids);
            state.list_rows = rows;
        });

        let toggle_parts = self.clone();
        let context = TrackPickerRowContext {
            on_pick: self.on_pick.clone(),
            on_toggle_playlist: Rc::new(move |id: &str, cx: &mut App| toggle_parts.toggle_playlist(id, cx)),
            highlighted_result,
            expanded_opacity,
        };
        let row_theme = theme.clone();
        let rows_state = self.state.clone();
        self.state
            .read(cx)
            .list_model
            .list()
            .element(move |index, window, cx| {
                let rows = Rc::clone(&rows_state.read(cx).list_rows);
                match rows.get(index) {
                    Some(row) => render_row(row, &context, &row_theme, window, cx),
                    None => div().into_any_element(),
                }
            })
            .into_any_element()
    }

    fn own_search_tracks(&self, cx: &App) -> Vec<RiseStoredMusicTrack> {
        let state = self.state.read(cx);
        if !state.has_own_search_results {
            return Vec::new();
        }
        Self::selectable_tracks(
            state
                .store
                .read(cx)
                .search_sai
                .data()
                .map(|data| data.tracks.iter().map(ProtoTrackRecordExt::stored).collect()),
        )
    }

    fn search_section(&self, cx: &mut App) -> Vec<TrackPickerRow> {
        let tracks = self.own_search_tracks(cx);
        let state = self.state.read(cx);
        let trimmed = trimming_whitespaces(&state.query).to_owned();
        let slice = Rc::clone(&state.store.read(cx).search_sai);
        if trimmed.is_empty() {
            return vec![Self::state_label("search", tr("music_search_placeholder"))];
        }
        if state.search_in_flight || !state.has_own_search_results {
            return vec![Self::loading_row("search")];
        }
        if let Some(error) = slice.error()
            && tracks.is_empty()
        {
            return vec![Self::state_label("search", error)];
        }
        if tracks.is_empty() {
            return vec![Self::state_label("search", tr("music_no_results"))];
        }
        let data = slice.data();
        track_list(
            "search",
            tracks,
            TrackPickerPagination::Search {
                query: trimmed,
                relative_id: data.as_ref().and_then(|data| data.next_cursor.clone()),
                has_more: data.is_some_and(|data| data.has_more),
            },
        )
    }

    fn my_music_section(&self, cx: &mut App) -> Vec<TrackPickerRow> {
        let mut rows = vec![TrackPickerRow::SectionTitle {
            id: "liked-title",
            text: tr("music_liked_songs"),
            top_padding: 12.0,
        }];
        rows.extend(self.liked_block(cx));
        rows.push(TrackPickerRow::SectionTitle {
            id: "playlists-title",
            text: tr("music_your_playlists"),
            top_padding: 16.0,
        });
        rows.extend(self.playlists_block(cx));
        rows
    }

    fn liked_block(&self, cx: &mut App) -> Vec<TrackPickerRow> {
        let slice = Rc::clone(&self.state.read(cx).store.read(cx).get_liked_sai);
        let tracks =
            Self::selectable_tracks(slice.data().map(|data| data.tracks.iter().map(ProtoTrackRecordExt::stored).collect()));
        if slice.is_pending() && tracks.is_empty() {
            return vec![Self::loading_row("liked")];
        }
        if let Some(error) = slice.error()
            && tracks.is_empty()
        {
            return vec![Self::state_label("liked", error)];
        }
        if tracks.is_empty() {
            return vec![Self::state_label("liked", tr("music_no_results"))];
        }
        let load_page: Rc<dyn Fn(&mut App)> =
            Rc::new(|cx: &mut App| MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_liked(cx)));
        track_list(
            "liked",
            tracks,
            TrackPickerPagination::music(MusicServicesStore::liked_cache_key(), load_page, cx),
        )
    }

    fn playlists_block(&self, cx: &mut App) -> Vec<TrackPickerRow> {
        let slice = Rc::clone(&self.state.read(cx).store.read(cx).get_playlists_mine_sai);
        let playlists = slice.data().and_then(|data| data.playlists).unwrap_or_default();
        if slice.is_pending() && playlists.is_empty() {
            return vec![Self::loading_row("playlists")];
        }
        if let Some(error) = slice.error()
            && playlists.is_empty()
        {
            return vec![Self::state_label("playlists", error)];
        }
        if playlists.is_empty() {
            return vec![Self::state_label("playlists", tr("music_home_playlists_empty"))];
        }
        let now = RiseClock::monotonic();
        let expanded = self.state.read(cx).expansion.expanded_playlist_id.clone();
        let mut rows = Vec::new();
        for playlist in playlists {
            let id = playlist.id.clone().unwrap_or_default();
            let is_expanded = expanded.as_deref() == Some(id.as_str());
            let subtitle = track_count_label(playlist.tracks.as_ref().map_or(0, Vec::len));
            rows.push(TrackPickerRow::Playlist(Box::new(TrackPickerPlaylistRow {
                id: id.clone(),
                playlist,
                subtitle,
                is_expanded,
            })));
            if is_expanded {
                rows.extend(
                    self.expanded_playlist_content(&id, cx).into_iter().map(|row| TrackPickerRow::Expanded(Box::new(row))),
                );
            } else {
                rows.extend(self.state.read(cx).expansion.closing_rows(&id, now));
            }
        }
        rows
    }

    fn expanded_playlist_content(&self, playlist_id: &str, cx: &mut App) -> Vec<TrackPickerRow> {
        let slice = Rc::clone(&self.state.read(cx).store.read(cx).get_playlist_sai);
        let loaded = slice.data().and_then(|data| data.playlist);
        let section = format!("playlist-rows/{playlist_id}");
        let matches = loaded.as_ref().and_then(|loaded| loaded.id.as_deref()).unwrap_or_default() == playlist_id;
        if matches {
            let tracks = Self::selectable_tracks(loaded.and_then(|loaded| loaded.tracks));
            if tracks.is_empty() {
                return vec![Self::state_label(&section, tr("music_no_results"))];
            }
            return track_list(&section, tracks, Self::no_pagination());
        }
        if slice.is_pending() {
            return vec![Self::loading_row(&section)];
        }
        if let Some(error) = slice.error() {
            return vec![Self::state_label(&section, error)];
        }
        vec![Self::loading_row(&section)]
    }

    fn no_pagination() -> TrackPickerPagination {
        TrackPickerPagination::none()
    }

    fn loading_row(section: &str) -> TrackPickerRow {
        TrackPickerRow::Loading { id: format!("{section}/loading").into(), rows: LOADING_ROWS }
    }

    fn state_label(section: &str, text: String) -> TrackPickerRow {
        TrackPickerRow::StateLabel { id: format!("{section}/state").into(), text }
    }

    fn run_search(&self, value: &str, cx: &mut App) {
        let trimmed = trimming_whitespaces(value);

        if trimmed.is_empty() {
            self.state.update(cx, |state, cx| {
                state.search_in_flight = false;
                state.highlighted_result = None;
                cx.notify();
            });
            MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_search(cx));
            return;
        }

        self.state.update(cx, |state, cx| {
            state.search_in_flight = true;
            state.highlighted_result = None;
            cx.notify();
        });
        let answered_state = self.state.clone();
        let on_result: Rc<dyn Fn(String, &mut App)> = Rc::new(move |answered_query: String, cx: &mut App| {
            answered_state.update(cx, |state, cx| {
                if answered_query != trimming_whitespaces(&state.query) {
                    return;
                }
                state.search_in_flight = false;
                state.has_own_search_results = true;
                cx.notify();
            });
        });
        let value = value.to_owned();
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.search(&value, Some(on_result), cx));
    }

    fn visible_search_results(&self, cx: &App) -> Vec<RiseStoredMusicTrack> {
        let state = self.state.read(cx);
        if state.search_in_flight || trimming_whitespaces(&state.query).is_empty() {
            return Vec::new();
        }
        let mut tracks = self.own_search_tracks(cx);
        tracks.retain(|track| track.as_track_model().is_some());
        tracks
    }

    fn move_search_highlight(&self, delta: isize, cx: &mut App) {
        let count = self.visible_search_results(cx).len();
        self.state.update(cx, |state, cx| {
            state.highlighted_result = stepped_highlight(state.highlighted_result, delta, count);
            if let Some(index) = state.highlighted_result {
                state.list_model.list().state().scroll_to_reveal_item(index);
            }
            cx.notify();
        });
    }

    fn pick_highlighted_result(&self, cx: &mut App) {
        let index = self.state.read(cx).highlighted_result.unwrap_or(0);
        if let Some(track) = self.visible_search_results(cx).into_iter().nth(index) {
            (self.on_pick)(track, cx);
        }
    }

    fn close_button(&self, theme: &AppTheme, action: impl Fn(&mut Window, &mut App) + 'static) -> LiquidGlassButton {
        LiquidGlassButton::new("post-track-picker-close", LiquidGlassButtonShape::Circle)
            .size(HEADER_BUTTON_SIDE)
            .accessibility_label(Some(tr("cancel")))
            .on_press(action)
            .children(IconUi::sized("xmark", px(15.0), theme.text.secondary, false))
    }

    fn open_search(&self, cx: &mut App) {
        let now = RiseClock::monotonic();
        self.state.update(cx, |state, cx| {
            state.is_searching = true;
            state.search_motion.set_in(1.0, PostTrackPickerSearchHero::animation(), now, cx);
            cx.notify();
        });
    }

    fn close_search(&self, window: &mut Window, cx: &mut App) {
        ScreenFocus::release_text_field(window, cx);
        let now = RiseClock::monotonic();
        self.state.update(cx, |state, cx| {
            state.query = String::new();
            state.search_in_flight = false;
            state.has_own_search_results = false;
            state.highlighted_result = None;
            state.is_searching = false;
            state.search_motion.set_in(0.0, PostTrackPickerSearchHero::animation(), now, cx);
            cx.notify();
        });
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_search(cx));
    }

    fn toggle_playlist(&self, id: &str, cx: &mut App) {
        if id.is_empty() {
            return;
        }
        let now = RiseClock::monotonic();
        let previous = self.state.read(cx).expansion.expanded_playlist_id.clone();
        let previous_rows = previous.map(|previous| self.expanded_playlist_content(&previous, cx)).unwrap_or_default();
        let opened = self.state.update(cx, |state, cx| {
            let opened = state.expansion.toggle(id, previous_rows, now, cx);
            cx.notify();
            opened
        });
        if opened {
            MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.open_playlist(id, cx));
        }
    }

    fn selectable_tracks(tracks: Option<Vec<RiseStoredMusicTrack>>) -> Vec<RiseStoredMusicTrack> {
        let mut tracks = tracks.unwrap_or_default();
        tracks.retain(|track| {
            let status = track.status.as_deref().unwrap_or(RiseMusicTrackStatus::Published.wire());
            status == RiseMusicTrackStatus::Published.wire() || status == RiseMusicTrackStatus::LocalLibrary.wire()
        });
        tracks
    }
}
