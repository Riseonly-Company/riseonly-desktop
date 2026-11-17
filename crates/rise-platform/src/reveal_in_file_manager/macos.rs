use std::os::unix::fs::PermissionsExt as _;

pub(super) fn executable_bit(metadata: &std::fs::Metadata) -> bool {
    metadata.permissions().mode() & 0o111 != 0
}
