use std::cell::{Cell, RefCell};
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::web_view::*;

const HELPER_ENGINE: &str = "WebKitGTK helper";

struct Shared {
    handlers: HandlerSlot,
    stdin: RefCell<Option<ChildStdin>>,
    can_go_back: Cell<bool>,
    url: RefCell<Option<String>>,
}

impl Shared {
    fn send(&self, command: &HelperCommand) {
        let mut stdin = self.stdin.borrow_mut();
        let Some(pipe) = stdin.as_mut() else {
            return;
        };
        let written = encode_helper_line(command)
            .map_err(std::io::Error::other)
            .and_then(|line| {
                pipe.write_all(line.as_bytes())?;
                pipe.flush()
            });
        if let Err(error) = written {
            tracing::warn!(target: "riseonly", %error, "web view helper stopped reading");
            stdin.take();
        }
    }

    fn receive(&self, event: HelperEvent) {
        match event {
            HelperEvent::Ready => {}
            HelperEvent::NavigationQuery { id, action } => {
                let policy = self.handlers.decide(&action).unwrap_or(NavigationPolicy::Cancel);
                self.send(&HelperCommand::NavigationDecision { id, policy });
            }
            HelperEvent::History { can_go_back, url } => {
                self.can_go_back.set(can_go_back);
                *self.url.borrow_mut() = url;
            }
            HelperEvent::View { event } => {
                match &event {
                    WebViewEvent::Progress { can_go_back, .. } => self.can_go_back.set(*can_go_back),
                    WebViewEvent::Finished { url, can_go_back } => {
                        self.can_go_back.set(*can_go_back);
                        *self.url.borrow_mut() = url.clone();
                    }
                    _ => {}
                }
                self.handlers.emit(event);
            }
            HelperEvent::Unavailable { reason } => {
                tracing::warn!(target: "riseonly", %reason, "web view helper unavailable");
                self.handlers.emit(WebViewEvent::Failed(NavigationFailure {
                    message: reason,
                    cancelled: false,
                    provisional: true,
                }));
            }
        }
    }
}

pub(super) struct LinuxWebView {
    shared: Rc<Shared>,
    child: Option<Child>,
    destroyed: bool,
}

fn helper_path() -> Result<PathBuf, WebViewUnavailable> {
    let executable = std::env::current_exe().map_err(|error| WebViewUnavailable::Helper(error.to_string()))?;
    let path = executable
        .parent()
        .map(|directory| directory.join(HELPER_BINARY))
        .ok_or_else(|| WebViewUnavailable::Helper("the executable has no directory".to_owned()))?;
    if !path.is_file() {
        return Err(WebViewUnavailable::EngineMissing {
            engine: HELPER_ENGINE,
            reason: format!("{} is missing", path.display()),
            install_url: None,
        });
    }
    Ok(path)
}

fn parent_of(window: &gpui::Window) -> Result<HelperParent, WebViewUnavailable> {
    let handle = HasWindowHandle::window_handle(window).map_err(|_| WebViewUnavailable::NoNativeWindow)?;
    match handle.as_raw() {
        RawWindowHandle::Xlib(xlib) => Ok(HelperParent::X11 {
            window: u64::from(xlib.window),
        }),
        RawWindowHandle::Xcb(xcb) => Ok(HelperParent::X11 {
            window: u64::from(xcb.window.get()),
        }),
        RawWindowHandle::Wayland(_) => Ok(HelperParent::Wayland),
        _ => Err(WebViewUnavailable::NoNativeWindow),
    }
}

pub(super) fn create(
    window: &gpui::Window,
    cx: &mut gpui::App,
    frame: WebViewFrame,
    configuration: &WebViewConfiguration,
    handlers: WebViewHandlers,
) -> Result<Box<dyn WebView>, WebViewUnavailable> {
    let parent = parent_of(window)?;
    let path = helper_path()?;
    let mut child = Command::new(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| WebViewUnavailable::Helper(error.to_string()))?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        reap(child);
        return Err(WebViewUnavailable::Helper("the helper has no pipes".to_owned()));
    };

    let (sender, mut receiver) = mpsc::unbounded::<HelperEvent>();
    let reader = std::thread::Builder::new()
        .name("web-view-helper".to_owned())
        .spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                match decode_helper_line::<HelperEvent>(&line) {
                    Ok(event) => {
                        if sender.unbounded_send(event).is_err() {
                            break;
                        }
                    }
                    Err(error) => tracing::warn!(target: "riseonly", %error, "unreadable web view helper line"),
                }
            }
        });
    if let Err(error) = reader {
        reap(child);
        return Err(WebViewUnavailable::Helper(error.to_string()));
    }

    let shared = Rc::new(Shared {
        handlers: HandlerSlot::new(handlers),
        stdin: RefCell::new(Some(stdin)),
        can_go_back: Cell::new(false),
        url: RefCell::new(None),
    });
    let delivered = Rc::clone(&shared);
    cx.foreground_executor()
        .spawn(async move {
            while let Some(event) = receiver.next().await {
                delivered.receive(event);
            }
        })
        .detach();

    shared.send(&HelperCommand::Create {
        parent,
        scale_factor: window.scale_factor(),
        frame,
        configuration: configuration.clone(),
    });

    Ok(Box::new(LinuxWebView {
        shared,
        child: Some(child),
        destroyed: false,
    }))
}

/// waits for an exited or exiting helper off the main thread so it doesnt stay a zombie
fn reap(mut child: Child) {
    let waited = std::thread::Builder::new()
        .name("web-view-helper-reap".to_owned())
        .spawn(move || {
            if let Err(error) = child.wait() {
                tracing::warn!(target: "riseonly", %error, "web view helper wait failed");
            }
        });
    if let Err(error) = waited {
        tracing::warn!(target: "riseonly", %error, "web view helper left unreaped");
    }
}

impl LinuxWebView {
    fn send(&self, command: &HelperCommand) {
        if !self.destroyed {
            self.shared.send(command);
        }
    }

    fn teardown(&mut self, step: TeardownStep) {
        match step {
            TeardownStep::StopLoading => self.shared.send(&HelperCommand::StopLoading),
            TeardownStep::DetachNavigationDelegate => self.shared.handlers.detach(),
            TeardownStep::DetachUiDelegate => {}
            TeardownStep::RemoveScriptMessageHandler => self.shared.send(&HelperCommand::Destroy),
            TeardownStep::StopProgressObservation => {
                self.shared.stdin.borrow_mut().take();
                if let Some(child) = self.child.take() {
                    reap(child);
                }
            }
        }
    }
}

impl WebView for LinuxWebView {
    fn set_frame(&mut self, frame: WebViewFrame) {
        self.send(&HelperCommand::SetFrame { frame });
    }

    fn set_visible(&mut self, visible: bool) {
        self.send(&HelperCommand::SetVisible { visible });
    }

    fn load(&mut self, request: &LoadRequest) {
        self.send(&HelperCommand::Load {
            request: request.clone(),
        });
    }

    fn add_user_script(&mut self, script: &UserScript) {
        self.send(&HelperCommand::AddUserScript {
            script: script.clone(),
        });
    }

    fn remove_all_user_scripts(&mut self) {
        self.send(&HelperCommand::RemoveAllUserScripts);
    }

    fn evaluate_script(&mut self, source: &str) {
        self.send(&HelperCommand::EvaluateScript {
            source: source.to_owned(),
        });
    }

    fn reload(&mut self) {
        self.send(&HelperCommand::Reload);
    }

    fn go_back(&mut self) -> bool {
        if self.destroyed || !self.shared.can_go_back.get() {
            return false;
        }
        self.shared.send(&HelperCommand::GoBack);
        true
    }

    fn can_go_back(&self) -> bool {
        !self.destroyed && self.shared.can_go_back.get()
    }

    fn stop_loading(&mut self) {
        self.send(&HelperCommand::StopLoading);
    }

    fn url(&self) -> Option<String> {
        if self.destroyed {
            return None;
        }
        self.shared.url.borrow().clone()
    }

    fn set_background(&mut self, color: WebViewColor) {
        self.send(&HelperCommand::SetBackground { color });
    }

    fn focus(&mut self) {
        self.send(&HelperCommand::Focus);
    }

    fn destroy(&mut self) {
        if self.destroyed {
            return;
        }
        self.destroyed = true;
        for step in DISMANTLE_ORDER {
            self.teardown(step);
        }
    }
}

impl Drop for LinuxWebView {
    fn drop(&mut self) {
        self.destroy();
    }
}
