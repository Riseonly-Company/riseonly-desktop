use gpui::{App, FontWeight, Hsla, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, hsla, px};
use rise_i18n::tr;
use rise_theme::theme::rise_theme_preferences::RiseThemeAppearance;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::environment_values::{ColorScheme, color_scheme};
use rise_ui::unread_count_badge_ui::UnreadCountBadgeUi;

use crate::app::router::navigation_types::MainTab;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::navigations::main_tab_bar::{MainTabBarItem, MainTabBarMetrics};
use crate::core::widgets::navigations::main_tab_bar_rtc_hooks::main_tab_bar_rtc_hooks;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

/// one button, the tinted copy is what the pill reveals
#[derive(IntoElement)]
pub struct MainTabBarItemLabel {
    item: MainTabBarItem,
    tinted: bool,
}

impl MainTabBarItemLabel {
    pub fn new(item: MainTabBarItem, tinted: bool) -> Self {
        Self { item, tinted }
    }

    /// canvas goes dark over riffs in a light theme, theme ink would vanish there
    fn over_media(cx: &App) -> bool {
        color_scheme(cx) == ColorScheme::Dark
            && ThemeStore::installed(cx).is_some_and(|store| store.read(cx).resolved_appearance() != RiseThemeAppearance::Dark)
    }

    fn secondary_ink(over_media: bool, text_secondary: Hsla) -> Hsla {
        if over_media { hsla(0.0, 0.0, 1.0, 0.72) } else { text_secondary }
    }

    fn icon(&self, primary: Hsla, secondary: Hsla) -> gpui::AnyElement {
        if self.item.id == MainTab::Profile {
            return MainTabBarAvatar { ringed: self.tinted, ring: primary, fallback: secondary }.into_any_element();
        }
        let color = if self.tinted { primary } else { secondary };
        let mut icon = div()
            .relative()
            .children(IconUi::sized(self.item.symbol, px(MainTabBarMetrics::RAIL_ICON_SIZE), color, self.tinted));
        if self.item.id == MainTab::Chats {
            icon = icon.child(
                div()
                    .absolute()
                    .top(px(-6.0))
                    .left(px(MainTabBarMetrics::RAIL_ICON_SIZE / 2.0 + 12.0 - 6.0))
                    .child(MainTabBarChatsBadge),
            );
        }
        icon.into_any_element()
    }
}

impl RenderOnce for MainTabBarItemLabel {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (primary, text_primary, text_secondary) = {
            let theme = ThemeStore::shared(cx).read(cx).current_theme();
            (theme.primary._100, theme.text.primary, theme.text.secondary)
        };
        let over_media = Self::over_media(cx);
        let text_primary = if over_media { gpui::white() } else { text_primary };
        let text_secondary = Self::secondary_ink(over_media, text_secondary);
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.0))
            .px(px(3.0))
            .child(
                div()
                    .h(px(MainTabBarMetrics::RAIL_ICON_SIZE + 4.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(self.icon(primary, text_secondary)),
            )
            .child(
                MainText::new(tr(self.item.title_key))
                    .px(10.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(if self.tinted { primary } else { text_primary })
                    .number_of_lines(1)
                    .minimum_scale_factor(0.7),
            )
    }
}

#[derive(IntoElement)]
struct MainTabBarChatsBadge;

impl RenderOnce for MainTabBarChatsBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        UnreadCountBadgeUi::new(ChatServicesStore::inbox_unread_badge(cx).unread_count)
    }
}

#[derive(IntoElement)]
struct MainTabBarAvatar {
    ringed: bool,
    ring: Hsla,
    fallback: Hsla,
}

impl RenderOnce for MainTabBarAvatar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let auth = AuthServicesStore::shared(cx);
        let logo = auth.read(cx).profile().map(|profile| {
            let source = profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone());
            UserLogoUi::new(SharedString::from(format!("main-tab-bar-avatar-{}", profile.id)))
                .source(Some(SharedString::from(source)))
                .name(Some(SharedString::from(profile.name.clone())))
                .user_id(Some(SharedString::from(profile.id.clone())))
                .size(22.0)
                .show_border(false)
                .show_story_ring(false)
                .open_story_on_press(false)
                .into_any_element()
        });
        let content = logo.unwrap_or_else(|| {
            div()
                .size(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .children(IconUi::sized("person.crop.circle", px(25.0), self.fallback, false))
                .into_any_element()
        });
        div()
            .relative()
            .p(px(2.5))
            .child(content)
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .rounded_full()
                    .border(px(1.6))
                    .border_color(if self.ringed { self.ring } else { gpui::transparent_black() }),
            )
            .children(main_tab_bar_rtc_hooks().map(|hooks| (hooks.local_speaking_ring)(cx)))
    }
}
