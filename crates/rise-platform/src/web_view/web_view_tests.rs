use std::cell::RefCell;
use std::rc::Rc;

use crate::web_view::*;

fn action(url: &str, target_frame: TargetFrame) -> NavigationAction {
    NavigationAction {
        url: Some(url.to_owned()),
        target_frame,
    }
}

/// test handler that allows its own host, loads new window requests in place and cancels the rest
/// records every event it gets
fn recording_slot(events: Rc<RefCell<Vec<WebViewEvent>>>) -> HandlerSlot {
    HandlerSlot::new(WebViewHandlers {
        navigation: Box::new(|action| match (&action.url, action.target_frame) {
            (None, _) => NavigationPolicy::Cancel,
            (Some(url), _) if !url.starts_with("https://app.example/") => NavigationPolicy::Cancel,
            (Some(_), TargetFrame::NewWindow) => NavigationPolicy::LoadInPlace,
            (Some(_), _) => NavigationPolicy::Allow,
        }),
        event: Box::new(move |event| events.borrow_mut().push(event)),
    })
}

#[test]
fn dismantle_follows_the_ios_order() {
    assert_eq!(
        DISMANTLE_ORDER,
        [
            TeardownStep::StopLoading,
            TeardownStep::DetachNavigationDelegate,
            TeardownStep::DetachUiDelegate,
            TeardownStep::RemoveScriptMessageHandler,
            TeardownStep::StopProgressObservation,
        ]
    );
}

#[test]
fn policy_maps_to_the_engine_decision() {
    assert_eq!(
        NavigationPolicy::Allow.engine_decision(),
        EngineDecision {
            load_in_place: false,
            allow: true
        }
    );
    assert_eq!(
        NavigationPolicy::Cancel.engine_decision(),
        EngineDecision {
            load_in_place: false,
            allow: false
        }
    );
    assert_eq!(
        NavigationPolicy::LoadInPlace.engine_decision(),
        EngineDecision {
            load_in_place: true,
            allow: false
        }
    );
}

#[test]
fn the_slot_hands_navigation_to_the_handler() {
    let slot = recording_slot(Rc::default());
    assert_eq!(
        slot.decide(&action("https://app.example/page", TargetFrame::MainFrame)),
        Some(NavigationPolicy::Allow)
    );
    assert_eq!(
        slot.decide(&action("https://elsewhere.example/", TargetFrame::MainFrame)),
        Some(NavigationPolicy::Cancel)
    );
    assert_eq!(
        slot.decide(&action("https://app.example/popup", TargetFrame::NewWindow)),
        Some(NavigationPolicy::LoadInPlace)
    );
    assert_eq!(
        slot.decide(&action("riseonly://bot/1", TargetFrame::MainFrame)),
        Some(NavigationPolicy::Cancel)
    );
    let no_url = NavigationAction {
        url: None,
        target_frame: TargetFrame::MainFrame,
    };
    assert_eq!(slot.decide(&no_url), Some(NavigationPolicy::Cancel));
}

#[test]
fn a_detached_slot_delivers_nothing() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let slot = recording_slot(Rc::clone(&events));
    assert!(slot.emit(WebViewEvent::ProvisionalNavigationStarted));
    slot.detach();
    assert!(slot.is_detached());
    assert!(!slot.emit(WebViewEvent::ProvisionalNavigationStarted));
    assert_eq!(slot.decide(&action("https://app.example/", TargetFrame::MainFrame)), None);
    assert_eq!(*events.borrow(), [WebViewEvent::ProvisionalNavigationStarted]);
}

#[test]
fn a_detach_from_inside_a_handler_stops_delivery() {
    let slot = Rc::new(RefCell::new(None::<Rc<HandlerSlot>>));
    let calls = Rc::new(RefCell::new(0));
    let seen = Rc::clone(&slot);
    let counted = Rc::clone(&calls);
    let handler_slot = Rc::new(HandlerSlot::new(WebViewHandlers {
        navigation: Box::new(|_| NavigationPolicy::Allow),
        event: Box::new(move |_| {
            *counted.borrow_mut() += 1;
            if let Some(slot) = seen.borrow().as_ref() {
                slot.detach();
            }
        }),
    }));
    *slot.borrow_mut() = Some(Rc::clone(&handler_slot));
    assert!(handler_slot.emit(WebViewEvent::ProvisionalNavigationStarted));
    assert!(!handler_slot.emit(WebViewEvent::ProvisionalNavigationStarted));
    assert_eq!(*calls.borrow(), 1);
    slot.borrow_mut().take();
}

#[test]
fn a_reentrant_call_is_not_delivered() {
    let outer = Rc::new(RefCell::new(None::<Rc<HandlerSlot>>));
    let inner_answer = Rc::new(RefCell::new(None));
    let seen = Rc::clone(&outer);
    let answer = Rc::clone(&inner_answer);
    let slot = Rc::new(HandlerSlot::new(WebViewHandlers {
        navigation: Box::new(move |_| {
            if let Some(slot) = seen.borrow().as_ref() {
                *answer.borrow_mut() = Some(slot.emit(WebViewEvent::ProvisionalNavigationStarted));
            }
            NavigationPolicy::Allow
        }),
        event: Box::new(|_| {}),
    }));
    *outer.borrow_mut() = Some(Rc::clone(&slot));
    assert_eq!(
        slot.decide(&action("https://app.example/", TargetFrame::MainFrame)),
        Some(NavigationPolicy::Allow)
    );
    assert_eq!(*inner_answer.borrow(), Some(false));
    assert!(slot.emit(WebViewEvent::ProvisionalNavigationStarted));
    outer.borrow_mut().take();
}

#[test]
fn a_panicking_handler_does_not_unwind() {
    let slot = HandlerSlot::new(WebViewHandlers {
        navigation: Box::new(|_| panic!("handler failure")),
        event: Box::new(|_| {}),
    });
    assert_eq!(slot.decide(&action("https://app.example/", TargetFrame::MainFrame)), None);
    assert!(slot.emit(WebViewEvent::ProvisionalNavigationStarted));
}

#[test]
fn an_envelope_becomes_a_script_message() {
    let origin = SecurityOrigin {
        protocol: "https".to_owned(),
        host: "app.example".to_owned(),
        port: 0,
    };
    let message = ScriptMessage::from_envelope(
        r#"{"name":"riseonlyWebApp","body":{"event":"sendData","data":"{\"a\":1}"}}"#,
        true,
        Some(origin.clone()),
    );
    assert_eq!(
        message,
        Some(ScriptMessage {
            name: "riseonlyWebApp".to_owned(),
            is_main_frame: true,
            security_origin: Some(origin),
            body: serde_json::json!({ "event": "sendData", "data": "{\"a\":1}" }),
        })
    );
    assert_eq!(ScriptMessage::from_envelope("not json", true, None), None);
    assert_eq!(ScriptMessage::from_envelope(r#"{"body":1}"#, true, None), None);
}

#[test]
fn the_shim_defines_the_webkit_handler() {
    let shim = webkit_message_shim("riseonly\"WebApp", "window.chrome.webview.postMessage");
    assert!(shim.contains(r#"const name = "riseonly\"WebApp";"#));
    assert!(shim.contains("window.chrome.webview.postMessage(JSON.stringify({ name, body:"));
    assert!(shim.contains("root.messageHandlers"));
}

#[test]
fn helper_lines_round_trip() {
    let commands = [
        HelperCommand::Create {
            parent: HelperParent::X11 { window: 0x0420_0007 },
            scale_factor: 2.0,
            frame: WebViewFrame {
                x: 0.0,
                y: 44.0,
                width: 390.0,
                height: 600.0,
            },
            configuration: WebViewConfiguration {
                script_message_handler: "riseonlyWebApp".to_owned(),
                javascript_can_open_windows_automatically: false,
                allows_back_forward_navigation_gestures: true,
                background: WebViewColor {
                    red: 0.1,
                    green: 0.1,
                    blue: 0.1,
                    alpha: 1.0,
                },
            },
        },
        HelperCommand::AddUserScript {
            script: UserScript {
                source: "(() => {})();".to_owned(),
                injection_time: UserScriptInjectionTime::AtDocumentStart,
                for_main_frame_only: true,
            },
        },
        HelperCommand::NavigationDecision {
            id: 7,
            policy: NavigationPolicy::LoadInPlace,
        },
        HelperCommand::Destroy,
    ];
    for command in commands {
        let line = encode_helper_line(&command).expect("encodes");
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        assert_eq!(decode_helper_line::<HelperCommand>(&line).expect("decodes"), command);
    }

    let events = [
        HelperEvent::NavigationQuery {
            id: 7,
            action: action("https://app.example/", TargetFrame::NewWindow),
        },
        HelperEvent::View {
            event: WebViewEvent::MainFrameResponse {
                status_code: Some(200),
            },
        },
        HelperEvent::View {
            event: WebViewEvent::Failed(NavigationFailure {
                message: "offline".to_owned(),
                cancelled: false,
                provisional: true,
            }),
        },
        HelperEvent::Unavailable {
            reason: "no display".to_owned(),
        },
    ];
    for event in events {
        let line = encode_helper_line(&event).expect("encodes");
        assert_eq!(decode_helper_line::<HelperEvent>(&format!("{}\r", line.trim_end_matches('\n'))).expect("decodes"), event);
    }
}
