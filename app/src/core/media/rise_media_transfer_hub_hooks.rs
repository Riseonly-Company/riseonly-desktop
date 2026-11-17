use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct RiseMediaTransferHubHooks {
    pub optional_normalized: fn(Option<&str>) -> Option<String>,
}

pub(crate) static RISE_MEDIA_TRANSFER_HUB_HOOKS: FeatureHookSlot<RiseMediaTransferHubHooks> = FeatureHookSlot::new("RISE_MEDIA_TRANSFER_HUB_HOOKS");

impl RiseMediaTransferHubHooks {
    pub fn install(hooks: Self) {
        let _ = RISE_MEDIA_TRANSFER_HUB_HOOKS.set(hooks);
    }

    pub(crate) fn installed() -> Option<&'static Self> {
        RISE_MEDIA_TRANSFER_HUB_HOOKS.get()
    }

    pub(crate) fn optional_normalized(raw: Option<&str>) -> Option<String> {
        if let Some(hooks) = Self::installed() {
            return (hooks.optional_normalized)(raw);
        }
        let trimmed = trimming_whitespaces_and_newlines(raw?);
        if trimmed.is_empty() {
            return None;
        }
        Some(trimmed.to_owned())
    }
}
