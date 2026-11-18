use std::ffi::{c_char, c_void};
use std::path::PathBuf;
use std::ptr;
use std::sync::OnceLock;

use rise_platform::HostOs;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use windows::core::{s, w};

use super::map_app_file_source::use_app_http_client;
use super::map_view::*;

pub(super) struct NativeMapEngine;

pub(super) const NO_VULKAN_DEVICE: &str =
    "no Vulkan graphics card answers on this Windows (no vulkan-1.dll or no device behind it)";

impl MapEngine for NativeMapEngine {
    fn available(&self, host: HostOs) -> Result<GpuBackend, MapUnavailable> {
        vulkan_backend(host)
    }

    fn open(&self, host: HostOs, options: MapOptions) -> Result<MapHandle, MapUnavailable> {
        vulkan_backend(host)?;
        use_app_http_client();
        super::maplibre_map::open(options)
    }

    fn snapshot(&self, host: HostOs, request: &SnapshotRequest, tile_cache: PathBuf) -> Result<MapFrame, SnapshotError> {
        vulkan_backend(host)?;
        use_app_http_client();
        super::maplibre_map::snapshot(request, tile_cache)
    }
}

#[repr(C)]
struct InstanceCreateInfo {
    structure_type: u32,
    next: *const c_void,
    flags: u32,
    application_info: *const c_void,
    enabled_layer_count: u32,
    enabled_layer_names: *const *const c_char,
    enabled_extension_count: u32,
    enabled_extension_names: *const *const c_char,
}

const INSTANCE_CREATE_INFO: u32 = 1;
const VK_SUCCESS: i32 = 0;

type CreateInstance = unsafe extern "system" fn(*const InstanceCreateInfo, *const c_void, *mut *mut c_void) -> i32;
type EnumeratePhysicalDevices = unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut c_void) -> i32;
type DestroyInstance = unsafe extern "system" fn(*mut c_void, *const c_void);
type GetInstanceProcAddr = unsafe extern "system" fn(*mut c_void, *const c_char) -> Option<unsafe extern "system" fn()>;

fn vulkan_backend(host: HostOs) -> Result<GpuBackend, MapUnavailable> {
    static DEVICE_PRESENT: OnceLock<bool> = OnceLock::new();
    let present = *DEVICE_PRESENT.get_or_init(|| {
        let present = vulkan_device_present();
        if !present {
            tracing::warn!(target: "riseonly", reason = NO_VULKAN_DEVICE, "map view: no Vulkan device, the map is not drawn");
        }
        present
    });
    let backend = backend_for(host);
    if present {
        return Ok(backend);
    }
    Err(MapUnavailable::NoNativeCore { host, backend, reason: NO_VULKAN_DEVICE })
}

fn vulkan_device_present() -> bool {
    let Ok(loader) = (unsafe { LoadLibraryExW(w!("vulkan-1.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }) else {
        return false;
    };
    let Some(get_proc) = (unsafe { GetProcAddress(loader, s!("vkGetInstanceProcAddr")) }) else {
        return false;
    };
    let get_proc: GetInstanceProcAddr = unsafe { std::mem::transmute(get_proc) };
    let lookup = |name: &std::ffi::CStr| unsafe { get_proc(ptr::null_mut(), name.as_ptr()) };
    let Some(create) = lookup(c"vkCreateInstance") else {
        return false;
    };
    let create: CreateInstance = unsafe { std::mem::transmute(create) };
    let info = InstanceCreateInfo {
        structure_type: INSTANCE_CREATE_INFO,
        next: ptr::null(),
        flags: 0,
        application_info: ptr::null(),
        enabled_layer_count: 0,
        enabled_layer_names: ptr::null(),
        enabled_extension_count: 0,
        enabled_extension_names: ptr::null(),
    };
    let mut instance = ptr::null_mut();
    if unsafe { create(&info, ptr::null(), &mut instance) } != VK_SUCCESS || instance.is_null() {
        return false;
    }
    let instance_lookup = |name: &std::ffi::CStr| unsafe { get_proc(instance, name.as_ptr()) };
    let mut count = 0_u32;
    let counted = instance_lookup(c"vkEnumeratePhysicalDevices").is_some_and(|enumerate| {
        let enumerate: EnumeratePhysicalDevices = unsafe { std::mem::transmute(enumerate) };
        (unsafe { enumerate(instance, &mut count, ptr::null_mut()) }) == VK_SUCCESS
    });
    if let Some(destroy) = instance_lookup(c"vkDestroyInstance") {
        let destroy: DestroyInstance = unsafe { std::mem::transmute(destroy) };
        unsafe { destroy(instance, ptr::null()) };
    }
    counted && count > 0
}

pub(super) fn frame_pool<R>(frame: impl FnOnce() -> R) -> R {
    frame()
}
