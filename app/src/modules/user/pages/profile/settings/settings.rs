use std::sync::Arc;

use gpui::{App, ClipboardItem, Context, Entity, FontFeatures, FontWeight, Hsla, SharedString, Subscription, Window};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_theme::{AppTheme, color};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::profile_field_formatting::PhoneNumberFormatting;
use crate::core::lib::rise_feature_gate::RiseFeatureGate;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::reaction_emoji_glyph::ReactionEmojiGlyph;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::Notifier;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_interactions::auth_interactions::AuthInteractionsStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::diagnostics::pages::bug_report::bug_report_sheet_host::BugReportSurfaceProbe;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_interactions::bug_report_interactions::BugReportInteractionsStore;
use crate::modules::gifts::engine::rise_gifts_engine_models::RiseGiftItemKind;
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::GiftsInteractionsStore;
use crate::modules::notify::engine::core::rise_release_notes_seen_store::RiseReleaseNotesDeviceContext;
use crate::modules::sparks::components::sparks_icon_view::{SparksAmountLabel, SparksSettingsTile};
use crate::modules::sparks::stores::sparks::sparks_services::sparks_services::SparksServicesStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::PackKind;
use crate::modules::user::stores::user::user_actions::user_types::ProfileModel;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

type NavStore = Option<Entity<NavigationStore>>;

/// phone revealed flag and the stores the rows read, changes re-render the page
struct SettingsState {
    phone_revealed: bool,
    open_section: Option<RootRoute>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsState {
    fn new(nav_store: &NavStore, cx: &mut Context<Self>) -> Self {
        let auth = AuthServicesStore::shared(cx);
        let sparks = SparksServicesStore::shared(cx);
        let mut subscriptions = vec![
            cx.observe(&auth, |_, _, cx| cx.notify()),
            cx.observe(&sparks, |_, _, cx| cx.notify()),
        ];
        let open_section = nav_store.as_ref().and_then(|nav_store| nav_store.read(cx).settings_open_section().cloned());
        if let Some(nav_store) = nav_store {
            subscriptions.push(cx.observe(nav_store, |state, nav_store, cx| {
                let open = nav_store.read(cx).settings_open_section().cloned();
                if open != state.open_section {
                    state.open_section = open;
                    cx.notify();
                }
            }));
        }
        Self { phone_revealed: false, open_section, _subscriptions: subscriptions }
    }
}

fn push_route(nav_store: &NavStore, route: RootRoute, cx: &mut App) {
    tracing::warn!(target: "riseonly::X1418", ?route, has_store = nav_store.is_some(), "settings row press");
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |nav_store, cx| nav_store.open_from_settings_list(route, cx));
    }
}

fn route_row(
    nav_store: &NavStore,
    open: Option<&RootRoute>,
    id: &'static str,
    icon: &'static str,
    icon_background: Hsla,
    text: String,
    route: RootRoute,
) -> GroupedBtn {
    let nav_store = nav_store.clone();
    GroupedBtn::new(id, text)
        .icon(icon)
        .icon_background(icon_background)
        .selected(open == Some(&route))
        .callback(move |_, cx| push_route(&nav_store, route.clone(), cx))
}

fn group(id: &'static str, title: String, items: Vec<GroupedBtn>) -> GroupedBtns {
    GroupedBtns::new(id, items).title(title.to_uppercase()).uses_lazy_rows(false)
}

#[derive(IntoElement)]
pub struct Settings;

impl Settings {
    const GROUP_SPACING: f32 = 20.0;

    pub fn new() -> Self {
        Self
    }

    pub fn first_section_route() -> RootRoute {
        RootRoute::ProfileSettings
    }

    fn profile_group(nav_store: &NavStore, open: Option<&RootRoute>) -> GroupedBtns {
        group(
            "settings-group-profile",
            tr("settings_section_profile"),
            vec![
                route_row(
                    nav_store,
                    open,
                    "profile",
                    "person.fill",
                    SettingsIconPalette::profile(),
                    tr("settings_profile_title"),
                    RootRoute::ProfileSettings,
                )
                .guide_anchor(RiseGuideAnchor::SettingsProfile),
                route_row(
                    nav_store,
                    open,
                    "settings_achievements_row",
                    "trophy",
                    SettingsIconPalette::achievements(),
                    tr("settings_achievements_title"),
                    RootRoute::Achievements { user_id: None },
                )
                .guide_anchor(RiseGuideAnchor::SettingsAchievements),
                route_row(
                    nav_store,
                    open,
                    "settings_stories_archive_row",
                    "archivebox.fill",
                    SettingsIconPalette::profile(),
                    tr("archive_title"),
                    RootRoute::StoriesArchive,
                ),
                route_row(
                    nav_store,
                    open,
                    "settings_close_friends_row",
                    "star.fill",
                    SettingsIconPalette::privacy_friends(),
                    tr("story_close_friends_title"),
                    RootRoute::CloseFriends,
                )
                .guide_anchor(RiseGuideAnchor::SettingsCloseFriends),
            ],
        )
    }

    fn security_group(nav_store: &NavStore, open: Option<&RootRoute>) -> GroupedBtns {
        group(
            "settings-group-security",
            tr("settings_section_security"),
            vec![
                route_row(
                    nav_store,
                    open,
                    "privacy",
                    "lock.fill",
                    SettingsIconPalette::privacy(),
                    tr("settings_privacy_title"),
                    RootRoute::PrivacySettings,
                )
                .guide_anchor(RiseGuideAnchor::SettingsPrivacy),
                route_row(
                    nav_store,
                    open,
                    "sessions",
                    "iphone",
                    SettingsIconPalette::sessions(),
                    tr("settings_sessions_title"),
                    RootRoute::SessionsSettings,
                )
                .guide_anchor(RiseGuideAnchor::SettingsSessions),
                route_row(
                    nav_store,
                    open,
                    "blocks",
                    "hand.raised.fill",
                    SettingsIconPalette::blocks(),
                    tr("blocks_page_title"),
                    RootRoute::MyBlocks,
                ),
                route_row(
                    nav_store,
                    open,
                    "view_history",
                    "clock.arrow.circlepath",
                    color("#5AC8FA"),
                    tr("view_history_page_title"),
                    RootRoute::PostViewHistory,
                ),
                route_row(
                    nav_store,
                    open,
                    "moderation",
                    "shield.lefthalf.filled",
                    SettingsIconPalette::moderation(),
                    tr("settings_moderations_title"),
                    RootRoute::ModerationSettings,
                ),
            ],
        )
    }

    fn chats_group(nav_store: &NavStore, open: Option<&RootRoute>, profile: &ProfileModel) -> GroupedBtns {
        let quick_reaction_nav_store = nav_store.clone();
        group(
            "settings-group-chats",
            tr("settings_section_chats"),
            vec![
                route_row(
                    nav_store,
                    open,
                    "notify",
                    "bell.fill",
                    SettingsIconPalette::notifications(),
                    tr("settings_notifications_title"),
                    RootRoute::NotificationSettings,
                )
                .guide_anchor(RiseGuideAnchor::SettingsNotifications),
                GroupedBtn::new(rise_canon::rise_key::user::QUICK_REACTION, tr("settings_quick_reaction_title"))
                    .icon("hand.thumbsup.fill")
                    .icon_background(SettingsIconPalette::quick_reaction())
                    .trailing_slot(ReactionEmojiGlyph::new(profile.quick_reaction.clone(), 23.0))
                    .show_chevron_when_trailing_slot(true)
                    .selected(open == Some(&RootRoute::QuickReactionSettings))
                    .callback(move |_, cx| push_route(&quick_reaction_nav_store, RootRoute::QuickReactionSettings, cx)),
                route_row(
                    nav_store,
                    open,
                    "stickers",
                    "face.smiling.inverse",
                    SettingsIconPalette::stickers(),
                    tr("settings_stickers_title"),
                    RootRoute::StickerSettings { kind: PackKind::Sticker },
                )
                .guide_anchor(RiseGuideAnchor::SettingsStickers),
                route_row(
                    nav_store,
                    open,
                    "emoji",
                    "face.smiling",
                    SettingsIconPalette::stickers(),
                    tr("settings_emoji_title"),
                    RootRoute::StickerSettings { kind: PackKind::Emoji },
                ),
                route_row(
                    nav_store,
                    open,
                    "chat_folders",
                    "folder.fill",
                    SettingsIconPalette::chat_folders(),
                    tr("settings_chat_folders_title"),
                    RootRoute::ChatFoldersSettings,
                ),
                route_row(
                    nav_store,
                    open,
                    "my_bots",
                    "command.circle.fill",
                    color("#5856D6"),
                    tr("bot_my_bots_title"),
                    RootRoute::MyBots,
                )
                .guide_anchor(RiseGuideAnchor::SettingsMyBots),
            ],
        )
    }

    fn app_group(nav_store: &NavStore, open: Option<&RootRoute>) -> GroupedBtns {
        group(
            "settings-group-app",
            tr("settings_section_app"),
            vec![
                route_row(
                    nav_store,
                    open,
                    "guides",
                    "book.closed.fill",
                    SettingsIconPalette::customization(),
                    tr("guide_catalogue_title"),
                    RootRoute::GuidesCatalogue,
                ),
                route_row(
                    nav_store,
                    open,
                    "custom",
                    "paintpalette.fill",
                    SettingsIconPalette::customization(),
                    tr("settings_customization_title"),
                    RootRoute::CustomizationSettings,
                )
                .guide_anchor(RiseGuideAnchor::SettingsAppearance),
                route_row(
                    nav_store,
                    open,
                    "language",
                    "globe",
                    SettingsIconPalette::language(),
                    tr("settings_language_title"),
                    RootRoute::LanguageSettings,
                ),
                route_row(
                    nav_store,
                    open,
                    "region",
                    "globe.europe.africa.fill",
                    SettingsIconPalette::language(),
                    tr("settings_region_title"),
                    RootRoute::RegionSettings,
                ),
                route_row(
                    nav_store,
                    open,
                    "memory",
                    "externaldrive.fill",
                    SettingsIconPalette::memory(),
                    tr("settings_memory_title"),
                    RootRoute::MemorySettings,
                ),
                route_row(
                    nav_store,
                    open,
                    "voice_video",
                    "waveform.and.mic",
                    color("#34C759"),
                    tr("rtc_voice_settings"),
                    RootRoute::RtcAudioSettings,
                ),
            ],
        )
    }

    fn report_group(nav_store: &NavStore, open: Option<&RootRoute>) -> GroupedBtns {
        let mine_nav_store = nav_store.clone();
        group(
            "settings-group-report",
            tr("report_riseonly_title"),
            vec![
                GroupedBtn::new("bug_report_open", tr("report_riseonly_title"))
                    .icon("bubble.left.and.exclamationmark.bubble.right.fill")
                    .icon_background(color("#FF3B30"))
                    .subtitle(tr("report_riseonly_subtitle"))
                    .guide_anchor(RiseGuideAnchor::SettingsBugReport)
                    .callback(|_, cx| {
                        let surface = BugReportSurfaceProbe::current();
                        BugReportInteractionsStore::shared(cx).update(cx, |store, cx| store.present(surface, cx));
                    }),
                GroupedBtn::new("bug_report_mine", tr("bug_report_my_title"))
                    .icon("tray.full.fill")
                    .icon_background(color("#5856D6"))
                    .subtitle(tr("bug_report_my_hint"))
                    .selected(open == Some(&RootRoute::MyBugReports))
                    .callback(move |_, cx| {
                        BugReportInteractionsStore::shared(cx).update(cx, |store, cx| store.select_my_reports_tab(0, cx));
                        push_route(&mine_nav_store, RootRoute::MyBugReports, cx);
                    }),
            ],
        )
    }

    fn services_group(nav_store: &NavStore, open: Option<&RootRoute>) -> GroupedBtns {
        group(
            "settings-group-services",
            tr("settings_section_services"),
            vec![
                route_row(
                    nav_store,
                    open,
                    "music",
                    "music.note",
                    color("#E84393"),
                    tr("settings_music_title"),
                    RootRoute::MusicSettings,
                ),
                route_row(
                    nav_store,
                    open,
                    "vacancies",
                    "briefcase.fill",
                    color("#5856D6"),
                    tr("settings_vacancies_title"),
                    RootRoute::VacancySettings,
                )
                .guide_anchor(RiseGuideAnchor::SettingsVacancies),
            ]
            .into_iter()
            .filter(|row| RiseFeatureGate::VACANCIES || row.id.as_ref() != "vacancies")
            .collect(),
        )
    }

    fn more_group(nav_store: &NavStore, open: Option<&RootRoute>) -> GroupedBtns {
        group(
            "settings-group-more",
            tr("more"),
            vec![route_row(
                nav_store,
                open,
                "about_project",
                "info.circle.fill",
                SettingsIconPalette::legal(),
                tr("settings_about_project"),
                RootRoute::AboutProject,
            )],
        )
    }

    fn account_group(nav_store: &NavStore, open: Option<&RootRoute>, cx: &mut App) -> GroupedBtns {
        let sparks_nav_store = nav_store.clone();
        let mut sparks_row = GroupedBtn::new("sparks", tr("sparks_my_title"))
            .leading_slot(SparksSettingsTile::new())
            .show_chevron_when_trailing_slot(true)
            .selected(open == Some(&RootRoute::SparksWallet))
            .callback(move |_, cx| push_route(&sparks_nav_store, RootRoute::SparksWallet, cx));
        let sparks = SparksServicesStore::shared(cx);
        let sparks = sparks.read(cx);
        if sparks.has_balance() {
            sparks_row = sparks_row.trailing_slot(SparksAmountLabel::new(sparks.balance().available).px(15.0).icon_size(15.0));
        }
        group(
            "settings-group-account",
            tr("settings_section_account"),
            vec![
                route_row(
                    nav_store,
                    open,
                    "subscription",
                    "crown.fill",
                    SettingsIconPalette::subscription(),
                    tr("settings_subscription_title"),
                    RootRoute::SubscriptionSettings,
                ),
                sparks_row,
                GroupedBtn::new("items", tr("settings_items_title"))
                    .icon("gift.fill")
                    .icon_background(SettingsIconPalette::privacy_gifts())
                    .callback(|_, cx| {
                        GiftsInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.open_shop_for_self(RiseGiftItemKind::Item, cx));
                    }),
                GroupedBtn::new("logout", tr("settings_logout_title"))
                    .icon("arrow.right.square.fill")
                    .icon_background(SettingsIconPalette::logout())
                    .destructive(true)
                    .callback(|_, cx| Self::present_logout_confirm(cx)),
            ],
        )
    }

    fn present_logout_confirm(cx: &mut App) {
        NativeConfirmAlert::present_delete(
            &tr("settings_logout_confirm_title"),
            "",
            Some(tr("settings_logout_title")),
            None,
            false,
            None,
            None,
            |cx| {
                AuthInteractionsStore::shared(cx).update(cx, |store, cx| store.logout(cx));
            },
            cx,
        );
    }

    fn present_delete_account_confirm(cx: &mut App) {
        NativeConfirmAlert::present_delete(
            &tr("settings_delete_account_confirm_title"),
            &format!(
                "{}\n\n{}",
                tr("settings_delete_account_confirm_message"),
                tr("settings_delete_account_subscription_note_ios")
            ),
            Some(tr("settings_delete_account_confirm_button")),
            None,
            false,
            None,
            None,
            |cx| {
                UserInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.delete_my_account(
                        |_, cx| {
                            NativeConfirmAlert::present_notice(
                                &tr("settings_delete_account_error"),
                                "",
                                None,
                                None,
                                |_| {},
                                cx,
                            );
                        },
                        cx,
                    );
                });
            },
            cx,
        );
    }

    fn delete_account_group() -> Div {
        div().w_full().pt(px(Self::GROUP_SPACING)).child(
            GroupedBtns::new(
                "settings-group-delete",
                vec![
                    GroupedBtn::new("delete-account", tr("settings_delete_account_title"))
                        .icon("trash.fill")
                        .icon_background(SettingsIconPalette::delete_account())
                        .destructive(true)
                        .callback(|_, cx| Self::present_delete_account_confirm(cx)),
                ],
            )
            .uses_lazy_rows(false),
        )
    }

    fn version_footer() -> Div {
        div().w_full().pt(px(4.0)).child(
            SecondaryText::new(format!("{} {}", tr("settings_version"), RiseReleaseNotesDeviceContext::app_version()))
                .px(12.0)
                .tac(TextAlignT::Center),
        )
    }

    fn top_profile(state: &Entity<SettingsState>, profile: &ProfileModel, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let name_color = PremiumNameColor::color_for_user(Some(profile.id.as_str()), Some(&profile.more), cx);
        let source = profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone());
        let phone_revealed = state.read(cx).phone_revealed;

        let name_row = hstack(4.0)
            .min_w_0()
            .max_w_full()
            .child(MainText::new(profile.name.clone()).px(18.0).font_weight(FontWeight::BOLD).color(name_color).line_limit(1))
            .when(profile.more.is_official, |row| row.child(OfficialBadge::new().size(px(20.0))))
            .when(profile.more.is_premium, |row| {
                row.child(
                    PremiumBadge::new()
                        .user_id(Some(SharedString::from(profile.id.clone())))
                        .emoji(profile.more.premium_badge_emoji.clone().map(SharedString::from))
                        .size(20.0),
                )
            });

        let phone = profile.more.phone.clone().filter(|phone| !phone.is_empty());
        let tag = profile.tag.clone();

        hstack(12.0)
            .w_full()
            .min_w_0()
            .px(px(16.0))
            .child(
                div().flex_none().child(
                    UserLogoUi::new("settings-top-profile-logo")
                        .source(SharedString::from(source))
                        .name(SharedString::from(profile.name.clone()))
                        .user_id(SharedString::from(profile.id.clone()))
                        .size(75.0),
                ),
            )
            .child(
                vstack(4.0)
                    .min_w_0()
                    .flex_grow(1.0)
                    .flex_shrink(1.0)
                    .align(HorizontalAlignment::Leading)
                    .child(name_row)
                    .when_some(phone, |column, phone| column.child(Self::phone_row(state, &phone, phone_revealed, theme)))
                    .when(!tag.is_empty(), |column| column.child(SecondaryText::new(format!("@{tag}")).px(14.0).line_limit(1))),
            )
            .into_any_element()
    }

    fn phone_row(state: &Entity<SettingsState>, phone: &str, phone_revealed: bool, theme: &AppTheme) -> AnyElement {
        let formatted = PhoneNumberFormatting::format(phone);
        let shown = if phone_revealed { formatted.clone() } else { RiseFormat::masked_phone(phone) };
        let toggle = state.clone();

        hstack(6.0)
            .child(
                div()
                    .id("settings-phone-value")
                    .child(
                        SecondaryText::new(shown)
                            .px(14.0)
                            .line_limit(1)
                            .glyph_features(FontFeatures(Arc::new(vec![("case".to_string(), 1)]))),
                    )
                    .when(phone_revealed, |value| {
                        value.cursor_pointer().on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(formatted.clone()));
                            Notifier::copied(cx);
                        })
                    }),
            )
            .child(
                div()
                    .relative()
                    .flex_none()
                    .w(px(22.0))
                    .h(px(16.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized(
                        if phone_revealed { "eye.slash" } else { "eye" },
                        px(14.0),
                        theme.text.secondary,
                        false,
                    ))
                    .child(
                        div()
                            .id("settings-phone-eye-hint")
                            .absolute()
                            .top(px(-14.0))
                            .left(px(-14.0))
                            .right(px(-14.0))
                            .bottom(px(-14.0))
                            .aria_label(Self::phone_toggle_label(phone_revealed))
                            .tooltip_label(Self::phone_toggle_label(phone_revealed))
                            .child(plain("settings-phone-eye").size_full().on_press(move |_, cx| {
                                toggle.update(cx, |state, cx| {
                                    state.phone_revealed = !state.phone_revealed;
                                    cx.notify();
                                });
                            })),
                    ),
            )
            .into_any_element()
    }

    fn phone_toggle_label(phone_revealed: bool) -> String {
        if phone_revealed { tr("contact_phone_hide_action") } else { tr("contact_phone_show_action") }
    }
}

impl RenderOnce for Settings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state("settings-state", cx, |_, cx| SettingsState::new(&nav_store, cx));
        let open_section = nav_store.as_ref().and_then(|nav_store| nav_store.read(cx).settings_open_section().cloned());
        let open = open_section.as_ref();
        let auth = AuthServicesStore::shared(cx);
        let profile = auth.read(cx).profile().cloned().unwrap_or_default();

        let top_profile = Self::top_profile(&state, &profile, &theme, cx);
        let account_group = Self::account_group(&nav_store, open, cx);
        let content = vstack(Self::GROUP_SPACING)
            .w_full()
            .pt(px(12.0))
            .child(top_profile)
            .child(Self::profile_group(&nav_store, open))
            .child(Self::security_group(&nav_store, open))
            .child(Self::chats_group(&nav_store, open, &profile))
            .child(Self::app_group(&nav_store, open))
            .child(Self::report_group(&nav_store, open))
            .child(Self::services_group(&nav_store, open))
            .child(account_group)
            .child(Self::more_group(&nav_store, open))
            .child(Self::delete_account_group())
            .child(Self::version_footer())
            .child(div().flex_none().h(px(80.0)));

        let shows_back = nav_store.as_ref().is_none_or(|nav_store| nav_store.read(cx).shows_settings_back());
        SettingsWrapperUi::new(tr("settings_page_title"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop_settings(cx));
            }
        })
        .shows_back_button(shows_back)
        .content(
            scroll_view("settings-scroll")
                .size_full()
                .bg(theme.bg._100)
                .list_shell_scroll_under_chrome(cx)
                .child(content),
        )
    }
}
