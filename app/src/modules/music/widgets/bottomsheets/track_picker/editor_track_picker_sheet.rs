use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, FontWeight, Hsla, PromptButton, PromptLevel, Render, SharedString,
    Subscription, TextAlign, Window,
};
use rise_canon::RiseUploadFormat;
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::{trimming_whitespaces, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::{FormatArg, format_printf, tr};
use rise_platform::uniform_type::UTType;
use rise_theme::{AppTheme, color};

use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::input_ui::input_actions::{Cancel, MoveDown, MoveUp};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::progress_view::progress_view_value;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};
use rise_ui::{ButtonUi, IconUi, MainText, SecondaryText, TextAlignT};

use super::track_picker_shared::{
    TrackPickHandler, TrackPickerExpansion, TrackPickerListModel, TrackPickerPagination, TrackPickerPlaylistRow, TrackPickerRow,
    TrackPickerRowContext, render_row, section_title, stepped_highlight, track_count_label, track_list,
};
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder};
use crate::core::ui::input_field_accessory::InputFieldAccessory;
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_presenter_resolver::MediaPickerPresenterResolver;
use crate::modules::media_picker::api::media_picker_result::MediaPickerTrackSelection;
use crate::modules::media_picker::send::document_export_bridge::DocumentExportBridge;
use crate::modules::media_picker::shared::media_picker_recent_files::{
    MediaPickerRecentFile, MediaPickerRecentFiles, MediaPickerRecentKind,
};
use crate::modules::music::shared::music_field_rules::MusicFieldRules;
use crate::modules::music::shared::utils::music::music_track_file_upload::MusicTrackFileUpload;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_types::MusicLocalPlaylistDestination;
use crate::modules::music::stores::music_actions::music_wire_types::{
    ProtoTrackRecordExt, RiseStoredMusicPlaylist, RiseStoredMusicTrack,
};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_guide_sandbox_hooks::MusicGuideSandboxHooks;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::music_cover_picker_ui::MusicCoverPickerUi;
use crate::modules::music::widgets::music_track_cover_image::MusicTrackCoverImage;

type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;

const MINIMUM_AUDIO_DURATION_MS: i64 = 500;
const UPLOAD_AUDIO_ICON_BACKGROUND_HEX: &str = "#7E57F7";
const RECENTS_LIMIT: usize = 10;
const LOADING_ROWS: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditorTrackPickerSheetPhase {
    Library,
    Upload,
}

/// media editor music library
/// search over the users own library
/// plus an inline upload flow for own mp3s
#[derive(IntoElement)]
pub struct EditorTrackPickerSheet {
    id: ElementId,
    attached_track: Option<MediaPickerTrackSelection>,
    on_pick: TrackPickHandler,
    on_edit_attached: SheetAction,
    on_remove_attached: SheetAction,
    on_dismiss: SheetAction,
}

impl EditorTrackPickerSheet {
    pub fn new(
        id: impl Into<ElementId>,
        attached_track: Option<MediaPickerTrackSelection>,
        on_pick: impl Fn(RiseStoredMusicTrack, &mut App) + 'static,
        on_edit_attached: impl Fn(&mut Window, &mut App) + 'static,
        on_remove_attached: impl Fn(&mut Window, &mut App) + 'static,
        on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            attached_track,
            on_pick: Rc::new(on_pick),
            on_edit_attached: Rc::new(on_edit_attached),
            on_remove_attached: Rc::new(on_remove_attached),
            on_dismiss: Rc::new(on_dismiss),
        }
    }
}

/// the sheet as a view of its own, drawn cached by the media editor
pub struct EditorTrackPickerSheetView {
    attached_track: Option<MediaPickerTrackSelection>,
    on_pick: TrackPickHandler,
    on_edit_attached: SheetAction,
    on_remove_attached: SheetAction,
    on_dismiss: SheetAction,
}

impl EditorTrackPickerSheetView {
    pub fn new(
        attached_track: Option<MediaPickerTrackSelection>,
        on_pick: impl Fn(RiseStoredMusicTrack, &mut App) + 'static,
        on_edit_attached: impl Fn(&mut Window, &mut App) + 'static,
        on_remove_attached: impl Fn(&mut Window, &mut App) + 'static,
        on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            attached_track,
            on_pick: Rc::new(on_pick),
            on_edit_attached: Rc::new(on_edit_attached),
            on_remove_attached: Rc::new(on_remove_attached),
            on_dismiss: Rc::new(on_dismiss),
        }
    }

    /// rerenders the sheet only when the attached track really changed
    pub fn set_attached_track(&mut self, attached_track: Option<MediaPickerTrackSelection>, cx: &mut Context<Self>) {
        if self.attached_track != attached_track {
            self.attached_track = attached_track;
            cx.notify();
        }
    }
}

impl Render for EditorTrackPickerSheetView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        EditorTrackPickerSheet {
            id: ElementId::Name(SharedString::new_static("media-editor-track-picker-sheet")),
            attached_track: self.attached_track.clone(),
            on_pick: self.on_pick.clone(),
            on_edit_attached: self.on_edit_attached.clone(),
            on_remove_attached: self.on_remove_attached.clone(),
            on_dismiss: self.on_dismiss.clone(),
        }
    }
}

/// view state, alive as long as the sheet is drawn
struct EditorTrackPickerSheetState {
    store: Entity<MusicActionsStore>,
    services: Entity<MusicServicesStore>,
    recent_files: Entity<MediaPickerRecentFiles>,
    phase: EditorTrackPickerSheetPhase,
    query: String,
    expansion: TrackPickerExpansion,
    search_in_flight: bool,
    has_own_search_results: bool,
    upload_request_id: String,
    upload_temp_id: String,
    upload_title: String,
    upload_artist: String,
    upload_file_name: String,
    upload_duration_ms: i64,
    upload_audio_file: Option<SaiWsOptimisticLocalFile>,
    upload_cover_file: Option<SaiWsOptimisticLocalFile>,
    upload_cover_preview: Option<CleverImage>,
    is_waiting_for_created_track: bool,
    upload_error_message: Option<String>,
    upload_pick_generation: u64,
    highlighted_result: Option<usize>,
    list_model: TrackPickerListModel,
    library_rows: Rc<Vec<EditorTrackPickerRow>>,
    library_rows_key: Option<EditorTrackPickerRowsKey>,
    rows_generation: u64,
    library_presence: Entity<Presence<()>>,
    upload_presence: Entity<Presence<()>>,
    save_bar_presence: Entity<Presence<()>>,
    _observations: Vec<Subscription>,
}

impl EditorTrackPickerSheetState {
    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = MusicActionsStore::shared(cx);
        let services = MusicServicesStore::shared(cx);
        let recent_files = MediaPickerRecentFiles::shared(cx);
        let observations = vec![
            cx.observe(&store, |state: &mut Self, _, cx| {
                state.rows_generation += 1;
                cx.notify();
            }),
            cx.observe(&services, |state: &mut Self, _, cx| {
                state.rows_generation += 1;
                cx.notify();
            }),
            cx.observe(&recent_files, |state: &mut Self, _, cx| {
                state.rows_generation += 1;
                cx.notify();
            }),
        ];
        let library_transition = Transition::move_edge(Edge::Leading).combined(Transition::opacity());
        let upload_transition = Transition::move_edge(Edge::Trailing).combined(Transition::opacity());
        Self {
            store,
            services,
            recent_files,
            phase: EditorTrackPickerSheetPhase::Library,
            query: String::new(),
            expansion: TrackPickerExpansion::new(),
            search_in_flight: false,
            has_own_search_results: false,
            upload_request_id: Uuid::new_v4().uuid_string().to_lowercase(),
            upload_temp_id: format!("temp_local_{}", Uuid::new_v4().uuid_string()),
            upload_title: String::new(),
            upload_artist: String::new(),
            upload_file_name: String::new(),
            upload_duration_ms: 0,
            upload_audio_file: None,
            upload_cover_file: None,
            upload_cover_preview: None,
            is_waiting_for_created_track: false,
            upload_error_message: None,
            upload_pick_generation: 0,
            highlighted_result: None,
            list_model: TrackPickerListModel::measuring_all(cx),
            library_rows: Rc::new(Vec::new()),
            library_rows_key: None,
            rows_generation: 0,
            library_presence: cx.new(|_| Presence::new(Some(())).with_transition(library_transition)),
            upload_presence: cx.new(|_| Presence::new(None).with_transition(upload_transition)),
            save_bar_presence: cx.new(|_| Presence::new(None).with_transition(Transition::opacity())),
            _observations: observations,
        }
    }
}

#[derive(Clone)]
struct EditorTrackPickerSheetParts {
    state: Entity<EditorTrackPickerSheetState>,
    attached_track: Option<MediaPickerTrackSelection>,
    on_pick: TrackPickHandler,
    on_edit_attached: SheetAction,
    on_remove_attached: SheetAction,
    on_dismiss: SheetAction,
}

impl RenderOnce for EditorTrackPickerSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(self.id.clone(), cx, EditorTrackPickerSheetState::new);
        let parts = EditorTrackPickerSheetParts {
            state,
            attached_track: self.attached_track,
            on_pick: self.on_pick,
            on_edit_attached: self.on_edit_attached,
            on_remove_attached: self.on_remove_attached,
            on_dismiss: self.on_dismiss,
        };

        let child_id = |name: &'static str| ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name));
        parts.alert(child_id("upload-error"), window, cx);

        let header = parts.header(&theme);
        let body = parts.body(&theme, window, cx);
        let save_bar = parts.upload_save_bar_layer(&theme, window, cx);

        let disappearing = parts.state.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg._100)
            .child(header)
            .child(body)
            .children(save_bar)
            .on_appear(child_id("appear"), |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_liked_if_needed(cx);
                    store.load_playlists_if_needed(cx);
                    store.load_recent_if_needed(cx);
                    store.load_my_local_tracks_if_needed(cx);
                });
            })
            .on_disappear(child_id("disappear"), move |_, cx| {
                let state = disappearing.read(cx);
                if state.is_waiting_for_created_track {
                    let temp_id = state.upload_temp_id.clone();
                    MusicInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.clear_local_track_creation_observer(&temp_id, cx));
                }
            })
    }
}

impl EditorTrackPickerSheetParts {
    /// modal alert of the window for upload errors, clears the message once answered
    fn alert(&self, id: ElementId, window: &mut Window, cx: &mut App) {
        let message = self.state.read(cx).upload_error_message.clone();
        let state = self.state.clone();
        on_change(id, message, false, window, cx, move |_, message, window, cx| {
            let Some(message) = message else {
                return;
            };
            let answer = window.prompt(
                PromptLevel::Info,
                &tr("music_local_upload_error_title"),
                Some(message.as_str()),
                &[PromptButton::cancel(tr("ok"))],
                cx,
            );
            cx.spawn(async move |cx| {
                let _ = answer.await;
                state.update(cx, |state, cx| {
                    state.upload_error_message = None;
                    cx.notify();
                });
            })
            .detach();
        });
    }

    fn header(&self, theme: &AppTheme) -> EditorTrackPickerSheetHeader {
        EditorTrackPickerSheetHeader { theme: theme.clone(), state: self.state.clone(), on_dismiss: self.on_dismiss.clone() }
    }

    fn body(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let now = RiseClock::monotonic();
        let spec = spring(0.34, 0.88);
        let (phase, library_presence, upload_presence) = {
            let state = self.state.read(cx);
            (state.phase, state.library_presence.clone(), state.upload_presence.clone())
        };
        library_presence.update(cx, |presence, cx| {
            presence.set_in((phase == EditorTrackPickerSheetPhase::Library).then_some(()), spec, now, cx);
        });
        upload_presence.update(cx, |presence, cx| {
            presence.set_in((phase == EditorTrackPickerSheetPhase::Upload).then_some(()), spec, now, cx);
        });

        let library = library_presence.read(cx).value(now).is_some().then(|| self.library_content(theme, window, cx));
        let upload = upload_presence.read(cx).value(now).is_some().then(|| {
            scroll_view("editor-track-picker-upload")
                .shows_indicators(false)
                .child(div().w_full().pb(px(24.0)).child(self.upload_content(theme, cx)))
                .into_any_element()
        });

        let library = library_presence
            .read(cx)
            .render_transitioned(now, window, move |_| div().absolute().inset_0().flex().flex_col().children(library));
        let upload = upload_presence
            .read(cx)
            .render_transitioned(now, window, move |_| div().absolute().inset_0().children(upload));
        div().relative().w_full().flex_1().min_h_0().overflow_hidden().children(library).children(upload).into_any_element()
    }

    fn library_content(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let binding = Binding::entity(
            self.state.clone(),
            |state: &EditorTrackPickerSheetState| state.query.clone(),
            |state, query, _| state.query = query,
        );
        let on_change_parts = self.clone();
        let on_submit_parts = self.clone();
        let up_parts = self.clone();
        let down_parts = self.clone();
        let cancel_state = self.state.clone();
        let field = div()
            .w_full()
            .flex_none()
            .px(px(12.0))
            .pb(px(8.0))
            .on_action(move |_: &MoveUp, _, cx| up_parts.move_search_highlight(-1, cx))
            .on_action(move |_: &MoveDown, _, cx| down_parts.move_search_highlight(1, cx))
            .capture_action(move |_: &Cancel, window, cx| {
                if cancel_state.read(cx).query.is_empty() {
                    ScreenFocus::release_text_field(window, cx);
                    cx.stop_propagation();
                }
            })
            .child(
                SearchFieldUi::new("editor-track-picker-search", binding)
                    .placeholder(SharedString::from(tr("search_placeholder")))
                    .on_change(move |value, _, cx| on_change_parts.run_search(value, cx))
                    .on_submit(move |_, cx| on_submit_parts.pick_highlighted_result(cx))
                    .uses_inline_text_field(true),
            );

        let settled_at = RiseClock::monotonic();
        self.state.update(cx, |state, _| state.expansion.settle(settled_at));
        self.refresh_library_rows(cx);
        let now = RiseClock::monotonic();
        let (highlighted_result, expanded_opacity) = {
            let state = self.state.read(cx);
            (state.highlighted_result, state.expansion.frame_value(now, window))
        };

        let toggle_parts = self.clone();
        let context = TrackPickerRowContext {
            on_pick: self.on_pick.clone(),
            on_toggle_playlist: Rc::new(move |id: &str, cx: &mut App| toggle_parts.toggle_playlist(id, cx)),
            highlighted_result,
            expanded_opacity,
        };
        let row_parts = self.clone();
        let row_theme = theme.clone();
        let rows_state = self.state.clone();
        let list = self.state.read(cx).list_model.list().element(move |index, window, cx| {
            let rows = Rc::clone(&rows_state.read(cx).library_rows);
            match rows.get(index) {
                Some(EditorTrackPickerRow::Attached(selection)) => row_parts.attached_row(selection, &row_theme),
                Some(EditorTrackPickerRow::UploadEntry) => row_parts.upload_entry_row(&row_theme),
                Some(EditorTrackPickerRow::Shared(row)) => render_row(row, &context, &row_theme, window, cx),
                None => {
                    tracing::warn!(target: "riseonly::X1646", index, rows = rows.len(), "track picker row past the rows");
                    div().into_any_element()
                }
            }
        });
        vstack(0.0)
            .size_full()
            .child(field)
            .child(div().w_full().flex_1().min_h_0().child(list))
            .into_any_element()
    }

    fn refresh_library_rows(&self, cx: &mut App) {
        let (key, fading) = {
            let state = self.state.read(cx);
            let store = state.store.read(cx);
            let key = EditorTrackPickerRowsKey {
                generation: state.rows_generation,
                slices: [
                    store.search_sai.revision(),
                    store.get_recent_sai.revision(),
                    store.get_liked_sai.revision(),
                    store.get_playlists_mine_sai.revision(),
                    store.get_playlist_sai.revision(),
                    store.list_my_local_tracks_sai.revision(),
                ],
                query: state.query.clone(),
                search_in_flight: state.search_in_flight,
                has_own_search_results: state.has_own_search_results,
                attached_track: self.attached_track.clone(),
                expanded_playlist_id: state.expansion.expanded_playlist_id.clone(),
            };
            (key, state.expansion.has_closing())
        };
        if !fading && self.state.read(cx).library_rows_key.as_ref() == Some(&key) {
            return;
        }
        let rows = self.library_rows(cx);
        let ids: Vec<SharedString> = rows.iter().map(EditorTrackPickerRow::id).collect();
        self.state.update(cx, |state, _| {
            state.list_model.sync(ids);
            state.library_rows = Rc::new(rows);
            state.library_rows_key = (!fading).then_some(key);
        });
    }

    fn library_rows(&self, cx: &mut App) -> Vec<EditorTrackPickerRow> {
        let trimmed = trimming_whitespaces(&self.state.read(cx).query).to_owned();
        let mut rows = if trimmed.is_empty() {
            self.library_sections(cx)
        } else {
            self.search_results(&trimmed, cx).into_iter().map(EditorTrackPickerRow::Shared).collect()
        };
        rows.push(EditorTrackPickerRow::Shared(TrackPickerRow::BottomPadding));
        rows
    }

    fn own_search_tracks(&self, cx: &mut App) -> Vec<RiseStoredMusicTrack> {
        let state = self.state.read(cx);
        if !state.has_own_search_results {
            return Vec::new();
        }
        let tracks = state
            .store
            .read(cx)
            .search_sai
            .data()
            .map(|data| data.tracks.iter().map(ProtoTrackRecordExt::stored).collect::<Vec<_>>());
        Self::selectable_tracks(tracks, cx)
    }

    fn search_results(&self, trimmed: &str, cx: &mut App) -> Vec<TrackPickerRow> {
        let tracks = self.own_search_tracks(cx);
        let state = self.state.read(cx);
        let slice = Rc::clone(&state.store.read(cx).search_sai);
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
                query: trimmed.to_owned(),
                relative_id: data.as_ref().and_then(|data| data.next_cursor.clone()),
                has_more: data.is_some_and(|data| data.has_more),
            },
        )
    }

    fn library_sections(&self, cx: &mut App) -> Vec<EditorTrackPickerRow> {
        let mut rows = Vec::new();
        if let Some(selection) = self.attached_track.clone() {
            rows.push(EditorTrackPickerRow::Attached(selection));
        }
        rows.push(EditorTrackPickerRow::UploadEntry);

        let recent = self
            .state
            .read(cx)
            .store
            .read(cx)
            .get_recent_sai
            .data()
            .map(|data| data.tracks.iter().map(RiseStoredMusicTrack::from_canon).collect::<Vec<_>>());
        let mut recents = MusicGuideSandboxHooks::composer_suggested_tracks(cx);
        recents.extend(Self::selectable_tracks(recent, cx));
        let mut shared = Vec::new();
        if !recents.is_empty() {
            recents.truncate(RECENTS_LIMIT);
            shared.push(Self::section_title_row("recent-title", tr("music_recently_played")));
            shared.extend(track_list("recent", recents, Self::no_pagination()));
        }
        shared.push(Self::section_title_row("liked-title", tr("music_liked_songs")));
        shared.extend(self.liked_block(cx));
        shared.push(Self::section_title_row("playlists-title", tr("music_your_playlists")));
        shared.extend(self.playlists_block(cx));
        shared.push(Self::section_title_row("local-title", tr("music_local_tracks_title")));
        shared.extend(self.local_tracks_block(cx));
        rows.extend(shared.into_iter().map(EditorTrackPickerRow::Shared));
        rows
    }

    fn attached_row(&self, selection: &MediaPickerTrackSelection, theme: &AppTheme) -> AnyElement {
        let on_edit = self.on_edit_attached.clone();
        let on_remove = self.on_remove_attached.clone();
        let cover_url = RemoteMediaURL::optional_normalized(selection.track.cover_url.as_deref()).unwrap_or_default();
        let range = format!(
            "{}\u{2013}{}",
            RiseFormat::duration_milliseconds(selection.start_ms),
            RiseFormat::duration_milliseconds(selection.end_ms)
        );
        div()
            .w_full()
            .pt(px(4.0))
            .px(px(12.0))
            .child(
                hstack(10.0)
                    .w_full()
                    .px(px(12.0))
                    .py(px(8.0))
                    .bg(theme.bg._200)
                    .rounded(px(14.0))
                    .child(
                        plain("editor-track-picker-attached")
                            .on_press(move |window, cx| on_edit(window, cx))
                            .flex_1()
                            .min_w_0()
                            .child(
                                hstack(12.0)
                                    .w_full()
                                    .child(
                                        MusicTrackCoverImage::new("editor-track-picker-attached-cover", cover_url)
                                            .size(40.0)
                                            .corner_radius(8.0),
                                    )
                                    .child(
                                        vstack(3.0)
                                            .min_w_0()
                                            .items_start()
                                            .child(
                                                MainText::new(selection.track.artist_title_line())
                                                    .px(14.0)
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .number_of_lines(1),
                                            )
                                            .child(SecondaryText::new(range).px(12.0)),
                                    )
                                    .child(spacer_min_w(0.0)),
                            ),
                    )
                    .child(
                        plain("editor-track-picker-attached-remove")
                            .on_press(move |window, cx| on_remove(window, cx))
                            .flex_none()
                            .size(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(theme.bg._300)
                            .rounded_full()
                            .children(IconUi::sized("xmark", px(12.0), theme.text.secondary, false)),
                    ),
            )
            .into_any_element()
    }

    fn upload_entry_row(&self, theme: &AppTheme) -> AnyElement {
        let state = self.state.clone();
        div()
            .w_full()
            .pt(px(8.0))
            .px(px(12.0))
            .child(
                plain("editor-track-picker-upload-entry")
                    .on_press(move |_, cx| {
                        state.update(cx, |state, cx| {
                            state.phase = EditorTrackPickerSheetPhase::Upload;
                            cx.notify();
                        });
                        MusicInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.load_local_playlist_destinations_if_needed(cx));
                    })
                    .w_full()
                    .child(
                        hstack(12.0)
                            .w_full()
                            .px(px(12.0))
                            .py(px(8.0))
                            .bg(theme.bg._200)
                            .rounded(px(14.0))
                            .child(
                                div()
                                    .flex_none()
                                    .size(px(40.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(theme.primary._100.opacity(0.16))
                                    .rounded(px(8.0))
                                    .children(IconUi::sized("square.and.arrow.up", px(16.0), theme.primary._100, false)),
                            )
                            .child(MainText::new(tr("music_upload_track")).px(14.0).font_weight(FontWeight::MEDIUM))
                            .child(spacer_min_w(0.0))
                            .children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(13.0), theme.text.secondary, false)),
                    ),
            )
            .into_any_element()
    }

    fn liked_block(&self, cx: &mut App) -> Vec<TrackPickerRow> {
        let slice = Rc::clone(&self.state.read(cx).store.read(cx).get_liked_sai);
        let tracks = Self::selectable_tracks(
            slice.data().map(|data| data.tracks.iter().map(ProtoTrackRecordExt::stored).collect()),
            cx,
        );
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

    fn local_tracks_block(&self, cx: &mut App) -> Vec<TrackPickerRow> {
        let (services, slice) = {
            let state = self.state.read(cx);
            (state.services.clone(), Rc::clone(&state.store.read(cx).list_my_local_tracks_sai))
        };
        let page = services.update(cx, |services, cx| services.local_tracks_page(cx));
        let tracks = Self::selectable_tracks(Some(page.tracks), cx);
        if slice.is_pending() && tracks.is_empty() {
            return vec![Self::loading_row("local")];
        }
        if let Some(error) = slice.error()
            && tracks.is_empty()
        {
            return vec![Self::state_label("local", error)];
        }
        if tracks.is_empty() {
            return vec![Self::state_label("local", tr("music_local_tracks_empty"))];
        }
        let load_page: Rc<dyn Fn(&mut App)> =
            Rc::new(|cx: &mut App| MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_my_local_tracks(cx)));
        track_list(
            "local",
            tracks,
            TrackPickerPagination::music(MusicServicesStore::my_local_tracks_cache_key(), load_page, cx),
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
            let subtitle = Self::playlist_subtitle(&playlist, cx);
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
            let loaded_tracks = loaded.and_then(|loaded| loaded.tracks);
            let has_loaded_tracks = loaded_tracks.as_ref().is_some_and(|tracks| !tracks.is_empty());
            let tracks = Self::selectable_tracks(loaded_tracks, cx);
            if tracks.is_empty() {
                let key = if has_loaded_tracks { "music_playlist_shared_local_hidden" } else { "music_local_playlist_empty" };
                return vec![Self::state_label(&section, tr(key))];
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

    fn upload_progress(&self, cx: &App) -> f64 {
        self.state
            .read(cx)
            .store
            .read(cx)
            .create_local_track_in_playlist_sai
            .data()
            .and_then(|data| data.upload_progress)
            .unwrap_or(0.0)
    }

    fn is_uploading(&self, cx: &App) -> bool {
        let state = self.state.read(cx);
        let slice = &state.store.read(cx).create_local_track_in_playlist_sai;
        state.is_waiting_for_created_track
            || slice.is_pending()
            || slice.data().and_then(|data| data.is_uploading) == Some(true)
    }

    /// never shown to the user, the upload is an original sound and its playlist is internal
    fn resolved_upload_destination(&self, cx: &mut App) -> Option<MusicLocalPlaylistDestination> {
        let services = self.state.read(cx).services.clone();
        services.update(cx, |services, cx| {
            services.local_playlist_destinations(cx).into_iter().find(|destination| destination.can_add_tracks)
        })
    }

    fn can_start_upload(&self, cx: &mut App) -> bool {
        let is_uploading = self.is_uploading(cx);
        let has_destination = self.resolved_upload_destination(cx).is_some();
        let state = self.state.read(cx);
        let clean_title = trimming_whitespaces_and_newlines(&state.upload_title);
        let clean_artist = trimming_whitespaces_and_newlines(&state.upload_artist);
        state.upload_audio_file.is_some()
            && state.upload_duration_ms >= MINIMUM_AUDIO_DURATION_MS
            && !clean_title.is_empty()
            && !clean_artist.is_empty()
            && MusicFieldRules::local_track_title().is_satisfied(clean_title)
            && MusicFieldRules::local_track_artist().is_satisfied(clean_artist)
            && has_destination
            && !is_uploading
    }

    fn upload_content(&self, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let has_audio = self.state.read(cx).upload_audio_file.is_some();
        let mut content = vstack(14.0).w_full().pt(px(4.0));
        if !has_audio {
            content = content
                .child(GroupedBtns::new("editor-track-picker-choose", vec![self.choose_file_item(cx)]))
                .children(self.recent_audio_section(theme, cx));
        } else {
            let progress = self.upload_progress(cx);
            let show_overlay = self.is_uploading(cx) && self.state.read(cx).upload_cover_file.is_some();
            let cover_file = Binding::entity(
                self.state.clone(),
                |state: &EditorTrackPickerSheetState| state.upload_cover_file.clone(),
                |state, file, _| state.upload_cover_file = file,
            );
            let cover_preview = Binding::entity(
                self.state.clone(),
                |state: &EditorTrackPickerSheetState| state.upload_cover_preview.clone(),
                |state, preview, _| state.upload_cover_preview = preview,
            );
            content = content
                .child(
                    MusicCoverPickerUi::new("editor-track-picker-cover", cover_file, cover_preview)
                        .placeholder_label(Some(tr("music_local_upload_cover_optional")))
                        .cover_upload_progress(progress)
                        .show_upload_overlay(show_overlay)
                        .size(116.0)
                        .corner_radius(58.0)
                        .picker_configuration(MediaPickerConfiguration::track_cover_inside_editor()),
                )
                .child(GroupedBtns::new("editor-track-picker-form", self.upload_items(theme, cx)));
            let services = self.state.read(cx).services.clone();
            let destinations_loading = services.update(cx, |services, cx| services.local_playlist_destinations_loading(cx));
            if self.resolved_upload_destination(cx).is_none() && !destinations_loading {
                content = content.child(
                    div().w_full().px(px(16.0)).child(
                        SecondaryText::new(tr("music_message_import_no_destinations")).px(13.0).tac(TextAlignT::Center),
                    ),
                );
            }
        }
        content.into_any_element()
    }

    fn upload_audio_icon_background() -> Hsla {
        color(UPLOAD_AUDIO_ICON_BACKGROUND_HEX)
    }

    fn choose_file_item(&self, cx: &App) -> GroupedBtn {
        let file_name = self.state.read(cx).upload_file_name.clone();
        let text = if file_name.is_empty() { tr("music_local_upload_choose_mp3") } else { file_name };
        let parts = self.clone();
        GroupedBtn::new("editorUploadAudio", text)
            .icon("waveform")
            .icon_background(Self::upload_audio_icon_background())
            .subtitle(self.upload_subtitle(cx))
            .disabled(self.is_uploading(cx))
            .callback(move |_, cx| parts.present_upload_audio_picker(cx))
    }

    fn recent_audio_section(&self, theme: &AppTheme, cx: &mut App) -> Option<AnyElement> {
        let recent_files = self.state.read(cx).recent_files.clone();
        let recents: Vec<MediaPickerRecentFile> = recent_files
            .update(cx, |files, cx| files.files("", MediaPickerRecentKind::Audio, cx))
            .into_iter()
            .filter(|entry| {
                entry
                    .local_url()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(str::to_lowercase)
                    .is_some_and(|extension| MusicTrackFileUpload::MP3_EXTENSIONS.iter().any(|known| *known == extension))
            })
            .collect();
        if recents.is_empty() {
            return None;
        }
        let is_uploading = self.is_uploading(cx);
        Some(
            vstack(0.0)
                .w_full()
                .child(section_title(&tr("media_picker_recently_sent"), theme, 8.0))
                .children(recents.iter().map(|entry| self.recent_audio_row(entry, is_uploading, theme)))
                .into_any_element(),
        )
    }

    fn recent_audio_row(&self, entry: &MediaPickerRecentFile, is_uploading: bool, theme: &AppTheme) -> AnyElement {
        let parts = self.clone();
        let picked = entry.clone();
        let background = Self::upload_audio_icon_background();
        plain(SharedString::from(format!("editor-track-picker-recent/{}", entry.id)))
            .disabled(is_uploading)
            .on_press(move |_, cx| {
                parts.handle_upload_audio_picked(
                    &picked.local_url(),
                    Some(picked.display_name.clone()),
                    picked.title.clone(),
                    picked.artist.clone(),
                    cx,
                );
            })
            .w_full()
            .child(
                hstack(12.0)
                    .w_full()
                    .px(px(16.0))
                    .py(px(8.0))
                    .child(
                        div()
                            .flex_none()
                            .size(px(40.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(background.opacity(0.16))
                            .rounded(px(8.0))
                            .children(IconUi::sized("music.note", px(16.0), background, false)),
                    )
                    .child(
                        vstack(3.0)
                            .min_w_0()
                            .items_start()
                            .child(
                                MainText::new(entry.title.clone().unwrap_or_else(|| entry.display_name.clone()))
                                    .px(14.0)
                                    .font_weight(FontWeight::MEDIUM)
                                    .number_of_lines(1),
                            )
                            .children(
                                entry
                                    .artist
                                    .clone()
                                    .filter(|artist| !artist.is_empty())
                                    .map(|artist| SecondaryText::new(artist).px(12.0)),
                            ),
                    )
                    .child(spacer_min_w(0.0))
                    .children(
                        entry
                            .duration
                            .filter(|duration| *duration > 0.0)
                            .map(|duration| SecondaryText::new(RiseFormat::duration_time_interval(duration)).px(12.0)),
                    ),
            )
            .into_any_element()
    }

    fn upload_items(&self, theme: &AppTheme, cx: &App) -> Vec<GroupedBtn> {
        let is_uploading = self.is_uploading(cx);
        let title = Binding::entity(
            self.state.clone(),
            |state: &EditorTrackPickerSheetState| state.upload_title.clone(),
            |state, title, _| state.upload_title = title,
        );
        let artist = Binding::entity(
            self.state.clone(),
            |state: &EditorTrackPickerSheetState| state.upload_artist.clone(),
            |state, artist, _| state.upload_artist = artist,
        );
        vec![
            self.choose_file_item(cx),
            GroupedBtn::new("editorUploadTitle", "").disabled(is_uploading).content_slot(Self::upload_field_row(
                "editor-track-picker-title",
                tr("music_upload_title"),
                title,
                MusicFieldRules::local_track_title(),
                tr("music_track_title_placeholder"),
                theme,
                cx,
            )),
            GroupedBtn::new("editorUploadArtist", "").disabled(is_uploading).content_slot(Self::upload_field_row(
                "editor-track-picker-artist",
                tr("music_local_upload_artist"),
                artist,
                MusicFieldRules::local_track_artist(),
                tr("music_unknown_artist"),
                theme,
                cx,
            )),
        ]
    }

    fn upload_save_bar_layer(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let now = RiseClock::monotonic();
        let (phase, has_audio, presence) = {
            let state = self.state.read(cx);
            (state.phase, state.upload_audio_file.is_some(), state.save_bar_presence.clone())
        };
        let shows_bar = phase == EditorTrackPickerSheetPhase::Upload && has_audio;
        presence.update(cx, |presence, cx| presence.set_in(shows_bar.then_some(()), spring(0.3, 0.9), now, cx));
        presence.read(cx).value(now)?;
        let bar = self.upload_save_bar(theme, cx);
        presence
            .read(cx)
            .render_transitioned(now, window, move |_| div().w_full().flex_none().child(bar))
            .map(IntoElement::into_any_element)
    }

    fn upload_save_bar(&self, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let can_start = self.can_start_upload(cx);
        let is_uploading = self.is_uploading(cx);
        let progress = self.upload_progress(cx);
        let is_waiting = self.state.read(cx).is_waiting_for_created_track;
        let parts = self.clone();
        vstack(10.0)
            .w_full()
            .px(px(12.0))
            .pt(px(8.0))
            .pb(px(10.0))
            .bg(theme.bg._100)
            .when(is_waiting, |bar| {
                let label = format_printf(
                    &tr("music_local_upload_progress"),
                    &[FormatArg::Int(progress.round() as i64)],
                );
                bar.child(
                    vstack(6.0)
                        .w_full()
                        .child(progress_view_value((progress / 100.0).clamp(0.0, 1.0)).tint(theme.primary._100))
                        .child(SecondaryText::new(label).px(12.0).tac(TextAlignT::Center)),
                )
            })
            .child(
                ButtonUi::new("editor-track-picker-save")
                    .disabled(!can_start)
                    .on_press(move |_, cx| parts.start_upload(cx))
                    .child(MainText::new(if is_uploading { tr("music_uploading") } else { tr("music_upload_track") })),
            )
            .into_any_element()
    }

    fn upload_subtitle(&self, cx: &App) -> String {
        let duration_ms = self.state.read(cx).upload_duration_ms;
        if duration_ms < MINIMUM_AUDIO_DURATION_MS {
            return tr("music_local_upload_mp3_hint");
        }
        RiseFormat::duration_milliseconds(duration_ms)
    }

    /// same row as in the add local track sheet
    /// label keeps its own width
    /// canon rule cuts input at the server ceiling
    fn upload_field_row(
        id: &'static str,
        label: String,
        value: Binding<String>,
        rule: RiseFieldRule,
        placeholder: String,
        theme: &AppTheme,
        cx: &App,
    ) -> AnyElement {
        let remaining = rule.remaining(&value.get(cx));
        let child_id = |name: &'static str| {
            ElementId::NamedChild(Arc::new(ElementId::from(SharedString::new_static(id))), SharedString::new_static(name))
        };
        hstack(12.0)
            .w_full()
            .px(px(16.0))
            .min_h(px(54.0))
            .capture_action(|_: &Cancel, window, cx| {
                ScreenFocus::release_text_field(window, cx);
                cx.stop_propagation();
            })
            .child(
                div().flex_none().child(MainText::new(label).px(14.0).color(theme.text.secondary).number_of_lines(1)),
            )
            .child(
                ShapedTextField::new(child_id("field"), placeholder, rule.binding(value))
                    .text_align(TextAlign::Right)
                    .placeholder_color(theme.text.secondary.opacity(0.75)),
            )
            .when(InputFieldAccessory::is_visible(remaining, None), |row| {
                row.child(InputFieldAccessory::new(child_id("accessory"), remaining, None))
            })
            .into_any_element()
    }

    fn present_upload_audio_picker(&self, cx: &mut App) {
        let Some(presenter) = MediaPickerPresenterResolver::top_presenter(cx) else {
            return;
        };
        let Some(mp3) = UTType::from_filename_extension("mp3") else {
            return;
        };
        let parts = self.clone();
        let bridge = DocumentExportBridge::new(
            Some(vec![mp3]),
            false,
            Box::new(move |urls: Vec<PathBuf>, cx: &mut App| {
                tracing::warn!(target: "riseonly::X1417", urls = urls.len(), "editor track upload: open panel answered");
                let Some(url) = urls.first() else {
                    return;
                };
                parts.handle_upload_audio_picked(url, None, None, None, cx);
            }),
        );
        bridge.present(presenter, cx);
    }

    /// deleting is disk work and the callers are on the main thread
    fn remove_prepared_file(url: PathBuf) {
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            let _ = std::fs::remove_file(&url);
        }));
    }

    fn handle_upload_audio_picked(
        &self,
        source_url: &Path,
        display_name: Option<String>,
        preferred_title: Option<String>,
        preferred_artist: Option<String>,
        cx: &mut App,
    ) {
        let source_name =
            source_url.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let file_name = display_name.filter(|name| !name.is_empty()).unwrap_or(source_name);

        let source_path = source_url.to_path_buf();

        self.state.update(cx, |state, cx| {
            state.upload_pick_generation = state.upload_pick_generation.wrapping_add(1);
            let generation = state.upload_pick_generation;

            cx.spawn(async move |this, cx| {
                let prepared = RiseAccountRuntime::current()
                    .spawn_blocking(move || MusicTrackFileUpload::prepare_mp3(&source_path))
                    .await;
                let url = match prepared {
                    Ok(Ok(url)) => url,
                    Ok(Err(error)) => {
                        tracing::warn!(target: "riseonly::X1417", %error, "editor track upload: prepare failed");
                        let _ = this.update(cx, |state, cx| {
                            if generation == state.upload_pick_generation {
                                state.upload_error_message = Some(error.to_string());
                                cx.notify();
                            }
                        });
                        return;
                    }
                    Err(error) => {
                        tracing::warn!(target: "riseonly::X1417", %error, "editor track upload: prepare task dropped");
                        return;
                    }
                };
                let is_current = this
                    .update(cx, |state, cx| {
                        let is_current = generation == state.upload_pick_generation;
                        if is_current {
                            state.upload_file_name = file_name.clone();
                            state.upload_duration_ms = 0;
                            state.upload_audio_file = None;
                            cx.notify();
                        }
                        is_current
                    })
                    .unwrap_or(false);
                if !is_current {
                    Self::remove_prepared_file(url);
                    return;
                }

                let metadata = MusicTrackFileUpload::imported_metadata(url.clone(), Some(file_name)).await;
                let artwork_data = metadata.artwork_data.clone();
                let artwork = match artwork_data {
                    Some(artwork_data) => RiseAccountRuntime::current()
                        .spawn_blocking(move || {
                            let file = MusicTrackFileUpload::local_artwork_file(&artwork_data)?;
                            let preview = file
                                .url
                                .to_file_path()
                                .and_then(|path| std::fs::read(path).ok())
                                .and_then(|data| CleverImageBitmapDecoder::decode(&data, None));
                            Some((file, preview))
                        })
                        .await
                        .ok()
                        .flatten(),
                    None => None,
                };
                let _ = this.update(cx, |state, cx| {
                    if generation != state.upload_pick_generation {
                        return;
                    }
                    let audio_url = Url::from_file_path(&url);
                    let (Some(audio_url), true) = (audio_url, metadata.duration_ms >= MINIMUM_AUDIO_DURATION_MS) else {
                        state.upload_file_name = String::new();
                        Self::remove_prepared_file(url.clone());
                        state.upload_error_message = Some(tr("music_local_upload_mp3_required"));
                        cx.notify();
                        return;
                    };
                    let mut audio_file = SaiWsOptimisticLocalFile::new(
                        audio_url.clone(),
                        audio_url.last_path_component(),
                        RiseUploadFormat::AUDIO_MPEG.mime.to_owned(),
                    );
                    audio_file.file_context_override = Some("music_audio".to_owned());
                    state.upload_audio_file = Some(audio_file);
                    state.upload_title = MusicFieldRules::local_track_title()
                        .clamped(&preferred_title.filter(|title| !title.is_empty()).unwrap_or(metadata.title));
                    state.upload_artist = MusicFieldRules::local_track_artist().clamped(
                        &preferred_artist
                            .filter(|artist| !artist.is_empty())
                            .or(metadata.artist)
                            .unwrap_or_else(|| tr("music_unknown_artist")),
                    );
                    state.upload_duration_ms = metadata.duration_ms;
                    if let Some((file, preview)) = artwork {
                        state.upload_cover_file = Some(file);
                        state.upload_cover_preview = preview;
                    }
                    cx.notify();
                });
            })
            .detach();
        });
    }

    fn start_upload(&self, cx: &mut App) {
        if !self.can_start_upload(cx) {
            return;
        }
        let Some(destination) = self.resolved_upload_destination(cx) else {
            return;
        };
        let (request_id, temp_id, title, artist, duration_ms, audio_file, cover_file) = {
            let state = self.state.read(cx);
            let Some(audio_file) = state.upload_audio_file.clone() else {
                return;
            };
            (
                state.upload_request_id.clone(),
                state.upload_temp_id.clone(),
                state.upload_title.clone(),
                state.upload_artist.clone(),
                state.upload_duration_ms,
                audio_file,
                state.upload_cover_file.clone(),
            )
        };
        self.state.update(cx, |state, cx| {
            state.is_waiting_for_created_track = true;
            cx.notify();
        });
        let created_state = self.state.clone();
        let error_state = self.state.clone();
        let on_pick = self.on_pick.clone();
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.create_local_track_in_playlist(
                &request_id,
                &temp_id,
                &destination.id,
                &title,
                None,
                &artist,
                duration_ms,
                audio_file,
                cover_file,
                None,
                Some(Box::new(move |track: RiseStoredMusicTrack, cx: &mut App| {
                    created_state.update(cx, |state, cx| {
                        state.is_waiting_for_created_track = false;
                        cx.notify();
                    });
                    on_pick(track, cx);
                })),
                Some(Rc::new(move |message: String, cx: &mut App| {
                    error_state.update(cx, |state, cx| {
                        state.is_waiting_for_created_track = false;
                        state.upload_error_message = Some(message);
                        cx.notify();
                    });
                })),
                cx,
            );
        });
    }

    fn no_pagination() -> TrackPickerPagination {
        TrackPickerPagination::none()
    }

    fn section_title_row(id: &'static str, text: String) -> TrackPickerRow {
        TrackPickerRow::SectionTitle { id, text, top_padding: 16.0 }
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
                state.has_own_search_results = false;
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

    fn visible_search_results(&self, cx: &mut App) -> Vec<RiseStoredMusicTrack> {
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

    fn playlist_subtitle(playlist: &RiseStoredMusicPlaylist, cx: &App) -> String {
        let count = track_count_label(Self::selectable_tracks(playlist.tracks.clone(), cx).len());
        let viewer_id = AuthServicesStore::try_read(cx).and_then(AuthServicesStore::effective_user_id_for_wire);
        match playlist.owner_user_id.as_deref() {
            Some(owner) if !owner.is_empty() && Some(owner) != viewer_id.as_deref() => {
                format!("{} \u{b7} {count}", tr("music_playlist_shared_badge"))
            }
            _ => count,
        }
    }

    /// a collaborators upload in a shared local playlist stays private to it
    /// server refuses to attach it so its not offered
    fn selectable_tracks(tracks: Option<Vec<RiseStoredMusicTrack>>, cx: &App) -> Vec<RiseStoredMusicTrack> {
        let viewer_id = AuthServicesStore::try_read(cx).and_then(AuthServicesStore::effective_user_id_for_wire);
        let mut tracks = tracks.unwrap_or_default();
        tracks.retain(|track| {
            let status = track.status.as_deref().unwrap_or(RiseMusicTrackStatus::Published.wire());
            if status == RiseMusicTrackStatus::Published.wire() {
                true
            } else if status == RiseMusicTrackStatus::LocalLibrary.wire() {
                viewer_id.is_some() && track.library_owner_user_id == viewer_id
            } else {
                false
            }
        });
        tracks
    }
}

/// what the library rows are built from, an equal key gives the same rows
#[derive(PartialEq)]
struct EditorTrackPickerRowsKey {
    generation: u64,
    slices: [u64; 6],
    query: String,
    search_in_flight: bool,
    has_own_search_results: bool,
    attached_track: Option<MediaPickerTrackSelection>,
    expanded_playlist_id: Option<String>,
}

/// rows only the editor sheet has, beside the ones both sheets share
enum EditorTrackPickerRow {
    Attached(MediaPickerTrackSelection),
    UploadEntry,
    Shared(TrackPickerRow),
}

impl EditorTrackPickerRow {
    fn id(&self) -> SharedString {
        match self {
            Self::Attached(_) => SharedString::new_static("attached"),
            Self::UploadEntry => SharedString::new_static("upload-entry"),
            Self::Shared(row) => row.id(),
        }
    }
}

#[derive(IntoElement)]
struct EditorTrackPickerSheetHeader {
    theme: AppTheme,
    state: Entity<EditorTrackPickerSheetState>,
    on_dismiss: SheetAction,
}

impl RenderOnce for EditorTrackPickerSheetHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_upload = self.state.read(cx).phase == EditorTrackPickerSheetPhase::Upload;
        let title = tr(if is_upload { "music_local_upload_title" } else { "audio_page_title" });
        let label = tr(if is_upload { "back" } else { "cancel" });
        let symbol = if is_upload { "chevron.backward" } else { "xmark" };
        let state = self.state.clone();
        let on_dismiss = self.on_dismiss.clone();
        div()
            .relative()
            .w_full()
            .flex_none()
            .h(px(48.0))
            .px(px(12.0))
            .pb(px(6.0))
            .flex()
            .items_start()
            .child(
                div().absolute().top_0().left_0().right_0().h(px(42.0)).flex().items_center().justify_center().child(
                    MainText::new(title)
                        .px(17.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(self.theme.text.primary)
                        .line_limit(1),
                ),
            )
            .child(
                LiquidGlassButton::new("editor-track-picker-back", LiquidGlassButtonShape::Circle)
                    .size(42.0)
                    .accessibility_label(Some(label))
                    .on_press(move |window, cx| {
                        let is_upload = state.read(cx).phase == EditorTrackPickerSheetPhase::Upload;
                        if is_upload {
                            state.update(cx, |state, cx| {
                                state.phase = EditorTrackPickerSheetPhase::Library;
                                cx.notify();
                            });
                        } else {
                            on_dismiss(window, cx);
                        }
                    })
                    .children(IconUi::sized(symbol, px(15.0), self.theme.text.secondary, false)),
            )
    }
}
