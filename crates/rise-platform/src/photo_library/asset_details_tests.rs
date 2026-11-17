use std::io::Cursor;

use super::asset_details::*;
use super::photo_library::*;

fn asset(id: &str, kind: LibraryAssetKind, created_at_ms: Option<i64>) -> LibraryAsset {
    LibraryAsset {
        id: id.to_owned(),
        kind,
        pixel_width: 0,
        pixel_height: 0,
        duration_secs: 0.0,
        created_at_ms,
    }
}

fn newest_first() -> Vec<LibraryAsset> {
    vec![
        asset("e", LibraryAssetKind::Image, Some(500)),
        asset("d", LibraryAssetKind::Video, Some(400)),
        asset("c", LibraryAssetKind::Image, Some(300)),
        asset("b", LibraryAssetKind::Image, Some(200)),
        asset("a", LibraryAssetKind::Image, None),
    ]
}

fn ids(assets: &[LibraryAsset]) -> Vec<&str> {
    assets.iter().map(|asset| asset.id.as_str()).collect()
}

#[test]
fn only_assets_created_strictly_after_the_date_match() {
    let picked = created_after_of_kind(&newest_first(), LibraryMediaKind::All, 300, 0);
    assert_eq!(ids(&picked), ["e", "d"]);
}

#[test]
fn undated_assets_never_match() {
    let picked = created_after_of_kind(&newest_first(), LibraryMediaKind::All, i64::MIN, 0);
    assert_eq!(ids(&picked), ["e", "d", "c", "b"]);
}

#[test]
fn the_kind_and_the_limit_apply_in_newest_first_order() {
    let all = newest_first();
    assert_eq!(ids(&created_after_of_kind(&all, LibraryMediaKind::Images, 0, 2)), ["e", "c"]);
    assert_eq!(ids(&created_after_of_kind(&all, LibraryMediaKind::Videos, 0, 5)), ["d"]);
    assert_eq!(ids(&created_after_of_kind(&all, LibraryMediaKind::All, 0, 1)), ["e"]);
}

type Dms = [(u32, u32); 3];

/// test helper, a tiff block whose ifd0 points at a gps ifd with both references and positions
fn tiff(little: bool, latitude_ref: u8, latitude: Dms, longitude_ref: u8, longitude: Dms) -> Vec<u8> {
    let u16b = |value: u16| if little { value.to_le_bytes() } else { value.to_be_bytes() };
    let u32b = |value: u32| if little { value.to_le_bytes() } else { value.to_be_bytes() };
    let mut out = Vec::new();
    out.extend_from_slice(if little { b"II" } else { b"MM" });
    out.extend_from_slice(&u16b(42));
    out.extend_from_slice(&u32b(8));
    out.extend_from_slice(&u16b(1));
    out.extend_from_slice(&u16b(0x8825));
    out.extend_from_slice(&u16b(4));
    out.extend_from_slice(&u32b(1));
    out.extend_from_slice(&u32b(26));
    out.extend_from_slice(&u32b(0));
    out.extend_from_slice(&u16b(4));
    let ascii = |out: &mut Vec<u8>, tag: u16, letter: u8| {
        out.extend_from_slice(&u16b(tag));
        out.extend_from_slice(&u16b(2));
        out.extend_from_slice(&u32b(2));
        out.extend_from_slice(&[letter, 0, 0, 0]);
    };
    let rational = |out: &mut Vec<u8>, tag: u16, at: u32| {
        out.extend_from_slice(&u16b(tag));
        out.extend_from_slice(&u16b(5));
        out.extend_from_slice(&u32b(3));
        out.extend_from_slice(&u32b(at));
    };
    ascii(&mut out, 1, latitude_ref);
    rational(&mut out, 2, 80);
    ascii(&mut out, 3, longitude_ref);
    rational(&mut out, 4, 104);
    out.extend_from_slice(&u32b(0));
    assert_eq!(out.len(), 80);
    for (numerator, denominator) in latitude.into_iter().chain(longitude) {
        out.extend_from_slice(&u32b(numerator));
        out.extend_from_slice(&u32b(denominator));
    }
    out
}

const MOSCOW_LATITUDE: Dms = [(55, 1), (45, 1), (2106, 100)];
const MOSCOW_LONGITUDE: Dms = [(37, 1), (37, 1), (1728, 100)];

fn moscow_degrees() -> (f64, f64) {
    (55.0 + 45.0 / 60.0 + 21.06 / 3600.0, 37.0 + 37.0 / 60.0 + 17.28 / 3600.0)
}

fn assert_place(place: Option<LibraryAssetPlace>, latitude: f64, longitude: f64) {
    let place = place.expect("a place");
    assert!((place.latitude - latitude).abs() < 1e-9, "{place:?}");
    assert!((place.longitude - longitude).abs() < 1e-9, "{place:?}");
}

#[test]
fn gps_rationals_become_signed_degrees_in_both_byte_orders() {
    let (latitude, longitude) = moscow_degrees();
    for little in [true, false] {
        let north_east = tiff(little, b'N', MOSCOW_LATITUDE, b'E', MOSCOW_LONGITUDE);
        assert_place(tiff_gps_place(&north_east), latitude, longitude);
        let south_west = tiff(little, b'S', MOSCOW_LATITUDE, b'W', MOSCOW_LONGITUDE);
        assert_place(tiff_gps_place(&south_west), -latitude, -longitude);
    }
}

#[test]
fn a_zero_denominator_an_out_of_range_value_or_null_island_is_no_place() {
    let broken = tiff(true, b'N', [(55, 0), (0, 1), (0, 1)], b'E', MOSCOW_LONGITUDE);
    assert_eq!(tiff_gps_place(&broken), None);
    let too_far = tiff(true, b'N', [(91, 1), (0, 1), (0, 1)], b'E', MOSCOW_LONGITUDE);
    assert_eq!(tiff_gps_place(&too_far), None);
    let no_fix = tiff(true, b'N', [(0, 1), (0, 1), (0, 1)], b'E', [(0, 1), (0, 1), (0, 1)]);
    assert_eq!(tiff_gps_place(&no_fix), None);
}

#[test]
fn a_tiff_without_a_gps_pointer_has_no_place() {
    let mut plain = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
    plain.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    assert_eq!(tiff_gps_place(&plain), None);
    assert_eq!(tiff_gps_place(b"not a tiff"), None);
}

fn jpeg_with(tiff: &[u8]) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8];
    out.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
    out.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((2 + 6 + tiff.len()) as u16).to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(tiff);
    out.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]);
    out
}

#[test]
fn a_jpeg_hands_out_the_tiff_of_its_exif_segment() {
    let block = tiff(false, b'N', MOSCOW_LATITUDE, b'E', MOSCOW_LONGITUDE);
    let found = exif_tiff_of(&mut Cursor::new(jpeg_with(&block)));
    assert_eq!(found.as_deref(), Some(block.as_slice()));
}

#[test]
fn a_jpeg_without_exif_or_a_foreign_file_has_none() {
    let bare = vec![0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9, 0, 0, 0, 0];
    assert_eq!(exif_tiff_of(&mut Cursor::new(bare)), None);
    assert_eq!(exif_tiff_of(&mut Cursor::new(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec())), None);
}

fn bmff_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = ((8 + body.len()) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

fn infe_v2(id: u16, item_type: &[u8; 4]) -> Vec<u8> {
    let mut body = vec![2, 0, 0, 0];
    body.extend_from_slice(&id.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(item_type);
    body.push(0);
    bmff_box(b"infe", &body)
}

/// test helper, an iloc v1 with four byte offsets and lengths and one extent for item 2
fn iloc_v1(construction: u16, offset: u32, length: u32) -> Vec<u8> {
    let mut body = vec![1, 0, 0, 0, 0x44, 0x00];
    body.extend_from_slice(&1u16.to_be_bytes());
    body.extend_from_slice(&2u16.to_be_bytes());
    body.extend_from_slice(&construction.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(&1u16.to_be_bytes());
    body.extend_from_slice(&offset.to_be_bytes());
    body.extend_from_slice(&length.to_be_bytes());
    bmff_box(b"iloc", &body)
}

fn exif_item(tiff: &[u8]) -> Vec<u8> {
    let mut item = 6u32.to_be_bytes().to_vec();
    item.extend_from_slice(b"Exif\0\0");
    item.extend_from_slice(tiff);
    item
}

fn heif_with(tiff: &[u8], in_idat: bool) -> Vec<u8> {
    let ftyp = bmff_box(b"ftyp", b"heic\0\0\0\0mif1heic");
    let mut iinf = vec![0, 0, 0, 0];
    iinf.extend_from_slice(&2u16.to_be_bytes());
    iinf.extend_from_slice(&infe_v2(1, b"hvc1"));
    iinf.extend_from_slice(&infe_v2(2, b"Exif"));
    let iinf = bmff_box(b"iinf", &iinf);
    let item = exif_item(tiff);
    let meta_of = |iloc: Vec<u8>, idat: Option<Vec<u8>>| {
        let mut body = vec![0, 0, 0, 0];
        body.extend_from_slice(&iinf);
        body.extend_from_slice(&iloc);
        if let Some(idat) = idat {
            body.extend_from_slice(&idat);
        }
        bmff_box(b"meta", &body)
    };
    let mut out = ftyp.clone();
    if in_idat {
        let meta = meta_of(iloc_v1(1, 0, item.len() as u32), Some(bmff_box(b"idat", &item)));
        out.extend_from_slice(&meta);
        out.extend_from_slice(&bmff_box(b"mdat", b"pixels"));
    } else {
        let probe = meta_of(iloc_v1(0, 0, 0), None);
        let offset = (ftyp.len() + probe.len() + 8) as u32;
        out.extend_from_slice(&meta_of(iloc_v1(0, offset, item.len() as u32), None));
        out.extend_from_slice(&bmff_box(b"mdat", &item));
    }
    out
}

#[test]
fn a_heif_hands_out_the_tiff_of_its_exif_item_from_mdat_or_idat() {
    let block = tiff(true, b'S', MOSCOW_LATITUDE, b'W', MOSCOW_LONGITUDE);
    for in_idat in [false, true] {
        let found = exif_tiff_of(&mut Cursor::new(heif_with(&block, in_idat)));
        assert_eq!(found.as_deref(), Some(block.as_slice()), "in_idat {in_idat}");
        let (latitude, longitude) = moscow_degrees();
        assert_place(tiff_gps_place(&found.unwrap_or_default()), -latitude, -longitude);
    }
}

#[test]
fn iloc_names_no_extents_for_an_unknown_item() {
    let iloc = iloc_v1(0, 100, 20);
    let body = &iloc[8..];
    assert_eq!(
        item_location(body, 2),
        Some(ItemExtents {
            in_idat: false,
            extents: vec![(100, 20)],
        })
    );
    assert_eq!(item_location(body, 7), None);
}

/// test helper, a tiff block whose ifd0 points at an exif ifd with DateTimeOriginal as stamp
/// count includes the closing nul
fn tiff_with_date(little: bool, stamp: &[u8]) -> Vec<u8> {
    let u16b = |value: u16| if little { value.to_le_bytes() } else { value.to_be_bytes() };
    let u32b = |value: u32| if little { value.to_le_bytes() } else { value.to_be_bytes() };
    let mut out = Vec::new();
    out.extend_from_slice(if little { b"II" } else { b"MM" });
    out.extend_from_slice(&u16b(42));
    out.extend_from_slice(&u32b(8));
    out.extend_from_slice(&u16b(1));
    out.extend_from_slice(&u16b(0x8769));
    out.extend_from_slice(&u16b(4));
    out.extend_from_slice(&u32b(1));
    out.extend_from_slice(&u32b(26));
    out.extend_from_slice(&u32b(0));
    out.extend_from_slice(&u16b(1));
    out.extend_from_slice(&u16b(0x9003));
    out.extend_from_slice(&u16b(2));
    out.extend_from_slice(&u32b(stamp.len() as u32 + 1));
    if stamp.len() < 4 {
        let mut inline = [0u8; 4];
        inline[..stamp.len()].copy_from_slice(stamp);
        out.extend_from_slice(&inline);
        out.extend_from_slice(&u32b(0));
        return out;
    }
    out.extend_from_slice(&u32b(44));
    out.extend_from_slice(&u32b(0));
    assert_eq!(out.len(), 44);
    out.extend_from_slice(stamp);
    out.push(0);
    out
}

#[test]
fn date_time_original_comes_out_as_written_in_both_byte_orders() {
    for little in [true, false] {
        let block = tiff_with_date(little, b"2024:07:15 18:42:05");
        assert_eq!(tiff_original_date(&block).as_deref(), Some("2024:07:15 18:42:05"));
    }
}

#[test]
fn a_jpeg_hands_out_its_date_time_original() {
    let block = tiff_with_date(true, b"2023:01:02 03:04:05");
    let found = exif_tiff_of(&mut Cursor::new(jpeg_with(&block)))
        .and_then(|tiff| tiff_original_date(&tiff));
    assert_eq!(found.as_deref(), Some("2023:01:02 03:04:05"));
}

#[test]
fn a_tiff_without_an_exif_pointer_or_an_empty_date_has_no_date() {
    let gps_only = tiff(true, b'N', MOSCOW_LATITUDE, b'E', MOSCOW_LONGITUDE);
    assert_eq!(tiff_original_date(&gps_only), None);
    assert_eq!(tiff_original_date(&tiff_with_date(true, b"")), None);
    assert_eq!(tiff_original_date(b"not a tiff"), None);
}

#[test]
fn a_short_value_is_read_from_the_entry_itself() {
    assert_eq!(tiff_original_date(&tiff_with_date(false, b"ab")).as_deref(), Some("ab"));
}
