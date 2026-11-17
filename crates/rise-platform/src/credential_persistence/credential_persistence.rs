use crate::secure_store::SecureStoreError;

/// what a this device only write of one entry did
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LocalWrite {
    #[allow(dead_code, reason = "only the Windows layer writes natively")]
    Written,
    StoreIsLocal,
}

pub(crate) trait LocalEntryWriter {
    fn write_this_device_only(
        &self,
        _target_name: &str,
        _user_name: &str,
        _bytes: &[u8],
    ) -> Result<LocalWrite, SecureStoreError> {
        Ok(LocalWrite::StoreIsLocal)
    }
}

pub(crate) fn write_this_device_only(
    service: &str,
    name: &str,
    bytes: &[u8],
) -> Result<LocalWrite, SecureStoreError> {
    let target_name = keyring_target_name(service, name);
    super::os::NativeLocalEntries.write_this_device_only(&target_name, name, bytes)
}

/// credential manager target name the keyring crate gives an entry
/// user dot service where user is the entry name
/// reads and deletes go through keyring so a native write must use the same name
pub(crate) fn keyring_target_name(service: &str, name: &str) -> String {
    format!("{name}.{service}")
}
