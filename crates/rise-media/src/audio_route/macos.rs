use std::collections::BTreeSet;
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;

use super::audio_route::*;

pub(super) struct NativeRouteSource;

type OsStatus = i32;
type AudioObjectId = u32;

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioObjectPropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

type PropertyListenerProc = unsafe extern "C" fn(
    object: AudioObjectId,
    address_count: u32,
    addresses: *const AudioObjectPropertyAddress,
    client_data: *mut c_void,
) -> OsStatus;

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectGetPropertyDataSize(
        object: AudioObjectId,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        data_size: *mut u32,
    ) -> OsStatus;
    fn AudioObjectGetPropertyData(
        object: AudioObjectId,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        data_size: *mut u32,
        data: *mut c_void,
    ) -> OsStatus;
    fn AudioObjectAddPropertyListener(
        object: AudioObjectId,
        address: *const AudioObjectPropertyAddress,
        listener: PropertyListenerProc,
        client_data: *mut c_void,
    ) -> OsStatus;
    fn AudioObjectRemovePropertyListener(
        object: AudioObjectId,
        address: *const AudioObjectPropertyAddress,
        listener: PropertyListenerProc,
        client_data: *mut c_void,
    ) -> OsStatus;
}

const fn four_cc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

const SYSTEM_OBJECT: AudioObjectId = 1;
const UNKNOWN_OBJECT: AudioObjectId = 0;
const SCOPE_GLOBAL: u32 = four_cc(b"glob");
const SCOPE_OUTPUT: u32 = four_cc(b"outp");
const ELEMENT_MAIN: u32 = 0;

const DEFAULT_OUTPUT: AudioObjectPropertyAddress = AudioObjectPropertyAddress {
    selector: four_cc(b"dOut"),
    scope: SCOPE_GLOBAL,
    element: ELEMENT_MAIN,
};
const DEVICES: AudioObjectPropertyAddress = AudioObjectPropertyAddress {
    selector: four_cc(b"dev#"),
    scope: SCOPE_GLOBAL,
    element: ELEMENT_MAIN,
};
const OUTPUT_STREAMS: AudioObjectPropertyAddress = AudioObjectPropertyAddress {
    selector: four_cc(b"stm#"),
    scope: SCOPE_OUTPUT,
    element: ELEMENT_MAIN,
};

const WATCHED: [AudioObjectPropertyAddress; 2] = [DEFAULT_OUTPUT, DEVICES];

unsafe extern "C" fn on_property_changed(
    _object: AudioObjectId,
    _address_count: u32,
    _addresses: *const AudioObjectPropertyAddress,
    client_data: *mut c_void,
) -> OsStatus {
    let signal = unsafe { &*(client_data as *const RouteSignal) };
    signal();
    0
}

struct CoreAudioRoutes {
    signal: *mut RouteSignal,
    registered: Vec<AudioObjectPropertyAddress>,
}

impl AudioRouteSource for NativeRouteSource {
    fn open(&self, signal: RouteSignal) -> Result<Box<dyn AudioRouteBackend>, AudioRouteError> {
        let mut routes = CoreAudioRoutes {
            signal: Box::into_raw(Box::new(signal)),
            registered: Vec::with_capacity(WATCHED.len()),
        };
        for address in WATCHED {
            let status = unsafe {
                AudioObjectAddPropertyListener(
                    SYSTEM_OBJECT,
                    &address,
                    on_property_changed,
                    routes.signal.cast::<c_void>(),
                )
            };
            if status != 0 {
                return Err(failed("AudioObjectAddPropertyListener", status));
            }
            routes.registered.push(address);
        }
        Ok(Box::new(routes))
    }
}

impl AudioRouteBackend for CoreAudioRoutes {
    fn snapshot(&mut self) -> Result<AudioOutputSnapshot, AudioRouteError> {
        let default_output = read_default_output()?;
        let outputs = read_devices()?
            .into_iter()
            .filter(|device| has_output_streams(*device))
            .map(|device| device.to_string())
            .collect::<BTreeSet<String>>();
        Ok(AudioOutputSnapshot {
            default_output: default_output.map(|device| device.to_string()),
            outputs,
        })
    }
}

impl Drop for CoreAudioRoutes {
    fn drop(&mut self) {
        for address in self.registered.drain(..) {
            let status = unsafe {
                AudioObjectRemovePropertyListener(
                    SYSTEM_OBJECT,
                    &address,
                    on_property_changed,
                    self.signal.cast::<c_void>(),
                )
            };
            if status != 0 {
                tracing::warn!(status, "AudioObjectRemovePropertyListener failed");
            }
        }
        drop(unsafe { Box::from_raw(self.signal) });
    }
}

fn failed(call: &'static str, status: OsStatus) -> AudioRouteError {
    AudioRouteError::Failed {
        call,
        detail: format!("OSStatus {status}"),
    }
}

fn read_default_output() -> Result<Option<AudioObjectId>, AudioRouteError> {
    let mut device: AudioObjectId = UNKNOWN_OBJECT;
    let mut size = size_of::<AudioObjectId>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            SYSTEM_OBJECT,
            &DEFAULT_OUTPUT,
            0,
            ptr::null(),
            &mut size,
            (&mut device as *mut AudioObjectId).cast::<c_void>(),
        )
    };
    if status != 0 {
        return Err(failed("AudioObjectGetPropertyData(DefaultOutputDevice)", status));
    }
    Ok((device != UNKNOWN_OBJECT).then_some(device))
}

fn read_devices() -> Result<Vec<AudioObjectId>, AudioRouteError> {
    let mut size: u32 = 0;
    let status = unsafe {
        AudioObjectGetPropertyDataSize(SYSTEM_OBJECT, &DEVICES, 0, ptr::null(), &mut size)
    };
    if status != 0 {
        return Err(failed("AudioObjectGetPropertyDataSize(Devices)", status));
    }
    let count = size as usize / size_of::<AudioObjectId>();
    let mut devices: Vec<AudioObjectId> = vec![UNKNOWN_OBJECT; count];
    let mut filled = (count * size_of::<AudioObjectId>()) as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            SYSTEM_OBJECT,
            &DEVICES,
            0,
            ptr::null(),
            &mut filled,
            devices.as_mut_ptr().cast::<c_void>(),
        )
    };
    if status != 0 {
        return Err(failed("AudioObjectGetPropertyData(Devices)", status));
    }
    devices.truncate(filled as usize / size_of::<AudioObjectId>());
    Ok(devices)
}

fn has_output_streams(device: AudioObjectId) -> bool {
    let mut size: u32 = 0;
    let status = unsafe {
        AudioObjectGetPropertyDataSize(device, &OUTPUT_STREAMS, 0, ptr::null(), &mut size)
    };
    status == 0 && size > 0
}
