use std::collections::HashSet;

use gpui::{App, Context, Entity, FontWeight, SharedString, Subscription, Window};
use rise_i18n::tr;
use rise_theme::color;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::riseonly_app_logo_view::RiseonlyAppLogoView;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::prelude::*;
use rise_ui::IconUi;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::push_notifications::push_deep_link_store::PushDeepLinkStore;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::diagnostics::rise_log_file::RiseLogFile;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, ListShellUi};
use crate::modules::bot::components::bot_ui_components::BotStateView;
use crate::modules::notify::engine::core::rise_release_notes_seen_store::RiseReleaseNotesDeviceContext;
use crate::modules::notify::stores::notify_faq::notify_faq_store::{RiseFAQStore, RiseFAQStoreItem};
use crate::modules::user::shared::rise_legal_links::{RiseLegalLinks, RiseLegalLinksDocument};
use crate::modules::user::stores::user::user_actions::user_wire_types::UserGetProfileByTagGatewayDTOExt;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

type NavStore = Option<Entity<NavigationStore>>;

fn pop(nav_store: &NavStore, cx: &mut App) {
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
    }
}

fn support_email_row() -> GroupedBtn {
    GroupedBtn::new("support_email", tr("settings_legal_support_email"))
        .icon("envelope.fill")
        .icon_background(color("#007AFF"))
        .subtitle(RiseLegalLinks::SUPPORT_EMAIL)
        .callback(|_, cx| RiseLegalLinks::open_support_email(cx))
}

struct AboutProjectState {
    founder_avatar: String,
    _subscriptions: Vec<Subscription>,
}

impl AboutProjectState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = UserServicesStore::shared(cx);
        let subscription = cx.observe(&services, |state, _, cx| {
            let avatar = AboutProject::founder_avatar(cx);
            if avatar != state.founder_avatar {
                state.founder_avatar = avatar;
                cx.notify();
            }
        });
        Self { founder_avatar: AboutProject::founder_avatar(cx), _subscriptions: vec![subscription] }
    }
}

/// about the project page in settings
/// support questions and mail, team as on the landing and legal docs
#[derive(IntoElement)]
pub struct AboutProject;

impl AboutProject {
    pub const FOUNDER_TAG: &'static str = "nics51";
    pub const FOUNDER_NAME: &'static str = "Ayanov Dulat";
    pub const FOUNDER_PHOTO: &'static str = "https://riseonly.net/team-ayanov-dulat.jpg";

    const PRODUCT_NAME: &'static str = "Riseonly";

    pub fn new() -> Self {
        Self
    }

    /// founders avatar from his profile here not a copy, the site photo stands in until the profile loads
    fn founder_avatar(cx: &mut App) -> String {
        let live = UserServicesStore::shared(cx)
            .update(cx, |services, cx| services.cached_foreign_profile_tag(Self::FOUNDER_TAG, cx))
            .and_then(|response| response.as_profile_model())
            .and_then(|profile| profile.avatar_url)
            .unwrap_or_default();
        if live.is_empty() { Self::FOUNDER_PHOTO.to_owned() } else { live }
    }

    pub fn open_founder(cx: &mut App) {
        let Some(link) = (RiseonlyPublicLink::Profile { tag: Self::FOUNDER_TAG.to_owned() }).deep_link() else {
            return;
        };
        PushDeepLinkStore::shared(cx).update(cx, |store, cx| {
            let _ = store.enqueue(link, cx);
        });
    }

    fn header() -> Div {
        vstack(10.0)
            .w_full()
            .items_center()
            .pt(px(8.0))
            .child(RiseonlyAppLogoView::new().size(84.0))
            .child(MainText::new(Self::PRODUCT_NAME).px(24.0).tac(TextAlignT::Center).font_weight(FontWeight::BOLD))
            .child(
                div()
                    .w_full()
                    .px(px(32.0))
                    .child(SecondaryText::new(tr("about_project_tagline")).px(15.0).tac(TextAlignT::Center)),
            )
    }

    fn support_group(nav_store: &NavStore) -> GroupedBtns {
        let faq_nav_store = nav_store.clone();
        GroupedBtns::new(
            "about-project-support",
            vec![
                GroupedBtn::new("faq", tr("about_project_faq"))
                    .icon("questionmark.circle.fill")
                    .icon_background(color("#FF9500"))
                    .callback(move |_, cx| {
                        if let Some(nav_store) = &faq_nav_store {
                            nav_store.update(cx, |nav_store, cx| {
                                nav_store.push(RootRoute::AboutProjectFAQ, NavigationStoreSettlingPush::Drop, cx)
                            });
                        }
                    }),
                support_email_row(),
            ],
        )
        .title(tr("about_project_support").to_uppercase())
        .uses_lazy_rows(false)
    }

    fn founder_row(avatar: &str) -> GroupedBtn {
        GroupedBtn::new("founder", Self::FOUNDER_NAME)
            .leading_slot(
                UserLogoUi::new("about-project-founder-logo")
                    .source(Some(SharedString::from(avatar.to_owned())))
                    .name(Some(SharedString::from(Self::FOUNDER_NAME)))
                    .size(44.0)
                    .show_story_ring(false),
            )
            .subtitle(format!("{} \u{b7} @{}", tr("about_project_founder_role"), Self::FOUNDER_TAG))
            .callback(|_, cx| Self::open_founder(cx))
    }

    fn role_row(key: &'static str, avatar: &str) -> GroupedBtn {
        GroupedBtn::new(key, tr(key))
            .show_chevron(false)
            .trailing_slot(
                hstack(6.0)
                    .items_center()
                    .child(
                        UserLogoUi::new(SharedString::from(format!("about-project-role-logo-{key}")))
                            .source(Some(SharedString::from(avatar.to_owned())))
                            .name(Some(SharedString::from(Self::FOUNDER_NAME)))
                            .size(22.0)
                            .show_story_ring(false),
                    )
                    .child(SecondaryText::new(format!("@{}", Self::FOUNDER_TAG)).px(14.0)),
            )
            .callback(|_, cx| Self::open_founder(cx))
    }

    fn legal_group() -> GroupedBtns {
        GroupedBtns::new(
            "about-project-legal",
            vec![
                GroupedBtn::new("legal_terms", tr("settings_legal_terms"))
                    .icon("doc.text.fill")
                    .icon_background(SettingsIconPalette::legal())
                    .callback(|_, cx| RiseLegalLinks::open(RiseLegalLinksDocument::TermsOfService, cx)),
                GroupedBtn::new("legal_privacy", tr("settings_legal_privacy"))
                    .icon("hand.raised.square.fill")
                    .icon_background(SettingsIconPalette::legal())
                    .callback(|_, cx| RiseLegalLinks::open(RiseLegalLinksDocument::PrivacyPolicy, cx)),
                GroupedBtn::new("legal_child_safety", tr("settings_legal_child_safety"))
                    .icon("figure.and.child.holdinghands")
                    .icon_background(SettingsIconPalette::legal())
                    .callback(|_, cx| RiseLegalLinks::open(RiseLegalLinksDocument::ChildSafety, cx)),
            ],
        )
        .title(tr("settings_legal_title").to_uppercase())
        .uses_lazy_rows(false)
    }
}

impl RenderOnce for AboutProject {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state("about-project-state", cx, |_, cx| AboutProjectState::new(cx));
        let avatar = state.read(cx).founder_avatar.clone();

        let content = vstack(22.0)
            .w_full()
            .pt(px(12.0))
            .child(Self::header())
            .child(Self::support_group(&nav_store))
            .child(
                GroupedBtns::new("about-project-team", vec![Self::founder_row(&avatar)])
                    .title(tr("about_project_team").to_uppercase())
                    .uses_lazy_rows(false),
            )
            .child(
                GroupedBtns::new("about-project-roles", vec![Self::role_row("about_project_all_roles", &avatar)])
                    .title(tr("about_project_team_roles").to_uppercase())
                    .uses_lazy_rows(false),
            )
            .child(Self::legal_group())
            .child(
                div()
                    .w_full()
                    .child(
                        SecondaryText::new(format!("{} {}", tr("settings_version"), RiseReleaseNotesDeviceContext::app_version()))
                            .px(13.0)
                            .tac(TextAlignT::Center),
                    )
                    .when_some(RiseLogFile::current_path(), |footer, path| {
                        footer.child(SecondaryText::new(path.display().to_string()).px(13.0).mt(4.0).tac(TextAlignT::Center))
                    }),
            )
            .child(div().flex_none().h(px(60.0)));

        let back_nav_store = nav_store.clone();
        ListShellUi::new()
            .title(SharedString::from(tr("settings_about_project")))
            .on_back(move |_, cx| pop(&back_nav_store, cx))
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()))
            .content(
                scroll_view("about-project-scroll")
                    .size_full()
                    .bg(theme.bg._100)
                    .list_shell_scroll_under_chrome(cx)
                    .child(content),
            )
            .on_appear("about-project-appear", |_, cx| {
                UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.load_foreign_profile_tag(AboutProject::FOUNDER_TAG.to_owned(), cx)
                });
            })
    }
}

/// open state and store of the faq, changes re-render the page
struct AboutProjectFAQState {
    open: HashSet<i64>,
    _subscriptions: Vec<Subscription>,
}

impl AboutProjectFAQState {
    fn new(cx: &mut Context<Self>) -> Self {
        let store = RiseFAQStore::shared(cx);
        Self { open: HashSet::new(), _subscriptions: vec![cx.observe(&store, |_, _, cx| cx.notify())] }
    }
}

/// questions people ask, each opens its answer in place. come from the server in the app language
#[derive(IntoElement)]
pub struct AboutProjectFAQ;

impl AboutProjectFAQ {
    pub fn new() -> Self {
        Self
    }

    fn row(state: &Entity<AboutProjectFAQState>, item: &RiseFAQStoreItem, is_open: bool, cx: &App) -> GroupedBtn {
        let theme = rise_ui::theme(cx).clone();
        let id = item.id;
        let toggle = state.clone();
        let mut row = GroupedBtn::new(SharedString::from(format!("faq_{id}")), item.question.clone())
            .show_chevron(false)
            .trailing_slot(
                div()
                    .flex_none()
                    .children(IconUi::sized("chevron.down", px(13.0), theme.text.secondary, false))
                    .rotation_effect(if is_open { 180.0 } else { 0.0 }),
            )
            .callback(move |_, cx| {
                toggle.update(cx, |state, cx| {
                    if !state.open.remove(&id) {
                        state.open.insert(id);
                    }
                    cx.notify();
                });
            });
        if is_open {
            row = row.expanded_slot(
                div()
                    .w_full()
                    .px(px(16.0))
                    .pb(px(12.0))
                    .child(SecondaryText::new(item.answer.clone()).px(15.0)),
            );
        }
        row
    }
}

impl RenderOnce for AboutProjectFAQ {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state("about-project-faq-state", cx, |_, cx| AboutProjectFAQState::new(cx));
        let store = RiseFAQStore::shared(cx);
        let (items, failed) = {
            let store = store.read(cx);
            (store.items().to_vec(), store.failed())
        };
        let open = state.read(cx).open.clone();

        let body = if !items.is_empty() {
            let rows = items.iter().map(|item| Self::row(&state, item, open.contains(&item.id), cx)).collect();
            GroupedBtns::new("about-project-faq-rows", rows).uses_lazy_rows(false).into_any_element()
        } else if failed {
            div()
                .w_full()
                .pt(px(40.0))
                .child(
                    BotStateView::new("about-project-faq-error", "wifi.exclamationmark", tr("about_project_faq_load_error"))
                        .action_title(Some(tr("common_retry")))
                        .action(|_, cx| RiseFAQStore::shared(cx).update(cx, |store, cx| store.load(cx))),
                )
                .into_any_element()
        } else {
            div().w_full().flex().justify_center().pt(px(60.0)).child(progress_view()).into_any_element()
        };

        let content = vstack(22.0)
            .w_full()
            .pt(px(12.0))
            .child(body)
            .child(GroupedBtns::new("about-project-faq-support", vec![support_email_row()]).uses_lazy_rows(false))
            .child(div().flex_none().h(px(60.0)));

        let back_nav_store = nav_store.clone();
        ListShellUi::new()
            .title(SharedString::from(tr("about_project_faq")))
            .on_back(move |_, cx| pop(&back_nav_store, cx))
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()))
            .content(
                scroll_view("about-project-faq-scroll")
                    .size_full()
                    .bg(theme.bg._100)
                    .list_shell_scroll_under_chrome(cx)
                    .child(content),
            )
            .on_appear("about-project-faq-appear", |_, cx| {
                RiseFAQStore::shared(cx).update(cx, |store, cx| store.load(cx));
            })
    }
}
