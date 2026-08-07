//! The binding to symphonia.
//!
//! Everything above this file works in interleaved f32 frames and knows nothing
//! about packets, codec parameters or sample formats. That is the whole job
//! here: turn a byte buffer into frames, and turn symphonia's per-codec buffer
//! zoo into one representation.
//!
//! Not behind a feature, unlike the FFmpeg backend, because symphonia is pure
//! Rust with no system dependency and no vendored prefix — there is nothing for
//! a feature to protect a build from.

use std::io::Cursor;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use super::codec::{self, Container, Support};
use super::decoder::{AudioDecoder, DecodeError, Decoded, DecoderPlan};
use super::format::{FormatError, StreamFormat};
use super::gapless::{self, Trim};

pub struct SymphoniaDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    stream_format: StreamFormat,
    duration_millis: Option<u64>,
    plan: DecoderPlan,
    trim: Trim,
    /// Reused across packets. Allocating one per packet is an allocation every
    /// 20 ms per stream, which is the shape of cost this crate exists to avoid.
    buffer: Option<SampleBuffer<f32>>,
}

impl SymphoniaDecoder {
    /// Open a fully downloaded track.
    ///
    /// Takes the bytes rather than a path because the reference plays from its
    /// own disk cache, never from the network directly, and because a seek on
    /// a buffer cannot fail halfway through a track the way a seek on a partial
    /// download can.
    pub fn open(bytes: Vec<u8>) -> Result<Self, DecodeError> {
        let container = codec::sniff(&bytes).ok_or(DecodeError::Unrecognised)?;
        let plan = DecoderPlan::for_container(container);

        let parsed_trim = read_trim(&bytes, container);
        let trim = plan.our_trim(parsed_trim);

        let mut hint = Hint::new();
        hint.with_extension(container.extension());
        hint.mime_type(container.mime());

        let stream = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());

        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                stream,
                &FormatOptions {
                    enable_gapless: plan.decoder_gapless,
                    ..Default::default()
                },
                &MetadataOptions::default(),
            )
            .map_err(map_error)?;

        let format = probed.format;

        let track = format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or(DecodeError::NoAudioTrack)?;

        let track_id = track.id;
        let parameters = track.codec_params.clone();

        let sample_rate = parameters
            .sample_rate
            .ok_or(DecodeError::ImplausibleFormat)?;
        let channels = parameters
            .channels
            .ok_or(DecodeError::ImplausibleFormat)?
            .count() as u16;
        let stream_format = StreamFormat::new(sample_rate, channels).map_err(map_format_error)?;

        let duration_millis = parameters
            .n_frames
            .map(|frames| stream_format.frames_to_millis(frames));

        let decoder = symphonia::default::get_codecs()
            .make(&parameters, &DecoderOptions::default())
            .map_err(|error| refusal_or(map_error(error), container))?;

        Ok(Self {
            format,
            decoder,
            track_id,
            stream_format,
            duration_millis,
            plan,
            trim,
            buffer: None,
        })
    }

    pub fn plan(&self) -> DecoderPlan {
        self.plan
    }

    /// The priming and padding this decoder will not remove itself. The pump
    /// feeds it to a `Trimmer`.
    pub fn trim(&self) -> Trim {
        self.trim
    }
}

impl AudioDecoder for SymphoniaDecoder {
    fn format(&self) -> StreamFormat {
        self.stream_format
    }

    fn duration_millis(&self) -> Option<u64> {
        self.duration_millis
    }

    fn decode(&mut self, out: &mut Vec<f32>) -> Result<Decoded, DecodeError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(error))
                    if error.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(Decoded::EndOfStream);
                }
                Err(error) => return Err(map_error(error)),
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                // A recoverable stream error is one corrupt packet, not a
                // broken file. Skipping it is what every media player does;
                // failing the track would turn one bad frame into a track the
                // user cannot play at all.
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(SymphoniaError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(error) => return Err(map_error(error)),
            };

            let specification = *decoded.spec();
            let capacity = decoded.capacity() as u64;

            let buffer = self
                .buffer
                .get_or_insert_with(|| SampleBuffer::new(capacity, specification));
            if buffer.capacity() < (capacity as usize * specification.channels.count()) {
                *buffer = SampleBuffer::new(capacity, specification);
            }

            buffer.copy_interleaved_ref(decoded);
            let samples = buffer.samples();
            if samples.is_empty() {
                continue;
            }

            out.extend_from_slice(samples);
            return Ok(Decoded::Frames(
                samples.len() / specification.channels.count(),
            ));
        }
    }

    fn seek(&mut self, millis: u64) -> Result<u64, DecodeError> {
        let seconds = millis / 1000;
        let fraction = (millis % 1000) as f64 / 1000.0;

        let seeked = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time: Time::new(seconds, fraction),
                    track_id: Some(self.track_id),
                },
            )
            .map_err(map_error)?;

        self.decoder.reset();

        Ok(self.stream_format.frames_to_millis(seeked.actual_ts))
    }
}

/// Read whatever gapless information the container carries.
///
/// Only the MP4 path does any work: everywhere else the decoder owns the trim
/// and reading the tag ourselves would invite double-trimming.
fn read_trim(bytes: &[u8], container: Container) -> Option<Trim> {
    if !container.needs_our_gapless_trim() {
        return None;
    }

    gapless::find_itunsmpb(bytes).and_then(gapless::parse_itunsmpb)
}

/// symphonia reports "no decoder registered" the same way for a codec we chose
/// not to build in as for a corrupt one. Reporting Opus as malformed would send
/// whoever hits it looking for a broken file that is not broken.
fn refusal_or(error: DecodeError, container: Container) -> DecodeError {
    let refused = match container {
        Container::Ogg => Some(codec::AudioCodec::Opus),
        Container::Amr => Some(codec::AudioCodec::Amr),
        _ => None,
    };

    match refused.map(codec::AudioCodec::support) {
        Some(Support::Refused(reason)) => DecodeError::Refused(reason),
        _ => error,
    }
}

fn map_error(error: SymphoniaError) -> DecodeError {
    match error {
        SymphoniaError::IoError(inner) => DecodeError::Io(inner.to_string()),
        SymphoniaError::Unsupported(what) => DecodeError::Malformed(what.to_string()),
        other => DecodeError::Malformed(other.to_string()),
    }
}

fn map_format_error(error: FormatError) -> DecodeError {
    match error {
        FormatError::UnsupportedSampleRate { .. } | FormatError::UnsupportedChannelCount { .. } => {
            DecodeError::ImplausibleFormat
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A real 16-bit PCM WAV file, built here so the suite needs no fixture on
    /// disk and the test proves an actual decode rather than a mocked one.
    pub(crate) fn wav(sample_rate: u32, channels: u16, frames: usize) -> Vec<u8> {
        let data_bytes = frames * channels as usize * 2;
        let mut file = Vec::with_capacity(44 + data_bytes);

        file.extend_from_slice(b"RIFF");
        file.extend_from_slice(&((36 + data_bytes) as u32).to_le_bytes());
        file.extend_from_slice(b"WAVE");

        file.extend_from_slice(b"fmt ");
        file.extend_from_slice(&16u32.to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&channels.to_le_bytes());
        file.extend_from_slice(&sample_rate.to_le_bytes());
        file.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes());
        file.extend_from_slice(&(channels * 2).to_le_bytes());
        file.extend_from_slice(&16u16.to_le_bytes());

        file.extend_from_slice(b"data");
        file.extend_from_slice(&(data_bytes as u32).to_le_bytes());
        for frame in 0..frames {
            let value = (frame as f64 / sample_rate as f64 * 440.0 * std::f64::consts::TAU).sin();
            let sample = (value * 30_000.0) as i16;
            for _ in 0..channels {
                file.extend_from_slice(&sample.to_le_bytes());
            }
        }

        file
    }

    #[test]
    fn a_real_file_is_decoded_frame_for_frame() {
        let mut decoder = SymphoniaDecoder::open(wav(44_100, 2, 44_100)).unwrap();

        assert_eq!(decoder.format(), StreamFormat::new(44_100, 2).unwrap());
        assert_eq!(decoder.duration_millis(), Some(1_000));

        let mut out = Vec::new();
        let mut frames = 0;
        while let Decoded::Frames(count) = decoder.decode(&mut out).unwrap() {
            frames += count;
        }

        assert_eq!(frames, 44_100);
        assert_eq!(out.len(), 44_100 * 2);
    }

    #[test]
    fn the_decoded_audio_is_the_signal_that_went_in() {
        let mut decoder = SymphoniaDecoder::open(wav(44_100, 1, 4_410)).unwrap();
        let mut out = Vec::new();
        while let Ok(Decoded::Frames(_)) = decoder.decode(&mut out) {}

        let crossings = out
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();

        assert!(
            (43..=45).contains(&crossings),
            "a 440 Hz tone over a tenth of a second came out at {crossings} cycles"
        );
        assert!(out.iter().all(|sample| sample.abs() <= 1.0));
    }

    #[test]
    fn seeking_lands_where_it_was_asked_to() {
        let mut decoder = SymphoniaDecoder::open(wav(44_100, 2, 44_100 * 5)).unwrap();

        let reached = decoder.seek(3_000).unwrap();

        assert!(
            reached.abs_diff(3_000) < 50,
            "asked for 3000 ms and landed at {reached} ms"
        );

        let mut out = Vec::new();
        assert!(matches!(
            decoder.decode(&mut out).unwrap(),
            Decoded::Frames(_)
        ));
    }

    #[test]
    fn seeking_back_to_the_start_plays_the_track_again() {
        let mut decoder = SymphoniaDecoder::open(wav(44_100, 2, 44_100)).unwrap();
        let mut out = Vec::new();
        while let Ok(Decoded::Frames(_)) = decoder.decode(&mut out) {}

        decoder.seek(0).unwrap();

        let mut again = Vec::new();
        let mut frames = 0;
        while let Ok(Decoded::Frames(count)) = decoder.decode(&mut again) {
            frames += count;
        }

        assert!(frames > 40_000, "only {frames} frames after seeking home");
    }

    #[test]
    fn a_file_that_is_not_audio_is_refused_before_a_parser_is_started() {
        assert_eq!(
            SymphoniaDecoder::open(b"<!DOCTYPE html>".to_vec()).err(),
            Some(DecodeError::Unrecognised)
        );
    }

    #[test]
    fn a_truncated_file_is_an_error_rather_than_a_panic() {
        let mut file = wav(44_100, 2, 1_000);
        file.truncate(60);

        // Either the probe rejects it or the first decode hits the end; both
        // are fine, neither may crash.
        if let Ok(mut decoder) = SymphoniaDecoder::open(file) {
            let mut out = Vec::new();
            let _ = decoder.decode(&mut out);
        }
    }

    #[test]
    fn a_header_that_lies_about_its_rate_is_refused() {
        let mut file = wav(44_100, 2, 100);
        file[24..28].copy_from_slice(&4_000_000_000u32.to_le_bytes());

        assert!(matches!(
            SymphoniaDecoder::open(file),
            Err(DecodeError::ImplausibleFormat | DecodeError::Malformed(_))
        ));
    }

    #[test]
    fn a_wav_is_planned_as_nobodys_trim() {
        let decoder = SymphoniaDecoder::open(wav(44_100, 2, 100)).unwrap();

        assert_eq!(decoder.plan().container, Container::Wav);
        assert!(decoder.trim().is_none());
    }
}
