use std::ffi::{c_char, c_int, c_ulong, c_void};
use std::path::Path;
use std::sync::mpsc::{SyncSender, sync_channel};

use objc2::encode::{Encoding, RefEncode};
use objc2::msg_send;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::{NSArray, NSData, NSError, NSString, NSURL};

use super::audio_metadata::*;

const AV_EXPORT_COMPLETED: isize = 3;
const AV_EXPORT_CANCELLED: isize = 5;

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVAssetExportPresetPassthrough: &'static NSString;
    static AVFileTypeAppleM4A: &'static NSString;
    static AVMetadataIdentifieriTunesMetadataSongName: &'static NSString;
    static AVMetadataIdentifieriTunesMetadataArtist: &'static NSString;
    static AVMetadataIdentifieriTunesMetadataAlbumArtist: &'static NSString;
    static AVMetadataIdentifieriTunesMetadataCoverArt: &'static NSString;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    static kCMMetadataBaseDataType_JPEG: &'static NSString;
}

unsafe extern "C" {
    static _NSConcreteStackBlock: [u8; 0];
}

pub(super) struct NativeAudioMetadata;

impl AudioMetadataWriter for NativeAudioMetadata {
    fn support(&self) -> Result<(), AudioMetadataError> {
        class(c"AVAssetExportSession")?;
        class(c"AVMutableMetadataItem")?;
        Ok(())
    }

    fn export_m4a_passthrough(
        &self,
        source: &Path,
        destination: &Path,
        metadata: &M4aMetadata,
    ) -> Result<(), AudioMetadataError> {
        autoreleasepool(|_| export(source, destination, metadata))
    }
}

fn export(source: &Path, destination: &Path, metadata: &M4aMetadata) -> Result<(), AudioMetadataError> {
    let asset = url_asset(source)?;
    let session_class = class(c"AVAssetExportSession")?;
    let session: Option<Retained<AnyObject>> = unsafe {
        msg_send![session_class, exportSessionWithAsset: &*asset, presetName: AVAssetExportPresetPassthrough]
    };
    let session = session.ok_or(AudioMetadataError::Unavailable)?;
    let output = file_url(destination)?;
    let items = metadata_items(metadata)?;
    unsafe {
        let _: () = msg_send![&*session, setOutputURL: &*output];
        let _: () = msg_send![&*session, setOutputFileType: AVFileTypeAppleM4A];
        if !items.is_empty() {
            let list = NSArray::from_retained_slice(&items);
            let _: () = msg_send![&*session, setMetadata: &*list];
        }
    }
    wait_for_export(&session)?;
    let (status, error): (isize, Option<Retained<NSError>>) =
        unsafe { (msg_send![&*session, status], msg_send![&*session, error]) };
    match status {
        AV_EXPORT_COMPLETED => Ok(()),
        AV_EXPORT_CANCELLED => Err(AudioMetadataError::Cancelled),
        _ => Err(AudioMetadataError::Failed(
            error.map_or_else(|| "export failed".to_owned(), |error| error.localizedDescription().to_string()),
        )),
    }
}

fn wait_for_export(session: &AnyObject) -> Result<(), AudioMetadataError> {
    let (sender, receiver) = sync_channel::<()>(1);
    let block = DoneBlock {
        isa: (&raw const _NSConcreteStackBlock).cast(),
        flags: BLOCK_HAS_SIGNATURE,
        reserved: 0,
        invoke: export_done as *const c_void,
        descriptor: &DONE_DESCRIPTOR,
        sender: Box::into_raw(Box::new(sender)),
    };
    unsafe {
        let _: () = msg_send![session, exportAsynchronouslyWithCompletionHandler: &raw const block];
    }
    receiver
        .recv()
        .map_err(|_| AudioMetadataError::Failed("the export session ended without answering".to_owned()))
}

fn metadata_items(metadata: &M4aMetadata) -> Result<Vec<Retained<AnyObject>>, AudioMetadataError> {
    metadata
        .items()
        .into_iter()
        .map(|item| match item {
            M4aMetadataItem::Text(key, value) => {
                let identifier = text_identifier(key)?;
                let value = NSString::from_str(value);
                metadata_item(identifier, &value, None)
            }
            M4aMetadataItem::CoverJpeg(bytes) => {
                let value = NSData::with_bytes(bytes);
                let (identifier, data_type) =
                    unsafe { (AVMetadataIdentifieriTunesMetadataCoverArt, kCMMetadataBaseDataType_JPEG) };
                metadata_item(identifier, &value, Some(data_type))
            }
        })
        .collect()
}

fn text_identifier(key: [u8; 4]) -> Result<&'static NSString, AudioMetadataError> {
    unsafe {
        match key {
            ITUNES_SONG_NAME => Ok(AVMetadataIdentifieriTunesMetadataSongName),
            ITUNES_ARTIST => Ok(AVMetadataIdentifieriTunesMetadataArtist),
            ITUNES_ALBUM_ARTIST => Ok(AVMetadataIdentifieriTunesMetadataAlbumArtist),
            _ => Err(AudioMetadataError::Failed(format!("no AVFoundation identifier for the tag {key:?}"))),
        }
    }
}

/// metadata item with identifier and value and for the cover the data type
fn metadata_item(
    identifier: &NSString,
    value: &AnyObject,
    data_type: Option<&NSString>,
) -> Result<Retained<AnyObject>, AudioMetadataError> {
    let item_class = class(c"AVMutableMetadataItem")?;
    let item: Option<Retained<AnyObject>> = unsafe { msg_send![item_class, new] };
    let item = item.ok_or_else(|| AudioMetadataError::Failed("AVMutableMetadataItem new returned nil".to_owned()))?;
    unsafe {
        let _: () = msg_send![&*item, setIdentifier: identifier];
        let _: () = msg_send![&*item, setValue: value];
        if let Some(data_type) = data_type {
            let _: () = msg_send![&*item, setDataType: data_type];
        }
    }
    Ok(item)
}

fn class(name: &'static std::ffi::CStr) -> Result<&'static AnyClass, AudioMetadataError> {
    AnyClass::get(name).ok_or(AudioMetadataError::Unavailable)
}

fn file_url(path: &Path) -> Result<Retained<NSURL>, AudioMetadataError> {
    let text = path
        .to_str()
        .ok_or_else(|| AudioMetadataError::Failed(format!("the path {} is not UTF-8", path.display())))?;
    Ok(NSURL::fileURLWithPath(&NSString::from_str(text)))
}

/// asset for the source file url
fn url_asset(path: &Path) -> Result<Retained<AnyObject>, AudioMetadataError> {
    let url = file_url(path)?;
    let asset_class = class(c"AVURLAsset")?;
    let none: Option<&AnyObject> = None;
    let asset: Option<Retained<AnyObject>> = unsafe { msg_send![asset_class, URLAssetWithURL: &*url, options: none] };
    asset.ok_or_else(|| AudioMetadataError::Failed("AVURLAsset returned nil".to_owned()))
}

const BLOCK_HAS_SIGNATURE: c_int = 1 << 30;

#[repr(C)]
struct BlockDescriptor {
    reserved: c_ulong,
    size: c_ulong,
    signature: *const c_char,
}

unsafe impl Sync for BlockDescriptor {}

#[repr(C)]
struct DoneBlock {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: *const c_void,
    descriptor: *const BlockDescriptor,
    sender: *mut SyncSender<()>,
}

unsafe impl RefEncode for DoneBlock {
    const ENCODING_REF: Encoding = Encoding::Block;
}

static DONE_DESCRIPTOR: BlockDescriptor = BlockDescriptor {
    reserved: 0,
    size: std::mem::size_of::<DoneBlock>() as c_ulong,
    signature: c"v8@?0".as_ptr(),
};

unsafe extern "C" fn export_done(block: *const DoneBlock) {
    let sender = unsafe { Box::from_raw((*block).sender) };
    let _ = sender.send(());
}
