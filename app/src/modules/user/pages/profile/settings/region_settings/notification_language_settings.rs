use gpui::{App, Context, Entity, SharedString, Subscription, Window};
use rise_i18n::LanguageStore;
use rise_i18n::{AppLanguageCatalog, tr};
use rise_ui::flag_icon_ui::FlagIconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::main_text::TextAlignT;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, ListShellUi};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::user::engine::rise_user_locale_presentation::RiseUserLocalePresentation;
use crate::modules::user::stores::user_locale::user_locale_interactions::user_locale_interactions::UserLocaleInteractionsStore;
use crate::modules::user::stores::user_locale::user_locale_services::user_locale_services::UserLocaleServicesStore;

const SELECTED_MARK: &str = "\u{2713}";

/// search query and services store, changes re-render the page
struct NotificationLanguageSettingsState {
    query: String,
    _subscriptions: Vec<Subscription>,
}

impl NotificationLanguageSettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseUserLocalePresentation::shared(cx);
        Self {
            query: String::new(),
            _subscriptions: vec![cx.observe(&presentation, |_, _, cx| cx.notify())],
        }
    }
}

/// language of server rendered notification text
/// separate from the interface language, server renders it per account
/// follows the ui language unless pinned
#[derive(IntoElement)]
pub struct NotificationLanguageSettings;

impl NotificationLanguageSettings {
    pub fn new() -> Self {
        Self
    }

    fn select_notification_language(language: Option<String>, cx: &mut App) {
        UserLocaleInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.select_notification_language(language, cx));
    }

    fn follow_group(follows_app: bool, cx: &mut App) -> GroupedBtns {
        let app_language_code = LanguageStore::shared(cx).code().to_owned();
        let mut follow = GroupedBtn::new("follow", tr("settings_notification_language_follow"))
            .icon("arrow.triangle.2.circlepath")
            .icon_background(SettingsIconPalette::language())
            .accessibility_selected(follows_app)
            .show_chevron(false)
            .callback(|_, cx| Self::select_notification_language(None, cx));
        if let Some(language) = AppLanguageCatalog::language(&app_language_code) {
            follow = follow.subtitle(language.native_name);
        }
        if follows_app {
            follow = follow.trailing(SELECTED_MARK);
        }
        GroupedBtns::new("notification-language-follow", vec![follow])
    }

    fn rows(query: &str, follows_app: bool, selected: &str, cx: &mut App) -> Vec<GroupedBtn> {
        let services = UserLocaleServicesStore::shared(cx);
        let rows = services.update(cx, |services, cx| services.languages(query, cx));
        rows.into_iter()
            .map(|language| {
                let code = language.code;
                let is_selected = !follows_app && code == selected;
                let mut row = GroupedBtn::new(code, language.native_name)
                    .leading_slot(FlagIconUi::new(language.flag()))
                    .accessibility_selected(is_selected)
                    .show_chevron(false)
                    .callback(move |_, cx| Self::select_notification_language(Some(code.to_string()), cx));
                if language.english_name != language.native_name {
                    row = row.subtitle(language.english_name);
                }
                if is_selected {
                    row = row.trailing(SELECTED_MARK);
                }
                row
            })
            .collect()
    }
}

impl RenderOnce for NotificationLanguageSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state: Entity<NotificationLanguageSettingsState> =
            window.use_keyed_state("notification-language-settings-state", cx, |_, cx| {
                NotificationLanguageSettingsState::new(cx)
            });
        let nav_store = env::<NavigationStoreKey>(cx);
        let settings =
            UserLocaleServicesStore::shared(cx).update(cx, |services, cx| services.settings(cx));
        let follows_app = settings.notification_language_follows_app;
        let query = state.read(cx).query.clone();
        let rows = Self::rows(&query, follows_app, &settings.notification_language, cx);
        let follow_group = Self::follow_group(follows_app, cx);

        let content = vstack(14.0)
            .w_full()
            .pt(px(12.0))
            .child(
                div().w_full().pl(px(16.0)).pr(px(16.0)).child(
                    SecondaryText::new(tr("settings_notification_language_hint"))
                        .px(13.0)
                        .tac(TextAlignT::Center),
                ),
            )
            .child(follow_group)
            .when(rows.is_empty(), |stack| {
                stack.child(
                    div()
                        .w_full()
                        .pt(px(40.0))
                        .child(ChatProfileEmptyState::new("globe", tr("settings_language_empty_title"))),
                )
            })
            .when(!rows.is_empty(), |stack| {
                stack.child(
                    GroupedBtns::new("notification-language-rows", rows)
                        .title(tr("settings_notification_language_pin")),
                )
            })
            .child(div().flex_none().h(px(80.0)));

        ListShellUi::new()
            .title(SharedString::from(tr("settings_notification_language_title")))
            .on_back(move |_, cx| {
                if let Some(nav_store) = &nav_store {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            })
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()))
            .below_header(
                div().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING)).child(
                    SearchFieldUi::new(
                        "notification-language-search",
                        Binding::entity(
                            state,
                            |state: &NotificationLanguageSettingsState| state.query.clone(),
                            |state: &mut NotificationLanguageSettingsState, query, cx| {
                                state.query = query;
                                cx.notify();
                            },
                        ),
                    )
                    .placeholder(SharedString::from(tr("settings_language_search_placeholder")))
                    .uses_inline_text_field(true),
                ),
            )
            .content(
                scroll_view("notification-language-scroll")
                    .size_full()
                    .bg(theme.bg._100)
                    .list_shell_scroll_under_chrome(cx)
                    .child(content),
            )
            .on_appear("notification-language-appear", |_, cx| {
                UserLocaleInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.open_language_settings(cx));
            })
    }
}
