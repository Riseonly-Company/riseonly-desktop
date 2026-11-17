use gpui::{App, Task};
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSBundle, NSString, NSURL};

use super::url_scheme::*;

pub(super) struct NativeUrlScheme;

impl UrlSchemeRegistrar for NativeUrlScheme {
    fn is_default_handler(&self, scheme: &str) -> Result<bool, UrlSchemeUnavailable> {
        let bundle = NSBundle::mainBundle();
        if bundle.bundleIdentifier().is_none() {
            return Err(UrlSchemeUnavailable::NotInstalled);
        }
        let raw = NSString::from_str(&format!("{scheme}://"));
        let Some(url) = NSURL::URLWithString(&raw) else {
            return Ok(false);
        };
        let Some(handler) = NSWorkspace::sharedWorkspace().URLForApplicationToOpenURL(&url) else {
            return Ok(false);
        };
        let handler_path = handler.path().map(|path| path.to_string());
        let bundle_path = bundle.bundleURL().path().map(|path| path.to_string());
        Ok(handler_path.is_some() && handler_path == bundle_path)
    }

    fn register(
        &self,
        scheme: &str,
        cx: &App,
    ) -> Result<Task<anyhow::Result<()>>, UrlSchemeUnavailable> {
        Ok(cx.register_url_scheme(scheme))
    }
}
