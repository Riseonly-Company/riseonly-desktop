use gpui::App;

use crate::modules::user::stores::user::user_actions::user_wire_types::UserGetProfileByTagGatewayDTO;
use crate::modules::user::stores::user::user_actions::user_wire_types_hooks::{
    UserWireTypesHooks, set_user_wire_types_hooks,
};
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

pub(crate) fn install_user_wire_types_hooks() {
    set_user_wire_types_hooks(UserWireTypesHooks { foreign_profile_payload });
}

fn foreign_profile_payload(user_id: &str, cx: &mut App) -> Option<UserGetProfileByTagGatewayDTO> {
    UserServicesStore::shared(cx).update(cx, |services, cx| services.foreign_profile_payload(user_id, cx))
}
