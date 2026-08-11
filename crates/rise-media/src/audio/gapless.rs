//! Removing the silence the encoder added, so an album plays as one piece.
//!
//! symphonia already applies the MP3 LAME tag itself; the AAC `iTunSMPB` atom
//! is ours to parse.

/// How much of a decoded stream is real audio.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Trim {
    /// Priming frames to drop from the front.
    pub start_frames: u64,
    /// Real frames after the priming. `None` when the source did not say, in
    /// which case the end padding is left alone rather than guessed at.
    pub valid_frames: Option<u64>,
}

impl Trim {
    pub const NONE: Self = Self {
        start_frames: 0,
        valid_frames: None,
    };

    pub fn is_none(&self) -> bool {
        *self == Self::NONE
    }
}

/// Applies a `Trim` to a stream of decoded blocks, in order.
#[derive(Clone, Copy, Debug)]
pub struct Trimmer {
    trim: Trim,
    skipped: u64,
    emitted: u64,
}

/// The part of a decoded block that is real audio, as an index range in frames.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Kept {
    pub start: usize,
    pub end: usize,
}

impl Kept {
    pub const fn len(&self) -> usize {
        self.end - self.start
    }

    pub const fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

impl Trimmer {
    pub const fn new(trim: Trim) -> Self {
        Self {
            trim,
            skipped: 0,
            emitted: 0,
        }
    }

    pub const fn emitted(&self) -> u64 {
        self.emitted
    }

    /// Whether the valid audio has all been emitted. The caller stops here
    /// rather than playing the encoder's end padding.
    pub fn is_complete(&self) -> bool {
        self.trim
            .valid_frames
            .is_some_and(|valid| self.emitted >= valid)
    }

    /// Which frames of the next decoded block to keep.
    pub fn accept(&mut self, block_frames: usize) -> Kept {
        let block = block_frames as u64;

        let skip = (self.trim.start_frames - self.skipped).min(block);
        self.skipped += skip;

        let mut available = block - skip;
        if let Some(valid) = self.trim.valid_frames {
            available = available.min(valid.saturating_sub(self.emitted));
        }
        self.emitted += available;

        Kept {
            start: skip as usize,
            end: (skip + available) as usize,
        }
    }

    /// Restart the trim from a seek.
    pub fn seek_to(&mut self, frame: u64) {
        self.skipped = self.trim.start_frames.min(frame);
        self.emitted = frame.saturating_sub(self.trim.start_frames);
    }
}

/// The iTunes gapless atom, as written by iTunes, afconvert and FFmpeg.
pub fn parse_itunsmpb(value: &str) -> Option<Trim> {
    let mut fields = value.split_ascii_whitespace();

    let _leading = fields.next()?;
    let delay = u64::from_str_radix(fields.next()?, 16).ok()?;
    let padding = u64::from_str_radix(fields.next()?, 16).ok()?;
    let total = u64::from_str_radix(fields.next()?, 16).ok()?;

    if !is_plausible(delay, padding) {
        return None;
    }

    Some(Trim {
        start_frames: delay,
        valid_frames: Some(total),
    })
}

/// Find the `iTunSMPB` value in an MP4's metadata.
///
/// Every read is bounds-checked: the input is an untrusted file off the network.
pub fn find_itunsmpb(bytes: &[u8]) -> Option<&str> {
    const NAME: &[u8] = b"iTunSMPB";
    /// The `data` atom follows the `name` atom immediately.
    const SEARCH_WINDOW: usize = 64;

    let name_at = bytes
        .windows(NAME.len())
        .position(|window| window == NAME)?;
    let after_name = name_at + NAME.len();

    let window_end = (after_name + SEARCH_WINDOW).min(bytes.len());
    let data_at = after_name
        + bytes[after_name..window_end]
            .windows(4)
            .position(|window| window == b"data")?;

    // size(4) 'data'(4) version+flags(4) reserved(4) then the payload.
    let size = u32::from_be_bytes(bytes.get(data_at - 4..data_at)?.try_into().ok()?) as usize;
    if !(16..=1024).contains(&size) {
        return None;
    }

    let payload = bytes.get(data_at + 12..data_at - 4 + size)?;
    std::str::from_utf8(payload).ok()
}

/// Frames one MPEG audio frame carries, from the four-byte frame header.
pub fn mpeg_frames_per_packet(header: [u8; 4]) -> Option<u64> {
    if header[0] != 0xFF || (header[1] & 0xE0) != 0xE0 {
        return None;
    }

    let layer = (header[1] >> 1) & 0b11;
    if layer != 0b01 {
        return None;
    }

    match (header[1] >> 3) & 0b11 {
        0b11 => Some(1152),
        0b10 | 0b00 => Some(576),
        _ => None,
    }
}

/// The LAME/Xing tag an MP3 encoder writes into its first frame.
pub fn parse_lame_tag(head: &[u8]) -> Option<Trim> {
    /// LAME always writes every Xing field, so its extension begins at a fixed offset.
    const LAME_OFFSET: usize = 120;
    const DELAY_OFFSET: usize = 21;
    const SEARCH_WINDOW: usize = 2048;

    let frames_per_packet = mpeg_frames_per_packet(head.get(..4)?.try_into().ok()?)?;

    let window = &head[..head.len().min(SEARCH_WINDOW)];
    let tag_at = window
        .windows(4)
        .position(|w| w == b"Xing" || w == b"Info")?;

    let flags = u32::from_be_bytes(head.get(tag_at + 4..tag_at + 8)?.try_into().ok()?);
    let packet_count = if flags & 0x0001 != 0 {
        Some(u32::from_be_bytes(head.get(tag_at + 8..tag_at + 12)?.try_into().ok()?) as u64)
    } else {
        None
    };

    let delay_at = tag_at + LAME_OFFSET + DELAY_OFFSET;
    let triple: [u8; 3] = head.get(delay_at..delay_at + 3)?.try_into().ok()?;

    let delay = ((triple[0] as u64) << 4) | ((triple[1] as u64) >> 4);
    let padding = (((triple[1] & 0x0F) as u64) << 8) | triple[2] as u64;

    if !is_plausible(delay, padding) {
        return None;
    }

    // The Xing frame itself is a header, not audio, so it is not counted.
    let valid_frames = packet_count.map(|packets| {
        (packets * frames_per_packet).saturating_sub(delay + padding + frames_per_packet)
    });

    Some(Trim {
        start_frames: delay,
        valid_frames,
    })
}

/// Both fields are twelve bits, so a value near the ceiling means the bytes were not a tag.
fn is_plausible(delay: u64, padding: u64) -> bool {
    delay <= 3_000 && padding <= 6_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_with_no_trim_passes_through_whole() {
        let mut trimmer = Trimmer::new(Trim::NONE);

        assert_eq!(
            trimmer.accept(1024),
            Kept {
                start: 0,
                end: 1024
            }
        );
        assert_eq!(
            trimmer.accept(1024),
            Kept {
                start: 0,
                end: 1024
            }
        );
        assert!(!trimmer.is_complete());
    }

    #[test]
    fn the_priming_is_dropped_even_when_it_spans_several_blocks() {
        let mut trimmer = Trimmer::new(Trim {
            start_frames: 2500,
            valid_frames: None,
        });

        assert_eq!(
            trimmer.accept(1024),
            Kept {
                start: 1024,
                end: 1024
            }
        );
        assert_eq!(
            trimmer.accept(1024),
            Kept {
                start: 1024,
                end: 1024
            }
        );
        assert_eq!(
            trimmer.accept(1024),
            Kept {
                start: 452,
                end: 1024
            }
        );
        assert_eq!(
            trimmer.accept(1024),
            Kept {
                start: 0,
                end: 1024
            }
        );
    }

    #[test]
    fn the_end_padding_is_cut_mid_block() {
        let mut trimmer = Trimmer::new(Trim {
            start_frames: 576,
            valid_frames: Some(1000),
        });

        assert_eq!(
            trimmer.accept(1024),
            Kept {
                start: 576,
                end: 1024
            }
        );
        assert_eq!(trimmer.accept(1024), Kept { start: 0, end: 552 });
        assert!(trimmer.is_complete());
        assert_eq!(trimmer.emitted(), 1000);
    }

    #[test]
    fn nothing_is_emitted_after_the_valid_audio_ends() {
        let mut trimmer = Trimmer::new(Trim {
            start_frames: 0,
            valid_frames: Some(100),
        });

        trimmer.accept(1024);
        assert!(trimmer.accept(1024).is_empty());
        assert_eq!(trimmer.emitted(), 100);
    }

    #[test]
    fn a_seek_past_the_priming_does_not_cut_the_start_of_the_block() {
        let mut trimmer = Trimmer::new(Trim {
            start_frames: 2112,
            valid_frames: Some(1_000_000),
        });

        trimmer.seek_to(500_000);

        assert_eq!(
            trimmer.accept(1024),
            Kept {
                start: 0,
                end: 1024
            }
        );
        assert_eq!(trimmer.emitted(), 500_000 - 2112 + 1024);
    }

    #[test]
    fn a_seek_back_to_zero_reinstates_the_priming_cut() {
        let mut trimmer = Trimmer::new(Trim {
            start_frames: 1000,
            valid_frames: Some(1_000_000),
        });
        trimmer.seek_to(500_000);

        trimmer.seek_to(0);

        assert_eq!(
            trimmer.accept(1024),
            Kept {
                start: 1000,
                end: 1024
            }
        );
    }

    #[test]
    fn a_real_itunsmpb_value_is_read() {
        let value = " 00000000 00000840 000001B0 0000000000633340 \
                     00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000";

        assert_eq!(
            parse_itunsmpb(value),
            Some(Trim {
                start_frames: 0x840,
                valid_frames: Some(0x633340),
            })
        );
    }

    #[test]
    fn an_itunsmpb_value_claiming_an_absurd_delay_is_ignored() {
        let value = " 00000000 000FFFFF 000FFFFF 0000000000633340";

        assert_eq!(parse_itunsmpb(value), None);
    }

    #[test]
    fn a_truncated_itunsmpb_value_is_ignored_rather_than_panicking() {
        for value in ["", " 00000000", " 00000000 00000840", "not hex at all"] {
            assert_eq!(parse_itunsmpb(value), None, "{value:?}");
        }
    }

    fn itunsmpb_atom(value: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0, 0, 0, 16]);
        bytes.extend_from_slice(b"name");
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(b"iTunSMPB");

        let size = 16 + value.len() as u32;
        bytes.extend_from_slice(&size.to_be_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&[0, 0, 0, 1]);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(value.as_bytes());
        bytes
    }

    #[test]
    fn the_gapless_atom_is_found_inside_a_container() {
        let value = " 00000000 00000840 000001B0 0000000000633340";
        let mut file = vec![0u8; 4096];
        file.extend_from_slice(&itunsmpb_atom(value));
        file.extend_from_slice(&[0u8; 128]);

        assert_eq!(find_itunsmpb(&file), Some(value));
        assert_eq!(
            find_itunsmpb(&file).and_then(parse_itunsmpb),
            Some(Trim {
                start_frames: 0x840,
                valid_frames: Some(0x633340),
            })
        );
    }

    #[test]
    fn a_file_without_the_atom_reports_nothing() {
        assert_eq!(find_itunsmpb(&[0u8; 8192]), None);
        assert_eq!(find_itunsmpb(b""), None);
    }

    #[test]
    fn a_corrupt_atom_never_reads_past_the_buffer() {
        let value = " 00000000 00000840 000001B0 0000000000633340";
        let atom = itunsmpb_atom(value);

        for truncated in 1..atom.len() {
            find_itunsmpb(&atom[..truncated]);
        }

        let mut lying = atom.clone();
        let size_at = lying.len() - value.len() - 16;
        lying[size_at..size_at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(find_itunsmpb(&lying), None);
    }

    #[test]
    fn mpeg_frame_sizes_follow_the_version() {
        assert_eq!(mpeg_frames_per_packet([0xFF, 0xFB, 0x90, 0x00]), Some(1152));
        assert_eq!(mpeg_frames_per_packet([0xFF, 0xF3, 0x90, 0x00]), Some(576));
        assert_eq!(mpeg_frames_per_packet([0xFF, 0xE3, 0x90, 0x00]), Some(576));
        assert_eq!(mpeg_frames_per_packet([0x00, 0x00, 0x00, 0x00]), None);
        // Layer II, not III.
        assert_eq!(mpeg_frames_per_packet([0xFF, 0xFD, 0x90, 0x00]), None);
    }

    fn info_tag_frame(delay: u16, padding: u16, packets: u32) -> Vec<u8> {
        let mut frame = vec![0u8; 1024];
        frame[0..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);

        let tag_at = 36;
        frame[tag_at..tag_at + 4].copy_from_slice(b"Info");
        frame[tag_at + 4..tag_at + 8].copy_from_slice(&1u32.to_be_bytes());
        frame[tag_at + 8..tag_at + 12].copy_from_slice(&packets.to_be_bytes());

        let delay_at = tag_at + 120 + 21;
        frame[delay_at] = (delay >> 4) as u8;
        frame[delay_at + 1] = (((delay & 0x0F) << 4) | ((padding >> 8) & 0x0F)) as u8;
        frame[delay_at + 2] = (padding & 0xFF) as u8;
        frame
    }

    #[test]
    fn a_lame_tag_yields_the_delay_padding_and_length() {
        let frame = info_tag_frame(576, 1000, 1000);

        assert_eq!(
            parse_lame_tag(&frame),
            Some(Trim {
                start_frames: 576,
                valid_frames: Some(1000 * 1152 - 576 - 1000 - 1152),
            })
        );
    }

    #[test]
    fn a_lame_tag_with_nonsense_in_the_delay_field_is_ignored() {
        let frame = info_tag_frame(4000, 5000, 1000);

        assert_eq!(parse_lame_tag(&frame), None);
    }

    #[test]
    fn a_file_with_no_tag_at_all_reports_nothing() {
        let mut frame = vec![0u8; 1024];
        frame[0..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);

        assert_eq!(parse_lame_tag(&frame), None);
        assert_eq!(parse_lame_tag(&[]), None);
    }

    #[test]
    fn two_tracks_trimmed_and_joined_lose_no_frames_and_add_no_silence() {
        // An album split into two files plays back as exactly the master's frames.
        let first = Trim {
            start_frames: 1105,
            valid_frames: Some(44_100 * 3),
        };
        let second = Trim {
            start_frames: 1105,
            valid_frames: Some(44_100 * 4),
        };

        let mut played = 0;
        for trim in [first, second] {
            let mut trimmer = Trimmer::new(trim);
            let mut decoded = 0;
            while !trimmer.is_complete() && decoded < 44_100 * 10 {
                played += trimmer.accept(1024).len() as u64;
                decoded += 1024;
            }
        }

        assert_eq!(played, 44_100 * 7);
    }
}
