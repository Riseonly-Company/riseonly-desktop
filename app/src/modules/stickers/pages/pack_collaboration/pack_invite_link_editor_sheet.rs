use std::collections::HashMap;

use gpui::{AnyWindowHandle, App, Context, ElementId, Entity, FontWeight, Hsla, SharedString, Window};
use rise_core::foundation::foundation_date::Date;
use rise_i18n::{tr, tr_plural};
use rise_limits::service_limit::MAX_PACK_INVITE_LINK_USAGE_LIMIT;
use rise_ui::main_text::MainText;
use rise_ui::layer_keys::LayerKeysExt;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::segmented_picker::SegmentedPicker;
use rise_ui::switch_ui::SwitchRowUi;

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::input_ui::InputUi;
use crate::core::widgets::notifier::notifier_host::NotifierHost;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::stickers::engine::rise_pack_collaboration_repository::RisePackInviteLink;
use crate::modules::stickers::pages::pack_collaboration::pack_invite_links_screen::PackInviteLinkEditorTarget;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_coauthors_types::PackInviteLinkDraft;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;

const NAME_FIELD: &str = "editor.name";
const LIMIT_FIELD: &str = "editor.limit";

fn sheet_id(pack_id: &str, target: &str, name: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("pack-invite-link-editor-{pack_id}-{target}-{name}")))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExpiryPreset {
    Keep,
    None,
    Hour,
    Day,
    Week,
}

impl ExpiryPreset {
    fn title_key(self) -> &'static str {
        match self {
            Self::Keep => "sticker_pack_invite_link_expiry_keep",
            Self::None => "chat_links_expiry_none",
            Self::Hour => "chat_links_expiry_hour",
            Self::Day => "chat_links_expiry_day",
            Self::Week => "chat_links_expiry_week",
        }
    }

    fn seconds(self) -> Option<f64> {
        match self {
            Self::Keep | Self::None => None,
            Self::Hour => Some(3600.0),
            Self::Day => Some(86_400.0),
            Self::Week => Some(604_800.0),
        }
    }
}

/// state of the sheet, gone when it stops rendering so a new presentation starts clean
/// made with the edited link to prefill it
struct PackInviteLinkEditorState {
    values: HashMap<String, String>,
    expiry: ExpiryPreset,
    requires_approval: bool,
    is_saving: bool,
}

impl PackInviteLinkEditorState {
    fn new(edited_link: Option<&RisePackInviteLink>, _cx: &mut Context<Self>) -> Self {
        let mut state =
            Self { values: HashMap::new(), expiry: ExpiryPreset::None, requires_approval: false, is_saving: false };
        if let Some(link) = edited_link {
            state.values.insert(NAME_FIELD.to_owned(), link.name.clone().unwrap_or_default());
            state.values.insert(LIMIT_FIELD.to_owned(), link.usage_limit.map(|limit| limit.to_string()).unwrap_or_default());
            state.expiry = if link.expires_at_ms.is_some() { ExpiryPreset::Keep } else { ExpiryPreset::None };
            state.requires_approval = link.creates_join_request;
        }
        state
    }

    fn value(&self, name: &str) -> &str {
        self.values.get(name).map(String::as_str).unwrap_or_default()
    }

    fn usage_limit_error(&self) -> Option<String> {
        let digits = self.value(LIMIT_FIELD);
        if digits.is_empty() {
            return None;
        }
        let ceiling = rise_limits::signed(MAX_PACK_INVITE_LINK_USAGE_LIMIT);
        match digits.parse::<i64>() {
            Ok(limit) if limit <= ceiling => None,
            _ => Some(tr_plural("people_picker_limit_reached", ceiling)),
        }
    }

    fn draft(&self, edited_link: Option<&RisePackInviteLink>) -> PackInviteLinkDraft {
        let expires_at_ms = if self.expiry == ExpiryPreset::Keep {
            edited_link.and_then(|link| link.expires_at_ms)
        } else {
            self.expiry.seconds().map(|seconds| ((Date::now().time_interval_since_1970() + seconds) * 1000.0) as i64)
        };
        PackInviteLinkDraft {
            name: self.values.get(NAME_FIELD).cloned(),
            expires_at_ms,
            usage_limit: self.value(LIMIT_FIELD).parse::<i64>().ok(),
            creates_join_request: self.requires_approval,
        }
    }
}

/// creates or edits a pack invite link, name and lifetime and usage limit and join by request
#[derive(IntoElement)]
pub struct PackInviteLinkEditorSheet {
    pack_id: String,
    target: PackInviteLinkEditorTarget,
}

impl PackInviteLinkEditorSheet {
    pub fn new(pack_id: impl Into<String>, target: PackInviteLinkEditorTarget) -> Self {
        Self { pack_id: pack_id.into(), target }
    }

    fn edited_link(&self) -> Option<&RisePackInviteLink> {
        match &self.target {
            PackInviteLinkEditorTarget::Edit(link) => Some(link),
            PackInviteLinkEditorTarget::Create => None,
        }
    }

    fn presets(edited_link: Option<&RisePackInviteLink>) -> Vec<ExpiryPreset> {
        if edited_link.is_some_and(|link| link.expires_at_ms.is_some()) {
            vec![ExpiryPreset::Keep, ExpiryPreset::None, ExpiryPreset::Hour, ExpiryPreset::Day, ExpiryPreset::Week]
        } else {
            vec![ExpiryPreset::None, ExpiryPreset::Hour, ExpiryPreset::Day, ExpiryPreset::Week]
        }
    }

    fn close(dismiss_action: &DismissAction, window_handle: AnyWindowHandle, cx: &mut App) {
        let dismiss_action = dismiss_action.clone();
        cx.defer(move |cx| {
            window_handle.update(cx, |_, window, cx| dismiss_action.call_as_function(window, cx)).ok();
        });
    }

    fn save(
        pack_id: &str,
        edited_link: Option<&RisePackInviteLink>,
        state: &Entity<PackInviteLinkEditorState>,
        dismiss_action: &DismissAction,
        window: &Window,
        cx: &mut App,
    ) {
        let sheet = state.read(cx);
        if sheet.usage_limit_error().is_some() || sheet.is_saving {
            return;
        }
        let draft = sheet.draft(edited_link);
        state.update(cx, |sheet, cx| {
            sheet.is_saving = true;
            cx.notify();
        });
        let window_handle = window.window_handle();
        let success_state = state.clone();
        let failure_state = state.clone();
        let success_dismiss = dismiss_action.clone();
        let on_success = move |_: RisePackInviteLink, cx: &mut App| {
            success_state.update(cx, |sheet, cx| {
                sheet.is_saving = false;
                cx.notify();
            });
            Self::close(&success_dismiss, window_handle, cx);
        };
        let on_failure = move |text: String, cx: &mut App| {
            failure_state.update(cx, |sheet, cx| {
                sheet.is_saving = false;
                cx.notify();
            });
            Notifier::send(
                text,
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        };
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| match edited_link {
            Some(link) => interactions.edit_pack_invite_link(link, draft, on_success, on_failure, cx),
            None => interactions.create_pack_invite_link(pack_id, draft, on_success, on_failure, cx),
        });
    }

    fn toolbar_button(id: ElementId, title: String, color: Hsla, weight: FontWeight) -> PlainButton {
        plain(id).child(MainText::new(title).px(17.0).font_weight(weight).color(color).number_of_lines(1))
    }
}

impl RenderOnce for PackInviteLinkEditorSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let target_id = self.target.id();
        let pack_id = self.pack_id.clone();
        let edited = self.edited_link().cloned();
        let initial = edited.clone();
        let state = window.use_keyed_state(sheet_id(&pack_id, &target_id, "state"), cx, move |_, cx| {
            PackInviteLinkEditorState::new(initial.as_ref(), cx)
        });
        let notifier = window.use_keyed_state(sheet_id(&pack_id, &target_id, "notifier"), cx, |_, cx| NotifierHost::new(cx));
        let dismiss_action = dismiss(cx);

        let presets = Self::presets(edited.as_ref());
        let sheet = state.read(cx);
        let values = sheet.values.clone();
        let is_saving = sheet.is_saving;
        let selected_index = presets.iter().position(|preset| *preset == sheet.expiry).unwrap_or(0);
        let usage_limit_error = sheet.usage_limit_error();
        let done_enabled = usage_limit_error.is_none();
        let title = if edited.is_none() { tr("chat_links_create") } else { tr("sticker_pack_invite_link_edit") };

        let set_name = state.clone();
        let set_limit = state.clone();
        let set_expiry = state.clone();
        let picker_presets = presets.clone();
        let cancel_dismiss = dismiss_action.clone();
        let done_state = state.clone();
        let done_pack_id = pack_id.clone();
        let done_link = edited.clone();
        let (submit_state, submit_pack_id, submit_link) = (state.clone(), pack_id.clone(), edited.clone());
        let submit_dismiss = dismiss_action.clone();

        let toolbar_trailing = if is_saving {
            progress_view().into_any_element()
        } else {
            Self::toolbar_button(
                sheet_id(&pack_id, &target_id, "done"),
                tr("common_done"),
                if done_enabled { theme.primary._100 } else { theme.text.secondary },
                FontWeight::SEMIBOLD,
            )
            .disabled(!done_enabled)
            .on_press(move |window, cx| {
                Self::save(&done_pack_id, done_link.as_ref(), &done_state, &dismiss_action, window, cx);
            })
            .into_any_element()
        };

        div()
            .relative()
            .size_full()
            .bg(theme.bg._100)
            .form_primary(done_enabled && !is_saving, move |window, cx| {
                Self::save(&submit_pack_id, submit_link.as_ref(), &submit_state, &submit_dismiss, window, cx);
            })
            .child(
                vstack(0.0)
                    .size_full()
                    .child(
                        hstack(8.0)
                            .w_full()
                            .h(px(52.0))
                            .px(px(16.0))
                            .child(
                                div().flex_1().min_w_0().flex().justify_start().child(
                                    Self::toolbar_button(
                                        sheet_id(&pack_id, &target_id, "cancel"),
                                        tr("common_cancel"),
                                        theme.primary._100,
                                        FontWeight::NORMAL,
                                    )
                                    .on_press(move |window, cx| cancel_dismiss.call_as_function(window, cx)),
                                ),
                            )
                            .child(
                                MainText::new(title)
                                    .px(17.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.text.primary)
                                    .number_of_lines(1),
                            )
                            .child(div().flex_1().min_w_0().flex().justify_end().child(toolbar_trailing)),
                    )
                    .child(
                        div().flex_1().min_h_0().w_full().child(
                            scroll_view(sheet_id(&pack_id, &target_id, "scroll")).size_full().bg(theme.bg._100).child(
                                vstack(22.0)
                                    .w_full()
                                    .p(px(16.0))
                                    .align(HorizontalAlignment::Leading)
                                    .child(
                                        InputUi::new(sheet_id(&pack_id, &target_id, "name"))
                                            .values(values.clone())
                                            .name(NAME_FIELD)
                                            .placeholder(tr("chat_links_name_placeholder"))
                                            .rule(RiseFieldRule::new("stickers_create_pack_invite_link", "/name"))
                                            .set_value(move |_, value, _, cx| {
                                                set_name.update(cx, |sheet, cx| {
                                                    sheet.values.insert(NAME_FIELD.to_owned(), value.to_owned());
                                                    cx.notify();
                                                });
                                            }),
                                    )
                                    .child(
                                        vstack(8.0)
                                            .w_full()
                                            .align(HorizontalAlignment::Leading)
                                            .child(MainText::new(tr("chat_links_expiry_title")).px(13.0).color(theme.text.secondary))
                                            .child(SegmentedPicker::new(
                                                sheet_id(&pack_id, &target_id, "expiry"),
                                                presets.iter().map(|preset| SharedString::from(tr(preset.title_key()))).collect(),
                                                selected_index,
                                                move |index, _, cx| {
                                                    let Some(preset) = picker_presets.get(index).copied() else {
                                                        return;
                                                    };
                                                    set_expiry.update(cx, |sheet, cx| {
                                                        sheet.expiry = preset;
                                                        cx.notify();
                                                    });
                                                },
                                            )),
                                    )
                                    .child(
                                        InputUi::new(sheet_id(&pack_id, &target_id, "limit"))
                                            .values(values)
                                            .error_message(usage_limit_error)
                                            .name(LIMIT_FIELD)
                                            .placeholder(tr("chat_links_usage_limit_placeholder"))
                                            .format(|raw, _| raw.chars().filter(|character| character.is_numeric()).collect())
                                            .set_value(move |_, value, _, cx| {
                                                set_limit.update(cx, |sheet, cx| {
                                                    sheet.values.insert(LIMIT_FIELD.to_owned(), value.to_owned());
                                                    cx.notify();
                                                });
                                            }),
                                    )
                                    .child(
                                        vstack(6.0)
                                            .w_full()
                                            .align(HorizontalAlignment::Leading)
                                            .child(div().w_full().child(div().mx(px(-16.0)).child(SwitchRowUi::new(
                                                sheet_id(&pack_id, &target_id, "approval"),
                                                tr("sticker_pack_invite_link_request_toggle"),
                                                Binding::entity(
                                                    state.clone(),
                                                    |sheet: &PackInviteLinkEditorState| sheet.requires_approval,
                                                    |sheet, value, _| sheet.requires_approval = value,
                                                ),
                                            ))))
                                            .child(
                                                MainText::new(tr("sticker_pack_invite_link_request_hint"))
                                                    .px(12.0)
                                                    .color(theme.text.secondary)
                                                    .number_of_lines(3),
                                            ),
                                    ),
                            ),
                        ),
                    ),
            )
            .child(div().absolute().inset_0().child(notifier))
    }
}
