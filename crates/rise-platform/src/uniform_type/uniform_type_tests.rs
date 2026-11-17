use super::uniform_type::*;

#[test]
fn mime_type_resolves_to_the_apple_identifier() {
    assert_eq!(UTType::from_mime_type("video/mp4").map(|r#type| r#type.identifier()), Some("public.mpeg-4".to_owned()));
    assert_eq!(
        UTType::from_mime_type("video/quicktime").map(|r#type| r#type.identifier()),
        Some("com.apple.quicktime-movie".to_owned())
    );
    assert_eq!(UTType::from_mime_type("audio/mpeg"), Some(UTType::mp3()));
}

#[test]
fn mime_type_ignores_case_and_parameters() {
    assert_eq!(UTType::from_mime_type("Video/MP4; codecs=\"avc1.42E01E\""), Some(UTType::mpeg4_movie()));
    assert_eq!(UTType::from_mime_type(" video/webm"), UTType::from_identifier("org.webmproject.webm"));
}

#[test]
fn unknown_or_empty_tags_resolve_to_none() {
    assert_eq!(UTType::from_mime_type("application/x-riseonly-unknown"), None);
    assert_eq!(UTType::from_mime_type(""), None);
    assert_eq!(UTType::from_filename_extension(""), None);
    assert_eq!(UTType::from_filename_extension("riseonly"), None);
    assert_eq!(UTType::from_identifier("com.example.unknown"), None);
}

#[test]
fn filename_extension_resolves_without_dot_and_case() {
    assert_eq!(UTType::from_filename_extension("MOV"), Some(UTType::quick_time_movie()));
    assert_eq!(UTType::from_filename_extension(".mp4"), Some(UTType::mpeg4_movie()));
    assert_eq!(UTType::from_filename_extension("m4a").map(|r#type| r#type.identifier()), Some("com.apple.m4a-audio".to_owned()));
    assert_eq!(UTType::from_filename_extension("jpg"), Some(UTType::jpeg()));
}

#[test]
fn media_conforms_to_audiovisual_content_and_images_do_not() {
    let audiovisual = UTType::audiovisual_content();
    for extension in ["mp4", "mov", "m4v", "webm", "mp3", "m4a", "wav", "flac", "caf", "ts", "3gp"] {
        let r#type = UTType::from_filename_extension(extension).expect(extension);
        assert!(r#type.conforms_to(&audiovisual), "{extension}");
    }
    for extension in ["jpg", "png", "gif", "heic", "txt", "json", "pdf"] {
        let r#type = UTType::from_filename_extension(extension).expect(extension);
        assert!(!r#type.conforms_to(&audiovisual), "{extension}");
    }
}

#[test]
fn conformance_is_transitive_and_reflexive() {
    let m4a = UTType::from_filename_extension("m4a").expect("m4a");
    assert!(m4a.conforms_to(&UTType::mpeg4_audio()));
    assert!(m4a.conforms_to(&UTType::audio()));
    assert!(m4a.conforms_to(&UTType::mpeg4_movie()));
    assert!(m4a.conforms_to(&UTType::data()));
    assert!(m4a.conforms_to(&UTType::item()));
    assert!(UTType::heic().conforms_to(&UTType::heif()));
    assert!(UTType::mpeg4_movie().conforms_to(&UTType::mpeg4_movie()));
    assert!(!UTType::movie().conforms_to(&UTType::mpeg4_movie()));
    assert!(!UTType::audio().conforms_to(&UTType::movie()));
}

#[test]
fn file_url_conforms_to_url_and_data_but_not_to_content() {
    assert!(UTType::file_url().conforms_to(&UTType::url()));
    assert!(UTType::file_url().conforms_to(&UTType::data()));
    assert!(UTType::url().conforms_to(&UTType::item()));
    assert!(!UTType::url().conforms_to(&UTType::file_url()));
    assert!(!UTType::url().conforms_to(&UTType::content()));
    assert!(!UTType::url().conforms_to(&UTType::plain_text()));
    assert_eq!(UTType::from_identifier("public.file-url"), Some(UTType::file_url()));
    assert_eq!(UTType::url().preferred_mime_type(), None);
}

#[test]
fn preferred_tags_come_first() {
    assert_eq!(UTType::mpeg4_movie().preferred_mime_type(), Some("video/mp4".to_owned()));
    assert_eq!(UTType::mpeg4_movie().preferred_filename_extension(), Some("mp4".to_owned()));
    assert_eq!(UTType::jpeg().preferred_filename_extension(), Some("jpeg".to_owned()));
    assert_eq!(UTType::movie().preferred_mime_type(), None);
}

#[test]
fn every_named_parent_and_constant_is_declared() {
    let declared = declared_identifiers();
    for parent in named_parents() {
        assert!(declared.contains(&parent), "{parent}");
    }
    for constant in [
        UTType::item(),
        UTType::content(),
        UTType::data(),
        UTType::audiovisual_content(),
        UTType::movie(),
        UTType::video(),
        UTType::audio(),
        UTType::mpeg4_movie(),
        UTType::mpeg(),
        UTType::quick_time_movie(),
        UTType::mpeg4_audio(),
        UTType::mp3(),
        UTType::image(),
        UTType::jpeg(),
        UTType::png(),
        UTType::gif(),
        UTType::tiff(),
        UTType::heif(),
        UTType::heic(),
        UTType::text(),
        UTType::plain_text(),
        UTType::json(),
        UTType::pdf(),
        UTType::url(),
        UTType::file_url(),
    ] {
        assert_eq!(UTType::from_identifier(&constant.identifier()), Some(constant));
    }
}
