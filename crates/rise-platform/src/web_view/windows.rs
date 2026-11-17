use std::cell::{Cell, RefCell};
use std::rc::Rc;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_COLOR, COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC, COREWEBVIEW2_WEB_ERROR_STATUS,
    COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT,
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT,
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED,
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID,
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED,
    COREWEBVIEW2_WEB_ERROR_STATUS_CLIENT_CERTIFICATE_CONTAINS_ERRORS,
    COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED,
    COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET,
    COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED,
    COREWEBVIEW2_WEB_ERROR_STATUS_ERROR_HTTP_INVALID_SERVER_RESPONSE,
    COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED,
    COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED, COREWEBVIEW2_WEB_ERROR_STATUS_REDIRECT_FAILED,
    COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE, COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT,
    COREWEBVIEW2_WEB_ERROR_STATUS_UNEXPECTED_ERROR, COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
    COREWEBVIEW2_WEB_ERROR_STATUS_VALID_AUTHENTICATION_CREDENTIALS_REQUIRED,
    COREWEBVIEW2_WEB_ERROR_STATUS_VALID_PROXY_AUTHENTICATION_REQUIRED,
    CreateCoreWebView2EnvironmentWithOptions, GetAvailableCoreWebView2BrowserVersionString,
    ICoreWebView2, ICoreWebView2_2,
    ICoreWebView2AddScriptToExecuteOnDocumentCreatedCompletedHandler,
    ICoreWebView2Controller, ICoreWebView2Controller2,
    ICoreWebView2CreateCoreWebView2ControllerCompletedHandler, ICoreWebView2Environment,
    ICoreWebView2EnvironmentOptions,
    ICoreWebView2NavigationCompletedEventArgs, ICoreWebView2NavigationCompletedEventArgs2,
    ICoreWebView2NavigationStartingEventArgs, ICoreWebView2NewWindowRequestedEventArgs,
    ICoreWebView2Settings3, ICoreWebView2Settings6, ICoreWebView2WebMessageReceivedEventArgs,
};
use webview2_com::{
    AddScriptToExecuteOnDocumentCreatedCompletedHandler, ContentLoadingEventHandler,
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    DOMContentLoadedEventHandler, ExecuteScriptCompletedHandler, FocusChangedEventHandler,
    NavigationCompletedEventHandler,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler,
    WebMessageReceivedEventHandler, take_pwstr,
};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
use windows::core::{BOOL, HSTRING, IUnknown, Interface as _, PCWSTR, PWSTR};

use super::web_view::*;
use crate::paths::{APPLICATION, ORGANISATION, QUALIFIER};

const ENGINE: &str = "WebView2";
const EVERGREEN_INSTALL_URL: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";
const POST_CHANNEL: &str = "window.chrome.webview.postMessage";
const USER_DATA_FOLDER: &str = "WebView2";

const PROGRESS_STARTED: f64 = 0.1;
const PROGRESS_COMMITTED: f64 = 0.5;
const PROGRESS_DOCUMENT_READY: f64 = 0.8;
const PROGRESS_FINISHED: f64 = 1.0;

/// what the event handlers share with the view
struct Shared {
    handlers: HandlerSlot,
    javascript_can_open_windows_automatically: bool,
    current_navigation: Cell<Option<u64>>,
    committed: Cell<bool>,
    ever_committed: Cell<bool>,
    page_focused: Cell<bool>,
}

impl Shared {
    fn emit_progress(&self, estimated_progress: f64, web_view: &ICoreWebView2) {
        self.handlers.emit(WebViewEvent::Progress {
            estimated_progress,
            can_go_back: can_go_back(web_view),
        });
    }
}

/// a call made before the controller existed, run in order when it arrives
enum Pending {
    SetFrame(WebViewFrame),
    SetVisible(bool),
    Load(LoadRequest),
    AddUserScript(UserScript),
    RemoveAllUserScripts,
    EvaluateScript(String),
    Reload,
    GoBack,
    StopLoading,
    SetBackground(WebViewColor),
    Focus,
}

/// registration tokens of the event handlers for the teardown remove calls
struct Tokens {
    navigation_starting: i64,
    frame_navigation_starting: i64,
    content_loading: i64,
    navigation_completed: i64,
    new_window_requested: i64,
    web_message_received: i64,
    dom_content_loaded: Option<i64>,
    got_focus: i64,
    lost_focus: i64,
}

struct Engine {
    controller: ICoreWebView2Controller,
    web_view: ICoreWebView2,
    tokens: Tokens,
}

enum Phase {
    Creating(Vec<Pending>),
    Ready(Engine),
    Gone,
}

/// ids that AddScriptToExecuteOnDocumentCreated answers, for removing all user scripts
/// an id arriving after its generation was removed is removed at once
#[derive(Default)]
struct UserScripts {
    generation: u64,
    ids: Vec<String>,
}

pub(super) struct WindowsWebView {
    shared: Rc<Shared>,
    phase: Rc<RefCell<Phase>>,
    scripts: Rc<RefCell<UserScripts>>,
    destroyed: bool,
}

pub(super) fn create(
    window: &gpui::Window,
    _cx: &mut gpui::App,
    frame: WebViewFrame,
    configuration: &WebViewConfiguration,
    handlers: WebViewHandlers,
) -> Result<Box<dyn WebView>, WebViewUnavailable> {
    let handle =
        HasWindowHandle::window_handle(window).map_err(|_| WebViewUnavailable::NoNativeWindow)?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return Err(WebViewUnavailable::NoNativeWindow);
    };
    let parent = HWND(win32.hwnd.get() as *mut core::ffi::c_void);
    let on_window_thread =
        unsafe { GetWindowThreadProcessId(parent, None) == GetCurrentThreadId() };
    if !on_window_thread {
        return Err(WebViewUnavailable::NotMainThread);
    }
    runtime_version()?;
    let user_data_folder = directories::ProjectDirs::from(QUALIFIER, ORGANISATION, APPLICATION)
        .map(|dirs| dirs.data_local_dir().join(USER_DATA_FOLDER))
        .ok_or_else(|| missing("no local data folder for the WebView2 profile".to_owned()))?;

    let shared = Rc::new(Shared {
        handlers: HandlerSlot::new(handlers),
        javascript_can_open_windows_automatically: configuration
            .javascript_can_open_windows_automatically,
        current_navigation: Cell::new(None),
        committed: Cell::new(false),
        ever_committed: Cell::new(false),
        page_focused: Cell::new(false),
    });
    let phase = Rc::new(RefCell::new(Phase::Creating(vec![
        Pending::SetBackground(configuration.background),
        Pending::SetFrame(frame),
    ])));
    let scripts = Rc::new(RefCell::new(UserScripts::default()));

    let environment_created = {
        let shared = Rc::clone(&shared);
        let phase = Rc::clone(&phase);
        let scripts = Rc::clone(&scripts);
        let configuration = configuration.clone();
        CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
            move |result, environment: Option<ICoreWebView2Environment>| {
                let environment = match (result, environment) {
                    (Ok(()), Some(environment)) => environment,
                    (Err(error), _) => {
                        fail_creation(&shared, &phase, format!("environment: {error}"));
                        return Ok(());
                    }
                    (Ok(()), None) => {
                        fail_creation(&shared, &phase, "environment: none".to_owned());
                        return Ok(());
                    }
                };
                if !matches!(phase.try_borrow().as_deref(), Ok(Phase::Creating(_))) {
                    return Ok(());
                }
                let controller_created = controller_created_handler(
                    Rc::clone(&shared),
                    Rc::clone(&phase),
                    Rc::clone(&scripts),
                    configuration.clone(),
                );
                if let Err(error) =
                    unsafe { environment.CreateCoreWebView2Controller(parent, &controller_created) }
                {
                    fail_creation(&shared, &phase, format!("controller: {error}"));
                }
                Ok(())
            },
        ))
    };
    let folder = HSTRING::from(user_data_folder.as_os_str());
    unsafe {
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            &folder,
            None::<&ICoreWebView2EnvironmentOptions>,
            &environment_created,
        )
    }
    .map_err(|error| missing(format!("CreateCoreWebView2EnvironmentWithOptions: {error}")))?;

    Ok(Box::new(WindowsWebView {
        shared,
        phase,
        scripts,
        destroyed: false,
    }))
}

fn missing(reason: String) -> WebViewUnavailable {
    WebViewUnavailable::EngineMissing {
        engine: ENGINE,
        reason,
        install_url: Some(EVERGREEN_INSTALL_URL),
    }
}

fn runtime_version() -> Result<String, WebViewUnavailable> {
    let mut version = PWSTR::null();
    unsafe { GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut version) }
        .map_err(|error| missing(format!("no WebView2 runtime: {error}")))?;
    let version = take_pwstr(version);
    if version.is_empty() {
        return Err(missing("no WebView2 runtime is installed".to_owned()));
    }
    Ok(version)
}

/// engine couldnt be built after create answered
/// caller sees it as a failed first navigation and the screen shows its error state
fn fail_creation(shared: &Shared, phase: &RefCell<Phase>, reason: String) {
    tracing::warn!(target: "riseonly", "WebView2 creation failed: {reason}");
    if let Ok(mut phase) = phase.try_borrow_mut() {
        *phase = Phase::Gone;
    }
    shared.handlers.emit(WebViewEvent::Failed(NavigationFailure {
        message: reason,
        cancelled: false,
        provisional: true,
    }));
}

fn controller_created_handler(
    shared: Rc<Shared>,
    phase: Rc<RefCell<Phase>>,
    scripts: Rc<RefCell<UserScripts>>,
    configuration: WebViewConfiguration,
) -> ICoreWebView2CreateCoreWebView2ControllerCompletedHandler {
    CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
        move |result, controller: Option<ICoreWebView2Controller>| {
            let controller = match (result, controller) {
                (Ok(()), Some(controller)) => controller,
                (Err(error), _) => {
                    fail_creation(&shared, &phase, format!("controller: {error}"));
                    return Ok(());
                }
                (Ok(()), None) => {
                    fail_creation(&shared, &phase, "controller: none".to_owned());
                    return Ok(());
                }
            };
            let queued = match phase.try_borrow_mut().as_deref_mut() {
                Ok(Phase::Creating(queued)) => Some(std::mem::take(queued)),
                Ok(Phase::Ready(_) | Phase::Gone) | Err(_) => None,
            };
            let Some(queued) = queued else {
                log_failure("Close", unsafe { controller.Close() });
                return Ok(());
            };
            let engine = match build_engine(controller, &shared, &configuration) {
                Ok(engine) => engine,
                Err(error) => {
                    fail_creation(&shared, &phase, format!("set up: {error}"));
                    return Ok(());
                }
            };
            *phase.borrow_mut() = Phase::Ready(engine);
            if let Ok(current) = phase.try_borrow() {
                if let Phase::Ready(engine) = &*current {
                    for call in queued {
                        apply(engine, &shared, &scripts, call);
                    }
                }
            }
            Ok(())
        },
    ))
}

fn build_engine(
    controller: ICoreWebView2Controller,
    shared: &Rc<Shared>,
    configuration: &WebViewConfiguration,
) -> windows::core::Result<Engine> {
    unsafe {
        let web_view = controller.CoreWebView2()?;
        let settings = web_view.Settings()?;
        settings.SetIsScriptEnabled(true)?;
        settings.SetIsWebMessageEnabled(true)?;
        settings.SetAreDevToolsEnabled(false)?;
        settings.SetIsStatusBarEnabled(false)?;
        if let Ok(settings3) = settings.cast::<ICoreWebView2Settings3>() {
            settings3.SetAreBrowserAcceleratorKeysEnabled(false)?;
        }
        if let Ok(settings6) = settings.cast::<ICoreWebView2Settings6>() {
            let swipe = configuration.allows_back_forward_navigation_gestures;
            settings6.SetIsSwipeNavigationEnabled(swipe)?;
        }
        let shim = HSTRING::from(webkit_message_shim(
            &configuration.script_message_handler,
            POST_CHANNEL,
        ));
        web_view.AddScriptToExecuteOnDocumentCreated(&shim, &ignore_script_id())?;

        let (got_focus, lost_focus) = register_focus_tracking(&controller, shared)?;
        let tokens = Tokens {
            got_focus,
            lost_focus,
            navigation_starting: register_navigation_starting(&web_view, shared, false)?,
            frame_navigation_starting: register_navigation_starting(&web_view, shared, true)?,
            content_loading: register_content_loading(&web_view, shared)?,
            navigation_completed: register_navigation_completed(&web_view, shared)?,
            new_window_requested: register_new_window_requested(&web_view, shared)?,
            web_message_received: register_web_message_received(&web_view, shared)?,
            dom_content_loaded: register_dom_content_loaded(&web_view, shared),
        };
        Ok(Engine {
            controller,
            web_view,
            tokens,
        })
    }
}

fn ignore_script_id() -> ICoreWebView2AddScriptToExecuteOnDocumentCreatedCompletedHandler {
    AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(|result, _id| {
        log_failure("AddScriptToExecuteOnDocumentCreated", result);
        Ok(())
    }))
}

/// decides policy for a main frame navigation (subframes false) or for iframes
/// # Safety
/// web_view must be a live ICoreWebView2 of this thread
unsafe fn register_navigation_starting(
    web_view: &ICoreWebView2,
    shared: &Rc<Shared>,
    subframes: bool,
) -> windows::core::Result<i64> {
    let shared = Rc::clone(shared);
    let handler = NavigationStartingEventHandler::create(Box::new(
        move |sender: Option<ICoreWebView2>,
              args: Option<ICoreWebView2NavigationStartingEventArgs>| {
            let (Some(sender), Some(args)) = (sender, args) else {
                return Ok(());
            };
            unsafe { on_navigation_starting(&shared, &sender, &args, subframes) };
            Ok(())
        },
    ));
    let mut token = 0i64;
    unsafe {
        if subframes {
            web_view.add_FrameNavigationStarting(&handler, &mut token)?;
        } else {
            web_view.add_NavigationStarting(&handler, &mut token)?;
        }
    }
    Ok(token)
}

unsafe fn on_navigation_starting(
    shared: &Shared,
    sender: &ICoreWebView2,
    args: &ICoreWebView2NavigationStartingEventArgs,
    subframes: bool,
) {
    unsafe {
        let mut uri = PWSTR::null();
        let url = match args.Uri(&mut uri) {
            Ok(()) => Some(take_pwstr(uri)).filter(|url| !url.is_empty()),
            Err(_) => None,
        };
        let target_frame = if subframes {
            TargetFrame::Subframe
        } else {
            TargetFrame::MainFrame
        };
        let action = NavigationAction {
            url: url.clone(),
            target_frame,
        };
        let policy = shared.handlers.decide(&action).unwrap_or(NavigationPolicy::Cancel);
        let decision = policy.engine_decision();
        if decision.load_in_place {
            if let Some(url) = &url {
                log_failure("Navigate", sender.Navigate(&HSTRING::from(url.as_str())));
            }
        }
        if !decision.allow {
            log_failure("SetCancel", args.SetCancel(true));
            return;
        }
        if subframes {
            return;
        }
        let mut redirected = BOOL::default();
        let redirected = args.IsRedirected(&mut redirected).is_ok() && redirected.as_bool();
        if redirected {
            return;
        }
        let mut navigation_id = 0u64;
        if args.NavigationId(&mut navigation_id).is_ok() {
            shared.current_navigation.set(Some(navigation_id));
        }
        shared.committed.set(false);
        shared.handlers.emit(WebViewEvent::ProvisionalNavigationStarted);
        shared.emit_progress(PROGRESS_STARTED, sender);
    }
}

unsafe fn register_content_loading(
    web_view: &ICoreWebView2,
    shared: &Rc<Shared>,
) -> windows::core::Result<i64> {
    let shared = Rc::clone(shared);
    let handler = ContentLoadingEventHandler::create(Box::new(move |sender, _args| {
        let Some(sender) = sender else {
            return Ok(());
        };
        shared.committed.set(true);
        shared.ever_committed.set(true);
        shared.emit_progress(PROGRESS_COMMITTED, &sender);
        Ok(())
    }));
    let mut token = 0i64;
    unsafe { web_view.add_ContentLoading(&handler, &mut token)? };
    Ok(token)
}

unsafe fn register_dom_content_loaded(
    web_view: &ICoreWebView2,
    shared: &Rc<Shared>,
) -> Option<i64> {
    let web_view2 = web_view.cast::<ICoreWebView2_2>().ok()?;
    let shared = Rc::clone(shared);
    let handler = DOMContentLoadedEventHandler::create(Box::new(move |sender, _args| {
        if let Some(sender) = sender {
            shared.emit_progress(PROGRESS_DOCUMENT_READY, &sender);
        }
        Ok(())
    }));
    let mut token = 0i64;
    unsafe { web_view2.add_DOMContentLoaded(&handler, &mut token) }.ok()?;
    Some(token)
}

unsafe fn register_navigation_completed(
    web_view: &ICoreWebView2,
    shared: &Rc<Shared>,
) -> windows::core::Result<i64> {
    let shared = Rc::clone(shared);
    let handler = NavigationCompletedEventHandler::create(Box::new(
        move |sender: Option<ICoreWebView2>,
              args: Option<ICoreWebView2NavigationCompletedEventArgs>| {
            let (Some(sender), Some(args)) = (sender, args) else {
                return Ok(());
            };
            unsafe { on_navigation_completed(&shared, &sender, &args) };
            Ok(())
        },
    ));
    let mut token = 0i64;
    unsafe { web_view.add_NavigationCompleted(&handler, &mut token)? };
    Ok(token)
}

unsafe fn on_navigation_completed(
    shared: &Shared,
    sender: &ICoreWebView2,
    args: &ICoreWebView2NavigationCompletedEventArgs,
) {
    unsafe {
        let mut navigation_id = 0u64;
        if args.NavigationId(&mut navigation_id).is_ok()
            && shared.current_navigation.get() != Some(navigation_id)
        {
            return;
        }
        shared.current_navigation.set(None);
        let mut success = BOOL::default();
        let success = args.IsSuccess(&mut success).is_ok() && success.as_bool();
        let mut status = COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
        let _ = args.WebErrorStatus(&mut status);
        let http_status = args
            .cast::<ICoreWebView2NavigationCompletedEventArgs2>()
            .ok()
            .and_then(|args2| {
                let mut code = 0i32;
                args2.HttpStatusCode(&mut code).ok().map(|()| code)
            })
            .and_then(|code| u16::try_from(code).ok())
            .filter(|code| *code > 0);
        if http_status.is_some() || success {
            shared.handlers.emit(WebViewEvent::MainFrameResponse {
                status_code: http_status,
            });
        }
        let loaded = success
            || (http_status.is_some() && status == COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN);
        if loaded {
            shared.emit_progress(PROGRESS_FINISHED, sender);
            shared.handlers.emit(WebViewEvent::Finished {
                url: source(sender),
                can_go_back: can_go_back(sender),
            });
        } else {
            shared.handlers.emit(WebViewEvent::Failed(NavigationFailure {
                message: web_error_description(status).to_owned(),
                cancelled: status == COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED,
                provisional: !shared.committed.get(),
            }));
        }
    }
}

unsafe fn register_new_window_requested(
    web_view: &ICoreWebView2,
    shared: &Rc<Shared>,
) -> windows::core::Result<i64> {
    let shared = Rc::clone(shared);
    let handler = NewWindowRequestedEventHandler::create(Box::new(
        move |sender: Option<ICoreWebView2>,
              args: Option<ICoreWebView2NewWindowRequestedEventArgs>| {
            let (Some(sender), Some(args)) = (sender, args) else {
                return Ok(());
            };
            unsafe {
                args.SetHandled(true)?;
                let mut user_initiated = BOOL::default();
                let user_initiated =
                    args.IsUserInitiated(&mut user_initiated).is_ok() && user_initiated.as_bool();
                if !user_initiated && !shared.javascript_can_open_windows_automatically {
                    return Ok(());
                }
                let mut uri = PWSTR::null();
                let url = match args.Uri(&mut uri) {
                    Ok(()) => Some(take_pwstr(uri)).filter(|url| !url.is_empty()),
                    Err(_) => None,
                };
                let action = NavigationAction {
                    url: url.clone(),
                    target_frame: TargetFrame::NewWindow,
                };
                let policy = shared.handlers.decide(&action).unwrap_or(NavigationPolicy::Cancel);
                if policy != NavigationPolicy::Cancel {
                    if let Some(url) = url {
                        log_failure("Navigate", sender.Navigate(&HSTRING::from(url)));
                    }
                }
            }
            Ok(())
        },
    ));
    let mut token = 0i64;
    unsafe { web_view.add_NewWindowRequested(&handler, &mut token)? };
    Ok(token)
}

unsafe fn register_web_message_received(
    web_view: &ICoreWebView2,
    shared: &Rc<Shared>,
) -> windows::core::Result<i64> {
    let shared = Rc::clone(shared);
    let handler = WebMessageReceivedEventHandler::create(Box::new(
        move |_sender, args: Option<ICoreWebView2WebMessageReceivedEventArgs>| {
            let Some(args) = args else {
                return Ok(());
            };
            let (text, origin) = unsafe {
                let mut text = PWSTR::null();
                if args.TryGetWebMessageAsString(&mut text).is_err() {
                    return Ok(());
                }
                let text = take_pwstr(text);
                let mut source = PWSTR::null();
                let origin = match args.Source(&mut source) {
                    Ok(()) => security_origin(&take_pwstr(source)),
                    Err(_) => None,
                };
                (text, origin)
            };
            if let Some(message) = ScriptMessage::from_envelope(&text, true, origin) {
                shared.handlers.emit(WebViewEvent::ScriptMessage(message));
            }
            Ok(())
        },
    ));
    let mut token = 0i64;
    unsafe { web_view.add_WebMessageReceived(&handler, &mut token)? };
    Ok(token)
}

/// scheme, host and port of a document url
/// port is 0 when the url names none since chromium drops a default port
fn security_origin(url: &str) -> Option<SecurityOrigin> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_and_port = authority.rsplit_once('@').map_or(authority, |(_, after)| after);
    let (host, port) = match host_and_port.strip_prefix('[') {
        Some(bracketed) => {
            let (host, after) = bracketed.split_once(']')?;
            (host, after.strip_prefix(':'))
        }
        None => match host_and_port.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_and_port, None),
        },
    };
    let port = match port {
        Some(port) if !port.is_empty() => port.parse::<u16>().ok()?,
        _ => 0,
    };
    Some(SecurityOrigin {
        protocol: scheme.to_ascii_lowercase(),
        host: host.to_ascii_lowercase(),
        port,
    })
}

fn web_error_description(status: COREWEBVIEW2_WEB_ERROR_STATUS) -> &'static str {
    match status {
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT => {
            "certificate common name is incorrect"
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED => "certificate expired",
        COREWEBVIEW2_WEB_ERROR_STATUS_CLIENT_CERTIFICATE_CONTAINS_ERRORS => {
            "client certificate contains errors"
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED => "certificate revoked",
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID => "certificate is invalid",
        COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE => "server unreachable",
        COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT => "the request timed out",
        COREWEBVIEW2_WEB_ERROR_STATUS_ERROR_HTTP_INVALID_SERVER_RESPONSE => {
            "invalid server response"
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED => "connection aborted",
        COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET => "connection reset",
        COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED => "the network connection was lost",
        COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT => "cannot connect to the server",
        COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED => "host name not resolved",
        COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED => "cancelled",
        COREWEBVIEW2_WEB_ERROR_STATUS_REDIRECT_FAILED => "redirect failed",
        COREWEBVIEW2_WEB_ERROR_STATUS_UNEXPECTED_ERROR => "unexpected error",
        COREWEBVIEW2_WEB_ERROR_STATUS_VALID_AUTHENTICATION_CREDENTIALS_REQUIRED => {
            "authentication required"
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_VALID_PROXY_AUTHENTICATION_REQUIRED => {
            "proxy authentication required"
        }
        _ => "the page could not be loaded",
    }
}

fn document_created_source(script: &UserScript) -> String {
    let body = match script.injection_time {
        UserScriptInjectionTime::AtDocumentStart => script.source.clone(),
        UserScriptInjectionTime::AtDocumentEnd => format!(
            "const riseUserScript = () => {{\n{}\n}};\n\
             if (document.readyState === 'loading') {{\n\
             document.addEventListener('DOMContentLoaded', riseUserScript, {{ once: true }});\n\
             }} else {{\n\
             riseUserScript();\n\
             }}",
            script.source
        ),
    };
    if script.for_main_frame_only {
        format!("if (window === window.top) {{\n{body}\n}}")
    } else if script.injection_time == UserScriptInjectionTime::AtDocumentEnd {
        format!("{{\n{body}\n}}")
    } else {
        body
    }
}

fn source(web_view: &ICoreWebView2) -> Option<String> {
    let mut uri = PWSTR::null();
    unsafe { web_view.Source(&mut uri) }.ok()?;
    Some(take_pwstr(uri)).filter(|url| !url.is_empty())
}

fn can_go_back(web_view: &ICoreWebView2) -> bool {
    let mut value = BOOL::default();
    unsafe { web_view.CanGoBack(&mut value) }.is_ok() && value.as_bool()
}

fn log_failure(call: &str, result: windows::core::Result<()>) {
    if let Err(error) = result {
        tracing::debug!(target: "riseonly", "WebView2 {call} failed: {error}");
    }
}

unsafe fn register_focus_tracking(
    controller: &ICoreWebView2Controller,
    shared: &Rc<Shared>,
) -> windows::core::Result<(i64, i64)> {
    let handler = |focused: bool| {
        let shared = Rc::clone(shared);
        FocusChangedEventHandler::create(Box::new(
            move |_: Option<ICoreWebView2Controller>, _: Option<IUnknown>| {
                shared.page_focused.set(focused);
                Ok(())
            },
        ))
    };
    let (mut got_focus, mut lost_focus) = (0i64, 0i64);
    unsafe {
        controller.add_GotFocus(&handler(true), &mut got_focus)?;
        controller.add_LostFocus(&handler(false), &mut lost_focus)?;
    }
    Ok((got_focus, lost_focus))
}

/// a hidden or closed child window holding focus leaves the window with no keys until a click
/// so the keyboard goes back to the parent first
fn return_keyboard_to_parent(controller: &ICoreWebView2Controller, shared: &Shared) {
    if !shared.page_focused.replace(false) {
        return;
    }
    let mut parent = HWND(std::ptr::null_mut());
    unsafe {
        if controller.ParentWindow(&mut parent).is_ok() {
            if let Err(error) = SetFocus(Some(parent)) {
                tracing::debug!(target: "riseonly", "WebView2 SetFocus failed: {error}");
            }
        }
    }
}

fn apply(engine: &Engine, shared: &Shared, scripts: &Rc<RefCell<UserScripts>>, call: Pending) {
    let web_view = &engine.web_view;
    let controller = &engine.controller;
    unsafe {
        match call {
            Pending::SetFrame(frame) => {
                let mut parent = HWND(std::ptr::null_mut());
                if controller.ParentWindow(&mut parent).is_ok() {
                    log_failure("SetBounds", controller.SetBounds(physical_rect(parent, frame)));
                }
            }
            Pending::SetVisible(visible) => {
                if !visible {
                    return_keyboard_to_parent(controller, shared);
                }
                log_failure("SetIsVisible", controller.SetIsVisible(visible));
            }
            Pending::Load(request) => {
                if let Err(error) = web_view.Navigate(&HSTRING::from(request.url.as_str())) {
                    tracing::debug!(target: "riseonly", "WebView2 Navigate failed: {error}");
                    shared.handlers.emit(WebViewEvent::Failed(NavigationFailure {
                        message: format!("unsupported URL: {}", request.url),
                        cancelled: false,
                        provisional: true,
                    }));
                }
            }
            Pending::AddUserScript(script) => add_user_script(web_view, scripts, &script),
            Pending::RemoveAllUserScripts => {
                let ids = match scripts.try_borrow_mut() {
                    Ok(mut scripts) => {
                        scripts.generation += 1;
                        std::mem::take(&mut scripts.ids)
                    }
                    Err(_) => Vec::new(),
                };
                for id in ids {
                    log_failure(
                        "RemoveScriptToExecuteOnDocumentCreated",
                        web_view.RemoveScriptToExecuteOnDocumentCreated(&HSTRING::from(id)),
                    );
                }
            }
            Pending::EvaluateScript(source) => {
                let ignore = ExecuteScriptCompletedHandler::create(Box::new(|result, _json| {
                    log_failure("ExecuteScript", result);
                    Ok(())
                }));
                log_failure(
                    "ExecuteScript",
                    web_view.ExecuteScript(&HSTRING::from(source), &ignore),
                );
            }
            Pending::Reload => log_failure("Reload", web_view.Reload()),
            Pending::GoBack => {
                if can_go_back(web_view) {
                    log_failure("GoBack", web_view.GoBack());
                }
            }
            Pending::StopLoading => log_failure("Stop", web_view.Stop()),
            Pending::SetBackground(color) => {
                if let Ok(controller2) = controller.cast::<ICoreWebView2Controller2>() {
                    log_failure(
                        "SetDefaultBackgroundColor",
                        controller2.SetDefaultBackgroundColor(engine_color(color)),
                    );
                }
            }
            Pending::Focus => log_failure(
                "MoveFocus",
                controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC),
            ),
        }
    }
}

fn add_user_script(
    web_view: &ICoreWebView2,
    scripts: &Rc<RefCell<UserScripts>>,
    script: &UserScript,
) {
    let Ok(generation) = scripts.try_borrow().map(|scripts| scripts.generation) else {
        return;
    };
    let registered = {
        let scripts = Rc::clone(scripts);
        let web_view = web_view.clone();
        AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
            move |result, id: String| {
                if result.is_err() || id.is_empty() {
                    log_failure("AddScriptToExecuteOnDocumentCreated", result);
                    return Ok(());
                }
                let current = match scripts.try_borrow_mut() {
                    Ok(mut scripts) if scripts.generation == generation => {
                        scripts.ids.push(id.clone());
                        true
                    }
                    _ => false,
                };
                if !current {
                    let id = HSTRING::from(id);
                    let removed = unsafe { web_view.RemoveScriptToExecuteOnDocumentCreated(&id) };
                    log_failure("RemoveScriptToExecuteOnDocumentCreated", removed);
                }
                Ok(())
            },
        ))
    };
    let source = HSTRING::from(document_created_source(script));
    let added = unsafe { web_view.AddScriptToExecuteOnDocumentCreated(&source, &registered) };
    log_failure("AddScriptToExecuteOnDocumentCreated", added);
}

fn physical_rect(parent: HWND, frame: WebViewFrame) -> RECT {
    let dpi = unsafe { GetDpiForWindow(parent) };
    let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
    let width = frame.width.max(0.0);
    let height = frame.height.max(0.0);
    RECT {
        left: (frame.x * scale).round() as i32,
        top: (frame.y * scale).round() as i32,
        right: ((frame.x + width) * scale).round() as i32,
        bottom: ((frame.y + height) * scale).round() as i32,
    }
}

fn engine_color(color: WebViewColor) -> COREWEBVIEW2_COLOR {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    COREWEBVIEW2_COLOR {
        A: if color.alpha >= 0.5 { 255 } else { 0 },
        R: channel(color.red),
        G: channel(color.green),
        B: channel(color.blue),
    }
}

impl WindowsWebView {
    /// runs call now when the engine exists, queues it while the engine is being built
    fn run(&mut self, call: Pending) {
        if self.destroyed {
            return;
        }
        {
            let Ok(mut phase) = self.phase.try_borrow_mut() else {
                tracing::debug!(target: "riseonly", "WebView2 call dropped: the view is busy");
                return;
            };
            if let Phase::Creating(queued) = &mut *phase {
                queued.push(call);
                return;
            }
        }
        let Ok(phase) = self.phase.try_borrow() else {
            return;
        };
        if let Phase::Ready(engine) = &*phase {
            apply(engine, &self.shared, &self.scripts, call);
        }
    }

    fn with_engine<R>(&self, read: impl FnOnce(&Engine) -> R) -> Option<R> {
        if self.destroyed {
            return None;
        }
        match &*self.phase.try_borrow().ok()? {
            Phase::Ready(engine) => Some(read(engine)),
            Phase::Creating(_) | Phase::Gone => None,
        }
    }

    fn teardown(engine: &Engine, shared: &Shared, step: TeardownStep) {
        let web_view = &engine.web_view;
        let tokens = &engine.tokens;
        unsafe {
            match step {
                TeardownStep::StopLoading => log_failure("Stop", web_view.Stop()),
                TeardownStep::DetachNavigationDelegate => {
                    shared.handlers.detach();
                    log_failure(
                        "remove_NavigationStarting",
                        web_view.remove_NavigationStarting(tokens.navigation_starting),
                    );
                    log_failure(
                        "remove_FrameNavigationStarting",
                        web_view.remove_FrameNavigationStarting(tokens.frame_navigation_starting),
                    );
                    log_failure(
                        "remove_ContentLoading",
                        web_view.remove_ContentLoading(tokens.content_loading),
                    );
                    log_failure(
                        "remove_NavigationCompleted",
                        web_view.remove_NavigationCompleted(tokens.navigation_completed),
                    );
                }
                TeardownStep::DetachUiDelegate => log_failure(
                    "remove_NewWindowRequested",
                    web_view.remove_NewWindowRequested(tokens.new_window_requested),
                ),
                TeardownStep::RemoveScriptMessageHandler => log_failure(
                    "remove_WebMessageReceived",
                    web_view.remove_WebMessageReceived(tokens.web_message_received),
                ),
                TeardownStep::StopProgressObservation => {
                    if let (Some(token), Ok(web_view2)) =
                        (tokens.dom_content_loaded, web_view.cast::<ICoreWebView2_2>())
                    {
                        log_failure(
                            "remove_DOMContentLoaded",
                            web_view2.remove_DOMContentLoaded(token),
                        );
                    }
                }
            }
        }
    }
}

impl WebView for WindowsWebView {
    fn set_frame(&mut self, frame: WebViewFrame) {
        self.run(Pending::SetFrame(frame));
    }

    fn set_visible(&mut self, visible: bool) {
        self.run(Pending::SetVisible(visible));
    }

    fn load(&mut self, request: &LoadRequest) {
        self.run(Pending::Load(request.clone()));
    }

    fn add_user_script(&mut self, script: &UserScript) {
        self.run(Pending::AddUserScript(script.clone()));
    }

    fn remove_all_user_scripts(&mut self) {
        self.run(Pending::RemoveAllUserScripts);
    }

    fn evaluate_script(&mut self, source: &str) {
        self.run(Pending::EvaluateScript(source.to_owned()));
    }

    fn reload(&mut self) {
        self.run(Pending::Reload);
    }

    fn go_back(&mut self) -> bool {
        if !self.can_go_back() {
            return false;
        }
        self.run(Pending::GoBack);
        true
    }

    fn can_go_back(&self) -> bool {
        self.with_engine(|engine| can_go_back(&engine.web_view)).unwrap_or(false)
    }

    fn stop_loading(&mut self) {
        self.run(Pending::StopLoading);
    }

    fn url(&self) -> Option<String> {
        if !self.shared.ever_committed.get() {
            return None;
        }
        self.with_engine(|engine| source(&engine.web_view)).flatten()
    }

    fn set_background(&mut self, color: WebViewColor) {
        self.run(Pending::SetBackground(color));
    }

    fn focus(&mut self) {
        self.run(Pending::Focus);
    }

    fn destroy(&mut self) {
        if self.destroyed {
            return;
        }
        self.destroyed = true;
        let previous = match self.phase.try_borrow_mut() {
            Ok(mut phase) => std::mem::replace(&mut *phase, Phase::Gone),
            Err(_) => {
                self.shared.handlers.detach();
                return;
            }
        };
        match previous {
            Phase::Ready(engine) => {
                return_keyboard_to_parent(&engine.controller, &self.shared);
                for step in DISMANTLE_ORDER {
                    Self::teardown(&engine, &self.shared, step);
                }
                unsafe {
                    log_failure(
                        "remove_GotFocus",
                        engine.controller.remove_GotFocus(engine.tokens.got_focus),
                    );
                    log_failure(
                        "remove_LostFocus",
                        engine.controller.remove_LostFocus(engine.tokens.lost_focus),
                    );
                }
                log_failure("Close", unsafe { engine.controller.Close() });
            }
            Phase::Creating(_) | Phase::Gone => self.shared.handlers.detach(),
        }
    }
}

impl Drop for WindowsWebView {
    fn drop(&mut self) {
        self.destroy();
    }
}
