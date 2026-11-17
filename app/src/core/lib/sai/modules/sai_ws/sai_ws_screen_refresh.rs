use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Global};

use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh_debug::SaiWsScreenRefreshDebug;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaiWsScreenRefreshParams {
    Flat(HashMap<String, String>),
    PerScreen(HashMap<String, HashMap<String, String>>),
    PerScreenPresentKeys(HashMap<String, Vec<String>>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaiWsScreenRefresh {
    pub enabled: bool,
    pub when_on_screen: Vec<String>,
    pub route_params: Option<SaiWsScreenRefreshParams>,
}

impl SaiWsScreenRefresh {
    pub fn new(
        enabled: bool,
        when_on_screen: Vec<String>,
        route_params: Option<SaiWsScreenRefreshParams>,
    ) -> Self {
        Self { enabled, when_on_screen, route_params }
    }

    pub fn with_screen(
        enabled: bool,
        when_on_screen: String,
        route_params: Option<SaiWsScreenRefreshParams>,
    ) -> Self {
        Self { enabled, when_on_screen: vec![when_on_screen], route_params }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaiActiveScreen {
    pub id: String,
    pub params: HashMap<String, String>,
}

impl SaiActiveScreen {
    pub fn new(id: String, params: HashMap<String, String>) -> Self {
        Self { id, params }
    }
}

pub type SaiNavigationBridgeProvider = Rc<dyn Fn(&App) -> Option<SaiActiveScreen>>;
pub type SaiNavigationBridgeVisibleProvider = Rc<dyn Fn(&App) -> Vec<SaiActiveScreen>>;

#[derive(Default)]
struct SaiNavigationBridgeState {
    provider: Option<SaiNavigationBridgeProvider>,
    visible_provider: Option<SaiNavigationBridgeVisibleProvider>,
}

impl Global for SaiNavigationBridgeState {}

pub enum SaiNavigationBridge {}

impl SaiNavigationBridge {
    pub fn install(provider: Box<dyn Fn(&App) -> Option<SaiActiveScreen>>, cx: &mut App) {
        cx.default_global::<SaiNavigationBridgeState>().provider = Some(Rc::from(provider));
        SaiWsScreenRefreshDebug::log(|| "SaiNavigationBridge.install".to_owned(), cx);
    }

    pub fn current_screen(cx: &mut App) -> Option<SaiActiveScreen> {
        let provider = cx.default_global::<SaiNavigationBridgeState>().provider.clone();
        let Some(provider) = provider else {
            SaiWsScreenRefreshDebug::log(
                || "currentScreen → nil (SaiNavigationBridge not installed)".to_owned(),
                cx,
            );
            return None;
        };
        provider(cx)
    }

    pub fn install_visible(provider: Box<dyn Fn(&App) -> Vec<SaiActiveScreen>>, cx: &mut App) {
        cx.default_global::<SaiNavigationBridgeState>().visible_provider = Some(Rc::from(provider));
        SaiWsScreenRefreshDebug::log(|| "SaiNavigationBridge.installVisible".to_owned(), cx);
    }

    pub fn visible_screens(cx: &mut App) -> Vec<SaiActiveScreen> {
        let visible = cx.default_global::<SaiNavigationBridgeState>().visible_provider.clone();
        match visible {
            Some(visible) => visible(cx),
            None => Self::current_screen(cx).into_iter().collect(),
        }
    }
}
