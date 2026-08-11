//! What a downloaded file actually is, and whether we can play it.
//!
//! Identified from the bytes, never from the filename or the server's
//! Content-Type. Opus and AMR are recognised and refused by name: neither has
//! a pure-Rust decoder.

/// The wrapper the bytes are in. Not the codec: an MP4 holds AAC or ALAC, an
/// Ogg holds Vorbis or Opus, and only the demuxer knows which.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Container {
    /// Bare MPEG audio, with or without an ID3 header.
    Mp3,
    /// `.m4a`, `.mp4`, `.aac` in an ISO base media container.
    IsoMp4,
    Flac,
    Ogg,
    Wav,
    Caf,
    Amr,
}

impl Container {
    /// The MIME type the response header is overridden with.
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Mp3 => "audio/mpeg",
            Self::IsoMp4 => "audio/mp4",
            Self::Flac => "audio/flac",
            Self::Ogg => "audio/ogg",
            Self::Wav => "audio/wav",
            Self::Caf => "audio/x-caf",
            Self::Amr => "audio/amr",
        }
    }

    /// The extension the cached file is stored under, never the uploader's.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::IsoMp4 => "m4a",
            Self::Flac => "flac",
            Self::Ogg => "ogg",
            Self::Wav => "wav",
            Self::Caf => "caf",
            Self::Amr => "amr",
        }
    }

    /// Whether end-padding must be trimmed by us, not the decoder (`gapless.rs`).
    pub const fn needs_our_gapless_trim(self) -> bool {
        matches!(self, Self::IsoMp4)
    }
}

/// The audio codecs the client will decode, plus the ones it recognises and
/// declines.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AudioCodec {
    Mp3,
    AacLc,
    Alac,
    Flac,
    Vorbis,
    Pcm,
    Opus,
    Amr,
}

/// Why a recognised codec will not play.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unsupported {
    /// symphonia has no decoder, and adding one means a C library or LGPL FFmpeg.
    NoPureRustDecoder,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Support {
    Decodable,
    Refused(Unsupported),
}

impl AudioCodec {
    pub const ALL: [Self; 8] = [
        Self::Mp3,
        Self::AacLc,
        Self::Alac,
        Self::Flac,
        Self::Vorbis,
        Self::Pcm,
        Self::Opus,
        Self::Amr,
    ];

    pub const fn support(self) -> Support {
        match self {
            Self::Mp3 | Self::AacLc | Self::Alac | Self::Flac | Self::Vorbis | Self::Pcm => {
                Support::Decodable
            }
            Self::Opus | Self::Amr => Support::Refused(Unsupported::NoPureRustDecoder),
        }
    }

    pub const fn is_decodable(self) -> bool {
        matches!(self.support(), Support::Decodable)
    }
}

/// Bytes needed to identify anything here.
pub const SNIFF_BYTES: usize = 16;

/// Identify a container from its first bytes.
pub fn sniff(head: &[u8]) -> Option<Container> {
    if head.starts_with(b"ID3") {
        return Some(Container::Mp3);
    }
    if head.len() >= 2 && head[0] == 0xFF && (head[1] & 0xE0) == 0xE0 {
        return Some(Container::Mp3);
    }
    if head.len() >= 8 && &head[4..8] == b"ftyp" {
        return Some(Container::IsoMp4);
    }
    if head.starts_with(b"fLaC") {
        return Some(Container::Flac);
    }
    if head.starts_with(b"OggS") {
        return Some(Container::Ogg);
    }
    if head.starts_with(b"RIFF") {
        return Some(Container::Wav);
    }
    if head.starts_with(b"caff") {
        return Some(Container::Caf);
    }
    if head.starts_with(b"#!AMR") {
        return Some(Container::Amr);
    }

    None
}

/// Whether a file is plausibly audio at all — a failed download can leave an
/// HTML error page in the cache under a `.mp3` name.
pub fn is_probably_audio(head: &[u8]) -> bool {
    sniff(head).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(bytes: &[u8]) -> Vec<u8> {
        let mut head = bytes.to_vec();
        head.resize(SNIFF_BYTES, 0);
        head
    }

    #[test]
    fn every_container_the_backend_accepts_is_identified() {
        assert_eq!(sniff(&head(b"ID3\x04\x00")), Some(Container::Mp3));
        assert_eq!(
            sniff(&head(&[0xFF, 0xFB, 0x90, 0x00])),
            Some(Container::Mp3)
        );
        assert_eq!(
            sniff(&head(b"\x00\x00\x00\x20ftypM4A ")),
            Some(Container::IsoMp4)
        );
        assert_eq!(sniff(&head(b"fLaC")), Some(Container::Flac));
        assert_eq!(sniff(&head(b"OggS")), Some(Container::Ogg));
        assert_eq!(sniff(&head(b"RIFF....WAVE")), Some(Container::Wav));
        assert_eq!(sniff(&head(b"caff")), Some(Container::Caf));
        assert_eq!(sniff(&head(b"#!AMR\n")), Some(Container::Amr));
    }

    #[test]
    fn an_error_page_saved_under_an_mp3_name_is_not_audio() {
        assert!(!is_probably_audio(b"<!DOCTYPE html><html><head>"));
        assert!(!is_probably_audio(b"{\"error\":\"not found\"}"));
        assert!(!is_probably_audio(b""));
    }

    #[test]
    fn a_free_format_mpeg_sync_word_is_not_mistaken_for_something_else() {
        // Every 11-bit sync word, whatever the layer and rate bits say.
        for second in [0xE0u8, 0xE3, 0xF3, 0xFA, 0xFB, 0xFF] {
            assert_eq!(
                sniff(&head(&[0xFF, second])),
                Some(Container::Mp3),
                "second byte {second:#04x}"
            );
        }
    }

    #[test]
    fn the_codecs_with_no_pure_rust_decoder_are_named_rather_than_played_silently() {
        assert_eq!(
            AudioCodec::Opus.support(),
            Support::Refused(Unsupported::NoPureRustDecoder)
        );
        assert_eq!(
            AudioCodec::Amr.support(),
            Support::Refused(Unsupported::NoPureRustDecoder)
        );
    }

    #[test]
    fn everything_the_music_catalogue_serves_is_decodable() {
        // The catalogue is mp3 originals plus AAC-in-MP4 from the extraction job.
        assert!(AudioCodec::Mp3.is_decodable());
        assert!(AudioCodec::AacLc.is_decodable());
    }

    #[test]
    fn only_the_iso_container_needs_our_own_trimming() {
        assert!(Container::IsoMp4.needs_our_gapless_trim());
        assert!(!Container::Mp3.needs_our_gapless_trim());
        assert!(!Container::Flac.needs_our_gapless_trim());
    }
}
