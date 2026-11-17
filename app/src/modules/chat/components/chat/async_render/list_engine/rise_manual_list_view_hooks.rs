use std::ops::Range;

use gpui::AnyView;

use crate::core::feature_hooks::FeatureHookSlot;

pub(crate) struct RiseManualListViewHooks {
    pub is_message_hosting_view: fn(view: &AnyView) -> bool,
    pub maximum_measurements_per_pass: fn() -> usize,
    pub prioritized_indices: fn(range: Range<usize>) -> Vec<usize>,
}

pub(crate) static RISE_MANUAL_LIST_VIEW_HOOKS: FeatureHookSlot<RiseManualListViewHooks> = FeatureHookSlot::new("RISE_MANUAL_LIST_VIEW_HOOKS");

/// installs the hooks once, a second call keeps the hooks of the first installer
pub(crate) fn set_rise_manual_list_view_hooks(hooks: RiseManualListViewHooks) {
    let _ = RISE_MANUAL_LIST_VIEW_HOOKS.set(hooks);
}

pub(crate) fn rise_manual_list_view_hooks() -> Option<&'static RiseManualListViewHooks> {
    RISE_MANUAL_LIST_VIEW_HOOKS.get()
}

impl RiseManualListViewHooks {
    pub(crate) fn is_message_hosting_view_or_default(view: &AnyView) -> bool {
        rise_manual_list_view_hooks().is_some_and(|hooks| (hooks.is_message_hosting_view)(view))
    }

    pub(crate) fn maximum_measurements_per_pass_or_default() -> usize {
        rise_manual_list_view_hooks().map_or(0, |hooks| (hooks.maximum_measurements_per_pass)())
    }

    pub(crate) fn prioritized_indices_or_default(range: Range<usize>) -> Vec<usize> {
        rise_manual_list_view_hooks().map_or_else(Vec::new, |hooks| (hooks.prioritized_indices)(range))
    }
}
