//! What arrives off the network, and whether rlottie is allowed to see it.
//!
//! This is the security boundary of the sticker path. rlottie is C++ parsing
//! attacker-supplied JSON, animated stickers have published exploit precedent,
//! and a sticker is a thing strangers can send you. Everything below runs
//! before a single byte reaches the C++ side: the container is identified from
//! its magic rather than its filename or its Content-Type, inflation is bounded
//! so a gzip bomb cannot be turned into an allocation, and the document must
//! actually look like a Lottie before it is handed over.
//!
//! Ported from `LottieStickerLoader.detectedContainer` and
//! `normalizedJSONData` in the reference, with the inflation bound tightened:
//! the reference inflates fully and then checks the size, which is one
//! allocation too late.

use std::io::Read;

/// The wrappers the product actually serves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Container {
    /// Bare Lottie JSON.
    Json,
    /// Riseonly's own sticker container: gzipped Lottie JSON.
    Ros,
    /// Telegram's `.tgs`, which is the same thing. Kept as a separate variant
    /// because the reference does, and because a pack imported from Telegram
    /// is worth being able to see in a trace.
    TgsLegacy,
    /// A `.lottie` archive: a zip with a manifest and one or more animations.
    DotLottie,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ContainerError {
    Empty,
    /// Larger than `MAXIMUM_FILE_BYTES`, before or after inflation.
    TooLarge {
        bytes: u64,
    },
    /// The magic matches nothing this client renders.
    Unrecognised,
    /// Detected, deliberately not handled. The reference renders these through
    /// Airbnb's engine, which has no counterpart here; a zip reader plus
    /// manifest handling is a decision, not an oversight, and this variant is
    /// what makes it visible instead of silently blank.
    DotLottieUnsupported,
    /// Inflation failed, or the payload is not a Lottie document.
    Malformed,
}

/// The largest sticker accepted, compressed or inflated.
///
/// The reference's number. A Lottie above this is not a sticker, it is a
/// denial-of-service with a preview image.
pub const MAXIMUM_FILE_BYTES: u64 = 20 * 1024 * 1024;

/// Identify the container from its first bytes.
///
/// Never from the file extension or the server's Content-Type: both are
/// attacker-controlled for user-uploaded media, and the reference's own Accept
/// header asks for five different types for the same sticker.
pub fn detect(bytes: &[u8]) -> Option<Container> {
    if bytes.is_empty() {
        return None;
    }

    if bytes.starts_with(&[0x50, 0x4B]) {
        return Some(Container::DotLottie);
    }
    if bytes.starts_with(&[0x1F, 0x8B]) {
        return Some(Container::Ros);
    }

    let start = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        0
    };

    for byte in &bytes[start..] {
        match byte {
            0x09 | 0x0A | 0x0D | 0x20 => continue,
            0x7B => return Some(Container::Json),
            _ => return None,
        }
    }

    None
}

/// Unwrap a payload to Lottie JSON and prove it is one.
///
/// The returned bytes are the only thing the rasteriser is ever given.
pub fn to_animation_json(bytes: &[u8]) -> Result<Vec<u8>, ContainerError> {
    if bytes.is_empty() {
        return Err(ContainerError::Empty);
    }
    if bytes.len() as u64 > MAXIMUM_FILE_BYTES {
        return Err(ContainerError::TooLarge {
            bytes: bytes.len() as u64,
        });
    }

    let json = match detect(bytes) {
        None => return Err(ContainerError::Unrecognised),
        Some(Container::DotLottie) => return Err(ContainerError::DotLottieUnsupported),
        Some(Container::Json) => bytes.to_vec(),
        Some(Container::Ros | Container::TgsLegacy) => inflate(bytes)?,
    };

    validate(&json)?;
    Ok(json)
}

/// Inflate with a hard ceiling on the OUTPUT, not just the input.
///
/// A 20 MiB gzip of zeroes inflates to gigabytes. Reading through `take` means
/// the ceiling is enforced by the reader rather than by a length check that
/// only runs once the allocation already happened.
fn inflate(bytes: &[u8]) -> Result<Vec<u8>, ContainerError> {
    let mut decoder = flate2::read::GzDecoder::new(bytes).take(MAXIMUM_FILE_BYTES + 1);
    let mut out = Vec::new();

    decoder
        .read_to_end(&mut out)
        .map_err(|_| ContainerError::Malformed)?;

    if out.len() as u64 > MAXIMUM_FILE_BYTES {
        return Err(ContainerError::TooLarge {
            bytes: out.len() as u64,
        });
    }
    if out.is_empty() {
        return Err(ContainerError::Empty);
    }

    Ok(out)
}

/// The reference's shape check: a Lottie has layers, a frame rate, an in and
/// out point, and a size. Anything else is not worth starting a C++ parser for.
fn validate(json: &[u8]) -> Result<(), ContainerError> {
    let value: serde_json::Value =
        serde_json::from_slice(json).map_err(|_| ContainerError::Malformed)?;

    let root = value.as_object().ok_or(ContainerError::Malformed)?;

    if !root.get("layers").is_some_and(serde_json::Value::is_array) {
        return Err(ContainerError::Malformed);
    }
    for field in ["fr", "ip", "op", "w", "h"] {
        if !root.get(field).is_some_and(serde_json::Value::is_number) {
            return Err(ContainerError::Malformed);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn minimal_lottie() -> Vec<u8> {
        br#"{"v":"5.5.7","fr":60,"ip":0,"op":60,"w":512,"h":512,"layers":[]}"#.to_vec()
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn each_container_is_identified_by_its_magic() {
        assert_eq!(detect(&minimal_lottie()), Some(Container::Json));
        assert_eq!(detect(&gzip(&minimal_lottie())), Some(Container::Ros));
        assert_eq!(detect(b"PK\x03\x04rest"), Some(Container::DotLottie));
        assert_eq!(detect(b""), None);
        assert_eq!(detect(b"<html>"), None);
    }

    #[test]
    fn a_byte_order_mark_and_leading_whitespace_do_not_hide_the_json() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"  \n\t");
        bytes.extend_from_slice(&minimal_lottie());

        assert_eq!(detect(&bytes), Some(Container::Json));
    }

    #[test]
    fn whitespace_alone_is_not_a_document() {
        assert_eq!(detect(b"   \n  "), None);
    }

    #[test]
    fn bare_json_passes_through_unchanged() {
        let json = minimal_lottie();
        assert_eq!(to_animation_json(&json).unwrap(), json);
    }

    #[test]
    fn a_gzipped_sticker_is_inflated() {
        let json = minimal_lottie();
        assert_eq!(to_animation_json(&gzip(&json)).unwrap(), json);
    }

    #[test]
    fn a_gzip_bomb_is_refused_without_allocating_what_it_asked_for() {
        let bomb = gzip(&vec![b' '; (MAXIMUM_FILE_BYTES + 4096) as usize]);
        assert!(
            (bomb.len() as u64) < MAXIMUM_FILE_BYTES,
            "the test payload has to be small compressed or it proves nothing"
        );

        assert!(matches!(
            to_animation_json(&bomb),
            Err(ContainerError::TooLarge { .. })
        ));
    }

    #[test]
    fn a_dotlottie_archive_is_named_rather_than_rendered_blank() {
        assert_eq!(
            to_animation_json(b"PK\x03\x04whatever"),
            Err(ContainerError::DotLottieUnsupported)
        );
    }

    #[test]
    fn json_that_is_not_a_lottie_never_reaches_the_rasteriser() {
        for payload in [
            &br#"{"hello":"world"}"#[..],
            &br#"{"layers":{},"fr":60,"ip":0,"op":60,"w":1,"h":1}"#[..],
            &br#"{"layers":[],"fr":"60","ip":0,"op":60,"w":1,"h":1}"#[..],
            &br#"{"layers":[],"ip":0,"op":60,"w":1,"h":1}"#[..],
            &br#"{"#[..],
        ] {
            assert_eq!(
                to_animation_json(payload),
                Err(ContainerError::Malformed),
                "{}",
                String::from_utf8_lossy(payload)
            );
        }
    }

    #[test]
    fn truncated_gzip_is_malformed_not_a_panic() {
        let compressed = gzip(&minimal_lottie());
        let truncated = &compressed[..compressed.len() / 2];

        assert_eq!(to_animation_json(truncated), Err(ContainerError::Malformed));
    }

    #[test]
    fn an_oversized_payload_is_refused_before_it_is_parsed() {
        let huge = vec![b'{'; (MAXIMUM_FILE_BYTES + 1) as usize];

        assert!(matches!(
            to_animation_json(&huge),
            Err(ContainerError::TooLarge { .. })
        ));
    }
}
