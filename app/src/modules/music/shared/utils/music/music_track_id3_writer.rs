use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use rise_canon::RiseLimits;
use rise_platform::file_manager::FileManager;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MusicTrackID3Error {
    #[error("unreadable source")]
    UnreadableSource,
    #[error("write failed")]
    WriteFailed,
}

/// writes title, artist and cover into an mp3 without re-encoding
/// new ID3v2 tag replaces the old head
/// mpeg frames copied byte for byte so bitrate stays
pub enum MusicTrackID3Writer {}

impl MusicTrackID3Writer {
    const COPY_CHUNK_BYTES: u64 = 1 << 20;
    const ID3_HEADER_BYTES: u64 = 10;
    const ID3V1_TRAILER_BYTES: u64 = 128;

    /// server refuses an mp3 with a bigger tag, and only after the full upload
    /// so such a file must never leave the app
    fn maximum_tag_bytes() -> i64 {
        RiseLimits::service_value("max_id3v2_tag_bytes")
    }

    /// blocking file io, call it off the main thread
    pub fn write(
        source: &Path,
        destination: &Path,
        title: Option<&str>,
        artist: Option<&str>,
        artwork: Option<&[u8]>,
    ) -> Result<(), MusicTrackID3Error> {
        let mut reader = File::open(source).map_err(|_| MusicTrackID3Error::UnreadableSource)?;

        let bounds = Self::audio_bounds(&mut reader, source)?;
        let maximum_tag_bytes = Self::maximum_tag_bytes();
        let mut tag = Self::tag_data(title, artist, artwork);
        if Self::byte_count(&tag) > maximum_tag_bytes {
            tag = Self::tag_data(title, artist, None);
        }
        if Self::byte_count(&tag) > maximum_tag_bytes {
            return Err(MusicTrackID3Error::WriteFailed);
        }

        let _ = FileManager.remove_item(destination);
        if !FileManager.create_file(destination, None) {
            return Err(MusicTrackID3Error::WriteFailed);
        }
        let Ok(mut writer) = File::options().write(true).open(destination) else {
            return Err(MusicTrackID3Error::WriteFailed);
        };

        let copied = Self::copy_audio(&mut reader, &mut writer, &tag, bounds);
        drop(writer);
        if copied.is_err() {
            let _ = FileManager.remove_item(destination);
            return Err(MusicTrackID3Error::WriteFailed);
        }
        Ok(())
    }

    fn copy_audio(
        reader: &mut File,
        writer: &mut File,
        tag: &[u8],
        bounds: (u64, u64),
    ) -> std::io::Result<()> {
        let (start, end) = bounds;
        writer.write_all(tag)?;
        reader.seek(SeekFrom::Start(start))?;
        let mut remaining = end - start;
        let mut chunk = Vec::new();
        while remaining > 0 {
            let take = Self::COPY_CHUNK_BYTES.min(remaining);
            chunk.clear();
            (&mut *reader).take(take).read_to_end(&mut chunk)?;
            if chunk.is_empty() {
                break;
            }
            writer.write_all(&chunk)?;
            remaining -= u64::try_from(chunk.len()).unwrap_or(remaining);
        }
        Ok(())
    }

    fn audio_bounds(reader: &mut File, source: &Path) -> Result<(u64, u64), MusicTrackID3Error> {
        let size = std::fs::metadata(source).map_or(0, |metadata| metadata.len());
        if size == 0 {
            return Err(MusicTrackID3Error::UnreadableSource);
        }

        let mut start: u64 = 0;
        reader
            .seek(SeekFrom::Start(0))
            .map_err(|_| MusicTrackID3Error::UnreadableSource)?;
        let header = Self::read_up_to(reader, Self::ID3_HEADER_BYTES)?;
        if header.len() == 10
            && header[0] == 0x49
            && header[1] == 0x44
            && header[2] == 0x33
            && header[3] != 0xFF
            && header[4] != 0xFF
            && header[6] < 0x80
            && header[7] < 0x80
            && header[8] < 0x80
            && header[9] < 0x80
        {
            let mut tag_bytes = u64::from(header[6]) << 21;
            tag_bytes |= u64::from(header[7]) << 14;
            tag_bytes |= u64::from(header[8]) << 7;
            tag_bytes |= u64::from(header[9]);
            start = Self::ID3_HEADER_BYTES + tag_bytes;
            if header[3] == 0x04 && header[5] & 0x10 != 0 {
                start += Self::ID3_HEADER_BYTES;
            }
        }

        let mut end = size;
        if size >= Self::ID3V1_TRAILER_BYTES {
            reader
                .seek(SeekFrom::Start(size - Self::ID3V1_TRAILER_BYTES))
                .map_err(|_| MusicTrackID3Error::UnreadableSource)?;
            let trailer = Self::read_up_to(reader, 3)?;
            if trailer.len() == 3 && trailer[0] == 0x54 && trailer[1] == 0x41 && trailer[2] == 0x47 {
                end = size - Self::ID3V1_TRAILER_BYTES;
            }
        }

        if start >= end {
            return Err(MusicTrackID3Error::UnreadableSource);
        }
        Ok((start, end))
    }

    fn read_up_to(reader: &mut File, count: u64) -> Result<Vec<u8>, MusicTrackID3Error> {
        let mut data = Vec::new();
        (&mut *reader)
            .take(count)
            .read_to_end(&mut data)
            .map_err(|_| MusicTrackID3Error::UnreadableSource)?;
        Ok(data)
    }

    fn tag_data(title: Option<&str>, artist: Option<&str>, artwork: Option<&[u8]>) -> Vec<u8> {
        let mut frames = Vec::new();
        if let Some(title) = title.filter(|title| !title.is_empty()) {
            frames.extend(Self::text_frame("TIT2", title));
        }
        if let Some(artist) = artist.filter(|artist| !artist.is_empty()) {
            frames.extend(Self::text_frame("TPE1", artist));
            frames.extend(Self::text_frame("TPE2", artist));
        }
        if let Some(artwork) = artwork.filter(|artwork| !artwork.is_empty()) {
            frames.extend(Self::picture_frame(artwork));
        }

        let mut tag = vec![0x49, 0x44, 0x33, 0x03, 0x00, 0x00];
        tag.extend(Self::syncsafe(u32::try_from(frames.len()).unwrap_or(u32::MAX)));
        tag.extend(frames);
        tag
    }

    /// encoding 0x01 is UTF-16 with a bom, the only ID3v2.3 text encoding that fits a cyrillic title
    fn text_frame(identifier: &str, value: &str) -> Vec<u8> {
        let mut payload = vec![0x01, 0xFF, 0xFE];
        for unit in value.encode_utf16() {
            payload.extend(unit.to_le_bytes());
        }
        payload.extend([0x00, 0x00]);
        Self::frame(identifier, &payload)
    }

    fn picture_frame(jpeg: &[u8]) -> Vec<u8> {
        let mut payload = vec![0x00];
        payload.extend("image/jpeg".as_bytes());
        payload.push(0x00);
        payload.push(0x03);
        payload.push(0x00);
        payload.extend(jpeg);
        Self::frame("APIC", &payload)
    }

    /// frame sizes are plain big-endian in ID3v2.3, only the tag header is syncsafe
    fn frame(identifier: &str, payload: &[u8]) -> Vec<u8> {
        let mut data: Vec<u8> = identifier.as_bytes().iter().copied().take(4).collect();
        let size = u32::try_from(payload.len()).unwrap_or(u32::MAX);
        data.extend(size.to_be_bytes());
        data.extend([0x00, 0x00]);
        data.extend(payload);
        data
    }

    fn syncsafe(value: u32) -> [u8; 4] {
        [
            ((value >> 21) & 0x7F) as u8,
            ((value >> 14) & 0x7F) as u8,
            ((value >> 7) & 0x7F) as u8,
            (value & 0x7F) as u8,
        ]
    }

    fn byte_count(data: &[u8]) -> i64 {
        i64::try_from(data.len()).unwrap_or(i64::MAX)
    }
}
