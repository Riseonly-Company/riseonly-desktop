use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{
    App, Context, ElementId, Entity, FontWeight, Hsla, MouseButton, MouseDownEvent, MouseUpEvent, SharedString,
    Subscription, Window, black, white,
};
use gpui::prelude::FluentBuilder as _;
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::color;
use rise_theme::theme::app_theme::AppTheme;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::IconUi;
use rise_ui::bg_wrapper_ui::BgWrapperUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::MainText;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::scroll_chrome_shadow_ui::{
    ScrollChromeShadowStyle, ScrollChromeShadowStyleAnchor, ScrollChromeShadowStyleColorRole, ScrollChromeShadowUi,
};
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::skeleton_ui::{SkeletonCircle, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{TransitionModifiers, UnitPoint};

use crate::core::lib::rise_feature_gate::RiseFeatureGate;
use crate::app::aside::aside_navigator;
use crate::app::router::main_tab_chrome::main_tab_bar_underlap;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::music::widgets::music_cover_placeholder::MusicCoverPlaceholder;
use crate::modules::sparks::components::sparks_icon_view::SparksSettingsTile;
use crate::modules::user::stores::user::user_actions::user_types::ProfileModel;

struct MoreTabState {
    _subscription: Subscription,
}

impl MoreTabState {
    fn new(cx: &mut Context<Self>) -> Self {
        let auth_services = AuthServicesStore::shared(cx);
        Self { _subscription: cx.observe(&auth_services, |_, _, cx| cx.notify()) }
    }
}

/// nav store read in render because the environment stack is gone once a button is pressed
#[derive(Clone)]
struct MoreTabNavigator(Option<Entity<NavigationStore>>);

impl MoreTabNavigator {
    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &self.0 {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    /// a menu pick opens beside the menu, settings keep their own split in the centre
    fn open_beside(&self, route: RootRoute, cx: &mut App) {
        if matches!(route, RootRoute::Settings) || !aside_navigator::has_frontmost(cx) {
            self.push(route, cx);
            return;
        }
        aside_navigator::open(route, cx);
    }

    fn card_action(&self, route: fn() -> RootRoute) -> Rc<dyn Fn(&mut Window, &mut App)> {
        let navigator = self.clone();
        Rc::new(move |_: &mut Window, cx: &mut App| navigator.push(route(), cx))
    }

    fn btn(&self, id: &'static str, icon: &'static str, background: Hsla, title_key: &str, route: fn(&mut App) -> RootRoute) -> GroupedBtn {
        let navigator = self.clone();
        GroupedBtn::new(id, tr(title_key)).icon(icon).icon_background(background).callback(move |_, cx| {
            let route = route(cx);
            navigator.open_beside(route, cx);
        })
    }
}

#[derive(IntoElement, Default)]
pub struct MoreTab;

impl MoreTab {
    pub fn new() -> Self {
        Self
    }

    fn personal_group(navigator: &MoreTabNavigator) -> GroupedBtns {
        GroupedBtns::new(
            "more-tab-personal",
            vec![
                navigator.btn("more_achievements", "trophy", SettingsIconPalette::achievements(), "settings_achievements_title", |_| {
                    RootRoute::Achievements { user_id: None }
                }),
                navigator.btn("more_friends", "person.2.fill", SettingsIconPalette::privacy_friends(), "profile_friends", |cx| {
                    let user_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone());
                    RootRoute::UserFriends { user_id }
                }),
                navigator.btn("more_close_friends", "star.fill", SettingsIconPalette::privacy_friends(), "story_close_friends_title", |_| {
                    RootRoute::CloseFriends
                }),
                navigator.btn("more_stories_archive", "archivebox.fill", SettingsIconPalette::profile(), "archive_title", |_| {
                    RootRoute::StoriesArchive
                }),
                navigator.btn("more_view_history", "clock.arrow.circlepath", SettingsIconPalette::sessions(), "view_history_page_title", |_| {
                    RootRoute::PostViewHistory
                }),
            ],
        )
        .title(tr("more_tab_section_personal").to_uppercase())
        .horizontal_padding(0.0)
    }

    fn app_group(navigator: &MoreTabNavigator) -> GroupedBtns {
        let sparks_navigator = navigator.clone();
        GroupedBtns::new(
            "more-tab-app",
            vec![
                navigator.btn("more_subscription", "crown.fill", SettingsIconPalette::subscription(), "settings_subscription_title", |_| {
                    RootRoute::SubscriptionSettings
                }),
                GroupedBtn::new("more_sparks", tr("sparks_my_title"))
                    .leading_slot(SparksSettingsTile::new())
                    .callback(move |_, cx| sparks_navigator.open_beside(RootRoute::SparksWallet, cx)),
                navigator.btn("more_customization", "paintpalette.fill", SettingsIconPalette::customization(), "settings_customization_title", |_| {
                    RootRoute::CustomizationSettings
                }),
                navigator
                    .btn("more_guides", "book.closed.fill", SettingsIconPalette::customization(), "guide_catalogue_title", |_| {
                        RootRoute::GuidesCatalogue
                    })
                    .guide_anchor(RiseGuideAnchor::MoreGuides),
                navigator.btn("more_bots", "command.circle.fill", SettingsIconPalette::chat_folders(), "bot_my_bots_title", |_| {
                    RootRoute::MyBots
                }),
                navigator.btn("more_bug_reports", "ladybug.fill", SettingsIconPalette::moderation(), "bug_report_my_title", |_| {
                    RootRoute::MyBugReports
                }),
                navigator.btn("more_settings", "gearshape.fill", SettingsIconPalette::memory(), "settings_page_title", |_| {
                    RootRoute::Settings
                }),
            ],
        )
        .title(tr("more_tab_section_app").to_uppercase())
        .horizontal_padding(0.0)
    }

    fn profile_card(profile: Option<ProfileModel>, theme: &AppTheme, navigator: &MoreTabNavigator) -> MoreCardUi {
        let (tile, label) = match profile {
            Some(profile) => (
                UserLogoUi::new("more-tab-profile-logo")
                    .source(SharedString::from(profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone())))
                    .name(SharedString::from(profile.name.clone()))
                    .user_id(SharedString::from(profile.id.clone()))
                    .size(MoreTabStyle::TILE_SIDE)
                    .show_story_ring(false)
                    .open_story_on_press(false)
                    .into_any_element(),
                Self::profile_label(&profile, theme),
            ),
            None => (
                SkeletonCircle::new(MoreTabStyle::TILE_SIDE).skeleton_shimmer(None).into_any_element(),
                vstack(7.0)
                    .items_start()
                    .child(SkeletonLine::new(132.0, 11.0))
                    .child(SkeletonLine::new(84.0, 9.0))
                    .skeleton_shimmer(None)
                    .into_any_element(),
            ),
        };
        MoreCardUi::new("more-tab-profile", navigator.card_action(|| RootRoute::MyProfile), tile, label)
    }

    fn profile_label(profile: &ProfileModel, theme: &AppTheme) -> AnyElement {
        let name = if profile.name.is_empty() { tr("more_tab_profile") } else { profile.name.clone() };
        vstack(3.0)
            .items_start()
            .child(
                hstack(5.0)
                    .child(MoreCardTitle::text(name))
                    .when(profile.more.is_official, |row| row.child(OfficialBadge::new().size(px(16.0))))
                    .when(profile.more.is_premium, |row| {
                        row.child(
                            PremiumBadge::new()
                                .user_id(Some(SharedString::from(profile.id.clone())))
                                .emoji(profile.more.premium_badge_emoji.clone().map(SharedString::from))
                                .size(16.0),
                        )
                    }),
            )
            .when(!profile.tag.is_empty(), |column| {
                column.child(MainText::new(format!("@{}", profile.tag)).px(13.0).color(theme.text.secondary).number_of_lines(1))
            })
            .into_any_element()
    }

    fn symbol_tile(system_name: &'static str, fill: [Hsla; 2]) -> AnyElement {
        let glyph = px(MoreTabStyle::TILE_SIDE * MoreTabStyle::TILE_ICON_SCALE);
        let gradient = LinearGradient::colors(fill, UnitPoint::TOP_LEADING, UnitPoint::BOTTOM_TRAILING);
        div()
            .relative()
            .size(px(MoreTabStyle::TILE_SIDE))
            .clip_shape(Shape::continuous(MoreTabStyle::TILE_RADIUS))
            .child(div().absolute().inset_0().child(gradient.view()))
            .child(
                zstack(Alignment::Center)
                    .absolute()
                    .inset_0()
                    .child(div().relative().top(px(2.0)).children(IconUi::sized(system_name, glyph, black().opacity(0.22), true)))
                    .children(IconUi::sized(system_name, glyph, white(), true)),
            )
            .into_any_element()
    }
}

impl RenderOnce for MoreTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        window.use_keyed_state("more-tab-state", cx, |_, cx| MoreTabState::new(cx));
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let navigator = MoreTabNavigator(env::<NavigationStoreKey>(cx));
        let tab_underlap = main_tab_bar_underlap(cx);
        let profile = AuthServicesStore::shared(cx).read(cx).profile().cloned();

        let music_tile = div()
            .size(px(MoreTabStyle::TILE_SIDE))
            .child(MusicCoverPlaceholder::new().icon_scale(MoreTabStyle::TILE_ICON_SCALE).corner_radius(MoreTabStyle::TILE_RADIUS));

        BgWrapperUi::new().child(
            div()
                .relative()
                .size_full()
                .bg(theme.bg._100)
                .child(
                    scroll_view("more-tab-scroll").size_full().child(
                        vstack(12.0)
                            .w_full()
                            .px(px(16.0))
                            .pt(px(24.0))
                            .pb(px((tab_underlap + 12.0).max(24.0)))
                            .child(Self::profile_card(profile, &theme, &navigator))
                            .child(MoreCardUi::new(
                                "more-tab-music",
                                navigator.card_action(|| RootRoute::MusicTab),
                                music_tile.into_any_element(),
                                MoreCardTitle::element("more_tab_music"),
                            ))
                            .when(RiseFeatureGate::VACANCIES, |column| {
                                column.child(MoreCardUi::new(
                                    "more-tab-vacancies",
                                    navigator.card_action(|| RootRoute::VacancyList),
                                    Self::symbol_tile("briefcase.fill", *VACANCY_FILL),
                                    MoreCardTitle::element("more_tab_vacancies"),
                                ))
                            })
                            .child(MoreCardUi::new(
                                "more-tab-map",
                                navigator.card_action(|| RootRoute::WorldMap),
                                Self::symbol_tile("map.fill", *MAP_FILL),
                                MoreCardTitle::element("more_tab_map"),
                            ))
                            .child(div().w_full().pt(px(8.0)).child(Self::personal_group(&navigator)))
                            .child(div().w_full().pt(px(8.0)).child(Self::app_group(&navigator))),
                    ),
                )
                .child(MoreTabStatusBarScrim),
        )
    }
}

enum MoreCardTitle {}

impl MoreCardTitle {
    fn text(text: String) -> MainText {
        MainText::new(text).px(17.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(1)
    }

    fn element(key: &str) -> AnyElement {
        Self::text(tr(key)).into_any_element()
    }
}

static VACANCY_FILL: LazyLock<[Hsla; 2]> = LazyLock::new(|| [color("#009562"), color("#006E48")]);

static MAP_FILL: LazyLock<[Hsla; 2]> = LazyLock::new(|| [color("#1C8CE6"), color("#0B63B8")]);

enum MoreTabStyle {}

impl MoreTabStyle {
    const TILE_SIDE: f32 = 54.0;
    const TILE_RADIUS: f32 = 17.0;
    const TILE_ICON_SCALE: f32 = 0.42;
    const STATUS_BAR_FADE_HEIGHT: f32 = 18.0;

    fn status_bar_fade() -> ScrollChromeShadowStyle {
        ScrollChromeShadowStyle {
            anchor: ScrollChromeShadowStyleAnchor::FromTop,
            extent: Self::STATUS_BAR_FADE_HEIGHT,
            overflow_below_header: 0.0,
            peak_opacity: 1.0,
            color_role: ScrollChromeShadowStyleColorRole::Surface,
            mid_location: 0.5,
            mid_opacity_factor: 0.55,
            top_plateau_end: 0.12,
            top_plateau_opacity_factor: 0.96,
            ..Default::default()
        }
    }
}

#[derive(IntoElement)]
struct MoreTabStatusBarScrim;

impl RenderOnce for MoreTabStatusBarScrim {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(MoreTabStyle::STATUS_BAR_FADE_HEIGHT))
            .child(ScrollChromeShadowUi::new(MoreTabStyle::status_bar_fade()))
    }
}

#[derive(IntoElement)]
struct MoreCardUi {
    id: ElementId,
    on_press: Rc<dyn Fn(&mut Window, &mut App)>,
    tile: AnyElement,
    label: AnyElement,
}

impl MoreCardUi {
    fn new(id: impl Into<ElementId>, on_press: Rc<dyn Fn(&mut Window, &mut App)>, tile: AnyElement, label: AnyElement) -> Self {
        Self { id: id.into(), on_press, tile, label }
    }
}

impl RenderOnce for MoreCardUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let press = window.use_keyed_state(self.id.clone(), cx, |_, _| MoreCardPressStyle::new());
        let now = RiseClock::monotonic();
        let scale = {
            let press = press.read(cx);
            if press.scale.is_running(now) {
                window.request_animation_frame();
            }
            press.scale.value(now)
        };
        let down_press = press.clone();
        let up_press = press.clone();
        let on_press = self.on_press;

        plain(self.id).w_full().on_press(move |window, cx| on_press(window, cx)).child(
            hstack(14.0)
                .w_full()
                .p(px(14.0))
                .bg(theme.bg._200)
                .rounded(theme.radius._100)
                .border(px(0.5))
                .border_color(theme.border._100)
                .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _, cx| {
                    down_press.update(cx, |press, cx| press.set_pressed(true, cx));
                })
                .on_mouse_up(MouseButton::Left, {
                    let up_press = up_press.clone();
                    move |_: &MouseUpEvent, _, cx| {
                        up_press.update(cx, |press, cx| press.set_pressed(false, cx));
                    }
                })
                .on_mouse_up_out(MouseButton::Left, move |_: &MouseUpEvent, _, cx| {
                    up_press.update(cx, |press, cx| press.set_pressed(false, cx));
                })
                .child(div().flex_none().child(self.tile))
                .child(div().flex_shrink(1.0).min_w(px(0.0)).child(self.label))
                .child(spacer_min_w(8.0))
                .children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(13.0), theme.text.secondary.opacity(0.55), false))
                .scale_effect(scale),
        )
    }
}

struct MoreCardPressStyle {
    scale: Animated<f32>,
}

impl MoreCardPressStyle {
    const PRESSED_SCALE: f32 = 0.975;
    const DURATION_SECONDS: f64 = 0.14;

    fn new() -> Self {
        Self { scale: Animated::new(1.0) }
    }

    fn set_pressed(&mut self, is_pressed: bool, cx: &mut Context<Self>) {
        let target = if is_pressed { Self::PRESSED_SCALE } else { 1.0 };
        self.scale.set_in(target, ease_out(Self::DURATION_SECONDS), RiseClock::monotonic(), cx);
        cx.notify();
    }
}
