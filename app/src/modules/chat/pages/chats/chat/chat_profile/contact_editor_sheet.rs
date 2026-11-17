use std::sync::Arc;

use gpui::{AnyWindowHandle, App, Axis, Context, ElementId, Entity, FontWeight, SharedString, Window};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::swiftui::prelude::*;
use rise_ui::switch_ui::SwitchUi;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::profile_field_formatting::PhoneNumberFormatting;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapEncoder};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::select_media_ui::SelectMediaUi;
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::user::shared::contact_device_book::ContactDeviceBook;
use crate::modules::user::stores::user::user_actions::user_contact_types::{
    UserContactDraft, UserContactMutationError, UserContactPeer,
};
use crate::modules::user::stores::user::user_actions::user_contact_wire_types::{
    SavedContactDtoExt, UserSavedContactWireDTO,
};
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

const SHEET_CORNER_RADIUS: f32 = 30.0;
const AVATAR_JPEG_QUALITY: u8 = 90;

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

fn trimmed(raw: &str) -> &str {
    trimming_whitespaces_and_newlines(raw)
}

/// state of the sheet, it stays mounted between presentations so a new one calls reset
struct ContactEditorSheetState {
    first_name: String,
    last_name: String,
    note: String,
    share_phone_number: bool,
    sync_to_phone: bool,
    avatar_local_file: Option<SaiWsOptimisticLocalFile>,
    avatar_preview: Option<CleverImage>,
    suggestion_local_file: Option<SaiWsOptimisticLocalFile>,
    suggestion_preview: Option<CleverImage>,
    is_saving: bool,
    is_suggesting: bool,
    error_text: Option<String>,
    shows_contact_settings_action: bool,
}

impl ContactEditorSheetState {
    fn new(peer: &UserContactPeer, existing: Option<&UserSavedContactWireDTO>) -> Self {
        let components: Vec<&str> = trimmed(&peer.name).split(' ').filter(|part| !part.is_empty()).collect();
        Self {
            first_name: existing
                .map(|existing| existing.first_name.clone())
                .or_else(|| components.first().map(|first| (*first).to_owned()))
                .unwrap_or_else(|| peer.name.clone()),
            last_name: existing
                .map(|existing| existing.last_name.clone())
                .unwrap_or_else(|| components.iter().skip(1).copied().collect::<Vec<_>>().join(" ")),
            note: existing.map(|existing| existing.note.clone()).unwrap_or_default(),
            share_phone_number: existing.is_none_or(|existing| existing.share_phone_number),
            sync_to_phone: existing.is_none_or(|existing| existing.sync_to_phone)
                && ContactDeviceBook::authorization().can_read_and_write(),
            avatar_local_file: None,
            avatar_preview: None,
            suggestion_local_file: None,
            suggestion_preview: None,
            is_saving: false,
            is_suggesting: false,
            error_text: None,
            shows_contact_settings_action: false,
        }
    }

    fn reset(&mut self, peer: &UserContactPeer, existing: Option<&UserSavedContactWireDTO>) {
        *self = Self::new(peer, existing);
    }

    fn can_save(&self) -> bool {
        !self.is_saving && !trimmed(&self.first_name).is_empty()
    }
}

#[derive(Clone)]
struct ContactEditorSheetParts {
    id: ElementId,
    peer: UserContactPeer,
    existing: Option<UserSavedContactWireDTO>,
    state: Entity<ContactEditorSheetState>,
    is_presented: Binding<bool>,
    window: AnyWindowHandle,
}

impl ContactEditorSheetParts {
    fn is_editing(&self) -> bool {
        self.existing.is_some()
    }

    /// dismiss from a store callback, which runs without a window
    fn dismiss(&self, cx: &mut App) {
        let is_presented = self.is_presented.clone();
        let window = self.window;
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| is_presented.set(false, window, cx));
        });
    }

    fn update_state(&self, cx: &mut App, change: impl FnOnce(&mut ContactEditorSheetState)) {
        self.state.update(cx, |state, cx| {
            change(state);
            cx.notify();
        });
    }

    fn hint_name(&self, cx: &App) -> String {
        let first_name = self.state.read(cx).first_name.clone();
        let typed = trimmed(&first_name);
        if !typed.is_empty() {
            return typed.to_owned();
        }
        let real = trimmed(&self.peer.name);
        if let Some(first) = real.split(' ').next().filter(|first| !first.is_empty()) {
            return first.to_owned();
        }
        if self.peer.tag.is_empty() { real.to_owned() } else { format!("@{}", self.peer.tag) }
    }

    fn remote_avatar(&self) -> String {
        self.existing.as_ref().map_or_else(|| self.peer.logo.clone(), SavedContactDtoExt::effective_avatar_url)
    }

    fn peer_phone(&self) -> Option<String> {
        let raw = trimmed(self.peer.phone.as_deref()?);
        (!raw.is_empty()).then(|| PhoneNumberFormatting::format(raw))
    }

    fn private_avatar_cache_policy(&self) -> CleverImageCachePolicy {
        let has_override = self
            .existing
            .as_ref()
            .and_then(|existing| existing.avatar_override_url.as_deref())
            .is_some_and(|url| !trimmed(url).is_empty());
        if has_override { CleverImageCachePolicy::PrivateEphemeral } else { CleverImageCachePolicy::Standard }
    }

    fn contact_photo_action_text(&self, key: &str, cx: &App) -> String {
        let first_name = self.state.read(cx).first_name.clone();
        let name = trimmed(&first_name);
        tr(key).replace("{name}", if name.is_empty() { self.peer.name.as_str() } else { name })
    }

    fn save_contact(&self, cx: &mut App) {
        {
            let state = self.state.read(cx);
            tracing::warn!(
                target: "riseonly::X908",
                can_save = state.can_save(),
                saving = state.is_saving,
                has_photo = state.avatar_local_file.is_some(),
                has_preview = state.avatar_preview.is_some(),
                "contact done pressed"
            );
        }
        if !self.state.read(cx).can_save() {
            return;
        }
        self.update_state(cx, |state| {
            state.is_saving = true;
            state.error_text = None;
            state.shows_contact_settings_action = false;
        });
        let draft = {
            let state = self.state.read(cx);
            UserContactDraft {
                target_user_id: self.peer.id.clone(),
                first_name: state.first_name.clone(),
                last_name: state.last_name.clone(),
                note: state.note.clone(),
                share_phone_number: state.share_phone_number,
                sync_to_phone: state.sync_to_phone,
            }
        };
        let parts = self.clone();
        UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.save_contact(
                draft,
                self.peer.clone(),
                move |result, cx: &mut App| match result {
                    Ok(_) => {
                        tracing::warn!(target: "riseonly::X908", "contact saved");
                        parts.save_photo_after_contact(cx);
                    }
                    Err(error) => {
                        tracing::warn!(target: "riseonly::X908", %error, "contact save failed");
                        let mutation = error.downcast_ref::<UserContactMutationError>();
                        let text = Self::failure_text(&error);
                        let permission_missing =
                            matches!(mutation, Some(UserContactMutationError::ContactsPermissionUnavailable));
                        parts.update_state(cx, |state| {
                            state.is_saving = false;
                            state.error_text = Some(text);
                            if permission_missing {
                                state.shows_contact_settings_action = true;
                            }
                        });
                    }
                },
                cx,
            );
        });
    }

    fn save_photo_after_contact(&self, cx: &mut App) {
        let (local_file, preview) = {
            let state = self.state.read(cx);
            (state.avatar_local_file.clone(), state.avatar_preview.clone())
        };
        let Some(local_file) = local_file else {
            self.update_state(cx, |state| state.is_saving = false);
            self.dismiss(cx);
            return;
        };
        let parts = self.clone();
        let interactions = UserInteractionsStore::shared(cx);
        cx.spawn(async move |cx| {
            let image_data = cx
                .background_executor()
                .spawn(async move {
                    preview.and_then(|preview| CleverImageBitmapEncoder::jpeg_data(&preview, AVATAR_JPEG_QUALITY))
                })
                .await;
            tracing::warn!(target: "riseonly::X908", jpeg = image_data.as_ref().map_or(0, Vec::len), "contact photo encoded");
            let target_user_id = parts.peer.id.clone();
            interactions.update(cx, |interactions, cx| {
                interactions.set_contact_photo(
                    &target_user_id,
                    local_file,
                    image_data,
                    move |result, cx: &mut App| {
                        let failure = result.err().map(|error| Self::failure_text(&error));
                        let succeeded = failure.is_none();
                        tracing::warn!(target: "riseonly::X908", failure = ?failure, "contact photo result");
                        parts.update_state(cx, |state| {
                            state.is_saving = false;
                            if failure.is_some() {
                                state.error_text = failure;
                            }
                        });
                        if succeeded {
                            parts.dismiss(cx);
                        }
                    },
                    cx,
                );
            });
        })
        .detach();
    }

    /// only local contact texts and canon phrases reach the sheet
    /// a transport error reads as the retry text and never as its rust description
    fn failure_text(error: &anyhow::Error) -> String {
        if error.downcast_ref::<UserContactMutationError>().is_some() {
            return error.to_string();
        }
        CanonErrors::refusal_text(error)
    }

    fn submit_photo_suggestion(&self, cx: &mut App) {
        let (local_file, is_suggesting) = {
            let state = self.state.read(cx);
            (state.suggestion_local_file.clone(), state.is_suggesting)
        };
        let Some(local_file) = local_file.filter(|_| !is_suggesting) else {
            return;
        };
        self.update_state(cx, |state| {
            state.is_suggesting = true;
            state.error_text = None;
        });
        let parts = self.clone();
        UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.suggest_contact_photo(
                &self.peer.id,
                local_file,
                move |result, cx: &mut App| {
                    let failure = result.err().map(|error| Self::failure_text(&error));
                    parts.update_state(cx, |state| {
                        state.is_suggesting = false;
                        state.suggestion_local_file = None;
                        state.suggestion_preview = None;
                        if failure.is_some() {
                            state.error_text = failure;
                        }
                    });
                },
                cx,
            );
        });
    }

    fn delete_contact(&self, cx: &mut App) {
        if self.state.read(cx).is_saving {
            return;
        }
        self.update_state(cx, |state| {
            state.is_saving = true;
            state.error_text = None;
        });
        let parts = self.clone();
        UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.delete_contact(
                &self.peer.id,
                move |result, cx: &mut App| {
                    let failure = result.err().map(|error| Self::failure_text(&error));
                    let succeeded = failure.is_none();
                    parts.update_state(cx, |state| {
                        state.is_saving = false;
                        if failure.is_some() {
                            state.error_text = failure;
                        }
                    });
                    if succeeded {
                        parts.dismiss(cx);
                    }
                },
                cx,
            );
        });
    }

    fn confirm_delete(&self, cx: &mut App) {
        let parts = self.clone();
        NativeConfirmAlert::present_delete(
            &tr("contact_delete_confirmation_title"),
            "",
            Some(tr("contact_delete_action")),
            None,
            false,
            None,
            None,
            move |cx| parts.delete_contact(cx),
            cx,
        );
    }

    fn toggle_share_phone_number(&self, cx: &mut App) {
        self.update_state(cx, |state| state.share_phone_number = !state.share_phone_number);
    }

    fn toggle_sync_to_phone(&self, cx: &mut App) {
        self.update_state(cx, |state| {
            state.sync_to_phone = !state.sync_to_phone;
            if !state.sync_to_phone && state.shows_contact_settings_action {
                state.shows_contact_settings_action = false;
                state.error_text = None;
            }
        });
    }

    fn string_binding(
        &self,
        get: impl Fn(&ContactEditorSheetState) -> &String + 'static,
        set: impl Fn(&mut ContactEditorSheetState, String) + 'static,
    ) -> Binding<String> {
        Binding::entity(self.state.clone(), move |state| get(state).clone(), move |state, value, _| set(state, value))
    }

    fn avatar_bindings(&self) -> (Binding<Option<SaiWsOptimisticLocalFile>>, Binding<Option<CleverImage>>) {
        (
            Binding::entity(
                self.state.clone(),
                |state: &ContactEditorSheetState| state.avatar_local_file.clone(),
                |state, value, _| state.avatar_local_file = value,
            ),
            Binding::entity(
                self.state.clone(),
                |state: &ContactEditorSheetState| state.avatar_preview.clone(),
                |state, value, _| state.avatar_preview = value,
            ),
        )
    }

    fn suggestion_bindings(&self) -> (Binding<Option<SaiWsOptimisticLocalFile>>, Binding<Option<CleverImage>>) {
        (
            Binding::entity(
                self.state.clone(),
                |state: &ContactEditorSheetState| state.suggestion_local_file.clone(),
                |state, value, _| state.suggestion_local_file = value,
            ),
            Binding::entity(
                self.state.clone(),
                |state: &ContactEditorSheetState| state.suggestion_preview.clone(),
                |state, value, _| state.suggestion_preview = value,
            ),
        )
    }

    fn body(&self, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let is_editing = self.is_editing();
        let (error_text, shows_settings) = {
            let state = self.state.read(cx);
            (state.error_text.clone().filter(|text| !text.is_empty()), state.shows_contact_settings_action)
        };

        let mut content = vstack(20.0).w_full().px(px(16.0)).pt(px(18.0));
        if is_editing {
            content = content.child(self.edit_avatar(cx)).child(self.edit_name_card(&theme));
        } else {
            content = content.child(self.new_contact_identity_card(cx, &theme));
            if let Some(peer_phone) = self.peer_phone() {
                content = content.child(self.peer_phone_card(peer_phone, &theme));
            }
            content = content
                .child(self.share_phone_card(cx, &theme))
                .child(self.phone_privacy_hint(cx, &theme))
                .child(self.sync_card(cx, &theme));
        }
        content = content.child(self.note_card(&theme));
        if is_editing {
            content = content
                .child(self.hint_text(tr("contact_note_private_hint"), 2, &theme))
                .child(self.photo_actions(cx, &theme))
                .child(self.hint_text(
                    tr("contact_photo_private_hint").replace("{name}", &self.hint_name(cx)),
                    4,
                    &theme,
                ))
                .child(self.delete_card(cx, &theme));
        }
        if let Some(error_text) = error_text {
            content = content.child(
                div().px(px(20.0)).child(
                    MainText::new(error_text)
                        .px(13.0)
                        .tac(TextAlignT::Center)
                        .color(theme.semantic.error_100)
                        .number_of_lines(3),
                ),
            );
            if shows_settings {
                content = content.child(
                    plain(named_child(&self.id, "open-settings"))
                        .mt(px(-12.0))
                        .px(px(16.0))
                        .min_h(px(44.0))
                        .flex()
                        .items_center()
                        .on_press(move |_, cx| {
                            UserInteractionsStore::shared(cx)
                                .update(cx, |interactions, cx| interactions.open_contact_system_settings(cx));
                        })
                        .child(
                            MainText::new(tr("contact_sync_open_settings"))
                                .px(15.0)
                                .font_weight(FontWeight::MEDIUM)
                                .color(theme.primary._100),
                        ),
                );
            }
        }
        content = content.child(div().w_full().h(px(30.0)));

        vstack(0.0)
            .size_full()
            .bg(theme.bg._100)
            .child(self.header(cx, &theme))
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .child(scroll_view(named_child(&self.id, "scroll")).child(content)),
            )
            .into_any_element()
    }

    fn hint_text(&self, text: String, lines: usize, theme: &AppTheme) -> Div {
        div()
            .w_full()
            .px(px(16.0))
            .mt(px(-12.0))
            .child(MainText::new(text).px(13.0).color(theme.text.secondary).number_of_lines(lines))
    }

    fn header(&self, cx: &mut App, theme: &AppTheme) -> Div {
        let (is_saving, is_suggesting, can_save) = {
            let state = self.state.read(cx);
            (state.is_saving, state.is_suggesting, state.can_save())
        };
        let is_editing = self.is_editing();
        let dismissing = self.is_presented.clone();
        let leading = if is_editing {
            self.header_capsule(
                "cancel",
                tr("common_cancel"),
                !is_saving && !is_suggesting,
                theme,
                move |window, cx| dismissing.set(false, window, cx),
            )
            .into_any_element()
        } else {
            PageHeaderIconButton::new(named_child(&self.id, "close"), "xmark", move |window, cx| {
                dismissing.set(false, window, cx);
            })
            .font_size(17.0)
            .disabled(is_saving)
            .into_any_element()
        };
        let saving = self.clone();
        let trailing = if is_editing {
            self.header_capsule("done", tr("common_done"), can_save, theme, move |_, cx| saving.save_contact(cx))
                .into_any_element()
        } else {
            let ink = theme.content_on(theme.primary._100);
            plain(named_child(&self.id, "save"))
                .flex_none()
                .w(px(48.0))
                .h(px(48.0))
                .circle()
                .bg(theme.primary._100)
                .flex()
                .items_center()
                .justify_center()
                .opacity(if can_save { 1.0 } else { 0.42 })
                .disabled(!can_save)
                .on_press(move |_, cx| saving.save_contact(cx))
                .when(is_saving, |button| button.child(progress_view().tint(ink)))
                .when(!is_saving, |button| button.children(IconUi::sized("checkmark", px(20.0), ink, false)))
                .into_any_element()
        };

        div()
            .relative()
            .w_full()
            .flex_none()
            .px(px(16.0))
            .pt(px(12.0))
            .pb(px(8.0))
            .when(!is_editing, |header| {
                header.child(
                    div().absolute().inset_0().flex().items_center().justify_center().child(
                        MainText::new(tr("contact_new_title"))
                            .px(18.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .number_of_lines(1),
                    ),
                )
            })
            .child(hstack(DEFAULT_STACK_SPACING).w_full().child(leading).child(spacer()).child(trailing))
    }

    fn header_capsule(
        &self,
        id: &'static str,
        text: String,
        enabled: bool,
        theme: &AppTheme,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> PlainButton {
        plain(named_child(&self.id, id))
            .relative()
            .px(px(16.0))
            .h(px(44.0))
            .flex()
            .items_center()
            .rounded_full()
            .bg(theme.bg._200)
            .opacity(if enabled { 1.0 } else { 0.45 })
            .disabled(!enabled)
            .on_press(action)
            .child(centred_stroke(theme.border._200.opacity(0.65), 0.7, Shape::Capsule))
            .child(MainText::new(text).px(16.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary))
    }

    fn select_media(
        &self,
        id: &'static str,
        bindings: (Binding<Option<SaiWsOptimisticLocalFile>>, Binding<Option<CleverImage>>),
    ) -> SelectMediaUi {
        SelectMediaUi::new(named_child(&self.id, id), bindings.0, bindings.1)
            .picker_configuration(MediaPickerConfiguration::profile_avatar())
    }

    fn avatar_picker(&self, id: &'static str, size: f32, remote_name: String, cx: &App) -> SelectMediaUi {
        let show_upload_overlay = {
            let state = self.state.read(cx);
            state.is_saving && state.avatar_local_file.is_some()
        };
        self.select_media(id, self.avatar_bindings())
            .size(size)
            .placeholder_system_image("camera.fill")
            .show_upload_overlay(show_upload_overlay)
            .remote_source(self.remote_avatar())
            .remote_name(remote_name)
            .remote_user_id(self.peer.id.clone())
            .remote_cache_policy(self.private_avatar_cache_policy())
    }

    fn contact_text_field(
        &self,
        id: &'static str,
        placeholder: String,
        text: Binding<String>,
        theme: &AppTheme,
    ) -> Div {
        div().w_full().min_h(px(52.0)).px(px(16.0)).flex().items_center().child(
            ShapedTextField::new(named_child(&self.id, id), placeholder, text)
                .font_size(17.0)
                .text_color(theme.text.primary),
        )
    }

    fn first_name_field(&self, theme: &AppTheme) -> Div {
        self.contact_text_field(
            "first-name",
            tr("contact_first_name"),
            self.string_binding(|state| &state.first_name, |state, value| state.first_name = value),
            theme,
        )
    }

    fn last_name_field(&self, theme: &AppTheme) -> Div {
        self.contact_text_field(
            "last-name",
            tr("contact_last_name"),
            self.string_binding(|state| &state.last_name, |state, value| state.last_name = value),
            theme,
        )
    }

    fn new_contact_identity_card(&self, cx: &App, theme: &AppTheme) -> Div {
        hstack(16.0)
            .w_full()
            .px(px(16.0))
            .py(px(14.0))
            .bg(theme.bg._200)
            .rounded(px(24.0))
            .child(self.avatar_picker("new-avatar", 68.0, self.peer.name.clone(), cx))
            .child(
                vstack(0.0)
                    .flex_1()
                    .min_w_0()
                    .child(self.first_name_field(theme))
                    .child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55)))
                    .child(self.last_name_field(theme)),
            )
    }

    fn edit_avatar(&self, cx: &App) -> Div {
        let remote_name = self
            .existing
            .as_ref()
            .map_or_else(|| self.peer.name.clone(), SavedContactDtoExt::resolved_display_name);
        div().w_full().flex().justify_center().child(self.avatar_picker("edit-avatar", 112.0, remote_name, cx))
    }

    fn edit_name_card(&self, theme: &AppTheme) -> Div {
        vstack(0.0)
            .w_full()
            .bg(theme.bg._200)
            .rounded(px(24.0))
            .child(self.first_name_field(theme))
            .child(div().w_full().px(px(16.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55))))
            .child(self.last_name_field(theme))
    }

    fn peer_phone_card(&self, phone: String, theme: &AppTheme) -> Div {
        vstack(3.0)
            .w_full()
            .items_start()
            .justify_center()
            .px(px(16.0))
            .min_h(px(74.0))
            .bg(theme.bg._200)
            .rounded(px(24.0))
            .child(MainText::new(tr("contact_phone_mobile")).px(13.0).color(theme.text.secondary))
            .child(MainText::new(phone).px(17.0).color(theme.text.primary))
    }

    /// row and its switch share one toggle
    /// on desktop a click reaches the inner control
    fn switch_card(
        &self,
        id: &'static str,
        title: String,
        is_on: bool,
        disabled: bool,
        toggle: impl Fn(&Self, &mut App) + Clone + 'static,
        theme: &AppTheme,
    ) -> PlainButton {
        let (row_parts, row_toggle) = (self.clone(), toggle.clone());
        let (switch_parts, switch_toggle) = (self.clone(), toggle);
        let binding = Binding::new(move |_: &App| is_on, move |_: bool, _: &mut Window, cx: &mut App| {
            if !disabled {
                switch_toggle(&switch_parts, cx);
            }
        });
        plain(named_child(&self.id, id))
            .w_full()
            .min_h(px(58.0))
            .px(px(16.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .bg(theme.bg._200)
            .rounded(px(24.0))
            .disabled(disabled)
            .on_press(move |_, cx| row_toggle(&row_parts, cx))
            .child(MainText::new(title).px(17.0).color(theme.text.primary).number_of_lines(2))
            .child(spacer())
            .child(SwitchUi::new(named_child(&self.id, SharedString::from(format!("{id}-switch"))), binding))
    }

    fn share_phone_card(&self, cx: &App, theme: &AppTheme) -> PlainButton {
        let state = self.state.read(cx);
        self.switch_card(
            "share-phone",
            tr("contact_share_my_phone"),
            state.share_phone_number,
            state.is_saving,
            Self::toggle_share_phone_number,
            theme,
        )
    }

    fn phone_privacy_hint(&self, cx: &App, theme: &AppTheme) -> Div {
        let key = if self.state.read(cx).share_phone_number {
            "contact_phone_privacy_hint"
        } else {
            "contact_phone_hidden_hint"
        };
        self.hint_text(tr(key).replace("{name}", &self.hint_name(cx)), 3, theme)
    }

    fn sync_card(&self, cx: &App, theme: &AppTheme) -> PlainButton {
        let state = self.state.read(cx);
        self.switch_card(
            "sync-phone",
            tr("contact_sync_to_phone"),
            state.sync_to_phone,
            state.is_saving,
            Self::toggle_sync_to_phone,
            theme,
        )
    }

    fn note_card(&self, theme: &AppTheme) -> Div {
        div().w_full().min_h(px(58.0)).px(px(16.0)).py(px(16.0)).bg(theme.bg._200).rounded(px(24.0)).child(
            ShapedTextField::new(
                named_child(&self.id, "note"),
                tr("contact_note_placeholder"),
                self.string_binding(|state| &state.note, |state, value| state.note = value),
            )
            .axis(Axis::Vertical)
            .max_visible_lines(5)
            .font_size(17.0)
            .text_color(theme.text.primary),
        )
    }

    fn photo_actions(&self, cx: &App, theme: &AppTheme) -> Div {
        let disabled = {
            let state = self.state.read(cx);
            state.is_saving || state.is_suggesting
        };
        vstack(0.0)
            .w_full()
            .bg(theme.bg._200)
            .rounded(px(24.0))
            .child(self.photo_action(
                "suggest-photo",
                self.suggestion_bindings(),
                "person.crop.rectangle.badge.plus",
                "contact_suggest_photo_action",
                true,
                disabled,
                theme,
            ))
            .child(div().w_full().pl(px(56.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55))))
            .child(self.photo_action(
                "set-photo",
                self.avatar_bindings(),
                "camera.badge.ellipsis",
                "contact_set_photo_action",
                false,
                disabled,
                theme,
            ))
    }

    #[allow(clippy::too_many_arguments)]
    fn photo_action(
        &self,
        id: &'static str,
        bindings: (Binding<Option<SaiWsOptimisticLocalFile>>, Binding<Option<CleverImage>>),
        icon: &'static str,
        text_key: &'static str,
        shows_suggesting: bool,
        disabled: bool,
        theme: &AppTheme,
    ) -> Div {
        let parts = self.clone();
        let label_theme = theme.clone();
        let media = self.select_media(id, bindings).disabled(disabled).custom_label(move |_, cx| {
            let loading = shows_suggesting && parts.state.read(cx).is_suggesting;
            parts.photo_action_label(icon, parts.contact_photo_action_text(text_key, cx), loading, &label_theme)
        });
        div().w_full().child(media)
    }

    fn photo_action_label(&self, icon: &'static str, text: String, loading: bool, theme: &AppTheme) -> Div {
        hstack(14.0)
            .w_full()
            .min_h(px(58.0))
            .px(px(16.0))
            .child(
                div()
                    .flex_none()
                    .w(px(28.0))
                    .flex()
                    .justify_center()
                    .children(IconUi::sized(icon, px(20.0), theme.primary._100, false)),
            )
            .child(MainText::new(text).px(17.0).color(theme.primary._100).number_of_lines(2))
            .child(spacer())
            .when(loading, |row| row.child(progress_view().tint(theme.primary._100)))
    }

    fn delete_card(&self, cx: &App, theme: &AppTheme) -> PlainButton {
        let disabled = {
            let state = self.state.read(cx);
            state.is_saving || state.is_suggesting
        };
        let parts = self.clone();
        plain(named_child(&self.id, "delete"))
            .w_full()
            .min_h(px(58.0))
            .px(px(16.0))
            .flex()
            .items_center()
            .bg(theme.bg._200)
            .rounded(px(24.0))
            .disabled(disabled)
            .on_press(move |_, cx| parts.confirm_delete(cx))
            .child(MainText::new(tr("contact_delete_action")).px(17.0).color(theme.semantic.error_100))
    }
}

/// contact editor sheet, the presentation is the sheet own because dismissing reads the sheet state
#[derive(IntoElement)]
pub struct ContactEditorSheet {
    id: ElementId,
    is_presented: Binding<bool>,
    peer: UserContactPeer,
    existing: Option<UserSavedContactWireDTO>,
}

impl ContactEditorSheet {
    pub fn new(
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        peer: UserContactPeer,
        existing: Option<UserSavedContactWireDTO>,
    ) -> Self {
        Self { id: id.into(), is_presented, peer, existing }
    }
}

impl RenderOnce for ContactEditorSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let (seed_peer, seed_existing) = (self.peer.clone(), self.existing.clone());
        let state = window.use_keyed_state(
            named_child(&self.id, "state"),
            cx,
            move |_, _: &mut Context<ContactEditorSheetState>| {
                ContactEditorSheetState::new(&seed_peer, seed_existing.as_ref())
            },
        );

        let presented = self.is_presented.get(cx);
        let (reset_state, reset_peer, reset_existing) = (state.clone(), self.peer.clone(), self.existing.clone());
        on_change(
            named_child(&self.id, "presented"),
            presented,
            false,
            window,
            cx,
            move |_, presented: &bool, _, cx| {
                if *presented {
                    reset_state.update(cx, |state, cx| {
                        state.reset(&reset_peer, reset_existing.as_ref());
                        cx.notify();
                    });
                }
            },
        );

        let parts = ContactEditorSheetParts {
            id: self.id.clone(),
            peer: self.peer,
            existing: self.existing,
            state: state.clone(),
            is_presented: self.is_presented.clone(),
            window: window.window_handle(),
        };

        let suggestion_url = state.read(cx).suggestion_local_file.as_ref().map(|file| file.url.clone());
        let suggesting = parts.clone();
        on_change(named_child(&self.id, "suggestion"), suggestion_url, false, window, cx, move |_, url, _, cx| {
            if url.is_some() {
                suggesting.submit_photo_suggestion(cx);
            }
        });

        let busy = {
            let state = state.read(cx);
            state.is_saving || state.is_suggesting
        };
        NativeBottomSheetPresentationHost::new(
            named_child(&self.id, "sheet"),
            self.is_presented,
            move |_, cx| parts.body(cx),
        )
        .presentation_detents([PresentationDetent::Large])
        .presentation_drag_indicator(false)
        .presentation_background(theme.bg._100)
        .presentation_corner_radius(SHEET_CORNER_RADIUS)
        .interactive_dismiss_disabled(busy)
    }
}
