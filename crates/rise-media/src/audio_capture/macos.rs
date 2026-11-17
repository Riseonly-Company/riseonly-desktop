use std::ffi::c_void;
use std::mem::size_of;
use std::path::Path;
use std::ptr;

use core_foundation::base::TCFType;
use core_foundation::url::{CFURL, CFURLRef};

use super::audio_capture::*;

pub(super) struct NativeAacEncoder;

type OsStatus = i32;
type ExtAudioFileRef = *mut c_void;
type AudioConverterRef = *mut c_void;

#[repr(C)]
struct AudioStreamBasicDescription {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

#[repr(C)]
struct AudioBuffer {
    number_channels: u32,
    data_byte_size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    number_buffers: u32,
    buffers: [AudioBuffer; 1],
}

const fn four_char_code(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

const AUDIO_FILE_M4A_TYPE: u32 = four_char_code(b"m4af");
const AUDIO_FORMAT_MPEG4_AAC: u32 = four_char_code(b"aac ");
const AUDIO_FORMAT_LINEAR_PCM: u32 = four_char_code(b"lpcm");
const AUDIO_FORMAT_FLAGS_FLOAT_PACKED: u32 = (1 << 0) | (1 << 3);
const AUDIO_FILE_FLAGS_ERASE_FILE: u32 = 1;
const EXT_AUDIO_FILE_PROPERTY_CLIENT_DATA_FORMAT: u32 = four_char_code(b"cfmt");
const EXT_AUDIO_FILE_PROPERTY_AUDIO_CONVERTER: u32 = four_char_code(b"acnv");
const EXT_AUDIO_FILE_PROPERTY_CONVERTER_CONFIG: u32 = four_char_code(b"accf");
const AUDIO_CONVERTER_ENCODE_BIT_RATE: u32 = four_char_code(b"brat");
const AUDIO_CONVERTER_CODEC_QUALITY: u32 = four_char_code(b"cdqu");
const AAC_FRAMES_PER_PACKET: u32 = 1024;
const FLOAT_BYTES: u32 = size_of::<f32>() as u32;

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn ExtAudioFileCreateWithURL(
        url: CFURLRef,
        file_type: u32,
        stream_desc: *const AudioStreamBasicDescription,
        channel_layout: *const c_void,
        flags: u32,
        out_file: *mut ExtAudioFileRef,
    ) -> OsStatus;
    fn ExtAudioFileSetProperty(
        file: ExtAudioFileRef,
        property: u32,
        size: u32,
        data: *const c_void,
    ) -> OsStatus;
    fn ExtAudioFileGetProperty(
        file: ExtAudioFileRef,
        property: u32,
        size: *mut u32,
        data: *mut c_void,
    ) -> OsStatus;
    fn ExtAudioFileWrite(file: ExtAudioFileRef, frames: u32, data: *const AudioBufferList) -> OsStatus;
    fn ExtAudioFileDispose(file: ExtAudioFileRef) -> OsStatus;
    fn AudioConverterSetProperty(
        converter: AudioConverterRef,
        property: u32,
        size: u32,
        data: *const c_void,
    ) -> OsStatus;
}

fn describe(status: OsStatus) -> String {
    let bytes = status.to_be_bytes();
    if bytes.iter().all(|byte| byte.is_ascii_graphic() || *byte == b' ') {
        format!("'{}' ({status})", String::from_utf8_lossy(&bytes))
    } else {
        status.to_string()
    }
}

fn check(status: OsStatus, call: &str) -> Result<(), AudioCaptureError> {
    if status == 0 {
        Ok(())
    } else {
        Err(AudioCaptureError::EncoderFailed(format!("{call} returned {}", describe(status))))
    }
}

impl AacEncoderSource for NativeAacEncoder {
    fn support(&self) -> Result<(), AudioCaptureError> {
        Ok(())
    }

    fn create(&self, path: &Path, settings: &AacSettings) -> Result<Box<dyn AacFileWriter>, AudioCaptureError> {
        let url = CFURL::from_path(path, false)
            .ok_or_else(|| AudioCaptureError::FileFailed(format!("{} is not a file URL", path.display())))?;
        let channels = u32::from(settings.channels);
        let file_format = AudioStreamBasicDescription {
            sample_rate: f64::from(settings.sample_rate),
            format_id: AUDIO_FORMAT_MPEG4_AAC,
            format_flags: 0,
            bytes_per_packet: 0,
            frames_per_packet: AAC_FRAMES_PER_PACKET,
            bytes_per_frame: 0,
            channels_per_frame: channels,
            bits_per_channel: 0,
            reserved: 0,
        };
        let mut file: ExtAudioFileRef = ptr::null_mut();
        let status = unsafe {
            ExtAudioFileCreateWithURL(
                url.as_concrete_TypeRef(),
                AUDIO_FILE_M4A_TYPE,
                &file_format,
                ptr::null(),
                AUDIO_FILE_FLAGS_ERASE_FILE,
                &mut file,
            )
        };
        check(status, "ExtAudioFileCreateWithURL")?;
        if file.is_null() {
            return Err(AudioCaptureError::FileFailed("ExtAudioFileCreateWithURL gave no file".to_owned()));
        }
        let writer = ExtAudioFileWriter { file, channels };
        let client_format = AudioStreamBasicDescription {
            sample_rate: f64::from(settings.sample_rate),
            format_id: AUDIO_FORMAT_LINEAR_PCM,
            format_flags: AUDIO_FORMAT_FLAGS_FLOAT_PACKED,
            bytes_per_packet: FLOAT_BYTES * channels,
            frames_per_packet: 1,
            bytes_per_frame: FLOAT_BYTES * channels,
            channels_per_frame: channels,
            bits_per_channel: FLOAT_BYTES * 8,
            reserved: 0,
        };
        let status = unsafe {
            ExtAudioFileSetProperty(
                writer.file,
                EXT_AUDIO_FILE_PROPERTY_CLIENT_DATA_FORMAT,
                size_of::<AudioStreamBasicDescription>() as u32,
                (&client_format as *const AudioStreamBasicDescription).cast(),
            )
        };
        check(status, "kExtAudioFileProperty_ClientDataFormat")?;
        writer.configure_encoder(settings)?;
        Ok(Box::new(writer))
    }
}

struct ExtAudioFileWriter {
    file: ExtAudioFileRef,
    channels: u32,
}

impl ExtAudioFileWriter {
    fn configure_encoder(&self, settings: &AacSettings) -> Result<(), AudioCaptureError> {
        let mut converter: AudioConverterRef = ptr::null_mut();
        let mut size = size_of::<AudioConverterRef>() as u32;
        let status = unsafe {
            ExtAudioFileGetProperty(
                self.file,
                EXT_AUDIO_FILE_PROPERTY_AUDIO_CONVERTER,
                &mut size,
                (&mut converter as *mut AudioConverterRef).cast(),
            )
        };
        check(status, "kExtAudioFileProperty_AudioConverter")?;
        if converter.is_null() {
            return Err(AudioCaptureError::EncoderFailed("ExtAudioFile has no AAC converter".to_owned()));
        }
        let quality = settings.quality.raw();
        let status = unsafe {
            AudioConverterSetProperty(
                converter,
                AUDIO_CONVERTER_CODEC_QUALITY,
                size_of::<u32>() as u32,
                (&quality as *const u32).cast(),
            )
        };
        check(status, "kAudioConverterCodecQuality")?;
        if let Some(bit_rate) = settings.bit_rate {
            let status = unsafe {
                AudioConverterSetProperty(
                    converter,
                    AUDIO_CONVERTER_ENCODE_BIT_RATE,
                    size_of::<u32>() as u32,
                    (&bit_rate as *const u32).cast(),
                )
            };
            check(status, "kAudioConverterEncodeBitRate")?;
        }
        let config: *const c_void = ptr::null();
        let status = unsafe {
            ExtAudioFileSetProperty(
                self.file,
                EXT_AUDIO_FILE_PROPERTY_CONVERTER_CONFIG,
                size_of::<*const c_void>() as u32,
                (&config as *const *const c_void).cast(),
            )
        };
        check(status, "kExtAudioFileProperty_ConverterConfig")
    }

    fn dispose(&mut self) -> Result<(), AudioCaptureError> {
        let file = std::mem::replace(&mut self.file, ptr::null_mut());
        if file.is_null() {
            return Ok(());
        }
        let status = unsafe { ExtAudioFileDispose(file) };
        check(status, "ExtAudioFileDispose")
    }
}

impl AacFileWriter for ExtAudioFileWriter {
    fn write(&mut self, interleaved: &[f32]) -> Result<(), AudioCaptureError> {
        let channels = self.channels as usize;
        let frames = interleaved.len() / channels;
        if frames == 0 {
            return Ok(());
        }
        let samples = frames * channels;
        let frame_count = u32::try_from(frames)
            .map_err(|_| AudioCaptureError::EncoderFailed("a block larger than u32 frames".to_owned()))?;
        let byte_size = u32::try_from(samples * size_of::<f32>())
            .map_err(|_| AudioCaptureError::EncoderFailed("a block larger than u32 bytes".to_owned()))?;
        let list = AudioBufferList {
            number_buffers: 1,
            buffers: [AudioBuffer {
                number_channels: self.channels,
                data_byte_size: byte_size,
                data: interleaved.as_ptr() as *mut c_void,
            }],
        };
        let status = unsafe { ExtAudioFileWrite(self.file, frame_count, &list) };
        check(status, "ExtAudioFileWrite")
    }

    fn finish(mut self: Box<Self>) -> Result<(), AudioCaptureError> {
        self.dispose()
    }
}

impl Drop for ExtAudioFileWriter {
    fn drop(&mut self) {
        if let Err(error) = self.dispose() {
            tracing::warn!(%error, "the m4a file did not close cleanly");
        }
    }
}
