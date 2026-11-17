use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, ElementId, Entity, FontWeight, SharedString, Task, TextAlign, Window};
use rise_canon::{RiseUploadContextFormats, RiseUploadFormat};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{FormatArg, format_printf, tr};
use rise_platform::file_manager::FileManager;
use rise_platform::uniform_type::UTType;
use rise_theme::AppTheme;
use rise_ui::button_ui::ButtonUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::progress_view::progress_view_value;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::layer_keys::LayerKeysExt;

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder};
use crate::core::ui::input_field_accessory::InputFieldAccessory;
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::modules::media_picker::api::media_picker_presenter_resolver::MediaPickerPresenterResolver;
use crate::modules::media_picker::send::document_export_bridge::DocumentExportBridge;
use crate::modules::music::shared::music_field_rules::MusicFieldRules;
use crate::modules::music::shared::utils::music::music_track_file_upload::{
    MusicTrackFileUpload, MusicTrackImportedMetadata,
};
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::widgets::music_cover_picker_ui::MusicCoverPickerUi;

const MINIMUM_AUDIO_DURATION_MS: i64 = 500;

type AddLocalTrackDismiss = Rc<dyn Fn(&mut App)>;

struct AddLocalTrackSheetState {
    playlist_id: String,
    on_dismiss: AddLocalTrackDismiss,
    request_id: String,
    temp_id: String,
    title: String,
    artist_name: String,
    audio_file_name: String,
    audio_duration_ms: i64,
    local_audio_file: Option<SaiWsOptimisticLocalFile>,
    local_cover_file: Option<SaiWsOptimisticLocalFile>,
    cover_preview: Option<CleverImage>,
    error_message: Option<String>,
    pick_generation: u64,
    form_height: f32,
    prepare_task: Option<Task<()>>,
    import_task: Option<Task<()>>,
}

impl AddLocalTrackSheetState {
    fn new(playlist_id: String, on_dismiss: AddLocalTrackDismiss) -> Self {
        Self {
            playlist_id,
            on_dismiss,
            request_id: Uuid::new_v4().uuid_string().to_lowercase(),
            temp_id: format!("temp_local_{}", Uuid::new_v4().uuid_string()),
            title: String::new(),
            artist_name: String::new(),
            audio_file_name: String::new(),
            audio_duration_ms: 0,
            local_audio_file: None,
            local_cover_file: None,
            cover_preview: None,
            error_message: None,
            pick_generation: 0,
            form_height: 0.,
            prepare_task: None,
            import_task: None,
        }
    }

    fn upload_progress(cx: &mut App) -> f64 {
        MusicActionsStore::slices(cx)
            .create_local_track_in_playlist_sai
            .data()
            .and_then(|upload| upload.upload_progress)
            .unwrap_or(0.)
    }

    fn is_uploading(cx: &mut App) -> bool {
        let store = MusicActionsStore::shared(cx);
        let slice = &store.read(cx).create_local_track_in_playlist_sai;
        slice.is_pending() || slice.data().and_then(|upload| upload.is_uploading) == Some(true)
    }

    fn can_save(&self, is_uploading: bool) -> bool {
        let clean_title = trimming_whitespaces_and_newlines(&self.title);
        let clean_artist = trimming_whitespaces_and_newlines(&self.artist_name);
        self.local_audio_file.is_some()
            && self.audio_duration_ms >= MINIMUM_AUDIO_DURATION_MS
            && !clean_title.is_empty()
            && !clean_artist.is_empty()
            && MusicFieldRules::local_track_title().is_satisfied(clean_title)
            && MusicFieldRules::local_track_artist().is_satisfied(clean_artist)
            && !is_uploading
    }

    fn audio_subtitle(&self) -> String {
        if self.audio_duration_ms < MINIMUM_AUDIO_DURATION_MS {
            return tr("music_local_upload_mp3_hint");
        }
        RiseFormat::duration_milliseconds(self.audio_duration_ms)
    }

    /// dialog of the window stands for the bound alert, answering it clears the message
    fn present_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.error_message = Some(message.clone());
        cx.notify();
        let this = cx.weak_entity();
        NativeConfirmAlert::present_notice(
            &tr("music_local_upload_error_title"),
            &message,
            None,
            None,
            move |cx| {
                let _ = this.update(cx, |state, cx| {
                    state.error_message = None;
                    cx.notify();
                });
            },
            cx,
        );
    }

    fn handle_audio_picked(&mut self, source_url: PathBuf, cx: &mut Context<Self>) {
        let file_name = source_url.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let prepared = cx.background_spawn(async move { MusicTrackFileUpload::prepare_mp3(&source_url) });
        self.prepare_task = Some(cx.spawn(async move |this, cx| {
            let prepared = prepared.await;
            let _ = this.update(cx, |state, cx| match prepared {
                Ok(url) => state.audio_prepared(url, file_name, cx),
                Err(error) => state.present_error(error.to_string(), cx),
            });
        }));
    }

    fn audio_prepared(&mut self, url: PathBuf, file_name: String, cx: &mut Context<Self>) {
        self.pick_generation = self.pick_generation.wrapping_add(1);
        let generation = self.pick_generation;
        self.audio_file_name = file_name.clone();
        self.audio_duration_ms = 0;
        self.local_audio_file = None;
        cx.notify();

        self.import_task = Some(cx.spawn(async move |this, cx| {
            let metadata = MusicTrackFileUpload::imported_metadata(url.clone(), Some(file_name)).await;
            let artwork_data = metadata.artwork_data.clone();
            let artwork = cx
                .background_spawn(async move {
                    let artwork_file = artwork_data.and_then(|data| MusicTrackFileUpload::local_artwork_file(&data))?;
                    let preview = artwork_file
                        .url
                        .to_file_path()
                        .and_then(|path| std::fs::read(path).ok())
                        .and_then(|data| CleverImageBitmapDecoder::decode(&data, None));
                    Some((artwork_file, preview))
                })
                .await;
            let _ = this.update(cx, |state, cx| state.metadata_imported(generation, url, metadata, artwork, cx));
        }));
    }

    fn metadata_imported(
        &mut self,
        generation: u64,
        url: PathBuf,
        metadata: MusicTrackImportedMetadata,
        artwork: Option<(SaiWsOptimisticLocalFile, Option<CleverImage>)>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.pick_generation {
            return;
        }
        if metadata.duration_ms < MINIMUM_AUDIO_DURATION_MS {
            self.audio_file_name = String::new();
            let _ = FileManager.remove_item(&url);
            self.present_error(tr("music_local_upload_mp3_required"), cx);
            return;
        }
        let Some(file_url) = Url::from_file_path(&url) else {
            self.audio_file_name = String::new();
            self.present_error(tr("music_local_upload_mp3_required"), cx);
            return;
        };
        let filename = file_url.last_path_component();
        let mut local_audio_file =
            SaiWsOptimisticLocalFile::new(file_url, filename, RiseUploadFormat::AUDIO_MPEG.mime.to_owned());
        local_audio_file.file_context_override = Some(RiseUploadContextFormats::MUSIC_AUDIO.context.to_owned());
        self.local_audio_file = Some(local_audio_file);
        self.title = MusicFieldRules::local_track_title().clamped(&metadata.title);
        let artist = metadata.artist.unwrap_or_else(|| tr("music_unknown_artist"));
        self.artist_name = MusicFieldRules::local_track_artist().clamped(&artist);
        self.audio_duration_ms = metadata.duration_ms;
        if let Some((artwork_file, preview)) = artwork {
            self.local_cover_file = Some(artwork_file);
            self.cover_preview = preview;
        }
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if !self.can_save(Self::is_uploading(cx)) {
            return;
        }
        let Some(local_audio_file) = self.local_audio_file.clone() else {
            return;
        };
        let on_dismiss = self.on_dismiss.clone();
        let this = cx.weak_entity();
        let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(move |message: String, cx: &mut App| {
            let _ = this.update(cx, |state, cx| state.present_error(message, cx));
        });
        let local_cover_file = self.local_cover_file.clone();
        let request_id = self.request_id.clone();
        let temp_id = self.temp_id.clone();
        let playlist_id = self.playlist_id.clone();
        let title = self.title.clone();
        let artist_name = self.artist_name.clone();
        let duration_ms = self.audio_duration_ms;
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.create_local_track_in_playlist(
                &request_id,
                &temp_id,
                &playlist_id,
                &title,
                None,
                &artist_name,
                duration_ms,
                local_audio_file,
                local_cover_file,
                Some(Box::new(move |cx: &mut App| on_dismiss(cx))),
                None,
                Some(on_error),
                cx,
            );
        });
    }

    fn present_audio_picker(this: &Entity<Self>, cx: &mut App) {
        let (Some(presenter), Some(mp3)) =
            (MediaPickerPresenterResolver::top_presenter(cx), UTType::from_filename_extension("mp3"))
        else {
            return;
        };
        let this = this.downgrade();
        let bridge = DocumentExportBridge::new(
            Some(vec![mp3]),
            false,
            Box::new(move |urls: Vec<PathBuf>, cx: &mut App| {
                let Some(url) = urls.into_iter().next() else {
                    return;
                };
                let _ = this.update(cx, |state, cx| state.handle_audio_picked(url, cx));
            }),
        );
        bridge.present(presenter, cx);
    }
}

/// uploads a personal mp3 into a local playlist
#[derive(IntoElement)]
pub struct AddLocalTrackSheet {
    id: ElementId,
    playlist_id: String,
    is_visible_to_members: bool,
    on_dismiss: AddLocalTrackDismiss,
}

impl AddLocalTrackSheet {
    pub fn new(id: impl Into<ElementId>, playlist_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            playlist_id: playlist_id.into(),
            is_visible_to_members: false,
            on_dismiss: Rc::new(|_: &mut App| {}),
        }
    }

    pub fn is_visible_to_members(mut self, is_visible_to_members: bool) -> Self {
        self.is_visible_to_members = is_visible_to_members;
        self
    }

    /// runs from the upload success callback so it gets no window
    pub fn on_dismiss(mut self, on_dismiss: impl Fn(&mut App) + 'static) -> Self {
        self.on_dismiss = Rc::new(on_dismiss);
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn grouped_items(
        &self,
        state: &Entity<AddLocalTrackSheetState>,
        is_uploading: bool,
        theme: &AppTheme,
        cx: &App,
    ) -> Vec<GroupedBtn> {
        let form = state.read(cx);
        let has_audio = form.local_audio_file.is_some();
        let picker_state = state.clone();
        let mut items = vec![
            GroupedBtn::new(
                "audio",
                if form.audio_file_name.is_empty() {
                    tr("music_local_upload_choose_mp3")
                } else {
                    form.audio_file_name.clone()
                },
            )
            .icon("waveform")
            .subtitle(form.audio_subtitle())
            .disabled(is_uploading)
            .callback(move |_, cx| AddLocalTrackSheetState::present_audio_picker(&picker_state, cx)),
        ];

        if has_audio {
            let title = Binding::entity(state.clone(), |form: &AddLocalTrackSheetState| form.title.clone(), |form, value, _| {
                form.title = value;
            });
            items.push(
                GroupedBtn::new("title", tr("music_upload_title")).disabled(is_uploading).content_slot(self.text_field_row(
                    "title-field",
                    tr("music_upload_title"),
                    title,
                    MusicFieldRules::local_track_title(),
                    tr("music_track_title_placeholder"),
                    theme,
                    cx,
                )),
            );
            let artist = Binding::entity(
                state.clone(),
                |form: &AddLocalTrackSheetState| form.artist_name.clone(),
                |form, value, _| {
                    form.artist_name = value;
                },
            );
            items.push(
                GroupedBtn::new("artist", tr("music_local_upload_artist")).disabled(is_uploading).content_slot(
                    self.text_field_row(
                        "artist-field",
                        tr("music_local_upload_artist"),
                        artist,
                        MusicFieldRules::local_track_artist(),
                        tr("music_unknown_artist"),
                        theme,
                        cx,
                    ),
                ),
            );
        }

        items
    }

    /// label keeps its own width, a fixed column broke it mid word on narrow screens
    /// canon rule cuts input at the server ceiling and the counter shows the limit
    #[allow(clippy::too_many_arguments)]
    fn text_field_row(
        &self,
        name: &'static str,
        label: String,
        value: Binding<String>,
        rule: RiseFieldRule,
        placeholder: String,
        theme: &AppTheme,
        cx: &App,
    ) -> impl IntoElement {
        let remaining = rule.remaining(&value.get(cx));
        hstack(12.)
            .w_full()
            .px(px(16.))
            .min_h(px(54.))
            .bg(theme.bg._200)
            .child(
                div().flex_none().child(MainText::new(label).px(14.).color(theme.text.secondary).number_of_lines(1)),
            )
            .child(
                div().flex_1().min_w_0().text_color(theme.text.primary).child(
                    ShapedTextField::new(self.child_id(name), placeholder, rule.binding(value))
                        .text_align(TextAlign::Right),
                ),
            )
            .child(InputFieldAccessory::new(
                ElementId::NamedChild(Arc::new(self.child_id(name)), SharedString::new_static("accessory")),
                remaining,
                None,
            ))
    }
}

impl RenderOnce for AddLocalTrackSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let playlist_id = self.playlist_id.clone();
        let on_dismiss = self.on_dismiss.clone();
        let state = window.use_keyed_state(self.child_id("state"), cx, move |_, _| {
            AddLocalTrackSheetState::new(playlist_id, on_dismiss)
        });
        let playlist_id = self.playlist_id.clone();
        let on_dismiss = self.on_dismiss.clone();
        state.update(cx, |form, _| {
            form.playlist_id = playlist_id;
            form.on_dismiss = on_dismiss;
        });

        let upload_progress = AddLocalTrackSheetState::upload_progress(cx);
        let is_uploading = AddLocalTrackSheetState::is_uploading(cx);
        let can_save = state.read(cx).can_save(is_uploading);
        let has_audio = state.read(cx).local_audio_file.is_some();
        let has_cover = state.read(cx).local_cover_file.is_some();

        let mut form = vstack(18.).w_full().pt(px(4.)).pb(px(6.)).child(
            MainText::new(tr("music_local_upload_title"))
                .px(18.)
                .font_weight(FontWeight::BOLD)
                .color(theme.text.primary),
        );

        if has_audio {
            let local_cover_file = Binding::entity(
                state.clone(),
                |form: &AddLocalTrackSheetState| form.local_cover_file.clone(),
                |form, value, _| {
                    form.local_cover_file = value;
                },
            );
            let cover_preview = Binding::entity(
                state.clone(),
                |form: &AddLocalTrackSheetState| form.cover_preview.clone(),
                |form, value, _| {
                    form.cover_preview = value;
                },
            );
            form = form.child(
                MusicCoverPickerUi::new(self.child_id("cover"), local_cover_file, cover_preview)
                    .placeholder_label(Some(tr("music_local_upload_cover_optional")))
                    .cover_upload_progress(upload_progress)
                    .show_upload_overlay(is_uploading && has_cover)
                    .size(126.)
                    .corner_radius(63.),
            );
        }

        form = form.child(
            GroupedBtns::new(self.child_id("rows"), self.grouped_items(&state, is_uploading, &theme, cx)).end_group_title(
                tr(if self.is_visible_to_members {
                    "music_local_upload_shared_hint"
                } else {
                    "music_local_upload_private_hint"
                }),
            ),
        );

        if is_uploading {
            form = form.child(
                vstack(8.)
                    .w_full()
                    .px(px(16.))
                    .child(progress_view_value((upload_progress / 100.).clamp(0., 1.)).tint(theme.primary._100).w_full())
                    .child(
                        SecondaryText::new(format_printf(
                            &tr("music_local_upload_progress"),
                            &[FormatArg::Int(upload_progress.round() as i64)],
                        ))
                        .px(12.)
                        .tac(TextAlignT::Center),
                    ),
            );
        }

        let form_height = state.read(cx).form_height;
        let measured = state.clone();
        let form = div().relative().w_full().child(form).child(measure(
            self.child_id("form-size"),
            move |bounds, _, cx| {
                let height = f32::from(bounds.size.height);
                measured.update(cx, |state, cx| {
                    if (state.form_height - height).abs() <= 0.5 {
                        return;
                    }
                    state.form_height = height;
                    cx.notify();
                });
            },
        ));

        let saving = state.clone();
        let submit = state.clone();
        vstack(8.)
            .w_full()
            .max_h_full()
            .bg(theme.bg._100)
            .form_primary(can_save, move |_, cx| submit.update(cx, |form, cx| form.save(cx)))
            .child(
                scroll_view(self.child_id("scroll"))
                    .shows_indicators(false)
                    .w_full()
                    .h(px(form_height))
                    .flex_shrink_1()
                    .min_h_0()
                    .child(form),
            )
            .child(
                div().w_full().flex_none().px(px(12.)).child(
                    ButtonUi::new(self.child_id("save"))
                        .disabled(!can_save)
                        .on_press(move |_, cx| saving.update(cx, |form, cx| form.save(cx)))
                        .child(MainText::new(if is_uploading {
                            tr("music_uploading")
                        } else {
                            tr("music_local_upload_save")
                        })),
                ),
            )
    }
}
