use std::collections::BTreeSet;
use std::io::Write as _;
use std::time::Duration;

use crate::web_view::{
    EngineDecision, HelperCommand, HelperEvent, HelperParent, LoadRequest, NavigationAction, NavigationFailure,
    ScriptMessage, SecurityOrigin, UserScript, WebViewColor, WebViewEvent, WebViewFrame, encode_helper_line,
};

pub fn run_helper() -> i32 {
    super::os::run()
}

pub const TIMED_OUT_MESSAGE: &str = "The request timed out.";

/// where the engine window goes, in the units each windowing system takes
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub physical_width: u32,
    pub physical_height: u32,
    pub zoom: f64,
}

/// place of frame in gpui logical points for a gpui window of scale_factor
/// and a gtk window of integer toolkit_scale
pub fn placement(parent: HelperParent, scale_factor: f32, toolkit_scale: i32, frame: WebViewFrame) -> Placement {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        f64::from(scale_factor)
    } else {
        1.0
    };
    let toolkit = f64::from(toolkit_scale.max(1));
    let length = |value: f32| {
        if value.is_finite() {
            f64::from(value)
        } else {
            0.0
        }
    };
    match parent {
        HelperParent::X11 { .. } => {
            let physical_width = (length(frame.width) * scale).round().max(1.0);
            let physical_height = (length(frame.height) * scale).round().max(1.0);
            Placement {
                x: (length(frame.x) * scale).round() as i32,
                y: (length(frame.y) * scale).round() as i32,
                width: (physical_width / toolkit).round().max(1.0) as i32,
                height: (physical_height / toolkit).round().max(1.0) as i32,
                physical_width: physical_width as u32,
                physical_height: physical_height as u32,
                zoom: scale / toolkit,
            }
        }
        HelperParent::Wayland => {
            let width = length(frame.width).round().max(1.0);
            let height = length(frame.height).round().max(1.0);
            Placement {
                x: 0,
                y: 0,
                width: width as i32,
                height: height as i32,
                physical_width: (width * scale).round() as u32,
                physical_height: (height * scale).round() as u32,
                zoom: 1.0,
            }
        }
    }
}

/// http status of a main frame response
/// none if its not http, webkitgtk gives 0 for file and data urls
pub fn http_status(status_code: u32) -> Option<u16> {
    (100..=999).contains(&status_code).then_some(status_code as u16)
}

/// origin of a page url, scheme and host and explicit port
/// 0 for the default
pub fn security_origin(url: &str) -> Option<SecurityOrigin> {
    let (protocol, rest) = url.split_once("://")?;
    if protocol.is_empty() {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, tail) = bracketed.split_once(']')?;
        (host, tail.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    let port = match port {
        Some(port) if !port.is_empty() => port.parse::<u16>().ok()?,
        _ => 0,
    };
    Some(SecurityOrigin {
        protocol: protocol.to_ascii_lowercase(),
        host: host.to_ascii_lowercase(),
        port,
    })
}

/// what the engine reports before it becomes a HelperEvent
#[derive(Clone, Debug, PartialEq)]
pub enum EngineSignal {
    Started,
    Committed { url: Option<String>, can_go_back: bool },
    Finished { url: Option<String>, can_go_back: bool },
    Failed(NavigationFailure),
    Progress { estimated_progress: f64, can_go_back: bool },
    Response { status_code: u32, main_frame: bool },
    Script {
        name: String,
        body_json: Option<String>,
        page_url: Option<String>,
    },
}

/// the engine the session drives
/// linux.rs implements it over webkitgtk and tests over a fake
pub trait Engine {
    /// integer scale of the toolkit window (gtk scale factor)
    fn toolkit_scale(&self) -> i32;
    fn place(&mut self, placement: Placement);
    fn set_visible(&mut self, visible: bool);
    fn load(&mut self, request: &LoadRequest);
    fn add_user_script(&mut self, script: &UserScript);
    fn remove_all_user_scripts(&mut self);
    fn evaluate_script(&mut self, source: &str);
    fn reload(&mut self);
    fn go_back(&mut self);
    fn can_go_back(&self) -> bool;
    fn stop_loading(&mut self);
    fn set_background(&mut self, color: WebViewColor);
    fn focus(&mut self);
    /// settles the navigation held for query id
    fn decide(&mut self, id: u64, decision: EngineDecision);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}

/// a load timeout to arm, call Session timed_out with generation after after
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoadTimer {
    pub generation: u64,
    pub after: Duration,
}

/// one view state between the app and the engine
pub struct Session {
    parent: HelperParent,
    scale_factor: f32,
    next_query: u64,
    held: BTreeSet<u64>,
    load_generation: u64,
    provisional: bool,
    failed: bool,
    destroyed: bool,
}

impl Session {
    pub fn new(parent: HelperParent, scale_factor: f32) -> Self {
        Self {
            parent,
            scale_factor,
            next_query: 1,
            held: BTreeSet::new(),
            load_generation: 0,
            provisional: false,
            failed: false,
            destroyed: false,
        }
    }

    pub fn parent(&self) -> HelperParent {
        self.parent
    }

    pub fn place(&self, engine: &mut dyn Engine, frame: WebViewFrame) {
        engine.place(placement(self.parent, self.scale_factor, engine.toolkit_scale(), frame));
    }

    /// holds a navigation the engine asked about, the app answers with NavigationDecision
    pub fn query(&mut self, action: NavigationAction) -> (u64, HelperEvent) {
        let id = self.next_query;
        self.next_query += 1;
        self.held.insert(id);
        (id, HelperEvent::NavigationQuery { id, action })
    }

    /// runs one command, timer gets a load timeout to arm
    pub fn command(
        &mut self,
        engine: &mut dyn Engine,
        command: HelperCommand,
        timer: &mut dyn FnMut(LoadTimer),
    ) -> Flow {
        if self.destroyed {
            return Flow::Exit;
        }
        match command {
            HelperCommand::Create { .. } => {
                tracing::warn!(target: "riseonly", "web view helper got a second create");
            }
            HelperCommand::SetFrame { frame } => self.place(engine, frame),
            HelperCommand::SetVisible { visible } => engine.set_visible(visible),
            HelperCommand::Load { request } => {
                self.load_generation += 1;
                self.provisional = true;
                engine.load(&request);
                if request.timeout_seconds.is_finite() && request.timeout_seconds > 0.0 {
                    timer(LoadTimer {
                        generation: self.load_generation,
                        after: Duration::from_secs_f64(request.timeout_seconds.min(86_400.0)),
                    });
                }
            }
            HelperCommand::AddUserScript { script } => engine.add_user_script(&script),
            HelperCommand::RemoveAllUserScripts => engine.remove_all_user_scripts(),
            HelperCommand::EvaluateScript { source } => engine.evaluate_script(&source),
            HelperCommand::Reload => engine.reload(),
            HelperCommand::GoBack => {
                if engine.can_go_back() {
                    engine.go_back();
                }
            }
            HelperCommand::StopLoading => engine.stop_loading(),
            HelperCommand::SetBackground { color } => engine.set_background(color),
            HelperCommand::Focus => engine.focus(),
            HelperCommand::NavigationDecision { id, policy } => {
                if self.held.remove(&id) {
                    engine.decide(id, policy.engine_decision());
                }
            }
            HelperCommand::Destroy => {
                self.shut_down(engine);
                return Flow::Exit;
            }
        }
        Flow::Continue
    }

    /// the app is gone (Destroy or end of input), stop loading and refuse every held navigation
    pub fn shut_down(&mut self, engine: &mut dyn Engine) {
        if self.destroyed {
            return;
        }
        self.destroyed = true;
        engine.stop_loading();
        for id in std::mem::take(&mut self.held) {
            engine.decide(
                id,
                EngineDecision {
                    load_in_place: false,
                    allow: false,
                },
            );
        }
    }

    /// load of generation ran out of time, stops it if still provisional
    pub fn timed_out(&mut self, engine: &mut dyn Engine, generation: u64) -> Option<HelperEvent> {
        if self.destroyed || generation != self.load_generation || !self.provisional {
            return None;
        }
        self.provisional = false;
        self.failed = true;
        engine.stop_loading();
        Some(HelperEvent::View {
            event: WebViewEvent::Failed(NavigationFailure {
                message: TIMED_OUT_MESSAGE.to_owned(),
                cancelled: false,
                provisional: true,
            }),
        })
    }

    /// the events an engine report becomes, in the order the coordinator gets them
    pub fn signal(&mut self, signal: EngineSignal) -> Vec<HelperEvent> {
        if self.destroyed {
            return Vec::new();
        }
        match signal {
            EngineSignal::Started => {
                self.provisional = true;
                self.failed = false;
                vec![HelperEvent::View {
                    event: WebViewEvent::ProvisionalNavigationStarted,
                }]
            }
            EngineSignal::Committed { url, can_go_back } => {
                self.provisional = false;
                vec![HelperEvent::History { can_go_back, url }]
            }
            EngineSignal::Finished { url, can_go_back } => {
                self.provisional = false;
                if std::mem::take(&mut self.failed) {
                    return vec![HelperEvent::History { can_go_back, url }];
                }
                vec![
                    HelperEvent::History {
                        can_go_back,
                        url: url.clone(),
                    },
                    HelperEvent::View {
                        event: WebViewEvent::Finished { url, can_go_back },
                    },
                ]
            }
            EngineSignal::Failed(failure) => {
                self.provisional = false;
                self.failed = true;
                vec![HelperEvent::View {
                    event: WebViewEvent::Failed(failure),
                }]
            }
            EngineSignal::Progress {
                estimated_progress,
                can_go_back,
            } => vec![HelperEvent::View {
                event: WebViewEvent::Progress {
                    estimated_progress: estimated_progress.clamp(0.0, 1.0),
                    can_go_back,
                },
            }],
            EngineSignal::Response {
                status_code,
                main_frame,
            } => {
                if !main_frame {
                    return Vec::new();
                }
                vec![HelperEvent::View {
                    event: WebViewEvent::MainFrameResponse {
                        status_code: http_status(status_code),
                    },
                }]
            }
            EngineSignal::Script {
                name,
                body_json,
                page_url,
            } => {
                let body = body_json
                    .and_then(|json| serde_json::from_str(&json).ok())
                    .unwrap_or(serde_json::Value::Null);
                vec![HelperEvent::View {
                    event: WebViewEvent::ScriptMessage(ScriptMessage {
                        name,
                        is_main_frame: true,
                        security_origin: page_url.as_deref().and_then(security_origin),
                        body,
                    }),
                }]
            }
        }
    }
}

/// token of one helper process in the form the scripts carry
pub fn main_frame_token(random: &[u8; 16]) -> String {
    random.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// document start script for the top frame only
/// webkitgtk shows messageHandlers to subframes too and names no frame in the message
/// so each top frame body is wrapped with a token kept in a closure
pub fn main_frame_script(handler_name: &str, token: &str) -> String {
    let name = serde_json::Value::String(handler_name.to_owned()).to_string();
    let token = serde_json::Value::String(token.to_owned()).to_string();
    format!(
        "(() => {{\n\
         const name = {name};\n\
         const token = {token};\n\
         const handler = window.webkit.messageHandlers[name];\n\
         const send = handler.postMessage.bind(handler);\n\
         const postMessage = (body) => send({{ token, body: body === undefined ? null : body }});\n\
         const handlers = Object.freeze({{ [name]: Object.freeze({{ postMessage }}) }});\n\
         const value = Object.freeze({{ messageHandlers: handlers }});\n\
         Object.defineProperty(window, \"webkit\", {{ value, writable: false, configurable: false }});\n\
         }})();"
    )
}

/// the page body from what main_frame_script posted, None for anything not from the top frame
pub fn main_frame_script_body(posted_json: &str, token: &str) -> Option<String> {
    let serde_json::Value::Object(mut posted) = serde_json::from_str(posted_json).ok()? else {
        return None;
    };
    if posted.get("token")?.as_str()? != token {
        return None;
    }
    Some(posted.remove("body")?.to_string())
}

/// writes one event line to the app, false when the app stopped reading
pub fn send_event(out: &mut dyn std::io::Write, event: &HelperEvent) -> bool {
    let written = encode_helper_line(event)
        .map_err(std::io::Error::other)
        .and_then(|line| {
            out.write_all(line.as_bytes())?;
            out.flush()
        });
    if let Err(error) = written {
        tracing::warn!(target: "riseonly", %error, "the app stopped reading the web view helper");
        return false;
    }
    true
}

/// send_event to the process standard output
pub fn send_to_app(event: &HelperEvent) -> bool {
    send_event(&mut std::io::stdout().lock(), event)
}

/// tells the app the view cant exist and returns the exit code
pub fn unavailable(reason: impl Into<String>) -> i32 {
    send_to_app(&HelperEvent::Unavailable { reason: reason.into() });
    1
}
