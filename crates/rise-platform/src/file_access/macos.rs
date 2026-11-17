use std::path::Path;

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_foundation::{NSString, NSURL};

use super::file_access::*;

pub(super) struct NativeSecurityScope;

impl SecurityScopeSource for NativeSecurityScope {
    fn start(&self, path: &Path) -> Option<ScopeRelease> {
        let path = path.to_str()?;
        let url: Retained<NSURL> = NSURL::fileURLWithPath(&NSString::from_str(path));
        let did_access: Bool = unsafe { msg_send![&*url, startAccessingSecurityScopedResource] };
        if !did_access.as_bool() {
            return None;
        }
        Some(Box::new(move || {
            let _: () = unsafe { msg_send![&*url, stopAccessingSecurityScopedResource] };
        }))
    }
}
