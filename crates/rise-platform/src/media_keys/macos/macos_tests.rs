use std::sync::Arc;

use objc2::runtime::AnyClass;

use super::command_target_class;
use crate::media_keys::*;

#[test]
fn the_mediaplayer_framework_is_linked_and_its_classes_are_present() {
    assert!(AnyClass::get(c"MPNowPlayingInfoCenter").is_some());
    assert!(AnyClass::get(c"MPRemoteCommandCenter").is_some());
    assert!(AnyClass::get(c"MPMediaItemArtwork").is_some());
}

#[test]
fn the_remote_command_target_class_registers_once() {
    let first = command_target_class();
    assert!(first.is_some());
    assert_eq!(
        first.map(std::ptr::from_ref),
        command_target_class().map(std::ptr::from_ref),
        "a second registration under the same name would answer None"
    );
}

#[test]
fn an_unbundled_process_is_refused_the_card_instead_of_being_ignored() {
    let identity = MediaIdentity::new("Riseonly", "riseonly.test");
    assert!(
        matches!(
            MediaKeys::install(&identity, Arc::new(|_| CommandOutcome::Handled)),
            Err(MediaKeysError::NotBundled)
        ),
        "silently publishing to a card that cannot appear is the failure this prevents"
    );
}
