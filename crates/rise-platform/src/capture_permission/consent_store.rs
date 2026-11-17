use super::capture_permission::{CaptureAccess, CaptureKind};

pub const STATUS_DENIED_BY_SYSTEM: i32 = 0;
pub const STATUS_NOT_DECLARED_BY_APP: i32 = 1;
pub const STATUS_DENIED_BY_USER: i32 = 2;
pub const STATUS_USER_PROMPT_REQUIRED: i32 = 3;
pub const STATUS_ALLOWED: i32 = 4;

/// capability name that both app capability and the consent store key use
pub const fn capability_name(kind: CaptureKind) -> &'static str {
    match kind {
        CaptureKind::Camera => "webcam",
        CaptureKind::Microphone => "microphone",
    }
}

/// capability status as the hub reads it
/// not declared by app means the manifest lacks webcam or mic, user cant fix it so its restricted
/// unknown value is a refusal
pub const fn access_from_capability_status(raw: i32) -> CaptureAccess {
    match raw {
        STATUS_ALLOWED => CaptureAccess::Granted,
        STATUS_USER_PROMPT_REQUIRED => CaptureAccess::NotDetermined,
        STATUS_DENIED_BY_SYSTEM | STATUS_NOT_DECLARED_BY_APP => CaptureAccess::Restricted,
        _ => CaptureAccess::Denied,
    }
}

/// one consent store toggle, allow or deny and no value when settings never wrote it
/// or an unknown word
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Consent {
    Allow,
    Deny,
    Unset,
    Other,
}

impl Consent {
    pub const ALL: [Self; 4] = [Self::Allow, Self::Deny, Self::Unset, Self::Other];

    pub fn from_value(value: Option<&str>) -> Self {
        match value {
            None => Self::Unset,
            Some(text) if text.eq_ignore_ascii_case("Allow") => Self::Allow,
            Some(text) if text.eq_ignore_ascii_case("Deny") => Self::Deny,
            Some(_) => Self::Other,
        }
    }
}

/// the three toggles that gate an unpackaged app, in windows order
/// device switch from the admin, the user switch and let desktop apps access
/// a toggle settings never wrote is on, unknown word is a refusal
pub const fn access_from_consent(
    device: Consent,
    user: Consent,
    desktop_apps: Consent,
) -> CaptureAccess {
    match (device, user, desktop_apps) {
        (Consent::Deny, _, _) => CaptureAccess::Restricted,
        (Consent::Other, _, _)
        | (_, Consent::Other | Consent::Deny, _)
        | (_, _, Consent::Other | Consent::Deny) => CaptureAccess::Denied,
        _ => CaptureAccess::Granted,
    }
}
