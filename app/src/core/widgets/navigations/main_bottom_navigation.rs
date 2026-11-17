use gpui::{
    AnyElement, App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use rise_canon::{RiseGuideAnchor, RiseTabBarButton};
use rise_i18n::tr;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::modifiers::ModifierExt;
use rise_ui::unread_count_badge_ui::UnreadCountBadgeUi;

use crate::app::router::more_tab_bar_menu::{MoreTabBarMenuHoldModifier, MoreTabBarMenuState};
use crate::app::router::navigation_types::MainTab;
#[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
use crate::core::lib::sai::modules::sai_debugger::sai_debugger_overlay_state::SaiDebuggerOverlayState;
use crate::core::widgets::navigations::main_tab_bar_buttons::RiseTabBarButtonExt;
use crate::core::widgets::navigations::main_tab_scroll_to_top::MainTabScrollToTop;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::user::engine::rise_user_tab_bar_presentation::RiseUserTabBarPresentation;

#[derive(Clone, Copy)]
struct NavBtn {
    id: MainTab,
    text_key: &'static str,
    icon: &'static str,
    anchor: RiseGuideAnchor,
}

const NAV_BTNS: [NavBtn; 5] = [
    NavBtn { id: MainTab::Posts, text_key: "navbtn_posts", icon: "doc.text", anchor: RiseGuideAnchor::TabPosts },
    NavBtn { id: MainTab::GlobalSearch, text_key: "navbtn_search", icon: "magnifyingglass", anchor: RiseGuideAnchor::TabSearch },
    NavBtn {
        id: MainTab::Chats,
        text_key: "navbtn_chats",
        icon: "bubble.left.and.bubble.right",
        anchor: RiseGuideAnchor::TabChats,
    },
    NavBtn { id: MainTab::Shorts, text_key: "riffs", icon: "play.rectangle.on.rectangle", anchor: RiseGuideAnchor::TabRiffs },
    NavBtn { id: MainTab::Profile, text_key: "more_tab_title", icon: "ellipsis.circle", anchor: RiseGuideAnchor::TabMore },
];

#[derive(IntoElement)]
struct ChatsTabUnreadBadge;

impl RenderOnce for ChatsTabUnreadBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        UnreadCountBadgeUi::new(ChatServicesStore::inbox_unread_badge(cx).unread_count)
    }
}

#[derive(IntoElement)]
pub struct MainBottomNavigation {
    current_tab: Binding<MainTab>,
    is_preview: bool,
    order: Option<Vec<RiseTabBarButton>>,
}

impl MainBottomNavigation {
    pub fn new(current_tab: Binding<MainTab>) -> Self {
        Self { current_tab, is_preview: false, order: None }
    }

    pub fn order(mut self, order: Option<Vec<RiseTabBarButton>>) -> Self {
        self.order = order;
        self
    }

    /// the four buttons in the reader order, more always last
    fn ordered_btns(&self, cx: &mut App) -> Vec<NavBtn> {
        let four = match &self.order {
            Some(order) => order.clone(),
            None => RiseUserTabBarPresentation::shared(cx).read(cx).current().more_mode_order(),
        };
        let placed: Vec<NavBtn> = four
            .iter()
            .filter_map(|button| NAV_BTNS.iter().find(|btn| btn.id == button.tab()).copied())
            .collect();
        if placed.len() != 4 {
            return NAV_BTNS.to_vec();
        }
        placed.into_iter().chain(NAV_BTNS.iter().copied().filter(|btn| btn.id == MainTab::Profile)).collect()
    }

    pub fn is_preview(mut self, is_preview: bool) -> Self {
        self.is_preview = is_preview;
        self
    }

    fn tap(btn: NavBtn, current_tab: &Binding<MainTab>, is_preview: bool, window: &mut Window, cx: &mut App) {
        if is_preview {
            current_tab.set(btn.id, window, cx);
            return;
        }
        if btn.id == MainTab::Profile && MoreTabBarMenuState::shared(cx).read(cx).swallows_tap() {
            return;
        }
        #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
        if btn.id == MainTab::Profile {
            SaiDebuggerOverlayState::shared(cx).update(cx, |overlay, cx| overlay.register_profile_tab_tap(cx));
        }
        if btn.id == current_tab.get(cx) {
            MainTabScrollToTop::reselect(btn.id, window, cx);
        }
        current_tab.set(btn.id, window, cx);
    }
}

impl RenderOnce for MainBottomNavigation {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (primary, text_primary, text_secondary) = {
            let theme = ThemeStore::shared(cx).read(cx).current_theme();
            (theme.primary._100, theme.text.primary, theme.text.secondary)
        };
        let current = self.current_tab.get(cx);
        let ordered_btns = self.ordered_btns(cx);

        let mut row = div().flex().flex_row().w_full().px(px(10.0));
        for (index, btn) in ordered_btns.into_iter().enumerate() {
            let active = btn.id == current;
            let mut icon = div()
                .relative()
                .children(IconUi::sized(btn.icon, px(18.0), if active { primary } else { text_secondary }, false));
            if btn.id == MainTab::Chats {
                icon = icon.child(div().absolute().top(px(5.0)).left(px(9.0)).child(ChatsTabUnreadBadge));
            }
            let current_tab = self.current_tab.clone();
            let is_preview = self.is_preview;
            let button: AnyElement = div()
                .id(ElementId::NamedInteger(SharedString::new_static("main-bottom-navigation"), index as u64))
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(3.0))
                .pt(px(9.0))
                .pb(px(7.0))
                .cursor_pointer()
                .on_click(move |_: &ClickEvent, window, cx| Self::tap(btn, &current_tab, is_preview, window, cx))
                .child(icon)
                .child(
                    MainText::new(tr(btn.text_key))
                        .px(10.0)
                        .tac(TextAlignT::Center)
                        .color(if active { primary } else { text_primary })
                        .number_of_lines(1)
                        .minimum_scale_factor(0.72)
                        .w_full(),
                )
                .into_any_element();
            let button = button.modifier(MoreTabBarMenuHoldModifier::new(!self.is_preview && btn.id == MainTab::Profile));
            let anchor = if self.is_preview { None } else { Some(btn.anchor) };
            row = row.child(button.guide_anchor(anchor));
        }
        row.liquid_chrome_capsule_with(0.12, true, None, false)
    }
}
