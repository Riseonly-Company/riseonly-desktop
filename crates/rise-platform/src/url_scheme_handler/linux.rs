use rise_core::foundation::foundation_url::Url;

use super::mime_apps::{XdgDisk, XdgSearchPaths, XdgValues, has_scheme_handler};
use super::url_scheme_handler::*;

pub(super) struct NativeUrlSchemeHandler;

impl UrlSchemeHandlerSource for NativeUrlSchemeHandler {
    fn has_handler(&self, url: &Url) -> Result<bool, UrlSchemeHandlerUnavailable> {
        let Some(scheme) = url.scheme() else {
            return Ok(false);
        };
        let paths = XdgSearchPaths::from_values(&XdgValues::from_environment());
        Ok(has_scheme_handler(scheme, &paths, &XdgDisk))
    }
}
