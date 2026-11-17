use std::path::PathBuf;

use crate::audio_metadata::audio_metadata::*;
use crate::audio_metadata::m4a_ilst::*;

fn stco(offsets: &[u32]) -> Vec<u8> {
    let mut body = 0u32.to_be_bytes().to_vec();
    body.extend_from_slice(&(offsets.len() as u32).to_be_bytes());
    for offset in offsets {
        body.extend_from_slice(&offset.to_be_bytes());
    }
    make_box(*b"stco", &body)
}

fn moov_with(stco_box: Vec<u8>, extra: Vec<u8>) -> Vec<u8> {
    let stbl = make_box(*b"stbl", &stco_box);
    let minf = make_box(*b"minf", &stbl);
    let mdia = make_box(*b"mdia", &minf);
    let trak = make_box(*b"trak", &mdia);
    let mut body = make_box(*b"mvhd", &[0u8; 4]);
    body.extend(trak);
    body.extend(extra);
    make_box(*b"moov", &body)
}

fn find(bytes: &[u8], start: u64, end: u64, path: &[&[u8; 4]]) -> Option<BoxSpan> {
    let (first, rest) = path.split_first()?;
    let span = children(bytes, start, end).ok()?.into_iter().find(|span| &span.kind == *first)?;
    if rest.is_empty() {
        return Some(span);
    }
    let body = if span.kind == *b"meta" { span.body_start() + 4 } else { span.body_start() };
    find(bytes, body, span.end(), rest)
}

fn tags() -> M4aMetadata {
    M4aMetadata {
        song_name: Some("Song".to_owned()),
        artist: Some("Artist".to_owned()),
        album_artist: Some("Artist".to_owned()),
        cover_art_jpeg: Some(vec![0xFF, 0xD8, 0xFF, 0xD9]),
    }
}

#[test]
fn items_follow_the_ios_order_and_skip_empty_fields() {
    let metadata = M4aMetadata {
        song_name: Some(String::new()),
        artist: Some("A".to_owned()),
        album_artist: Some("A".to_owned()),
        cover_art_jpeg: Some(Vec::new()),
    };
    assert_eq!(
        metadata.items(),
        vec![M4aMetadataItem::Text(ITUNES_ARTIST, "A"), M4aMetadataItem::Text(ITUNES_ALBUM_ARTIST, "A")]
    );
    assert!(!M4aMetadata::default().replaces_source_tags());
}

#[test]
fn empty_metadata_keeps_the_movie_box() {
    let moov = moov_with(stco(&[100]), Vec::new());
    assert_eq!(rebuilt_moov(&moov, &M4aMetadata::default()).unwrap(), moov);
}

#[test]
fn a_movie_without_user_data_gets_an_itunes_list() {
    let moov = moov_with(stco(&[100]), Vec::new());
    let rebuilt = rebuilt_moov(&moov, &tags()).unwrap();
    let end = rebuilt.len() as u64;
    let ilst = find(&rebuilt, 8, end, &[b"udta", b"meta", b"ilst"]).expect("ilst");
    let items: Vec<[u8; 4]> =
        children(&rebuilt, ilst.body_start(), ilst.end()).unwrap().iter().map(|span| span.kind).collect();
    assert_eq!(items, vec![ITUNES_SONG_NAME, ITUNES_ARTIST, ITUNES_ALBUM_ARTIST, ITUNES_COVER_ART]);
    let handler = find(&rebuilt, 8, end, &[b"udta", b"meta", b"hdlr"]).expect("hdlr");
    let at = (handler.body_start() + 8) as usize;
    assert_eq!(&rebuilt[at..at + 4], b"mdir");
}

#[test]
fn an_existing_list_is_replaced_and_other_user_data_stays() {
    let old_ilst = make_box(*b"ilst", &make_box(*b"\xA9day", b"2020"));
    let mut meta_body = 0u32.to_be_bytes().to_vec();
    let mut hdlr = vec![0u8; 8];
    hdlr.extend_from_slice(b"mdirappl");
    hdlr.extend_from_slice(&[0u8; 9]);
    meta_body.extend(make_box(*b"hdlr", &hdlr));
    meta_body.extend(old_ilst);
    let mut udta_body = make_box(*b"name", b"x");
    udta_body.extend(make_box(*b"meta", &meta_body));
    let moov = moov_with(stco(&[100]), make_box(*b"udta", &udta_body));
    let rebuilt = rebuilt_moov(&moov, &tags()).unwrap();
    let end = rebuilt.len() as u64;
    assert!(find(&rebuilt, 8, end, &[b"udta", b"name"]).is_some());
    let ilst = find(&rebuilt, 8, end, &[b"udta", b"meta", b"ilst"]).unwrap();
    assert_eq!(children(&rebuilt, ilst.body_start(), ilst.end()).unwrap().len(), 4);
    let meta = find(&rebuilt, 8, end, &[b"udta", b"meta"]).unwrap();
    let lists = children(&rebuilt, meta.body_start() + 4, meta.end())
        .unwrap()
        .iter()
        .filter(|span| &span.kind == b"ilst")
        .count();
    assert_eq!(lists, 1);
}

#[test]
fn chunk_offsets_after_the_movie_move_by_the_growth() {
    let mut moov = moov_with(stco(&[50, 500]), Vec::new());
    shift_chunk_offsets(&mut moov, 100, 40).unwrap();
    let end = moov.len() as u64;
    let table = find(&moov, 8, end, &[b"trak", b"mdia", b"minf", b"stbl", b"stco"]).unwrap();
    let at = (table.body_start() + 8) as usize;
    assert_eq!(u32::from_be_bytes(moov[at..at + 4].try_into().unwrap()), 50);
    assert_eq!(u32::from_be_bytes(moov[at + 4..at + 8].try_into().unwrap()), 540);
}

#[test]
fn a_thirty_two_bit_offset_past_four_gibibytes_fails() {
    let mut moov = moov_with(stco(&[u32::MAX - 1]), Vec::new());
    assert!(matches!(shift_chunk_offsets(&mut moov, 0, 10), Err(AudioMetadataError::Failed(_))));
}

#[test]
fn an_overrunning_box_is_refused() {
    let mut bytes = make_box(*b"free", &[0u8; 4]);
    bytes[3] = 200;
    assert!(parse_header(&bytes, 0, bytes.len() as u64).is_err());
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rise-m4a-ilst-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn rewrite_keeps_the_samples_and_points_the_chunks_at_them() {
    let dir = scratch("rewrite");
    let source = dir.join("source.m4a");
    let destination = dir.join("destination.m4a");
    std::fs::remove_file(&destination).ok();

    let ftyp = make_box(*b"ftyp", b"M4A \0\0\0\0M4A isom");
    let samples = b"AUDIOSAMPLES".to_vec();
    let probe = moov_with(stco(&[0]), Vec::new());
    let mdat_body_at = (ftyp.len() + probe.len() + 8) as u32;
    let moov = moov_with(stco(&[mdat_body_at]), Vec::new());
    let mut file = ftyp.clone();
    file.extend(&moov);
    file.extend(make_box(*b"mdat", &samples));
    std::fs::write(&source, &file).unwrap();

    rewrite(&source, &destination, &tags()).unwrap();
    let written = std::fs::read(&destination).unwrap();
    let end = written.len() as u64;
    let table = find(&written, 0, end, &[b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stco"]).unwrap();
    let at = (table.body_start() + 8) as usize;
    let offset = u32::from_be_bytes(written[at..at + 4].try_into().unwrap()) as usize;
    assert_eq!(&written[offset..offset + samples.len()], samples.as_slice());
    assert!(find(&written, 0, end, &[b"moov", b"udta", b"meta", b"ilst"]).is_some());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rewrite_refuses_a_file_that_is_not_mpeg4() {
    let dir = scratch("not-mp4");
    let source = dir.join("source.mp3");
    std::fs::write(&source, b"ID3\x03\0\0\0\0\0\0").unwrap();
    let result = rewrite(&source, &dir.join("out.m4a"), &tags());
    assert!(matches!(result, Err(AudioMetadataError::Failed(_))));
    std::fs::remove_dir_all(&dir).ok();
}
