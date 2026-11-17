use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use super::audio_metadata::{AudioMetadataError, ITUNES_COVER_ART, M4aMetadata, M4aMetadataItem};

const DATA_TYPE_UTF8: u32 = 1;
const DATA_TYPE_JPEG: u32 = 13;
const SAMPLE_TABLE_PATH: [&[u8; 4]; 4] = [b"trak", b"mdia", b"minf", b"stbl"];

/// one box, its type and where its header starts and how long it is with the header
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxSpan {
    pub kind: [u8; 4],
    pub start: u64,
    pub header_len: u64,
    pub len: u64,
}

impl BoxSpan {
    pub fn end(&self) -> u64 {
        self.start + self.len
    }

    pub fn body_start(&self) -> u64 {
        self.start + self.header_len
    }
}

fn failed(message: impl Into<String>) -> AudioMetadataError {
    AudioMetadataError::Failed(message.into())
}

fn io_failed(error: io::Error) -> AudioMetadataError {
    AudioMetadataError::Failed(error.to_string())
}

/// reads the box header at start of a region ending at limit, none past the end
pub fn parse_header(bytes: &[u8], start: u64, limit: u64) -> Result<Option<BoxSpan>, AudioMetadataError> {
    if start >= limit {
        return Ok(None);
    }
    if limit - start < 8 {
        return Err(failed("a truncated box header in the MPEG-4 file"));
    }
    let at = usize::try_from(start).map_err(|_| failed("the MPEG-4 box lies out of reach"))?;
    let header = bytes.get(at..at + 8).ok_or_else(|| failed("a truncated box header in the MPEG-4 file"))?;
    let size32 = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
    let kind = [header[4], header[5], header[6], header[7]];
    let (header_len, len) = match size32 {
        0 => (8, limit - start),
        1 => {
            let large = bytes
                .get(at + 8..at + 16)
                .ok_or_else(|| failed("a truncated large box header in the MPEG-4 file"))?;
            let mut buffer = [0u8; 8];
            buffer.copy_from_slice(large);
            (16, u64::from_be_bytes(buffer))
        }
        size => (8, u64::from(size)),
    };
    if len < header_len || len > limit - start {
        return Err(failed("a box overruns its parent in the MPEG-4 file"));
    }
    Ok(Some(BoxSpan { kind, start, header_len, len }))
}

/// child boxes of the region start..end of bytes
pub fn children(bytes: &[u8], start: u64, end: u64) -> Result<Vec<BoxSpan>, AudioMetadataError> {
    let mut spans = Vec::new();
    let mut cursor = start;
    while let Some(span) = parse_header(bytes, cursor, end)? {
        cursor = span.end();
        spans.push(span);
    }
    Ok(spans)
}

fn slice(bytes: &[u8], start: u64, end: u64) -> &[u8] {
    &bytes[start as usize..end as usize]
}

/// a box with a 32 bit size header, or the large form when the body needs it
pub fn make_box(kind: [u8; 4], body: &[u8]) -> Vec<u8> {
    let compact = body.len() as u64 + 8;
    let mut out = Vec::with_capacity(body.len() + 16);
    if let Ok(size) = u32::try_from(compact) {
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(&kind);
    } else {
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&kind);
        out.extend_from_slice(&(compact + 8).to_be_bytes());
    }
    out.extend_from_slice(body);
    out
}

/// one ilst entry, the item box around a data box (type and locale 0 and payload)
fn item_box(kind: [u8; 4], data_type: u32, payload: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(payload.len() + 8);
    data.extend_from_slice(&data_type.to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes());
    data.extend_from_slice(payload);
    make_box(kind, &make_box(*b"data", &data))
}

pub fn ilst_box(metadata: &M4aMetadata) -> Vec<u8> {
    let mut body = Vec::new();
    for item in metadata.items() {
        match item {
            M4aMetadataItem::Text(kind, value) => body.extend(item_box(kind, DATA_TYPE_UTF8, value.as_bytes())),
            M4aMetadataItem::CoverJpeg(jpeg) => body.extend(item_box(ITUNES_COVER_ART, DATA_TYPE_JPEG, jpeg)),
        }
    }
    make_box(*b"ilst", &body)
}

fn mdir_handler() -> Vec<u8> {
    let mut body = Vec::with_capacity(25);
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(b"mdir");
    body.extend_from_slice(b"appl");
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.push(0);
    make_box(*b"hdlr", &body)
}

/// fresh meta box (full box version 0) holding the handler and the list
fn fresh_meta(ilst: &[u8]) -> Vec<u8> {
    let mut body = 0u32.to_be_bytes().to_vec();
    body.extend(mdir_handler());
    body.extend_from_slice(ilst);
    make_box(*b"meta", &body)
}

fn meta_children_start(bytes: &[u8], meta: &BoxSpan) -> u64 {
    let body = meta.body_start();
    let flags_are_zero = meta.end() - body >= 4 && slice(bytes, body, body + 4) == [0, 0, 0, 0];
    if flags_are_zero { body + 4 } else { body }
}

fn meta_handler(bytes: &[u8], meta: &BoxSpan, children_start: u64) -> Result<Option<[u8; 4]>, AudioMetadataError> {
    for child in children(bytes, children_start, meta.end())? {
        if &child.kind == b"hdlr" && child.end() - child.body_start() >= 12 {
            let at = child.body_start() + 8;
            let mut handler = [0u8; 4];
            handler.copy_from_slice(slice(bytes, at, at + 4));
            return Ok(Some(handler));
        }
    }
    Ok(None)
}

fn rebuilt_meta(bytes: &[u8], meta: &BoxSpan, ilst: &[u8]) -> Result<Vec<u8>, AudioMetadataError> {
    let children_start = meta_children_start(bytes, meta);
    if meta_handler(bytes, meta, children_start)? != Some(*b"mdir") {
        return Ok(fresh_meta(ilst));
    }
    let mut body = slice(bytes, meta.body_start(), children_start).to_vec();
    let mut placed = false;
    for child in children(bytes, children_start, meta.end())? {
        if &child.kind == b"ilst" {
            if !placed {
                body.extend_from_slice(ilst);
                placed = true;
            }
        } else {
            body.extend_from_slice(slice(bytes, child.start, child.end()));
        }
    }
    if !placed {
        body.extend_from_slice(ilst);
    }
    Ok(make_box(*b"meta", &body))
}

/// udta with its meta rebuilt, or a new udta when the movie has none
fn rebuilt_udta(bytes: &[u8], udta: Option<&BoxSpan>, ilst: &[u8]) -> Result<Vec<u8>, AudioMetadataError> {
    let Some(udta) = udta else {
        return Ok(make_box(*b"udta", &fresh_meta(ilst)));
    };
    let mut body = Vec::new();
    let mut placed = false;
    for child in children(bytes, udta.body_start(), udta.end())? {
        if &child.kind == b"meta" {
            if !placed {
                body.extend(rebuilt_meta(bytes, &child, ilst)?);
                placed = true;
            }
        } else {
            body.extend_from_slice(slice(bytes, child.start, child.end()));
        }
    }
    if !placed {
        body.extend(fresh_meta(ilst));
    }
    Ok(make_box(*b"udta", &body))
}

pub fn rebuilt_moov(moov: &[u8], metadata: &M4aMetadata) -> Result<Vec<u8>, AudioMetadataError> {
    if !metadata.replaces_source_tags() {
        return Ok(moov.to_vec());
    }
    let span = parse_header(moov, 0, moov.len() as u64)?.ok_or_else(|| failed("an empty moov box"))?;
    let ilst = ilst_box(metadata);
    let all = children(moov, span.body_start(), span.end())?;
    let udta = all.iter().find(|child| &child.kind == b"udta");
    let mut body = Vec::new();
    for child in &all {
        if &child.kind == b"udta" {
            if udta.is_some_and(|first| first.start == child.start) {
                body.extend(rebuilt_udta(moov, Some(child), &ilst)?);
            }
        } else {
            body.extend_from_slice(slice(moov, child.start, child.end()));
        }
    }
    if udta.is_none() {
        body.extend(rebuilt_udta(moov, None, &ilst)?);
    }
    Ok(make_box(*b"moov", &body))
}

/// moves every chunk offset at or after from by delta inside a moov
pub fn shift_chunk_offsets(moov: &mut [u8], from: u64, delta: i64) -> Result<(), AudioMetadataError> {
    if delta == 0 {
        return Ok(());
    }
    let span = parse_header(moov, 0, moov.len() as u64)?.ok_or_else(|| failed("an empty moov box"))?;
    let mut tables = Vec::new();
    collect_offset_tables(moov, span.body_start(), span.end(), 0, &mut tables)?;
    for table in tables {
        shift_table(moov, &table, from, delta)?;
    }
    Ok(())
}

fn collect_offset_tables(
    bytes: &[u8],
    start: u64,
    end: u64,
    depth: usize,
    out: &mut Vec<BoxSpan>,
) -> Result<(), AudioMetadataError> {
    for child in children(bytes, start, end)? {
        if depth == SAMPLE_TABLE_PATH.len() {
            if &child.kind == b"stco" || &child.kind == b"co64" {
                out.push(child);
            }
        } else if &child.kind == SAMPLE_TABLE_PATH[depth] {
            collect_offset_tables(bytes, child.body_start(), child.end(), depth + 1, out)?;
        }
    }
    Ok(())
}

fn shift_table(bytes: &mut [u8], table: &BoxSpan, from: u64, delta: i64) -> Result<(), AudioMetadataError> {
    let wide = &table.kind == b"co64";
    let width: u64 = if wide { 8 } else { 4 };
    let body = table.body_start();
    if table.end() - body < 8 {
        return Err(failed("a truncated chunk offset table"));
    }
    let count_at = (body + 4) as usize;
    let count = u64::from(u32::from_be_bytes([
        bytes[count_at],
        bytes[count_at + 1],
        bytes[count_at + 2],
        bytes[count_at + 3],
    ]));
    if body + 8 + count * width > table.end() {
        return Err(failed("a chunk offset table longer than its box"));
    }
    for index in 0..count {
        let at = (body + 8 + index * width) as usize;
        let offset = if wide {
            let mut buffer = [0u8; 8];
            buffer.copy_from_slice(&bytes[at..at + 8]);
            u64::from_be_bytes(buffer)
        } else {
            u64::from(u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]))
        };
        if offset < from {
            continue;
        }
        let moved = offset
            .checked_add_signed(delta)
            .ok_or_else(|| failed("a chunk offset moved out of range"))?;
        if wide {
            bytes[at..at + 8].copy_from_slice(&moved.to_be_bytes());
        } else {
            let narrow = u32::try_from(moved).map_err(|_| failed("a 32-bit chunk offset moved past 4 GiB"))?;
            bytes[at..at + 4].copy_from_slice(&narrow.to_be_bytes());
        }
    }
    Ok(())
}

fn read_top_level(file: &mut File, length: u64) -> Result<Vec<BoxSpan>, AudioMetadataError> {
    let mut spans = Vec::new();
    let mut cursor = 0u64;
    while cursor < length {
        let mut header = [0u8; 16];
        let available = (length - cursor).min(16) as usize;
        file.seek(SeekFrom::Start(cursor)).map_err(io_failed)?;
        file.read_exact(&mut header[..available]).map_err(io_failed)?;
        let mut span = parse_header(&header[..available], 0, length - cursor)?
            .ok_or_else(|| failed("a truncated top level box"))?;
        span.start = cursor;
        cursor = span.end();
        spans.push(span);
    }
    Ok(spans)
}

fn copy_range(source: &mut File, destination: &mut File, start: u64, end: u64) -> Result<(), AudioMetadataError> {
    source.seek(SeekFrom::Start(start)).map_err(io_failed)?;
    let copied = io::copy(&mut Read::by_ref(source).take(end - start), destination).map_err(io_failed)?;
    if copied != end - start {
        return Err(failed("the source file ended early"));
    }
    Ok(())
}

/// writes destination from source with metadata in place of the source tags
/// the caller removes destination beforehand and a partial one on error
pub fn rewrite(source: &Path, destination: &Path, metadata: &M4aMetadata) -> Result<(), AudioMetadataError> {
    let mut input = File::open(source).map_err(io_failed)?;
    let length = input.metadata().map_err(io_failed)?.len();
    let top = read_top_level(&mut input, length)?;
    if top.first().is_none_or(|first| &first.kind != b"ftyp") {
        return Err(failed("the source is not an MPEG-4 audio file"));
    }
    let moov = *top
        .iter()
        .find(|span| &span.kind == b"moov")
        .ok_or_else(|| failed("the MPEG-4 file has no moov box"))?;
    if top.iter().any(|span| &span.kind == b"moof") && metadata.replaces_source_tags() {
        return Err(failed("fragmented MPEG-4 audio cannot be tagged in place"));
    }
    let moov_len = usize::try_from(moov.len).map_err(|_| failed("the moov box is too large"))?;
    let mut original = vec![0u8; moov_len];
    input.seek(SeekFrom::Start(moov.start)).map_err(io_failed)?;
    input.read_exact(&mut original).map_err(io_failed)?;
    let mut rebuilt = rebuilt_moov(&original, metadata)?;
    let delta = i64::try_from(rebuilt.len() as u64)
        .ok()
        .zip(i64::try_from(moov.len).ok())
        .map(|(new, old)| new - old)
        .ok_or_else(|| failed("the moov box is too large"))?;
    shift_chunk_offsets(&mut rebuilt, moov.end(), delta)?;

    let mut output = File::create_new(destination).map_err(io_failed)?;
    copy_range(&mut input, &mut output, 0, moov.start)?;
    output.write_all(&rebuilt).map_err(io_failed)?;
    copy_range(&mut input, &mut output, moov.end(), length)?;
    output.sync_all().map_err(io_failed)?;
    Ok(())
}
