use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AnyElement, Axis, Context, ElementId, Entity, FontWeight, SharedString, Subscription, Task, Window};
use rise_canon::{RiseGuideAnchor, RiseTimingCanon};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::UrlComponents;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::pasteboard::{UIPasteboard, UIPasteboardItem, UIPasteboardOptions};
use rise_theme::ui_color_system::UIColor;
use rise_theme::{AppTheme, color};
use rise_ui::button_ui::ButtonUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::icon_ui::icon_ui::IconUi;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::{PageHeaderIconButton, PageHeaderToolbarSlot};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::environment_values::IsEnabledKey;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::{SaiDebouncer, SaiDebouncerOptions};
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_configuration::SaiFetchHttpConfiguration;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::select_media_ui::SelectMediaUi;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::bot::components::bot_ui_components::{
    BotBannerPicker, BotSettingsFieldRow, BotStateView, BotToggleRow, bot_symbol_is_solid,
};
use crate::modules::bot::shared::bot_ui_validation::{BotFieldRules, BotFormValidation, BotTagAvailabilityState};
use crate::modules::bot::stores::bot::bot_actions::bot_actions::BotActionsStore;
use crate::modules::bot::stores::bot::bot_actions::bot_types::{
    BotMediaKind, BotModel, BotTagAvailability, BotTagRules, BotTokenResult, BotUpdateValues,
};
use crate::modules::bot::stores::bot::bot_interactions::bot_interactions::BotInteractionsStore;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;

const TAG_DEBOUNCE_ID: &str = "botSettingsTagAvailability";
const TAG_DEBOUNCE_GROUP: &str = "botSettings";
const TAG_CHECK_TIMEOUT: Duration = Duration::from_secs(8);
const SENSITIVE_COPY_LIFETIME_SECONDS: f64 = 120.0;
const API_ORIGIN_FALLBACK: &str = "https://api.riseonly.net";
const GROUP_RADIUS: f32 = 24.0;

fn page_id(bot_id: &str, name: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("bot-settings-{bot_id}-{name}")))
}

fn bot_of(bot_id: &str, cx: &mut App) -> Option<BotModel> {
    BotServicesStore::bot(bot_id, cx)
}

fn load_bot(bot_id: &str, cx: &mut App) {
    BotInteractionsStore::shared(cx).update(cx, |store, cx| store.load_bot(bot_id, cx));
}

fn api_origin(cx: &mut App) -> String {
    let base_url = SaiFetchHttpConfiguration::shared(cx).read(cx).base_url.clone();
    let Some(mut components) = base_url.and_then(|url| UrlComponents::from_url(&url, false)) else {
        return API_ORIGIN_FALLBACK.to_owned();
    };
    components.set_path("");
    components.set_query(None);
    components.set_fragment(None);
    components
        .string()
        .map(|origin| origin.trim_matches('/').to_owned())
        .unwrap_or_else(|| API_ORIGIN_FALLBACK.to_owned())
}

fn canonical_endpoint(cx: &mut App) -> String {
    format!("{}/api/bot/v1/{{method}}", api_origin(cx))
}

fn copy_sensitive_text(value: String, cx: &App) {
    UIPasteboard::general().set_items(
        vec![UIPasteboardItem::PlainText(value)],
        UIPasteboardOptions {
            local_only: true,
            expiration_date: Some(Date::from_time_interval_since_now(SENSITIVE_COPY_LIFETIME_SECONDS)),
        },
        cx,
    );
}

/// bot settings view, the stack keeps it under commands and webhook and mini apps
/// unsaved form and revealed token must outlive those pushes
pub struct BotSettings {
    bot_id: String,
    name: String,
    tag: String,
    description: String,
    can_join_groups: bool,
    privacy_mode: bool,
    did_hydrate: bool,
    avatar_file: Option<SaiWsOptimisticLocalFile>,
    avatar_preview: Option<CleverImage>,
    banner_file: Option<SaiWsOptimisticLocalFile>,
    banner_preview: Option<CleverImage>,
    revealed_token: Option<String>,
    token_revealed: bool,
    token_copied: bool,
    save_request_id: Option<Uuid>,
    tag_availability: BotTagAvailabilityState,
    tag_check_generation: i64,
    _services_subscription: Subscription,
}

impl BotSettings {
    pub fn new(bot_id: String, cx: &mut Context<Self>) -> Self {
        let services = BotServicesStore::shared(cx);
        Self {
            bot_id,
            name: String::new(),
            tag: String::new(),
            description: String::new(),
            can_join_groups: true,
            privacy_mode: true,
            did_hydrate: false,
            avatar_file: None,
            avatar_preview: None,
            banner_file: None,
            banner_preview: None,
            revealed_token: None,
            token_revealed: false,
            token_copied: false,
            save_request_id: None,
            tag_availability: BotTagAvailabilityState::Idle,
            tag_check_generation: 0,
            _services_subscription: cx.observe(&services, |page, _, cx| {
                let bot = bot_of(&page.bot_id, cx);
                page.hydrate_if_needed(bot.as_ref());
                cx.notify();
            }),
        }
    }

    fn tag_unchanged(&self, bot: Option<&BotModel>) -> bool {
        bot.is_some_and(|bot| bot.tag == self.tag)
    }

    fn form_valid(&self, bot: Option<&BotModel>) -> bool {
        BotFormValidation::is_valid_bot_name(&self.name)
            && BotFieldRules::description().is_satisfied(&self.description)
            && (self.tag_unchanged(bot)
                || (BotTagRules::is_valid(&self.tag)
                    && matches!(self.tag_availability, BotTagAvailabilityState::Available)))
    }

    fn hydrate_if_needed(&mut self, bot: Option<&BotModel>) {
        let Some(bot) = bot else {
            return;
        };
        if self.did_hydrate {
            return;
        }
        self.did_hydrate = true;
        self.name = bot.name.clone();
        self.tag = bot.tag.clone();
        self.description = bot.description.clone();
        self.can_join_groups = bot.can_join_groups;
        self.privacy_mode = bot.privacy_mode;
    }

    fn set_tag(&mut self, raw: &str, cx: &mut Context<Self>) {
        let sanitized = BotTagRules::sanitize(raw, false, true);
        if sanitized == self.tag {
            return;
        }
        self.tag = sanitized;
        self.schedule_tag_check(cx);
        cx.notify();
    }

    fn schedule_tag_check(&mut self, cx: &mut Context<Self>) {
        self.tag_check_generation += 1;
        let generation = self.tag_check_generation;
        SaiDebouncer::cancel(TAG_DEBOUNCE_ID, TAG_DEBOUNCE_GROUP, cx);

        let bot = bot_of(&self.bot_id, cx);
        if self.tag.is_empty() || self.tag_unchanged(bot.as_ref()) {
            self.tag_availability = BotTagAvailabilityState::Idle;
            return;
        }
        if let Some(issue) = BotTagRules::validation_issue(&self.tag) {
            self.tag_availability = BotTagAvailabilityState::Invalid(issue);
            return;
        }

        self.tag_availability = BotTagAvailabilityState::Checking;
        let candidate = self.tag.clone();
        let page = cx.weak_entity();
        SaiDebouncer::debounced_action_id(
            TAG_DEBOUNCE_ID,
            Box::new(move |cx: &mut App| {
                let is_current = page
                    .read_with(cx, |page, _| page.tag_check_generation == generation && page.tag == candidate)
                    .unwrap_or(false);
                if !is_current {
                    return;
                }
                let result_page = page.clone();
                BotInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.check_tag_availability(
                        &candidate,
                        Some(Rc::new(move |checked_tag: String, result: BotTagAvailability, cx: &mut App| {
                            let _ = result_page.update(cx, |page, cx| {
                                if generation != page.tag_check_generation || checked_tag != page.tag {
                                    return;
                                }
                                page.tag_availability = if result.available {
                                    BotTagAvailabilityState::Available
                                } else {
                                    BotTagAvailabilityState::Unavailable { suggestions: result.suggestions.clone() }
                                };
                                cx.notify();
                            });
                        })),
                        cx,
                    );
                });
                let timeout_page = page.clone();
                let timeout_candidate = candidate.clone();
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(TAG_CHECK_TIMEOUT).await;
                    let _ = timeout_page.update(cx, |page, cx| {
                        if generation != page.tag_check_generation
                            || page.tag != timeout_candidate
                            || !matches!(page.tag_availability, BotTagAvailabilityState::Checking)
                        {
                            return;
                        }
                        page.tag_availability = BotTagAvailabilityState::Failed;
                        cx.notify();
                    });
                })
                .detach();
            }),
            SaiDebouncerOptions {
                delay_milliseconds: RiseTimingCanon::debounce("bot_tag_availability").client_debounce_ms,
                group_key: TAG_DEBOUNCE_GROUP.to_owned(),
                ..Default::default()
            },
            cx,
        );
    }

    fn save(&mut self, nav_store: Option<Entity<NavigationStore>>, cx: &mut Context<Self>) {
        let bot = bot_of(&self.bot_id, cx);
        if !self.form_valid(bot.as_ref()) || BotActionsStore::shared(cx).read(cx).update_bot_sai.is_pending() {
            return;
        }
        let Some(bot) = bot else {
            return;
        };

        let name = trimming_whitespaces_and_newlines(&self.name).to_string();
        let description = trimming_whitespaces_and_newlines(&self.description).to_string();
        if name == bot.name
            && description == bot.description
            && self.can_join_groups == bot.can_join_groups
            && self.privacy_mode == bot.privacy_mode
            && self.tag == bot.tag
        {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
            return;
        }
        let values = BotUpdateValues {
            name: Some(name),
            description: Some(description),
            can_join_groups: Some(self.can_join_groups),
            privacy_mode: Some(self.privacy_mode),
            tag: if self.tag == bot.tag { None } else { Some(self.tag.clone()) },
            ..Default::default()
        };

        let request_id = Uuid::new_v4();
        self.save_request_id = Some(request_id);
        cx.notify();
        let bot_id = self.bot_id.clone();
        let page = cx.weak_entity();
        BotInteractionsStore::shared(cx).update(cx, |store, cx| {
            let result_bot_id = bot_id.clone();
            store.update_bot(
                &bot_id,
                values,
                Some(Rc::new(move |updated_bot: BotModel, cx: &mut App| {
                    let is_current =
                        page.read_with(cx, |page, _| page.save_request_id == Some(request_id)).unwrap_or(false);
                    let Some(nav_store) = &nav_store else {
                        return;
                    };
                    let route = RootRoute::BotSettings { bot_id: result_bot_id.clone() };
                    if !is_current
                        || updated_bot.id != result_bot_id
                        || nav_store.read(cx).path.last() != Some(&route)
                    {
                        return;
                    }
                    let _ = page.update(cx, |page, cx| {
                        page.save_request_id = None;
                        cx.notify();
                    });
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                })),
                cx,
            );
        });
    }

    fn set_avatar_file(&mut self, value: Option<SaiWsOptimisticLocalFile>, cx: &mut Context<Self>) {
        let url_changed = self.avatar_file.as_ref().map(|file| &file.url) != value.as_ref().map(|file| &file.url);
        self.avatar_file = value;
        if url_changed {
            self.upload_avatar_if_needed(cx);
        }
        cx.notify();
    }

    fn set_banner_file(&mut self, value: Option<SaiWsOptimisticLocalFile>, cx: &mut Context<Self>) {
        let url_changed = self.banner_file.as_ref().map(|file| &file.url) != value.as_ref().map(|file| &file.url);
        self.banner_file = value;
        if url_changed {
            self.upload_banner_if_needed(cx);
        }
        cx.notify();
    }

    fn upload_avatar_if_needed(&mut self, cx: &mut Context<Self>) {
        let Some(avatar_file) = self.avatar_file.clone() else {
            return;
        };
        let bot_id = self.bot_id.clone();
        let page = cx.weak_entity();
        BotInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.update_bot_avatar(
                &bot_id,
                avatar_file,
                Some(Rc::new(move |_: String, cx: &mut App| {
                    let _ = page.update(cx, |page, cx| {
                        page.avatar_file = None;
                        page.avatar_preview = None;
                        cx.notify();
                    });
                })),
                cx,
            );
        });
    }

    fn upload_banner_if_needed(&mut self, cx: &mut Context<Self>) {
        let Some(banner_file) = self.banner_file.clone() else {
            return;
        };
        let bot_id = self.bot_id.clone();
        let page = cx.weak_entity();
        BotInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.update_bot_banner(
                &bot_id,
                banner_file,
                Some(Rc::new(move |_: String, cx: &mut App| {
                    let _ = page.update(cx, |page, cx| {
                        page.banner_file = None;
                        page.banner_preview = None;
                        cx.notify();
                    });
                })),
                cx,
            );
        });
    }

    fn apply_token(&mut self, result: BotTokenResult, cx: &mut Context<Self>) {
        self.revealed_token = Some(result.token);
        self.token_revealed = true;
        self.token_copied = false;
        cx.notify();
    }

    fn reveal_token(&mut self, cx: &mut Context<Self>) {
        let bot_id = self.bot_id.clone();
        let page = cx.weak_entity();
        BotInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.reveal_token(
                &bot_id,
                Some(Rc::new(move |result: BotTokenResult, cx: &mut App| {
                    let _ = page.update(cx, |page, cx| page.apply_token(result, cx));
                })),
                cx,
            );
        });
    }

    fn rotate_token(&mut self, cx: &mut Context<Self>) {
        let bot_id = self.bot_id.clone();
        let page = cx.weak_entity();
        BotInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.rotate_token(
                &bot_id,
                Some(Rc::new(move |result: BotTokenResult, cx: &mut App| {
                    let _ = page.update(cx, |page, cx| page.apply_token(result, cx));
                })),
                cx,
            );
        });
    }

    fn toggle_token_visibility(&mut self, cx: &mut Context<Self>) {
        if self.revealed_token.is_some() {
            self.token_revealed = !self.token_revealed;
            cx.notify();
        } else {
            self.reveal_token(cx);
        }
    }

    fn copy_token(&mut self, cx: &mut Context<Self>) {
        let Some(revealed_token) = self.revealed_token.clone().filter(|_| self.token_revealed) else {
            self.toggle_token_visibility(cx);
            self.token_copied = false;
            cx.notify();
            return;
        };
        copy_sensitive_text(revealed_token, cx);
        self.token_copied = true;
        cx.notify();
    }

    fn copy_compatibility_endpoint(&mut self, cx: &mut Context<Self>) {
        let Some(revealed_token) = self.revealed_token.clone().filter(|_| self.token_revealed) else {
            self.toggle_token_visibility(cx);
            self.token_copied = false;
            cx.notify();
            return;
        };
        copy_sensitive_text(format!("{}/bot{revealed_token}/{{method}}", api_origin(cx)), cx);
        self.token_copied = true;
        cx.notify();
    }

    fn resolved_token_text(&self) -> String {
        let Some(token) = &self.revealed_token else {
            return tr("bot_token_hidden");
        };
        if self.token_revealed {
            return token.clone();
        }
        "\u{2022}".repeat(token.chars().count().clamp(12, 32))
    }

    fn tag_status_icon(&self, bot: Option<&BotModel>, theme: &AppTheme) -> AnyElement {
        let green = UIColor::system_green().resolved_color(theme.appearance);
        let check = || div().children(IconUi::sized("checkmark.circle.fill", px(17.0), green, true)).into_any_element();
        if self.tag_unchanged(bot) {
            return check();
        }
        match self.tag_availability {
            BotTagAvailabilityState::Checking => progress_view().control_size(ControlSize::Small).into_any_element(),
            BotTagAvailabilityState::Available => check(),
            BotTagAvailabilityState::Invalid(_)
            | BotTagAvailabilityState::Unavailable { .. }
            | BotTagAvailabilityState::Failed => div()
                .children(IconUi::sized("exclamationmark.circle.fill", px(17.0), theme.semantic.error_100, true))
                .into_any_element(),
            BotTagAvailabilityState::Idle => div().into_any_element(),
        }
    }

    fn tag_status_text(&self, bot: Option<&BotModel>) -> Option<String> {
        if self.tag_unchanged(bot) {
            return None;
        }
        match &self.tag_availability {
            BotTagAvailabilityState::Idle => None,
            BotTagAvailabilityState::Checking => Some(tr("bot_tag_checking")),
            BotTagAvailabilityState::Available => Some(tr("bot_tag_available")),
            BotTagAvailabilityState::Unavailable { suggestions } => match suggestions.first() {
                Some(suggestion) if !suggestion.is_empty() => {
                    Some(format!("{} @{suggestion}", tr("bot_tag_unavailable")))
                }
                _ => Some(tr("bot_tag_unavailable")),
            },
            BotTagAvailabilityState::Failed => Some(tr("bot_tag_check_failed")),
            BotTagAvailabilityState::Invalid(issue) => Some(tr(&issue.message_key)),
        }
    }

    fn tag_status_color(&self, theme: &AppTheme) -> gpui::Hsla {
        match self.tag_availability {
            BotTagAvailabilityState::Available => UIColor::system_green().resolved_color(theme.appearance),
            BotTagAvailabilityState::Invalid(_)
            | BotTagAvailabilityState::Unavailable { .. }
            | BotTagAvailabilityState::Failed => theme.semantic.error_100,
            _ => theme.text.secondary,
        }
    }
}

fn mutation_error(cx: &mut App) -> Option<String> {
    let actions = BotActionsStore::shared(cx);
    let actions = actions.read(cx);
    actions.update_bot_sai.error().or_else(|| actions.reveal_bot_token_sai.error()).or_else(|| actions.rotate_bot_token_sai.error())
}

fn profile_header(this: &Entity<BotSettings>, bot: &BotModel, theme: &AppTheme, cx: &mut App) -> impl IntoElement + use<> {
    let bot_id = bot.id.clone();
    let services = BotServicesStore::shared(cx);
    let banner_progress = services
        .update(cx, |services, _| services.media_upload_progress(&bot_id, BotMediaKind::Banner))
        .unwrap_or(0.0);
    let banner_uploading = services.update(cx, |services, _| services.is_media_uploading(&bot_id, BotMediaKind::Banner));
    let avatar_progress = services
        .update(cx, |services, _| services.media_upload_progress(&bot_id, BotMediaKind::Avatar))
        .unwrap_or(0.0);
    let avatar_uploading = services.update(cx, |services, _| services.is_media_uploading(&bot_id, BotMediaKind::Avatar));

    div()
        .w_full()
        .px(px(12.0))
        .flex()
        .flex_col()
        .child(
            BotBannerPicker::new(
                page_id(&bot_id, "banner"),
                Binding::entity(this.clone(), |page: &BotSettings| page.banner_file.clone(), |page, value, cx| {
                    page.set_banner_file(value, cx);
                }),
                Binding::entity(this.clone(), |page: &BotSettings| page.banner_preview.clone(), |page, value, cx| {
                    page.banner_preview = value;
                    cx.notify();
                }),
                bot.banner_url.clone(),
            )
            .upload_progress(banner_progress)
            .is_uploading(banner_uploading),
        )
        .child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(7.0))
                .child(
                    div()
                        .relative()
                        .mt(px(-34.0))
                        .size(px(92.0))
                        .child(
                            SelectMediaUi::new(
                                page_id(&bot_id, "avatar"),
                                Binding::entity(
                                    this.clone(),
                                    |page: &BotSettings| page.avatar_file.clone(),
                                    |page, value, cx| page.set_avatar_file(value, cx),
                                ),
                                Binding::entity(
                                    this.clone(),
                                    |page: &BotSettings| page.avatar_preview.clone(),
                                    |page, value, cx| {
                                        page.avatar_preview = value;
                                        cx.notify();
                                    },
                                ),
                            )
                            .size(92.0)
                            .placeholder_system_image("command")
                            .cover_upload_progress(avatar_progress * 100.0)
                            .show_upload_overlay(avatar_uploading)
                            .picker_configuration(MediaPickerConfiguration::profile_avatar())
                            .remote_source(bot.avatar_url.clone())
                            .remote_name(bot.name.clone())
                            .remote_user_id(bot.bot_user_id.clone())
                            .uses_app_sheet_presentation(true),
                        )
                        .child(
                            div()
                                .absolute()
                                .right(px(-1.5))
                                .bottom(px(-1.5))
                                .size(px(32.0))
                                .rounded_full()
                                .bg(theme.bg._100)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    div()
                                        .size(px(26.0))
                                        .rounded_full()
                                        .bg(theme.primary._100)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .children(IconUi::sized("camera.fill", px(12.0), theme.content_on(theme.primary._100), true)),
                                ),
                        ),
                )
                .child(
                    MainText::new(bot.name.clone()).px(20.0).tac(TextAlignT::Center).font_weight(FontWeight::BOLD),
                )
                .child(SecondaryText::new(format!("@{}", bot.tag)).px(13.0).tac(TextAlignT::Center)),
        )
}

fn identity_fields(this: &Entity<BotSettings>, trailing: AnyElement, bot_id: &str) -> impl IntoElement + use<> {
    GroupedBtns::new(
        page_id(bot_id, "identity"),
        vec![
            GroupedBtn::new("name", "").content_slot(BotSettingsFieldRow::new(
                page_id(bot_id, "field-name"),
                "person.fill",
                SettingsIconPalette::profile(),
                tr("bot_name_placeholder"),
                Binding::entity(this.clone(), |page: &BotSettings| page.name.clone(), |page, value: String, cx| {
                    page.name = BotFieldRules::name().clamped(&value);
                    cx.notify();
                }),
            )),
            GroupedBtn::new("tag", "").content_slot(
                BotSettingsFieldRow::new(
                    page_id(bot_id, "field-tag"),
                    "at",
                    SettingsIconPalette::customization(),
                    tr("bot_tag_placeholder"),
                    Binding::entity(this.clone(), |page: &BotSettings| page.tag.clone(), |page, value: String, cx| {
                        page.set_tag(&value, cx);
                    }),
                )
                .trailing(trailing),
            ),
            GroupedBtn::new("description", "").content_slot(
                BotSettingsFieldRow::new(
                    page_id(bot_id, "field-description"),
                    "text.alignleft",
                    SettingsIconPalette::privacy_description(),
                    tr("bot_description_placeholder"),
                    Binding::entity(
                        this.clone(),
                        |page: &BotSettings| page.description.clone(),
                        |page, value: String, cx| {
                            page.description = BotFieldRules::description().clamped(&value);
                            cx.notify();
                        },
                    ),
                )
                .axis(Axis::Vertical),
            ),
        ],
    )
    .title(tr("bot_profile_section"))
    .corner_radius(GROUP_RADIUS)
}

fn behavior_settings(this: &Entity<BotSettings>, bot_id: &str) -> impl IntoElement + use<> {
    GroupedBtns::new(
        page_id(bot_id, "behavior"),
        vec![
            GroupedBtn::new("groups", "").content_slot(BotToggleRow::new(
                page_id(bot_id, "toggle-groups"),
                "person.3.fill",
                color("#5856D6"),
                tr("bot_can_join_groups"),
                tr("bot_can_join_groups_hint"),
                Binding::entity(this.clone(), |page: &BotSettings| page.can_join_groups, |page, value, cx| {
                    page.can_join_groups = value;
                    cx.notify();
                }),
            )),
            GroupedBtn::new("privacy", "").content_slot(BotToggleRow::new(
                page_id(bot_id, "toggle-privacy"),
                "lock.shield.fill",
                color("#34C759"),
                tr("bot_privacy_mode"),
                tr("bot_privacy_mode_hint"),
                Binding::entity(this.clone(), |page: &BotSettings| page.privacy_mode, |page, value, cx| {
                    page.privacy_mode = value;
                    cx.notify();
                }),
            )),
        ],
    )
    .title(tr("bot_behavior_section"))
    .corner_radius(GROUP_RADIUS)
}

fn developer_tools(bot_id: &str, is_system: bool, nav_store: Option<Entity<NavigationStore>>) -> impl IntoElement + use<> {
    let push = move |route: RootRoute, cx: &mut App| {
        if let Some(nav_store) = &nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    };
    let push = Rc::new(push);
    let commands_push = push.clone();
    let webhook_push = push.clone();
    let commands_bot_id = bot_id.to_owned();
    let webhook_bot_id = bot_id.to_owned();
    let mini_apps_bot_id = bot_id.to_owned();
    let items = vec![
            GroupedBtn::new("commands", tr("bot_commands_title"))
                .icon("terminal.fill")
                .icon_background(color("#5856D6"))
                .guide_anchor(RiseGuideAnchor::BotSettingsCommands)
                .callback(move |_: &mut Window, cx: &mut App| {
                    commands_push(RootRoute::BotCommands { bot_id: commands_bot_id.clone() }, cx);
                }),
            GroupedBtn::new("webhook", tr("bot_webhook_title"))
                .icon("network")
                .icon_background(color("#FF9500"))
                .guide_anchor(RiseGuideAnchor::BotSettingsWebhook)
                .callback(move |_: &mut Window, cx: &mut App| {
                    webhook_push(RootRoute::BotWebhook { bot_id: webhook_bot_id.clone() }, cx);
                }),
            GroupedBtn::new("miniapps", tr("bot_mini_apps_title"))
                .icon("square.grid.2x2.fill")
                .icon_background(color("#AF52DE"))
                .guide_anchor(RiseGuideAnchor::BotSettingsMiniApps)
                .callback(move |_: &mut Window, cx: &mut App| {
                    push(RootRoute::BotMiniApps { bot_id: mini_apps_bot_id.clone() }, cx);
                }),
        ];
    let items = if is_system { items.into_iter().filter(|item| item.id.as_ref() == "commands").collect() } else { items };
    GroupedBtns::new(page_id(bot_id, "developer"), items)
    .title(tr("bot_developer_tools"))
    .corner_radius(GROUP_RADIUS)
}

fn token_section(this: &Entity<BotSettings>, bot_id: &str, theme: &AppTheme, cx: &mut App) -> impl IntoElement + use<> {
    let (reveal_pending, rotate_pending) = {
        let actions = BotActionsStore::shared(cx);
        let actions = actions.read(cx);
        (actions.reveal_bot_token_sai.is_pending(), actions.rotate_bot_token_sai.is_pending())
    };
    let page = this.read(cx);
    let token_text = page.resolved_token_text();
    let has_token = page.revealed_token.is_some();
    let token_revealed = page.token_revealed;
    let eye_symbol = if token_revealed { "eye.slash.fill" } else { "eye.fill" };
    let token_copied = page.token_copied;
    let toggle_page = this.clone();
    let copy_page = this.clone();
    let visibility_page = this.clone();
    let rotate_page = this.clone();

    let content = div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .p(px(16.0))
        .child(
            div()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .children(IconUi::sized("key.fill", px(17.0), theme.primary._100, true))
                .child(
                    div()
                        .min_w_0()
                        .swiftui_font(Font::system(13.0, FontWeight::MEDIUM, FontDesign::Monospaced))
                        .text_color(theme.text.primary)
                        .line_clamp(2)
                        .child(token_text),
                )
                .child(div().flex_1().min_w(px(8.0)))
                .when(has_token, |row| {
                    row.child(
                        plain(page_id(bot_id, "token-eye"))
                            .on_press(move |_: &mut Window, cx: &mut App| {
                                toggle_page.update(cx, |page, cx| {
                                    page.token_revealed = !page.token_revealed;
                                    cx.notify();
                                });
                            })
                            .children(IconUi::sized(
                                eye_symbol,
                                px(17.0),
                                theme.primary._100,
                                bot_symbol_is_solid(eye_symbol),
                            )),
                    )
                    .child(
                        plain(page_id(bot_id, "token-copy"))
                            .on_press(move |_: &mut Window, cx: &mut App| {
                                copy_page.update(cx, |page, cx| page.copy_token(cx));
                            })
                            .children(IconUi::sized(
                                if token_copied { "checkmark" } else { "doc.on.doc" },
                                px(17.0),
                                theme.primary._100,
                                false,
                            )),
                    )
                }),
        )
        .child(
            div()
                .w_full()
                .flex()
                .flex_row()
                .gap(px(10.0))
                .child(
                    div().flex_1().child(
                        ButtonUi::new(page_id(bot_id, "token-reveal"))
                            .disabled(reveal_pending)
                            .is_pending(reveal_pending)
                            .height(42.0)
                            .on_press(move |_: &mut Window, cx: &mut App| {
                                visibility_page.update(cx, |page, cx| page.toggle_token_visibility(cx));
                            })
                            .child(
                                MainText::new(tr(if has_token && token_revealed {
                                    "bot_hide_token"
                                } else {
                                    "bot_reveal_token"
                                }))
                                .px(13.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(gpui::white()),
                            ),
                    ),
                )
                .child(
                    div().flex_1().child(
                        ButtonUi::new(page_id(bot_id, "token-rotate"))
                            .disabled(rotate_pending)
                            .background_color(theme.semantic.error_100)
                            .height(42.0)
                            .on_press(move |_: &mut Window, cx: &mut App| {
                                let rotate_page = rotate_page.clone();
                                NativeConfirmAlert::present_delete(
                                    &tr("bot_rotate_token_title"),
                                    &tr("bot_rotate_token_message"),
                                    Some(tr("bot_rotate_token")),
                                    None,
                                    false,
                                    None,
                                    None,
                                    move |cx: &mut App| {
                                        rotate_page.update(cx, |page, cx| page.rotate_token(cx));
                                    },
                                    cx,
                                );
                            })
                            .child(
                                MainText::new(tr("bot_rotate_token"))
                                    .px(13.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(gpui::white()),
                            ),
                    ),
                ),
        );

    GroupedBtns::new(page_id(bot_id, "token"), vec![GroupedBtn::new("token", "").content_slot(content)])
        .title(tr("bot_api_token"))
        .end_group_title(tr("bot_token_security_hint"))
        .corner_radius(GROUP_RADIUS)
}

fn api_section(this: &Entity<BotSettings>, bot_id: &str, cx: &mut App) -> impl IntoElement + use<> {
    let endpoint = canonical_endpoint(cx);
    let compatibility_page = this.clone();
    let copied_endpoint = endpoint.clone();
    GroupedBtns::new(
        page_id(bot_id, "api"),
        vec![
            GroupedBtn::new("endpoint", tr("bot_api_endpoint"))
                .icon("link")
                .icon_background(color("#007AFF"))
                .subtitle(endpoint)
                .show_chevron(false)
                .callback(move |_: &mut Window, cx: &mut App| {
                    UIPasteboard::general().set_string(&copied_endpoint, cx);
                }),
            GroupedBtn::new("compatibility", tr("bot_api_compatibility"))
                .icon("curlybraces")
                .icon_background(color("#34C759"))
                .subtitle(tr("bot_api_compatibility_hint"))
                .show_chevron(false)
                .callback(move |_: &mut Window, cx: &mut App| {
                    compatibility_page.update(cx, |page, cx| page.copy_compatibility_endpoint(cx));
                }),
            GroupedBtn::new("help", tr("bot_api_help"))
                .icon("questionmark.circle.fill")
                .icon_background(color("#8E8E93"))
                .subtitle(tr("bot_api_help_hint"))
                .show_chevron(false),
        ],
    )
    .title(tr("bot_api_section"))
    .corner_radius(GROUP_RADIUS)
}

fn destructive_section(bot_id: &str, nav_store: Option<Entity<NavigationStore>>, theme: &AppTheme) -> impl IntoElement + use<> {
    let delete_bot_id = bot_id.to_owned();
    GroupedBtns::new(
        page_id(bot_id, "destructive"),
        vec![
            GroupedBtn::new("delete", tr("bot_delete"))
                .icon("trash.fill")
                .icon_background(UIColor::system_red().resolved_color(theme.appearance))
                .destructive(true)
                .show_chevron(false)
                .callback(move |_: &mut Window, cx: &mut App| {
                    let delete_bot_id = delete_bot_id.clone();
                    let nav_store = nav_store.clone();
                    NativeConfirmAlert::present_delete(
                        &tr("bot_delete_title"),
                        &tr("bot_delete_message"),
                        Some(tr("bot_delete")),
                        None,
                        false,
                        None,
                        None,
                        move |cx: &mut App| {
                            BotInteractionsStore::shared(cx).update(cx, |store, cx| {
                                store.delete_bot(
                                    &delete_bot_id,
                                    Some(Rc::new(move |cx: &mut App| {
                                        if let Some(nav_store) = &nav_store {
                                            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                                        }
                                    })),
                                    cx,
                                );
                            });
                        },
                        cx,
                    );
                }),
        ],
    )
    .corner_radius(GROUP_RADIUS)
}

fn settings_content(
    this: &Entity<BotSettings>,
    bot_id: &str,
    nav_store: Option<Entity<NavigationStore>>,
    cx: &mut App,
) -> impl IntoElement + use<> {
    let theme = rise_ui::theme(cx).clone();
    let bot = bot_of(bot_id, cx);
    let error = mutation_error(cx);
    let is_update_pending = BotActionsStore::shared(cx).read(cx).update_bot_sai.is_pending();
    let refresh_bot_id = bot_id.to_owned();
    let page = this.read(cx);
    let tag_trailing = page.tag_status_icon(bot.as_ref(), &theme);
    let tag_status_text = page.tag_status_text(bot.as_ref());
    let tag_status_color = page.tag_status_color(&theme);

    let sections = bot.as_ref().map(|bot| {
        let mut sections = vec![
            env_scope::<IsEnabledKey>(!is_update_pending).child(profile_header(this, bot, &theme, cx)).into_any_element(),
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(env_scope::<IsEnabledKey>(!is_update_pending).child(identity_fields(this, tag_trailing, bot_id)))
                .when_some(tag_status_text, |fields, text| {
                    fields.child(div().w_full().px(px(18.0)).child(MainText::new(text).px(12.0).color(tag_status_color)))
                })
                .into_any_element(),
            env_scope::<IsEnabledKey>(!is_update_pending).child(behavior_settings(this, bot_id)).into_any_element(),
            developer_tools(bot_id, bot.is_system, nav_store.clone()).into_any_element(),
        ];
        if !bot.is_system {
            sections.push(token_section(this, bot_id, &theme, cx).into_any_element());
            sections.push(api_section(this, bot_id, cx).into_any_element());
            sections.push(destructive_section(bot_id, nav_store.clone(), &theme).into_any_element());
        }
        sections
    });

    scroll_view(page_id(bot_id, "scroll"))
        .size_full()
        .list_shell_scroll_under_chrome(cx)
        .refreshable(move |_, cx| {
            load_bot(&refresh_bot_id, cx);
            Task::ready(())
        })
        .bg(theme.bg._100)
        .child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(20.0))
                .pt(px(12.0))
                .when_some(error.filter(|error| !error.is_empty()), |content, error| {
                    content.child(
                        div().w_full().px(px(18.0)).child(MainText::new(error).px(13.0).color(theme.semantic.error_100)),
                    )
                })
                .children(sections.into_iter().flatten())
                .child(div().h(px(90.0))),
        )
}

impl Render for BotSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        let bot_id = self.bot_id.clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let bot = bot_of(&bot_id, cx);
        self.hydrate_if_needed(bot.as_ref());
        let form_valid = self.form_valid(bot.as_ref());
        let (status, get_bot_error, is_update_pending) = {
            let actions = BotActionsStore::shared(cx);
            let actions = actions.read(cx);
            let status = if bot.is_none() { actions.get_bot_sai.status() } else { SaiWsSliceStatus::Fulfilled };
            (status, actions.get_bot_sai.error(), actions.update_bot_sai.is_pending())
        };

        let task_page = this.downgrade();
        let task_bot_id = bot_id.clone();
        use_task(page_id(&bot_id, "task"), (), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| {
                BotInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_settings_errors(cx));
                let bot = bot_of(&task_bot_id, cx);
                let _ = task_page.update(cx, |page, cx| {
                    page.hydrate_if_needed(bot.as_ref());
                    cx.notify();
                });
                load_bot(&task_bot_id, cx);
            });
        });

        let back_page = this.clone();
        let back_store = nav_store.clone();
        let right_jsx = if is_update_pending {
            PageHeaderToolbarSlot::new(LoaderUi::new().size(LoaderUiSize::Small)).into_any_element()
        } else {
            let save_page = this.clone();
            let save_store = nav_store.clone();
            div()
                .id(page_id(&bot_id, "save-wrap"))
                .tooltip_label(tr("common_done"))
                .opacity(if form_valid { 1.0 } else { 0.42 })
                .child(
                    PageHeaderIconButton::new(
                        page_id(&bot_id, "save"),
                        "checkmark",
                        move |_: &mut Window, cx: &mut App| {
                            let save_store = save_store.clone();
                            save_page.update(cx, |page, cx| page.save(save_store, cx));
                        },
                    )
                    .disabled(!form_valid),
                )
                .into_any_element()
        };

        let content_page = this.clone();
        let content_bot_id = bot_id.clone();
        let content_store = nav_store.clone();
        let retry_bot_id = bot_id.clone();
        let error_id = page_id(&bot_id, "error");
        let error_message = get_bot_error.clone();
        let keys_bot_id = bot_id.clone();

        let shell = SettingsWrapperUi::new(tr("bot_settings_title"), move |_, cx| {
            back_page.update(cx, |page, cx| {
                page.save_request_id = None;
                cx.notify();
            });
            if let Some(nav_store) = &back_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .right_jsx(right_jsx)
        .content(
            SaiDataRender::new(status, move |_, cx| {
                settings_content(&content_page, &content_bot_id, content_store, cx)
            })
            .id(page_id(&bot_id, "render"))
            .is_empty(false)
            .error_text(get_bot_error.or_else(|| Some(tr("bot_load_error"))))
            .sai_data_render_error(move |_, _| {
                let retry_bot_id = retry_bot_id.clone();
                BotStateView::new(
                    error_id.clone(),
                    "exclamationmark.arrow.triangle.2.circlepath",
                    tr("bot_load_error"),
                )
                .message(error_message.clone())
                .action_title(tr("bot_retry"))
                .action(move |_: &mut Window, cx: &mut App| load_bot(&retry_bot_id, cx))
            }),
        );

        let disappear_page = this.downgrade();
        PageRefreshKeys::new(
            page_id(&bot_id, "keys"),
            move |_: &mut Window, cx: &mut App| load_bot(&keys_bot_id, cx),
            shell,
        )
        .on_disappear(page_id(&bot_id, "disappear"), move |_: Option<&mut Window>, cx: &mut App| {
            let _ = disappear_page.update(cx, |page, cx| {
                page.save_request_id = None;
                page.tag_check_generation += 1;
                page.revealed_token = None;
                page.token_revealed = false;
                page.token_copied = false;
                cx.notify();
            });
            SaiDebouncer::cancel(TAG_DEBOUNCE_ID, TAG_DEBOUNCE_GROUP, cx);
            BotInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.clear_sensitive_results(cx);
                store.clear_settings_errors(cx);
            });
        })
    }
}
