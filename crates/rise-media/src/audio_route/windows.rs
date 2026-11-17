use std::collections::BTreeSet;

use ::windows::Win32::Foundation::PROPERTYKEY;
use ::windows::Win32::Media::Audio::{
    DEVICE_STATE, DEVICE_STATE_ACTIVE, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator,
    IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator, eConsole, eRender,
};
use ::windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize,
};
use ::windows::core::{PCWSTR, implement};

use super::audio_route::*;

pub(super) struct NativeRouteSource;

#[implement(IMMNotificationClient)]
struct EndpointEvents {
    signal: RouteSignal,
}

impl IMMNotificationClient_Impl for EndpointEvents_Impl {
    fn OnDeviceStateChanged(&self, _device_id: &PCWSTR, _new_state: DEVICE_STATE) -> ::windows::core::Result<()> {
        (self.signal)();
        Ok(())
    }

    fn OnDeviceAdded(&self, _device_id: &PCWSTR) -> ::windows::core::Result<()> {
        (self.signal)();
        Ok(())
    }

    fn OnDeviceRemoved(&self, _device_id: &PCWSTR) -> ::windows::core::Result<()> {
        (self.signal)();
        Ok(())
    }

    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _default_device_id: &PCWSTR) -> ::windows::core::Result<()> {
        if flow == eRender && role == eConsole {
            (self.signal)();
        }
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _device_id: &PCWSTR, _key: &PROPERTYKEY) -> ::windows::core::Result<()> {
        Ok(())
    }
}

struct ComApartment;

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct MmDeviceRoutes {
    client: IMMNotificationClient,
    enumerator: IMMDeviceEnumerator,
    _apartment: ComApartment,
}

impl AudioRouteSource for NativeRouteSource {
    fn open(&self, signal: RouteSignal) -> Result<Box<dyn AudioRouteBackend>, AudioRouteError> {
        let joined = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if joined.is_err() {
            return Err(failed("CoInitializeEx", &::windows::core::Error::from(joined)));
        }
        let apartment = ComApartment;
        let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|error| failed("CoCreateInstance(MMDeviceEnumerator)", &error))?;
        let client: IMMNotificationClient = EndpointEvents { signal }.into();
        unsafe { enumerator.RegisterEndpointNotificationCallback(&client) }
            .map_err(|error| failed("RegisterEndpointNotificationCallback", &error))?;
        Ok(Box::new(MmDeviceRoutes {
            client,
            enumerator,
            _apartment: apartment,
        }))
    }
}

impl AudioRouteBackend for MmDeviceRoutes {
    fn snapshot(&mut self) -> Result<AudioOutputSnapshot, AudioRouteError> {
        let default_output = unsafe { self.enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
            .ok()
            .and_then(|device| endpoint_id(&device));
        let collection = unsafe { self.enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
            .map_err(|error| failed("EnumAudioEndpoints", &error))?;
        let count = unsafe { collection.GetCount() }.map_err(|error| failed("IMMDeviceCollection::GetCount", &error))?;
        let mut outputs = BTreeSet::new();
        for index in 0..count {
            let Ok(device) = (unsafe { collection.Item(index) }) else {
                continue;
            };
            if let Some(id) = endpoint_id(&device) {
                outputs.insert(id);
            }
        }
        Ok(AudioOutputSnapshot { default_output, outputs })
    }
}

impl Drop for MmDeviceRoutes {
    fn drop(&mut self) {
        if let Err(error) = unsafe { self.enumerator.UnregisterEndpointNotificationCallback(&self.client) } {
            tracing::warn!(%error, "UnregisterEndpointNotificationCallback failed");
        }
    }
}

fn failed(call: &'static str, error: &::windows::core::Error) -> AudioRouteError {
    AudioRouteError::Failed {
        call,
        detail: error.to_string(),
    }
}

/// endpoint id string, the buffer GetId hands out is the caller to free
fn endpoint_id(device: &IMMDevice) -> Option<String> {
    let raw = unsafe { device.GetId() }.ok()?;
    let id = unsafe { raw.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(raw.0.cast_const().cast())) };
    id
}
