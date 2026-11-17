use crate::gpui_shim::PlatformSupport;

/// callback runs on the main thread outside any gpui update for each quit request
/// the exit is then the callbacks to do, a later call replaces the earlier one
/// unsupported means the host shell has no such request
pub fn on_quit_request(callback: impl Fn() + 'static) -> PlatformSupport {
    super::os::install(Box::new(callback))
}
