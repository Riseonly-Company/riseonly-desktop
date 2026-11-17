use std::path::PathBuf;

use rise_platform::HostOs;

use super::map_view::*;

pub(super) struct NativeMapEngine;

pub(super) const NO_WINDOWS_CORE: &str = "maplibre-native ships no prebuilt Windows core and maplibre_native 0.10 does not build on Windows";

fn no_core(host: HostOs) -> MapUnavailable {
    MapUnavailable::NoNativeCore {
        host,
        backend: backend_for(host),
        reason: NO_WINDOWS_CORE,
    }
}

impl MapEngine for NativeMapEngine {
    fn available(&self, host: HostOs) -> Result<GpuBackend, MapUnavailable> {
        Err(no_core(host))
    }

    fn open(&self, host: HostOs, _options: MapOptions) -> Result<MapHandle, MapUnavailable> {
        Err(no_core(host))
    }

    fn snapshot(&self, host: HostOs, _request: &SnapshotRequest, _tile_cache: PathBuf) -> Result<MapFrame, SnapshotError> {
        Err(SnapshotError::Unavailable(no_core(host)))
    }
}
