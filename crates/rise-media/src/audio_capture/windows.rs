use std::ffi::c_void;
use std::io::ErrorKind;
use std::iter;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::sync::OnceLock;

use ::windows::Win32::Foundation::{E_POINTER, RPC_E_CHANGED_MODE};
use ::windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFMediaBuffer, IMFMediaType, IMFSample, IMFSinkWriter, MF_E_TOPO_CODEC_NOT_FOUND,
    MF_MT_AAC_PAYLOAD_TYPE, MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE,
    MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE,
    MF_MT_SUBTYPE, MF_TRANSCODE_CONTAINERTYPE, MF_VERSION, MFAudioFormat_AAC, MFAudioFormat_PCM,
    MFMediaType_Audio, MFSTARTUP_LITE, MFTranscodeContainerType_MPEG4,
};
use ::windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use ::windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use ::windows::core::{HRESULT, Interface, PCSTR, PCWSTR, s, w};

use super::audio_capture::*;
use super::media_foundation_aac::*;
use crate::audio::format::StreamFormat;
use crate::audio::resample::Resampler;

pub(super) struct NativeAacEncoder;

type MfStartupFn = unsafe extern "system" fn(u32, u32) -> HRESULT;
type MfShutdownFn = unsafe extern "system" fn() -> HRESULT;
type MfCreateAttributesFn = unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT;
type MfCreateMediaTypeFn = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type MfCreateMemoryBufferFn = unsafe extern "system" fn(u32, *mut *mut c_void) -> HRESULT;
type MfCreateSampleFn = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type MfCreateSinkWriterFromUrlFn =
    unsafe extern "system" fn(PCWSTR, *mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT;

struct MediaFoundation {
    startup: MfStartupFn,
    shutdown: MfShutdownFn,
    create_attributes: MfCreateAttributesFn,
    create_media_type: MfCreateMediaTypeFn,
    create_memory_buffer: MfCreateMemoryBufferFn,
    create_sample: MfCreateSampleFn,
    create_sink_writer_from_url: MfCreateSinkWriterFromUrlFn,
}

fn media_foundation() -> Result<&'static MediaFoundation, AudioCaptureError> {
    static RESOLVED: OnceLock<Result<MediaFoundation, String>> = OnceLock::new();
    RESOLVED
        .get_or_init(resolve_media_foundation)
        .as_ref()
        .map_err(|reason| AudioCaptureError::EncoderUnavailable(reason.clone()))
}

fn resolve_media_foundation() -> Result<MediaFoundation, String> {
    let missing = |library: &str| {
        format!("{library} is not installed (a Windows N edition without the Media Feature Pack)")
    };
    let platform = unsafe { LoadLibraryExW(w!("mfplat.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
        .map_err(|_| missing("mfplat.dll"))?;
    let read_write = unsafe { LoadLibraryExW(w!("mfreadwrite.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
        .map_err(|_| missing("mfreadwrite.dll"))?;
    let export = |module, name: PCSTR, label: &str| {
        unsafe { GetProcAddress(module, name) }.ok_or_else(|| format!("Media Foundation exports no {label}"))
    };
    let startup = export(platform, s!("MFStartup"), "MFStartup")?;
    let shutdown = export(platform, s!("MFShutdown"), "MFShutdown")?;
    let create_attributes = export(platform, s!("MFCreateAttributes"), "MFCreateAttributes")?;
    let create_media_type = export(platform, s!("MFCreateMediaType"), "MFCreateMediaType")?;
    let create_memory_buffer = export(platform, s!("MFCreateMemoryBuffer"), "MFCreateMemoryBuffer")?;
    let create_sample = export(platform, s!("MFCreateSample"), "MFCreateSample")?;
    let create_sink_writer_from_url =
        export(read_write, s!("MFCreateSinkWriterFromURL"), "MFCreateSinkWriterFromURL")?;
    type Exported = unsafe extern "system" fn() -> isize;
    unsafe {
        Ok(MediaFoundation {
            startup: std::mem::transmute::<Exported, MfStartupFn>(startup),
            shutdown: std::mem::transmute::<Exported, MfShutdownFn>(shutdown),
            create_attributes: std::mem::transmute::<Exported, MfCreateAttributesFn>(create_attributes),
            create_media_type: std::mem::transmute::<Exported, MfCreateMediaTypeFn>(create_media_type),
            create_memory_buffer: std::mem::transmute::<Exported, MfCreateMemoryBufferFn>(create_memory_buffer),
            create_sample: std::mem::transmute::<Exported, MfCreateSampleFn>(create_sample),
            create_sink_writer_from_url: std::mem::transmute::<Exported, MfCreateSinkWriterFromUrlFn>(
                create_sink_writer_from_url,
            ),
        })
    }
}

/// wraps the interface a creation call wrote into its out pointer
fn created<T: Interface>(call: &str, make: impl FnOnce(*mut *mut c_void) -> HRESULT) -> Result<T, AudioCaptureError> {
    let mut raw: *mut c_void = ptr::null_mut();
    make(&mut raw).ok().map_err(|error| failed(call, error))?;
    if raw.is_null() {
        return Err(failed(call, E_POINTER.into()));
    }
    Ok(unsafe { T::from_raw(raw) })
}

fn failed(call: &str, error: ::windows::core::Error) -> AudioCaptureError {
    if error.code() == MF_E_TOPO_CODEC_NOT_FOUND {
        AudioCaptureError::EncoderUnavailable(format!(
            "no Media Foundation AAC encoder is registered ({call}: {error})"
        ))
    } else {
        AudioCaptureError::EncoderFailed(format!("{call} failed: {error}"))
    }
}

/// com and media foundation started on the capture thread, dropped last after every interface
struct MediaFoundationSession {
    api: &'static MediaFoundation,
    com_joined: bool,
    started: bool,
}

impl MediaFoundationSession {
    fn start(api: &'static MediaFoundation) -> Result<Self, AudioCaptureError> {
        let joined = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if joined.is_err() && joined != RPC_E_CHANGED_MODE {
            return Err(AudioCaptureError::EncoderFailed(format!("CoInitializeEx failed: {joined:?}")));
        }
        let mut session = Self {
            api,
            com_joined: joined.is_ok(),
            started: false,
        };
        let started = unsafe { (api.startup)(MF_VERSION, MFSTARTUP_LITE) };
        started.ok().map_err(|error| failed("MFStartup", error))?;
        session.started = true;
        Ok(session)
    }
}

impl Drop for MediaFoundationSession {
    fn drop(&mut self) {
        if self.started {
            let stopped = unsafe { (self.api.shutdown)() };
            if stopped.is_err() {
                tracing::debug!(?stopped, "MFShutdown");
            }
        }
        if self.com_joined {
            unsafe { CoUninitialize() };
        }
    }
}

impl AacEncoderSource for NativeAacEncoder {
    fn support(&self) -> Result<(), AudioCaptureError> {
        media_foundation().map(|_| ())
    }

    fn create(&self, path: &Path, settings: &AacSettings) -> Result<Box<dyn AacFileWriter>, AudioCaptureError> {
        let format = MfAacFormat::for_settings(settings)?;
        let api = media_foundation()?;
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(AudioCaptureError::FileFailed(format!("{}: {error}", path.display()))),
        }
        let session = MediaFoundationSession::start(api)?;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(iter::once(0)).collect();

        let attributes: IMFAttributes = created("MFCreateAttributes", |out| {
            unsafe { (api.create_attributes)(out, 1) }
        })?;
        unsafe { attributes.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4) }
            .map_err(|error| failed("MF_TRANSCODE_CONTAINERTYPE", error))?;
        let writer: IMFSinkWriter = created("MFCreateSinkWriterFromURL", |out| {
            unsafe {
                (api.create_sink_writer_from_url)(PCWSTR(wide.as_ptr()), ptr::null_mut(), attributes.as_raw(), out)
            }
        })
        .map_err(|error| match error {
            AudioCaptureError::EncoderFailed(detail) => {
                AudioCaptureError::FileFailed(format!("{}: {detail}", path.display()))
            }
            other => other,
        })?;

        let output = audio_type(api, "AAC output type", |media_type| {
            unsafe {
                media_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
                media_type.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                media_type.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, format.sample_rate)?;
                media_type.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, format.channels)?;
                media_type.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, format.aac_bytes_per_second)?;
                media_type.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)
            }
        })?;
        let stream = unsafe { writer.AddStream(&output) }.map_err(|error| failed("IMFSinkWriter::AddStream", error))?;
        let input = audio_type(api, "PCM input type", |media_type| {
            unsafe {
                media_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
                media_type.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                media_type.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, format.sample_rate)?;
                media_type.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, format.channels)?;
                media_type.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, format.pcm_block_alignment)?;
                media_type.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, format.pcm_bytes_per_second)
            }
        })?;
        unsafe { writer.SetInputMediaType(stream, &input, None::<&IMFAttributes>) }
            .map_err(|error| failed("IMFSinkWriter::SetInputMediaType", error))?;
        unsafe { writer.BeginWriting() }.map_err(|error| failed("IMFSinkWriter::BeginWriting", error))?;

        Ok(Box::new(SinkWriterFile {
            writer: Some(writer),
            stream,
            format,
            clock: SampleClock::new(format.sample_rate),
            resampler: format.resamples().then(|| {
                let take = StreamFormat {
                    sample_rate: format.take_rate,
                    channels: settings.channels,
                };
                Resampler::new(take, format.sample_rate)
            }),
            resampled: Vec::new(),
            pcm: Vec::new(),
            api,
            _session: session,
        }))
    }
}

/// new audio media type with major type set and the rest filled by fill
fn audio_type(
    api: &'static MediaFoundation,
    what: &str,
    fill: impl FnOnce(&IMFMediaType) -> ::windows::core::Result<()>,
) -> Result<IMFMediaType, AudioCaptureError> {
    let media_type: IMFMediaType = created("MFCreateMediaType", |out| {
        unsafe { (api.create_media_type)(out) }
    })?;
    unsafe { media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio) }.map_err(|error| failed(what, error))?;
    fill(&media_type).map_err(|error| failed(what, error))?;
    Ok(media_type)
}

struct SinkWriterFile {
    writer: Option<IMFSinkWriter>,
    stream: u32,
    format: MfAacFormat,
    clock: SampleClock,
    resampler: Option<Resampler>,
    resampled: Vec<f32>,
    pcm: Vec<u8>,
    api: &'static MediaFoundation,
    _session: MediaFoundationSession,
}

impl SinkWriterFile {
    fn write_block(&mut self, interleaved: &[f32]) -> Result<(), AudioCaptureError> {
        let Some(writer) = self.writer.as_ref() else {
            return Err(AudioCaptureError::Stopped);
        };
        let channels = self.format.channels as usize;
        let frames = interleaved.len() / channels;
        if frames == 0 {
            return Ok(());
        }
        self.pcm.clear();
        append_pcm16(&interleaved[..frames * channels], &mut self.pcm);
        let length = u32::try_from(self.pcm.len())
            .map_err(|_| AudioCaptureError::EncoderFailed("a block larger than u32 bytes".to_owned()))?;
        let api = self.api;
        let buffer: IMFMediaBuffer = created("MFCreateMemoryBuffer", |out| {
            unsafe { (api.create_memory_buffer)(length, out) }
        })?;
        let mut data: *mut u8 = ptr::null_mut();
        unsafe { buffer.Lock(&mut data, None, None) }.map_err(|error| failed("IMFMediaBuffer::Lock", error))?;
        if data.is_null() {
            return Err(failed("IMFMediaBuffer::Lock", E_POINTER.into()));
        }
        unsafe { ptr::copy_nonoverlapping(self.pcm.as_ptr(), data, self.pcm.len()) };
        unsafe { buffer.Unlock() }.map_err(|error| failed("IMFMediaBuffer::Unlock", error))?;
        unsafe { buffer.SetCurrentLength(length) }.map_err(|error| failed("IMFMediaBuffer::SetCurrentLength", error))?;

        let sample: IMFSample = created("MFCreateSample", |out| {
            unsafe { (api.create_sample)(out) }
        })?;
        let (time, duration) = self.clock.advance(frames as u64);
        unsafe {
            sample.AddBuffer(&buffer).map_err(|error| failed("IMFSample::AddBuffer", error))?;
            sample.SetSampleTime(time).map_err(|error| failed("IMFSample::SetSampleTime", error))?;
            sample.SetSampleDuration(duration).map_err(|error| failed("IMFSample::SetSampleDuration", error))?;
        }
        unsafe { writer.WriteSample(self.stream, &sample) }.map_err(|error| failed("IMFSinkWriter::WriteSample", error))
    }

    fn close(&mut self) -> Result<(), AudioCaptureError> {
        if self.writer.is_none() {
            return Ok(());
        }
        if self.clock.frames() == 0 {
            let silence = silent_aac_frame(self.format.channels);
            self.write_block(&silence)?;
        }
        let Some(writer) = self.writer.take() else {
            return Ok(());
        };
        unsafe { writer.Finalize() }.map_err(|error| failed("IMFSinkWriter::Finalize", error))
    }
}

impl AacFileWriter for SinkWriterFile {
    fn write(&mut self, interleaved: &[f32]) -> Result<(), AudioCaptureError> {
        let Some(resampler) = self.resampler.as_mut() else {
            return self.write_block(interleaved);
        };
        let mut resampled = std::mem::take(&mut self.resampled);
        resampled.clear();
        resampler.process(interleaved, &mut resampled);
        let written = self.write_block(&resampled);
        self.resampled = resampled;
        written
    }

    fn finish(mut self: Box<Self>) -> Result<(), AudioCaptureError> {
        self.close()
    }
}

impl Drop for SinkWriterFile {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            tracing::warn!(%error, "the m4a file did not close cleanly");
        }
        self.writer = None;
    }
}
