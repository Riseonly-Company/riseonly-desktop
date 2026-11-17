use objc2::rc::Retained;
use objc2_foundation::{NSBundle, NSString};

use super::app_info::*;

pub(super) fn bundle_values() -> BundleValues {
    let bundle = NSBundle::mainBundle();
    BundleValues {
        short_version: info_string(&bundle, "CFBundleShortVersionString"),
        build: info_string(&bundle, "CFBundleVersion"),
        bundle_identifier: bundle.bundleIdentifier().map(|identifier| identifier.to_string()),
        display_name: info_string(&bundle, "CFBundleDisplayName"),
    }
}

/// gives the localized value when one exists, a key holding anything but a string counts as absent
fn info_string(bundle: &NSBundle, key: &str) -> Option<String> {
    let value = bundle.objectForInfoDictionaryKey(&NSString::from_str(key))?;
    let text: Retained<NSString> = value.downcast().ok()?;
    Some(text.to_string())
}
