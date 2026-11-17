use std::sync::OnceLock;

use super::map_http::{MapHttpAnswer, MapHttpRequest};

/// called once with the answer on a runtime thread
/// dropped uncalled if the transfer was stopped or the runtime shut down
pub type MapHttpDone = Box<dyn FnOnce(MapHttpAnswer) + Send>;

/// stops the transfer and done is never called
/// safe to call after the end
pub type MapHttpCancel = Box<dyn Fn() + Send + Sync>;

pub struct MapHttpHooks {
    pub start: fn(MapHttpRequest, MapHttpDone) -> MapHttpCancel,
}

static HOOKS: OnceLock<MapHttpHooks> = OnceLock::new();

pub fn set_map_http_hooks(hooks: MapHttpHooks) {
    let _ = HOOKS.set(hooks);
}

pub fn map_http_hooks() -> Option<&'static MapHttpHooks> {
    HOOKS.get()
}
