use std::rc::Rc;

use gpui::{App, Context, Entity, FontWeight, ParentElement, Render, SharedString, Styled, Subscription, TextAlign, Window};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::button_ui::ButtonUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::progress_view::progress_view_value;
use rise_ui::layer_keys::LayerKeysExt;

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::input_field_accessory::InputFieldAccessory;
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::modules::music::engine::rise_music_presentation::RiseMusicPresentation;
use crate::modules::music::shared::music_field_rules::MusicFieldRules;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::widgets::music_cover_picker_ui::MusicCoverPickerUi;

pub struct EditLocalTrackSheet {
    track: TrackModel,
    on_dismiss: Rc<dyn Fn(&mut App)>,
    title: String,
    artist_name: String,
    local_cover_file: Option<SaiWsOptimisticLocalFile>,
    cover_preview: Option<CleverImage>,
    error_message: Option<String>,
    did_submit: bool,
    presentation: Entity<RiseMusicPresentation>,
    _presentation_observation: Subscription,
}

impl EditLocalTrackSheet {
    pub fn new(track: TrackModel, on_dismiss: impl Fn(&mut App) + 'static, cx: &mut Context<Self>) -> Self {
        let presentation = RiseMusicPresentation::shared(cx);
        let presentation_observation = cx.observe(&presentation, |_, _, cx| cx.notify());
        Self {
            title: track.title.clone(),
            artist_name: track.artist.clone(),
            track,
            on_dismiss: Rc::new(on_dismiss),
            local_cover_file: None,
            cover_preview: None,
            error_message: None,
            did_submit: false,
            presentation,
            _presentation_observation: presentation_observation,
        }
    }

    fn is_saving(&self, cx: &App) -> bool {
        self.did_submit || self.presentation.read(cx).snapshot().updating_track_ids.contains(&self.track.id)
    }

    fn upload_progress(&self, cx: &App) -> f64 {
        self.presentation.read(cx).snapshot().upload_progress_by_track_id.get(&self.track.id).copied().unwrap_or(0.0)
            * 100.0
    }

    fn can_save(&self, cx: &App) -> bool {
        let clean_title = trimming_whitespaces_and_newlines(&self.title);
        let clean_artist = trimming_whitespaces_and_newlines(&self.artist_name);
        let changed =
            clean_title != self.track.title || clean_artist != self.track.artist || self.local_cover_file.is_some();
        changed
            && !clean_title.is_empty()
            && !clean_artist.is_empty()
            && MusicFieldRules::local_track_title().is_satisfied(clean_title)
            && MusicFieldRules::local_track_artist().is_satisfied(clean_artist)
            && !self.is_saving(cx)
    }

    /// same row as in the add local track sheet
    /// label keeps its own width
    /// canon rule cuts input at the server ceiling
    fn text_field_row(
        name: &'static str,
        label: String,
        value: Binding<String>,
        current: &str,
        rule: RiseFieldRule,
        theme: &AppTheme,
    ) -> impl IntoElement {
        let trailing =
            if LayoutDirection::current().is_right_to_left() { TextAlign::Left } else { TextAlign::Right };
        hstack(12.0)
            .w_full()
            .px(px(16.0))
            .min_h(px(54.0))
            .bg(theme.bg._200)
            .child(
                div()
                    .flex_shrink_0()
                    .child(MainText::new(label).px(14.0).color(theme.text.secondary).number_of_lines(1)),
            )
            .child(
                ShapedTextField::new(
                    SharedString::from(format!("edit-local-track-field-{name}")),
                    "",
                    rule.binding(value),
                )
                .text_align(trailing),
            )
            .child(InputFieldAccessory::new(
                SharedString::from(format!("edit-local-track-accessory-{name}")),
                rule.remaining(current),
                None,
            ))
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if !self.can_save(cx) {
            return;
        }
        self.did_submit = true;
        cx.notify();
        let on_dismiss = self.on_dismiss.clone();
        let this = cx.entity().downgrade();
        let track_id = self.track.id.clone();
        let title = self.title.clone();
        let artist_name = self.artist_name.clone();
        let local_cover_file = self.local_cover_file.clone();
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.update_local_track(
                &track_id,
                &title,
                &artist_name,
                local_cover_file,
                Some(Box::new(move |cx: &mut App| on_dismiss(cx))),
                Some(Box::new(move |message: String, cx: &mut App| {
                    this.update(cx, |sheet, cx| sheet.present_error(message, cx)).ok();
                })),
                cx,
            );
        });
    }
}

impl EditLocalTrackSheet {
    fn present_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.did_submit = false;
        let message = self.error_message.insert(message).clone();
        let this = cx.entity().downgrade();
        NativeConfirmAlert::present_notice(
            &tr("music_local_upload_error_title"),
            &message,
            None,
            None,
            move |cx: &mut App| {
                this.update(cx, |sheet, cx| {
                    sheet.error_message = None;
                    cx.notify();
                })
                .ok();
            },
            cx,
        );
        cx.notify();
    }
}

impl Render for EditLocalTrackSheet {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let this = cx.entity();
        let is_saving = self.is_saving(cx);
        let upload_progress = self.upload_progress(cx);
        let title_rule = MusicFieldRules::local_track_title();
        let artist_rule = MusicFieldRules::local_track_artist();
        let title = Binding::entity(this.clone(), |sheet: &Self| sheet.title.clone(), |sheet, value, _| {
            sheet.title = value;
        });
        let artist_name = Binding::entity(this.clone(), |sheet: &Self| sheet.artist_name.clone(), |sheet, value, _| {
            sheet.artist_name = value;
        });
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
        let form = vstack(18.0)
            .w_full()
            .pt(px(4.0))
            .pb(px(6.0))
            .child(MainText::new(tr("ctx_edit")).px(18.0).font_weight(FontWeight::BOLD).color(theme.text.primary))
            .child(
                MusicCoverPickerUi::new("edit-local-track-cover", local_cover_file, cover_preview)
                    .existing_cover_url(Some(self.track.cover_image_url.clone()))
                    .placeholder_label(Some(tr("music_upload_cover")))
                    .cover_upload_progress(upload_progress)
                    .show_upload_overlay(is_saving && self.local_cover_file.is_some())
                    .size(126.0)
                    .corner_radius(63.0),
            )
            .child(GroupedBtns::new(
                "edit-local-track-fields",
                vec![
                    GroupedBtn::new("title", tr("music_upload_title")).disabled(is_saving).content_slot(
                        Self::text_field_row("title", tr("music_upload_title"), title, &self.title, title_rule, &theme),
                    ),
                    GroupedBtn::new("artist", tr("music_local_upload_artist")).disabled(is_saving).content_slot(
                        Self::text_field_row(
                            "artist",
                            tr("music_local_upload_artist"),
                            artist_name,
                            &self.artist_name,
                            artist_rule,
                            &theme,
                        ),
                    ),
                ],
            ))
            .children(is_saving.then(|| {
                div()
                    .w_full()
                    .px(px(16.0))
                    .child(progress_view_value((upload_progress / 100.0).clamp(0.0, 1.0)).tint(theme.primary._100))
            }));
        let on_save = this.clone();
        let submit = this.clone();
        vstack(8.0)
            .w_full()
            .max_h_full()
            .bg(theme.bg._100)
            .form_primary(self.can_save(cx), move |_, cx| submit.update(cx, |sheet, cx| sheet.save(cx)))
            .child(div().id("edit-local-track-scroll").w_full().flex_shrink_1().min_h_0().overflow_y_scroll().child(form))
            .child(
                div().w_full().flex_shrink_0().px(px(12.0)).child(
                    ButtonUi::new("edit-local-track-save")
                        .disabled(!self.can_save(cx))
                        .on_press(move |_, cx| on_save.update(cx, |sheet, cx| sheet.save(cx)))
                        .child(MainText::new(if is_saving { tr("music_uploading") } else { tr("music_save") })),
                ),
            )
    }
}
