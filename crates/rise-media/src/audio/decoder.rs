//! Audio decode, and the choice of who trims the encoder's padding.
//!
//! The decoder itself is symphonia — pure Rust, MPL-2.0, no C parsing
//! attacker-supplied bytes and, more to the point, no FFmpeg on the critical
//! path of the music player. That was a deliberate call in PHASES.txt: video
//! carries an LGPL FFmpeg because there is no alternative, and audio does not
//! have to.
//!
//! Everything here that makes a DECISION is pure and always compiled. The
//! binding to symphonia is `symphonia_backend.rs`.

use super::codec::{Container, Unsupported};
use super::format::StreamFormat;
use super::gapless::Trim;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DecodeError {
    /// The bytes are not a container this client identifies.
    Unrecognised,
    /// Identified, and deliberately not played. See `codec::Unsupported`.
    Refused(Unsupported),
    /// The container parsed but the audio inside it did not.
    Malformed(String),
    /// The file has no audio track at all — a video-only MP4, usually.
    NoAudioTrack,
    /// The stream describes a rate or channel count outside what the pipeline
    /// accepts.
    ImplausibleFormat,
    Io(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decoded {
    /// Frames appended to the output buffer.
    Frames(usize),
    /// Nothing more will be produced. Not an error: it is how every track ends.
    EndOfStream,
}

/// Who removes the priming and padding frames.
///
/// Getting this wrong in either direction is audible: nobody trimming leaves
/// the gap between album tracks, and both trimming cuts real audio out of the
/// start of every track.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrimOwner {
    /// symphonia reads the LAME/Xing tag itself when gapless support is
    /// enabled, and Vorbis and FLAC carry their own sample-accurate lengths.
    Decoder,
    /// The iTunes `iTunSMPB` atom in an MP4. symphonia does not read it, and
    /// `file-service`'s extraction job writes exactly this container.
    Us,
    /// Uncompressed, so there is nothing to trim.
    Nobody,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DecoderPlan {
    pub container: Container,
    pub trim: TrimOwner,
    /// Whether to ask symphonia for its own gapless handling. Only ever true
    /// where `trim` is `Decoder`, or the two would both cut.
    pub decoder_gapless: bool,
}

impl DecoderPlan {
    pub const fn for_container(container: Container) -> Self {
        let trim = match container {
            Container::Mp3 | Container::Flac | Container::Ogg => TrimOwner::Decoder,
            Container::IsoMp4 => TrimOwner::Us,
            Container::Wav | Container::Caf | Container::Amr => TrimOwner::Nobody,
        };

        Self {
            container,
            trim,
            decoder_gapless: matches!(trim, TrimOwner::Decoder),
        }
    }

    /// The trim to apply on top of whatever the decoder did.
    pub const fn our_trim(&self, parsed: Option<Trim>) -> Trim {
        match (self.trim, parsed) {
            (TrimOwner::Us, Some(trim)) => trim,
            _ => Trim::NONE,
        }
    }
}

pub trait AudioDecoder: Send {
    fn format(&self) -> StreamFormat;
    /// `None` for a stream whose container does not state a length.
    fn duration_millis(&self) -> Option<u64>;
    /// Append interleaved f32 frames to `out`.
    fn decode(&mut self, out: &mut Vec<f32>) -> Result<Decoded, DecodeError>;
    /// Returns the position actually reached, which is the start of the packet
    /// containing the requested time and therefore never later than it.
    fn seek(&mut self, millis: u64) -> Result<u64, DecodeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp3_is_trimmed_by_the_decoder_and_never_twice() {
        let plan = DecoderPlan::for_container(Container::Mp3);

        assert_eq!(plan.trim, TrimOwner::Decoder);
        assert!(plan.decoder_gapless);
        assert!(
            plan.our_trim(Some(Trim {
                start_frames: 1105,
                valid_frames: Some(100),
            }))
            .is_none(),
            "trimming on top of symphonia's own gapless cuts real audio"
        );
    }

    #[test]
    fn the_extraction_jobs_mp4_is_ours_to_trim() {
        let plan = DecoderPlan::for_container(Container::IsoMp4);

        assert_eq!(plan.trim, TrimOwner::Us);
        assert!(!plan.decoder_gapless);
        assert_eq!(
            plan.our_trim(Some(Trim {
                start_frames: 2112,
                valid_frames: Some(1000),
            })),
            Trim {
                start_frames: 2112,
                valid_frames: Some(1000),
            }
        );
    }

    #[test]
    fn an_mp4_with_no_gapless_atom_is_played_whole_rather_than_guessed_at() {
        let plan = DecoderPlan::for_container(Container::IsoMp4);

        assert!(plan.our_trim(None).is_none());
    }

    #[test]
    fn uncompressed_audio_has_nothing_to_trim() {
        for container in [Container::Wav, Container::Caf] {
            let plan = DecoderPlan::for_container(container);
            assert_eq!(plan.trim, TrimOwner::Nobody);
            assert!(!plan.decoder_gapless);
        }
    }

    #[test]
    fn exactly_one_party_ever_trims() {
        for container in [
            Container::Mp3,
            Container::IsoMp4,
            Container::Flac,
            Container::Ogg,
            Container::Wav,
            Container::Caf,
            Container::Amr,
        ] {
            let plan = DecoderPlan::for_container(container);
            let ours = plan.our_trim(Some(Trim {
                start_frames: 1000,
                valid_frames: Some(1000),
            }));

            assert!(
                !plan.decoder_gapless || ours.is_none(),
                "{container:?} would be trimmed twice"
            );
        }
    }
}
