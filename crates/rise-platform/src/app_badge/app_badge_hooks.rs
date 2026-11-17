use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

pub type AppBadgeWork = Pin<Box<dyn Future<Output = ()> + Send>>;

pub struct AppBadgeHooks {
    pub spawn: fn(AppBadgeWork),
    pub bundle_identifier: fn() -> Option<&'static str>,
}

static HOOKS: OnceLock<AppBadgeHooks> = OnceLock::new();

pub fn set_app_badge_hooks(hooks: AppBadgeHooks) {
    let _ = HOOKS.set(hooks);
}

pub fn app_badge_hooks() -> Option<&'static AppBadgeHooks> {
    HOOKS.get()
}
