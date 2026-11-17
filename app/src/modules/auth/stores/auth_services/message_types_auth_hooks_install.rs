use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types_auth_hooks::MessageTypesAuthHooks;

pub fn install_message_types_auth_hooks() {
    MessageTypesAuthHooks::install(MessageTypesAuthHooks {
        message_author_user_id_for_outgoing: || {
            AuthServicesStore::viewer_mirror().effective_user_id_for_wire.unwrap_or_default()
        },
        profile_name: || AuthServicesStore::viewer_mirror().profile_name,
        profile_logo: || AuthServicesStore::viewer_mirror().profile_logo,
    });
}
