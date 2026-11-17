use rise_core::foundation::foundation_url::Url;
use windows::Win32::UI::Shell::{
    ASSOCF_INIT_IGNOREUNKNOWN, ASSOCF_IS_PROTOCOL, ASSOCSTR, ASSOCSTR_APPID, ASSOCSTR_EXECUTABLE, AssocQueryStringW,
};
use windows::core::PCWSTR;

use super::url_scheme_handler::*;

pub(super) struct NativeUrlSchemeHandler;

/// asks for the length of one association string
/// when S_FALSE has length above the terminator it exists, any failure reads as not claimed
fn association_exists(scheme: &[u16], kind: ASSOCSTR) -> bool {
    let mut length = 0u32;
    let answer = unsafe {
        AssocQueryStringW(
            ASSOCF_IS_PROTOCOL | ASSOCF_INIT_IGNOREUNKNOWN,
            kind,
            PCWSTR(scheme.as_ptr()),
            PCWSTR::null(),
            None,
            &mut length,
        )
    };
    answer.is_ok() && length > 1
}

impl UrlSchemeHandlerSource for NativeUrlSchemeHandler {
    fn has_handler(&self, url: &Url) -> Result<bool, UrlSchemeHandlerUnavailable> {
        let Some(scheme) = url.scheme().filter(|scheme| !scheme.is_empty()) else {
            return Ok(false);
        };
        let scheme: Vec<u16> = scheme.encode_utf16().chain(std::iter::once(0)).collect();
        Ok(association_exists(&scheme, ASSOCSTR_EXECUTABLE) || association_exists(&scheme, ASSOCSTR_APPID))
    }
}
