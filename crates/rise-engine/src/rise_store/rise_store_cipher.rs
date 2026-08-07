use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;
use thiserror::Error;

const AAD_PREFIX: &[u8] = b"rise.engine.store.aad.v1";
const NONCE_LEN: usize = 12;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CipherError {
    #[error("database key must be 32 bytes, got {0}")]
    InvalidKeyLength(usize),
    #[error("sealed value is shorter than a nonce")]
    Truncated,
    #[error("value failed authentication")]
    Unauthenticated,
}

/// Binds a sealed value to the account, the domain and the value's identity.
///
/// Every component is length-prefixed, so ("ab", "c") and ("a", "bc") produce
/// different additional data. Without that, a value could be lifted from one row
/// into another whose identity happens to concatenate the same way.
pub fn cipher_context(account_id: &str, domain: &str, components: &[&str]) -> Vec<u8> {
    let mut out = Vec::with_capacity(AAD_PREFIX.len() + 64);
    out.extend_from_slice(AAD_PREFIX);

    let mut push = |value: &str| {
        out.extend_from_slice(&(value.len() as u64).to_be_bytes());
        out.extend_from_slice(value.as_bytes());
    };

    push(account_id);
    push(domain);
    for component in components {
        push(component);
    }
    out
}

pub trait ValueCipher: Send + Sync {
    fn seal(&self, plaintext: &[u8], context: &[u8]) -> Result<Vec<u8>, CipherError>;
    fn open(&self, sealed: &[u8], context: &[u8]) -> Result<Vec<u8>, CipherError>;
}

/// Tests only. A production engine is always opened with AES-GCM.
pub struct PlaintextCipher;

impl ValueCipher for PlaintextCipher {
    fn seal(&self, plaintext: &[u8], _context: &[u8]) -> Result<Vec<u8>, CipherError> {
        Ok(plaintext.to_vec())
    }

    fn open(&self, sealed: &[u8], _context: &[u8]) -> Result<Vec<u8>, CipherError> {
        Ok(sealed.to_vec())
    }
}

pub struct AesGcmCipher {
    cipher: Aes256Gcm,
}

impl AesGcmCipher {
    pub const KEY_LEN: usize = 32;

    pub fn new(key_material: &[u8]) -> Result<Self, CipherError> {
        if key_material.len() != Self::KEY_LEN {
            return Err(CipherError::InvalidKeyLength(key_material.len()));
        }
        let key = Key::<Aes256Gcm>::from_slice(key_material);
        Ok(Self {
            cipher: Aes256Gcm::new(key),
        })
    }

    pub fn generate_key() -> [u8; Self::KEY_LEN] {
        let mut key = [0u8; Self::KEY_LEN];
        rand::rng().fill_bytes(&mut key);
        key
    }
}

// Layout is nonce ‖ ciphertext ‖ tag, matching CryptoKit's combined
// representation so a database stays readable across implementations.
impl ValueCipher for AesGcmCipher {
    fn seal(&self, plaintext: &[u8], context: &[u8]) -> Result<Vec<u8>, CipherError> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = self
            .cipher
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext,
                    aad: context,
                },
            )
            .map_err(|_| CipherError::Unauthenticated)?;

        let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    fn open(&self, sealed: &[u8], context: &[u8]) -> Result<Vec<u8>, CipherError> {
        if sealed.len() <= NONCE_LEN {
            return Err(CipherError::Truncated);
        }
        let (nonce_bytes, ciphertext) = sealed.split_at(NONCE_LEN);
        self.cipher
            .decrypt(
                Nonce::from_slice(nonce_bytes),
                Payload {
                    msg: ciphertext,
                    aad: context,
                },
            )
            .map_err(|_| CipherError::Unauthenticated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher() -> AesGcmCipher {
        AesGcmCipher::new(&[7u8; 32]).unwrap()
    }

    #[test]
    fn a_sealed_value_opens_with_the_same_context() {
        let c = cipher();
        let context = cipher_context("u1", "chat", &["message", "m1"]);
        let sealed = c.seal(b"hello", &context).unwrap();
        assert_eq!(c.open(&sealed, &context).unwrap(), b"hello");
    }

    #[test]
    fn a_value_cannot_be_opened_under_another_account() {
        let c = cipher();
        let mine = cipher_context("u1", "chat", &["message", "m1"]);
        let theirs = cipher_context("u2", "chat", &["message", "m1"]);
        let sealed = c.seal(b"private", &mine).unwrap();
        assert_eq!(c.open(&sealed, &theirs), Err(CipherError::Unauthenticated));
    }

    #[test]
    fn a_value_cannot_be_moved_to_another_row() {
        let c = cipher();
        let row_a = cipher_context("u1", "chat", &["message", "m1"]);
        let row_b = cipher_context("u1", "chat", &["message", "m2"]);
        let sealed = c.seal(b"payload", &row_a).unwrap();
        assert_eq!(c.open(&sealed, &row_b), Err(CipherError::Unauthenticated));
    }

    #[test]
    fn a_value_cannot_be_moved_to_another_domain() {
        let c = cipher();
        let chat = cipher_context("u1", "chat", &["m1"]);
        let post = cipher_context("u1", "post", &["m1"]);
        let sealed = c.seal(b"payload", &chat).unwrap();
        assert_eq!(c.open(&sealed, &post), Err(CipherError::Unauthenticated));
    }

    #[test]
    fn length_prefixing_stops_component_boundaries_from_colliding() {
        assert_ne!(
            cipher_context("u1", "d", &["ab", "c"]),
            cipher_context("u1", "d", &["a", "bc"]),
            "without a length prefix these two would produce identical aad"
        );
    }

    #[test]
    fn the_context_is_versioned() {
        let context = cipher_context("u1", "d", &[]);
        assert!(context.starts_with(AAD_PREFIX));
    }

    #[test]
    fn another_key_cannot_open_the_value() {
        let context = cipher_context("u1", "d", &["k"]);
        let sealed = cipher().seal(b"secret", &context).unwrap();
        let other = AesGcmCipher::new(&[9u8; 32]).unwrap();
        assert_eq!(
            other.open(&sealed, &context),
            Err(CipherError::Unauthenticated)
        );
    }

    #[test]
    fn tampering_with_the_ciphertext_is_detected() {
        let c = cipher();
        let context = cipher_context("u1", "d", &["k"]);
        let mut sealed = c.seal(b"secret", &context).unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0xFF;
        assert_eq!(c.open(&sealed, &context), Err(CipherError::Unauthenticated));
    }

    #[test]
    fn sealing_twice_produces_different_bytes() {
        let c = cipher();
        let context = cipher_context("u1", "d", &["k"]);
        let first = c.seal(b"same", &context).unwrap();
        let second = c.seal(b"same", &context).unwrap();
        assert_ne!(
            first, second,
            "a repeated nonce would leak plaintext equality"
        );
    }

    #[test]
    fn a_short_key_is_rejected_rather_than_padded() {
        assert_eq!(
            AesGcmCipher::new(&[0u8; 16]).err(),
            Some(CipherError::InvalidKeyLength(16))
        );
    }

    #[test]
    fn a_truncated_value_is_reported_as_truncated() {
        assert_eq!(
            cipher().open(&[0u8; 4], b"ctx"),
            Err(CipherError::Truncated)
        );
    }

    #[test]
    fn generated_keys_differ() {
        assert_ne!(AesGcmCipher::generate_key(), AesGcmCipher::generate_key());
    }
}
