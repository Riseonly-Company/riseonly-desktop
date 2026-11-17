use std::path::PathBuf;
use std::sync::OnceLock;

use rise_platform::HostOs;

use super::map_app_file_source::use_app_http_client;
use super::map_view::*;
use super::vulkan_icd::*;

pub(super) struct NativeMapEngine;

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

fn vulkan_backend(host: HostOs) -> Result<GpuBackend, MapUnavailable> {
    static DRIVER_PRESENT: OnceLock<bool> = OnceLock::new();
    let present = *DRIVER_PRESENT.get_or_init(|| {
        let search = driver_search_paths(|name| std::env::var(name).ok());
        has_driver_manifest(&search, |path| path.is_file(), dir_has_json_manifest)
    });
    let backend = backend_for(host);
    if present {
        return Ok(backend);
    }
    tracing::warn!(target: "riseonly", reason = NO_VULKAN_DRIVER, "map view: no Vulkan driver, the map is not drawn");
    Err(MapUnavailable::NoNativeCore { host, backend, reason: NO_VULKAN_DRIVER })
}

pub(super) fn frame_pool<R>(frame: impl FnOnce() -> R) -> R {
    frame()
}
