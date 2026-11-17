use std::io::Read;

use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::{Crc, Decompress, FlushDecompress, Status};

/// inflates one gzip or zlib stream, told apart by its header
/// none for a broken stream or once the output would pass maximum_bytes
pub fn inflate_gzip_or_zlib(data: &[u8], maximum_bytes: usize) -> Option<Vec<u8>> {
    inflate_bounded(data, maximum_bytes).ok()
}

/// why a bounded inflate gave nothing
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InflateRefusal {
    Broken,
    OverCeiling,
}

/// same as inflate_gzip_or_zlib but says which of the two refusals it was
pub fn inflate_bounded(data: &[u8], maximum_bytes: usize) -> Result<Vec<u8>, InflateRefusal> {
    let ceiling = u64::try_from(maximum_bytes)
        .map_err(|_| InflateRefusal::OverCeiling)?
        .saturating_add(1);
    let mut output = Vec::new();
    let read = if data.starts_with(&[0x1F, 0x8B]) {
        GzDecoder::new(data).take(ceiling).read_to_end(&mut output)
    } else {
        ZlibDecoder::new(data).take(ceiling).read_to_end(&mut output)
    };
    if read.is_err() {
        return Err(InflateRefusal::Broken);
    }
    if output.len() > maximum_bytes {
        return Err(InflateRefusal::OverCeiling);
    }
    Ok(output)
}

/// raw inflate into exactly expanded_size bytes
/// none unless the stream ends there and all input was used
pub fn inflate_raw(input: &[u8], expanded_size: usize) -> Option<Vec<u8>> {
    let mut stream = Decompress::new(false);
    let mut output = vec![0_u8; expanded_size];
    let status = stream
        .decompress(input, &mut output, FlushDecompress::Finish)
        .ok()?;
    let consumed_all = u64::try_from(input.len()).is_ok_and(|count| stream.total_in() == count);
    let filled = u64::try_from(expanded_size).is_ok_and(|count| stream.total_out() == count);
    (status == Status::StreamEnd && consumed_all && filled).then_some(output)
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = Crc::new();
    crc.update(data);
    crc.sum()
}
