use rise_engine::rise_upload::rise_url_session_hooks::{RiseUrlSessionHooks, set_rise_url_session_hooks};

use super::network_transport_config::NetworkClientSlot;

static URL_SESSION_CLIENT: NetworkClientSlot = NetworkClientSlot::standard();

pub(crate) fn install_rise_url_session_hooks() {
    set_rise_url_session_hooks(RiseUrlSessionHooks {
        client: || URL_SESSION_CLIENT.current(),
    });
}
