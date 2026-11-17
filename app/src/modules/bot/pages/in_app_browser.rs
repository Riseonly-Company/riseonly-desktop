use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{
    App, Bounds, BoxShadow, Context, Entity, FontWeight, Pixels, Render, Rgba, Task, WeakEntity, Window, black,
    canvas, point, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_platform::open_url::open_url;
use rise_platform::pasteboard::UIPasteboard;
use rise_platform::web_view::web_view::{
    LoadRequest, NavigationAction, NavigationPolicy, TargetFrame, WebView, WebViewColor, WebViewConfiguration,
    WebViewEvent, WebViewFrame, WebViewHandlers, create,
};
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::{PageHeaderIconButton, PageHeaderToolbarSlot};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::progress_view::progress_view_value;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};

use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::widgets::notifier::notifier_store::Notifier;
use crate::modules::bot::components::bot_ui_components::BotStateView;
use crate::modules::bot::shared::bot_ui_validation::BotMiniAppURLPolicy;

const LOAD_TIMEOUT_SECONDS: f64 = 30.0;
const SCRIPT_MESSAGE_HANDLER: &str = "riseonlyInAppBrowser";
const PAGE_SCHEMES: [&str; 4] = ["about", "data", "blob", "javascript"];
const HTTP_SCHEME: &str = "http";
const HTTPS_SCHEME: &str = "https";
const WEB_VIEW_CREATION_DELAY: Duration = Duration::from_millis(350);
const TRAY_HEIGHT: f32 = 52.0;
const TRAY_BOTTOM_GAP: f32 = 16.0;
const SWITCHER_CARD_OVERLAP: f32 = 150.0;
const SWITCHER_CARD_BODY_HEIGHT: f32 = 360.0;
const SWITCHER_CARD_ENTRY_STEP: f32 = 40.0;
const SWITCHER_DIM: f32 = 0.55;

const MENU_MINIMIZE: &str = "minimize";
const MENU_RELOAD: &str = "reload";
const MENU_COPY: &str = "copy";
const MENU_OPEN_IN_BROWSER: &str = "open_in_browser";
const MENU_CLOSE: &str = "close";

/// one page with its own web view and history
pub struct InAppBrowserTab {
    pub id: u64,
    pub title: String,
    pub url: Url,
    pub progress: f64,
    pub can_go_back: bool,
    pub error: Option<String>,
    web_view: Option<Box<dyn WebView>>,
    creation_failed: bool,
    pending_view: Option<(Bounds<Pixels>, bool, WebViewColor)>,
    creation: Option<Task<()>>,
    last_frame: Option<WebViewFrame>,
    last_visible: Option<bool>,
    last_background: Option<WebViewColor>,
    sender: UnboundedSender<InAppBrowserTabEvent>,
    _events: Task<()>,
}

/// what the engine handlers post to the page
/// they run in the engine callback and may not call back into the view or take the app
enum InAppBrowserTabEvent {
    MainFrameBlocked,
    OpenInSystem(Url),
    View(WebViewEvent),
}

impl InAppBrowserTab {
    fn new(id: u64, url: Url, cx: &mut Context<Self>) -> Self {
        let (sender, mut receiver) = unbounded::<InAppBrowserTabEvent>();
        let events = cx.spawn(async move |this: WeakEntity<Self>, cx| {
            while let Some(event) = receiver.next().await {
                if this.update(cx, |tab, cx| tab.handle(event, cx)).is_err() {
                    break;
                }
            }
        });
        let title = url.host().unwrap_or_else(|| url.absolute_string().to_owned());
        Self {
            id,
            title,
            url,
            progress: 0.0,
            can_go_back: false,
            error: None,
            web_view: None,
            creation_failed: false,
            pending_view: None,
            creation: None,
            last_frame: None,
            last_visible: None,
            last_background: None,
            sender,
            _events: events,
        }
    }

    fn host(&self) -> String {
        self.url.host().unwrap_or_default()
    }

    fn make_ui_view(&mut self, frame: WebViewFrame, background: WebViewColor, window: &mut Window, cx: &mut Context<Self>) {
        let navigation_sender = self.sender.clone();
        let event_sender = self.sender.clone();
        let handlers = WebViewHandlers {
            navigation: Box::new(move |action: &NavigationAction| {
                let (policy, event) = decide_policy(action);
                if let Some(event) = event {
                    let _ = navigation_sender.unbounded_send(event);
                }
                policy
            }),
            event: Box::new(move |event: WebViewEvent| {
                let _ = event_sender.unbounded_send(InAppBrowserTabEvent::View(event));
            }),
        };
        let configuration = WebViewConfiguration {
            script_message_handler: SCRIPT_MESSAGE_HANDLER.to_owned(),
            javascript_can_open_windows_automatically: false,
            allows_back_forward_navigation_gestures: true,
            background,
        };
        match create(window, cx, frame, &configuration, handlers) {
            Ok(mut web_view) => {
                web_view.load(&LoadRequest {
                    url: self.url.absolute_string().to_owned(),
                    timeout_seconds: LOAD_TIMEOUT_SECONDS,
                });
                self.web_view = Some(web_view);
                self.last_frame = Some(frame);
                self.last_visible = None;
                self.last_background = Some(background);
                self.creation_failed = false;
            }
            Err(unavailable) => {
                self.creation_failed = true;
                self.error = Some(unavailable.to_string());
                cx.notify();
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
        if self.web_view.is_none() {
            if self.creation_failed {
                return;
            }
            self.pending_view = Some((bounds, visible, background));
            self.schedule_creation(window, cx);
            return;
        }
        let frame = WebViewFrame::from_bounds(bounds);
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
    }

    fn schedule_creation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.creation.is_some() {
            return;
        }
        let handle = window.window_handle();
        self.creation = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(WEB_VIEW_CREATION_DELAY).await;
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |tab, cx| tab.create_pending_view(window, cx));
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

    /// the page element left the window (sheet went down or another page showed)
    /// native view stays alive with its history, just out of sight
    fn hide(&mut self) {
        self.pending_view = None;
        self.creation = None;
        if let Some(web_view) = self.web_view.as_mut()
            && self.last_visible != Some(false)
        {
            web_view.set_visible(false);
        }
        self.last_visible = Some(false);
    }

    fn go_back(&mut self) {
        if let Some(web_view) = self.web_view.as_mut() {
            web_view.go_back();
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        if self.creation_failed {
            self.creation_failed = false;
        } else if let Some(web_view) = self.web_view.as_mut() {
            web_view.reload();
        }
        cx.notify();
    }

    /// retry button of the error state, loads the page address again
    fn retry(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        if self.creation_failed {
            self.creation_failed = false;
        } else if let Some(web_view) = self.web_view.as_mut() {
            web_view.load(&LoadRequest {
                url: self.url.absolute_string().to_owned(),
                timeout_seconds: LOAD_TIMEOUT_SECONDS,
            });
        }
        cx.notify();
    }

    fn dismantle(&mut self) {
        self.pending_view = None;
        self.creation = None;
        if let Some(mut web_view) = self.web_view.take() {
            web_view.stop_loading();
            web_view.destroy();
        }
    }

    fn handle(&mut self, event: InAppBrowserTabEvent, cx: &mut Context<Self>) {
        match event {
            InAppBrowserTabEvent::MainFrameBlocked => {
                self.error = Some(tr("bot_mini_app_url_blocked"));
            }
            InAppBrowserTabEvent::OpenInSystem(url) => {
                open_url(&url, cx);
                return;
            }
            InAppBrowserTabEvent::View(WebViewEvent::ProvisionalNavigationStarted) => {
                self.error = None;
            }
            InAppBrowserTabEvent::View(WebViewEvent::Progress { estimated_progress, can_go_back }) => {
                self.progress = estimated_progress;
                self.can_go_back = can_go_back;
            }
            InAppBrowserTabEvent::View(WebViewEvent::Finished { url, can_go_back }) => {
                self.progress = 1.0;
                self.can_go_back = can_go_back;
                if let Some(url) = url.as_deref().and_then(Url::from_string) {
                    self.url = url;
                }
            }
            InAppBrowserTabEvent::View(WebViewEvent::Failed(failure)) => {
                if !failure.provisional || failure.cancelled {
                    return;
                }
                self.error = Some(failure.message);
            }
            InAppBrowserTabEvent::View(WebViewEvent::MainFrameResponse { .. })
            | InAppBrowserTabEvent::View(WebViewEvent::ScriptMessage(_)) => return,
        }
        cx.notify();
    }
}

fn decide_policy(action: &NavigationAction) -> (NavigationPolicy, Option<InAppBrowserTabEvent>) {
    let Some(raw) = action.url.as_deref() else {
        return (NavigationPolicy::Cancel, None);
    };
    let scheme = raw.split_once(':').map(|(scheme, _)| scheme.to_lowercase()).unwrap_or_default();
    if PAGE_SCHEMES.contains(&scheme.as_str()) {
        return (NavigationPolicy::Allow, None);
    }
    let is_main_frame = action.target_frame != TargetFrame::Subframe;
    if scheme == HTTP_SCHEME || scheme == HTTPS_SCHEME {
        let allowed = scheme == HTTP_SCHEME
            || Url::from_string(raw).is_some_and(|destination| BotMiniAppURLPolicy::allows(&destination, false));
        if allowed {
            if action.target_frame == TargetFrame::NewWindow {
                return (NavigationPolicy::LoadInPlace, None);
            }
            return (NavigationPolicy::Allow, None);
        }
        return (NavigationPolicy::Cancel, is_main_frame.then_some(InAppBrowserTabEvent::MainFrameBlocked));
    }
    let event = if is_main_frame { Url::from_string(raw).map(InAppBrowserTabEvent::OpenInSystem) } else { None };
    (NavigationPolicy::Cancel, event)
}

#[derive(Clone)]
struct InAppBrowserTabSlot {
    id: u64,
    tab: Entity<InAppBrowserTab>,
}

pub struct InAppBrowserStore {
    tabs: Vec<InAppBrowserTabSlot>,
    active_id: Option<u64>,
    is_sheet_presented: bool,
    is_switcher_presented: bool,
    shows_bar: bool,
    is_menu_presented: bool,
    next_tab_id: u64,
}

impl InAppBrowserStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            tabs: Vec::new(),
            active_id: None,
            is_sheet_presented: false,
            is_switcher_presented: false,
            shows_bar: false,
            is_menu_presented: false,
            next_tab_id: 0,
        }
    }

    fn active_tab(&self) -> Option<InAppBrowserTabSlot> {
        self.tabs.iter().find(|slot| Some(slot.id) == self.active_id).cloned()
    }

    /// new page for every link, the page that was open goes to the bar with the others
    pub fn open(&mut self, url: Url, cx: &mut Context<Self>) {
        self.next_tab_id += 1;
        let id = self.next_tab_id;
        let tab = cx.new(|cx| InAppBrowserTab::new(id, url, cx));
        self.tabs.push(InAppBrowserTabSlot { id, tab });
        self.active_id = Some(id);
        self.is_switcher_presented = false;
        self.is_menu_presented = false;
        self.is_sheet_presented = true;
        cx.notify();
    }

    /// sheet goes down, page stays in the bar
    pub fn minimize(&mut self, cx: &mut Context<Self>) {
        self.shows_bar = true;
        self.is_menu_presented = false;
        self.is_sheet_presented = false;
        cx.notify();
    }

    pub fn restore(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.tabs.iter().any(|slot| slot.id == id) {
            return;
        }
        self.active_id = Some(id);
        self.is_switcher_presented = false;
        self.is_sheet_presented = true;
        cx.notify();
    }

    pub fn show_switcher(&mut self, cx: &mut Context<Self>) {
        self.shows_bar = true;
        self.is_menu_presented = false;
        self.is_sheet_presented = false;
        self.is_switcher_presented = true;
        cx.notify();
    }

    pub fn hide_switcher(&mut self, cx: &mut Context<Self>) {
        self.is_switcher_presented = false;
        cx.notify();
    }

    pub fn close(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|slot| slot.id == id) else {
            return;
        };
        let slot = self.tabs.remove(index);
        slot.tab.update(cx, |tab, _| tab.dismantle());
        if self.active_id == Some(id) {
            self.active_id = self.tabs.last().map(|slot| slot.id);
            if self.active_id.is_none() {
                self.is_sheet_presented = false;
                self.is_menu_presented = false;
            }
        }
        if self.tabs.is_empty() {
            self.is_switcher_presented = false;
            self.shows_bar = false;
        }
        cx.notify();
    }

    pub fn close_all(&mut self, cx: &mut Context<Self>) {
        for slot in std::mem::take(&mut self.tabs) {
            slot.tab.update(cx, |tab, _| tab.dismantle());
        }
        self.active_id = None;
        self.shows_bar = false;
        self.is_sheet_presented = false;
        self.is_menu_presented = false;
        self.is_switcher_presented = false;
        cx.notify();
    }
}

/// mounted at the root, holds the sheet of the shown page and the bar of minimized pages and the stack
pub struct InAppBrowserHost {
    store: Entity<InAppBrowserStore>,
    tray: Presence<()>,
    switcher: Presence<()>,
    switcher_appeared: Animated<f32>,
}

impl InAppBrowserHost {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let store = InAppBrowserStore::shared(cx);
        cx.observe(&store, |this, _, cx| {
            this.sync(cx);
            cx.notify();
        })
        .detach();
        let mut host = Self {
            store,
            tray: Presence::new(None).with_transition(Transition::move_edge(Edge::Bottom).combined(Transition::opacity())),
            switcher: Presence::new(None).with_transition(Transition::opacity()),
            switcher_appeared: Animated::new(0.0),
        };
        host.sync(cx);
        host
    }

    /// spring settings, bar 0.36 and 0.86 stack opens 0.42 and 0.84 closes 0.38 and 0.86 cards
    /// settle 0.45 and 0.82
    fn sync(&mut self, cx: &mut Context<Self>) {
        let (shows_tray, shows_switcher) = {
            let state = self.store.read(cx);
            (state.shows_bar && !state.tabs.is_empty() && !state.is_switcher_presented, state.is_switcher_presented)
        };
        let now = RiseClock::monotonic();
        if shows_tray != self.tray.is_present() {
            self.tray.set_in(shows_tray.then_some(()), spring(0.36, 0.86), now, cx);
        }
        if shows_switcher != self.switcher.is_present() {
            let spec = if shows_switcher { spring(0.42, 0.84) } else { spring(0.38, 0.86) };
            self.switcher.set_in(shows_switcher.then_some(()), spec, now, cx);
            if shows_switcher {
                self.switcher_appeared.jump(0.0);
                self.switcher_appeared.set_in(1.0, spring(0.45, 0.82), now, cx);
            }
        }
    }
}

impl Render for InAppBrowserHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.clone();
        let now = RiseClock::monotonic();
        let appeared = self.switcher_appeared.frame_value(now, window);
        let tray_layer = self.tray.render_transitioned(now, window, |_| tray(&store, cx));
        let switcher_layer = self.switcher.render_transitioned(now, window, |_| switcher(&store, appeared, cx));
        let host_id = cx.entity_id();
        let sheet_store = store.clone();
        div()
            .absolute()
            .inset_0()
            .children(tray_layer)
            .children(switcher_layer)
            .child(
                NativeBottomSheetPresentationHost::new(
                    ("in-app-browser-sheet", host_id),
                    Binding::entity(
                        store.clone(),
                        |state: &InAppBrowserStore| state.is_sheet_presented,
                        |state, open, cx| {
                            if !open && state.is_sheet_presented {
                                state.minimize(cx);
                            }
                        },
                    ),
                    move |_, cx| {
                        let active = sheet_store.read(cx).active_tab();
                        match active {
                            Some(slot) => sheet_content(&sheet_store, slot, cx),
                            None => div().into_any_element(),
                        }
                    },
                )
                .presentation_detents([PresentationDetent::Large])
                .presentation_drag_indicator(true)
                .presentation_background(rise_ui::theme(cx).bg._100)
                .fills_window(true),
            )
    }
}

/// browser mark, the primary compass glyph at full button size
/// it has its own ring so no second circle around it
fn glyph(size: f32, cx: &App) -> AnyElement {
    let color = rise_ui::theme(cx).primary._100;
    div()
        .size(px(size))
        .flex_none()
        .children(IconUi::sized("safari", px(size), color, false))
        .into_any_element()
}

fn sheet_content(store: &Entity<InAppBrowserStore>, slot: InAppBrowserTabSlot, cx: &mut App) -> AnyElement {
    let theme = rise_ui::theme(cx).clone();
    let (error, menu_open) = (slot.tab.read(cx).error.clone(), store.read(cx).is_menu_presented);
    let visible = error.is_none() && !menu_open;
    let background = WebViewColor::from_rgba(Rgba::from(theme.bg._100));
    let painting = slot.tab.clone();
    let hiding = slot.tab.clone();
    let web_host = canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            painting.update(cx, |tab, cx| tab.update_ui_view(bounds, visible, background, window, cx));
        },
    )
    .size_full()
    .on_disappear(("in-app-browser-web-lifecycle", slot.id as usize), move |_, cx| {
        hiding.update(cx, |tab, _| tab.hide());
    });
    let retrying = slot.tab.clone();
    let page = div()
        .relative()
        .flex_1()
        .min_h_0()
        .w_full()
        .bg(theme.bg._100)
        .child(web_host)
        .children(error.map(|error| {
            div().absolute().inset_0().bg(theme.bg._100).child(
                BotStateView::new(
                    ("in-app-browser-error", slot.id as usize),
                    "wifi.exclamationmark",
                    tr("bot_mini_app_load_error"),
                )
                .message(error)
                .action_title(tr("bot_retry"))
                .action(move |_, cx| retrying.update(cx, |tab, cx| tab.retry(cx))),
            )
        }));
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(header(store, &slot, cx))
        .child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.35)))
        .child(page)
        .into_any_element()
}

fn header(store: &Entity<InAppBrowserStore>, slot: &InAppBrowserTabSlot, cx: &mut App) -> AnyElement {
    let theme = rise_ui::theme(cx).clone();
    let (title, host, can_go_back, progress) = {
        let tab = slot.tab.read(cx);
        (tab.title.clone(), tab.host(), tab.can_go_back, tab.progress)
    };
    let tab_count = store.read(cx).tabs.len();
    let id = slot.id as usize;

    let back_tab = slot.tab.clone();
    let back_store = store.clone();
    let tab_id = slot.id;
    let back = PageHeaderIconButton::new(
        ("in-app-browser-back", id),
        if can_go_back { "chevron.backward" } else { "xmark" },
        move |_, cx| {
            if back_tab.read(cx).can_go_back {
                back_tab.update(cx, |tab, _| tab.go_back());
            } else {
                back_store.update(cx, |store, cx| store.close(tab_id, cx));
            }
        },
    )
    .font_size(if can_go_back { 16.0 } else { 15.0 });

    let tabs_button = (tab_count > 1).then(|| {
        let switching = store.clone();
        let counter = div()
            .size(px(20.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .border(px(1.6))
            .border_color(theme.text.primary)
            .child(
                MainText::new(tab_count.to_string())
                    .px(11.0)
                    .font_weight(FontWeight::BOLD)
                    .color(theme.text.primary),
            );
        div().id(("in-app-browser-tabs-hit", id)).occlude().tooltip_label(tr("browser_tabs")).child(
            plain(("in-app-browser-tabs", id))
                .on_press(move |_, cx| switching.update(cx, |store, cx| store.show_switcher(cx)))
                .child(PageHeaderToolbarSlot::new(counter))
                .liquid_chrome_capsule_with(0.12, true, Some(0.84), false),
        )
    });

    let progress_bar = (progress > 0.0 && progress < 1.0).then(|| {
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(px(2.0))
            .child(progress_view_value(progress).tint(theme.primary._100))
    });

    div()
        .relative()
        .w_full()
        .px(px(12.0))
        .py(px(5.0))
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(104.0))
                .right(px(104.0))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(1.0))
                .child(
                    MainText::new(title)
                        .px(16.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.primary)
                        .number_of_lines(1),
                )
                .child(SecondaryText::new(host).px(11.0).tac(TextAlignT::Center).color(theme.text.secondary)),
        )
        .child(
            div()
                .relative()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(back)
                .child(div().flex_1())
                .children(tabs_button)
                .child(menu(store, slot, cx)),
        )
        .children(progress_bar)
        .into_any_element()
}

fn menu_glyph(cx: &App) -> AnyElement {
    let color = rise_ui::theme(cx).text.primary;
    PageHeaderToolbarSlot::new(
        div()
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized("ellipsis", px(17.0), color, false)),
    )
    .into_any_element()
}

fn menu(store: &Entity<InAppBrowserStore>, slot: &InAppBrowserTabSlot, cx: &mut App) -> AnyElement {
    let id = slot.id as usize;
    let is_presented = Binding::entity(
        store.clone(),
        |state: &InAppBrowserStore| state.is_menu_presented,
        |state, value, _| state.is_menu_presented = value,
    );
    let actions: HoldContextMenuPresentationActions = Rc::new(|_: &App| {
        vec![
            HoldContextMenuAction::new(MENU_MINIMIZE, tr("browser_minimize")).system_image("chevron.down"),
            HoldContextMenuAction::new(MENU_RELOAD, tr("bot_mini_app_reload")).system_image("arrow.clockwise"),
            HoldContextMenuAction::new(MENU_COPY, tr("copy_link")).system_image("link"),
            HoldContextMenuAction::new(MENU_OPEN_IN_BROWSER, tr("bot_mini_app_open_in_browser")).system_image("safari"),
            HoldContextMenuAction::new(MENU_CLOSE, tr("close")).system_image("xmark").destructive(true),
        ]
    });
    let on_action: HoldContextMenuPresentationOnAction = {
        let store = store.clone();
        let tab = slot.tab.clone();
        let tab_id = slot.id;
        Rc::new(move |action: String, _: &mut Window, cx: &mut App| match action.as_str() {
            MENU_MINIMIZE => store.update(cx, |store, cx| store.minimize(cx)),
            MENU_RELOAD => tab.update(cx, |tab, cx| tab.reload(cx)),
            MENU_COPY => {
                let url = tab.read(cx).url.clone();
                UIPasteboard::general().set_url(&url, cx);
                Notifier::copied(cx);
            }
            MENU_OPEN_IN_BROWSER => {
                let url = tab.read(cx).url.clone();
                cx.open_url(url.absolute_string());
            }
            MENU_CLOSE => store.update(cx, |store, cx| store.close(tab_id, cx)),
            _ => {}
        })
    };
    let source: HoldContextMenuTapSource = Rc::new(|_: &mut Window, cx: &mut App| menu_glyph(cx));
    let opening = store.clone();
    let button = plain(("in-app-browser-menu", id))
        .on_press(move |_, cx| {
            opening.update(cx, |store, cx| {
                store.is_menu_presented = true;
                cx.notify();
            });
        })
        .child(menu_glyph(cx));
    div()
        .id(("in-app-browser-menu-hit", id))
        .occlude()
        .child(
            div()
                .child(button)
                .liquid_chrome_capsule_with(0.12, true, Some(0.84), false)
                .tap_context_menu(
                    ("in-app-browser-menu-anchor", id),
                    is_presented,
                    HoldContextMenuAlignment::Trailing,
                    false,
                    false,
                    true,
                    None,
                    actions,
                    on_action,
                    source,
                ),
        )
        .into_any_element()
}

/// bar of minimized pages, shown page on top of the edges of the others
fn tray(store: &Entity<InAppBrowserStore>, cx: &mut App) -> Div {
    let theme = rise_ui::theme(cx).clone();
    let (slot, count) = {
        let state = store.read(cx);
        (state.active_tab().or_else(|| state.tabs.last().cloned()), state.tabs.len())
    };
    let (title, host) = slot
        .as_ref()
        .map(|slot| {
            let tab = slot.tab.read(cx);
            (tab.title.clone(), tab.host())
        })
        .unwrap_or_default();
    let subtitle = if count > 1 { tr("browser_tabs_count").replace("{count}", &count.to_string()) } else { host };
    let behind = count.saturating_sub(1).min(2);
    let layers = (0..behind).map(|depth| {
        let step = (behind - depth) as f32;
        div()
            .absolute()
            .top(px(-step * 6.0))
            .left(px(12.0 + step * 8.0))
            .right(px(12.0 + step * 8.0))
            .h(px(TRAY_HEIGHT))
            .rounded(px(16.0))
            .bg(theme.bg._300)
            .opacity(0.6 + 0.2 * depth as f32)
    });
    let closing = store.clone();
    let opening = store.clone();
    let restore_id = slot.as_ref().map(|slot| slot.id);
    let close_all = div().id("in-app-browser-tray-close-hit").occlude().tooltip_label(tr("browser_close_all")).child(
        plain("in-app-browser-tray-close")
            .size(px(30.0))
            .flex()
            .items_center()
            .justify_center()
            .on_press(move |_, cx| closing.update(cx, |store, cx| store.close_all(cx)))
            .children(IconUi::sized("xmark", px(13.0), theme.text.secondary, false)),
    );
    let bar = div()
        .id("in-app-browser-tray")
        .relative()
        .mx(px(12.0))
        .h(px(TRAY_HEIGHT))
        .px(px(12.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .rounded(px(16.0))
        .bg(theme.bg._200)
        .shadow(vec![BoxShadow {
            color: black().opacity(0.25),
            offset: point(px(0.0), px(4.0)),
            blur_radius: px(12.0),
            spread_radius: px(0.0),
            inset: false,
        }])
        .on_click(move |_, _, cx| {
            opening.update(cx, |store, cx| {
                if store.tabs.len() > 1 {
                    store.show_switcher(cx);
                } else if let Some(id) = restore_id {
                    store.restore(id, cx);
                }
            });
        })
        .child(glyph(30.0, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.0))
                .child(MainText::new(title).px(15.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
                .child(SecondaryText::new(subtitle).px(12.0).line_limit(1)),
        )
        .child(close_all);
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom(px(TRAY_BOTTOM_GAP))
        .flex()
        .justify_center()
        .child(
            div()
                .relative()
                .w_full()
                .max_w(theme.modal.width_medium)
                .children(layers)
                .child(bar),
        )
}

/// stack of pages, cards one behind the other newest in front
fn switcher(store: &Entity<InAppBrowserStore>, appeared: f32, cx: &mut App) -> Div {
    let theme = rise_ui::theme(cx).clone();
    let slots = store.read(cx).tabs.clone();
    let count = slots.len();
    let hiding = store.clone();
    let closing = store.clone();
    let cards = slots
        .into_iter()
        .enumerate()
        .map(|(index, slot)| {
            let entry = (1.0 - appeared) * (count - index) as f32 * SWITCHER_CARD_ENTRY_STEP;
            switcher_card(store, slot, index, entry, cx)
        })
        .collect::<Vec<_>>();
    div()
        .absolute()
        .inset_0()
        .occlude()
        .child(
            div()
                .id("in-app-browser-switcher-backdrop")
                .absolute()
                .inset_0()
                .bg(black().opacity(SWITCHER_DIM * appeared))
                .on_click(move |_, _, cx| hiding.update(cx, |store, cx| store.hide_switcher(cx))),
        )
        .child(
            scroll_view("in-app-browser-switcher-scroll").shows_indicators(false).child(
                div().w_full().flex().justify_center().child(
                    div()
                        .w_full()
                        .max_w(theme.modal.width_medium)
                        .px(px(20.0))
                        .pt(px(90.0))
                        .pb(px(120.0))
                        .flex()
                        .flex_col()
                        .children(cards),
                ),
            ),
        )
        .child(
            div().absolute().left_0().right_0().bottom(px(40.0)).flex().justify_center().child(
                plain("in-app-browser-switcher-close-all")
                    .h(px(44.0))
                    .px(px(22.0))
                    .flex()
                    .items_center()
                    .rounded_full()
                    .bg(white().opacity(0.18))
                    .on_press(move |_, cx| closing.update(cx, |store, cx| store.close_all(cx)))
                    .child(
                        MainText::new(tr("browser_close_all"))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(white()),
                    ),
            ),
        )
}

fn switcher_card(
    store: &Entity<InAppBrowserStore>,
    slot: InAppBrowserTabSlot,
    index: usize,
    entry: f32,
    cx: &mut App,
) -> AnyElement {
    let theme = rise_ui::theme(cx).clone();
    let (title, host) = {
        let tab = slot.tab.read(cx);
        (tab.title.clone(), tab.host())
    };
    let id = slot.id as usize;
    let closing = store.clone();
    let restoring = store.clone();
    let tab_id = slot.id;
    let close = div().id(("in-app-browser-card-close-hit", id)).occlude().child(
        plain(("in-app-browser-card-close", id))
            .size(px(28.0))
            .flex()
            .items_center()
            .justify_center()
            .on_press(move |_, cx| closing.update(cx, |store, cx| store.close(tab_id, cx)))
            .children(IconUi::sized("xmark", px(12.0), theme.text.secondary, false)),
    );
    div()
        .id(("in-app-browser-card", id))
        .relative()
        .top(px(entry))
        .w_full()
        .when(index > 0, |card| card.mt(px(-SWITCHER_CARD_OVERLAP)))
        .flex()
        .flex_col()
        .rounded(px(14.0))
        .overflow_hidden()
        .shadow(vec![BoxShadow {
            color: black().opacity(0.35),
            offset: point(px(0.0), px(-2.0)),
            blur_radius: px(14.0),
            spread_radius: px(0.0),
            inset: false,
        }])
        .on_click(move |_, _, cx| restoring.update(cx, |store, cx| store.restore(tab_id, cx)))
        .child(
            div()
                .w_full()
                .h(px(40.0))
                .px(px(12.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .bg(theme.bg._200)
                .child(glyph(22.0, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(MainText::new(title).px(14.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(1)),
                )
                .child(close),
        )
        .child(
            div()
                .w_full()
                .h(px(SWITCHER_CARD_BODY_HEIGHT))
                .flex()
                .justify_center()
                .pt(px(12.0))
                .bg(theme.bg._100)
                .child(SecondaryText::new(host).px(13.0)),
        )
        .into_any_element()
}

