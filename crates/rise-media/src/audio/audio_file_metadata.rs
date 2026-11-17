use std::io::{self, Cursor, Read, Seek, SeekFrom};

use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::{Metadata, MetadataOptions, MetadataRevision, StandardTagKey, Value};
use symphonia::core::probe::Hint;

/// one answer of a range fetcher
/// a server that ignores Range sends the whole file so offset says where bytes start
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioFileRange {
    pub offset: u64,
    pub bytes: Vec<u8>,
    pub total_length: u64,
}

/// the network behind read_ranges, called on a blocking thread
/// an error ends the read so a cancelled caller answers with one
pub trait AudioFileRangeFetcher: Send + Sync {
    fn fetch(&self, offset: u64, length: u64) -> io::Result<AudioFileRange>;
}

/// remote file as a seekable stream so demuxers read only the atoms they ask for
/// and skip an mdat by seeking over it
pub struct AudioFileRangeSource<F> {
    fetcher: F,
    position: u64,
    total_length: u64,
    window: AudioFileRange,
    remaining_bytes: u64,
}

impl<F: AudioFileRangeFetcher> AudioFileRangeSource<F> {
    /// asks for the first window right away, the probe needs the file length before its first read
    pub fn open(fetcher: F) -> io::Result<Self> {
        let mut source = Self {
            fetcher,
            position: 0,
            total_length: 0,
            window: AudioFileRange::default(),
            remaining_bytes: AudioFileMetadata::MAXIMUM_RANGE_READ_BYTES,
        };
        source.load(0, AudioFileMetadata::RANGE_READ_WINDOW_BYTES)?;
        Ok(source)
    }

    fn load(&mut self, offset: u64, length: u64) -> io::Result<()> {
        if self.remaining_bytes == 0 {
            return Err(io::Error::other("audio tag read exceeded its byte budget"));
        }
        let window = self.fetcher.fetch(offset, length)?;
        let end = window.offset.saturating_add(window.bytes.len() as u64);
        let is_empty_file = offset == 0 && window.total_length == 0;
        if !is_empty_file && !(window.offset..end).contains(&offset) {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        self.remaining_bytes = self.remaining_bytes.saturating_sub(window.bytes.len() as u64);
        self.total_length = window.total_length;
        self.window = window;
        Ok(())
    }
}

impl<F: AudioFileRangeFetcher> Read for AudioFileRangeSource<F> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() || self.position >= self.total_length {
            return Ok(0);
        }
        let window_end = self.window.offset.saturating_add(self.window.bytes.len() as u64);
        if self.position < self.window.offset || self.position >= window_end {
            let length = AudioFileMetadata::RANGE_READ_WINDOW_BYTES.min(self.total_length - self.position);
            self.load(self.position, length)?;
        }
        let start = usize::try_from(self.position - self.window.offset)
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        let Some(available) = self.window.bytes.get(start..) else {
            return Ok(0);
        };
        let count = available.len().min(buffer.len());
        buffer[..count].copy_from_slice(&available[..count]);
        self.position += count as u64;
        Ok(count)
    }
}

impl<F: AudioFileRangeFetcher> Seek for AudioFileRangeSource<F> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let target = match position {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(delta) => self.total_length.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
        };
        self.position = target.ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        Ok(self.position)
    }
}

impl<F: AudioFileRangeFetcher> MediaSource for AudioFileRangeSource<F> {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.total_length)
    }
}

/// title and artist and attached picture of one file, each the first non empty value found
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioFileMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub artwork: Option<Vec<u8>>,
}

impl AudioFileMetadata {
    pub const RANGE_READ_WINDOW_BYTES: u64 = 256 * 1024;
    pub const MAXIMUM_RANGE_READ_BYTES: u64 = 16 * 1024 * 1024;

    /// reads every metadata revision of a whole file
    /// tags in front of the container (id3v2 on mp3) and the containers own (mp4 ilst)
    /// an unrecognised file answers empty
    pub fn read(bytes: Vec<u8>, file_extension: Option<&str>) -> Self {
        Self::read_source(Box::new(Cursor::new(bytes)), file_extension)
    }

    /// read over a remote file, only the ranges the demuxer asks for travel
    /// blocks so run it off the main thread and off async workers
    /// a failed fetch answers empty
    pub fn read_ranges<F: AudioFileRangeFetcher + 'static>(fetcher: F, file_extension: Option<&str>) -> Self {
        match AudioFileRangeSource::open(fetcher) {
            Ok(source) => Self::read_source(Box::new(source), file_extension),
            Err(_) => Self::default(),
        }
    }

    fn read_source(source: Box<dyn MediaSource>, file_extension: Option<&str>) -> Self {
        let mut hint = Hint::new();
        if let Some(extension) = file_extension.filter(|extension| !extension.is_empty()) {
            hint.with_extension(extension);
        }
        let stream = MediaSourceStream::new(source, Default::default());
        let Ok(mut probed) = symphonia::default::get_probe().format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        ) else {
            return Self::default();
        };

        let mut revisions = Vec::new();
        if let Some(metadata) = probed.metadata.get() {
            collect(metadata, &mut revisions);
        }
        collect(probed.format.metadata(), &mut revisions);
        Self::from_revisions(&revisions)
    }

    fn from_revisions(revisions: &[MetadataRevision]) -> Self {
        Self {
            title: first_string(revisions, StandardTagKey::TrackTitle),
            artist: first_string(revisions, StandardTagKey::Artist),
            album_artist: first_string(revisions, StandardTagKey::AlbumArtist),
            artwork: revisions
                .iter()
                .flat_map(|revision| revision.visuals())
                .find(|visual| !visual.data.is_empty())
                .map(|visual| visual.data.to_vec()),
        }
    }
}

fn collect(mut metadata: Metadata<'_>, into: &mut Vec<MetadataRevision>) {
    while let Some(revision) = metadata.pop() {
        into.push(revision);
    }
    if let Some(current) = metadata.current() {
        into.push(current.clone());
    }
}

fn first_string(revisions: &[MetadataRevision], key: StandardTagKey) -> Option<String> {
    revisions
        .iter()
        .flat_map(|revision| revision.tags())
        .filter(|tag| tag.std_key == Some(key))
        .find_map(|tag| match &tag.value {
            Value::String(value) if !value.is_empty() => Some(value.clone()),
            _ => None,
        })
}
