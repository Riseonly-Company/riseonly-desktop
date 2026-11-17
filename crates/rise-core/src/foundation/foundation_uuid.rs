use std::fmt;

use serde::de::{self, Deserialize, Deserializer, Visitor};
use serde::{Serialize, Serializer};

/// random v4 uuid, text form is upper case
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Uuid {
    bytes: [u8; 16],
}

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";
const UUID_STRING_LEN: usize = 36;
const HYPHEN_POSITIONS: [usize; 4] = [8, 13, 18, 23];

impl Uuid {
    /// new random v4 uuid
    pub fn new_v4() -> Self {
        let mut bytes: [u8; 16] = rand::random();
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Self { bytes }
    }

    /// uuid from 16 bytes
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self { bytes }
    }

    /// the 16 raw bytes
    pub const fn uuid(&self) -> [u8; 16] {
        self.bytes
    }

    /// upper case 8-4-4-4-12 hex string
    pub fn uuid_string(&self) -> String {
        let mut out = String::with_capacity(UUID_STRING_LEN);
        for (index, byte) in self.bytes.iter().enumerate() {
            if matches!(index, 4 | 6 | 8 | 10) {
                out.push('-');
            }
            out.push(char::from(HEX_UPPER[usize::from(byte >> 4)]));
            out.push(char::from(HEX_UPPER[usize::from(byte & 0x0f)]));
        }
        out
    }

    /// parses 8-4-4-4-12 hex digits in any case, else none
    pub fn from_uuid_string(string: &str) -> Option<Self> {
        let raw = string.as_bytes();
        if raw.len() != UUID_STRING_LEN {
            return None;
        }
        let mut bytes = [0u8; 16];
        let mut nibble_index = 0usize;
        for (position, &symbol) in raw.iter().enumerate() {
            if HYPHEN_POSITIONS.contains(&position) {
                if symbol != b'-' {
                    return None;
                }
                continue;
            }
            let value = hex_value(symbol)?;
            let byte = &mut bytes[nibble_index / 2];
            if nibble_index % 2 == 0 {
                *byte = value << 4;
            } else {
                *byte |= value;
            }
            nibble_index += 1;
        }
        Some(Self { bytes })
    }
}

fn hex_value(symbol: u8) -> Option<u8> {
    match symbol {
        b'0'..=b'9' => Some(symbol - b'0'),
        b'a'..=b'f' => Some(symbol - b'a' + 10),
        b'A'..=b'F' => Some(symbol - b'A' + 10),
        _ => None,
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.uuid_string())
    }
}

impl Serialize for Uuid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.uuid_string())
    }
}

impl<'de> Deserialize<'de> for Uuid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UuidVisitor;

        impl Visitor<'_> for UuidVisitor {
            type Value = Uuid;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a UUID string")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Uuid, E> {
                Uuid::from_uuid_string(value)
                    .ok_or_else(|| E::invalid_value(de::Unexpected::Str(value), &self))
            }
        }

        deserializer.deserialize_str(UuidVisitor)
    }
}
