use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Context, Entity, Global};
use rise_canon::rise_wire_canon::{PAYLOAD_KEYS, PING_REPLY_TYPE, PUSH_TYPE_PATHS};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_debugger::sai_ws_event_store::SaiWsEventStore;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_canon_events::RiseWsEvents;

/// handler for a ws event, payload is borrowed so clone it to keep it
pub type SaiWsEventHandler = Rc<dyn Fn(Option<&Value>, &mut App)>;

#[derive(Clone)]
pub struct SaiWsEventHandlerRegistration {
    pub r#type: String,
    pub handler: SaiWsEventHandler,
}

/// sees every event frame whoever owns it
/// the registry holds one handler per type so the notification source needs this
pub type SaiWsFrameObserver = Rc<dyn Fn(&str, &Map<String, Value>, &mut App)>;

struct SaiWsFrameObserverSlot(SaiWsFrameObserver);

impl Global for SaiWsFrameObserverSlot {}

pub struct SaiWsEventRegistry {
    handlers: HashMap<String, SaiWsEventHandler>,
}

impl SaiWsEventRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    pub fn set_frame_observer(observer: SaiWsFrameObserver, cx: &mut App) {
        cx.set_global(SaiWsFrameObserverSlot(observer));
    }

    pub fn observe_frame(json: &Map<String, Value>, cx: &mut App) {
        let Some(observer) = cx.try_global::<SaiWsFrameObserverSlot>().map(|slot| Rc::clone(&slot.0)) else {
            return;
        };
        let Some(event_type) = Self::event_type(json) else {
            return;
        };
        observer(&event_type, json, cx);
    }

    const fn debug_tools() -> bool {
        cfg!(debug_assertions) || cfg!(feature = "sai_debug_tools")
    }

    pub fn register(&mut self, items: Vec<SaiWsEventHandlerRegistration>, cx: &mut Context<Self>) {
        for item in items {
            let t = Self::normalize_type(&item.r#type);
            if t.is_empty() {
                continue;
            }
            self.handlers.insert(t, item.handler);
        }
        cx.notify();
    }

    pub fn unregister(&mut self, r#type: &str, cx: &mut Context<Self>) {
        let t = Self::normalize_type(r#type);
        self.handlers.remove(&t);
        cx.notify();
    }

    pub fn remove_all(&mut self, cx: &mut Context<Self>) {
        self.handlers.clear();
        cx.notify();
    }

    pub fn dispatch_if_needed(&mut self, json: &Map<String, Value>, cx: &mut Context<Self>) -> bool {
        let Some(t) = Self::event_type(json) else {
            return false;
        };
        let handler = self.handlers.get(&t).cloned();
        if Self::debug_tools() {
            let handled = handler.is_some();
            SaiWsEventStore::shared(cx).update(cx, |store, cx| store.record(&t, json, handled, cx));
        }
        let Some(handler) = handler else {
            if Self::debug_tools() && t != PING_REPLY_TYPE {
                let line = format!("\u{2190} event type={t} (no handler)");
                Sai::log("WS", move || line, SaiLogKind::Websocket, cx);
            }
            return false;
        };

        let payload = Self::event_payload(json);
        if Self::debug_tools() {
            let line = format!("\u{2190} event type={t} \u{2192} handler");
            Sai::log("WS", move || line, SaiLogKind::Websocket, cx);
        }
        let app: &mut App = cx;
        handler(payload.as_ref(), app);
        true
    }

    pub fn event_type(json: &Map<String, Value>) -> Option<String> {
        for path in PUSH_TYPE_PATHS {
            let Some(Value::String(raw)) = Self::value(json, path) else {
                continue;
            };
            let t = Self::normalize_type(raw);
            if !t.is_empty() {
                return Some(t);
            }
        }
        None
    }

    /// event payload from a frame, nested proto objects are already flat in data
    /// keys that sit beside data are listed in frameExtras and merged in here
    pub fn event_payload(json: &Map<String, Value>) -> Option<Value> {
        for key in PAYLOAD_KEYS {
            let Some(payload) = json.get(*key) else {
                continue;
            };
            let Value::Object(object) = payload else {
                if payload.is_null() {
                    return None;
                }
                return Some(payload.clone());
            };
            let mut object = object.clone();
            for extra in Self::frame_extras(json) {
                if object.contains_key(&extra) {
                    continue;
                }
                if let Some(value) = json.get(&extra) {
                    object.insert(extra, value.clone());
                }
            }
            return Some(Value::Object(object));
        }
        Some(Value::Object(json.clone()))
    }

    fn frame_extras(json: &Map<String, Value>) -> Vec<String> {
        let Some(r#type) = Self::event_type(json) else {
            return Vec::new();
        };
        RiseWsEvents::frame_extras(&r#type)
    }

    fn value<'a>(json: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
        let mut components = path.split('.');
        let first = components.next()?;
        let mut current = json.get(first)?;
        for component in components {
            current = current.as_object()?.get(component)?;
        }
        Some(current)
    }

    fn normalize_type(s: &str) -> String {
        trimming_whitespaces_and_newlines(s).to_owned()
    }
}
