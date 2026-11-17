use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Context, ElementId, Entity, FontWeight, ParentElement, Render, SharedString, Styled, Subscription, Window};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::layer_keys::LayerKeysExt;

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::input_ui::InputUi;
use crate::modules::music::shared::music_field_rules::MusicFieldRules;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicPlaylist;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::widgets::bottomsheets::playlists::playlist_visibility::PlaylistVisibility;
use crate::modules::music::widgets::music_cover_picker_ui::MusicCoverPickerUi;

const VALUE_TITLE: &str = "title";
const VALUE_DESCRIPTION: &str = "description";

pub struct EditPlaylistSheet {
    playlist: RiseStoredMusicPlaylist,
    on_dismiss: Option<Rc<dyn Fn(&mut App)>>,
    values: HashMap<String, String>,
    selected_visibility: PlaylistVisibility,
    local_cover_file: Option<SaiWsOptimisticLocalFile>,
    cover_preview: Option<CleverImage>,
    submit_error: Option<String>,
    store: Entity<MusicActionsStore>,
    _store_observation: Subscription,
}

impl EditPlaylistSheet {
    pub fn new(
        playlist: RiseStoredMusicPlaylist,
        on_dismiss: Option<Rc<dyn Fn(&mut App)>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let store = MusicActionsStore::shared(cx);
        let store_observation = cx.observe(&store, |_, _, cx| cx.notify());
        let values = HashMap::from([
            (VALUE_TITLE.to_owned(), playlist.display_title()),
            (VALUE_DESCRIPTION.to_owned(), playlist.description.clone().unwrap_or_default()),
        ]);
        let selected_visibility = playlist
            .visibility
            .as_deref()
            .and_then(PlaylistVisibility::from_raw)
            .unwrap_or(PlaylistVisibility::Private);
        Self {
            playlist,
            on_dismiss,
            values,
            selected_visibility,
            local_cover_file: None,
            cover_preview: None,
            submit_error: None,
            store,
            _store_observation: store_observation,
        }
    }

    fn is_local(&self) -> bool {
        self.playlist.visibility.as_deref().and_then(PlaylistVisibility::from_raw) == Some(PlaylistVisibility::Local)
    }

    fn title(&self) -> &str {
        self.values.get(VALUE_TITLE).map(String::as_str).unwrap_or_default()
    }

    fn description(&self) -> &str {
        self.values.get(VALUE_DESCRIPTION).map(String::as_str).unwrap_or_default()
    }

    fn cover_upload_progress(&self, cx: &App) -> f64 {
        self.store.read(cx).update_playlist_sai.data().and_then(|data| data.cover_upload_progress).unwrap_or(0.0)
    }

    fn is_cover_uploading(&self, cx: &App) -> bool {
        self.store.read(cx).update_playlist_sai.data().and_then(|data| data.cover_uploading) == Some(true)
    }

    fn is_submitting(&self, cx: &App) -> bool {
        self.store.read(cx).update_playlist_sai.is_pending() || self.is_cover_uploading(cx)
    }

    fn visibility_row(
        this: &Entity<Self>,
        vis: PlaylistVisibility,
        is_selected: bool,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement + use<> {
        let id = SharedString::from(format!("edit-playlist-visibility-{}", vis.raw_value()));
        let target = if is_selected { 1.0_f32 } else { 0.0 };
        let now = RiseClock::monotonic();
        let reduce_motion = reduce_motion(cx);
        let selection_id = SharedString::from(format!("edit-playlist-visibility-selection-{}", vis.raw_value()));
        let selection = window.use_keyed_state(ElementId::Name(selection_id), cx, |_, _| Animated::new(target));
        selection.update(cx, |selection, _| selection.set(target, spring(0.2, 0.8), now, reduce_motion));
        let progress = selection.read(cx).frame_value(now, window).clamp(0.0, 1.0);
        let is_filled = vis.icon().ends_with(".fill");
        let on_press = this.clone();
        plain(id)
            .w_full()
            .on_press(move |_, cx| {
                on_press.update(cx, |sheet, cx| {
                    sheet.selected_visibility = vis;
                    cx.notify();
                });
            })
            .child(
                hstack(12.0)
                    .w_full()
                    .px(px(12.0))
                    .py(px(12.0))
                    .child(
                        zstack(Alignment::Center)
                            .size(px(32.0))
                            .child(div().size(px(32.0)).rounded_full().bg(theme.bg._300))
                            .child(div().size(px(32.0)).rounded_full().bg(theme.primary._100.opacity(progress)))
                            .children(IconUi::sized(
                                vis.icon(),
                                px(14.0),
                                theme.text.secondary.opacity(1.0 - progress),
                                is_filled,
                            ))
                            .children(IconUi::sized(
                                vis.icon(),
                                px(14.0),
                                theme.content_on(theme.primary._100).opacity(progress),
                                is_filled,
                            )),
                    )
                    .child(
                        vstack(2.0)
                            .align(HorizontalAlignment::Leading)
                            .child(
                                MainText::new(vis.title())
                                    .px(15.0)
                                    .font_weight(FontWeight::MEDIUM)
                                    .color(theme.text.primary),
                            )
                            .child(SecondaryText::new(vis.description()).px(12.0).line_limit(2)),
                    )
                    .child(spacer_min_w(0.0))
                    .children((progress > 0.0).then(|| {
                        IconUi::sized("checkmark", px(14.0), theme.primary._100.opacity(progress), false)
                    }).flatten()),
            )
    }

    fn close(
        dismiss_action: &DismissAction,
        window_handle: gpui::AnyWindowHandle,
        on_dismiss: Option<Rc<dyn Fn(&mut App)>>,
        cx: &mut App,
    ) {
        let dismiss_action = dismiss_action.clone();
        cx.defer(move |cx| {
            window_handle.update(cx, |_, window, cx| dismiss_action.call_as_function(window, cx)).ok();
            if let Some(on_dismiss) = on_dismiss {
                on_dismiss(cx);
            }
        });
    }

    fn save(&mut self, dismiss_action: DismissAction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.playlist.id.clone() else {
            return;
        };
        let trimmed_title = trimming_whitespaces(self.title()).to_owned();
        if trimmed_title.is_empty() {
            return;
        }
        let keeps_default_title =
            self.playlist.is_default_local == Some(true) && trimmed_title == self.playlist.display_title();
        let title_to_send = (!keeps_default_title).then_some(trimmed_title.as_str());
        let trimmed_desc = trimming_whitespaces(self.description()).to_owned();
        let new_visibility = if self.is_local() { None } else { Some(self.selected_visibility.raw_value()) };
        let local_cover_file = self.local_cover_file.clone();
        let existing_playlist = self.playlist.clone();
        let window_handle = window.window_handle();
        let on_dismiss = self.on_dismiss.clone();
        let this = cx.entity().downgrade();
        let on_success: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            Self::close(&dismiss_action, window_handle, on_dismiss.clone(), cx);
        });
        let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(move |message: String, cx: &mut App| {
            this.update(cx, |sheet, cx| sheet.present_submit_error(message, cx)).ok();
        });
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.update_playlist(
                &id,
                title_to_send,
                Some(trimmed_desc.as_str()),
                new_visibility,
                local_cover_file,
                Some(existing_playlist),
                Some(on_success),
                Some(on_error),
                cx,
            );
        });
    }
}

impl EditPlaylistSheet {
    fn present_submit_error(&mut self, message: String, cx: &mut Context<Self>) {
        let message = self.submit_error.insert(message).clone();
        let this = cx.entity().downgrade();
        NativeConfirmAlert::present_notice(
            &tr("save_failed"),
            &message,
            None,
            None,
            move |cx: &mut App| {
                this.update(cx, |sheet, cx| {
                    sheet.submit_error = None;
                    cx.notify();
                })
                .ok();
            },
            cx,
        );
        cx.notify();
    }
}

impl Render for EditPlaylistSheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let this = cx.entity();
        let dismiss_action = dismiss(cx);
        let is_submitting = self.is_submitting(cx);
        let is_cover_uploading = self.is_cover_uploading(cx);
        let cover_upload_progress = self.cover_upload_progress(cx);
        let local_cover_file = Binding::entity(
            this.clone(),
            |sheet: &Self| sheet.local_cover_file.clone(),
            |sheet, value, _| sheet.local_cover_file = value,
        );
        let cover_preview = Binding::entity(
            this.clone(),
            |sheet: &Self| sheet.cover_preview.clone(),
            |sheet, value, _| sheet.cover_preview = value,
        );
        let set_title = this.clone();
        let set_description = this.clone();
        let visibility = (!self.is_local()).then(|| {
            let selected = self.selected_visibility;
            let private_row = Self::visibility_row(
                &this,
                PlaylistVisibility::Private,
                selected == PlaylistVisibility::Private,
                &theme,
                window,
                cx,
            );
            let public_row = Self::visibility_row(
                &this,
                PlaylistVisibility::Public,
                selected == PlaylistVisibility::Public,
                &theme,
                window,
                cx,
            );
            vstack(8.0)
                .w_full()
                .align(HorizontalAlignment::Leading)
                .child(
                    div().px(px(16.0)).child(
                        MainText::new(tr("music_playlist_visibility"))
                            .px(13.0)
                            .font_weight(FontWeight::MEDIUM)
                            .color(theme.text.secondary),
                    ),
                )
                .child(
                    div().w_full().px(px(16.0)).child(
                        vstack(0.0)
                            .w_full()
                            .bg(theme.bg._200)
                            .rounded(px(12.0))
                            .overflow_hidden()
                            .child(private_row)
                            .child(
                                div()
                                    .w_full()
                                    .pl(px(52.0))
                                    .child(div().w_full().h(px(0.5)).bg(theme.border._100)),
                            )
                            .child(public_row),
                    ),
                )
        });
        let on_save = this.clone();
        let (submit, submit_dismiss) = (this.clone(), dismiss_action.clone());
        let can_save = !trimming_whitespaces(self.title()).is_empty() && !is_submitting;
        vstack(8.0)
            .w_full()
            .bg(theme.bg._100)
            .form_primary(can_save, move |window, cx| {
                let dismiss_action = submit_dismiss.clone();
                submit.update(cx, |sheet, cx| sheet.save(dismiss_action, window, cx));
            })
            .child(
                vstack(18.0)
                    .w_full()
                    .pt(px(4.0))
                    .pb(px(6.0))
                    .child(
                        MainText::new(tr("music_edit_playlist"))
                            .px(18.0)
                            .font_weight(FontWeight::BOLD)
                            .color(theme.text.primary),
                    )
                    .child(
                        MusicCoverPickerUi::new("edit-playlist-cover", local_cover_file, cover_preview)
                            .existing_cover_url(self.playlist.cover_url.clone())
                            .placeholder_label(Some(tr("music_playlist_cover")))
                            .cover_upload_progress(cover_upload_progress)
                            .show_upload_overlay(is_cover_uploading && self.local_cover_file.is_some()),
                    )
                    .child(
                        vstack(12.0)
                            .w_full()
                            .px(px(16.0))
                            .child(
                                InputUi::new("edit-playlist-title")
                                    .values(self.values.clone())
                                    .name(VALUE_TITLE)
                                    .placeholder(tr("music_playlist_title_placeholder"))
                                    .rule(MusicFieldRules::playlist_title_on_edit())
                                    .set_value(move |key, value, _, cx| {
                                        set_title.update(cx, |sheet, cx| {
                                            sheet.values.insert(key.to_owned(), value.to_owned());
                                            cx.notify();
                                        });
                                    }),
                            )
                            .child(
                                InputUi::new("edit-playlist-description")
                                    .values(self.values.clone())
                                    .name(VALUE_DESCRIPTION)
                                    .placeholder(tr("music_playlist_description_placeholder"))
                                    .set_value(move |key, value, _, cx| {
                                        set_description.update(cx, |sheet, cx| {
                                            sheet.values.insert(key.to_owned(), value.to_owned());
                                            cx.notify();
                                        });
                                    }),
                            ),
                    )
                    .children(visibility),
            )
            .child(
                div().w_full().px(px(16.0)).child(
                    ButtonUi::new("edit-playlist-save")
                        .disabled(trimming_whitespaces(self.title()).is_empty() || is_submitting)
                        .on_press(move |window, cx| {
                            let dismiss_action = dismiss_action.clone();
                            on_save.update(cx, |sheet, cx| sheet.save(dismiss_action, window, cx));
                        })
                        .child(MainText::new(if is_submitting { tr("music_uploading") } else { tr("music_save") })),
                ),
            )
    }
}
