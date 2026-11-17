use super::foundation_crypto::{HmacSha256, InsecureSha1, Sha256, hex_string, sha256_hex};

#[test]
fn sha256_matches_known_vectors() {
    assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
}

#[test]
fn hex_is_lower_case_and_two_digits_per_byte() {
    assert_eq!(hex_string(&[0x00, 0x0a, 0xff, 0xab]), "000affab");
    assert_eq!(hex_string(&[]), "");
}

#[test]
fn digest_prefix_gives_short_fingerprint() {
    let fingerprint = hex_string(&Sha256::hash(b"abc")[..12]);
    assert_eq!(fingerprint, "ba7816bf8f01cfea414140de");
}

#[test]
fn hmac_sha256_matches_rfc_4231_case_one() {
    let key = [0x0bu8; 20];
    let code = HmacSha256::authentication_code(b"Hi There", &key);
    assert_eq!(hex_string(&code), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
}

#[test]
fn hmac_sha256_matches_rfc_4231_case_two() {
    let code = HmacSha256::authentication_code(b"what do ya want for nothing?", b"Jefe");
    assert_eq!(hex_string(&code), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
}

#[test]
fn hmac_validation_accepts_own_code_only() {
    let key = b"secret";
    let code = HmacSha256::authentication_code(b"payload", key);
    assert!(HmacSha256::is_valid_authentication_code(&code, b"payload", key));
    assert!(!HmacSha256::is_valid_authentication_code(&code, b"payload!", key));
    assert!(!HmacSha256::is_valid_authentication_code(&code[..31], b"payload", key));
}

#[test]
fn insecure_sha1_matches_known_vectors() {
    assert_eq!(hex_string(&InsecureSha1::hash(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_eq!(hex_string(&InsecureSha1::hash(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(
        hex_string(&InsecureSha1::hash(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
        "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
    );
}
