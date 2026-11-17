use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::AsyncApp;
use rise_core::foundation::foundation_url::Url;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_media::audio::decoder::{AudioDecoder, Decoded};
use rise_media::audio::symphonia_backend::SymphoniaDecoder;
use rise_platform::file_manager::FileManager;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::music::shared::utils::music::music_playback_disk_cache::MusicPlaybackDiskCache;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;

/// offline peak extraction for a full audio file for the editor trim timeline
/// no server waveform
/// peaks come from the local upload or playback cache or a one off download
pub enum MusicTrackWaveform {}

struct MusicTrackWaveformCancelGuard(Arc<AtomicBool>);

impl Drop for MusicTrackWaveformCancelGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl MusicTrackWaveform {
    pub fn local_file_url(track: &RiseStoredMusicTrack) -> Option<Url> {
        if let Some(url) = track.audio_url.as_deref().and_then(Url::from_string) {
            let exists = url.is_file_url() && url.to_file_path().is_some_and(|path| FileManager.file_exists(&path));
            if exists {
                return Some(url);
            }
        }
        if let Some(id) = track.id.as_deref().filter(|id| !id.is_empty()) {
            let user_id = SaiAccountCacheScope::active_user_id();
            let cached = MusicPlaybackDiskCache::find_cached_url(id, user_id.as_deref()).or_else(|| {
                MusicPlaybackDiskCache::find_cached_url(&MusicPlaybackDiskCache::ambient_key(id), user_id.as_deref())
            });
            if let Some(cached) = cached {
                return Url::from_file_path(&cached);
            }
        }
        None
    }

    pub async fn peaks(track: &RiseStoredMusicTrack, sample_count: i64, cx: &mut AsyncApp) -> Option<Vec<f32>> {
        let bounded_count = sample_count.clamp(16, 2_048) as usize;
        if let Some(local) = Self::local_file_url(track) {
            return Self::extract_peaks(local, bounded_count).await;
        }

        let id = track.id.as_deref().filter(|id| !id.is_empty())?;
        let normalized = RemoteMediaURL::optional_normalized(track.audio_url.as_deref())?;
        let remote = Url::from_string(&normalized)?;

        let token = cx.update(|cx| AuthServicesStore::shared(cx).read(cx).tokens_and_other_data().access_token.clone());
        let downloaded = MusicPlayerState::materialize_audio_for_playback(
            remote,
            MusicPlaybackDiskCache::ambient_key(id),
            SaiAccountCacheScope::active_user_id(),
            SaiAccountCacheScope::active_epoch(),
            Some(token),
            cx,
        )
        .await?;
        Self::extract_peaks(downloaded, bounded_count).await
    }

    async fn extract_peaks(url: Url, sample_count: usize) -> Option<Vec<f32>> {
        let path = url.to_file_path()?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel_on_drop = MusicTrackWaveformCancelGuard(Arc::clone(&cancelled));
        let task = RiseAccountRuntime::current().spawn_blocking(move || {
            let bytes = std::fs::read(&path).ok()?;
            Self::decode_peaks(bytes, sample_count, &cancelled)
        });
        task.await.ok().flatten()
    }

    fn decode_peaks(bytes: Vec<u8>, sample_count: usize, cancelled: &AtomicBool) -> Option<Vec<f32>> {
        let mut decoder = SymphoniaDecoder::open(bytes).ok()?;
        let format = decoder.format();
        let channels = usize::from(format.channels.max(1));
        let sample_rate = f64::from(format.sample_rate.max(1));

        let duration_seconds = match decoder.duration_millis() {
            Some(millis) => millis as f64 / 1_000.0,
            None => {
                let frames = Self::count_frames(&mut decoder, cancelled)?;
                decoder.seek(0).ok()?;
                frames as f64 / sample_rate
            }
        };
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            return None;
        }

        let mut sum_squares = vec![0.0_f64; sample_count];
        let mut counts = vec![0_u64; sample_count];
        let seconds_per_frame = 1.0 / sample_rate;
        let last_bucket = sample_count.saturating_sub(1) as f64;
        let mut frame_index: u64 = 0;
        let mut buffer: Vec<f32> = Vec::new();

        loop {
            if cancelled.load(Ordering::Acquire) {
                return None;
            }
            buffer.clear();
            match decoder.decode(&mut buffer) {
                Ok(Decoded::Frames(_)) => {}
                Ok(Decoded::EndOfStream) => break,
                Err(_) => return None,
            }
            for frame in buffer.chunks_exact(channels) {
                let mono = frame.iter().sum::<f32>() / channels as f32;
                let value = f64::from(mono.clamp(-1.0, 1.0));
                let time = frame_index as f64 * seconds_per_frame;
                let position = time / duration_seconds * sample_count as f64;
                let bucket = position.clamp(0.0, last_bucket) as usize;
                sum_squares[bucket] += value * value;
                counts[bucket] += 1;
                frame_index += 1;
            }
        }

        let mut peaks: Vec<f32> = sum_squares
            .iter()
            .zip(&counts)
            .map(|(sum, count)| if *count > 0 { (sum / *count as f64).sqrt() as f32 } else { 0.0 })
            .collect();
        let maximum = peaks.iter().copied().fold(0.0_f32, f32::max);
        if maximum > 0.001 {
            for peak in &mut peaks {
                *peak = (*peak / maximum).min(1.0);
            }
        }
        Some(peaks)
    }

    fn count_frames(decoder: &mut SymphoniaDecoder, cancelled: &AtomicBool) -> Option<u64> {
        let mut buffer: Vec<f32> = Vec::new();
        let mut frames: u64 = 0;
        loop {
            if cancelled.load(Ordering::Acquire) {
                return None;
            }
            buffer.clear();
            match decoder.decode(&mut buffer).ok()? {
                Decoded::Frames(count) => frames += count as u64,
                Decoded::EndOfStream => return Some(frames),
            }
        }
    }
}
