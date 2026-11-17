use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rise_canon::rise_crypto::{
    CALL_KDF_INFO, CALL_KEY_BYTES, CALL_PUBLIC_KEY_BYTES, CALL_VERIFICATION_EMOJI_COUNT, CALL_VERIFICATION_EMOJIS,
    CALL_VERIFICATION_PREFIX,
};
use rise_core::foundation::foundation_crypto::{HmacSha256, Sha256};
use x25519_dalek::{PublicKey, SharedSecret, StaticSecret};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCCallCryptoKeyMaterial {
    pub media_key: String,
    pub verification_emojis: Vec<String>,
}

/// private half of one call, never leaves the process and only public_key reaches the wire
#[derive(Clone)]
pub struct RiseRTCCallCryptoIdentity {
    pub private_key: StaticSecret,
}

impl RiseRTCCallCryptoIdentity {
    pub fn new() -> Self {
        Self { private_key: StaticSecret::from(rand::random::<[u8; 32]>()) }
    }

    pub fn public_key(&self) -> String {
        STANDARD.encode(PublicKey::from(&self.private_key).as_bytes())
    }
}

impl Default for RiseRTCCallCryptoIdentity {
    fn default() -> Self {
        Self::new()
    }
}

pub enum RiseRTCCallCrypto {}

impl RiseRTCCallCrypto {
    pub fn derive(
        identity: &RiseRTCCallCryptoIdentity,
        peer_public_key: &str,
        call_id: &str,
    ) -> Option<RiseRTCCallCryptoKeyMaterial> {
        let peer_bytes = STANDARD.decode(peer_public_key).ok()?;
        if peer_bytes.len() != CALL_PUBLIC_KEY_BYTES {
            return None;
        }
        let peer: [u8; 32] = peer_bytes.as_slice().try_into().ok()?;
        let secret = identity.private_key.diffie_hellman(&PublicKey::from(peer));
        if !secret.was_contributory() {
            return None;
        }

        let key_bytes = Self::hkdf_sha256(secret.as_bytes(), call_id.as_bytes(), CALL_KDF_INFO.as_bytes(), CALL_KEY_BYTES);

        Some(RiseRTCCallCryptoKeyMaterial {
            media_key: STANDARD.encode(key_bytes),
            verification_emojis: Self::emojis(&secret, call_id),
        })
    }

    fn hkdf_sha256(input_key_material: &[u8], salt: &[u8], info: &[u8], output_byte_count: usize) -> Vec<u8> {
        let pseudo_random_key = HmacSha256::authentication_code(input_key_material, salt);
        let mut output = Vec::with_capacity(output_byte_count);
        let mut previous: Vec<u8> = Vec::new();
        let mut counter: u8 = 1;
        while output.len() < output_byte_count {
            let mut block_input = previous.clone();
            block_input.extend_from_slice(info);
            block_input.push(counter);
            previous = HmacSha256::authentication_code(&block_input, &pseudo_random_key).to_vec();
            output.extend_from_slice(&previous);
            counter = counter.wrapping_add(1);
        }
        output.truncate(output_byte_count);
        output
    }

    /// fingerprint comes from the shared secret under its own prefix and never from
    /// the media key, so knowing one derived value says nothing about the other
    fn emojis(secret: &SharedSecret, call_id: &str) -> Vec<String> {
        let mut input = CALL_VERIFICATION_PREFIX.as_bytes().to_vec();
        input.extend_from_slice(secret.as_bytes());
        input.extend_from_slice(call_id.as_bytes());

        let digest = Sha256::hash(&input);
        let table = &CALL_VERIFICATION_EMOJIS;
        digest
            .iter()
            .take(CALL_VERIFICATION_EMOJI_COUNT as usize)
            .map(|byte| table[usize::from(*byte) % table.len()].to_owned())
            .collect()
    }
}
