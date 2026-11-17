use aes_gcm::aead::{Aead, AeadCore, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;

type RiseAESGCMStoreCipherNonce = Nonce<<Aes256Gcm as AeadCore>::NonceSize>;

pub(crate) trait RiseStoreValueCipher: Send + Sync {
    fn seal(
        &self,
        plaintext: &[u8],
        authenticating: &[u8],
    ) -> Result<Vec<u8>, RiseAESGCMStoreCipherError>;
    fn open(
        &self,
        sealed: &[u8],
        authenticating: &[u8],
    ) -> Result<Vec<u8>, RiseAESGCMStoreCipherError>;
}

/// tests only, production engines use aes-gcm
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RisePlaintextStoreCipher;

impl RiseStoreValueCipher for RisePlaintextStoreCipher {
    fn seal(
        &self,
        plaintext: &[u8],
        _authenticating: &[u8],
    ) -> Result<Vec<u8>, RiseAESGCMStoreCipherError> {
        Ok(plaintext.to_vec())
    }

    fn open(
        &self,
        sealed: &[u8],
        _authenticating: &[u8],
    ) -> Result<Vec<u8>, RiseAESGCMStoreCipherError> {
        Ok(sealed.to_vec())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum RiseAESGCMStoreCipherError {
    #[error("the store key has the wrong length")]
    InvalidKeyLength,
    #[error("the sealed value has no combined representation")]
    MissingCombinedRepresentation,
    #[error("the sealed value has the wrong size")]
    IncorrectParameterSize,
    #[error("the sealed value failed authentication")]
    AuthenticationFailure,
}

pub(crate) struct RiseAESGCMStoreCipher {
    key_material: Aes256Gcm,
}

impl RiseAESGCMStoreCipher {
    pub(crate) fn new(key_material: &[u8]) -> Result<Self, RiseAESGCMStoreCipherError> {
        rise_crypto::check_key_bytes(key_material.len())
            .map_err(|_| RiseAESGCMStoreCipherError::InvalidKeyLength)?;
        let key_material = Aes256Gcm::new_from_slice(key_material)
            .map_err(|_| RiseAESGCMStoreCipherError::InvalidKeyLength)?;
        Ok(Self { key_material })
    }
}

impl RiseStoreValueCipher for RiseAESGCMStoreCipher {
    fn seal(
        &self,
        plaintext: &[u8],
        authenticating: &[u8],
    ) -> Result<Vec<u8>, RiseAESGCMStoreCipherError> {
        let mut nonce = RiseAESGCMStoreCipherNonce::default();
        rand::rng().fill_bytes(nonce.as_mut_slice());
        let ciphertext_and_tag = self
            .key_material
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: authenticating,
                },
            )
            .map_err(|_| RiseAESGCMStoreCipherError::IncorrectParameterSize)?;
        rise_crypto::seal_frame(nonce.as_slice(), &ciphertext_and_tag)
            .map_err(|_| RiseAESGCMStoreCipherError::MissingCombinedRepresentation)
    }

    fn open(
        &self,
        sealed: &[u8],
        authenticating: &[u8],
    ) -> Result<Vec<u8>, RiseAESGCMStoreCipherError> {
        let frame = rise_crypto::parse_sealed(sealed.len())
            .map_err(|_| RiseAESGCMStoreCipherError::IncorrectParameterSize)?;
        let (Some(nonce), Some(ciphertext_and_tag)) =
            (frame.nonce_of(sealed), frame.ciphertext_and_tag_of(sealed))
        else {
            return Err(RiseAESGCMStoreCipherError::IncorrectParameterSize);
        };
        if nonce.len() != RiseAESGCMStoreCipherNonce::default().len() {
            return Err(RiseAESGCMStoreCipherError::IncorrectParameterSize);
        }
        self.key_material
            .decrypt(
                RiseAESGCMStoreCipherNonce::from_slice(nonce),
                Payload {
                    msg: ciphertext_and_tag,
                    aad: authenticating,
                },
            )
            .map_err(|_| RiseAESGCMStoreCipherError::AuthenticationFailure)
    }
}
