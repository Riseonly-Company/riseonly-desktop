use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use super::folder_library::ThumbnailBox;
use super::photo_library::LibraryImage;

pub const XDG_PICTURES_KEY: &str = "XDG_PICTURES_DIR";
pub const XDG_VIDEOS_KEY: &str = "XDG_VIDEOS_DIR";
pub const XDG_PICTURES_DEFAULT: &str = "Pictures";
pub const XDG_VIDEOS_DEFAULT: &str = "Videos";

/// base dir variable, only an absolute value counts otherwise home plus the fallback
pub fn xdg_base_dir(value: Option<&OsStr>, home: &Path, fallback: &str) -> PathBuf {
    match value.map(Path::new) {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        _ => home.join(fallback),
    }
}

/// one XDG_NAME_DIR line of user-dirs.dirs, value is HOME/rel or an absolute path
/// none means the key is missing so the default applies
/// some none means malformed or equal to home
pub fn user_dirs_value(contents: &str, key: &str, home: &Path) -> Option<Option<PathBuf>> {
    let mut found = None;
    for line in contents.lines() {
        let line = trimming_whitespaces_and_newlines(line);
        if line.starts_with('#') {
            continue;
        }
        let Some(rest) = line.strip_prefix(key) else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = trimming_whitespaces_and_newlines(value);
        let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
            found = Some(None);
            continue;
        };
        let mut unquoted = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(next) = chars.next() {
                    unquoted.push(next);
                }
            } else {
                unquoted.push(c);
            }
        }
        found = Some(Some(unquoted));
    }
    let Some(value) = found? else {
        return Some(None);
    };
    let path = if let Some(rel) = value.strip_prefix("$HOME") {
        let rel = rel.trim_start_matches('/');
        if rel.is_empty() {
            return Some(None);
        }
        home.join(rel)
    } else if value.starts_with('/') {
        PathBuf::from(value)
    } else {
        return Some(None);
    };
    Some((path != home).then_some(path))
}

/// folder for key, what user-dirs.dirs names
/// the xdg default when the file or key is missing, none when the file disables it
pub fn user_dir_or_default(contents: Option<&str>, key: &str, home: &Path, default: &str) -> Option<PathBuf> {
    match contents.and_then(|contents| user_dirs_value(contents, key, home)) {
        Some(named) => named,
        None => Some(home.join(default)),
    }
}

/// canonical file uri the thumbnail cache is keyed by, escaped like g_filename_to_uri does
/// unreserved chars and a few symbols stay, everything else is %XX
pub fn file_uri(path: &str) -> String {
    const KEEP: &[u8] = b"-._~!$&'()*+,;=:@/";
    let mut uri = String::with_capacity(path.len() + 7);
    uri.push_str("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || KEEP.contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// thumbnail file name, lowercase md5 of the uri plus .png
pub fn thumbnail_file_name(uri: &str) -> String {
    format!("{}.png", md5_hex(uri.as_bytes()))
}

pub const THUMBNAIL_SIZES: [(&str, u32); 4] =
    [("normal", 128), ("large", 256), ("x-large", 512), ("xx-large", 1024)];

/// size folders to look in for a box, big enough ones smallest first
/// then the smaller ones biggest first (a degraded first answer while the original decodes)
pub fn thumbnail_candidates(target: ThumbnailBox) -> Vec<(&'static str, u32)> {
    let needed = target.width.max(target.height);
    let mut big: Vec<(&str, u32)> = THUMBNAIL_SIZES.iter().copied().filter(|(_, edge)| *edge >= needed).collect();
    let mut small: Vec<(&str, u32)> = THUMBNAIL_SIZES.iter().copied().filter(|(_, edge)| *edge < needed).collect();
    small.reverse();
    big.append(&mut small);
    big
}

/// true when a decoded picture of width x height is sharp enough for the box without upscaling
pub fn image_covers_box(width: u32, height: u32, target: ThumbnailBox) -> bool {
    if target.crop_to_square {
        width.min(height) >= target.width.min(target.height)
    } else {
        width >= target.width || height >= target.height
    }
}

/// thumbnail standard freshness test
/// thumb mtime equals file mtime in whole seconds and thumb uri if present is the file uri
pub fn thumbnail_is_fresh(text: &[(String, String)], uri: &str, mtime_secs: i64) -> bool {
    let value = |key: &str| text.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    let mtime_ok = value("Thumb::MTime")
        .and_then(|v| trimming_whitespaces_and_newlines(v).parse::<f64>().ok())
        .is_some_and(|v| v.trunc() as i64 == mtime_secs);
    let uri_ok = value("Thumb::URI").is_none_or(|v| v == uri);
    mtime_ok && uri_ok
}

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// tEXt chunks of a png (keyword and latin1 text), empty for anything else
pub fn png_text_chunks(bytes: &[u8]) -> Vec<(String, String)> {
    let mut text = Vec::new();
    if !bytes.starts_with(&PNG_SIGNATURE) {
        return text;
    }
    let mut at = PNG_SIGNATURE.len();
    while let Some(header) = bytes.get(at..at + 8) {
        let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let kind = &header[4..8];
        let Some(data) = at.checked_add(8 + len).and_then(|end| bytes.get(at + 8..end)) else {
            break;
        };
        if kind == b"tEXt" {
            if let Some(nul) = data.iter().position(|b| *b == 0) {
                let latin1 = |part: &[u8]| part.iter().map(|b| char::from(*b)).collect::<String>();
                text.push((latin1(&data[..nul]), latin1(&data[nul + 1..])));
            }
        }
        if kind == b"IEND" {
            break;
        }
        at += 12 + len;
    }
    text
}

fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    let b = bytes.get(at..at + 2)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

fn le16(bytes: &[u8], at: usize) -> Option<u16> {
    let b = bytes.get(at..at + 2)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 4)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn be64(bytes: &[u8], at: usize) -> Option<u64> {
    let b = bytes.get(at..at + 8)?;
    Some(u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

fn le24(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 3)?;
    Some(u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16)
}

pub const IMAGE_HEADER_READ: usize = 256 * 1024;

/// pixel size of a picture as shown (exif rotation applied) from its first bytes
pub fn image_header_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let size = if bytes.starts_with(&PNG_SIGNATURE) {
        (bytes.get(12..16)? == b"IHDR").then_some(())?;
        (be32(bytes, 16)?, be32(bytes, 20)?)
    } else if bytes.starts_with(b"GIF8") {
        (u32::from(le16(bytes, 6)?), u32::from(le16(bytes, 8)?))
    } else if bytes.starts_with(b"BM") {
        let width = le32(bytes, 18)? as i32;
        let height = le32(bytes, 22)? as i32;
        (width.unsigned_abs(), height.unsigned_abs())
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        jpeg_size(bytes)?
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12)? == b"WEBP" {
        webp_size(bytes)?
    } else {
        return None;
    };
    (size.0 > 0 && size.1 > 0).then_some(size)
}

fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    let mut orientation = 1;
    loop {
        while *bytes.get(at)? == 0xFF && *bytes.get(at + 1)? == 0xFF {
            at += 1;
        }
        if *bytes.get(at)? != 0xFF {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            at += 2;
            continue;
        }
        if marker == 0xD9 || marker == 0xDA {
            return None;
        }
        let len = usize::from(be16(bytes, at + 2)?);
        if len < 2 {
            return None;
        }
        let segment = at + 4;
        let is_sof = matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if marker == 0xE1 {
            if let Some(payload) = bytes.get(segment..at + 2 + len) {
                if let Some(found) = exif_orientation(payload) {
                    orientation = found;
                }
            }
        }
        if is_sof {
            let height = u32::from(be16(bytes, segment + 1)?);
            let width = u32::from(be16(bytes, segment + 3)?);
            return Some(oriented(width, height, orientation));
        }
        at += 2 + len;
    }
}

/// exif orientations 5 to 8 turn the picture a quarter so width and height swap
pub fn oriented(width: u32, height: u32, orientation: u16) -> (u32, u32) {
    if (5..=8).contains(&orientation) {
        (height, width)
    } else {
        (width, height)
    }
}

pub fn exif_orientation(payload: &[u8]) -> Option<u16> {
    let tiff = payload.strip_prefix(b"Exif\0\0")?;
    let little = match tiff.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |at: usize| if little { le16(tiff, at) } else { be16(tiff, at) };
    let u32_at = |at: usize| if little { le32(tiff, at) } else { be32(tiff, at) };
    let ifd = u32_at(4)? as usize;
    let entries = usize::from(u16_at(ifd)?);
    for index in 0..entries {
        let entry = ifd + 2 + index * 12;
        if u16_at(entry)? == 0x0112 {
            let value = u16_at(entry + 8)?;
            return (1..=8).contains(&value).then_some(value);
        }
    }
    None
}

fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    match bytes.get(12..16)? {
        b"VP8 " => Some((
            u32::from(le16(bytes, 26)? & 0x3FFF),
            u32::from(le16(bytes, 28)? & 0x3FFF),
        )),
        b"VP8L" => {
            let b = bytes.get(21..25)?;
            let width = 1 + ((u32::from(b[1]) & 0x3F) << 8 | u32::from(b[0]));
            let height = 1
                + ((u32::from(b[3]) & 0x0F) << 10 | u32::from(b[2]) << 2 | (u32::from(b[1]) & 0xC0) >> 6);
            Some((width, height))
        }
        b"VP8X" => Some((1 + le24(bytes, 24)?, 1 + le24(bytes, 27)?)),
        _ => None,
    }
}

/// size and duration an iso bmff video (mp4 mov m4v 3gp) declares in its moov box
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct VideoHeader {
    pub width: u32,
    pub height: u32,
    pub duration_secs: f64,
}

pub const BMFF_MOOV_MAX: u64 = 64 * 1024 * 1024;

/// reads the top level boxes of an iso bmff file up to its moov and parses that
pub fn bmff_video_header(reader: &mut (impl Read + Seek)) -> Option<VideoHeader> {
    let end = reader.seek(SeekFrom::End(0)).ok()?;
    let mut at = 0u64;
    while at + 8 <= end {
        reader.seek(SeekFrom::Start(at)).ok()?;
        let mut header = [0u8; 16];
        reader.read_exact(&mut header[..8]).ok()?;
        let mut size = u64::from(be32(&header, 0)?);
        let mut header_len = 8u64;
        if size == 1 {
            reader.read_exact(&mut header[8..16]).ok()?;
            size = be64(&header, 8)?;
            header_len = 16;
        } else if size == 0 {
            size = end - at;
        }
        if size < header_len {
            return None;
        }
        if &header[4..8] == b"moov" {
            let len = size - header_len;
            if len > BMFF_MOOV_MAX {
                return None;
            }
            let mut moov = vec![0u8; len as usize];
            reader.read_exact(&mut moov).ok()?;
            return parse_moov(&moov);
        }
        at = at.checked_add(size)?;
    }
    None
}

/// child boxes of a box payload as type and payload
fn child_boxes(bytes: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut boxes = Vec::new();
    let mut at = 0usize;
    while let Some(size) = be32(bytes, at) {
        let Some(kind) = bytes.get(at + 4..at + 8) else {
            break;
        };
        let (size, header_len) = match size {
            1 => match be64(bytes, at + 8).and_then(|s| usize::try_from(s).ok()) {
                Some(s) => (s, 16),
                None => break,
            },
            0 => (bytes.len() - at, 8),
            s => (s as usize, 8),
        };
        let Some(payload) = (size >= header_len)
            .then(|| at.checked_add(size))
            .flatten()
            .and_then(|end| bytes.get(at + header_len..end))
        else {
            break;
        };
        boxes.push(([kind[0], kind[1], kind[2], kind[3]], payload));
        at += size;
    }
    boxes
}

pub fn parse_moov(moov: &[u8]) -> Option<VideoHeader> {
    let mut header = VideoHeader::default();
    for (kind, payload) in child_boxes(moov) {
        match &kind {
            b"mvhd" => {
                let (timescale, duration) = if *payload.first()? == 1 {
                    (be32(payload, 20)?, be64(payload, 24)?)
                } else {
                    (be32(payload, 12)?, u64::from(be32(payload, 16)?))
                };
                if timescale > 0 && duration != u64::MAX && duration != u64::from(u32::MAX) {
                    header.duration_secs = duration as f64 / f64::from(timescale);
                }
            }
            b"trak" if header.width == 0 => {
                for (inner, tkhd) in child_boxes(payload) {
                    if &inner != b"tkhd" {
                        continue;
                    }
                    let base = if *tkhd.first()? == 1 { 52 } else { 40 };
                    let matrix_a = be32(tkhd, base)? as i32;
                    let matrix_b = be32(tkhd, base + 4)? as i32;
                    let width = be32(tkhd, base + 36)? >> 16;
                    let height = be32(tkhd, base + 40)? >> 16;
                    if width > 0 && height > 0 {
                        let turned = matrix_a == 0 && matrix_b != 0;
                        (header.width, header.height) = if turned { (height, width) } else { (width, height) };
                    }
                }
            }
            _ => {}
        }
    }
    (header.width > 0 || header.duration_secs > 0.0).then_some(header)
}

/// true when the extension names an iso bmff container bmff_video_header reads
pub fn is_bmff_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| matches!(ext.as_str(), "mp4" | "mov" | "m4v" | "3gp"))
}

/// scales straight alpha bgra into the box, a square box crops the center square first
/// result fits inside the box and is never larger than the source
/// area average with alpha weighting so transparent pixels dont darken edges
pub fn scale_bgra_to_box(width: u32, height: u32, bgra: &[u8], target: ThumbnailBox) -> Option<LibraryImage> {
    let (width, height) = (width as usize, height as usize);
    if width == 0 || height == 0 || bgra.len() < width * height * 4 {
        return None;
    }
    let (x0, y0, region_w, region_h) = if target.crop_to_square {
        let side = width.min(height);
        ((width - side) / 2, (height - side) / 2, side, side)
    } else {
        (0, 0, width, height)
    };
    let scale = (f64::from(target.width.max(1)) / region_w as f64)
        .min(f64::from(target.height.max(1)) / region_h as f64)
        .min(1.0);
    let out_w = ((region_w as f64 * scale).round() as usize).clamp(1, region_w);
    let out_h = ((region_h as f64 * scale).round() as usize).clamp(1, region_h);
    let mut out = vec![0u8; out_w * out_h * 4];
    for oy in 0..out_h {
        let sy0 = y0 + oy * region_h / out_h;
        let sy1 = (y0 + (oy + 1) * region_h / out_h).max(sy0 + 1);
        for ox in 0..out_w {
            let sx0 = x0 + ox * region_w / out_w;
            let sx1 = (x0 + (ox + 1) * region_w / out_w).max(sx0 + 1);
            let mut sums = [0u64; 3];
            let mut alpha_sum = 0u64;
            let mut count = 0u64;
            for sy in sy0..sy1 {
                let row = sy * width * 4;
                for sx in sx0..sx1 {
                    let pixel = &bgra[row + sx * 4..row + sx * 4 + 4];
                    let alpha = u64::from(pixel[3]);
                    for (sum, channel) in sums.iter_mut().zip(&pixel[..3]) {
                        *sum += u64::from(*channel) * alpha;
                    }
                    alpha_sum += alpha;
                    count += 1;
                }
            }
            let at = (oy * out_w + ox) * 4;
            if alpha_sum > 0 {
                for (index, sum) in sums.iter().enumerate() {
                    out[at + index] = ((sum + alpha_sum / 2) / alpha_sum).min(255) as u8;
                }
            }
            out[at + 3] = ((alpha_sum + count / 2) / count) as u8;
        }
    }
    Some(LibraryImage {
        width: out_w as u32,
        height: out_h as u32,
        bgra: out,
    })
}

pub const IN_CLOSE_WRITE: u32 = 0x0000_0008;
pub const IN_MOVED_FROM: u32 = 0x0000_0040;
pub const IN_MOVED_TO: u32 = 0x0000_0080;
pub const IN_CREATE: u32 = 0x0000_0100;
pub const IN_DELETE: u32 = 0x0000_0200;
pub const IN_DELETE_SELF: u32 = 0x0000_0400;
pub const IN_MOVE_SELF: u32 = 0x0000_0800;
pub const IN_Q_OVERFLOW: u32 = 0x0000_4000;
pub const IN_IGNORED: u32 = 0x0000_8000;
pub const IN_ONLYDIR: u32 = 0x0100_0000;
pub const IN_DONT_FOLLOW: u32 = 0x0200_0000;
pub const IN_EXCL_UNLINK: u32 = 0x0400_0000;
pub const IN_ISDIR: u32 = 0x4000_0000;
pub const FOLDER_WATCH_MASK: u32 = IN_CLOSE_WRITE
    | IN_MOVED_FROM
    | IN_MOVED_TO
    | IN_CREATE
    | IN_DELETE
    | IN_DELETE_SELF
    | IN_MOVE_SELF
    | IN_ONLYDIR
    | IN_DONT_FOLLOW
    | IN_EXCL_UNLINK;
pub const FOLDER_WATCH_MAX_DIRS: usize = 2048;

/// one inotify event, wd and mask and cookie and len then len bytes of nul padded name
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InotifyRecord {
    pub wd: i32,
    pub mask: u32,
    pub name: String,
}

impl InotifyRecord {
    /// true when a folder appeared under a watched one and needs its own watch
    pub fn is_new_dir(&self) -> bool {
        self.mask & IN_ISDIR != 0 && self.mask & (IN_CREATE | IN_MOVED_TO) != 0
    }
}

/// records of one read of an inotify descriptor in host byte order, a truncated record ends the list
pub fn inotify_records(buffer: &[u8]) -> Vec<InotifyRecord> {
    let ne32 = |at: usize| -> Option<[u8; 4]> {
        let b = buffer.get(at..at + 4)?;
        Some([b[0], b[1], b[2], b[3]])
    };
    let mut records = Vec::new();
    let mut at = 0usize;
    while let (Some(wd), Some(mask), Some(len)) = (ne32(at), ne32(at + 4), ne32(at + 12)) {
        let len = u32::from_ne_bytes(len) as usize;
        let Some(name) = buffer.get(at + 16..at + 16 + len) else {
            break;
        };
        let name = name.split(|b| *b == 0).next().unwrap_or_default();
        records.push(InotifyRecord {
            wd: i32::from_ne_bytes(wd),
            mask: u32::from_ne_bytes(mask),
            name: String::from_utf8_lossy(name).into_owned(),
        });
        at += 16 + len;
    }
    records
}

const MD5_SHIFTS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15,
    21, 6, 10, 15, 21,
];

const MD5_K: [u32; 64] = [
    0xd76a_a478, 0xe8c7_b756, 0x2420_70db, 0xc1bd_ceee, 0xf57c_0faf, 0x4787_c62a, 0xa830_4613, 0xfd46_9501,
    0x6980_98d8, 0x8b44_f7af, 0xffff_5bb1, 0x895c_d7be, 0x6b90_1122, 0xfd98_7193, 0xa679_438e, 0x49b4_0821,
    0xf61e_2562, 0xc040_b340, 0x265e_5a51, 0xe9b6_c7aa, 0xd62f_105d, 0x0244_1453, 0xd8a1_e681, 0xe7d3_fbc8,
    0x21e1_cde6, 0xc337_07d6, 0xf4d5_0d87, 0x455a_14ed, 0xa9e3_e905, 0xfcef_a3f8, 0x676f_02d9, 0x8d2a_4c8a,
    0xfffa_3942, 0x8771_f681, 0x6d9d_6122, 0xfde5_380c, 0xa4be_ea44, 0x4bde_cfa9, 0xf6bb_4b60, 0xbebf_bc70,
    0x289b_7ec6, 0xeaa1_27fa, 0xd4ef_3085, 0x0488_1d05, 0xd9d4_d039, 0xe6db_99e5, 0x1fa2_7cf8, 0xc4ac_5665,
    0xf429_2244, 0x432a_ff97, 0xab94_23a7, 0xfc93_a039, 0x655b_59c3, 0x8f0c_cc92, 0xffef_f47d, 0x8584_5dd1,
    0x6fa8_7e4f, 0xfe2c_e6e0, 0xa301_4314, 0x4e08_11a1, 0xf753_7e82, 0xbd3a_f235, 0x2ad7_d2bb, 0xeb86_d391,
];

/// md5 as lowercase hex, the thumbnail standard names files by it
/// no crypto rides on it and the crate graph has no md5 crate
pub fn md5_hex(data: &[u8]) -> String {
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    for chunk in message.chunks_exact(64) {
        let words: Vec<u32> = chunk
            .chunks_exact(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(MD5_K[i]).wrapping_add(words[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(MD5_SHIFTS[i]));
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d]) {
            *slot = slot.wrapping_add(value);
        }
    }
    state
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
