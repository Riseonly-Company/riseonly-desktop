use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::linux::SecretFileStore;
use crate::secure_store::{InMemorySecureStore, SecretAccessibility, SecretKey, SecureStore};

const MACHINE: &[u8] = b"0123456789abcdef0123456789abcdef";
const PURPOSE: &str = "net.riseonly.desktop.test";

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!("rise-secret-file-store-{}-{id}", std::process::id())))
    }

    fn store(&self, machine: &[u8], user: u32) -> SecretFileStore {
        SecretFileStore::with_material(self.0.join("secrets"), machine, user, PURPOSE).expect("key")
    }

    fn files(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut pending = vec![self.0.clone()];
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(&directory).into_iter().flatten().flatten() {
                if entry.path().is_dir() {
                    pending.push(entry.path());
                } else {
                    found.push(entry.path());
                }
            }
        }
        found
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn key(account: &str) -> SecretKey {
    SecretKey::new("riseonly.auth", account)
}

#[test]
fn a_secret_comes_back_and_is_gone_after_delete() {
    let scratch = Scratch::new();
    let store = scratch.store(MACHINE, 1000);
    assert_eq!(store.availability(), Ok(()));
    store.set(&key("session"), b"token-1", SecretAccessibility::AfterFirstUnlock).unwrap();
    store.set(&key("session"), b"token-2", SecretAccessibility::AfterFirstUnlock).unwrap();
    assert_eq!(store.get(&key("session")).unwrap().as_deref(), Some(&b"token-2"[..]));
    assert_eq!(store.get(&key("other")).unwrap(), None);
    store.delete(&key("session")).unwrap();
    store.delete(&key("session")).unwrap();
    assert_eq!(store.get(&key("session")).unwrap(), None);
}

#[test]
fn files_are_private_and_never_hold_the_secret_in_the_clear() {
    let scratch = Scratch::new();
    let store = scratch.store(MACHINE, 1000);
    let secret = b"refresh-token-that-must-not-leak";
    store.set(&key("session"), secret, SecretAccessibility::AfterFirstUnlock).unwrap();
    let files = scratch.files();
    assert_eq!(files.len(), 1);
    let bytes = std::fs::read(&files[0]).unwrap();
    assert!(!bytes.windows(secret.len()).any(|window| window == secret));
    assert_eq!(std::fs::metadata(&files[0]).unwrap().permissions().mode() & 0o777, 0o600);
    let directory = files[0].parent().unwrap();
    assert_eq!(std::fs::metadata(directory).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(std::fs::metadata(scratch.0.join("secrets")).unwrap().permissions().mode() & 0o777, 0o700);
}

#[test]
fn another_machine_or_user_cannot_open_the_files() {
    let scratch = Scratch::new();
    scratch.store(MACHINE, 1000).set(&key("session"), b"token", SecretAccessibility::AfterFirstUnlock).unwrap();
    assert!(scratch.store(b"another-machine-id", 1000).get(&key("session")).is_err());
    assert!(scratch.store(MACHINE, 1001).get(&key("session")).is_err());
}

#[test]
fn a_sealed_secret_is_bound_to_its_own_slot() {
    let scratch = Scratch::new();
    let store = scratch.store(MACHINE, 1000);
    store.set(&key("a"), b"token-a", SecretAccessibility::AfterFirstUnlock).unwrap();
    store.set(&key("b"), b"token-b", SecretAccessibility::AfterFirstUnlock).unwrap();
    let mut files = scratch.files();
    files.sort();
    let first = std::fs::read(&files[0]).unwrap();
    std::fs::write(&files[1], first).unwrap();
    let readable = [store.get(&key("a")).is_ok(), store.get(&key("b")).is_ok()];
    assert_eq!(readable.iter().filter(|ok| **ok).count(), 1);
}

#[test]
fn moving_into_the_os_store_empties_the_files() {
    let scratch = Scratch::new();
    let store = scratch.store(MACHINE, 1000);
    let large = vec![7u8; 9000];
    store.set(&key("session"), b"token", SecretAccessibility::AfterFirstUnlock).unwrap();
    store.set(&SecretKey::new("net.riseonly.engine", "db"), &large, SecretAccessibility::AfterFirstUnlock).unwrap();
    let target = InMemorySecureStore::new();
    assert_eq!(store.move_into(&target), 2);
    assert_eq!(target.get(&key("session")).unwrap().as_deref(), Some(&b"token"[..]));
    assert_eq!(target.get(&SecretKey::new("net.riseonly.engine", "db")).unwrap(), Some(large));
    assert!(scratch.files().is_empty());
    assert!(!scratch.0.join("secrets").exists());
}

#[test]
fn moving_clears_partial_writes_and_files_another_machine_sealed() {
    let scratch = Scratch::new();
    scratch.store(b"old-machine-id", 1000).set(&key("stale"), b"token", SecretAccessibility::AfterFirstUnlock).unwrap();
    let store = scratch.store(MACHINE, 1000);
    store.set(&key("session"), b"token", SecretAccessibility::AfterFirstUnlock).unwrap();
    let service_directory = scratch.files()[0].parent().unwrap().to_path_buf();
    std::fs::write(service_directory.join("left-over.partial"), b"half").unwrap();
    let target = InMemorySecureStore::new();
    assert_eq!(store.move_into(&target), 1);
    assert!(scratch.files().is_empty());
    assert!(!scratch.0.join("secrets").exists());
}
