use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256 as Sha256Engine};

/// 32 byte sha256 digest
pub type Sha256Digest = [u8; 32];

pub type HmacSha256Code = [u8; 32];

const HEX_LOWER: &[u8; 16] = b"0123456789abcdef";

pub struct Sha256;

impl Sha256 {
    pub fn hash(data: &[u8]) -> Sha256Digest {
        Sha256Engine::digest(data).into()
    }
}

pub type InsecureSha1Digest = [u8; 20];

/// sha1 hasher, only for ids some outside contract derives with it (eg uuid v5)
/// never use it where an attacker matters
pub struct InsecureSha1;

impl InsecureSha1 {
    pub fn hash(data: &[u8]) -> InsecureSha1Digest {
        let mut state: [u32; 5] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476, 0xc3d2_e1f0];
        let mut message = data.to_vec();
        message.push(0x80);
        while message.len() % 64 != 56 {
            message.push(0);
        }
        message.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_be_bytes());

        for block in message.chunks_exact(64) {
            let mut schedule = [0u32; 80];
            for (slot, word) in schedule.iter_mut().zip(block.chunks_exact(4)) {
                *slot = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
            }
            for index in 16..80 {
                schedule[index] =
                    (schedule[index - 3] ^ schedule[index - 8] ^ schedule[index - 14] ^ schedule[index - 16]).rotate_left(1);
            }

            let [mut a, mut b, mut c, mut d, mut e] = state;
            for (round, word) in schedule.iter().enumerate() {
                let (mix, constant) = match round {
                    0..=19 => ((b & c) | (!b & d), 0x5a82_7999),
                    20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                    40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                    _ => (b ^ c ^ d, 0xca62_c1d6),
                };
                let next = a
                    .rotate_left(5)
                    .wrapping_add(mix)
                    .wrapping_add(e)
                    .wrapping_add(constant)
                    .wrapping_add(*word);
                e = d;
                d = c;
                c = b.rotate_left(30);
                b = a;
                a = next;
            }
            for (slot, value) in state.iter_mut().zip([a, b, c, d, e]) {
                *slot = slot.wrapping_add(value);
            }
        }

        let mut digest = [0u8; 20];
        for (chunk, word) in digest.chunks_exact_mut(4).zip(state) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        digest
    }
}

pub struct HmacSha256;

impl HmacSha256 {
    pub fn authentication_code(data: &[u8], using_key: &[u8]) -> HmacSha256Code {
        Self::keyed(using_key, data).finalize().into_bytes().into()
    }

    /// checks an hmac sha256 code in constant time
    pub fn is_valid_authentication_code(code: &[u8], authenticating: &[u8], using_key: &[u8]) -> bool {
        Self::keyed(using_key, authenticating).verify_slice(code).is_ok()
    }

    fn keyed(key: &[u8], data: &[u8]) -> Hmac<Sha256Engine> {
        let mut mac = <Hmac<Sha256Engine> as Mac>::new_from_slice(key)
            .expect("HMAC accepts keys of any length");
        mac.update(data);
        mac
    }
}

pub fn hex_string(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX_LOWER[usize::from(byte >> 4)]));
        out.push(char::from(HEX_LOWER[usize::from(byte & 0x0f)]));
    }
    out
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex_string(&Sha256::hash(data))
}
