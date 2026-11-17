use gpui::{App, Context, Entity, FontWeight, Hsla, Subscription, Window, rgb};
use rise_canon::{RisePrivacyAudience, RisePrivacyRule, rise_key};
use rise_i18n::tr;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_privacy::RisePrivacyAudienceCanon;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::user::shared::privacy_mock::{PrivacyRuleCatalog, PrivacySetting, PrivacySettingsWireDTORules};
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;
use crate::modules::user::stores::user::user_actions::user_types::PrivacyExceptionAction;
use crate::modules::user::stores::user::user_actions::user_wire_types::PrivacySettingsWireDTO;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const SELECTED_MARK: &str = "\u{2713}";

struct PrivacySettingScreenState {
    _observations: [Subscription; 3],
}

impl PrivacySettingScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let user_store = UserServicesStore::shared(cx);
        let actions = UserActionsStore::shared(cx);
        let pending_requests = SaiWsPendingRegistry::shared(cx);
        Self {
            _observations: [
                cx.observe(&user_store, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&pending_requests, |_, _, cx| cx.notify()),
            ],
        }
    }
}

#[derive(IntoElement)]
pub struct PrivacySettingScreen {
    r#type: Option<String>,
}

impl PrivacySettingScreen {
    pub fn new(r#type: Option<String>) -> Self {
        Self { r#type }
    }
}

impl RenderOnce for PrivacySettingScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let _state: Entity<PrivacySettingScreenState> =
            window.use_keyed_state("privacy-setting-screen-state", cx, |_, cx| PrivacySettingScreenState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let setting = PrivacySetting::template(self.r#type.as_deref());
        let title = setting.map(|setting| tr(setting.title_key())).unwrap_or_default();

        let back_store = nav_store.clone();
        SettingsWrapperUi::new(title, move |_, cx| {
            if let Some(nav_store) = &back_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(PrivacySettingScreenBody { setting, nav_store })
        .on_appear("privacy-setting-screen-appear", |_, cx| {
            UserInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_privacy_loaded(cx));
        })
    }
}

#[derive(IntoElement)]
struct PrivacySettingScreenBody {
    setting: Option<PrivacySetting>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl PrivacySettingScreenBody {
    fn privacy_data(cx: &mut App) -> Option<PrivacySettingsWireDTO> {
        let optimistic = UserServicesStore::shared(cx).read(cx).optimistic_privacy().cloned();
        optimistic.or_else(|| UserActionsStore::shared(cx).read(cx).get_privacy_sai.data())
    }

    fn rules_content(&self, privacy_data: &PrivacySettingsWireDTO, cx: &mut App) -> Vec<AnyElement> {
        let setting = self.setting;
        let audience = Self::audience(setting, privacy_data);
        let items: Vec<GroupedBtn> = PrivacyRuleCatalog::AUDIENCE_ORDER
            .into_iter()
            .map(|value| {
                let row = GroupedBtn::new(value.raw_value(), tr(value.i18n_key()))
                    .icon(Self::icon_for(value))
                    .icon_background(Self::bg_for(value))
                    .show_chevron(false)
                    .callback(move |_, cx| Self::apply_rule(setting, value, cx));
                if audience == value { row.trailing(SELECTED_MARK) } else { row }
            })
            .collect();
        let mut group = GroupedBtns::new("privacy-setting-audience", items);
        if let Some(group_title) = setting.and_then(|setting| setting.group_title_key()) {
            group = group.title(tr(group_title).to_uppercase());
        }

        let mut groups = vec![group.into_any_element()];
        if let Some(setting) = setting {
            groups.push(self.exceptions_group(setting.rule, cx).into_any_element());
            if setting.rule == RisePrivacyRule::Subs {
                groups.push(Self::subs_count_group(privacy_data.subs_count_hidden).into_any_element());
            }
        }
        groups
    }

    fn subs_count_group(subs_count_hidden: bool) -> GroupedBtns {
        let row = GroupedBtn::new(rise_key::user::SUBS_COUNT_HIDDEN, tr("privacy_hide_subs_count"))
            .icon("number")
            .icon_background(Hsla::from(rgb(0x5AC8FA)))
            .show_chevron(false)
            .callback(move |_, cx| {
                UserInteractionsStore::shared(cx).update(cx, |store, cx| store.update_subs_count_hidden(!subs_count_hidden, cx));
            });
        let row = if subs_count_hidden { row.trailing(SELECTED_MARK) } else { row };
        GroupedBtns::new("privacy-setting-subs-count", vec![row]).title(tr("privacy_hide_subs_count_group").to_uppercase())
    }

    fn exceptions_group(&self, rule: RisePrivacyRule, cx: &mut App) -> GroupedBtns {
        let privacy_type = rule.privacy_type();
        let user_store = UserServicesStore::shared(cx);
        let allow = user_store.update(cx, |store, cx| store.privacy_exceptions(privacy_type, PrivacyExceptionAction::Allow, cx).len());
        let deny = user_store.update(cx, |store, cx| store.privacy_exceptions(privacy_type, PrivacyExceptionAction::Deny, cx).len());
        let show_for = self.exceptions_row(rule, PrivacyExceptionAction::Allow, allow);
        let hide_from = self.exceptions_row(rule, PrivacyExceptionAction::Deny, deny);
        GroupedBtns::new("privacy-setting-exceptions", vec![show_for, hide_from]).title(tr("privacy_exceptions_group").to_uppercase())
    }

    fn exceptions_row(&self, rule: RisePrivacyRule, action: PrivacyExceptionAction, count: usize) -> GroupedBtn {
        let (id, icon, background, title_key) = match action {
            PrivacyExceptionAction::Allow => ("show_for", "eye.fill", 0x34C759, "privacy_show_for"),
            PrivacyExceptionAction::Deny => ("hide_from", "eye.slash.fill", 0xFF3B30, "privacy_hide_from"),
        };
        let nav_store = self.nav_store.clone();
        let row = GroupedBtn::new(id, tr(title_key)).icon(icon).icon_background(Hsla::from(rgb(background))).callback(move |_, cx| {
            if let Some(nav_store) = &nav_store {
                let route = RootRoute::PrivacyExceptions {
                    r#type: rule.privacy_type().to_owned(),
                    action: action.raw_value().to_owned(),
                };
                nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
            }
        });
        if count > 0 { row.trailing(RiseFormat::compact_count(count as i64)) } else { row }
    }

    fn error_state() -> impl IntoElement {
        vstack(12.0)
            .w_full()
            .pt(px(60.0))
            .child(MainText::new(tr("retry_later_error")).px(14.0).secondary(true))
            .child(
                plain("privacy-setting-retry")
                    .on_press(|_, cx| {
                        UserInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_privacy_loaded(cx));
                    })
                    .child(MainText::new(tr("privacy_settings_retry")).px(14.0).font_weight(FontWeight::SEMIBOLD).primary(true)),
            )
    }

    fn apply_rule(setting: Option<PrivacySetting>, value: RisePrivacyAudience, cx: &mut App) {
        let Some(setting) = setting else {
            return;
        };
        UserInteractionsStore::shared(cx).update(cx, |store, cx| store.update_privacy_rule(setting.rule, value, cx));
    }

    /// a key canon doesnt know is a broken route not visible to everyone, closed audience stays selected
    fn audience(setting: Option<PrivacySetting>, privacy_data: &PrivacySettingsWireDTO) -> RisePrivacyAudience {
        match setting {
            Some(setting) => privacy_data.audience(setting.rule),
            None => RisePrivacyAudience::unknown_value(),
        }
    }

    fn icon_for(value: RisePrivacyAudience) -> &'static str {
        match value {
            RisePrivacyAudience::All => "globe",
            RisePrivacyAudience::None => "lock.fill",
            RisePrivacyAudience::Contacts => "person.2",
            RisePrivacyAudience::Friends => "person.crop.circle",
            RisePrivacyAudience::CloseFriends => "star.fill",
        }
    }

    fn bg_for(value: RisePrivacyAudience) -> Hsla {
        let hex = match value {
            RisePrivacyAudience::All => 0x0083E0,
            RisePrivacyAudience::None => 0x8E8E93,
            RisePrivacyAudience::Contacts => 0x00B894,
            RisePrivacyAudience::Friends => 0x7E57F7,
            RisePrivacyAudience::CloseFriends => 0xFF9F0A,
        };
        Hsla::from(rgb(hex))
    }
}

impl RenderOnce for PrivacySettingScreenBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._100;
        let content: Vec<AnyElement> = match Self::privacy_data(cx) {
            Some(privacy_data) => self.rules_content(&privacy_data, cx),
            None => {
                let failed = UserServicesStore::shared(cx).update(cx, |store, cx| store.privacy_load_failed(cx));
                if failed {
                    vec![Self::error_state().into_any_element()]
                } else {
                    vec![div().w_full().flex().justify_center().pt(px(60.0)).child(progress_view()).into_any_element()]
                }
            }
        };

        scroll_view("privacy-setting-scroll").size_full().bg(background).list_shell_scroll_under_chrome(cx).child(
            vstack(20.0)
                .w_full()
                .pt(px(12.0))
                .children(content)
                .child(div().flex_none().h(px(80.0))),
        )
    }
}
