use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::photo_library::{LibraryAsset, LibraryAssetPlace, LibraryMediaKind};

pub const JPEG_HEADER_MAX_BYTES: u64 = 4 * 1024 * 1024;
pub const HEIF_META_MAX_BYTES: u64 = 1024 * 1024;
pub const EXIF_MAX_BYTES: u64 = 256 * 1024;

/// assets of this kind created strictly after after_ms from a newest first snapshot, up to the limit
/// assets without a date never match
pub fn created_after_of_kind(
    all: &[LibraryAsset],
    kind: LibraryMediaKind,
    after_ms: i64,
    limit: usize,
) -> Vec<LibraryAsset> {
    let take = if limit == 0 { usize::MAX } else { limit };
    all.iter()
        .filter(|asset| kind.admits(asset.kind))
        .filter(|asset| asset.created_at_ms.is_some_and(|created| created > after_ms))
        .take(take)
        .cloned()
        .collect()
}

/// place from the exif gps block of a jpeg heic heif or avif picture
/// none when the file has no readable gps position
pub fn picture_place(path: &Path) -> Option<LibraryAssetPlace> {
    let mut file = File::open(path).ok()?;
    let tiff = exif_tiff_of(&mut file)?;
    tiff_gps_place(&tiff)
}

/// exif DateTimeOriginal of a picture as yyyy:MM:dd HH:mm:ss text
/// none when the file has no readable value
pub fn picture_original_date(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let tiff = exif_tiff_of(&mut file)?;
    tiff_original_date(&tiff)
}

/// tiff block of the exif in a jpeg or heif stream, starting at its II or MM byte order mark
pub fn exif_tiff_of(reader: &mut (impl Read + Seek)) -> Option<Vec<u8>> {
    let mut head = [0u8; 12];
    reader.seek(SeekFrom::Start(0)).ok()?;
    reader.read_exact(&mut head).ok()?;
    if head[0] == 0xFF && head[1] == 0xD8 {
        return jpeg_exif_tiff(reader);
    }
    if &head[4..8] == b"ftyp" {
        return heif_exif_tiff(reader);
    }
    None
}

fn jpeg_exif_tiff(reader: &mut (impl Read + Seek)) -> Option<Vec<u8>> {
    let mut at: u64 = 2;
    while at < JPEG_HEADER_MAX_BYTES {
        reader.seek(SeekFrom::Start(at)).ok()?;
        let mut marker = [0u8; 2];
        reader.read_exact(&mut marker).ok()?;
        if marker[0] != 0xFF {
            return None;
        }
        match marker[1] {
            0xFF => {
                at += 1;
                continue;
            }
            0xDA | 0xD9 => return None,
            0x01 | 0xD0..=0xD7 => {
                at += 2;
                continue;
            }
            _ => {}
        }
        let mut length = [0u8; 2];
        reader.read_exact(&mut length).ok()?;
        let length = u64::from(u16::from_be_bytes(length));
        if length < 2 {
            return None;
        }
        let payload_length = length - 2;
        if marker[1] == 0xE1 && payload_length > 6 {
            let mut payload = vec![0u8; payload_length.min(EXIF_MAX_BYTES) as usize];
            reader.read_exact(&mut payload).ok()?;
            if let Some(tiff) = payload.strip_prefix(b"Exif\0\0") {
                return Some(tiff.to_vec());
            }
        }
        at += 2 + length;
    }
    None
}

/// one iso bmff box, its four char type and where its body lies in the stream
struct BoxSpan {
    kind: [u8; 4],
    body_start: u64,
    body_end: u64,
}

fn box_at(reader: &mut (impl Read + Seek), at: u64, end: u64) -> Option<BoxSpan> {
    if at.checked_add(8)? > end {
        return None;
    }
    reader.seek(SeekFrom::Start(at)).ok()?;
    let mut header = [0u8; 8];
    reader.read_exact(&mut header).ok()?;
    let size = u64::from(u32::from_be_bytes([header[0], header[1], header[2], header[3]]));
    let kind = [header[4], header[5], header[6], header[7]];
    let (header_length, total) = match size {
        0 => (8, end - at),
        1 => {
            let mut large = [0u8; 8];
            reader.read_exact(&mut large).ok()?;
            (16, u64::from_be_bytes(large))
        }
        _ => (8, size),
    };
    if total < header_length {
        return None;
    }
    let body_end = at.checked_add(total)?.min(end);
    Some(BoxSpan {
        kind,
        body_start: at + header_length,
        body_end,
    })
}

fn heif_exif_tiff(reader: &mut (impl Read + Seek)) -> Option<Vec<u8>> {
    let file_end = reader.seek(SeekFrom::End(0)).ok()?;
    let mut at = 0;
    let meta = loop {
        let span = box_at(reader, at, file_end)?;
        if &span.kind == b"meta" {
            break span;
        }
        if span.body_end <= at {
            return None;
        }
        at = span.body_end;
    };
    let meta_length = meta.body_end.checked_sub(meta.body_start)?;
    if meta_length > HEIF_META_MAX_BYTES || meta_length < 4 {
        return None;
    }
    reader.seek(SeekFrom::Start(meta.body_start)).ok()?;
    let mut body = vec![0u8; meta_length as usize];
    reader.read_exact(&mut body).ok()?;
    let children = child_boxes(body.get(4..)?);
    let item_id = children
        .iter()
        .find(|(kind, _)| kind == b"iinf")
        .and_then(|(_, iinf)| exif_item_id(iinf))?;
    let idat = children.iter().find(|(kind, _)| kind == b"idat").map(|(_, data)| *data);
    let extents = children
        .iter()
        .find(|(kind, _)| kind == b"iloc")
        .and_then(|(_, iloc)| item_location(iloc, item_id))?;
    let item = read_item(reader, &extents, idat)?;
    let offset = be32(&item, 0)? as usize;
    let tiff_start = 4usize.checked_add(offset)?;
    item.get(tiff_start..).map(<[u8]>::to_vec)
}

/// child boxes of a box body held in memory
fn child_boxes(bytes: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut boxes = Vec::new();
    let mut at = 0usize;
    while at + 8 <= bytes.len() {
        let Some(size) = be32(bytes, at) else {
            break;
        };
        let kind = [bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]];
        let (header_length, total) = match size {
            0 => (8, bytes.len() - at),
            1 => match be64(bytes, at + 8).and_then(|large| usize::try_from(large).ok()) {
                Some(large) => (16, large),
                None => break,
            },
            _ => (8, size as usize),
        };
        if total < header_length || at + total > bytes.len() {
            break;
        }
        boxes.push((kind, &bytes[at + header_length..at + total]));
        at += total;
    }
    boxes
}

fn exif_item_id(iinf: &[u8]) -> Option<u32> {
    let version = *iinf.first()?;
    let entries_at = if version == 0 { 6 } else { 8 };
    for (kind, infe) in child_boxes(iinf.get(entries_at..)?) {
        if &kind != b"infe" {
            continue;
        }
        let infe_version = *infe.first()?;
        let (id, type_at) = match infe_version {
            2 => (u32::from(be16(infe, 4)?), 8),
            3 => (be32(infe, 4)?, 10),
            _ => continue,
        };
        if infe.get(type_at..type_at + 4)? == b"Exif" {
            return Some(id);
        }
    }
    None
}

/// where an item bytes lie, file offsets (construction 0) or offsets into idat (1)
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ItemExtents {
    pub in_idat: bool,
    pub extents: Vec<(u64, u64)>,
}

fn sized(bytes: &[u8], at: usize, size: u8) -> Option<u64> {
    match size {
        0 => Some(0),
        4 => be32(bytes, at).map(u64::from),
        8 => be64(bytes, at),
        _ => None,
    }
}

/// extents of wanted in an iloc body, versions 0 1 and 2
pub fn item_location(iloc: &[u8], wanted: u32) -> Option<ItemExtents> {
    let version = *iloc.first()?;
    if version > 2 {
        return None;
    }
    let sizes = *iloc.get(4)?;
    let offset_size = sizes >> 4;
    let length_size = sizes & 0x0F;
    let more = *iloc.get(5)?;
    let base_offset_size = more >> 4;
    let index_size = if version == 0 { 0 } else { more & 0x0F };
    let mut at = 6usize;
    let item_count = if version < 2 {
        let count = u32::from(be16(iloc, at)?);
        at += 2;
        count
    } else {
        let count = be32(iloc, at)?;
        at += 4;
        count
    };
    for _ in 0..item_count {
        let item_id = if version < 2 {
            let id = u32::from(be16(iloc, at)?);
            at += 2;
            id
        } else {
            let id = be32(iloc, at)?;
            at += 4;
            id
        };
        let construction = if version == 0 {
            0
        } else {
            let method = be16(iloc, at)? & 0x0F;
            at += 2;
            method
        };
        let data_reference = be16(iloc, at)?;
        at += 2;
        let base_offset = sized(iloc, at, base_offset_size)?;
        at += usize::from(base_offset_size);
        let extent_count = be16(iloc, at)?;
        at += 2;
        let mut extents = Vec::with_capacity(usize::from(extent_count));
        for _ in 0..extent_count {
            at += usize::from(index_size);
            let offset = sized(iloc, at, offset_size)?;
            at += usize::from(offset_size);
            let length = sized(iloc, at, length_size)?;
            at += usize::from(length_size);
            extents.push((base_offset.checked_add(offset)?, length));
        }
        if item_id == wanted {
            if data_reference != 0 || construction > 1 {
                return None;
            }
            return Some(ItemExtents {
                in_idat: construction == 1,
                extents,
            });
        }
    }
    None
}

fn read_item(
    reader: &mut (impl Read + Seek),
    location: &ItemExtents,
    idat: Option<&[u8]>,
) -> Option<Vec<u8>> {
    let mut item = Vec::new();
    for &(offset, length) in &location.extents {
        let room = EXIF_MAX_BYTES.checked_sub(item.len() as u64)?;
        if location.in_idat {
            let source = idat?;
            let start = usize::try_from(offset).ok()?;
            let end = if length == 0 {
                source.len()
            } else {
                start.checked_add(usize::try_from(length).ok()?)?
            };
            let part = source.get(start..end)?;
            if part.len() as u64 > room {
                return None;
            }
            item.extend_from_slice(part);
        } else {
            let file_end = reader.seek(SeekFrom::End(0)).ok()?;
            let length = if length == 0 { file_end.checked_sub(offset)? } else { length };
            if length > room || offset.checked_add(length)? > file_end {
                return None;
            }
            reader.seek(SeekFrom::Start(offset)).ok()?;
            let mut part = vec![0u8; length as usize];
            reader.read_exact(&mut part).ok()?;
            item.extend_from_slice(&part);
        }
    }
    Some(item)
}

const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_DATE_TIME_ORIGINAL: u16 = 0x9003;
const TAG_GPS_IFD: u16 = 0x8825;
const TAG_GPS_LATITUDE_REF: u16 = 0x0001;
const TAG_GPS_LATITUDE: u16 = 0x0002;
const TAG_GPS_LONGITUDE_REF: u16 = 0x0003;
const TAG_GPS_LONGITUDE: u16 = 0x0004;
const TYPE_ASCII: u16 = 2;
const TYPE_LONG: u16 = 4;
const TYPE_IFD: u16 = 13;
const TYPE_RATIONAL: u16 = 5;

/// reads the tiff block in its own byte order
struct Tiff<'a> {
    bytes: &'a [u8],
    little: bool,
}

/// one ifd entry, tag and type and count plus four bytes of value or offset
struct IfdEntry {
    tag: u16,
    kind: u16,
    count: u32,
    value_at: usize,
}

impl<'a> Tiff<'a> {
    fn new(bytes: &'a [u8]) -> Option<Self> {
        let little = match bytes.get(0..2)? {
            b"II" => true,
            b"MM" => false,
            _ => return None,
        };
        let tiff = Self { bytes, little };
        (tiff.u16_at(2)? == 42).then_some(tiff)
    }

    fn u16_at(&self, at: usize) -> Option<u16> {
        if self.little {
            le16(self.bytes, at)
        } else {
            be16(self.bytes, at)
        }
    }

    fn u32_at(&self, at: usize) -> Option<u32> {
        if self.little {
            le32(self.bytes, at)
        } else {
            be32(self.bytes, at)
        }
    }

    fn entries(&self, ifd: usize) -> Option<Vec<IfdEntry>> {
        let count = usize::from(self.u16_at(ifd)?);
        let mut entries = Vec::with_capacity(count);
        for index in 0..count {
            let at = ifd.checked_add(2 + index * 12)?;
            entries.push(IfdEntry {
                tag: self.u16_at(at)?,
                kind: self.u16_at(at + 2)?,
                count: self.u32_at(at + 4)?,
                value_at: at + 8,
            });
        }
        Some(entries)
    }

    /// first char of an ascii value, N S E or W for gps references
    fn ascii_first(&self, entry: &IfdEntry) -> Option<u8> {
        if entry.kind != TYPE_ASCII || entry.count == 0 {
            return None;
        }
        let at = if entry.count <= 4 {
            entry.value_at
        } else {
            self.u32_at(entry.value_at)? as usize
        };
        self.bytes.get(at).copied()
    }

    fn ascii_text(&self, entry: &IfdEntry) -> Option<String> {
        if entry.kind != TYPE_ASCII || entry.count == 0 {
            return None;
        }
        let count = entry.count as usize;
        let at = if count <= 4 {
            entry.value_at
        } else {
            self.u32_at(entry.value_at)? as usize
        };
        let raw = self.bytes.get(at..at.checked_add(count)?)?;
        let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
        let text = std::str::from_utf8(&raw[..end]).ok()?;
        (!text.is_empty()).then(|| text.to_owned())
    }

    /// offset of the sub ifd that ifd0 points at through the tag
    fn sub_ifd(&self, tag: u16) -> Option<usize> {
        let ifd0 = self.u32_at(4)? as usize;
        self.entries(ifd0)?
            .into_iter()
            .find(|entry| entry.tag == tag && matches!(entry.kind, TYPE_LONG | TYPE_IFD))
            .and_then(|entry| self.u32_at(entry.value_at))
            .map(|offset| offset as usize)
    }

    /// degrees minutes and seconds as one value in degrees
    fn degrees(&self, entry: &IfdEntry) -> Option<f64> {
        if entry.kind != TYPE_RATIONAL || entry.count < 3 {
            return None;
        }
        let at = self.u32_at(entry.value_at)? as usize;
        let mut parts = [0f64; 3];
        for (index, part) in parts.iter_mut().enumerate() {
            let numerator = self.u32_at(at.checked_add(index * 8)?)?;
            let denominator = self.u32_at(at.checked_add(index * 8 + 4)?)?;
            if denominator == 0 {
                return None;
            }
            *part = f64::from(numerator) / f64::from(denominator);
        }
        Some(parts[0] + parts[1] / 60.0 + parts[2] / 3600.0)
    }
}

/// gps position of a tiff block, latitude and longitude with references
/// south and west are negative, none without both values or out of range
/// also none at exactly 0 0 which cameras write with no fix
pub fn tiff_gps_place(tiff: &[u8]) -> Option<LibraryAssetPlace> {
    let tiff = Tiff::new(tiff)?;
    let gps_ifd = tiff.sub_ifd(TAG_GPS_IFD)?;
    let entries = tiff.entries(gps_ifd)?;
    let find = |tag: u16| entries.iter().find(|entry| entry.tag == tag);
    let mut latitude = tiff.degrees(find(TAG_GPS_LATITUDE)?)?;
    let mut longitude = tiff.degrees(find(TAG_GPS_LONGITUDE)?)?;
    if find(TAG_GPS_LATITUDE_REF).and_then(|entry| tiff.ascii_first(entry)) == Some(b'S') {
        latitude = -latitude;
    }
    if find(TAG_GPS_LONGITUDE_REF).and_then(|entry| tiff.ascii_first(entry)) == Some(b'W') {
        longitude = -longitude;
    }
    let valid = latitude.is_finite()
        && longitude.is_finite()
        && latitude.abs() <= 90.0
        && longitude.abs() <= 180.0
        && (latitude != 0.0 || longitude != 0.0);
    valid.then_some(LibraryAssetPlace { latitude, longitude })
}

/// exif DateTimeOriginal as written, without its closing nul
/// none without the pointer or the tag or a non empty ascii value
pub fn tiff_original_date(tiff: &[u8]) -> Option<String> {
    let tiff = Tiff::new(tiff)?;
    let exif_ifd = tiff.sub_ifd(TAG_EXIF_IFD)?;
    let entries = tiff.entries(exif_ifd)?;
    let entry = entries.iter().find(|entry| entry.tag == TAG_DATE_TIME_ORIGINAL)?;
    tiff.ascii_text(entry)
}

fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    let raw = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([raw[0], raw[1]]))
}

fn le16(bytes: &[u8], at: usize) -> Option<u16> {
    let raw = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([raw[0], raw[1]]))
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    let raw = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    let raw = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn be64(bytes: &[u8], at: usize) -> Option<u64> {
    let raw = bytes.get(at..at.checked_add(8)?)?;
    let mut word = [0u8; 8];
    word.copy_from_slice(raw);
    Some(u64::from_be_bytes(word))
}
