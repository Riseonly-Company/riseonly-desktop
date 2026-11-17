use gpui::{AnyView, App, Context, Entity, Hsla, StyleRefinement, Subscription, Window, rgb};
use rise_canon::{RiseGuideAnchor, RisePrivacyRule, rise_key};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::user::pages::profile::settings::privacy_settings::abuse_appeal_sheet::AbuseAppealSheetPresentation;
use crate::modules::user::shared::contact_device_book::ContactDeviceAuthorization;
use crate::modules::user::shared::privacy_mock::{PrivacyRuleCatalog, PrivacySetting, PrivacySettingsWireDTORules};
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;
use crate::modules::user::stores::user::user_actions::user_wire_types::PrivacySettingsWireDTO;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

struct PrivacySettingsState {
    shows_abuse_appeal: Entity<AbuseAppealSheetPresentation>,
    _observations: [Subscription; 4],
}

impl PrivacySettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let user_store = UserServicesStore::shared(cx);
        let auth_store = AuthServicesStore::shared(cx);
        let actions = UserActionsStore::shared(cx);
        let pending_requests = SaiWsPendingRegistry::shared(cx);
        Self {
            shows_abuse_appeal: cx.new(AbuseAppealSheetPresentation::new),
            _observations: [
                cx.observe(&user_store, |_, _, cx| cx.notify()),
                cx.observe(&auth_store, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&pending_requests, |_, _, cx| cx.notify()),
            ],
        }
    }
}

struct PrivacySettingsContactSync {
    settings_did_load: bool,
    enabled: bool,
    authorization: ContactDeviceAuthorization,
    in_progress: bool,
    failed: bool,
}

impl PrivacySettingsContactSync {
    fn read(cx: &mut App) -> Self {
        UserServicesStore::shared(cx).update(cx, |store, cx| Self {
            settings_did_load: store.contact_sync_settings_did_load,
            enabled: store.contact_sync_enabled(cx),
            authorization: store.contact_authorization,
            in_progress: store.contact_sync_in_progress,
            failed: [&store.contact_sync_settings_error, &store.contact_sync_error]
                .into_iter()
                .any(|error| !error.as_deref().unwrap_or_default().is_empty()),
        })
    }

    fn status(&self) -> String {
        if self.failed {
            return tr("contact_sync_status_error");
        }
        if !self.settings_did_load {
            return tr("contact_sync_status_loading");
        }
        match self.authorization {
            ContactDeviceAuthorization::Denied => tr("contact_sync_status_denied"),
            ContactDeviceAuthorization::Restricted => tr("contact_sync_status_restricted"),
            ContactDeviceAuthorization::NotDetermined => {
                if self.enabled {
                    tr("contact_sync_status_permission_needed")
                } else {
                    tr("contact_sync_status_disabled")
                }
            }
            ContactDeviceAuthorization::Authorized | ContactDeviceAuthorization::Limited => {
                if !self.enabled {
                    tr("contact_sync_status_disabled")
                } else if self.in_progress {
                    tr("contact_sync_status_syncing")
                } else {
                    tr("contact_sync_status_enabled")
                }
            }
        }
    }

    fn rows(&self) -> Vec<GroupedBtn> {
        let settings_did_load = self.settings_did_load;
        let enabled = self.enabled;
        let switch_value = settings_did_load && enabled;
        let switch = SwitchUi::new(
            "contact-sync-enabled-switch",
            Binding::new(move |_: &App| switch_value, move |value, _, cx| {
                if settings_did_load {
                    Self::set_enabled(value, cx);
                }
            }),
        );
        let mut sync_row = GroupedBtn::new("contact_sync_enabled", tr("contact_sync_settings_title"))
            .icon("person.crop.circle.badge.checkmark")
            .icon_background(SettingsIconPalette::privacy_friends())
            .subtitle(self.status())
            .subtitle_line_limit(2)
            .trailing_slot(switch)
            .callback(move |_, cx| Self::set_enabled(!enabled, cx));
        if !settings_did_load {
            sync_row = sync_row.disabled(true);
        }
        let mut items = vec![sync_row];
        if matches!(self.authorization, ContactDeviceAuthorization::Denied | ContactDeviceAuthorization::Restricted) {
            items.push(
                GroupedBtn::new("contact_sync_open_settings", tr("contact_sync_open_settings"))
                    .icon("gear")
                    .icon_background(Hsla::from(rgb(0x8E8E93)))
                    .callback(|_, cx| {
                        UserInteractionsStore::shared(cx).update(cx, |store, cx| store.open_contact_system_settings(cx));
                    }),
            );
        }
        if self.failed {
            items.push(
                GroupedBtn::new("contact_sync_retry", tr("contact_sync_retry"))
                    .icon("arrow.clockwise")
                    .icon_background(Hsla::from(rgb(0xF19A1A)))
                    .callback(|_, cx| {
                        UserInteractionsStore::shared(cx).update(cx, |store, cx| store.retry_contact_sync_settings(cx));
                    }),
            );
        }
        items
    }

    fn set_enabled(enabled: bool, cx: &mut App) {
        UserInteractionsStore::shared(cx).update(cx, |store, cx| store.set_contact_sync_enabled(enabled, cx));
    }
}

#[derive(IntoElement, Default)]
pub struct PrivacySettings;

impl PrivacySettings {
    pub fn new() -> Self {
        Self
    }

    fn privacy_data(cx: &mut App) -> Option<PrivacySettingsWireDTO> {
        let optimistic = UserServicesStore::shared(cx).read(cx).optimistic_privacy().cloned();
        optimistic.or_else(|| UserActionsStore::shared(cx).read(cx).get_privacy_sai.data())
    }

    fn message_requests_enabled(cx: &mut App) -> bool {
        Self::privacy_data(cx).is_some_and(|privacy| privacy.message_requests_enabled)
    }
}

impl RenderOnce for PrivacySettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("privacy-settings-state", cx, |_, cx| PrivacySettingsState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let presentation = state.read(cx).shows_abuse_appeal.clone();

        let message_requests_enabled = Self::message_requests_enabled(cx);
        on_change("privacy-settings-message-requests", message_requests_enabled, true, window, cx, |_, _, _, cx| {
            UserServicesStore::shared(cx).update(cx, |store, cx| store.report_message_requests_fact(cx));
        });

        let back_store = nav_store.clone();
        let shell = SettingsWrapperUi::new(tr("settings_privacy_title"), move |_, cx| {
            if let Some(nav_store) = &back_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(PrivacySettingsBody { nav_store, presentation: presentation.clone() })
        .on_appear("privacy-settings-appear", |_, cx| {
            UserInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.ensure_privacy_loaded(cx);
                store.refresh_my_profile(cx);
                store.bootstrap_contacts_for_authenticated_account(cx);
            });
        });

        let presentation = AnyView::from(presentation).cached(StyleRefinement::default().size_0());
        div().relative().size_full().child(shell).child(presentation)
    }
}

#[derive(IntoElement)]
struct PrivacySettingsBody {
    nav_store: Option<Entity<NavigationStore>>,
    presentation: Entity<AbuseAppealSheetPresentation>,
}

impl PrivacySettingsBody {
    fn push(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn phone_public_rows(cx: &mut App) -> Option<Vec<GroupedBtn>> {
        let auth = AuthServicesStore::shared(cx);
        let profile = auth.read(cx).profile()?;
        let phone = profile.more.phone.as_deref()?;
        if trimming_whitespaces_and_newlines(phone).is_empty() {
            return None;
        }
        let is_public = profile.is_phone_public;
        let subtitle = if is_public {
            tr("privacy_phone_public_subtitle_on")
        } else {
            tr("privacy_phone_public_subtitle_off")
        };
        Some(vec![
            GroupedBtn::new(rise_key::user::IS_PHONE_PUBLIC, tr("privacy_phone_public_title"))
                .icon("phone.fill")
                .icon_background(SettingsIconPalette::privacy_phone())
                .subtitle(subtitle)
                .subtitle_line_limit(0)
                .trailing_slot(SwitchUi::new(
                    "privacy-phone-public-switch",
                    Binding::new(move |_: &App| is_public, |value, _, cx| Self::set_phone_public(value, cx)),
                ))
                .callback(move |_, cx| Self::set_phone_public(!is_public, cx)),
        ])
    }

    fn set_phone_public(is_public: bool, cx: &mut App) {
        UserInteractionsStore::shared(cx).update(cx, |store, cx| store.set_phone_public(is_public, cx));
    }

    fn rule_rows(&self, privacy_data: Option<&PrivacySettingsWireDTO>) -> Vec<GroupedBtn> {
        PrivacyRuleCatalog::settings()
            .iter()
            .map(|setting| {
                let setting: PrivacySetting = match privacy_data {
                    Some(privacy) => PrivacySetting { audience: privacy.audience(setting.rule), ..*setting },
                    None => *setting,
                };
                let nav_store = self.nav_store.clone();
                let id = setting.id();
                GroupedBtn::new(id, tr(setting.title_key()))
                    .icon(PrivacyIconCatalog::symbol(setting.rule))
                    .icon_background(PrivacyIconCatalog::color(setting.rule))
                    .trailing(tr(setting.audience.i18n_key()))
                    .callback(move |_, cx| {
                        Self::push(&nav_store, RootRoute::PrivacySetting { r#type: Some(id.to_owned()) }, cx);
                    })
            })
            .collect()
    }

    fn message_requests_row(enabled: bool) -> GroupedBtn {
        GroupedBtn::new(rise_key::user::MESSAGE_REQUESTS_ENABLED, tr("privacy_message_requests_title"))
            .icon("envelope.badge")
            .icon_background(SettingsIconPalette::privacy_messages())
            .subtitle(tr("privacy_message_requests_subtitle"))
            .guide_anchor(RiseGuideAnchor::PrivacyMessageRequests)
            .trailing_slot(SwitchUi::new(
                "privacy-message-requests-switch",
                Binding::new(move |_: &App| enabled, |value, _, cx| Self::set_message_requests(value, cx)),
            ))
            .callback(move |_, cx| Self::set_message_requests(!enabled, cx))
    }

    fn set_message_requests(enabled: bool, cx: &mut App) {
        UserInteractionsStore::shared(cx).update(cx, |store, cx| store.update_message_requests_enabled(enabled, cx));
    }
}

impl RenderOnce for PrivacySettingsBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._100;
        let privacy_data = PrivacySettings::privacy_data(cx);
        let message_requests_enabled = privacy_data.as_ref().is_some_and(|privacy| privacy.message_requests_enabled);
        let phone_rows = Self::phone_public_rows(cx);
        let contact_sync = PrivacySettingsContactSync::read(cx);

        let blocked_store = self.nav_store.clone();
        let presentation = self.presentation.clone();
        scroll_view("privacy-settings-scroll").size_full().bg(background).list_shell_scroll_under_chrome(cx).child(
            vstack(20.0)
                .w_full()
                .pt(px(12.0))
                .child(GroupedBtns::new(
                    "privacy-settings-blocked",
                    vec![
                        GroupedBtn::new("blocked_users", tr("privacy_blocked_users_title"))
                            .icon("person.crop.circle.badge.xmark")
                            .icon_background(SettingsIconPalette::blocks())
                            .callback(move |_, cx| Self::push(&blocked_store, RootRoute::BlockedUsers, cx)),
                    ],
                ))
                .when_some(phone_rows, |stack, rows| {
                    stack.child(GroupedBtns::new("privacy-settings-phone", rows).title(tr("privacy_phone_group").to_uppercase()))
                })
                .child(
                    GroupedBtns::new("privacy-settings-rules", self.rule_rows(privacy_data.as_ref()))
                        .title(tr("settings_profile_title").to_uppercase()),
                )
                .child(
                    GroupedBtns::new("privacy-settings-messages", vec![Self::message_requests_row(message_requests_enabled)])
                        .title(tr("privacy_messages_group").to_uppercase()),
                )
                .child(
                    GroupedBtns::new("privacy-settings-contact-sync", contact_sync.rows())
                        .title(tr("contact_sync_settings_group").to_uppercase()),
                )
                .child(
                    GroupedBtns::new(
                        "privacy-settings-abuse-appeal",
                        vec![
                            GroupedBtn::new("abuse_appeal", tr("abuse_appeal_settings_row"))
                                .icon("checkmark.shield")
                                .icon_background(SettingsIconPalette::privacy_messages())
                                .callback(move |window, cx| {
                                    presentation.update(cx, |presentation, cx| presentation.present(window, cx));
                                }),
                        ],
                    )
                    .title(tr("abuse_appeal_title").to_uppercase()),
                )
                .child(div().flex_none().h(px(80.0))),
        )
    }
}

enum PrivacyIconCatalog {}

impl PrivacyIconCatalog {
    fn symbol(rule: RisePrivacyRule) -> &'static str {
        match rule {
            RisePrivacyRule::Phone => "phone.fill",
            RisePrivacyRule::ProfilePhoto => "person.crop.square.fill",
            RisePrivacyRule::Description => "text.alignleft",
            RisePrivacyRule::Hb => "gift.fill",
            RisePrivacyRule::Friend => "person.2.fill",
            RisePrivacyRule::Subs => "person.crop.circle.badge.checkmark",
            RisePrivacyRule::Plan => "calendar",
            RisePrivacyRule::Goal => "target",
            RisePrivacyRule::Comments => "text.bubble.fill",
            RisePrivacyRule::Reposts => "arrow.2.squarepath",
            RisePrivacyRule::LastSeen => "clock.fill",
            RisePrivacyRule::OnlineStatus => "bubble.left.fill",
            RisePrivacyRule::Achievements => "trophy.fill",
            RisePrivacyRule::Stories => "play.rectangle.on.rectangle.fill",
            RisePrivacyRule::Messages => "envelope.fill",
            RisePrivacyRule::Coauthor => "person.2.fill",
            RisePrivacyRule::Tag => "person.crop.square",
            RisePrivacyRule::Sparks => "sparkle",
            RisePrivacyRule::Gifts => "gift.fill",
            RisePrivacyRule::GiftSend => "shippingbox.fill",
            RisePrivacyRule::Calls | RisePrivacyRule::Forwards | RisePrivacyRule::Groups => "lock.fill",
        }
    }

    fn color(rule: RisePrivacyRule) -> Hsla {
        match rule {
            RisePrivacyRule::Phone => SettingsIconPalette::privacy_phone(),
            RisePrivacyRule::ProfilePhoto => SettingsIconPalette::privacy_avatar(),
            RisePrivacyRule::Description => SettingsIconPalette::privacy_description(),
            RisePrivacyRule::Hb => SettingsIconPalette::privacy_birthday(),
            RisePrivacyRule::Friend => SettingsIconPalette::privacy_friends(),
            RisePrivacyRule::Subs => SettingsIconPalette::privacy_friends(),
            RisePrivacyRule::Plan => SettingsIconPalette::privacy_plans(),
            RisePrivacyRule::Goal => SettingsIconPalette::privacy_goals(),
            RisePrivacyRule::Comments => SettingsIconPalette::privacy_messages(),
            RisePrivacyRule::Reposts => SettingsIconPalette::privacy_messages(),
            RisePrivacyRule::LastSeen => SettingsIconPalette::privacy_last_seen(),
            RisePrivacyRule::OnlineStatus => SettingsIconPalette::privacy_online_status(),
            RisePrivacyRule::Achievements => SettingsIconPalette::privacy_achievements(),
            RisePrivacyRule::Stories => SettingsIconPalette::privacy_avatar(),
            RisePrivacyRule::Messages => SettingsIconPalette::privacy_messages(),
            RisePrivacyRule::Coauthor => SettingsIconPalette::privacy_friends(),
            RisePrivacyRule::Tag => SettingsIconPalette::privacy_avatar(),
            RisePrivacyRule::Sparks => SettingsIconPalette::privacy_sparks(),
            RisePrivacyRule::Gifts | RisePrivacyRule::GiftSend => SettingsIconPalette::privacy_gifts(),
            RisePrivacyRule::Calls | RisePrivacyRule::Forwards | RisePrivacyRule::Groups => SettingsIconPalette::privacy(),
        }
    }
}
