use crate::decode_guard::decode_guard::{DecodeGuardError, inflate_lottie};

/// container formats the product serves
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Container {
    Json,
    Ros,
    TgsLegacy,
    DotLottie,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ContainerError {
    Empty,
    TooLarge {
        bytes: u64,
    },
    Unrecognised,
    DotLottieUnsupported,
    Malformed,
}

pub const MAXIMUM_FILE_BYTES: u64 = rise_limits::limit::STICKER_IMAGE_SOURCE;

/// finds the container from the first bytes
/// never from extension or content type, uploads control both
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

/// unwraps a payload to lottie json and proves it is one, the only bytes the rasteriser gets
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

fn inflate(bytes: &[u8]) -> Result<Vec<u8>, ContainerError> {
    let out = inflate_lottie(bytes).map_err(|refusal| match refusal {
        DecodeGuardError::FileTooLarge { bytes, .. } => ContainerError::TooLarge { bytes },
        DecodeGuardError::InflationRefused { limit } => ContainerError::TooLarge {
            bytes: limit.saturating_add(1),
        },
        _ => ContainerError::Malformed,
    })?;

    if out.is_empty() {
        return Err(ContainerError::Empty);
    }

    Ok(out)
}

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
