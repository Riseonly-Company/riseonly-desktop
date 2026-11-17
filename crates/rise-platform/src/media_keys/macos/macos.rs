use std::sync::{Arc, OnceLock};
use std::time::Duration;

use objc2::msg_send;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2_foundation::{NSNumber, NSString};
use parking_lot::Mutex;

use crate::host_os::HostOs;
use crate::media_keys::*;

#[link(name = "MediaPlayer", kind = "framework")]
unsafe extern "C" {}

const REMOTE_SUCCESS: isize = 0;

const REMOTE_COMMAND_FAILED: isize = 200;

static MAC_HANDLER: Mutex<Option<CommandHandler>> = Mutex::new(None);

pub(crate) struct Transport {
    info_center: Retained<AnyObject>,
    command_center: Retained<AnyObject>,
    target: Retained<AnyObject>,
}

const MAC_COMMANDS: [(&std::ffi::CStr, &std::ffi::CStr); 6] = [
    (c"playCommand", c"riseHandlePlay:"),
    (c"pauseCommand", c"riseHandlePause:"),
    (c"togglePlayPauseCommand", c"riseHandleToggle:"),
    (c"nextTrackCommand", c"riseHandleNext:"),
    (c"previousTrackCommand", c"riseHandlePrevious:"),
    (c"changePlaybackPositionCommand", c"riseHandleSeek:"),
];

const MAC_DISABLED_COMMANDS: [&std::ffi::CStr; 4] = [
    c"skipForwardCommand",
    c"skipBackwardCommand",
    c"seekForwardCommand",
    c"seekBackwardCommand",
];

unsafe fn command_of(center: &AnyObject, getter: &std::ffi::CStr) -> Option<Retained<AnyObject>> {
    let getter = Sel::register(getter);
    unsafe { msg_send![center, performSelector: getter] }
}

impl Transport {
    pub(crate) fn install(handler: CommandHandler) -> Result<Self, MediaKeysError> {
        if bundle_identifier().is_none() {
            return Err(MediaKeysError::NotBundled);
        }

        let info_class =
            AnyClass::get(c"MPNowPlayingInfoCenter").ok_or(MediaKeysError::Unavailable)?;
        let command_class =
            AnyClass::get(c"MPRemoteCommandCenter").ok_or(MediaKeysError::Unavailable)?;
        let target_class = command_target_class().ok_or(MediaKeysError::Unavailable)?;

        let (info_center, command_center, target) = unsafe {
            let info_center: Option<Retained<AnyObject>> = msg_send![info_class, defaultCenter];
            let command_center: Option<Retained<AnyObject>> =
                msg_send![command_class, sharedCommandCenter];
            let target: Option<Retained<AnyObject>> = msg_send![target_class, new];
            (info_center, command_center, target)
        };

        let info_center = info_center.ok_or(MediaKeysError::Unavailable)?;
        let command_center = command_center.ok_or(MediaKeysError::Unavailable)?;
        let target = target.ok_or(MediaKeysError::Unavailable)?;

        *MAC_HANDLER.lock() = Some(handler);

        unsafe {
            let center = &*command_center;

            for (getter, action) in MAC_COMMANDS {
                wire(command_of(center, getter), &target, action);
            }

            for getter in MAC_DISABLED_COMMANDS {
                if let Some(command) = command_of(center, getter) {
                    let _: () = msg_send![&*command, setEnabled: Bool::NO];
                }
            }
        }

        Ok(Self {
            info_center,
            command_center,
            target,
        })
    }

    pub(crate) fn publish(&self, track: &NowPlaying, state: PlaybackState) {
        let Some(dictionary_class) = AnyClass::get(c"NSMutableDictionary") else {
            return;
        };

        let info: Option<Retained<AnyObject>> = unsafe { msg_send![dictionary_class, dictionary] };
        let Some(info) = info else {
            return;
        };

        unsafe {
            set_entry(&info, "title", &*NSString::from_str(&track.title));
            set_entry(&info, "artist", &*NSString::from_str(&track.artist));
            set_entry(&info, "albumTitle", &*NSString::from_str(&track.album));
            if track.duration > Duration::ZERO {
                set_entry(
                    &info,
                    "playbackDuration",
                    &*NSNumber::new_f64(track.duration.as_secs_f64()),
                );
            }
            set_entry(
                &info,
                "MPNowPlayingInfoPropertyElapsedPlaybackTime",
                &*NSNumber::new_f64(track.elapsed.as_secs_f64()),
            );
            set_entry(
                &info,
                "MPNowPlayingInfoPropertyPlaybackRate",
                &*NSNumber::new_f64(state.effective_rate(track.rate)),
            );
            set_entry(
                &info,
                "MPNowPlayingInfoPropertyMediaType",
                &*NSNumber::new_isize(1),
            );

            if let Some(artwork) = track.artwork.as_ref().and_then(artwork_object) {
                set_entry(&info, "artwork", &*artwork);
            }
        }

        let raw = state
            .raw(HostOs::MacOs)
            .integer()
            .unwrap_or_default()
            .max(0) as usize;

        unsafe {
            let _: () = msg_send![&*self.info_center, setNowPlayingInfo: &*info];
            let _: () = msg_send![&*self.info_center, setPlaybackState: raw];
        }
    }

    pub(crate) fn clear(&self) {
        let stopped = PlaybackState::Stopped
            .raw(HostOs::MacOs)
            .integer()
            .unwrap_or_default()
            .max(0) as usize;

        unsafe {
            let nil: *mut AnyObject = std::ptr::null_mut();
            let _: () = msg_send![&*self.info_center, setNowPlayingInfo: nil];
            let _: () = msg_send![&*self.info_center, setPlaybackState: stopped];
        }
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        self.clear();

        unsafe {
            let center = &*self.command_center;

            for (getter, _) in MAC_COMMANDS {
                if let Some(command) = command_of(center, getter) {
                    let _: () = msg_send![&*command, removeTarget: &*self.target];
                }
            }
        }

        *MAC_HANDLER.lock() = None;
    }
}

unsafe fn wire(command: Option<Retained<AnyObject>>, target: &AnyObject, action: &std::ffi::CStr) {
    let Some(command) = command else {
        return;
    };
    let action = Sel::register(action);
    unsafe {
        let _: () = msg_send![&*command, setEnabled: Bool::YES];
        let _: () = msg_send![&*command, addTarget: target, action: action];
    }
}

unsafe fn set_entry<V: objc2::Message>(dictionary: &AnyObject, key: &str, value: &V) {
    let key = NSString::from_str(key);
    unsafe {
        let _: () = msg_send![dictionary, setObject: value, forKey: &*key];
    }
}

fn artwork_object(artwork: &Artwork) -> Option<Retained<AnyObject>> {
    let Artwork::File(path) = artwork else {
        return None;
    };

    let image_class = AnyClass::get(c"NSImage")?;
    let artwork_class = AnyClass::get(c"MPMediaItemArtwork")?;
    if !artwork_class.responds_to(Sel::register(c"initWithImage:")) {
        return None;
    }

    let path = NSString::from_str(path.to_str()?);

    unsafe {
        let allocated: Allocated<AnyObject> = msg_send![image_class, alloc];
        let image: Option<Retained<AnyObject>> =
            msg_send![allocated, initWithContentsOfFile: &*path];
        let image = image?;

        let allocated: Allocated<AnyObject> = msg_send![artwork_class, alloc];
        msg_send![allocated, initWithImage: &*image]
    }
}

fn bundle_identifier() -> Option<Retained<AnyObject>> {
    let bundle_class = AnyClass::get(c"NSBundle")?;

    unsafe {
        let bundle: Option<Retained<AnyObject>> = msg_send![bundle_class, mainBundle];
        let bundle = bundle?;
        msg_send![&*bundle, bundleIdentifier]
    }
}

pub(crate) fn command_target_class() -> Option<&'static AnyClass> {
    static CLASS: OnceLock<Option<&'static AnyClass>> = OnceLock::new();

    *CLASS.get_or_init(|| {
        let superclass = AnyClass::get(c"NSObject")?;
        let mut builder = ClassBuilder::new(c"RiseMediaCommandTarget", superclass)?;

        unsafe {
            builder.add_method(
                Sel::register(c"riseHandlePlay:"),
                handle_play as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> isize,
            );
            builder.add_method(
                Sel::register(c"riseHandlePause:"),
                handle_pause as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> isize,
            );
            builder.add_method(
                Sel::register(c"riseHandleToggle:"),
                handle_toggle as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> isize,
            );
            builder.add_method(
                Sel::register(c"riseHandleNext:"),
                handle_next as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> isize,
            );
            builder.add_method(
                Sel::register(c"riseHandlePrevious:"),
                handle_previous as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> isize,
            );
            builder.add_method(
                Sel::register(c"riseHandleSeek:"),
                handle_seek as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> isize,
            );
        }

        Some(builder.register())
    })
}

fn dispatch(event: MediaCommandEvent) -> isize {
    let handler = MAC_HANDLER.lock().clone();
    let Some(handler) = handler else {
        return REMOTE_COMMAND_FAILED;
    };

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || handler(event)));

    match outcome {
        Ok(CommandOutcome::Handled) => REMOTE_SUCCESS,
        Ok(CommandOutcome::Rejected) | Err(_) => REMOTE_COMMAND_FAILED,
    }
}

extern "C" fn handle_play(_this: *mut AnyObject, _cmd: Sel, _event: *mut AnyObject) -> isize {
    dispatch(MediaCommandEvent::simple(MediaCommand::Play))
}

extern "C" fn handle_pause(_this: *mut AnyObject, _cmd: Sel, _event: *mut AnyObject) -> isize {
    dispatch(MediaCommandEvent::simple(MediaCommand::Pause))
}

extern "C" fn handle_toggle(_this: *mut AnyObject, _cmd: Sel, _event: *mut AnyObject) -> isize {
    dispatch(MediaCommandEvent::simple(MediaCommand::Toggle))
}

extern "C" fn handle_next(_this: *mut AnyObject, _cmd: Sel, _event: *mut AnyObject) -> isize {
    dispatch(MediaCommandEvent::simple(MediaCommand::Next))
}

extern "C" fn handle_previous(_this: *mut AnyObject, _cmd: Sel, _event: *mut AnyObject) -> isize {
    dispatch(MediaCommandEvent::simple(MediaCommand::Previous))
}

extern "C" fn handle_seek(_this: *mut AnyObject, _cmd: Sel, event: *mut AnyObject) -> isize {
    let position: f64 = unsafe {
        let Some(event) = event.as_ref() else {
            return REMOTE_COMMAND_FAILED;
        };
        msg_send![event, positionTime]
    };

    if !position.is_finite() || position < 0.0 {
        return REMOTE_COMMAND_FAILED;
    }

    dispatch(MediaCommandEvent::seek(Duration::from_secs_f64(position)))
}

pub(crate) async fn serve(
    _share: Arc<MediaShare>,
    _identity: MediaIdentity,
) -> Result<(), MediaKeysError> {
    Ok(())
}
