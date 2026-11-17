use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead as _, Read as _};

use gtk::glib;
use gtk::prelude::*;
use javascriptcore::ValueExt as _;
use crate::web_view as wire;
use webkit2gtk as wk;
use webkit2gtk::{
    NavigationPolicyDecisionExt as _, PolicyDecisionExt as _, ResponsePolicyDecisionExt as _, SettingsExt as _,
    URIRequestExt as _, URIResponseExt as _, UserContentManagerExt as _, WebViewExt,
};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

use super::web_view_helper::*;

enum Input {
    Command(wire::HelperCommand),
    EndOfInput,
    Signal(EngineSignal),
    Policy(wk::PolicyDecision, wk::PolicyDecisionType),
    TimedOut(u64),
}

thread_local! {
    static HELPER: RefCell<Option<Helper>> = const { RefCell::new(None) };
    static INBOX: RefCell<VecDeque<Input>> = const { RefCell::new(VecDeque::new()) };
    static PUMPING: Cell<bool> = const { Cell::new(false) };
    static MAIN_LOOP_RUNNING: Cell<bool> = const { Cell::new(false) };
    static QUITTING: Cell<bool> = const { Cell::new(false) };
}

fn post(input: Input) {
    INBOX.with(|inbox| inbox.borrow_mut().push_back(input));
    pump();
}

/// handles queued inputs until the queue is empty, a call from inside a handler only queues
fn pump() {
    if PUMPING.with(|pumping| pumping.replace(true)) {
        return;
    }
    loop {
        if QUITTING.with(Cell::get) {
            break;
        }
        let ready = HELPER.with(|helper| helper.try_borrow().is_ok_and(|helper| helper.is_some()));
        if !ready {
            break;
        }
        let Some(input) = INBOX.with(|inbox| inbox.borrow_mut().pop_front()) else {
            break;
        };
        let flow = HELPER.with(|helper| match helper.borrow_mut().as_mut() {
            Some(helper) => helper.handle(input),
            None => Flow::Exit,
        });
        if flow == Flow::Exit {
            quit();
            break;
        }
    }
    PUMPING.with(|pumping| pumping.set(false));
}

fn quit() {
    QUITTING.with(|quitting| quitting.set(true));
    if MAIN_LOOP_RUNNING.with(Cell::get) {
        gtk::main_quit();
    }
}

struct Held {
    decision: wk::PolicyDecision,
    request: Option<wk::URIRequest>,
}

/// the view reparented into the app x11 window
struct X11Embed {
    connection: RustConnection,
    window: u32,
}

struct Gtk {
    window: gtk::Window,
    view: wk::WebView,
    content: wk::UserContentManager,
    main_frame_script: wk::UserScript,
    embed: Option<X11Embed>,
    held: HashMap<u64, Held>,
}

struct Helper {
    session: Session,
    engine: Gtk,
}

impl Helper {
    fn handle(&mut self, input: Input) -> Flow {
        match input {
            Input::Command(command) => {
                let mut timers = Vec::new();
                let flow = self
                    .session
                    .command(&mut self.engine, command, &mut |timer| timers.push(timer));
                for timer in timers {
                    glib::timeout_add_local_once(timer.after, move || post(Input::TimedOut(timer.generation)));
                }
                flow
            }
            Input::EndOfInput => {
                self.session.shut_down(&mut self.engine);
                Flow::Exit
            }
            Input::Signal(signal) => {
                let events = self.session.signal(signal);
                send_all(&events)
            }
            Input::Policy(decision, kind) => self.policy(decision, kind),
            Input::TimedOut(generation) => match self.session.timed_out(&mut self.engine, generation) {
                Some(event) => send_all(&[event]),
                None => Flow::Continue,
            },
        }
    }

    fn policy(&mut self, decision: wk::PolicyDecision, kind: wk::PolicyDecisionType) -> Flow {
        match kind {
            wk::PolicyDecisionType::Response => {
                let (status_code, main_frame) = decision
                    .downcast_ref::<wk::ResponsePolicyDecision>()
                    .map_or((0, false), |response| {
                        (
                            response.response().map_or(0, |answer| answer.status_code()),
                            response.is_main_frame_main_resource(),
                        )
                    });
                let events = self.session.signal(EngineSignal::Response {
                    status_code,
                    main_frame,
                });
                decision.use_();
                send_all(&events)
            }
            wk::PolicyDecisionType::NavigationAction | wk::PolicyDecisionType::NewWindowAction => {
                let request = decision
                    .downcast_ref::<wk::NavigationPolicyDecision>()
                    .and_then(|navigation| navigation.navigation_action())
                    .and_then(|mut action| action.request());
                let url = request.as_ref().and_then(|request| request.uri()).map(|uri| uri.to_string());
                let target_frame = if kind == wk::PolicyDecisionType::NewWindowAction {
                    wire::TargetFrame::NewWindow
                } else {
                    wire::TargetFrame::MainFrame
                };
                let (id, event) = self.session.query(wire::NavigationAction { url, target_frame });
                self.engine.held.insert(id, Held { decision, request });
                send_all(&[event])
            }
            _ => {
                decision.use_();
                Flow::Continue
            }
        }
    }
}

fn send_all(events: &[wire::HelperEvent]) -> Flow {
    for event in events {
        if !send_to_app(event) {
            return Flow::Exit;
        }
    }
    Flow::Continue
}

impl Engine for Gtk {
    fn toolkit_scale(&self) -> i32 {
        self.window.scale_factor()
    }

    fn place(&mut self, placement: Placement) {
        self.window.resize(placement.width, placement.height);
        self.view.set_zoom_level(placement.zoom);
        if let Some(embed) = &self.embed {
            let geometry = ConfigureWindowAux::new()
                .x(placement.x)
                .y(placement.y)
                .width(placement.physical_width)
                .height(placement.physical_height);
            let configured = embed
                .connection
                .configure_window(embed.window, &geometry)
                .map(drop)
                .and_then(|()| embed.connection.flush());
            if let Err(error) = configured {
                tracing::warn!(target: "riseonly", %error, "web view helper could not move its window");
            }
        }
    }

    fn set_visible(&mut self, visible: bool) {
        if visible {
            self.window.show_all();
        } else {
            self.window.hide();
        }
    }

    fn load(&mut self, request: &wire::LoadRequest) {
        self.view.load_uri(&request.url);
    }

    fn add_user_script(&mut self, script: &wire::UserScript) {
        let frames = if script.for_main_frame_only {
            wk::UserContentInjectedFrames::TopFrame
        } else {
            wk::UserContentInjectedFrames::AllFrames
        };
        let time = match script.injection_time {
            wire::UserScriptInjectionTime::AtDocumentStart => wk::UserScriptInjectionTime::Start,
            wire::UserScriptInjectionTime::AtDocumentEnd => wk::UserScriptInjectionTime::End,
        };
        let script = wk::UserScript::new(&script.source, frames, time, &[], &[]);
        self.content.add_script(&script);
    }

    fn remove_all_user_scripts(&mut self) {
        self.content.remove_all_scripts();
        self.content.add_script(&self.main_frame_script);
    }

    fn evaluate_script(&mut self, source: &str) {
        self.view
            .evaluate_javascript(source, None, None, None::<&wk::gio::Cancellable>, |result| {
                if let Err(error) = result {
                    tracing::debug!(target: "riseonly", %error, "web view script failed");
                }
            });
    }

    fn reload(&mut self) {
        self.view.reload();
    }

    fn go_back(&mut self) {
        self.view.go_back();
    }

    fn can_go_back(&self) -> bool {
        self.view.can_go_back()
    }

    fn stop_loading(&mut self) {
        self.view.stop_loading();
    }

    fn set_background(&mut self, color: wire::WebViewColor) {
        self.view.set_background_color(&rgba(color));
    }

    fn focus(&mut self) {
        self.window.present();
        self.view.grab_focus();
    }

    fn decide(&mut self, id: u64, decision: wire::EngineDecision) {
        let Some(held) = self.held.remove(&id) else {
            return;
        };
        if decision.load_in_place {
            if let Some(request) = &held.request {
                self.view.load_request(request);
            }
        }
        if decision.allow {
            held.decision.use_();
        } else {
            held.decision.ignore();
        }
    }
}

fn rgba(color: wire::WebViewColor) -> gtk::gdk::RGBA {
    gtk::gdk::RGBA::new(
        f64::from(color.red),
        f64::from(color.green),
        f64::from(color.blue),
        f64::from(color.alpha),
    )
}

fn connect_view(view: &wk::WebView, content: &wk::UserContentManager, handler_name: &str, token: String) {
    view.connect_load_changed(|view, event| {
        let url = || view.uri().map(|uri| uri.to_string());
        let signal = match event {
            wk::LoadEvent::Started => EngineSignal::Started,
            wk::LoadEvent::Committed => EngineSignal::Committed {
                url: url(),
                can_go_back: view.can_go_back(),
            },
            wk::LoadEvent::Finished => EngineSignal::Finished {
                url: url(),
                can_go_back: view.can_go_back(),
            },
            _ => return,
        };
        post(Input::Signal(signal));
    });
    view.connect_load_failed(|_, event, _, error| {
        post(Input::Signal(EngineSignal::Failed(wire::NavigationFailure {
            message: error.message().to_owned(),
            cancelled: error.matches(wk::NetworkError::Cancelled),
            provisional: matches!(event, wk::LoadEvent::Started | wk::LoadEvent::Redirected),
        })));
        true
    });
    view.connect_estimated_load_progress_notify(|view| {
        post(Input::Signal(EngineSignal::Progress {
            estimated_progress: view.estimated_load_progress(),
            can_go_back: view.can_go_back(),
        }));
    });
    view.connect_decide_policy(|_, decision, kind| {
        post(Input::Policy(decision.clone(), kind));
        true
    });
    let page = view.downgrade();
    let name = handler_name.to_owned();
    content.connect_script_message_received(Some(handler_name), move |_, result| {
        let Some(body_json) = result
            .js_value()
            .and_then(|value| value.to_json(0))
            .and_then(|json| main_frame_script_body(json.as_str(), &token))
        else {
            return;
        };
        let page_url = page.upgrade().and_then(|view| view.uri()).map(|uri| uri.to_string());
        post(Input::Signal(EngineSignal::Script {
            name: name.clone(),
            body_json: Some(body_json),
            page_url,
        }));
    });
}

fn embed(window: &gtk::Window, parent: wire::HelperParent) -> Result<Option<X11Embed>, String> {
    let wire::HelperParent::X11 { window: parent } = parent else {
        return Ok(None);
    };
    let parent = u32::try_from(parent).map_err(|_| format!("X11 window {parent} is out of range"))?;
    window.realize();
    let child = window
        .window()
        .ok_or_else(|| "the helper window has no GDK window".to_owned())?
        .downcast::<gdkx11::X11Window>()
        .map_err(|_| "the helper window is not an X11 window".to_owned())?
        .xid();
    let child = u32::try_from(child).map_err(|_| format!("X11 window {child} is out of range"))?;
    let (connection, _) = x11rb::connect(None).map_err(|error| error.to_string())?;
    connection
        .reparent_window(child, parent, 0, 0)
        .map(drop)
        .and_then(|()| connection.flush())
        .map_err(|error| error.to_string())?;
    Ok(Some(X11Embed {
        connection,
        window: child,
    }))
}

fn create(
    parent: wire::HelperParent,
    scale_factor: f32,
    frame: wire::WebViewFrame,
    configuration: &wire::WebViewConfiguration,
) -> Result<Helper, String> {
    gtk::gdk::set_allowed_backends(match parent {
        wire::HelperParent::X11 { .. } => "x11",
        wire::HelperParent::Wayland => "wayland",
    });
    gtk::init().map_err(|error| error.to_string())?;

    let content = wk::UserContentManager::new();
    if !content.register_script_message_handler(&configuration.script_message_handler) {
        return Err(format!(
            "the script message handler {} could not be registered",
            configuration.script_message_handler
        ));
    }
    let view = wk::WebView::with_user_content_manager(&content);
    if let Some(settings) = WebViewExt::settings(&view) {
        settings.set_javascript_can_open_windows_automatically(configuration.javascript_can_open_windows_automatically);
        settings.set_enable_back_forward_navigation_gestures(configuration.allows_back_forward_navigation_gestures);
    }
    view.set_background_color(&rgba(configuration.background));
    let mut random = [0_u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut random))
        .map_err(|error| error.to_string())?;
    let token = main_frame_token(&random);
    let main_frame_script = wk::UserScript::new(
        &main_frame_script(&configuration.script_message_handler, &token),
        wk::UserContentInjectedFrames::TopFrame,
        wk::UserScriptInjectionTime::Start,
        &[],
        &[],
    );
    content.add_script(&main_frame_script);
    connect_view(&view, &content, &configuration.script_message_handler, token);

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_decorated(false);
    window.add(&view);
    let embed = embed(&window, parent)?;

    let mut engine = Gtk {
        window,
        view,
        content,
        main_frame_script,
        embed,
        held: HashMap::new(),
    };
    let session = Session::new(parent, scale_factor);
    session.place(&mut engine, frame);
    engine.window.show_all();
    Ok(Helper { session, engine })
}

fn spawn_input() -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("web-view-helper-input".to_owned())
        .spawn(|| {
            let context = glib::MainContext::default();
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else {
                    break;
                };
                match wire::decode_helper_line::<wire::HelperCommand>(&line) {
                    Ok(command) => context.invoke(move || post(Input::Command(command))),
                    Err(error) => tracing::warn!(target: "riseonly", %error, "unreadable web view command"),
                }
            }
            context.invoke(|| post(Input::EndOfInput));
        })
        .map(drop)
}

pub(crate) fn run() -> i32 {
    let mut first = String::new();
    match std::io::stdin().read_line(&mut first) {
        Ok(0) => return 0,
        Ok(_) => {}
        Err(error) => return unavailable(error.to_string()),
    }
    let Ok(wire::HelperCommand::Create {
        parent,
        scale_factor,
        frame,
        configuration,
    }) = wire::decode_helper_line(&first)
    else {
        return unavailable("the first command was not create");
    };
    let helper = match create(parent, scale_factor, frame, &configuration) {
        Ok(helper) => helper,
        Err(reason) => return unavailable(reason),
    };
    HELPER.with(|slot| *slot.borrow_mut() = Some(helper));
    if !send_to_app(&wire::HelperEvent::Ready) {
        return 0;
    }
    if let Err(error) = spawn_input() {
        return unavailable(error.to_string());
    }
    pump();
    if !QUITTING.with(Cell::get) {
        MAIN_LOOP_RUNNING.with(|running| running.set(true));
        gtk::main();
        MAIN_LOOP_RUNNING.with(|running| running.set(false));
    }
    QUITTING.with(|quitting| quitting.set(true));
    HELPER.with(|slot| slot.borrow_mut().take());
    0
}
