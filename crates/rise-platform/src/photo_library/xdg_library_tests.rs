use std::io::Cursor;
use std::path::{Path, PathBuf};

use super::folder_library::ThumbnailBox;
use super::xdg_library::*;

#[test]
fn md5_matches_rfc_1321() {
    assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
    assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    assert_eq!(
        md5_hex(b"The quick brown fox jumps over the lazy dog"),
        "9e107d9d372bb6826bd81d3542a419d6"
    );
    assert_eq!(
        md5_hex(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"),
        "57edf4a22be3c955ac49da2e2107b67a"
    );
}

#[test]
fn thumbnail_names_follow_the_spec_example() {
    let uri = file_uri("/home/jens/photos/me.png");
    assert_eq!(uri, "file:///home/jens/photos/me.png");
    assert_eq!(thumbnail_file_name(&uri), "c6ee772d9e49320e97ec29a7eb5b1697.png");
}

#[test]
fn file_uris_escape_like_glib() {
    assert_eq!(file_uri("/a b/c#d%.jpg"), "file:///a%20b/c%23d%25.jpg");
    assert_eq!(file_uri("/x/(1)+@,;=.png"), "file:///x/(1)+@,;=.png");
    assert_eq!(file_uri("/\u{424}\u{43e}\u{442}\u{43e}"), "file:///%D0%A4%D0%BE%D1%82%D0%BE");
}

#[test]
fn user_dirs_are_read_from_the_file() {
    let home = Path::new("/home/u");
    let file = "# written by xdg-user-dirs-update\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_PICTURES_DIR=\"$HOME/Bilder\"\nXDG_VIDEOS_DIR=\"/mnt/media/Videos\"\n";
    assert_eq!(user_dirs_value(file, XDG_PICTURES_KEY, home), Some(Some(PathBuf::from("/home/u/Bilder"))));
    assert_eq!(user_dirs_value(file, XDG_VIDEOS_KEY, home), Some(Some(PathBuf::from("/mnt/media/Videos"))));
    assert_eq!(user_dirs_value(file, "XDG_MUSIC_DIR", home), None);
    let disabled = "XDG_PICTURES_DIR=\"$HOME/\"\n";
    assert_eq!(user_dirs_value(disabled, XDG_PICTURES_KEY, home), Some(None));
    assert_eq!(user_dir_or_default(Some(disabled), XDG_PICTURES_KEY, home, XDG_PICTURES_DEFAULT), None);
    assert_eq!(
        user_dir_or_default(None, XDG_VIDEOS_KEY, home, XDG_VIDEOS_DEFAULT),
        Some(PathBuf::from("/home/u/Videos"))
    );
    let escaped = "XDG_PICTURES_DIR=\"$HOME/My \\\"Pics\\\"\"\n";
    assert_eq!(
        user_dirs_value(escaped, XDG_PICTURES_KEY, home),
        Some(Some(PathBuf::from("/home/u/My \"Pics\"")))
    );
}

#[test]
fn base_dirs_need_an_absolute_value() {
    use std::ffi::OsStr;
    let home = Path::new("/home/u");
    assert_eq!(xdg_base_dir(Some(OsStr::new("/c")), home, ".cache"), PathBuf::from("/c"));
    assert_eq!(xdg_base_dir(Some(OsStr::new("rel")), home, ".cache"), PathBuf::from("/home/u/.cache"));
    assert_eq!(xdg_base_dir(None, home, ".config"), PathBuf::from("/home/u/.config"));
}

fn square(edge: u32) -> ThumbnailBox {
    ThumbnailBox {
        width: edge,
        height: edge,
        crop_to_square: true,
    }
}

#[test]
fn thumbnail_folders_are_tried_big_enough_first() {
    let names: Vec<&str> = thumbnail_candidates(square(256)).iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["large", "x-large", "xx-large", "normal"]);
    let names: Vec<&str> = thumbnail_candidates(square(2000)).iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["xx-large", "x-large", "large", "normal"]);
}

#[test]
fn a_thumbnail_covers_a_box_only_without_upscaling() {
    assert!(image_covers_box(256, 192, ThumbnailBox { width: 256, height: 256, crop_to_square: false }));
    assert!(!image_covers_box(256, 192, square(256)));
    assert!(image_covers_box(341, 256, square(256)));
}

#[test]
fn a_thumbnail_is_fresh_only_for_the_same_mtime_and_uri() {
    let text = vec![
        ("Thumb::URI".to_owned(), "file:///a.jpg".to_owned()),
        ("Thumb::MTime".to_owned(), "1700000000".to_owned()),
    ];
    assert!(thumbnail_is_fresh(&text, "file:///a.jpg", 1_700_000_000));
    assert!(!thumbnail_is_fresh(&text, "file:///a.jpg", 1_700_000_001));
    assert!(!thumbnail_is_fresh(&text, "file:///b.jpg", 1_700_000_000));
    assert!(!thumbnail_is_fresh(&[], "file:///a.jpg", 1_700_000_000));
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(data);
    chunk.extend_from_slice(&[0, 0, 0, 0]);
    chunk
}

#[test]
fn png_headers_and_text_are_read() {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = 640u32.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&480u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    png.extend(png_chunk(b"IHDR", &ihdr));
    png.extend(png_chunk(b"tEXt", b"Thumb::MTime\x001700000000"));
    png.extend(png_chunk(b"IEND", b""));
    assert_eq!(image_header_size(&png), Some((640, 480)));
    assert_eq!(
        png_text_chunks(&png),
        vec![("Thumb::MTime".to_owned(), "1700000000".to_owned())]
    );
}

#[test]
fn jpeg_size_turns_with_the_exif_orientation() {
    let mut exif = b"Exif\0\0MM\0\x2a\0\0\0\x08".to_vec();
    exif.extend_from_slice(&1u16.to_be_bytes());
    exif.extend_from_slice(&0x0112u16.to_be_bytes());
    exif.extend_from_slice(&3u16.to_be_bytes());
    exif.extend_from_slice(&1u32.to_be_bytes());
    exif.extend_from_slice(&[0, 6, 0, 0]);
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
    jpeg.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    jpeg.extend_from_slice(&exif);
    jpeg.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8]);
    jpeg.extend_from_slice(&3000u16.to_be_bytes());
    jpeg.extend_from_slice(&4000u16.to_be_bytes());
    jpeg.extend_from_slice(&[1, 1, 0x11, 0]);
    assert_eq!(image_header_size(&jpeg), Some((3000, 4000)));
    let plain = [&[0xFF, 0xD8][..], &jpeg[4 + 2 + exif.len()..]].concat();
    assert_eq!(image_header_size(&plain), Some((4000, 3000)));
}

#[test]
fn gif_bmp_and_webp_sizes() {
    let gif = [b"GIF89a".as_slice(), &[0x40, 0x01, 0xF0, 0x00]].concat();
    assert_eq!(image_header_size(&gif), Some((320, 240)));
    let mut bmp = vec![0u8; 26];
    bmp[..2].copy_from_slice(b"BM");
    bmp[18..22].copy_from_slice(&100i32.to_le_bytes());
    bmp[22..26].copy_from_slice(&(-50i32).to_le_bytes());
    assert_eq!(image_header_size(&bmp), Some((100, 50)));
    let mut webp = vec![0u8; 30];
    webp[..4].copy_from_slice(b"RIFF");
    webp[8..12].copy_from_slice(b"WEBP");
    webp[12..16].copy_from_slice(b"VP8X");
    webp[24..27].copy_from_slice(&[0x7F, 0x07, 0x00]);
    webp[27..30].copy_from_slice(&[0x37, 0x04, 0x00]);
    assert_eq!(image_header_size(&webp), Some((1920, 1080)));
    assert_eq!(image_header_size(b"not an image"), None);
}

fn bmff_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut b = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
    b.extend_from_slice(kind);
    b.extend_from_slice(payload);
    b
}

fn tkhd_v0(width: u32, height: u32, turned: bool) -> Vec<u8> {
    let mut tkhd = vec![0u8; 84];
    let (a, b) = if turned { (0i32, 0x10000i32) } else { (0x10000, 0) };
    tkhd[40..44].copy_from_slice(&a.to_be_bytes());
    tkhd[44..48].copy_from_slice(&b.to_be_bytes());
    tkhd[76..80].copy_from_slice(&(width << 16).to_be_bytes());
    tkhd[80..84].copy_from_slice(&(height << 16).to_be_bytes());
    tkhd
}

#[test]
fn mp4_size_and_duration_come_from_moov() {
    let mut mvhd = vec![0u8; 100];
    mvhd[12..16].copy_from_slice(&600u32.to_be_bytes());
    mvhd[16..20].copy_from_slice(&9000u32.to_be_bytes());
    let audio = bmff_box(b"trak", &bmff_box(b"tkhd", &tkhd_v0(0, 0, false)));
    let video = bmff_box(b"trak", &bmff_box(b"tkhd", &tkhd_v0(1920, 1080, true)));
    let moov = bmff_box(b"moov", &[bmff_box(b"mvhd", &mvhd), audio, video].concat());
    let file = [bmff_box(b"ftyp", b"isom\0\0\0\0"), bmff_box(b"mdat", &[0; 32]), moov].concat();
    let header = bmff_video_header(&mut Cursor::new(file)).expect("moov parses");
    assert_eq!((header.width, header.height), (1080, 1920));
    assert!((header.duration_secs - 15.0).abs() < 1e-9);
    assert!(bmff_video_header(&mut Cursor::new(bmff_box(b"ftyp", b"isom"))).is_none());
    assert!(is_bmff_extension(Path::new("/v/CLIP.MOV")));
    assert!(!is_bmff_extension(Path::new("/v/clip.mkv")));
}

#[test]
fn scaling_crops_the_centre_square_and_averages() {
    let mut bgra = Vec::new();
    for _ in 0..2 {
        for x in 0..4 {
            let v = if x == 1 || x == 2 { 255 } else { 0 };
            bgra.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let image = scale_bgra_to_box(4, 2, &bgra, square(1)).expect("scales");
    assert_eq!((image.width, image.height), (1, 1));
    assert_eq!(image.bgra, vec![255, 255, 255, 255]);
    let fit = ThumbnailBox { width: 2, height: 2, crop_to_square: false };
    let image = scale_bgra_to_box(4, 2, &bgra, fit).expect("scales");
    assert_eq!((image.width, image.height), (2, 1));
    assert_eq!(&image.bgra[..4], &[128, 128, 128, 255]);
    let image = scale_bgra_to_box(4, 2, &bgra, ThumbnailBox { width: 64, height: 64, crop_to_square: false })
        .expect("scales");
    assert_eq!((image.width, image.height), (4, 2));
    let edge = [0, 0, 0, 0, 200, 100, 50, 255];
    let image = scale_bgra_to_box(2, 1, &edge, ThumbnailBox { width: 1, height: 1, crop_to_square: false })
        .expect("scales");
    assert_eq!(image.bgra, vec![200, 100, 50, 128]);
    assert!(scale_bgra_to_box(2, 2, &edge, fit).is_none());
}

fn inotify_record(wd: i32, mask: u32, name: &str) -> Vec<u8> {
    let mut padded = name.as_bytes().to_vec();
    if !padded.is_empty() {
        padded.push(0);
        while padded.len() % 16 != 0 {
            padded.push(0);
        }
    }
    let mut record = wd.to_ne_bytes().to_vec();
    record.extend_from_slice(&mask.to_ne_bytes());
    record.extend_from_slice(&0u32.to_ne_bytes());
    record.extend_from_slice(&(padded.len() as u32).to_ne_bytes());
    record.extend_from_slice(&padded);
    record
}

#[test]
fn inotify_records_are_split_and_new_folders_seen() {
    let buffer = [
        inotify_record(1, IN_CLOSE_WRITE, "IMG_0001.jpg"),
        inotify_record(2, IN_CREATE | IN_ISDIR, "2026"),
        inotify_record(-1, IN_Q_OVERFLOW, ""),
    ]
    .concat();
    let records = inotify_records(&buffer);
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].name, "IMG_0001.jpg");
    assert!(!records[0].is_new_dir());
    assert!(records[1].is_new_dir());
    assert_eq!(records[2].mask, IN_Q_OVERFLOW);
    assert_eq!(records[2].name, "");
    assert_eq!(inotify_records(&buffer[..buffer.len() - 3]).len(), 2);
}
