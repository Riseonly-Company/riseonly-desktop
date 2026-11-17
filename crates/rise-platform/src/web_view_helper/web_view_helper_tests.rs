use std::time::Duration;

use crate::web_view::*;
use crate::web_view_helper::*;

#[derive(Debug, PartialEq)]
enum Call {
    Place(Placement),
    Visible(bool),
    Load(String),
    AddScript(String),
    RemoveScripts,
    Evaluate(String),
    Reload,
    GoBack,
    Stop,
    Background(WebViewColor),
    Focus,
    Decide(u64, EngineDecision),
}

#[derive(Default)]
struct FakeEngine {
    calls: Vec<Call>,
    can_go_back: bool,
    toolkit_scale: i32,
}

impl Engine for FakeEngine {
    fn toolkit_scale(&self) -> i32 {
        self.toolkit_scale.max(1)
    }
    fn place(&mut self, placement: Placement) {
        self.calls.push(Call::Place(placement));
    }
    fn set_visible(&mut self, visible: bool) {
        self.calls.push(Call::Visible(visible));
    }
    fn load(&mut self, request: &LoadRequest) {
        self.calls.push(Call::Load(request.url.clone()));
    }
    fn add_user_script(&mut self, script: &UserScript) {
        self.calls.push(Call::AddScript(script.source.clone()));
    }
    fn remove_all_user_scripts(&mut self) {
        self.calls.push(Call::RemoveScripts);
    }
    fn evaluate_script(&mut self, source: &str) {
        self.calls.push(Call::Evaluate(source.to_owned()));
    }
    fn reload(&mut self) {
        self.calls.push(Call::Reload);
    }
    fn go_back(&mut self) {
        self.calls.push(Call::GoBack);
    }
    fn can_go_back(&self) -> bool {
        self.can_go_back
    }
    fn stop_loading(&mut self) {
        self.calls.push(Call::Stop);
    }
    fn set_background(&mut self, color: WebViewColor) {
        self.calls.push(Call::Background(color));
    }
    fn focus(&mut self) {
        self.calls.push(Call::Focus);
    }
    fn decide(&mut self, id: u64, decision: EngineDecision) {
        self.calls.push(Call::Decide(id, decision));
    }
}

fn session() -> Session {
    Session::new(HelperParent::X11 { window: 7 }, 1.0)
}

fn run(session: &mut Session, engine: &mut FakeEngine, command: HelperCommand) -> (Flow, Vec<LoadTimer>) {
    let mut timers = Vec::new();
    let flow = session.command(engine, command, &mut |timer| timers.push(timer));
    (flow, timers)
}

fn load(url: &str, timeout_seconds: f64) -> HelperCommand {
    HelperCommand::Load {
        request: LoadRequest {
            url: url.to_owned(),
            timeout_seconds,
        },
    }
}

const ALLOW: EngineDecision = EngineDecision {
    load_in_place: false,
    allow: true,
};
const CANCEL: EngineDecision = EngineDecision {
    load_in_place: false,
    allow: false,
};

#[test]
fn a_navigation_runs_from_load_to_finish() {
    let mut session = session();
    let mut engine = FakeEngine::default();
    let (flow, timers) = run(&mut session, &mut engine, load("https://app.example/", 30.0));
    assert_eq!(flow, Flow::Continue);
    assert_eq!(
        timers,
        vec![LoadTimer {
            generation: 1,
            after: Duration::from_secs(30)
        }]
    );
    assert_eq!(engine.calls, vec![Call::Load("https://app.example/".to_owned())]);

    let (id, query) = session.query(NavigationAction {
        url: Some("https://app.example/".to_owned()),
        target_frame: TargetFrame::MainFrame,
    });
    assert_eq!(
        query,
        HelperEvent::NavigationQuery {
            id,
            action: NavigationAction {
                url: Some("https://app.example/".to_owned()),
                target_frame: TargetFrame::MainFrame,
            },
        }
    );
    run(
        &mut session,
        &mut engine,
        HelperCommand::NavigationDecision {
            id,
            policy: NavigationPolicy::Allow,
        },
    );
    run(
        &mut session,
        &mut engine,
        HelperCommand::NavigationDecision {
            id,
            policy: NavigationPolicy::Cancel,
        },
    );
    run(
        &mut session,
        &mut engine,
        HelperCommand::NavigationDecision {
            id: 99,
            policy: NavigationPolicy::Allow,
        },
    );
    assert_eq!(engine.calls[1..], [Call::Decide(id, ALLOW)]);

    assert_eq!(
        session.signal(EngineSignal::Started),
        vec![HelperEvent::View {
            event: WebViewEvent::ProvisionalNavigationStarted
        }]
    );
    assert_eq!(
        session.signal(EngineSignal::Response {
            status_code: 200,
            main_frame: true
        }),
        vec![HelperEvent::View {
            event: WebViewEvent::MainFrameResponse { status_code: Some(200) }
        }]
    );
    assert!(
        session
            .signal(EngineSignal::Response {
                status_code: 200,
                main_frame: false
            })
            .is_empty()
    );
    assert_eq!(
        session.signal(EngineSignal::Committed {
            url: Some("https://app.example/".to_owned()),
            can_go_back: false
        }),
        vec![HelperEvent::History {
            can_go_back: false,
            url: Some("https://app.example/".to_owned())
        }]
    );
    assert_eq!(
        session.signal(EngineSignal::Progress {
            estimated_progress: 1.5,
            can_go_back: false
        }),
        vec![HelperEvent::View {
            event: WebViewEvent::Progress {
                estimated_progress: 1.0,
                can_go_back: false
            }
        }]
    );
    let finished = session.signal(EngineSignal::Finished {
        url: Some("https://app.example/home".to_owned()),
        can_go_back: true,
    });
    assert_eq!(
        finished.last(),
        Some(&HelperEvent::View {
            event: WebViewEvent::Finished {
                url: Some("https://app.example/home".to_owned()),
                can_go_back: true
            }
        })
    );
    assert_eq!(session.timed_out(&mut engine, 1), None);
}

#[test]
fn a_new_window_request_loads_in_place() {
    let mut session = session();
    let mut engine = FakeEngine::default();
    let (id, _) = session.query(NavigationAction {
        url: Some("https://app.example/pay".to_owned()),
        target_frame: TargetFrame::NewWindow,
    });
    run(
        &mut session,
        &mut engine,
        HelperCommand::NavigationDecision {
            id,
            policy: NavigationPolicy::LoadInPlace,
        },
    );
    assert_eq!(
        engine.calls,
        vec![Call::Decide(
            id,
            EngineDecision {
                load_in_place: true,
                allow: false
            }
        )]
    );
}

#[test]
fn a_failed_load_does_not_finish() {
    let mut session = session();
    session.signal(EngineSignal::Started);
    let failure = NavigationFailure {
        message: "Could not resolve host".to_owned(),
        cancelled: false,
        provisional: true,
    };
    assert_eq!(
        session.signal(EngineSignal::Failed(failure.clone())),
        vec![HelperEvent::View {
            event: WebViewEvent::Failed(failure)
        }]
    );
    let after = session.signal(EngineSignal::Finished {
        url: None,
        can_go_back: false,
    });
    assert!(after.iter().all(|event| !matches!(event, HelperEvent::View { .. })));
    session.signal(EngineSignal::Started);
    let next = session.signal(EngineSignal::Finished {
        url: Some("https://app.example/".to_owned()),
        can_go_back: false,
    });
    assert!(next.iter().any(|event| matches!(
        event,
        HelperEvent::View {
            event: WebViewEvent::Finished { .. }
        }
    )));
}

#[test]
fn a_load_times_out_only_while_provisional() {
    let mut session = session();
    let mut engine = FakeEngine::default();
    run(&mut session, &mut engine, load("https://slow.example/", 10.0));
    run(&mut session, &mut engine, load("https://app.example/", 10.0));
    assert_eq!(session.timed_out(&mut engine, 1), None);
    assert_eq!(
        session.timed_out(&mut engine, 2),
        Some(HelperEvent::View {
            event: WebViewEvent::Failed(NavigationFailure {
                message: TIMED_OUT_MESSAGE.to_owned(),
                cancelled: false,
                provisional: true,
            })
        })
    );
    assert_eq!(engine.calls.last(), Some(&Call::Stop));
    assert_eq!(session.timed_out(&mut engine, 2), None);

    let (_, timers) = run(&mut session, &mut engine, load("https://app.example/", 0.0));
    assert!(timers.is_empty());
    let (_, timers) = run(&mut session, &mut engine, load("https://app.example/", f64::NAN));
    assert!(timers.is_empty());
}

#[test]
fn script_messages_carry_the_body_and_the_page_origin() {
    let mut session = session();
    assert_eq!(
        session.signal(EngineSignal::Script {
            name: "riseonlyWebApp".to_owned(),
            body_json: Some("{\"eventType\":\"web_app_ready\"}".to_owned()),
            page_url: Some("https://App.Example:8443/start?x=1".to_owned()),
        }),
        vec![HelperEvent::View {
            event: WebViewEvent::ScriptMessage(ScriptMessage {
                name: "riseonlyWebApp".to_owned(),
                is_main_frame: true,
                security_origin: Some(SecurityOrigin {
                    protocol: "https".to_owned(),
                    host: "app.example".to_owned(),
                    port: 8443,
                }),
                body: serde_json::json!({ "eventType": "web_app_ready" }),
            })
        }]
    );
    let undefined = session.signal(EngineSignal::Script {
        name: "riseonlyWebApp".to_owned(),
        body_json: None,
        page_url: None,
    });
    let [
        HelperEvent::View {
            event: WebViewEvent::ScriptMessage(message),
        },
    ] = undefined.as_slice()
    else {
        panic!("one script message expected");
    };
    assert_eq!(message.body, serde_json::Value::Null);
    assert_eq!(message.security_origin, None);
}

#[test]
fn commands_reach_the_engine() {
    let mut session = session();
    let mut engine = FakeEngine::default();
    let color = WebViewColor {
        red: 1.0,
        green: 0.5,
        blue: 0.0,
        alpha: 1.0,
    };
    for command in [
        HelperCommand::SetVisible { visible: false },
        HelperCommand::AddUserScript {
            script: UserScript {
                source: "window.a = 1;".to_owned(),
                injection_time: UserScriptInjectionTime::AtDocumentStart,
                for_main_frame_only: true,
            },
        },
        HelperCommand::RemoveAllUserScripts,
        HelperCommand::EvaluateScript {
            source: "window.b = 2;".to_owned(),
        },
        HelperCommand::Reload,
        HelperCommand::GoBack,
        HelperCommand::StopLoading,
        HelperCommand::SetBackground { color },
        HelperCommand::Focus,
    ] {
        assert_eq!(run(&mut session, &mut engine, command).0, Flow::Continue);
    }
    assert_eq!(
        engine.calls,
        vec![
            Call::Visible(false),
            Call::AddScript("window.a = 1;".to_owned()),
            Call::RemoveScripts,
            Call::Evaluate("window.b = 2;".to_owned()),
            Call::Reload,
            Call::Stop,
            Call::Background(color),
            Call::Focus,
        ]
    );
    engine.can_go_back = true;
    run(&mut session, &mut engine, HelperCommand::GoBack);
    assert_eq!(engine.calls.last(), Some(&Call::GoBack));
}

#[test]
fn destroy_refuses_held_navigations_and_ends_the_session() {
    let mut session = session();
    let mut engine = FakeEngine::default();
    let (first, _) = session.query(NavigationAction {
        url: None,
        target_frame: TargetFrame::MainFrame,
    });
    let (second, _) = session.query(NavigationAction {
        url: Some("https://app.example/".to_owned()),
        target_frame: TargetFrame::Subframe,
    });
    assert_ne!(first, second);
    assert_eq!(run(&mut session, &mut engine, HelperCommand::Destroy).0, Flow::Exit);
    assert_eq!(
        engine.calls,
        vec![Call::Stop, Call::Decide(first, CANCEL), Call::Decide(second, CANCEL)]
    );
    assert_eq!(run(&mut session, &mut engine, HelperCommand::Reload).0, Flow::Exit);
    assert!(session.signal(EngineSignal::Started).is_empty());
    session.shut_down(&mut engine);
    assert_eq!(engine.calls.len(), 3);
}

#[test]
fn x11_placement_is_in_parent_pixels_with_the_fractional_scale_as_zoom() {
    let frame = WebViewFrame {
        x: 10.0,
        y: 20.0,
        width: 300.0,
        height: 200.5,
    };
    let placed = placement(HelperParent::X11 { window: 1 }, 1.5, 1, frame);
    assert_eq!((placed.x, placed.y), (15, 30));
    assert_eq!((placed.physical_width, placed.physical_height), (450, 301));
    assert_eq!((placed.width, placed.height), (450, 301));
    assert_eq!(placed.zoom, 1.5);

    let doubled = placement(HelperParent::X11 { window: 1 }, 2.0, 2, frame);
    assert_eq!((doubled.width, doubled.height), (300, 201));
    assert_eq!(doubled.zoom, 1.0);

    let empty = placement(
        HelperParent::X11 { window: 1 },
        f32::NAN,
        0,
        WebViewFrame {
            x: f32::INFINITY,
            ..WebViewFrame::default()
        },
    );
    assert_eq!((empty.x, empty.width, empty.height, empty.zoom), (0, 1, 1, 1.0));
}

#[test]
fn wayland_placement_is_a_toplevel_of_the_frame_size() {
    let frame = WebViewFrame {
        x: 10.0,
        y: 20.0,
        width: 300.0,
        height: 200.0,
    };
    let placed = placement(HelperParent::Wayland, 2.0, 2, frame);
    assert_eq!((placed.x, placed.y, placed.width, placed.height), (0, 0, 300, 200));
    assert_eq!((placed.physical_width, placed.physical_height), (600, 400));
    assert_eq!(placed.zoom, 1.0);
}

#[test]
fn set_frame_places_with_the_session_scale() {
    let mut session = Session::new(HelperParent::X11 { window: 3 }, 2.0);
    let mut engine = FakeEngine {
        toolkit_scale: 1,
        ..FakeEngine::default()
    };
    let frame = WebViewFrame {
        x: 1.0,
        y: 2.0,
        width: 3.0,
        height: 4.0,
    };
    run(&mut session, &mut engine, HelperCommand::SetFrame { frame });
    assert_eq!(
        engine.calls,
        vec![Call::Place(placement(HelperParent::X11 { window: 3 }, 2.0, 1, frame))]
    );
}

#[test]
fn security_origins_read_like_webkit() {
    let origin = |protocol: &str, host: &str, port: u16| SecurityOrigin {
        protocol: protocol.to_owned(),
        host: host.to_owned(),
        port,
    };
    assert_eq!(
        security_origin("https://app.example/path"),
        Some(origin("https", "app.example", 0))
    );
    assert_eq!(
        security_origin("http://user:secret@app.example:8080#top"),
        Some(origin("http", "app.example", 8080))
    );
    assert_eq!(security_origin("https://[::1]:9000/"), Some(origin("https", "::1", 9000)));
    assert_eq!(security_origin("https://[::1]/"), Some(origin("https", "::1", 0)));
    assert_eq!(security_origin("about:blank"), None);
    assert_eq!(security_origin("https://app.example:99999/"), None);
}

#[test]
fn http_status_is_none_for_non_http_responses() {
    assert_eq!(http_status(0), None);
    assert_eq!(http_status(204), Some(204));
    assert_eq!(http_status(1000), None);
}

#[test]
fn events_go_out_as_lines_the_app_decodes() {
    let mut out = Vec::new();
    assert!(send_event(&mut out, &HelperEvent::Ready));
    let text = String::from_utf8(out).expect("utf8");
    assert!(text.ends_with('\n'));
    assert_eq!(decode_helper_line::<HelperEvent>(&text).expect("decodes"), HelperEvent::Ready);
}

#[test]
fn main_frame_token_is_hex_of_every_byte() {
    let mut random = [0_u8; 16];
    random[0] = 0x0a;
    random[15] = 0xff;
    assert_eq!(main_frame_token(&random), "0a0000000000000000000000000000ff");
}

#[test]
fn main_frame_body_needs_the_token() {
    let posted = "{\"token\":\"secret\",\"body\":{\"event\":\"close\"}}";
    assert_eq!(main_frame_script_body(posted, "secret").as_deref(), Some("{\"event\":\"close\"}"));
    assert_eq!(main_frame_script_body("{\"token\":\"secret\",\"body\":null}", "secret").as_deref(), Some("null"));
    assert_eq!(main_frame_script_body(posted, "other"), None);
    assert_eq!(main_frame_script_body("{\"event\":\"close\"}", "secret"), None);
    assert_eq!(main_frame_script_body("{\"token\":\"secret\"}", "secret"), None);
    assert_eq!(main_frame_script_body("\"secret\"", "secret"), None);
    assert_eq!(main_frame_script_body("not json", "secret"), None);
}

#[test]
fn main_frame_script_escapes_and_keeps_the_token_out_of_the_page() {
    let script = main_frame_script("riseonly\"WebApp", "abc");
    assert!(script.contains("const name = \"riseonly\\\"WebApp\";"));
    assert!(script.contains("const token = \"abc\";"));
    assert!(script.contains("Object.defineProperty(window, \"webkit\""));
    assert!(!script.contains("window.token"));
}
