use std::ffi::{c_int, c_void};
use std::sync::OnceLock;

use objc2::encode::{Encoding, RefEncode};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, Bool, NSObject};
use objc2::{define_class, msg_send, sel, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::web_view::*;

#[link(name = "WebKit", kind = "framework")]
unsafe extern "C" {}

const POLICY_CANCEL: isize = 0;
const POLICY_ALLOW: isize = 1;
const INJECT_AT_DOCUMENT_START: isize = 0;
const INJECT_AT_DOCUMENT_END: isize = 1;
const USE_PROTOCOL_CACHE_POLICY: usize = 0;
const OBSERVE_NEW: usize = 1;
const URL_ERROR_CANCELLED: isize = -999;

const PROGRESS_KEY: &str = "estimatedProgress";

/// head of a block object of the Block ABI
/// webkit decision handlers are called through invoke
/// nil goes where a completion handler is optional
#[repr(C)]
struct BlockLiteral {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: *const c_void,
}

unsafe impl RefEncode for BlockLiteral {
    const ENCODING_REF: Encoding = Encoding::Block;
}

/// calls a void decision handler taking an NSInteger, once
/// # Safety
/// block must be the live decision handler webkit passed to the running delegate method
unsafe fn decide(block: *mut BlockLiteral, policy: isize) {
    if block.is_null() {
        return;
    }
    unsafe {
        let invoke: unsafe extern "C" fn(*mut BlockLiteral, isize) = std::mem::transmute((*block).invoke);
        invoke(block, policy);
    }
}

fn webkit_class(name: &'static std::ffi::CStr) -> Result<&'static AnyClass, WebViewUnavailable> {
    AnyClass::get(name).ok_or_else(|| WebViewUnavailable::EngineMissing {
        engine: "WebKit",
        reason: format!("{} is not registered", name.to_string_lossy()),
        install_url: None,
    })
}

struct DelegateIvars {
    handlers: HandlerSlot,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "RiseWebViewDelegate"]
    #[ivars = DelegateIvars]
    struct WebViewDelegate;

    impl WebViewDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide_action(&self, web_view: &AnyObject, action: &AnyObject, handler: *mut BlockLiteral) {
            let navigation = read_navigation_action(action);
            let policy = self.ivars().handlers.decide(&navigation).unwrap_or(NavigationPolicy::Cancel);
            let decision = policy.engine_decision();
            if decision.load_in_place {
                unsafe {
                    let request: Option<Retained<AnyObject>> = msg_send![action, request];
                    if let Some(request) = request {
                        let _: Option<Retained<AnyObject>> = msg_send![web_view, loadRequest: &*request];
                    }
                }
            }
            let answer = if decision.allow { POLICY_ALLOW } else { POLICY_CANCEL };
            unsafe { decide(handler, answer) };
        }

        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn did_start_provisional(&self, _web_view: &AnyObject, _navigation: Option<&AnyObject>) {
            self.ivars().handlers.emit(WebViewEvent::ProvisionalNavigationStarted);
        }

        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn decide_response(&self, _web_view: &AnyObject, response: &AnyObject, handler: *mut BlockLiteral) {
            let main_frame: bool = unsafe { msg_send![response, isForMainFrame] };
            if main_frame {
                let status_code = read_status_code(response);
                self.ivars().handlers.emit(WebViewEvent::MainFrameResponse { status_code });
            }
            unsafe { decide(handler, POLICY_ALLOW) };
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish(&self, web_view: &AnyObject, _navigation: Option<&AnyObject>) {
            self.ivars().handlers.emit(WebViewEvent::Finished {
                url: current_url(web_view),
                can_go_back: can_go_back(web_view),
            });
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn did_fail(&self, _web_view: &AnyObject, _navigation: Option<&AnyObject>, error: &AnyObject) {
            self.ivars().handlers.emit(WebViewEvent::Failed(read_failure(error, false)));
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn did_fail_provisional(&self, _web_view: &AnyObject, _navigation: Option<&AnyObject>, error: &AnyObject) {
            self.ivars().handlers.emit(WebViewEvent::Failed(read_failure(error, true)));
        }

        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(
            &self,
            web_view: &AnyObject,
            _configuration: &AnyObject,
            action: &AnyObject,
            _features: &AnyObject,
        ) -> Option<Retained<AnyObject>> {
            let mut navigation = read_navigation_action(action);
            navigation.target_frame = TargetFrame::NewWindow;
            let policy = self.ivars().handlers.decide(&navigation).unwrap_or(NavigationPolicy::Cancel);
            if policy != NavigationPolicy::Cancel {
                unsafe {
                    let request: Option<Retained<AnyObject>> = msg_send![action, request];
                    if let Some(request) = request {
                        let _: Option<Retained<AnyObject>> = msg_send![web_view, loadRequest: &*request];
                    }
                }
            }
            None
        }

        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive(&self, _controller: &AnyObject, message: &AnyObject) {
            if let Some(message) = read_script_message(message) {
                self.ivars().handlers.emit(WebViewEvent::ScriptMessage(message));
            }
        }

        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            key_path: Option<&NSString>,
            object: Option<&AnyObject>,
            _change: Option<&AnyObject>,
            _context: *mut c_void,
        ) {
            let (Some(key_path), Some(web_view)) = (key_path, object) else {
                return;
            };
            if key_path.to_string() != PROGRESS_KEY {
                return;
            }
            let estimated_progress: f64 = unsafe { msg_send![web_view, estimatedProgress] };
            self.ivars().handlers.emit(WebViewEvent::Progress {
                estimated_progress,
                can_go_back: can_go_back(web_view),
            });
        }
    }
);

static CONFORMANCE: OnceLock<()> = OnceLock::new();

impl WebViewDelegate {
    fn new(mtm: MainThreadMarker, handlers: WebViewHandlers) -> Retained<Self> {
        declare_conformance();
        let this = Self::alloc(mtm).set_ivars(DelegateIvars {
            handlers: HandlerSlot::new(handlers),
        });
        unsafe { msg_send![super(this), init] }
    }
}

fn declare_conformance() {
    CONFORMANCE.get_or_init(|| {
        let class: *const AnyClass = WebViewDelegate::class();
        for name in [c"WKNavigationDelegate", c"WKUIDelegate", c"WKScriptMessageHandler"] {
            let Some(protocol) = AnyProtocol::get(name) else {
                tracing::warn!(target: "riseonly", protocol = ?name, "WebKit protocol is not registered");
                continue;
            };
            unsafe {
                objc2::ffi::class_addProtocol(class.cast_mut(), protocol);
            }
        }
    });
}

fn ns_string(object: Option<Retained<NSString>>) -> Option<String> {
    object.map(|string| string.to_string())
}

fn url_string(url: Option<&AnyObject>) -> Option<String> {
    let url = url?;
    ns_string(unsafe { msg_send![url, absoluteString] })
}

fn current_url(web_view: &AnyObject) -> Option<String> {
    let url: Option<Retained<AnyObject>> = unsafe { msg_send![web_view, URL] };
    url_string(url.as_deref())
}

fn can_go_back(web_view: &AnyObject) -> bool {
    unsafe { msg_send![web_view, canGoBack] }
}

fn read_navigation_action(action: &AnyObject) -> NavigationAction {
    unsafe {
        let request: Option<Retained<AnyObject>> = msg_send![action, request];
        let url = request.and_then(|request| {
            let url: Option<Retained<AnyObject>> = msg_send![&*request, URL];
            url_string(url.as_deref())
        });
        let target: Option<Retained<AnyObject>> = msg_send![action, targetFrame];
        let target_frame = match target {
            None => TargetFrame::NewWindow,
            Some(frame) => {
                let main: bool = msg_send![&*frame, isMainFrame];
                if main { TargetFrame::MainFrame } else { TargetFrame::Subframe }
            }
        };
        NavigationAction { url, target_frame }
    }
}

fn read_status_code(response: &AnyObject) -> Option<u16> {
    let http_class = AnyClass::get(c"NSHTTPURLResponse")?;
    unsafe {
        let response: Option<Retained<AnyObject>> = msg_send![response, response];
        let response = response?;
        let is_http: bool = msg_send![&*response, isKindOfClass: http_class];
        if !is_http {
            return None;
        }
        let status: isize = msg_send![&*response, statusCode];
        u16::try_from(status).ok()
    }
}

fn read_failure(error: &AnyObject, provisional: bool) -> NavigationFailure {
    unsafe {
        let code: isize = msg_send![error, code];
        let domain: Option<Retained<NSString>> = msg_send![error, domain];
        let message: Option<Retained<NSString>> = msg_send![error, localizedDescription];
        let cancelled = code == URL_ERROR_CANCELLED
            && domain.is_some_and(|domain| domain.to_string() == "NSURLErrorDomain");
        NavigationFailure {
            message: ns_string(message).unwrap_or_default(),
            cancelled,
            provisional,
        }
    }
}

fn read_security_origin(frame: &AnyObject) -> Option<SecurityOrigin> {
    unsafe {
        let origin: Option<Retained<AnyObject>> = msg_send![frame, securityOrigin];
        let origin = origin?;
        let protocol: Option<Retained<NSString>> = msg_send![&*origin, protocol];
        let host: Option<Retained<NSString>> = msg_send![&*origin, host];
        let port: isize = msg_send![&*origin, port];
        Some(SecurityOrigin {
            protocol: ns_string(protocol).unwrap_or_default(),
            host: ns_string(host).unwrap_or_default(),
            port: u16::try_from(port).unwrap_or(0),
        })
    }
}

/// body of a script message is a plist object
/// gets wrapped in a one element array and checked with isValidJSONObject
/// invalid ones raise, then the element is read back
fn body_json(body: &AnyObject) -> Option<serde_json::Value> {
    let array_class = AnyClass::get(c"NSArray")?;
    let json_class = AnyClass::get(c"NSJSONSerialization")?;
    let bytes = unsafe {
        let wrapped: Option<Retained<AnyObject>> = msg_send![array_class, arrayWithObject: body];
        let wrapped = wrapped?;
        let valid: bool = msg_send![json_class, isValidJSONObject: &*wrapped];
        if !valid {
            return None;
        }
        let no_error: *mut *mut AnyObject = std::ptr::null_mut();
        let data: Option<Retained<AnyObject>> =
            msg_send![json_class, dataWithJSONObject: &*wrapped, options: 0usize, error: no_error];
        let data = data?;
        let length: usize = msg_send![&*data, length];
        let start: *const c_void = msg_send![&*data, bytes];
        if start.is_null() || length == 0 {
            return None;
        }
        std::slice::from_raw_parts(start.cast::<u8>(), length).to_vec()
    };
    match serde_json::from_slice::<serde_json::Value>(&bytes).ok()? {
        serde_json::Value::Array(mut items) if items.len() == 1 => items.pop(),
        _ => None,
    }
}

fn read_script_message(message: &AnyObject) -> Option<ScriptMessage> {
    unsafe {
        let name: Option<Retained<NSString>> = msg_send![message, name];
        let body: Option<Retained<AnyObject>> = msg_send![message, body];
        let frame: Option<Retained<AnyObject>> = msg_send![message, frameInfo];
        let frame = frame?;
        let is_main_frame: bool = msg_send![&*frame, isMainFrame];
        Some(ScriptMessage {
            name: ns_string(name)?,
            is_main_frame,
            security_origin: read_security_origin(&frame),
            body: body.and_then(|body| body_json(&body)).unwrap_or(serde_json::Value::Null),
        })
    }
}

fn ns_color(color: WebViewColor) -> Option<Retained<AnyObject>> {
    let class = AnyClass::get(c"NSColor")?;
    unsafe {
        msg_send![
            class,
            colorWithSRGBRed: f64::from(color.red),
            green: f64::from(color.green),
            blue: f64::from(color.blue),
            alpha: f64::from(color.alpha)
        ]
    }
}

pub(super) struct MacWebView {
    view: Retained<AnyObject>,
    parent: Retained<AnyObject>,
    controller: Retained<AnyObject>,
    delegate: Retained<WebViewDelegate>,
    handler_name: Retained<NSString>,
    observing: bool,
    destroyed: bool,
}

pub(super) fn create(
    window: &gpui::Window,
    _cx: &mut gpui::App,
    frame: WebViewFrame,
    configuration: &WebViewConfiguration,
    handlers: WebViewHandlers,
) -> Result<Box<dyn WebView>, WebViewUnavailable> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err(WebViewUnavailable::NotMainThread);
    };
    let handle = HasWindowHandle::window_handle(window).map_err(|_| WebViewUnavailable::NoNativeWindow)?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return Err(WebViewUnavailable::NoNativeWindow);
    };

    let configuration_class = webkit_class(c"WKWebViewConfiguration")?;
    let controller_class = webkit_class(c"WKUserContentController")?;
    let store_class = webkit_class(c"WKWebsiteDataStore")?;
    let view_class = webkit_class(c"WKWebView")?;
    let missing = |what: &str| WebViewUnavailable::EngineMissing {
        engine: "WebKit",
        reason: format!("{what} returned nil"),
        install_url: None,
    };

    let delegate = WebViewDelegate::new(mtm, handlers);
    let handler_name = NSString::from_str(&configuration.script_message_handler);

    let (view, parent, controller) = unsafe {
        let parent: Retained<AnyObject> =
            Retained::retain(appkit.ns_view.cast::<AnyObject>().as_ptr()).ok_or(WebViewUnavailable::NoNativeWindow)?;

        let web_configuration: Option<Retained<AnyObject>> = msg_send![configuration_class, new];
        let web_configuration = web_configuration.ok_or_else(|| missing("WKWebViewConfiguration new"))?;
        let controller: Option<Retained<AnyObject>> = msg_send![controller_class, new];
        let controller = controller.ok_or_else(|| missing("WKUserContentController new"))?;
        let _: () = msg_send![&*controller, addScriptMessageHandler: &*delegate, name: &*handler_name];
        let _: () = msg_send![&*web_configuration, setUserContentController: &*controller];
        let store: Option<Retained<AnyObject>> = msg_send![store_class, defaultDataStore];
        if let Some(store) = store {
            let _: () = msg_send![&*web_configuration, setWebsiteDataStore: &*store];
        }
        let preferences: Option<Retained<AnyObject>> = msg_send![&*web_configuration, preferences];
        if let Some(preferences) = preferences {
            let _: () = msg_send![
                &*preferences,
                setJavaScriptCanOpenWindowsAutomatically: Bool::new(configuration.javascript_can_open_windows_automatically)
            ];
        }

        let allocated: Allocated<AnyObject> = msg_send![view_class, alloc];
        let view: Option<Retained<AnyObject>> =
            msg_send![allocated, initWithFrame: NSRect::ZERO, configuration: &*web_configuration];
        let view = view.ok_or_else(|| missing("WKWebView initWithFrame:configuration:"))?;
        let _: () = msg_send![&*view, setNavigationDelegate: &*delegate];
        let _: () = msg_send![&*view, setUIDelegate: &*delegate];
        let _: () = msg_send![
            &*view,
            setAllowsBackForwardNavigationGestures: Bool::new(configuration.allows_back_forward_navigation_gestures)
        ];
        let draws_background: bool = msg_send![&*view, respondsToSelector: sel!(_setDrawsBackground:)];
        if draws_background {
            let number_class = AnyClass::get(c"NSNumber");
            if let Some(number_class) = number_class {
                let no: Option<Retained<AnyObject>> = msg_send![number_class, numberWithBool: Bool::NO];
                if let Some(no) = no {
                    let key = NSString::from_str("drawsBackground");
                    let _: () = msg_send![&*view, setValue: &*no, forKey: &*key];
                }
            }
        }
        let key = NSString::from_str(PROGRESS_KEY);
        let _: () = msg_send![
            &*view,
            addObserver: &*delegate,
            forKeyPath: &*key,
            options: OBSERVE_NEW,
            context: std::ptr::null_mut::<c_void>()
        ];
        let _: () = msg_send![&*parent, addSubview: &*view];
        (view, parent, controller)
    };

    let mut web_view = MacWebView {
        view,
        parent,
        controller,
        delegate,
        handler_name,
        observing: true,
        destroyed: false,
    };
    web_view.set_background(configuration.background);
    web_view.set_frame(frame);
    Ok(Box::new(web_view))
}

impl MacWebView {
    fn native_rect(&self, frame: WebViewFrame) -> NSRect {
        let (flipped, bounds): (bool, NSRect) =
            unsafe { (msg_send![&*self.parent, isFlipped], msg_send![&*self.parent, bounds]) };
        let width = f64::from(frame.width.max(0.0));
        let height = f64::from(frame.height.max(0.0));
        let y = if flipped {
            f64::from(frame.y)
        } else {
            bounds.size.height - f64::from(frame.y) - height
        };
        NSRect::new(NSPoint::new(f64::from(frame.x), y), NSSize::new(width, height))
    }

    /// gives the first responder back to the gpui view when the page holds it
    /// a hidden or removed responder moves on to the next key view
    /// then gpui gets no keyDown until a click
    fn hand_first_responder_to_parent(&self) {
        let Some(view_class) = AnyClass::get(c"NSView") else {
            return;
        };
        unsafe {
            let window: Option<Retained<AnyObject>> = msg_send![&*self.view, window];
            let Some(window) = window else {
                return;
            };
            let responder: Option<Retained<AnyObject>> = msg_send![&*window, firstResponder];
            let Some(responder) = responder else {
                return;
            };
            let is_view: bool = msg_send![&*responder, isKindOfClass: view_class];
            if !is_view {
                return;
            }
            let is_inside_page: bool = msg_send![&*responder, isDescendantOf: &*self.view];
            if is_inside_page {
                let _: bool = msg_send![&*window, makeFirstResponder: &*self.parent];
            }
        }
    }

    fn teardown(&mut self, step: TeardownStep) {
        let nil: Option<&AnyObject> = None;
        unsafe {
            match step {
                TeardownStep::StopLoading => {
                    let _: () = msg_send![&*self.view, stopLoading];
                }
                TeardownStep::DetachNavigationDelegate => {
                    self.delegate.ivars().handlers.detach();
                    let _: () = msg_send![&*self.view, setNavigationDelegate: nil];
                }
                TeardownStep::DetachUiDelegate => {
                    let _: () = msg_send![&*self.view, setUIDelegate: nil];
                }
                TeardownStep::RemoveScriptMessageHandler => {
                    let _: () = msg_send![&*self.controller, removeScriptMessageHandlerForName: &*self.handler_name];
                }
                TeardownStep::StopProgressObservation => {
                    if self.observing {
                        self.observing = false;
                        let key = NSString::from_str(PROGRESS_KEY);
                        let _: () = msg_send![&*self.view, removeObserver: &*self.delegate, forKeyPath: &*key];
                    }
                }
            }
        }
    }
}

impl WebView for MacWebView {
    fn set_frame(&mut self, frame: WebViewFrame) {
        if self.destroyed {
            return;
        }
        let rect = self.native_rect(frame);
        unsafe {
            let _: () = msg_send![&*self.view, setFrame: rect];
        }
    }

    fn set_visible(&mut self, visible: bool) {
        if self.destroyed {
            return;
        }
        if !visible {
            self.hand_first_responder_to_parent();
        }
        unsafe {
            let _: () = msg_send![&*self.view, setHidden: Bool::new(!visible)];
        }
    }

    fn load(&mut self, request: &LoadRequest) {
        if self.destroyed {
            return;
        }
        let (Some(url_class), Some(request_class)) = (AnyClass::get(c"NSURL"), AnyClass::get(c"NSURLRequest")) else {
            return;
        };
        let started = unsafe {
            let text = NSString::from_str(&request.url);
            let url: Option<Retained<AnyObject>> = msg_send![url_class, URLWithString: &*text];
            match url {
                Some(url) => {
                    let native: Option<Retained<AnyObject>> = msg_send![
                        request_class,
                        requestWithURL: &*url,
                        cachePolicy: USE_PROTOCOL_CACHE_POLICY,
                        timeoutInterval: request.timeout_seconds
                    ];
                    match native {
                        Some(native) => {
                            let _: Option<Retained<AnyObject>> = msg_send![&*self.view, loadRequest: &*native];
                            true
                        }
                        None => false,
                    }
                }
                None => false,
            }
        };
        if !started {
            self.delegate.ivars().handlers.emit(WebViewEvent::Failed(NavigationFailure {
                message: format!("unsupported URL: {}", request.url),
                cancelled: false,
                provisional: true,
            }));
        }
    }

    fn add_user_script(&mut self, script: &UserScript) {
        if self.destroyed {
            return;
        }
        let Some(class) = AnyClass::get(c"WKUserScript") else {
            return;
        };
        let injection_time = match script.injection_time {
            UserScriptInjectionTime::AtDocumentStart => INJECT_AT_DOCUMENT_START,
            UserScriptInjectionTime::AtDocumentEnd => INJECT_AT_DOCUMENT_END,
        };
        unsafe {
            let source = NSString::from_str(&script.source);
            let allocated: Allocated<AnyObject> = msg_send![class, alloc];
            let user_script: Option<Retained<AnyObject>> = msg_send![
                allocated,
                initWithSource: &*source,
                injectionTime: injection_time,
                forMainFrameOnly: Bool::new(script.for_main_frame_only)
            ];
            if let Some(user_script) = user_script {
                let _: () = msg_send![&*self.controller, addUserScript: &*user_script];
            }
        }
    }

    fn remove_all_user_scripts(&mut self) {
        if self.destroyed {
            return;
        }
        unsafe {
            let _: () = msg_send![&*self.controller, removeAllUserScripts];
        }
    }

    fn evaluate_script(&mut self, source: &str) {
        if self.destroyed {
            return;
        }
        unsafe {
            let source = NSString::from_str(source);
            let _: () = msg_send![
                &*self.view,
                evaluateJavaScript: &*source,
                completionHandler: std::ptr::null_mut::<BlockLiteral>()
            ];
        }
    }

    fn reload(&mut self) {
        if self.destroyed {
            return;
        }
        unsafe {
            let _: Option<Retained<AnyObject>> = msg_send![&*self.view, reload];
        }
    }

    fn go_back(&mut self) -> bool {
        if self.destroyed || !can_go_back(&self.view) {
            return false;
        }
        unsafe {
            let _: Option<Retained<AnyObject>> = msg_send![&*self.view, goBack];
        }
        true
    }

    fn can_go_back(&self) -> bool {
        !self.destroyed && can_go_back(&self.view)
    }

    fn stop_loading(&mut self) {
        if self.destroyed {
            return;
        }
        self.teardown(TeardownStep::StopLoading);
    }

    fn url(&self) -> Option<String> {
        if self.destroyed {
            return None;
        }
        current_url(&self.view)
    }

    fn set_background(&mut self, color: WebViewColor) {
        if self.destroyed {
            return;
        }
        let Some(color) = ns_color(color) else {
            return;
        };
        unsafe {
            let answers: bool = msg_send![&*self.view, respondsToSelector: sel!(setUnderPageBackgroundColor:)];
            if answers {
                let _: () = msg_send![&*self.view, setUnderPageBackgroundColor: &*color];
            }
        }
    }

    fn focus(&mut self) {
        if self.destroyed {
            return;
        }
        unsafe {
            let window: Option<Retained<AnyObject>> = msg_send![&*self.view, window];
            if let Some(window) = window {
                let _: bool = msg_send![&*window, makeFirstResponder: &*self.view];
            }
        }
    }

    fn destroy(&mut self) {
        if self.destroyed {
            return;
        }
        self.destroyed = true;
        for step in DISMANTLE_ORDER {
            self.teardown(step);
        }
        self.hand_first_responder_to_parent();
        unsafe {
            let _: () = msg_send![&*self.view, removeFromSuperview];
        }
    }
}

impl Drop for MacWebView {
    fn drop(&mut self) {
        self.destroy();
    }
}
