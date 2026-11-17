use gpui::App;

use crate::modules::auth::stores::auth_services::auth_services_share_hooks::{
    AuthServicesShareHooks, set_auth_services_share_hooks,
};
use crate::modules::share_extension::services::rise_share_app_bridge::RiseShareAppBridge;
use crate::modules::share_extension::stores::share_targets::share_targets_interactions::ShareTargetsInteractionsStore;
use crate::modules::share_extension::stores::share_targets::share_targets_services::ShareTargetsServicesStore;

pub fn install_auth_services_share_hooks() {
    set_auth_services_share_hooks(AuthServicesShareHooks {
        reconcile_persisted_credential: RiseShareAppBridge::reconcile_persisted_credential,
        synchronize_credential: RiseShareAppBridge::synchronize_credential,
        write_snapshot_keeping_persisted_targets: |cx: &mut App| {
            let targets = ShareTargetsServicesStore::shared(cx).read(cx).targets().to_vec();
            RiseShareAppBridge::write_snapshot_keeping_persisted_targets(&targets, cx);
        },
        sign_out: |cx: &mut App| {
            ShareTargetsInteractionsStore::shared(cx).update(cx, |share_targets, cx| share_targets.sign_out(cx));
        },
    });
}
