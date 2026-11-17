use std::path::{Path, PathBuf};

use futures::channel::oneshot;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_media::audio::format::StreamFormat;
use rise_media::audio::pump::map_channels;
use rise_media::audio::resample::Resampler;
use rise_media::audio_capture::{AAC_SAMPLE_RATES, AacFileWriter, AacQuality, AacSettings, aac_file_writer};

use crate::modules::music::shared::utils::music::music_audio_effect_settings::{
    MusicAudioEffectDSP, MusicAudioEffectSettings,
};
use crate::modules::music::shared::utils::music::music_effect_audio_graph::{
    AVAudioEngineManualRenderingStatus, MusicEffectAudioChain, MusicEffectAudioEngine,
    MusicEffectAudioFile, MusicEffectPlayerNode,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicEffectRenderError {
    OpenFailed,
    UnsupportedFormat,
    EngineFailed,
    WriteFailed,
}

/// renders a track through the slowed + reverb chain faster than realtime, encodes to aac
/// aac since platforms decode mp3 but cant encode it
pub enum MusicEffectOfflineRenderer {}

impl MusicEffectOfflineRenderer {
    pub const OUTPUT_FILE_EXTENSION: &str = "m4a";

    const RENDER_BLOCK_FRAMES: usize = 8_192;
    const ENCODER_BIT_RATE: u32 = 256_000;
    const MAXIMUM_TAIL_SECONDS: f64 = 3.5;
    const FALLBACK_ENCODER_RATE: u32 = 48_000;
    const MAXIMUM_ENCODER_CHANNELS: u16 = 2;

    pub async fn render(
        source: PathBuf,
        destination: PathBuf,
        settings: MusicAudioEffectSettings,
        on_progress: impl Fn(f64) + Send + Sync + 'static,
    ) -> Result<(), MusicEffectRenderError> {
        let (sender, receiver) = oneshot::channel();
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            let _ = sender.send(Self::render_synchronously(
                &source,
                &destination,
                &settings,
                &on_progress,
            ));
        }));
        receiver
            .await
            .unwrap_or(Err(MusicEffectRenderError::EngineFailed))
    }

    fn render_synchronously(
        source: &Path,
        destination: &Path,
        settings: &MusicAudioEffectSettings,
        on_progress: &(impl Fn(f64) + Send + Sync),
    ) -> Result<(), MusicEffectRenderError> {
        let input_file =
            MusicEffectAudioFile::for_reading(source).map_err(|_| MusicEffectRenderError::OpenFailed)?;
        let source_format = input_file.processing_format();
        if source_format.sample_rate == 0 || source_format.channels == 0 || input_file.length() <= 0 {
            return Err(MusicEffectRenderError::UnsupportedFormat);
        }
        let render_format = StreamFormat::new(source_format.sample_rate, source_format.channels)
            .map_err(|_| MusicEffectRenderError::UnsupportedFormat)?;
        let sample_rate = f64::from(source_format.sample_rate);

        let clamped_rate = settings.playback_rate();
        let mut engine = MusicEffectAudioEngine::new();
        let player = MusicEffectPlayerNode::new();

        let mut chain = MusicEffectAudioChain::new(sample_rate);
        chain.attach(&mut engine, &player, source_format, settings);
        let _ = chain.update_reverb_preset(settings);
        chain.apply(settings);
        chain.set_reverb_mix(MusicAudioEffectDSP::reverb_wet_dry_mix(settings));
        chain.set_ambience_muted(false);

        engine.enable_manual_rendering_mode(render_format, Self::RENDER_BLOCK_FRAMES);
        player.schedule_file(&input_file);
        player.play();

        let encoder_settings = Self::encoder_settings(source_format);
        let mut output_file = aac_file_writer(destination, &encoder_settings)
            .map_err(|_| MusicEffectRenderError::EngineFailed)?;
        let mut encoder_input = Self::encoder_converter(source_format, encoder_settings);

        let tail_frames =
            (chain.reverb_tail_time().min(Self::MAXIMUM_TAIL_SECONDS) * sample_rate) as i64;
        let total_frames = (input_file.length() as f64 / clamped_rate) as i64 + tail_frames;
        if total_frames <= 0 {
            return Err(MusicEffectRenderError::UnsupportedFormat);
        }

        let mut buffer: Vec<f32> = Vec::with_capacity(
            engine.manual_rendering_maximum_frame_count() * usize::from(render_format.channels),
        );
        let mut converted = Vec::new();
        while engine.manual_rendering_sample_time() < total_frames {
            let remaining = total_frames - engine.manual_rendering_sample_time();
            let frames = (engine.manual_rendering_maximum_frame_count() as i64).min(remaining);
            if frames <= 0 {
                break;
            }

            chain.advance_spin(frames as f64 / sample_rate, settings);

            match engine.render_offline(frames as usize, &mut buffer) {
                AVAudioEngineManualRenderingStatus::Success => {
                    encoder_input.convert(&buffer, &mut converted);
                    output_file
                        .write(&converted)
                        .map_err(|_| MusicEffectRenderError::WriteFailed)?;
                    on_progress(
                        (engine.manual_rendering_sample_time() as f64 / total_frames as f64).min(1.0),
                    );
                }
                AVAudioEngineManualRenderingStatus::InsufficientDataFromInputNode => {
                    return output_file
                        .finish()
                        .map_err(|_| MusicEffectRenderError::WriteFailed);
                }
                AVAudioEngineManualRenderingStatus::CannotDoInCurrentContext => continue,
                AVAudioEngineManualRenderingStatus::Error => {
                    return Err(MusicEffectRenderError::EngineFailed);
                }
            }
        }
        player.stop();
        engine.disable_manual_rendering_mode();
        output_file
            .finish()
            .map_err(|_| MusicEffectRenderError::WriteFailed)?;
        on_progress(1.0);
        Ok(())
    }

    /// desktop encoders only take AAC-LC rates in mono or stereo, so it converts here
    fn encoder_settings(source: StreamFormat) -> AacSettings {
        let sample_rate = if AAC_SAMPLE_RATES.contains(&source.sample_rate) {
            source.sample_rate
        } else {
            Self::FALLBACK_ENCODER_RATE
        };
        AacSettings {
            sample_rate,
            channels: source.channels.min(Self::MAXIMUM_ENCODER_CHANNELS),
            bit_rate: Some(Self::ENCODER_BIT_RATE),
            quality: AacQuality::High,
        }
    }

    fn encoder_converter(source: StreamFormat, encoder: AacSettings) -> MusicEffectEncoderInput {
        MusicEffectEncoderInput {
            source,
            channels: encoder.channels,
            resampler: (source.sample_rate != encoder.sample_rate)
                .then(|| Resampler::new(source, encoder.sample_rate)),
            resampled: Vec::new(),
        }
    }
}

/// fits rendered blocks to the encoder, rate first then channel count
struct MusicEffectEncoderInput {
    source: StreamFormat,
    channels: u16,
    resampler: Option<Resampler>,
    resampled: Vec<f32>,
}

impl MusicEffectEncoderInput {
    fn convert(&mut self, rendered: &[f32], out: &mut Vec<f32>) {
        self.resampled.clear();
        let at_encoder_rate: &[f32] = match self.resampler.as_mut() {
            Some(resampler) => {
                resampler.process(rendered, &mut self.resampled);
                &self.resampled
            }
            None => rendered,
        };
        out.clear();
        map_channels(at_encoder_rate, self.source.channels, self.channels, out);
    }
}

pub enum MusicEffectMix {}

impl MusicEffectMix {
    pub const REVERB_WET_DRY_MIX: f32 = 28.0;
}
