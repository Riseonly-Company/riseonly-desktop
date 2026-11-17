/// the catalogue read, guides the account receipts for them and the panda tips switch
/// so a device that never acked a guide itself still knows its finished
pub type RiseGuidesResponseDTO = rise_canon::rise_response::notify::GetPendingGuides;
/// what the server stores now for the panda tips switch
pub type RiseSetGuideAutoStartResponseDTO = rise_canon::rise_response::notify::SetGuideAutoStart;
pub type RiseGuideReceiptDTO = rise_canon::rise_proto::notify::ProtoGuideReceipt;
/// a retired catalogue row answers successfully with found false not with an error
pub type RiseGuideResponseDTO = rise_canon::rise_response::notify::GetGuide;
pub type RiseAckGuideResponseDTO = rise_canon::rise_response::notify::AckGuide;
pub type RiseGuideDTO = rise_canon::rise_proto::notify::ProtoGuide;
/// timeout_ms 0 means the author named no timeout and the model installs the canon default
/// gesture is empty in every guide written before the field existed, no hint
pub type RiseGuideStepDTO = rise_canon::rise_proto::notify::ProtoGuideStep;
