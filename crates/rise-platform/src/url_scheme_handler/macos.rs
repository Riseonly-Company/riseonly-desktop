use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSString, NSURL};
use rise_core::foundation::foundation_url::Url;

use super::url_scheme_handler::*;

pub(super) struct NativeUrlSchemeHandler;

impl UrlSchemeHandlerSource for NativeUrlSchemeHandler {
    fn has_handler(&self, url: &Url) -> Result<bool, UrlSchemeHandlerUnavailable> {
        let raw = NSString::from_str(url.absolute_string());
        let Some(url) = NSURL::URLWithString(&raw) else {
            return Ok(false);
        };
        Ok(NSWorkspace::sharedWorkspace()
            .URLForApplicationToOpenURL(&url)
            .is_some())
    }
}
