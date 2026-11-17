use gpui::App;

use super::user_wire_types::UserGetProfileByTagGatewayDTO;

use crate::core::feature_hooks::FeatureHookSlot;

pub(crate) struct UserWireTypesHooks {
    pub(crate) foreign_profile_payload: fn(&str, &mut App) -> Option<UserGetProfileByTagGatewayDTO>,
}

pub(crate) static USER_WIRE_TYPES_HOOKS: FeatureHookSlot<UserWireTypesHooks> = FeatureHookSlot::new("USER_WIRE_TYPES_HOOKS");

pub(crate) fn set_user_wire_types_hooks(hooks: UserWireTypesHooks) {
    let _ = USER_WIRE_TYPES_HOOKS.set(hooks);
}

pub(crate) fn user_wire_types_hooks() -> Option<&'static UserWireTypesHooks> {
    USER_WIRE_TYPES_HOOKS.get()
}
