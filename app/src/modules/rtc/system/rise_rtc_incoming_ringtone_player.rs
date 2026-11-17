use std::f64::consts::PI;
use std::sync::LazyLock;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_media::audio::av_audio_player::AVAudioPlayer;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;

pub struct RiseRTCIncomingRingtonePlayer {
    player: Option<AVAudioPlayer>,
    active_call_identity: Option<String>,
}

static TONE_DATA: LazyLock<Vec<u8>> = LazyLock::new(RiseRTCIncomingRingtonePlayer::make_tone_data);

impl RiseRTCIncomingRingtonePlayer {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { player: None, active_call_identity: None }
    }

    pub fn start(&mut self, call_identity: &str, cx: &mut Context<Self>) {
        let identity = trimming_whitespaces_and_newlines(call_identity);
        if identity.is_empty() || self.active_call_identity.as_deref() == Some(identity) {
            return;
        }
        self.stop();
        match AVAudioPlayer::with_data(TONE_DATA.clone()) {
            Ok(mut player) => {
                player.set_number_of_loops(-1);
                player.set_volume(0.7);
                player.prepare_to_play();
                if !player.play() {
                    return;
                }
                self.player = Some(player);
                self.active_call_identity = Some(identity.to_owned());
            }
            Err(error) => {
                Sai::log("RTC", move || format!("ringtone unavailable: {error}"), SaiLogKind::Network, cx);
            }
        }
    }

    pub fn stop(&mut self) {
        if let Some(mut player) = self.player.take() {
            player.stop();
        }
        self.active_call_identity = None;
    }

    fn make_tone_data() -> Vec<u8> {
        let sample_rate: u32 = 48_000;
        let duration = 3.2_f64;
        let sample_count = (f64::from(sample_rate) * duration) as usize;
        let burst_starts = [0.0_f64, 0.9];
        let burst_duration = 0.7_f64;
        let mut samples = vec![0_i16; sample_count];

        for burst_start in burst_starts {
            let start_sample = (burst_start * f64::from(sample_rate)) as usize;
            let burst_samples = (burst_duration * f64::from(sample_rate)) as usize;
            for offset in 0..burst_samples {
                let index = start_sample + offset;
                let Some(slot) = samples.get_mut(index) else {
                    break;
                };
                let progress = offset as f64 / burst_samples.saturating_sub(1).max(1) as f64;
                let envelope = (progress / 0.06).min(1.0).min((1.0 - progress) / 0.12).min(1.0);
                let time = offset as f64 / f64::from(sample_rate);
                let low = (2.0 * PI * 660.0 * time).sin();
                let high = (2.0 * PI * 880.0 * time).sin();
                let value = (low + high) * 0.5 * envelope * 0.3;
                *slot = (value.clamp(-1.0, 1.0) * f64::from(i16::MAX)) as i16;
            }
        }

        let mut pcm = Vec::with_capacity(samples.len() * size_of::<i16>());
        for sample in samples {
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
        let pcm_len = u32::try_from(pcm.len()).unwrap_or(u32::MAX);

        let mut wave = Vec::with_capacity(44 + pcm.len());
        wave.extend_from_slice(b"RIFF");
        wave.extend_from_slice(&36_u32.saturating_add(pcm_len).to_le_bytes());
        wave.extend_from_slice(b"WAVE");
        wave.extend_from_slice(b"fmt ");
        wave.extend_from_slice(&16_u32.to_le_bytes());
        wave.extend_from_slice(&1_u16.to_le_bytes());
        wave.extend_from_slice(&1_u16.to_le_bytes());
        wave.extend_from_slice(&sample_rate.to_le_bytes());
        wave.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        wave.extend_from_slice(&2_u16.to_le_bytes());
        wave.extend_from_slice(&16_u16.to_le_bytes());
        wave.extend_from_slice(b"data");
        wave.extend_from_slice(&pcm_len.to_le_bytes());
        wave.extend_from_slice(&pcm);
        wave
    }
}
