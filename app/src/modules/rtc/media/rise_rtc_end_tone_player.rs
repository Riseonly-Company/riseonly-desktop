use std::collections::HashSet;
use std::f64::consts::PI;
use std::sync::Arc;
use std::time::Duration;

use futures::channel::oneshot;
use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_media::sound_clip::{SoundClip, SoundClipError, SoundClipPlayback};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;

/// the played clip and the number that tells its finish from a later clips
struct EndTonePlayback {
    playback: SoundClipPlayback,
    generation: u64,
}

pub struct RiseRTCEndTonePlayer {
    player: Option<EndTonePlayback>,
    player_generation: u64,
    played_identities: HashSet<String>,
    pending_identities: HashSet<String>,
    played_order: Vec<String>,
    tone_data: Option<Arc<[u8]>>,
}

impl RiseRTCEndTonePlayer {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            player: None,
            player_generation: 0,
            played_identities: HashSet::new(),
            pending_identities: HashSet::new(),
            played_order: Vec::new(),
            tone_data: None,
        }
    }

    /// activates_audio_session only sets the retry budget
    /// the caller that would activate a session plays after the calls own audio went down
    /// when the output can take a moment to come back so it retries, desktop has no audio session
    pub fn play_once(&mut self, call_identity: &str, activates_audio_session: bool, cx: &mut Context<Self>) {
        let identity = trimming_whitespaces_and_newlines(call_identity).to_owned();
        if identity.is_empty()
            || self.played_identities.contains(&identity)
            || !self.pending_identities.insert(identity.clone())
        {
            return;
        }
        let retries_remaining = if activates_audio_session { 5 } else { 0 };
        self.attempt_playback(identity, activates_audio_session, retries_remaining, cx);
    }

    fn attempt_playback(
        &mut self,
        identity: String,
        activates_audio_session: bool,
        retries_remaining: u32,
        cx: &mut Context<Self>,
    ) {
        let tone_data = self.tone_data.get_or_insert_with(|| Arc::from(Self::make_tone_data())).clone();
        let started = SoundClip::from_wav(tone_data).and_then(|clip| {
            let (finished_tx, finished_rx) = oneshot::channel::<bool>();
            let playback = clip.play(0.78, move |successfully| {
                let _ = finished_tx.send(successfully);
            })?;
            Ok((playback, finished_rx))
        });
        match started {
            Ok((playback, finished_rx)) => {
                self.player_generation += 1;
                let generation = self.player_generation;
                self.player = Some(EndTonePlayback { playback, generation });
                self.pending_identities.remove(&identity);
                self.played_identities.insert(identity.clone());
                self.played_order.push(identity);
                while self.played_order.len() > 32 {
                    let oldest = self.played_order.remove(0);
                    self.played_identities.remove(&oldest);
                }
                cx.spawn(async move |this, cx| {
                    let successfully = finished_rx.await.unwrap_or(false);
                    let _ = this.update(cx, |player, _| player.audio_player_did_finish_playing(generation, successfully));
                })
                .detach();
            }
            Err(error) => {
                self.schedule_retry(identity, activates_audio_session, retries_remaining, Some(error), cx);
            }
        }
    }

    fn schedule_retry(
        &mut self,
        identity: String,
        activates_audio_session: bool,
        retries_remaining: u32,
        final_error: Option<SoundClipError>,
        cx: &mut Context<Self>,
    ) {
        if retries_remaining == 0 {
            self.pending_identities.remove(&identity);
            let error_type = final_error
                .map(|error| format!("{}::{error:?}", std::any::type_name::<SoundClipError>()))
                .unwrap_or_else(|| "playback_not_started".to_owned());
            Sai::log("RTC", move || format!("end tone unavailable error_type={error_type}"), SaiLogKind::Network, cx);
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(180)).await;
            let _ = this.update(cx, |player, cx| {
                if !player.pending_identities.contains(&identity) || player.played_identities.contains(&identity) {
                    return;
                }
                player.attempt_playback(identity, activates_audio_session, retries_remaining - 1, cx);
            });
        })
        .detach();
    }

    fn audio_player_did_finish_playing(&mut self, generation: u64, _successfully: bool) {
        if self.player.as_ref().is_none_or(|player| player.generation != generation) {
            return;
        }
        self.player = None;
    }

    fn make_tone_data() -> Vec<u8> {
        let sample_rate: usize = 48_000;
        let duration = 0.58;
        let sample_count = (sample_rate as f64 * duration) as usize;
        let pulse_starts = [0.0, 0.145, 0.29];
        let pulse_frequencies = [620.0, 520.0, 430.0];
        let pulse_duration = 0.105;
        let mut samples = vec![0_i16; sample_count];

        for (pulse_start, frequency) in pulse_starts.iter().zip(pulse_frequencies.iter()) {
            let start_sample = (pulse_start * sample_rate as f64) as usize;
            let pulse_samples = (pulse_duration * sample_rate as f64) as usize;
            for offset in 0..pulse_samples {
                let index = start_sample + offset;
                if index >= samples.len() {
                    break;
                }
                let progress = offset as f64 / 1_usize.max(pulse_samples - 1) as f64;
                let attack = (progress / 0.1).min(1.0);
                let release = ((1.0 - progress) / 0.28).min(1.0);
                let envelope = attack.min(release);
                let time = offset as f64 / sample_rate as f64;
                let fundamental = (2.0 * PI * frequency * time).sin();
                let harmonic = (2.0 * PI * frequency * 1.5 * time).sin() * 0.13;
                let value = (fundamental + harmonic) * envelope * 0.25;
                samples[index] = (value.min(1.0).max(-1.0) * f64::from(i16::MAX)) as i16;
            }
        }

        let mut pcm = Vec::with_capacity(samples.len() * std::mem::size_of::<i16>());
        for sample in samples {
            Self::append_little_endian(&sample.to_le_bytes(), &mut pcm);
        }

        let pcm_count = pcm.len() as u32;
        let mut wave = Vec::with_capacity(44 + pcm.len());
        wave.extend_from_slice(b"RIFF");
        Self::append_little_endian(&(36 + pcm_count).to_le_bytes(), &mut wave);
        wave.extend_from_slice(b"WAVE");
        wave.extend_from_slice(b"fmt ");
        Self::append_little_endian(&16_u32.to_le_bytes(), &mut wave);
        Self::append_little_endian(&1_u16.to_le_bytes(), &mut wave);
        Self::append_little_endian(&1_u16.to_le_bytes(), &mut wave);
        Self::append_little_endian(&(sample_rate as u32).to_le_bytes(), &mut wave);
        Self::append_little_endian(&(sample_rate as u32 * 2).to_le_bytes(), &mut wave);
        Self::append_little_endian(&2_u16.to_le_bytes(), &mut wave);
        Self::append_little_endian(&16_u16.to_le_bytes(), &mut wave);
        wave.extend_from_slice(b"data");
        Self::append_little_endian(&pcm_count.to_le_bytes(), &mut wave);
        wave.extend_from_slice(&pcm);
        wave
    }

    /// bytes of value.to_le_bytes(), rust spells the little endian conversion per type
    fn append_little_endian(value: &[u8], data: &mut Vec<u8>) {
        data.extend_from_slice(value);
    }
}
