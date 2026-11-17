use rise_i18n::date_format::iso8601_date_formatter::ISO8601DateFormatter;

use super::session_types::SessionEntry;

pub type UserSessionWireDTO = rise_canon::rise_proto::session::SessionData;
pub type GetUserSessionsGatewayDTO = rise_canon::rise_proto::session::GetUserSessionsListResponse;
pub type DeleteUserSessionsGatewayDTO = rise_canon::rise_proto::session::DeleteUserSessionsAdvancedResponse;

impl SessionEntry {
    pub fn as_wire_dto(&self) -> UserSessionWireDTO {
        let iso = ISO8601DateFormatter::new();
        UserSessionWireDTO {
            created_at: self
                .first_active
                .map(|date| iso.string_from(&date))
                .unwrap_or_default(),
            device_info: self.device.clone(),
            id: self.id.clone(),
            ip_address: self.ip.clone(),
            last_accessed_at: iso.string_from(&self.last_active),
            location: self.location.clone(),
            ..UserSessionWireDTO::default()
        }
    }
}
