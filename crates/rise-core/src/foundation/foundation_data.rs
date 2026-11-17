use base64::Engine as _;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

const FOUNDATION_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_encode_padding(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireCanonical)
        .with_decode_allow_trailing_bits(true),
);

/// decodes base64, none on whitespace or line breaks or missing pad or odd bytes
pub fn data_base64_encoded(string: &str) -> Option<Vec<u8>> {
    FOUNDATION_BASE64.decode(string.as_bytes()).ok()
}

/// decodes base64 but skips bytes outside the alphabet and =
pub fn data_base64_encoded_ignoring_unknown_characters(string: &str) -> Option<Vec<u8>> {
    let kept: Vec<u8> = string
        .bytes()
        .filter(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
        .collect();
    FOUNDATION_BASE64.decode(&kept).ok()
}

/// base64 string, standard alphabet and padded
/// no line breaks
pub fn base64_encoded_string(data: &[u8]) -> String {
    FOUNDATION_BASE64.encode(data)
}
