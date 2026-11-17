use std::path::PathBuf;

use objc2::rc::autoreleasepool;

use rise_platform::HostOs;

use super::map_app_file_source::use_app_http_client;
use super::map_view::*;

pub(super) struct NativeMapEngine;

impl MapEngine for NativeMapEngine {
    fn available(&self, host: HostOs) -> Result<GpuBackend, MapUnavailable> {
        Ok(backend_for(host))
    }

    fn open(&self, _host: HostOs, options: MapOptions) -> Result<MapHandle, MapUnavailable> {
        use_app_http_client();
        super::maplibre_map::open(options)
    }

    fn snapshot(&self, _host: HostOs, request: &SnapshotRequest, tile_cache: PathBuf) -> Result<MapFrame, SnapshotError> {
        use_app_http_client();
        super::maplibre_map::snapshot(request, tile_cache)
    }
}

pub(super) fn frame_pool<R>(frame: impl FnOnce() -> R) -> R {
    autoreleasepool(|_| frame())
}
