use super::file_access::*;

pub(super) struct NativeSecurityScope;

impl SecurityScopeSource for NativeSecurityScope {}
