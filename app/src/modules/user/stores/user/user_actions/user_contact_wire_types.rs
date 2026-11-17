use rise_canon::{rise_event_payload, rise_proto, rise_request, rise_response};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

pub type UserContactSyncSettingsWireDTO = rise_proto::user::ContactSyncSettings;
pub type UserContactSyncSettingsGatewayDTO = rise_response::user::GetContactSyncSettings;
pub type UserContactUserWireDTO = rise_proto::user::ContactUserDto;
pub type UserSavedContactWireDTO = rise_proto::user::SavedContactDto;
pub type UserSavedContactGatewayDTO = rise_response::user::GetSavedContact;
pub type UserSavedContactsListGatewayDTO = rise_response::user::ListSavedContacts;
pub type UserContactPhotoSuggestionWireDTO = rise_proto::user::ContactPhotoSuggestionDto;
pub type UserContactPhotoSuggestionGatewayDTO = rise_response::user::SuggestContactPhoto;
pub type UserContactPhotoSuggestionsListGatewayDTO = rise_response::user::ListContactPhotoSuggestions;
pub type UserContactJoinEventWireDTO = rise_proto::user::ContactJoinEventDto;
pub type UserContactJoinEventsGatewayDTO = rise_response::user::ListContactJoinEvents;
pub type MarkUserContactJoinEventsReadGatewayDTO = rise_response::user::MarkContactJoinEventsRead;
pub type SyncUserContactsGatewayDTO = rise_response::user::SyncContacts;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UserDeviceContact {
    pub client_contact_id: String,
    pub raw_phone: String,
    pub region_iso: String,
    pub phone_e164: Option<String>,
    pub first_name: String,
    pub last_name: String,
    pub phone_label: String,
}

impl UserDeviceContact {
    pub fn wire(&self) -> rise_request::user::sync_contacts::ContactsElement {
        rise_request::user::sync_contacts::ContactsElement {
            client_contact_id: Some(self.client_contact_id.clone()),
            first_name: Some(self.first_name.clone()),
            last_name: Some(self.last_name.clone()),
            phone_e164: self.phone_e164.clone(),
            phone_label: Some(self.phone_label.clone()),
            raw_phone: Some(self.raw_phone.clone()),
            region_iso: Some(self.region_iso.clone()),
        }
    }
}

pub trait SavedContactDtoExt {
    fn resolved_display_name(&self) -> String;
    fn effective_avatar_url(&self) -> String;
}

impl SavedContactDtoExt for rise_proto::user::SavedContactDto {
    fn resolved_display_name(&self) -> String {
        let explicit = trimming_whitespaces_and_newlines(&self.display_name);
        if !explicit.is_empty() {
            return explicit.to_owned();
        }
        let joined = [self.first_name.as_str(), self.last_name.as_str()]
            .into_iter()
            .map(trimming_whitespaces_and_newlines)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if !joined.is_empty() {
            return joined;
        }
        self.user.as_ref().map(|user| user.name.clone()).unwrap_or_default()
    }

    fn effective_avatar_url(&self) -> String {
        let avatar_override =
            self.avatar_override_url.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or("");
        if avatar_override.is_empty() {
            self.user.as_ref().map(|user| user.logo.clone()).unwrap_or_default()
        } else {
            avatar_override.to_owned()
        }
    }
}

pub trait ContactJoinEventDtoExt {
    fn from_canon(event: &rise_event_payload::ContactJoinedInbox) -> Self;
}

impl ContactJoinEventDtoExt for rise_proto::user::ContactJoinEventDto {
    fn from_canon(event: &rise_event_payload::ContactJoinedInbox) -> Self {
        let contact = &event.contact;
        Self {
            contact: Some(rise_proto::user::ContactUserDto {
                id: contact.id.clone(),
                is_bot: contact.is_bot,
                is_online: contact.is_online,
                is_premium: contact.is_premium,
                last_seen: contact.last_seen,
                logo: contact.logo.clone(),
                name: contact.name.clone(),
                tag: contact.tag.clone(),
                user_chat_id: contact.user_chat_id.clone(),
                ..Default::default()
            }),
            created_at_ms: if event.created_at_ms != 0 { event.created_at_ms } else { event.occurred_at_ms },
            id: if event.id.is_empty() { event.event_id.clone() } else { event.id.clone() },
            is_read: event.is_read,
            text: event.text.clone(),
            r#type: event.r#type.clone(),
        }
    }
}
