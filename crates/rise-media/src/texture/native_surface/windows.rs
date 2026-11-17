use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::{AnyElement, IntoElement, OwnedSurfaceFrame, SharedSurfaceHandle, Styled};
use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::HostOs;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX,
    D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Device,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE, IDXGIKeyedMutex, IDXGIResource1,
};
use windows::core::{Interface, PCWSTR};

use super::native_surface::*;
use crate::video::d3d11_frame::{
    D3d11FrameRef, KEYED_MUTEX_KEY, KEYED_MUTEX_WAIT_MS, SlotChoice, choose_slot, shared_copy_size,
    surplus_free_slots,
};

pub(super) struct NativeSurfaceBinding;

impl SurfaceBinding for NativeSurfaceBinding {
    unsafe fn retain(
        &self,
        _host: HostOs,
        address: *mut c_void,
    ) -> Result<RetainedSurface, SurfaceError> {
        if address.is_null() {
            return Err(SurfaceError::Null);
        }
        let frame = unsafe { *address.cast::<D3d11FrameRef>() };
        let source = unsafe { ID3D11Texture2D::from_raw_borrowed(&frame.texture) }
            .ok_or(SurfaceError::Null)?;
        let lease = copy_into_shared_slot(source, &frame)?;
        Ok(RetainedSurface::new(frame.texture as usize, lease))
    }

    fn present(&self, _host: HostOs, surface: &RetainedSurface) -> Result<AnyElement, SurfaceError> {
        let lease = surface.keeper::<SlotLease>().ok_or(SurfaceError::WrongKind {
            expected: "a D3D11VA picture copied into a shared slot by this branch",
        })?;
        let owned = OwnedSurfaceFrame::shared_nv12(
            lease.stream_id,
            lease.generation,
            lease.width,
            lease.height,
            SharedSurfaceHandle {
                handle: lease.slot.handle.0 as isize,
                key: lease.slot.serial,
            },
            Arc::new(surface.clone()),
        )
        .ok_or(SurfaceError::WrongKind {
            expected: "a shared NV12 texture of a non-empty size",
        })?
        .with_ycbcr_to_rgb(lease.ycbcr_to_rgb);
        Ok(gpui::surface(owned).size_full().into_any_element())
    }

    fn present_host_frame(
        &self,
        _host: HostOs,
        frame: &HostNv12Frame,
    ) -> Result<AnyElement, SurfaceError> {
        let owned = OwnedSurfaceFrame::nv12(
            frame.stream_id,
            frame.generation,
            frame.width,
            frame.height,
            frame.luma.clone(),
            frame.luma_stride,
            frame.chroma.clone(),
            frame.chroma_stride,
        )
        .ok_or(SurfaceError::WrongKind {
            expected: "NV12 planes as long as their rows and strides need",
        })?
        .with_ycbcr_to_rgb(ycbcr_to_rgb_columns(&frame.conversion));

        Ok(gpui::surface(owned).size_full().into_any_element())
    }

    fn present_host_bgra_frame(
        &self,
        _host: HostOs,
        frame: &HostBgraFrame,
    ) -> Result<AnyElement, SurfaceError> {
        let owned = OwnedSurfaceFrame::bgra(
            frame.stream_id,
            frame.generation,
            frame.width,
            frame.height,
            frame.pixels.clone(),
            frame.stride,
        )
        .ok_or(SurfaceError::WrongKind {
            expected: "BGRA rows as long as their width and stride need",
        })?;

        Ok(gpui::surface(owned).size_full().into_any_element())
    }
}

struct SharedSlot {
    serial: u64,
    device: usize,
    width: u32,
    height: u32,
    texture: ID3D11Texture2D,
    mutex: IDXGIKeyedMutex,
    handle: HANDLE,
    last_used: Mutex<Duration>,
}

unsafe impl Send for SharedSlot {}
unsafe impl Sync for SharedSlot {}

impl Drop for SharedSlot {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

struct SlotLease {
    slot: Arc<SharedSlot>,
    stream_id: u64,
    generation: u64,
    width: u32,
    height: u32,
    ycbcr_to_rgb: [[f32; 4]; 4],
}

static SLOTS: Mutex<Vec<Arc<SharedSlot>>> = Mutex::new(Vec::new());
static NEXT_SERIAL: AtomicU64 = AtomicU64::new(1);

const SLOT_IDLE_LIMIT: Duration = Duration::from_secs(2);

fn platform(call: &'static str, error: windows::core::Error) -> SurfaceError {
    SurfaceError::Platform {
        call,
        status: error.code().0,
    }
}

fn copy_into_shared_slot(
    source: &ID3D11Texture2D,
    frame: &D3d11FrameRef,
) -> Result<SlotLease, SurfaceError> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { source.GetDesc(&mut desc) };
    if desc.Format != DXGI_FORMAT_NV12 {
        return Err(SurfaceError::WrongKind {
            expected: "an 8-bit NV12 D3D11VA texture (P010 goes through the CPU path)",
        });
    }
    let (width, height) = shared_copy_size(
        frame.width,
        frame.height,
        desc.Width,
        desc.Height,
        frame.array_index,
        desc.ArraySize,
    )
    .map_err(|_| SurfaceError::WrongKind {
        expected: "a picture inside its decoder pool texture",
    })?;

    let device = unsafe { source.GetDevice() }.map_err(|error| platform("GetDevice", error))?;
    let slot = take_slot(&device, width, height)?;

    unsafe {
        let status = (Interface::vtable(&slot.mutex).AcquireSync)(
            Interface::as_raw(&slot.mutex),
            KEYED_MUTEX_KEY,
            KEYED_MUTEX_WAIT_MS,
        );
        if status.0 != 0 {
            return Err(SurfaceError::Platform {
                call: "IDXGIKeyedMutex::AcquireSync",
                status: status.0,
            });
        }
        let context = device
            .GetImmediateContext()
            .map_err(|error| platform("GetImmediateContext", error));
        if let Ok(context) = &context {
            let region = D3D11_BOX {
                left: 0,
                top: 0,
                front: 0,
                right: width,
                bottom: height,
                back: 1,
            };
            context.CopySubresourceRegion(
                &slot.texture,
                0,
                0,
                0,
                0,
                source,
                frame.array_index * desc.MipLevels.max(1),
                Some(&region),
            );
        }
        slot.mutex
            .ReleaseSync(KEYED_MUTEX_KEY)
            .map_err(|error| platform("IDXGIKeyedMutex::ReleaseSync", error))?;
        context?.Flush();
    }

    Ok(SlotLease {
        stream_id: frame.texture as usize as u64,
        generation: NEXT_SERIAL.fetch_add(1, Ordering::Relaxed),
        width,
        height,
        ycbcr_to_rgb: frame.ycbcr_to_rgb,
        slot,
    })
}

fn take_slot(device: &ID3D11Device, width: u32, height: u32) -> Result<Arc<SharedSlot>, SurfaceError> {
    let device_key = device.as_raw() as usize;
    let mut slots = SLOTS.lock();
    let same = |slot: &Arc<SharedSlot>| {
        slot.device == device_key && slot.width == width && slot.height == height
    };
    let free = |slot: &Arc<SharedSlot>| Arc::strong_count(slot) == 1;

    let now = RiseClock::monotonic();
    slots.retain(|slot| !free(slot) || now.saturating_sub(*slot.last_used.lock()) < SLOT_IDLE_LIMIT);

    let live = slots.iter().filter(|slot| same(slot)).count();
    let idle = slots.iter().filter(|slot| same(slot) && free(slot)).count();
    let mut surplus = surplus_free_slots(idle);
    slots.retain(|slot| {
        let drop = surplus > 0 && same(slot) && free(slot);
        if drop {
            surplus -= 1;
        }
        !drop
    });

    match choose_slot(live, idle) {
        SlotChoice::Reuse => {
            let slot = slots
                .iter()
                .find(|slot| same(slot) && free(slot))
                .cloned()
                .ok_or(SurfaceError::Null)?;
            *slot.last_used.lock() = now;
            Ok(slot)
        }
        SlotChoice::Grow => {
            let slot = Arc::new(create_slot(device, device_key, width, height)?);
            slots.push(slot.clone());
            Ok(slot)
        }
        SlotChoice::Exhausted => Err(SurfaceError::WrongKind {
            expected: "a free shared slot (every slot of this size is still on screen)",
        }),
    }
}

fn create_slot(
    device: &ID3D11Device,
    device_key: usize,
    width: u32,
    height: u32,
) -> Result<SharedSlot, SurfaceError> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: (D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0 | D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0)
            as u32,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }
        .map_err(|error| platform("CreateTexture2D(NV12 shared)", error))?;
    let texture = texture.ok_or(SurfaceError::Null)?;
    let resource: IDXGIResource1 = texture
        .cast()
        .map_err(|error| platform("QueryInterface(IDXGIResource1)", error))?;
    let mutex: IDXGIKeyedMutex = texture
        .cast()
        .map_err(|error| platform("QueryInterface(IDXGIKeyedMutex)", error))?;
    let handle = unsafe {
        resource.CreateSharedHandle(
            None,
            DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0,
            PCWSTR::null(),
        )
    }
    .map_err(|error| platform("IDXGIResource1::CreateSharedHandle", error))?;

    Ok(SharedSlot {
        serial: NEXT_SERIAL.fetch_add(1, Ordering::Relaxed),
        device: device_key,
        width,
        height,
        texture,
        mutex,
        handle,
        last_used: Mutex::new(RiseClock::monotonic()),
    })
}
