use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::audio::audio_file_metadata::{
    AudioFileMetadata, AudioFileRange, AudioFileRangeFetcher, AudioFileRangeSource,
};

struct RangeServer {
    payload: Vec<u8>,
    supports_range: bool,
    fetches: Arc<AtomicU64>,
}

impl RangeServer {
    fn new(length: usize, supports_range: bool) -> (Self, Arc<AtomicU64>) {
        let fetches = Arc::new(AtomicU64::new(0));
        let payload = (0..length).map(|index| (index % 251) as u8).collect();
        let server = Self {
            payload,
            supports_range,
            fetches: Arc::clone(&fetches),
        };
        (server, fetches)
    }
}

impl AudioFileRangeFetcher for RangeServer {
    fn fetch(&self, offset: u64, length: u64) -> io::Result<AudioFileRange> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        let total_length = self.payload.len() as u64;
        if !self.supports_range {
            return Ok(AudioFileRange {
                offset: 0,
                bytes: self.payload.clone(),
                total_length,
            });
        }
        let end = offset.saturating_add(length).min(total_length);
        Ok(AudioFileRange {
            offset,
            bytes: self.payload[offset as usize..end as usize].to_vec(),
            total_length,
        })
    }
}

struct RefusingServer;

impl AudioFileRangeFetcher for RefusingServer {
    fn fetch(&self, _offset: u64, _length: u64) -> io::Result<AudioFileRange> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }
}

#[test]
fn a_read_crosses_windows_with_the_right_bytes() {
    let window = AudioFileMetadata::RANGE_READ_WINDOW_BYTES as usize;
    let (server, fetches) = RangeServer::new(window + 10, true);
    let expected = server.payload.clone();
    let mut source = AudioFileRangeSource::open(server).unwrap();
    let mut read = Vec::new();
    source.read_to_end(&mut read).unwrap();
    assert_eq!(read, expected);
    assert_eq!(fetches.load(Ordering::SeqCst), 2);
}

#[test]
fn a_seek_to_the_tail_skips_the_middle_of_the_file() {
    let (server, fetches) = RangeServer::new(32 * 1024 * 1024, true);
    let expected = server.payload[server.payload.len() - 16..].to_vec();
    let mut source = AudioFileRangeSource::open(server).unwrap();
    source.seek(SeekFrom::End(-16)).unwrap();
    let mut read = Vec::new();
    source.read_to_end(&mut read).unwrap();
    assert_eq!(read, expected);
    assert_eq!(fetches.load(Ordering::SeqCst), 2);
}

#[test]
fn a_seek_before_the_start_is_refused() {
    let (server, _) = RangeServer::new(16, true);
    let mut source = AudioFileRangeSource::open(server).unwrap();
    assert!(source.seek(SeekFrom::Current(-1)).is_err());
}

#[test]
fn a_server_without_range_support_is_read_from_its_whole_answer() {
    let (server, fetches) = RangeServer::new(1024, false);
    let expected = server.payload.clone();
    let mut source = AudioFileRangeSource::open(server).unwrap();
    source.seek(SeekFrom::Start(512)).unwrap();
    let mut read = Vec::new();
    source.read_to_end(&mut read).unwrap();
    assert_eq!(read, expected[512..]);
    assert_eq!(fetches.load(Ordering::SeqCst), 1);
}

#[test]
fn the_byte_budget_ends_a_read_that_never_finds_its_tags() {
    let (server, fetches) = RangeServer::new(32 * 1024 * 1024, true);
    let mut source = AudioFileRangeSource::open(server).unwrap();
    let mut read = Vec::new();
    assert!(source.read_to_end(&mut read).is_err());
    assert_eq!(read.len() as u64, AudioFileMetadata::MAXIMUM_RANGE_READ_BYTES);
    assert_eq!(
        fetches.load(Ordering::SeqCst),
        AudioFileMetadata::MAXIMUM_RANGE_READ_BYTES / AudioFileMetadata::RANGE_READ_WINDOW_BYTES
    );
}

#[test]
fn a_refused_file_answers_empty() {
    assert_eq!(AudioFileMetadata::read_ranges(RefusingServer, None), AudioFileMetadata::default());
}

#[test]
fn an_unrecognised_file_answers_empty() {
    let (server, _) = RangeServer::new(4096, true);
    assert_eq!(AudioFileMetadata::read_ranges(server, None), AudioFileMetadata::default());
}
