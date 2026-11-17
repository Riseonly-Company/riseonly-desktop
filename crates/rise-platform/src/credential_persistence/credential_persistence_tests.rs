use crate::credential_persistence::*;

struct DefaultWriter;

impl LocalEntryWriter for DefaultWriter {}

#[test]
fn a_store_that_keeps_entries_local_leaves_the_write_to_keyring() {
    let target_name = keyring_target_name("net.riseonly.desktop/net.riseonly.engine", "u1");
    let outcome = DefaultWriter.write_this_device_only(&target_name, "u1", b"key");
    assert_eq!(outcome.unwrap(), LocalWrite::StoreIsLocal);
}

#[test]
fn the_target_name_is_the_one_keyring_reads() {
    assert_eq!(
        keyring_target_name("net.riseonly.desktop/net.riseonly.engine", "#1.u1"),
        "#1.u1.net.riseonly.desktop/net.riseonly.engine"
    );
}

#[test]
fn different_entries_of_one_service_get_different_targets() {
    let service = "net.riseonly.desktop/riseonly.auth";
    assert_ne!(
        keyring_target_name(service, "tokens"),
        keyring_target_name(service, "#1.tokens")
    );
}
