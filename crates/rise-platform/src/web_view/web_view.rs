use std::cell::{Cell, RefCell};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// where the view sits in the window, in logical points from the top left of the content
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct WebViewFrame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl WebViewFrame {
    pub fn from_bounds(bounds: gpui::Bounds<gpui::Pixels>) -> Self {
        Self {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
        }
    }
}

/// srgb color with straight alpha, each component 0 to 1
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct WebViewColor {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

impl WebViewColor {
    pub fn from_rgba(color: gpui::Rgba) -> Self {
        Self {
            red: color.r,
            green: color.g,
            blue: color.b,
            alpha: color.a,
        }
    }
}

/// what the screen sets on the webview config and the view it creates
/// website data store is always the persistent default
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebViewConfiguration {
    pub script_message_handler: String,
    pub javascript_can_open_windows_automatically: bool,
    pub allows_back_forward_navigation_gestures: bool,
    pub background: WebViewColor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserScriptInjectionTime {
    AtDocumentStart,
    AtDocumentEnd,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserScript {
    pub source: String,
    pub injection_time: UserScriptInjectionTime,
    pub for_main_frame_only: bool,
}

/// load request with the protocol cache policy and a timeout
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoadRequest {
    pub url: String,
    pub timeout_seconds: f64,
}

/// port 0 means the default port of the protocol
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityOrigin {
    pub protocol: String,
    pub host: String,
    pub port: u16,
}

/// the frame a navigation lands in
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetFrame {
    MainFrame,
    Subframe,
    NewWindow,
}

/// a navigation as the handler sees it
/// url is None when the request has none, and that gets cancelled
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavigationAction {
    pub url: Option<String>,
    pub target_frame: TargetFrame,
}

/// the answer of the navigation handler
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NavigationPolicy {
    Allow,
    Cancel,
    LoadInPlace,
}

impl NavigationPolicy {
    /// the decision handed to the engine, and whether the request is loaded again in this view first
    pub fn engine_decision(self) -> EngineDecision {
        match self {
            Self::Allow => EngineDecision {
                load_in_place: false,
                allow: true,
            },
            Self::Cancel => EngineDecision {
                load_in_place: false,
                allow: false,
            },
            Self::LoadInPlace => EngineDecision {
                load_in_place: true,
                allow: false,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineDecision {
    pub load_in_place: bool,
    pub allow: bool,
}

/// a failed navigation, either full or provisional
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NavigationFailure {
    pub message: String,
    pub cancelled: bool,
    pub provisional: bool,
}

/// a body a page posted to the script message handler
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScriptMessage {
    pub name: String,
    pub is_main_frame: bool,
    pub security_origin: Option<SecurityOrigin>,
    pub body: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScriptMessageEnvelope {
    pub name: String,
    pub body: serde_json::Value,
}

impl ScriptMessage {
    /// reads an envelope posted through webkit_message_shim
    /// text that isnt an envelope is dropped
    pub fn from_envelope(
        text: &str,
        is_main_frame: bool,
        security_origin: Option<SecurityOrigin>,
    ) -> Option<Self> {
        let envelope: ScriptMessageEnvelope = serde_json::from_str(text).ok()?;
        Some(Self {
            name: envelope.name,
            is_main_frame,
            security_origin,
            body: envelope.body,
        })
    }
}

/// what the engine reports, in the order the coordinator gets it
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WebViewEvent {
    ProvisionalNavigationStarted,
    MainFrameResponse { status_code: Option<u16> },
    Progress {
        estimated_progress: f64,
        can_go_back: bool,
    },
    Finished {
        url: Option<String>,
        can_go_back: bool,
    },
    Failed(NavigationFailure),
    ScriptMessage(ScriptMessage),
}

pub type NavigationHandler = Box<dyn FnMut(&NavigationAction) -> NavigationPolicy>;
pub type EventHandler = Box<dyn FnMut(WebViewEvent)>;

pub struct WebViewHandlers {
    pub navigation: NavigationHandler,
    pub event: EventHandler,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WebViewUnavailable {
    #[error("web views are created on the main thread")]
    NotMainThread,
    #[error("the gpui window has no native handle this engine can host a view in")]
    NoNativeWindow,
    #[error("the {engine} engine is not available: {reason}")]
    EngineMissing {
        engine: &'static str,
        reason: String,
        install_url: Option<&'static str>,
    },
    #[error("the web view helper process failed: {0}")]
    Helper(String),
}

/// a native web view placed over a gpui element, every method runs on the main thread
pub trait WebView {
    fn set_frame(&mut self, frame: WebViewFrame);
    fn set_visible(&mut self, visible: bool);
    /// loads a request, a url the engine cant parse is reported as Failed
    fn load(&mut self, request: &LoadRequest);
    fn add_user_script(&mut self, script: &UserScript);
    fn remove_all_user_scripts(&mut self);
    fn evaluate_script(&mut self, source: &str);
    fn reload(&mut self);
    fn go_back(&mut self) -> bool;
    fn can_go_back(&self) -> bool;
    fn stop_loading(&mut self);
    /// url of the main frame, None before the first commit
    fn url(&self) -> Option<String>;
    fn set_background(&mut self, color: WebViewColor);
    /// gives keyboard focus to the page
    fn focus(&mut self);
    /// tears the view down in DISMANTLE_ORDER then it leaves the window
    /// safe to call twice, Drop calls it too
    fn destroy(&mut self);
}

/// creates the view inside window at frame and starts reporting to handlers
/// call on the main thread from a gpui update
pub fn create(
    window: &gpui::Window,
    cx: &mut gpui::App,
    frame: WebViewFrame,
    configuration: &WebViewConfiguration,
    handlers: WebViewHandlers,
) -> Result<Box<dyn WebView>, WebViewUnavailable> {
    super::os::create(window, cx, frame, configuration, handlers)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TeardownStep {
    StopLoading,
    DetachNavigationDelegate,
    DetachUiDelegate,
    RemoveScriptMessageHandler,
    StopProgressObservation,
}

pub const DISMANTLE_ORDER: [TeardownStep; 5] = [
    TeardownStep::StopLoading,
    TeardownStep::DetachNavigationDelegate,
    TeardownStep::DetachUiDelegate,
    TeardownStep::RemoveScriptMessageHandler,
    TeardownStep::StopProgressObservation,
];

/// holds the caller handlers for an engine delegate and calls them safely
/// a panic doesnt unwind into the engine and a destroyed view stops reporting at once
/// a re entrant call isnt delivered
pub struct HandlerSlot {
    handlers: RefCell<Option<WebViewHandlers>>,
    detached: Cell<bool>,
}

impl HandlerSlot {
    pub fn new(handlers: WebViewHandlers) -> Self {
        Self {
            handlers: RefCell::new(Some(handlers)),
            detached: Cell::new(false),
        }
    }

    /// the handler policy, None when the slot is detached or busy or the handler panicked
    pub fn decide(&self, action: &NavigationAction) -> Option<NavigationPolicy> {
        self.with_handlers(|handlers| (handlers.navigation)(action))
    }

    /// delivers event and says if a handler took it
    pub fn emit(&self, event: WebViewEvent) -> bool {
        self.with_handlers(move |handlers| (handlers.event)(event))
            .is_some()
    }

    /// drops the handlers, nothing is delivered after
    pub fn detach(&self) {
        self.detached.set(true);
        if let Ok(mut handlers) = self.handlers.try_borrow_mut() {
            handlers.take();
        }
    }

    pub fn is_detached(&self) -> bool {
        self.detached.get()
    }

    fn with_handlers<R>(&self, call: impl FnOnce(&mut WebViewHandlers) -> R) -> Option<R> {
        if self.detached.get() {
            return None;
        }
        let mut handlers = self.handlers.try_borrow_mut().ok()?.take()?;
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call(&mut handlers)));
        if !self.detached.get() {
            if let Ok(mut slot) = self.handlers.try_borrow_mut() {
                *slot = Some(handlers);
            }
        }
        match answer {
            Ok(answer) => Some(answer),
            Err(_) => {
                tracing::error!(target: "riseonly", "web view handler panicked");
                None
            }
        }
    }
}

/// document start script for engines with no webkit messageHandlers
/// defines that object for handler_name and forwards each body as ScriptMessageEnvelope json
/// sent to post, the engine own channel
pub fn webkit_message_shim(handler_name: &str, post: &str) -> String {
    let name = serde_json::Value::String(handler_name.to_owned()).to_string();
    format!(
        "(() => {{\n\
         const name = {name};\n\
         const post = (body) => {post}(JSON.stringify({{ name, body: body === undefined ? null : body }}));\n\
         const root = (window.webkit = window.webkit || {{}});\n\
         const handlers = (root.messageHandlers = root.messageHandlers || {{}});\n\
         handlers[name] = Object.freeze({{ postMessage: post }});\n\
         }})();"
    )
}

pub const HELPER_BINARY: &str = "riseonly-web-view-helper";

/// the native window the helper places its view over
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HelperParent {
    X11 { window: u64 },
    Wayland,
}

/// a line from the app to the helper
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum HelperCommand {
    Create {
        parent: HelperParent,
        scale_factor: f32,
        frame: WebViewFrame,
        configuration: WebViewConfiguration,
    },
    SetFrame { frame: WebViewFrame },
    SetVisible { visible: bool },
    Load { request: LoadRequest },
    AddUserScript { script: UserScript },
    RemoveAllUserScripts,
    EvaluateScript { source: String },
    Reload,
    GoBack,
    StopLoading,
    SetBackground { color: WebViewColor },
    Focus,
    NavigationDecision { id: u64, policy: NavigationPolicy },
    Destroy,
}

/// a line from the helper to the app
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HelperEvent {
    Ready,
    NavigationQuery { id: u64, action: NavigationAction },
    History {
        can_go_back: bool,
        url: Option<String>,
    },
    View { event: WebViewEvent },
    Unavailable { reason: String },
}

pub fn encode_helper_line<T: Serialize>(message: &T) -> serde_json::Result<String> {
    let mut line = serde_json::to_string(message)?;
    line.push('\n');
    Ok(line)
}

pub fn decode_helper_line<T: serde::de::DeserializeOwned>(line: &str) -> serde_json::Result<T> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    serde_json::from_str(line.strip_suffix('\r').unwrap_or(line))
}
