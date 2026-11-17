use windows::core::{HRESULT, PWSTR};
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_NO_SUCH_LOGON_SESSION};
use windows::Win32::Security::Credentials::{
    CredWriteW, CREDENTIALW, CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

use super::credential_persistence::*;
use crate::secure_store::SecureStoreError;

const ENTRY_COMMENT: &str = "riseonly secure store, this device only";

pub(super) struct NativeLocalEntries;

impl LocalEntryWriter for NativeLocalEntries {
    fn write_this_device_only(
        &self,
        target_name: &str,
        user_name: &str,
        bytes: &[u8],
    ) -> Result<LocalWrite, SecureStoreError> {
        let limit = CRED_MAX_CREDENTIAL_BLOB_SIZE as usize;
        if bytes.len() > limit {
            return Err(SecureStoreError::TooLarge {
                size: bytes.len(),
                limit,
            });
        }

        let mut target_name = wide(target_name);
        let mut user_name = wide(user_name);
        let mut comment = wide(ENTRY_COMMENT);
        let mut blob = bytes.to_vec();
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target_name.as_mut_ptr()),
            Comment: PWSTR(comment.as_mut_ptr()),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: PWSTR(user_name.as_mut_ptr()),
            ..CREDENTIALW::default()
        };

        let result = unsafe { CredWriteW(&credential, 0) };
        blob.fill(0);

        result.map(|()| LocalWrite::Written).map_err(map_error)
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn map_error(error: windows::core::Error) -> SecureStoreError {
    let code = error.code();
    if code == HRESULT::from_win32(ERROR_NO_SUCH_LOGON_SESSION.0) {
        return SecureStoreError::Unavailable;
    }
    if code == HRESULT::from_win32(ERROR_ACCESS_DENIED.0) {
        return SecureStoreError::AccessDenied;
    }
    SecureStoreError::Backend(format!("CredWriteW: {error}"))
}
