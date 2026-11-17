/// the wrapper the bytes are in, not the codec
/// an mp4 holds aac or alac and only the demuxer knows
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Container {
    Mp3,
    IsoMp4,
    Flac,
    Ogg,
    Wav,
    Caf,
    Amr,
}

impl Container {
    /// mime type the response header is overridden with
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

    /// extension the cached file is stored under, never the uploader one
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

    /// true when end padding must be trimmed here and not by the decoder
    pub const fn needs_our_gapless_trim(self) -> bool {
        matches!(self, Self::IsoMp4)
    }
}

/// audio codecs the client decodes, plus ones it recognises and declines
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

/// why a recognised codec wont play
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unsupported {
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

pub const SNIFF_BYTES: usize = 16;

/// identifies a container from its first bytes
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

/// true if the file looks like audio, a failed download can cache an html page as .mp3
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
