use gpui::{AnyElement, App, Context, IntoElement, Render, Window, div, px};
use rise_i18n::tr;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::swiftui::prelude::*;
use rise_ui::{MainText, TextAlignT};

use crate::app::router::navigation_aside_hooks::navigation_aside_hooks;
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRenderDefaultsLoading;
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::map::components::map_friend_profile_sheet::MapFriendProfileContent;
use crate::modules::map::components::tracks::map_tracks_panel::MapPanelMode;
use crate::modules::map::engine::rise_map_profiles_repository::MapProfileTab;
use crate::modules::auth::stores::auth_services::auth_services_user_hooks::AuthServicesUserHooks;
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;

pub struct MapProfilePage {
    requested_profile: bool,
}

impl MapProfilePage {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self { requested_profile: false }
    }

    fn profile_state(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (pending, failed) = {
            let actions = UserActionsStore::shared(cx);
            let slice = &actions.read(cx).my_profile_sai;
            (slice.is_pending(), slice.error().is_some())
        };
        if failed && !pending {
            let theme = rise_ui::theme(cx).clone();
            return vstack(14.0)
                .size_full()
                .justify_center()
                .px(px(24.0))
                .child(MainText::new(tr("map_load_failed")).px(15.0).tac(TextAlignT::Center).color(theme.text.secondary))
                .child(
                    LiquidGlassButton::new("map-profile-retry", LiquidGlassButtonShape::Capsule)
                        .accessibility_label(Some(tr("common_retry")))
                        .on_press(|_, cx| AuthServicesUserHooks::refresh_my_profile(cx))
                        .child(MainText::new(tr("common_retry")).px(14.0).color(theme.text.primary)),
                )
                .into_any_element();
        }
        if !pending && !self.requested_profile {
            self.requested_profile = true;
            cx.defer(AuthServicesUserHooks::refresh_my_profile);
        }
        SaiDataRenderDefaultsLoading { message_height_percent: 40.0, is_skeleton: false }.into_any_element()
    }

    fn tab(cx: &App) -> MapProfileTab {
        match rise_ui::shared::try_read::<MapInteractionsStore>(cx).map_or(MapPanelMode::Media, |interactions| interactions.panel_mode) {
            MapPanelMode::Tracks => MapProfileTab::Statistics,
            MapPanelMode::Media => MapProfileTab::Media,
            MapPanelMode::Friends => MapProfileTab::Friends,
        }
    }

    fn select_tab(tab: MapProfileTab, cx: &mut App) {
        let mode = match tab {
            MapProfileTab::Statistics => MapPanelMode::Tracks,
            MapProfileTab::Media => MapPanelMode::Media,
            MapProfileTab::Friends => MapPanelMode::Friends,
        };
        MapInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.set_panel_mode(mode, cx);
            if mode == MapPanelMode::Tracks {
                interactions.ensure_own_tracks(cx);
            }
        });
        if mode == MapPanelMode::Friends {
            Self::back_to_friends(cx);
        }
    }

    fn back_to_friends(cx: &mut App) {
        MapInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.set_panel_mode(MapPanelMode::Friends, cx));
        if let Some(aside) = navigation_aside_hooks() {
            (aside.open_over_default)(RootRoute::MapFriends, cx);
        }
    }
}

impl Render for MapProfilePage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = Self::tab(cx);
        let own = MapInteractionsStore::own_map_profile(cx);
        let wrapper = SettingsWrapperUi::new(tab.title(), |_, cx| Self::back_to_friends(cx));
        let Some(own) = own else {
            return wrapper.content(MapProfilePageBody { content: self.profile_state(cx) });
        };
        let owner = own.user.clone();
        let active_tab = Binding::new(Self::tab, |tab, _, cx| Self::select_tab(tab, cx));
        wrapper.content(MapProfilePageBody {
            content: MapFriendProfileContent::new(
                own,
                true,
                true,
                active_tab,
                move |grid, _, cx| {
                    let owner = owner.clone();
                    MapInteractionsStore::shared(cx)
                        .update(cx, |interactions, cx| interactions.focus_profile_grid(grid, owner, false, cx));
                },
                |_, cx| Self::back_to_friends(cx),
            )
            .into_any_element(),
        })
    }
}

#[derive(IntoElement)]
struct MapProfilePageBody {
    content: AnyElement,
}

impl RenderOnce for MapProfilePageBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let top_inset = list_shell_floating_top_inset(cx);
        div().size_full().pt(px(top_inset)).child(self.content)
    }
}
