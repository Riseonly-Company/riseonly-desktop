use std::any::TypeId;
use std::ops::Range;

use gpui::AnyView;

use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view_hooks::{
    RiseManualListViewHooks, set_rise_manual_list_view_hooks,
};
use crate::modules::chat::components::chat::async_render::rise_manual_message_hosting_view::{
    RiseHostedRowViewportAuditPolicy, RiseManualMessageHostingView,
};

pub(crate) fn install_rise_manual_list_view_hooks() {
    set_rise_manual_list_view_hooks(RiseManualListViewHooks {
        is_message_hosting_view,
        maximum_measurements_per_pass,
        prioritized_indices,
    });
}

fn is_message_hosting_view(view: &AnyView) -> bool {
    view.entity_type() == TypeId::of::<RiseManualMessageHostingView>()
}

fn maximum_measurements_per_pass() -> usize {
    RiseHostedRowViewportAuditPolicy::MAXIMUM_MEASUREMENTS_PER_PASS
}

fn prioritized_indices(range: Range<usize>) -> Vec<usize> {
    RiseHostedRowViewportAuditPolicy::prioritized_indices(range)
}
