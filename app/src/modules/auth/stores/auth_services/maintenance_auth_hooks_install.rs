use gpui::App;

use crate::core::maintenance::maintenance_auth_hooks::MaintenanceAuthHooks;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;

pub fn install_maintenance_auth_hooks() {
    MaintenanceAuthHooks::install(MaintenanceAuthHooks {
        viewer_bypasses_maintenance: Box::new(|cx: &App| {
            AuthServicesStore::try_read(cx).is_some_and(AuthServicesStore::viewer_bypasses_maintenance)
        }),
    });
}
