use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, AsyncApp};
use rise_canon::RiseGuideScreen;
use rise_canon::rise_key::guide_screen;
use rise_canon::rise_key::guide_screen::camel as guide_screen_keys;
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::app::router::navigation_types::{CreatePostIntent, MainTab, RootRoute};
use crate::core::guides::core_guides_guide_sandbox_hooks::SCENE_PARAM;
use crate::core::guides::guide_signal_bus::{GuideEventAwaiter, GuideEventWait};
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{SaiActiveScreen, SaiNavigationBridge};
use crate::modules::map::shared::map_settings_section::MapSettingsSection;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::PackKind;

/// where a guide asks the app to go
/// two shapes because the router has two, a tab is selected and a route is pushed
#[derive(Clone, PartialEq, Debug)]
pub enum GuideScreenDestination {
    Tab(MainTab),
    Route(RootRoute),
}

/// canon screen table, names to destinations
/// demo guide moves by route not fake taps, only maps canon screens to routes
pub enum GuideScreenResolver {}

impl GuideScreenResolver {
    pub const ROUTE_ONLY_PARAMS: [&str; 1] = [SCENE_PARAM];

    /// route or tab that puts the screen on top, none rather than a guess
    /// a screen that needs an id and got none has no destination
    pub fn destination(
        screen: RiseGuideScreen,
        params: &HashMap<String, String>,
    ) -> Option<GuideScreenDestination> {
        if cfg!(debug_assertions) {
            Self::verify_screen_table_once();
        }
        let route = |route: RootRoute| Some(GuideScreenDestination::Route(route));
        let chat_id = || Self::identifier(params.get(guide_screen_keys::CHAT_ID));
        let playlist_id = || Self::identifier(params.get(guide_screen_keys::PLAYLIST_ID));
        let bot_id = || Self::identifier(params.get(guide_screen_keys::BOT_ID));
        let intent = || {
            params
                .get(guide_screen::INTENT)
                .and_then(|raw| CreatePostIntent::from_raw(raw))
                .unwrap_or(CreatePostIntent::Publication)
        };
        match screen {
            RiseGuideScreen::Chats => Some(GuideScreenDestination::Tab(MainTab::Chats)),
            RiseGuideScreen::Home => Some(GuideScreenDestination::Tab(MainTab::Posts)),
            RiseGuideScreen::Search => Some(GuideScreenDestination::Tab(MainTab::GlobalSearch)),
            RiseGuideScreen::Riffs => Some(GuideScreenDestination::Tab(MainTab::Shorts)),
            RiseGuideScreen::More => Some(GuideScreenDestination::Tab(MainTab::Profile)),
            RiseGuideScreen::Profile => route(RootRoute::MyProfile),
            RiseGuideScreen::Music => Some(GuideScreenDestination::Tab(MainTab::Music)),
            RiseGuideScreen::Settings => route(RootRoute::Settings),
            RiseGuideScreen::Chat => {
                let chat_id = chat_id()?;
                route(RootRoute::Chat { chat_id: Some(chat_id) })
            }
            RiseGuideScreen::ChatProfile => {
                let chat_id = chat_id()?;
                route(RootRoute::ChatProfile { chat_id: Some(chat_id) })
            }
            RiseGuideScreen::CreatePost => {
                route(RootRoute::CreatePost { intent: intent(), journey_id: None })
            }
            RiseGuideScreen::MusicPlaylist => {
                let playlist_id = playlist_id()?;
                route(RootRoute::MusicPlaylistDetail { collection_id: playlist_id })
            }
            RiseGuideScreen::MusicPlaylistSettings => {
                let playlist_id = playlist_id()?;
                route(RootRoute::MusicPlaylistSettings { playlist_id })
            }
            RiseGuideScreen::MusicPlaylistInviteLink => {
                let playlist_id = playlist_id()?;
                route(RootRoute::MusicPlaylistInviteLink { playlist_id })
            }
            RiseGuideScreen::MusicPlaylistJoinRequests => {
                let playlist_id = playlist_id()?;
                route(RootRoute::MusicPlaylistJoinRequests { playlist_id })
            }
            RiseGuideScreen::PrivacySettings => route(RootRoute::PrivacySettings),
            RiseGuideScreen::MessageRequests => route(RootRoute::MessageRequests),
            RiseGuideScreen::NotificationSettings => route(RootRoute::NotificationSettings),
            RiseGuideScreen::ChatSettings => {
                let chat_id = chat_id()?;
                route(RootRoute::ChatSettings { chat_id: Some(chat_id) })
            }
            RiseGuideScreen::ChatTopics => {
                let chat_id = chat_id()?;
                route(RootRoute::ChatTopics { chat_id: Some(chat_id) })
            }
            RiseGuideScreen::ForumHome => {
                let chat_id = chat_id()?;
                route(RootRoute::ForumHome { chat_id: Some(chat_id) })
            }
            RiseGuideScreen::WorldMap => route(RootRoute::WorldMap),
            RiseGuideScreen::MapFriends => route(RootRoute::MapFriends),
            RiseGuideScreen::MapSettings => route(RootRoute::MapSettings),
            RiseGuideScreen::MapSettingsHome => {
                route(RootRoute::MapSettingsSection(MapSettingsSection::Home))
            }
            RiseGuideScreen::MapSettingsTracks => {
                route(RootRoute::MapSettingsSection(MapSettingsSection::Tracks))
            }
            RiseGuideScreen::ChatRoles => {
                let chat_id = chat_id()?;
                route(RootRoute::ChatRoles { chat_id: Some(chat_id) })
            }
            RiseGuideScreen::ChatRoleEdit => {
                let chat_id = chat_id()?;
                let role_id = Self::identifier(params.get(guide_screen_keys::ROLE_ID))?;
                route(RootRoute::ChatRoleEdit { chat_id: Some(chat_id), role_id: Some(role_id) })
            }
            RiseGuideScreen::ChatSettingsMembers => {
                let chat_id = chat_id()?;
                route(RootRoute::ChatSettingsMembers { chat_id: Some(chat_id) })
            }
            RiseGuideScreen::ChatSettingsLinkedChat => {
                let chat_id = chat_id()?;
                route(RootRoute::ChatSettingsLinkedChat { chat_id: Some(chat_id) })
            }
            RiseGuideScreen::ChatWallpaperPicker => {
                let chat_id = chat_id()?;
                route(RootRoute::ChatWallpaperPicker { chat_id: Some(chat_id), topic_id: None })
            }
            RiseGuideScreen::ChatFoldersSettings => route(RootRoute::ChatFoldersSettings),
            RiseGuideScreen::CloseFriends => route(RootRoute::CloseFriends),
            RiseGuideScreen::PostDetail => {
                let post_id = Self::identifier(params.get(guide_screen_keys::POST_ID))?;
                route(RootRoute::PostDetail { post_id: Some(post_id), origin: None })
            }
            RiseGuideScreen::PostStats => {
                let post_id = Self::identifier(params.get(guide_screen_keys::POST_ID))?;
                route(RootRoute::PostStats { post_id })
            }
            RiseGuideScreen::AudioPage => {
                let track_id = Self::identifier(params.get(guide_screen_keys::TRACK_ID))?;
                route(RootRoute::AudioPage { track_id, source_selection: None })
            }
            RiseGuideScreen::CreatePostMedia => route(RootRoute::CreatePostMedia {
                intent: intent(),
                track_seed: None,
                journey_id: None,
            }),
            RiseGuideScreen::CustomizationSettings => route(RootRoute::CustomizationSettings),
            RiseGuideScreen::ThemeSettings => route(RootRoute::ThemeSettings),
            RiseGuideScreen::BtnColorSettings => route(RootRoute::BtnColorSettings),
            RiseGuideScreen::ChatWallpapersSettings => route(RootRoute::ChatWallpapersSettings),
            RiseGuideScreen::EnergySavingSettings => route(RootRoute::EnergySavingSettings),
            RiseGuideScreen::SessionsSettings => route(RootRoute::SessionsSettings),
            RiseGuideScreen::Achievements => route(RootRoute::Achievements { user_id: None }),
            RiseGuideScreen::ProfileSettings => route(RootRoute::ProfileSettings),
            RiseGuideScreen::GoalDetail => {
                let goal_id = Self::identifier(params.get(guide_screen_keys::GOAL_ID))?;
                route(RootRoute::GoalDetail { goal_id })
            }
            RiseGuideScreen::StickerSettings => {
                route(RootRoute::StickerSettings { kind: PackKind::Sticker })
            }
            RiseGuideScreen::MyStickerPacks => {
                route(RootRoute::MyStickerPacks { kind: PackKind::Sticker })
            }
            RiseGuideScreen::MusicPlaylistMembers => {
                let playlist_id = playlist_id()?;
                route(RootRoute::MusicPlaylistMembers { playlist_id })
            }
            RiseGuideScreen::MusicPlaylistMemberRights => {
                let playlist_id = playlist_id()?;
                let member_user_id =
                    Self::identifier(params.get(guide_screen_keys::MEMBER_USER_ID))?;
                route(RootRoute::MusicPlaylistMemberRights { playlist_id, member_user_id })
            }
            RiseGuideScreen::VacancySettings => route(RootRoute::VacancySettings),
            RiseGuideScreen::CreateResume => route(RootRoute::CreateResume),
            RiseGuideScreen::MyResume => route(RootRoute::MyResume),
            RiseGuideScreen::VacancyList => route(RootRoute::VacancyList),
            RiseGuideScreen::VacancyDetail => {
                let raw = Self::identifier(params.get(guide_screen_keys::VACANCY_ID))?;
                let vacancy_id = int_from_string(&raw)?;
                route(RootRoute::VacancyDetail { vacancy_id })
            }
            RiseGuideScreen::MyVacancies => route(RootRoute::MyVacancies),
            RiseGuideScreen::CreateVacancy => route(RootRoute::CreateVacancy),
            RiseGuideScreen::MyBots => route(RootRoute::MyBots),
            RiseGuideScreen::BotSettings => {
                let bot_id = bot_id()?;
                route(RootRoute::BotSettings { bot_id })
            }
            RiseGuideScreen::BotCommands => {
                let bot_id = bot_id()?;
                route(RootRoute::BotCommands { bot_id })
            }
            RiseGuideScreen::BotWebhook => {
                let bot_id = bot_id()?;
                route(RootRoute::BotWebhook { bot_id })
            }
            RiseGuideScreen::BotMiniApps => {
                let bot_id = bot_id()?;
                route(RootRoute::BotMiniApps { bot_id })
            }
            RiseGuideScreen::BotMiniAppEditor => {
                let bot_id = bot_id()?;
                let mini_app_id = Self::identifier(params.get(guide_screen_keys::MINI_APP_ID));
                route(RootRoute::BotMiniAppEditor { bot_id, mini_app_id })
            }
        }
    }

    /// none when the app is somewhere canon doesnt address, which is most of the app
    pub fn screen(for_sai_screen_id: &str) -> Option<RiseGuideScreen> {
        RiseGuideScreen::from_raw(for_sai_screen_id)
    }

    /// reads the navigation bridge, installed once by the root view and answers from outside the view tree
    /// none before that and on screens canon doesnt name
    pub fn current_active_screen(cx: &mut App) -> Option<RiseGuideScreen> {
        let active = SaiNavigationBridge::current_screen(cx)?;
        Self::screen(&active.id)
    }

    /// extra parameters on the screen are ignored, a guide names only what it cares about
    /// same for a parameter the screen doesnt print at all
    pub fn matches(
        active: &SaiActiveScreen,
        expected: RiseGuideScreen,
        params: &HashMap<String, String>,
    ) -> bool {
        if Self::screen(&active.id) != Some(expected) {
            return false;
        }
        params
            .iter()
            .filter(|(key, _)| !Self::ROUTE_ONLY_PARAMS.iter().any(|route_only| *route_only == key.as_str()))
            .all(|(key, value)| active.params.get(key).is_none_or(|printed| printed == value))
    }

    fn identifier(raw: Option<&String>) -> Option<String> {
        let trimmed = raw.map_or("", |raw| trimming_whitespaces_and_newlines(raw));
        if trimmed.is_empty() { None } else { Some(trimmed.to_owned()) }
    }

    fn verify_screen_table_once() {
        if DID_VERIFY_SCREEN_TABLE.with(|did_verify| did_verify.replace(true)) {
            return;
        }

        let probe_params: HashMap<String, String> = [
            guide_screen_keys::CHAT_ID,
            guide_screen_keys::PLAYLIST_ID,
            guide_screen_keys::ROLE_ID,
            guide_screen_keys::POST_ID,
            guide_screen_keys::TRACK_ID,
            guide_screen_keys::GOAL_ID,
            guide_screen_keys::MEMBER_USER_ID,
            guide_screen_keys::VACANCY_ID,
            guide_screen_keys::BOT_ID,
        ]
        .into_iter()
        .map(|key| (key.to_owned(), "0".to_owned()))
        .collect();
        for canon_screen in RiseGuideScreen::ALL {
            let Some(resolved) = Self::destination(canon_screen, &probe_params) else {
                debug_assert!(false, "guide screen {} has no destination", canon_screen.raw_value());
                continue;
            };
            let id = match resolved {
                GuideScreenDestination::Tab(tab) => tab.sai_screen_id().to_owned(),
                GuideScreenDestination::Route(route) => route.sai_active_screen().id,
            };
            let mapped_back = Self::screen(&id);
            debug_assert!(
                mapped_back == Some(canon_screen),
                "guide screen {} navigates to {}, which maps back to {}",
                canon_screen.raw_value(),
                id,
                mapped_back.map_or("nothing", RiseGuideScreen::raw_value),
            );
        }
    }
}

thread_local! {
    static DID_VERIFY_SCREEN_TABLE: Cell<bool> = const { Cell::new(false) };
}

/// where the reader was before the guide moved them
/// stamped with the account since the stack is rebuilt on sign out and account switch
#[derive(Clone, PartialEq, Debug)]
pub struct GuideNavigationSnapshot {
    pub account_id: Option<String>,
    pub path: Vec<RootRoute>,
    pub tab: MainTab,
    pub adding_account: bool,
    pub creating_chat: bool,
    pub embedded_forum_chat_id: Option<String>,
}

type GuideNavigationApplier = Rc<dyn Fn(GuideScreenDestination, &mut App)>;
type GuideNavigationSnapshotter = Rc<dyn Fn(&mut App) -> Option<GuideNavigationSnapshot>>;
type GuideNavigationRestorer = Rc<dyn Fn(GuideNavigationSnapshot, &mut App)>;

#[derive(Default)]
struct GuideNavigationBridgeState {
    applier: Option<GuideNavigationApplier>,
    snapshotter: Option<GuideNavigationSnapshotter>,
    restorer: Option<GuideNavigationRestorer>,
    after_guide: Option<Box<dyn FnOnce()>>,
}

thread_local! {
    static NAVIGATION_BRIDGE: RefCell<GuideNavigationBridgeState> =
        RefCell::new(GuideNavigationBridgeState::default());
}

/// how a guide moves the app without touching the view tree
/// navigation store is view state and not a singleton
/// push deep links dont reach settings or the composer
pub enum GuideNavigationBridge {}

impl GuideNavigationBridge {
    pub fn install_applier(applier: impl Fn(GuideScreenDestination, &mut App) + 'static) {
        let applier: GuideNavigationApplier = Rc::new(applier);
        NAVIGATION_BRIDGE.with_borrow_mut(|state| state.applier = Some(applier));
    }

    pub fn install_snapshotter(
        snapshotter: impl Fn(&mut App) -> Option<GuideNavigationSnapshot> + 'static,
        restorer: impl Fn(GuideNavigationSnapshot, &mut App) + 'static,
    ) {
        let snapshotter: GuideNavigationSnapshotter = Rc::new(snapshotter);
        let restorer: GuideNavigationRestorer = Rc::new(restorer);
        NAVIGATION_BRIDGE.with_borrow_mut(|state| {
            state.snapshotter = Some(snapshotter);
            state.restorer = Some(restorer);
        });
    }

    pub fn install_after_guide(install: impl FnOnce() + 'static) {
        NAVIGATION_BRIDGE.with_borrow_mut(|state| state.after_guide = Some(Box::new(install)));
    }

    pub fn guide_did_end() {
        let install = NAVIGATION_BRIDGE.with_borrow_mut(|state| state.after_guide.take());
        if let Some(install) = install {
            install();
        }
    }

    pub fn snapshot(cx: &mut App) -> Option<GuideNavigationSnapshot> {
        let snapshotter = NAVIGATION_BRIDGE.with_borrow(|state| state.snapshotter.clone())?;
        snapshotter(cx)
    }

    pub fn restore(snapshot: GuideNavigationSnapshot, cx: &mut App) {
        let restorer = NAVIGATION_BRIDGE.with_borrow(|state| state.restorer.clone());
        if let Some(restorer) = restorer {
            restorer(snapshot, cx);
        }
    }

    /// false means the step cant be reached at all, then the runtime reports screen_unreachable
    /// arrival is a separate question for the observer
    pub fn navigate(to: RiseGuideScreen, params: &HashMap<String, String>, cx: &mut App) -> bool {
        let Some(destination) = GuideScreenResolver::destination(to, params) else {
            return false;
        };
        let Some(applier) = NAVIGATION_BRIDGE.with_borrow(|state| state.applier.clone()) else {
            return false;
        };
        applier(destination, cx);
        true
    }
}

/// live subscription to the screen observer
/// cancelling is explicit because the runtime outlives any one step
pub struct GuideScreenObservation {
    id: Uuid,
}

impl GuideScreenObservation {
    fn new() -> Self {
        Self { id: Uuid::new_v4() }
    }

    pub fn cancel(&self) {
        GuideScreenObserver::remove(self.id);
    }
}

type GuideScreenHandler = Rc<dyn Fn(&SaiActiveScreen, &mut App)>;

#[derive(Default)]
struct GuideScreenObserverState {
    handlers: HashMap<Uuid, GuideScreenHandler>,
    current: Option<SaiActiveScreen>,
    visible_handlers: HashMap<Uuid, GuideVisibleScreensHandler>,
    visible: Vec<SaiActiveScreen>,
}

type GuideVisibleScreensHandler = Rc<dyn Fn(&[SaiActiveScreen], &mut App)>;

thread_local! {
    static SCREEN_OBSERVER: RefCell<GuideScreenObserverState> =
        RefCell::new(GuideScreenObserverState::default());
}

/// which screen is on top as a stream
/// the store has no such signal and its notify has one hardcoded destination so nobody else can listen
pub enum GuideScreenObserver {}

impl GuideScreenObserver {
    /// none until the first feed
    pub fn current() -> Option<SaiActiveScreen> {
        SCREEN_OBSERVER.with_borrow(|state| state.current.clone())
    }

    pub fn screen_did_change(screen: SaiActiveScreen, cx: &mut App) {
        let handlers = SCREEN_OBSERVER.with_borrow_mut(|state| {
            if state.current.as_ref() == Some(&screen) {
                return None;
            }
            state.current = Some(screen.clone());
            Some(state.handlers.values().cloned().collect::<Vec<GuideScreenHandler>>())
        });
        let Some(handlers) = handlers else {
            return;
        };
        for handler in handlers {
            handler(&screen, cx);
        }
    }

    /// fed by the root view when what the desktop draws beside the top page changes
    /// only arrival waits listen, the top page didnt move
    pub fn visible_did_change(cx: &mut App) {
        cx.defer(|cx| {
            let visible = SaiNavigationBridge::visible_screens(cx);
            let handlers = SCREEN_OBSERVER.with_borrow_mut(|state| {
                if state.visible == visible {
                    return None;
                }
                state.visible = visible.clone();
                Some(state.visible_handlers.values().cloned().collect::<Vec<GuideVisibleScreensHandler>>())
            });
            if let Some(handlers) = &handlers
                && !handlers.is_empty()
            {
                let ids = visible.iter().map(|screen| screen.id.as_str()).collect::<Vec<&str>>();
                tracing::warn!(target: "riseonly::X2043", screens = ?ids, waits = handlers.len(), "visible screens changed");
            }
            for handler in handlers.into_iter().flatten() {
                handler(&visible, cx);
            }
        });
    }

    pub fn observe(handler: impl Fn(&SaiActiveScreen, &mut App) + 'static) -> GuideScreenObservation {
        let observation = GuideScreenObservation::new();
        let handler: GuideScreenHandler = Rc::new(handler);
        SCREEN_OBSERVER.with_borrow_mut(|state| {
            state.handlers.insert(observation.id, handler);
        });
        observation
    }

    /// returns true at once when expected is already on top
    /// an active screen satisfies the condition without navigation, false on timeout or cancel
    pub async fn wait_for(
        expected: RiseGuideScreen,
        params: &HashMap<String, String>,
        timeout_ms: i64,
        cx: &AsyncApp,
    ) -> bool {
        let (is_on_top, visible) = cx.update(|cx| {
            let visible = SaiNavigationBridge::visible_screens(cx);
            let is_on_top = SaiNavigationBridge::current_screen(cx)
                .is_some_and(|active| GuideScreenResolver::matches(&active, expected, params))
                || visible.iter().any(|visible| GuideScreenResolver::matches(visible, expected, params));
            (is_on_top, visible)
        });
        if is_on_top {
            return true;
        }
        SCREEN_OBSERVER.with_borrow_mut(|state| state.visible = visible);
        if timeout_ms <= 0 {
            return false;
        }

        let awaiter = GuideEventAwaiter::new();
        let observation = {
            let awaiter = awaiter.clone();
            let params = params.clone();
            Self::observe(move |screen: &SaiActiveScreen, _: &mut App| {
                if !GuideScreenResolver::matches(screen, expected, &params) {
                    return;
                }
                awaiter.settle(true);
            })
        };
        let visible_observation = {
            let awaiter = awaiter.clone();
            let params = params.clone();
            Self::observe_visible(move |visible: &[SaiActiveScreen], _: &mut App| {
                if visible.iter().any(|screen| GuideScreenResolver::matches(screen, expected, &params)) {
                    awaiter.settle(true);
                }
            })
        };
        let observation_id = observation.id;
        let visible_observation_id = visible_observation.id;
        let arrived = GuideEventWait::run(
            awaiter,
            timeout_ms,
            move |_: &GuideEventAwaiter| {
                Self::remove(observation_id);
                Self::remove(visible_observation_id);
            },
            cx,
        )
        .await;
        observation.cancel();
        visible_observation.cancel();
        arrived
    }

    fn observe_visible(handler: impl Fn(&[SaiActiveScreen], &mut App) + 'static) -> GuideScreenObservation {
        let observation = GuideScreenObservation::new();
        let handler: GuideVisibleScreensHandler = Rc::new(handler);
        SCREEN_OBSERVER.with_borrow_mut(|state| {
            state.visible_handlers.insert(observation.id, handler);
        });
        observation
    }

    fn remove(id: Uuid) {
        SCREEN_OBSERVER.with_borrow_mut(|state| {
            state.handlers.remove(&id);
            state.visible_handlers.remove(&id);
        });
    }
}
