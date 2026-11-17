use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{
    AnyWindowHandle, App, Bounds, Context, ElementId, Entity, Pixels, Rgba, SharedString, Task, WeakEntity, Window,
    canvas,
};
use rise_canon::RiseValidation;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_platform::web_view::web_view::{
    LoadRequest, NavigationAction, NavigationPolicy, ScriptMessage, SecurityOrigin, TargetFrame, UserScript,
    UserScriptInjectionTime, WebView, WebViewColor, WebViewConfiguration, WebViewEvent, WebViewFrame, WebViewHandlers,
    create,
};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::progress_view::progress_view_value;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_rules::ValidationSchemaCanonRules;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::ui::bottom_sheet_ui::experimental_bottom_sheet_presentation_host::SheetLayerStack;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellUi, SettingsWrapperUi};
use crate::modules::bot::components::bot_ui_components::BotStateView;
use crate::modules::bot::shared::bot_ui_validation::{
    BotFormValidation, BotMiniAppInitData, BotMiniAppRouteSource, BotMiniAppURLPolicy,
};
use crate::modules::bot::stores::bot::bot_actions::bot_actions::BotActionsStore;
use crate::modules::bot::stores::bot::bot_actions::bot_types::{BotMiniAppLaunchData, BotTagRules};
use crate::modules::bot::stores::bot::bot_interactions::bot_interactions::BotInteractionsStore;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;

const SCRIPT_MESSAGE_HANDLER: &str = "riseonlyWebApp";
const ABOUT_BLANK: &str = "about:blank";
const LOAD_TIMEOUT_SECONDS: f64 = 30.0;
const WEB_VIEW_CREATION_DELAY: Duration = Duration::from_millis(350);
const HEADER_OFFSET: f32 = 56.0;
const EVENT_KEY: &str = "event";
const DATA_KEY: &str = "data";
const EVENT_CLOSE: &str = "close";
const EVENT_EXPAND: &str = "expand";
const EVENT_SEND_DATA: &str = "sendData";
const HTTPS_SCHEME: &str = "https";
const HTTPS_DEFAULT_PORT: i64 = 443;
const HTTP_DEFAULT_PORT: i64 = 80;

fn pop(nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
    }
}

#[derive(Clone)]
struct PendingLaunch {
    url: Url,
    allows_development_http: bool,
    tag: String,
    slug: String,
}

struct BotMiniAppLinkResolverState {
    error: Option<String>,
    pending_launch: Option<PendingLaunch>,
    list_finished: bool,
}

#[derive(IntoElement)]
pub struct BotMiniAppLinkResolverScreen {
    tag: String,
    slug: String,
    start_param: Option<String>,
    chat_id: Option<String>,
}

impl BotMiniAppLinkResolverScreen {
    pub fn new(
        tag: impl Into<String>,
        slug: impl Into<String>,
        start_param: Option<String>,
        chat_id: Option<String>,
    ) -> Self {
        Self { tag: tag.into(), slug: slug.into(), start_param, chat_id }
    }

    fn child_id(&self, name: &str) -> ElementId {
        ElementId::Name(SharedString::from(format!("bot-mini-app-link-{}-{}-{name}", self.tag, self.slug)))
    }
}

#[derive(Clone)]
struct LinkResolution {
    tag: String,
    slug: String,
    start_param: Option<String>,
    chat_id: Option<String>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl LinkResolution {
    fn resolve(&self, state: &Entity<BotMiniAppLinkResolverState>, cx: &mut App) {
        state.update(cx, |state, cx| {
            state.error = None;
            state.pending_launch = None;
            state.list_finished = false;
            cx.notify();
        });
        let normalized_tag = BotTagRules::sanitize(&self.tag, false, false);
        let normalized_slug = BotFormValidation::normalized_mini_app_short_name(&self.slug);
        if !BotTagRules::is_valid(&normalized_tag)
            || !BotFormValidation::is_valid_mini_app_short_name(&normalized_slug)
            || normalized_slug != self.slug.to_lowercase()
        {
            state.update(cx, |state, cx| {
                state.error = Some(tr("bot_mini_app_link_invalid"));
                cx.notify();
            });
            return;
        }

        let interactions = BotInteractionsStore::shared(cx);
        let finished_state = state.clone();
        let finished_resolution = self.clone();
        interactions.update(cx, |store, cx| {
            store.load_public_mini_apps(
                None,
                Some(&normalized_tag),
                None,
                Some(Rc::new(move |cx: &mut App| {
                    let finished_state = finished_state.clone();
                    let finished_resolution = finished_resolution.clone();
                    cx.defer(move |cx| {
                        finished_state.update(cx, |state, cx| {
                            state.list_finished = true;
                            cx.notify();
                        });
                        finished_resolution.open_pending_launch(&finished_state, cx);
                    });
                })),
                cx,
            );
        });

        let result_state = state.clone();
        let result_resolution = self.clone();
        let result_tag = normalized_tag.clone();
        let result_slug = normalized_slug.clone();
        let error_state = state.clone();
        let error_tag = normalized_tag.clone();
        let error_slug = normalized_slug.clone();
        interactions.update(cx, |store, cx| {
            store.resolve_mini_app_link(
                &normalized_tag,
                &normalized_slug,
                self.chat_id.as_deref(),
                None,
                self.start_param.as_deref(),
                true,
                Some(Rc::new(move |launch: BotMiniAppLaunchData, cx: &mut App| {
                    let result_state = result_state.clone();
                    let result_resolution = result_resolution.clone();
                    let result_tag = result_tag.clone();
                    let result_slug = result_slug.clone();
                    cx.defer(move |cx| {
                        let Some(url) = Url::from_string(&launch.launch_url) else {
                            result_state.update(cx, |state, cx| {
                                state.error = Some(tr("bot_mini_app_url_blocked"));
                                cx.notify();
                            });
                            return;
                        };
                        let allows_development_http = BotMiniAppURLPolicy::allows_development_http(&url);
                        if !BotMiniAppURLPolicy::allows(&url, allows_development_http) {
                            result_state.update(cx, |state, cx| {
                                state.error = Some(tr("bot_mini_app_url_blocked"));
                                cx.notify();
                            });
                            return;
                        }
                        result_state.update(cx, |state, cx| {
                            state.pending_launch =
                                Some(PendingLaunch { url, allows_development_http, tag: result_tag, slug: result_slug });
                            cx.notify();
                        });
                        result_resolution.open_pending_launch(&result_state, cx);
                    });
                })),
                Some(Rc::new(move |resolve_error: &anyhow::Error, cx: &mut App| {
                    let failure = resolve_error.downcast_ref::<SaiWsResponseError>();
                    let no_details = HashMap::new();
                    let text = CanonErrors::text(
                        failure.and_then(|failure| failure.code.as_deref()),
                        failure.map_or(&no_details, |failure| &failure.details),
                    )
                    .unwrap_or_else(|| format!("@{error_tag} \u{b7} {error_slug}"));
                    error_state.update(cx, |state, cx| {
                        state.error = Some(text);
                        cx.notify();
                    });
                })),
                cx,
            );
        });
    }

    fn public_bot_id(tag: &str, slug: &str, cx: &mut App) -> Option<String> {
        let apps = BotServicesStore::public_mini_apps(None, Some(tag), None, cx);
        let found = apps
            .iter()
            .find(|app| BotFormValidation::normalized_mini_app_short_name(&app.short_name) == slug)
            .or(apps.first());
        found.map(|app| app.bot_id.clone()).filter(|bot_id| !bot_id.is_empty())
    }

    fn open_pending_launch(&self, state: &Entity<BotMiniAppLinkResolverState>, cx: &mut App) {
        let Some(launch) = state.read(cx).pending_launch.clone() else {
            return;
        };
        let bot = BotServicesStore::bot_by_tag(&launch.tag, cx);
        let title = BotInteractionsStore::mini_app_title(
            bot.as_ref().map(|bot| bot.id.as_str()),
            None,
            Some(&launch.slug),
            Some(&launch.tag),
            bot.as_ref().map(|bot| bot.bot_user_id.as_str()),
            cx,
        );
        if title.is_none() && !state.read(cx).list_finished {
            return;
        }
        state.update(cx, |state, cx| {
            state.pending_launch = None;
            cx.notify();
        });
        if let Some(nav_store) = &self.nav_store {
            let route = RootRoute::BotMiniApp {
                url: launch.url,
                title: title.unwrap_or_else(|| self.slug.clone()),
                allows_development_http: launch.allows_development_http,
                chat_id: self.chat_id.clone(),
                bot_id: bot
                    .as_ref()
                    .map(|bot| bot.id.clone())
                    .or_else(|| Self::public_bot_id(&launch.tag, &launch.slug, cx)),
                is_web_link: false,
            };
            nav_store.update(cx, |nav_store, cx| nav_store.replace_top(route, cx));
        }
    }
}

impl RenderOnce for BotMiniAppLinkResolverScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._100;
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| BotMiniAppLinkResolverState {
            error: None,
            pending_launch: None,
            list_finished: false,
        });
        let error = state.read(cx).error.clone();
        let resolution = LinkResolution {
            tag: self.tag.clone(),
            slug: self.slug.clone(),
            start_param: self.start_param.clone(),
            chat_id: self.chat_id.clone(),
            nav_store: nav_store.clone(),
        };
        let retrying = resolution.clone();
        let retry_state = state.clone();

        let mut view = BotStateView::new(
            self.child_id("state-view"),
            if error.is_none() { "lock.shield.fill" } else { "exclamationmark.triangle.fill" },
            if error.is_none() { tr("bot_mini_app_resolving") } else { tr("bot_mini_app_launch_error") },
        )
        .message(error.clone().unwrap_or_else(|| format!("@{} \u{b7} {}", self.tag, self.slug)));
        if error.is_some() {
            view = view
                .action_title(tr("bot_retry"))
                .action(move |_, cx| retrying.resolve(&retry_state, cx));
        }

        let appearing = state.clone();
        SettingsWrapperUi::new(self.slug.clone(), move |_, cx| pop(&nav_store, cx))
            .content(div().size_full().bg(background).child(view))
            .on_appear(self.child_id("task"), move |_, cx| resolution.resolve(&appearing, cx))
            .on_disappear(self.child_id("closed"), |_, cx| {
                let is_pending = BotActionsStore::shared(cx).read(cx).resolve_mini_app_link_sai.is_pending();
                if !is_pending {
                    BotInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_sensitive_results(cx));
                }
            })
    }
}

struct BotMiniAppScreenState {
    progress: f64,
    can_go_back: bool,
    web_error: Option<String>,
    reload_trigger: i64,
    go_back_trigger: i64,
}

#[derive(IntoElement)]
pub struct BotMiniAppScreen {
    source: BotMiniAppRouteSource,
    title: String,
    allows_development_http: bool,
    on_send_data: Rc<dyn Fn(String, &mut Window, &mut App)>,
}

impl BotMiniAppScreen {
    pub fn new(source: BotMiniAppRouteSource, title: impl Into<String>, allows_development_http: bool) -> Self {
        Self { source, title: title.into(), allows_development_http, on_send_data: Rc::new(|_: String, _: &mut Window, _: &mut App| {}) }
    }

    pub fn on_send_data(mut self, on_send_data: impl Fn(String, &mut Window, &mut App) + 'static) -> Self {
        self.on_send_data = Rc::new(on_send_data);
        self
    }

    fn child_id(&self, name: &str) -> ElementId {
        let source = match &self.source {
            BotMiniAppRouteSource::LaunchUrl(url) => url.absolute_string().to_owned(),
            BotMiniAppRouteSource::Unresolved { tag, slug, .. } => format!("{tag}/{slug}"),
        };
        ElementId::Name(SharedString::from(format!("bot-mini-app-{source}-{name}")))
    }
}

impl RenderOnce for BotMiniAppScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| BotMiniAppScreenState {
            progress: 0.0,
            can_go_back: false,
            web_error: None,
            reload_trigger: 0,
            go_back_trigger: 0,
        });
        let (progress, web_error, reload_trigger, go_back_trigger) = {
            let current = state.read(cx);
            (current.progress, current.web_error.clone(), current.reload_trigger, current.go_back_trigger)
        };

        let content: AnyElement = match &self.source {
            BotMiniAppRouteSource::LaunchUrl(url) => {
                if BotMiniAppURLPolicy::allows(url, self.allows_development_http) {
                    let closing = nav_store.clone();
                    let reloading = state.clone();
                    zstack(Alignment::Center)
                        .size_full()
                        .child(
                            BotMiniAppWebView::new(
                                self.child_id("web-view"),
                                url.clone(),
                                self.allows_development_http,
                                Binding::entity(
                                    state.clone(),
                                    |state| state.progress,
                                    |state, value, _| state.progress = value,
                                ),
                                Binding::entity(
                                    state.clone(),
                                    |state| state.can_go_back,
                                    |state, value, _| state.can_go_back = value,
                                ),
                                Binding::entity(
                                    state.clone(),
                                    |state| state.web_error.clone(),
                                    |state, value, _| state.web_error = value,
                                ),
                                reload_trigger,
                                go_back_trigger,
                                move |_, cx| pop(&closing, cx),
                                {
                                    let on_send_data = self.on_send_data.clone();
                                    move |data: String, window: &mut Window, cx: &mut App| on_send_data(data, window, cx)
                                },
                            )
                            .is_covered(!SheetLayerStack::top_is_root(cx)),
                        )
                        .children(web_error.map(|web_error| {
                            div().absolute().size_full().bg(theme.bg._100).child(
                                BotStateView::new(
                                    self.child_id("web-error"),
                                    "wifi.exclamationmark",
                                    tr("bot_mini_app_load_error"),
                                )
                                .message(web_error)
                                .action_title(tr("bot_retry"))
                                .action(move |_, cx| {
                                    reloading.update(cx, |state, cx| {
                                        state.web_error = None;
                                        state.reload_trigger += 1;
                                        cx.notify();
                                    });
                                }),
                            )
                        }))
                        .into_any_element()
                } else {
                    BotStateView::new(
                        self.child_id("blocked"),
                        "lock.trianglebadge.exclamationmark.fill",
                        tr("bot_mini_app_url_blocked"),
                    )
                    .message(tr("bot_mini_app_url_policy_hint"))
                    .into_any_element()
                }
            }
            BotMiniAppRouteSource::Unresolved { tag, slug, start_param } => BotMiniAppUnresolvedView {
                id: self.child_id("unresolved"),
                tag: tag.clone(),
                slug: slug.clone(),
                start_param: start_param.clone(),
            }
            .into_any_element(),
        };

        let closing = nav_store.clone();
        let backing = state.clone();
        let backing_nav = nav_store;
        ListShellUi::new()
            .title(SharedString::from(self.title.clone()))
            .shows_back_button(true)
            .on_back(move |_, cx| {
                if backing.read(cx).can_go_back {
                    backing.update(cx, |state, cx| {
                        state.go_back_trigger += 1;
                        cx.notify();
                    });
                } else {
                    pop(&backing_nav, cx);
                }
            })
            .right_jsx(PageHeaderIconButton::new(self.child_id("close"), "xmark", move |_, cx| pop(&closing, cx)))
            .below_header(
                div().w_full().children(
                    (progress > 0.0 && progress < 1.0)
                        .then(|| progress_view_value(progress).tint(theme.primary._100)),
                ),
            )
            .content(div().size_full().pt(px(HEADER_OFFSET)).bg(theme.bg._100).child(content))
    }
}

#[derive(IntoElement)]
struct BotMiniAppUnresolvedView {
    id: ElementId,
    tag: String,
    slug: String,
    start_param: Option<String>,
}

impl BotMiniAppUnresolvedView {
    fn unresolved_message(&self) -> String {
        let mut value = format!("@{} \u{b7} {}", self.tag, self.slug);
        if let Some(start_param) = self.start_param.as_deref().filter(|start_param| !start_param.is_empty()) {
            value.push_str(&format!(" \u{b7} {start_param}"));
        }
        value
    }
}

impl RenderOnce for BotMiniAppUnresolvedView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let message = self.unresolved_message();
        BotStateView::new(self.id, "lock.shield.fill", tr("bot_mini_app_resolving")).message(message)
    }
}

#[derive(IntoElement, Clone)]
pub struct BotMiniAppWebView {
    id: ElementId,
    url: Url,
    allows_development_http: bool,
    progress: Binding<f64>,
    can_go_back: Binding<bool>,
    error: Binding<Option<String>>,
    reload_trigger: i64,
    go_back_trigger: i64,
    on_close: Rc<dyn Fn(&mut Window, &mut App)>,
    on_expand: Rc<dyn Fn(&mut Window, &mut App)>,
    on_send_data: Rc<dyn Fn(String, &mut Window, &mut App)>,
    is_covered: bool,
}

impl BotMiniAppWebView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        url: Url,
        allows_development_http: bool,
        progress: Binding<f64>,
        can_go_back: Binding<bool>,
        error: Binding<Option<String>>,
        reload_trigger: i64,
        go_back_trigger: i64,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
        on_send_data: impl Fn(String, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            url,
            allows_development_http,
            progress,
            can_go_back,
            error,
            reload_trigger,
            go_back_trigger,
            on_close: Rc::new(on_close),
            on_expand: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_send_data: Rc::new(on_send_data),
            is_covered: false,
        }
    }

    pub fn on_expand(mut self, on_expand: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_expand = Rc::new(on_expand);
        self
    }

    /// desktop native view floats over gpui so a sheet gpui draws above this view only shows while
    /// the page is hidden
    /// only the owner knows whats above it
    pub fn is_covered(mut self, is_covered: bool) -> Self {
        self.is_covered = is_covered;
        self
    }

    fn make_coordinator(self, cx: &mut Context<Coordinator>) -> Coordinator {
        Coordinator::new(self, cx)
    }

    fn dismantle_ui_view(coordinator: &mut Coordinator) {
        if let Some(mut web_view) = coordinator.web_view.take() {
            web_view.destroy();
        }
        coordinator.pending_view = None;
        coordinator.creation = None;
        coordinator.loaded_url = None;
    }
}

/// what the engine handlers post to the coordinator
/// they run inside the engine callback and may not call back into the view or take the app
enum CoordinatorEvent {
    NavigationBlocked,
    View(WebViewEvent),
}

struct Coordinator {
    parent: BotMiniAppWebView,
    web_view: Option<Box<dyn WebView>>,
    creation_failed: bool,
    window_handle: Option<AnyWindowHandle>,
    loaded_url: Option<Url>,
    blocked_url: Option<Url>,
    trusted_origin: Option<TrustedOrigin>,
    main_document_succeeded: bool,
    last_reload_trigger: i64,
    last_go_back_trigger: i64,
    last_frame: Option<WebViewFrame>,
    last_visible: Option<bool>,
    last_background: Option<WebViewColor>,
    allows_development_http: Rc<Cell<bool>>,
    sender: UnboundedSender<CoordinatorEvent>,
    pending_view: Option<(Bounds<Pixels>, bool, WebViewColor)>,
    creation: Option<Task<()>>,
    _events: Task<()>,
}

impl Coordinator {
    fn new(parent: BotMiniAppWebView, cx: &mut Context<Self>) -> Self {
        let (sender, mut receiver) = unbounded::<CoordinatorEvent>();
        let allows_development_http = Rc::new(Cell::new(parent.allows_development_http));
        let events = cx.spawn(async move |this: WeakEntity<Self>, cx| {
            while let Some(event) = receiver.next().await {
                if this.update(cx, |coordinator, cx| coordinator.handle(event, cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            parent,
            web_view: None,
            creation_failed: false,
            window_handle: None,
            loaded_url: None,
            blocked_url: None,
            trusted_origin: None,
            main_document_succeeded: false,
            last_reload_trigger: 0,
            last_go_back_trigger: 0,
            last_frame: None,
            last_visible: None,
            last_background: None,
            allows_development_http,
            sender,
            pending_view: None,
            creation: None,
            _events: events,
        }
    }

    /// bindings set owner state which needs the window and the paint calling load holds it
    /// so every report goes through one later turn, in order
    fn report(&self, cx: &mut App, report: impl FnOnce(&BotMiniAppWebView, &mut Window, &mut App) + 'static) {
        let Some(handle) = self.window_handle else {
            return;
        };
        let parent = self.parent.clone();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| report(&parent, window, cx));
        });
    }

    fn make_ui_view(&mut self, frame: WebViewFrame, background: WebViewColor, window: &mut Window, cx: &mut Context<Self>) {
        self.window_handle = Some(window.window_handle());
        let navigation_sender = self.sender.clone();
        let event_sender = self.sender.clone();
        let allows_development_http = self.allows_development_http.clone();
        let handlers = WebViewHandlers {
            navigation: Box::new(move |action: &NavigationAction| {
                let (policy, blocked) = decide_policy(action, allows_development_http.get());
                if blocked {
                    let _ = navigation_sender.unbounded_send(CoordinatorEvent::NavigationBlocked);
                }
                policy
            }),
            event: Box::new(move |event: WebViewEvent| {
                let _ = event_sender.unbounded_send(CoordinatorEvent::View(event));
            }),
        };
        let configuration = WebViewConfiguration {
            script_message_handler: SCRIPT_MESSAGE_HANDLER.to_owned(),
            javascript_can_open_windows_automatically: false,
            allows_back_forward_navigation_gestures: true,
            background,
        };
        match create(window, cx, frame, &configuration, handlers) {
            Ok(web_view) => {
                self.web_view = Some(web_view);
                self.last_frame = Some(frame);
                self.last_visible = None;
                self.last_background = Some(background);
                self.last_reload_trigger = self.parent.reload_trigger;
                self.last_go_back_trigger = self.parent.go_back_trigger;
                self.creation_failed = false;
                let url = self.parent.url.clone();
                self.load(&url, cx);
            }
            Err(unavailable) => {
                self.creation_failed = true;
                self.last_reload_trigger = self.parent.reload_trigger;
                let message = unavailable.to_string();
                self.report(cx, move |parent, window, cx| parent.error.set(Some(message), window, cx));
            }
        }
    }

    fn update_ui_view(
        &mut self,
        bounds: Bounds<Pixels>,
        visible: bool,
        background: WebViewColor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.allows_development_http.set(self.parent.allows_development_http);
        let frame = WebViewFrame::from_bounds(bounds);
        if self.web_view.is_none() {
            if self.creation_failed && self.last_reload_trigger == self.parent.reload_trigger {
                return;
            }
            self.pending_view = Some((bounds, visible, background));
            self.schedule_creation(window, cx);
            return;
        }
        if self.loaded_url.as_ref() != Some(&self.parent.url) && self.blocked_url.as_ref() != Some(&self.parent.url) {
            let url = self.parent.url.clone();
            self.load(&url, cx);
        }
        let reload = self.last_reload_trigger != self.parent.reload_trigger;
        self.last_reload_trigger = self.parent.reload_trigger;
        let go_back = self.last_go_back_trigger != self.parent.go_back_trigger;
        self.last_go_back_trigger = self.parent.go_back_trigger;
        let frame_changed = self.last_frame != Some(frame);
        let visible_changed = self.last_visible != Some(visible);
        let background_changed = self.last_background != Some(background);
        self.last_frame = Some(frame);
        self.last_visible = Some(visible);
        self.last_background = Some(background);

        let Some(web_view) = self.web_view.as_mut() else {
            return;
        };
        if frame_changed {
            web_view.set_frame(frame);
        }
        if visible_changed {
            web_view.set_visible(visible);
        }
        if background_changed {
            web_view.set_background(background);
        }
        if reload {
            web_view.reload();
        }
        if go_back && web_view.can_go_back() {
            web_view.go_back();
        }
    }

    fn schedule_creation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.creation.is_some() {
            return;
        }
        let handle = window.window_handle();
        self.creation = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(WEB_VIEW_CREATION_DELAY).await;
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |coordinator, cx| coordinator.create_pending_view(window, cx));
            });
        }));
    }

    fn create_pending_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((bounds, visible, background)) = self.pending_view.take() else {
            self.creation = None;
            return;
        };
        if self.web_view.is_none() {
            self.make_ui_view(WebViewFrame::from_bounds(bounds), background, window, cx);
        }
        self.creation = None;
        if self.web_view.is_some() {
            self.update_ui_view(bounds, visible, background, window, cx);
        }
    }

    fn load(&mut self, url: &Url, cx: &mut Context<Self>) {
        if !BotMiniAppURLPolicy::allows(url, self.parent.allows_development_http) {
            self.blocked_url = Some(url.clone());
            self.report(cx, |parent, window, cx| {
                parent.error.set(Some(tr("bot_mini_app_url_blocked")), window, cx);
            });
            return;
        }
        self.blocked_url = None;
        self.loaded_url = Some(url.clone());
        self.trusted_origin = origin_from_url(url);
        self.install_bridge(url);
        self.report(cx, |parent, window, cx| parent.error.set(None, window, cx));
        if let Some(web_view) = self.web_view.as_mut() {
            web_view.load(&LoadRequest {
                url: url.absolute_string().to_owned(),
                timeout_seconds: LOAD_TIMEOUT_SECONDS,
            });
        }
    }

    fn install_bridge(&mut self, url: &Url) {
        let Some(web_view) = self.web_view.as_mut() else {
            return;
        };
        web_view.remove_all_user_scripts();
        let Some(origin) = self.trusted_origin.as_ref() else {
            return;
        };
        web_view.add_user_script(&UserScript {
            source: bridge_script(&BotMiniAppInitData::extract(url), &origin.web_origin()),
            injection_time: UserScriptInjectionTime::AtDocumentStart,
            for_main_frame_only: true,
        });
    }

    fn handle(&mut self, event: CoordinatorEvent, cx: &mut Context<Self>) {
        match event {
            CoordinatorEvent::NavigationBlocked => {
                self.report(cx, |parent, window, cx| {
                    parent.error.set(Some(tr("bot_mini_app_url_blocked")), window, cx);
                });
            }
            CoordinatorEvent::View(WebViewEvent::ProvisionalNavigationStarted) => {
                self.main_document_succeeded = false;
            }
            CoordinatorEvent::View(WebViewEvent::MainFrameResponse { status_code }) => {
                self.main_document_succeeded = status_code.is_some_and(|status| (200..400).contains(&status));
            }
            CoordinatorEvent::View(WebViewEvent::Progress { estimated_progress, can_go_back }) => {
                self.report(cx, move |parent, window, cx| {
                    parent.progress.set(estimated_progress, window, cx);
                    parent.can_go_back.set(can_go_back, window, cx);
                });
            }
            CoordinatorEvent::View(WebViewEvent::Finished { url, can_go_back }) => {
                self.report(cx, move |parent, window, cx| {
                    parent.progress.set(1.0, window, cx);
                    parent.can_go_back.set(can_go_back, window, cx);
                    parent.error.set(None, window, cx);
                });
                let finished_url = url.as_deref().and_then(Url::from_string);
                if self.main_document_succeeded
                    && let Some(loaded_url) = self.loaded_url.clone()
                    && let Some(finished_url) = finished_url
                    && self.trusted_origin == origin_from_url(&finished_url)
                {
                    BotServicesStore::shared(cx)
                        .update(cx, |services, cx| services.confirm_mini_app_page_loaded(&loaded_url, cx));
                }
            }
            CoordinatorEvent::View(WebViewEvent::Failed(failure)) => {
                if failure.cancelled {
                    return;
                }
                let message = failure.message;
                self.report(cx, move |parent, window, cx| parent.error.set(Some(message), window, cx));
            }
            CoordinatorEvent::View(WebViewEvent::ScriptMessage(message)) => {
                self.user_content_controller(message, cx);
            }
        }
    }

    fn user_content_controller(&mut self, message: ScriptMessage, cx: &mut Context<Self>) {
        if message.name != SCRIPT_MESSAGE_HANDLER
            || !message.is_main_frame
            || self.trusted_origin != message.security_origin.as_ref().and_then(origin_from_security_origin)
        {
            return;
        }
        let Some(body) = message.body.as_object() else {
            return;
        };
        let Some(event) = body.get(EVENT_KEY).and_then(|event| event.as_str()) else {
            return;
        };
        match event {
            EVENT_CLOSE => self.report(cx, |parent, window, cx| (parent.on_close)(window, cx)),
            EVENT_EXPAND => self.report(cx, |parent, window, cx| (parent.on_expand)(window, cx)),
            EVENT_SEND_DATA => {
                let Some(data) = body.get(DATA_KEY).and_then(|data| data.as_str()) else {
                    return;
                };
                let clamped = RiseValidation::BOT_MINI_APP_SEND_DATA.clamped_for_tier(data, false);
                self.report(cx, move |parent, window, cx| (parent.on_send_data)(clamped, window, cx));
            }
            _ => {}
        }
    }
}

impl RenderOnce for BotMiniAppWebView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = WebViewColor::from_rgba(Rgba::from(rise_ui::theme(cx).bg._100));
        let visible = self.error.get(cx).is_none() && !self.is_covered;
        let lifecycle_id = ElementId::Name(SharedString::from(format!("{}-lifecycle", self.id)));
        let parent = self.clone();
        let coordinator = window.use_keyed_state(self.id.clone(), cx, move |_, cx| parent.make_coordinator(cx));
        coordinator.update(cx, |coordinator, _| coordinator.parent = self);
        let painting = coordinator.clone();
        let dismantling = coordinator;

        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                painting.update(cx, |coordinator, cx| {
                    coordinator.update_ui_view(bounds, visible, background, window, cx);
                });
            },
        )
        .size_full()
        .on_disappear(lifecycle_id, move |_, cx| {
            dismantling.update(cx, |coordinator, _| BotMiniAppWebView::dismantle_ui_view(coordinator));
        })
    }
}

fn decide_policy(action: &NavigationAction, allows_development_http: bool) -> (NavigationPolicy, bool) {
    let Some(raw_destination) = action.url.as_deref() else {
        return (NavigationPolicy::Cancel, false);
    };
    if raw_destination == ABOUT_BLANK {
        return (NavigationPolicy::Allow, false);
    }
    let allowed = Url::from_string(raw_destination)
        .is_some_and(|destination| BotMiniAppURLPolicy::allows(&destination, allows_development_http));
    if !allowed {
        return (NavigationPolicy::Cancel, true);
    }
    if action.target_frame == TargetFrame::NewWindow {
        return (NavigationPolicy::LoadInPlace, false);
    }
    (NavigationPolicy::Allow, false)
}

fn bridge_script(init_data: &str, origin: &str) -> String {
    let literal = javascript_string_literal(init_data);
    let origin_literal = javascript_string_literal(origin);
    format!(
        "(() => {{\n\
         if (window.location.origin !== {origin_literal}) return;\n\
         const post = (event, data) => window.webkit.messageHandlers.{SCRIPT_MESSAGE_HANDLER}.postMessage({{ event, data }});\n\
         window.Riseonly = window.Riseonly || {{}};\n\
         window.Riseonly.WebApp = {{\n\
         initData: {literal},\n\
         ready: () => post('ready', null),\n\
         close: () => post('close', null),\n\
         expand: () => post('expand', null),\n\
         sendData: (data) => post('sendData', typeof data === 'string' ? data : JSON.stringify(data))\n\
         }};\n\
         window.dispatchEvent(new Event('RiseonlyWebAppReady'));\n\
         }})();"
    )
}

fn javascript_string_literal(value: &str) -> String {
    serde_json::Value::String(value.to_owned()).to_string()
}

#[derive(Clone, PartialEq, Eq)]
struct TrustedOrigin {
    scheme: String,
    host: String,
    port: i64,
}

impl TrustedOrigin {
    fn web_origin(&self) -> String {
        let host_part = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        let port_part =
            if self.port == default_port(&self.scheme) { String::new() } else { format!(":{}", self.port) };
        format!("{}://{host_part}{port_part}", self.scheme)
    }
}

fn origin_from_url(url: &Url) -> Option<TrustedOrigin> {
    let scheme = url.scheme()?.to_lowercase();
    let host = url.host()?.to_lowercase();
    let port = url.port().unwrap_or_else(|| default_port(&scheme));
    Some(TrustedOrigin { scheme, host, port })
}

fn origin_from_security_origin(security_origin: &SecurityOrigin) -> Option<TrustedOrigin> {
    let scheme = security_origin.protocol.to_lowercase();
    let host = security_origin.host.to_lowercase();
    if scheme.is_empty() || host.is_empty() {
        return None;
    }
    let port = if security_origin.port == 0 { default_port(&scheme) } else { i64::from(security_origin.port) };
    Some(TrustedOrigin { scheme, host, port })
}

fn default_port(scheme: &str) -> i64 {
    if scheme == HTTPS_SCHEME { HTTPS_DEFAULT_PORT } else { HTTP_DEFAULT_PORT }
}
