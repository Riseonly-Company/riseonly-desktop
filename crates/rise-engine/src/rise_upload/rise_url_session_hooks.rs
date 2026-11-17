use std::sync::OnceLock;

pub struct RiseUrlSessionHooks {
    pub client: fn() -> Option<reqwest::Client>,
}

static HOOKS: OnceLock<RiseUrlSessionHooks> = OnceLock::new();

pub fn set_rise_url_session_hooks(hooks: RiseUrlSessionHooks) {
    let _ = HOOKS.set(hooks);
}

pub fn rise_url_session_hooks() -> Option<&'static RiseUrlSessionHooks> {
    HOOKS.get()
}
