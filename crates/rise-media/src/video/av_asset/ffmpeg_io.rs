use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use rsmpeg::avformat::AVIOContextCustom;
use rsmpeg::avutil::AVMem;
use rsmpeg::ffi;

use super::av_asset::{
    avio_seek_target, AVAssetByteSource, AVAssetReadError, AVURLAsset, AVSEEK_FORCE, AVSEEK_SIZE, SEEK_END,
};

pub const AVIO_BUFFER_SIZE: usize = 64 * 1_024;

const AVERROR_EIO: i32 = ffi::AVERROR(5);
const AVERROR_EINVAL: i32 = ffi::AVERROR(22);

/// position and size shared by the read and seek callbacks of one context
struct Cursor {
    position: AtomicI64,
    content_length: Mutex<Option<i64>>,
}

impl Cursor {
    /// resource size, asked of the loader once
    fn content_length(&self, source: &AVAssetByteSource) -> Option<i64> {
        let mut known = self.content_length.lock();
        if known.is_none() {
            match source.content_information() {
                Ok(information) if information.content_length > 0 => *known = Some(information.content_length),
                Ok(_) => {}
                Err(error) => tracing::warn!(target: "riseonly", %error, "stream size unavailable"),
            }
        }
        *known
    }
}

fn read_failure(error: &AVAssetReadError) -> i32 {
    match error {
        AVAssetReadError::Cancelled | AVAssetReadError::LoaderGone => ffi::AVERROR_EOF,
        AVAssetReadError::Declined | AVAssetReadError::TimedOut(_) | AVAssetReadError::Failed(_) => AVERROR_EIO,
    }
}

/// read only custom io context over the asset loader
/// the loader must already be started on the main thread
/// the context is used only on a decoder thread
pub fn avio_context(asset: &AVURLAsset) -> AVIOContextCustom {
    let source = asset.resource_loader().byte_source();
    let cursor = Arc::new(Cursor { position: AtomicI64::new(0), content_length: Mutex::new(None) });

    let read_source = source.clone();
    let read_cursor = Arc::clone(&cursor);
    let read_packet: rsmpeg::avformat::ReadPacketCallback = Box::new(move |_opaque, buffer| {
        let position = read_cursor.position.load(Ordering::Acquire);
        match read_source.read_at(position, buffer.len()) {
            Ok(bytes) if bytes.is_empty() => ffi::AVERROR_EOF,
            Ok(bytes) => {
                let count = bytes.len().min(buffer.len());
                buffer[..count].copy_from_slice(&bytes[..count]);
                let advanced = i64::try_from(count).unwrap_or(i64::MAX);
                read_cursor.position.store(position.saturating_add(advanced), Ordering::Release);
                i32::try_from(count).unwrap_or(i32::MAX)
            }
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, position, "stream read failed");
                read_failure(&error)
            }
        }
    });

    let seek_source = source;
    let seek_cursor = cursor;
    let seek: rsmpeg::avformat::SeekCallback = Box::new(move |_opaque, offset, whence| {
        let position = seek_cursor.position.load(Ordering::Acquire);
        let needs_length = (whence & AVSEEK_SIZE) != 0 || (whence & !AVSEEK_FORCE) == SEEK_END;
        let content_length = if needs_length { seek_cursor.content_length(&seek_source) } else { None };
        match avio_seek_target(offset, whence, position, content_length) {
            Some(target) if (whence & AVSEEK_SIZE) != 0 => target,
            Some(target) => {
                seek_cursor.position.store(target, Ordering::Release);
                target
            }
            None => i64::from(AVERROR_EINVAL),
        }
    });

    AVIOContextCustom::alloc_context(AVMem::new(AVIO_BUFFER_SIZE), false, Vec::new(), Some(read_packet), None, Some(seek))
}
