use crate::core::feature_hooks::FeatureHookSlot;

pub(crate) struct SaiRemoteMediaUrlHooks {
    pub optional_normalized: Box<dyn Fn(Option<&str>) -> Option<String> + Send + Sync>,
}

pub(crate) static SAI_REMOTE_MEDIA_URL_HOOKS: FeatureHookSlot<SaiRemoteMediaUrlHooks> = FeatureHookSlot::new("SAI_REMOTE_MEDIA_URL_HOOKS");

/// installed once, a second install keeps the first
pub(crate) fn set_sai_remote_media_url_hooks(hooks: SaiRemoteMediaUrlHooks) {
    let _ = SAI_REMOTE_MEDIA_URL_HOOKS.set(hooks);
}

pub(crate) fn sai_remote_media_url_hooks() -> Option<&'static SaiRemoteMediaUrlHooks> {
    SAI_REMOTE_MEDIA_URL_HOOKS.get()
}

/// normalizes a remote media url, none while chat isnt installed
pub(crate) fn sai_remote_media_url_optional_normalized(raw: Option<&str>) -> Option<String> {
    sai_remote_media_url_hooks().and_then(|hooks| (hooks.optional_normalized)(raw))
}
