use std::ffi::c_void;

use thiserror::Error;

pub const SHARED_SLOTS_KEPT: usize = 3;

pub const SHARED_SLOTS_MAX: usize = 8;

pub const KEYED_MUTEX_WAIT_MS: u32 = 16;

pub const KEYED_MUTEX_KEY: u64 = 0;

/// what the windows branch of retain_decoder_surface reads behind its address
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(C)]
pub struct D3d11FrameRef {
    pub texture: *mut c_void,
    pub array_index: u32,
    pub width: u32,
    pub height: u32,
    pub ycbcr_to_rgb: [[f32; 4]; 4],
}

/// why a d3d11va frame cant use the shared texture path, each case falls back to a cpu copy
#[derive(Clone, Copy, PartialEq, Eq, Debug, Error)]
pub enum D3d11FrameError {
    #[error("the decoder frame carries no D3D11 texture")]
    NoTexture,
    #[error("array slice {index} lies outside the decoder pool of {pool}")]
    SliceOutOfPool { index: u32, pool: u32 },
    #[error("the picture {width}x{height} does not fit the pool texture {pool_width}x{pool_height}")]
    LargerThanPool {
        width: u32,
        height: u32,
        pool_width: u32,
        pool_height: u32,
    },
    #[error("the picture size is empty")]
    Empty,
}

impl D3d11FrameRef {
    /// reads data[0] and data[1] of a d3d11 frame
    pub fn from_frame_planes(
        texture: *mut u8,
        slice: *mut u8,
        width: i32,
        height: i32,
        ycbcr_to_rgb: [[f32; 4]; 4],
    ) -> Result<Self, D3d11FrameError> {
        if texture.is_null() {
            return Err(D3d11FrameError::NoTexture);
        }
        let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
            return Err(D3d11FrameError::Empty);
        };
        let array_index = u32::try_from(slice as usize).map_err(|_| D3d11FrameError::SliceOutOfPool {
            index: u32::MAX,
            pool: 0,
        })?;
        Ok(Self {
            texture: texture.cast(),
            array_index,
            width,
            height,
            ycbcr_to_rgb,
        })
    }
}

/// region copied out of the pool texture and size of the shared texture
/// nv12 needs even sizes, odd rounds up if the pool has room (pools are aligned) else down
pub fn shared_copy_size(
    width: u32,
    height: u32,
    pool_width: u32,
    pool_height: u32,
    array_index: u32,
    array_size: u32,
) -> Result<(u32, u32), D3d11FrameError> {
    if array_index >= array_size {
        return Err(D3d11FrameError::SliceOutOfPool {
            index: array_index,
            pool: array_size,
        });
    }
    if width > pool_width || height > pool_height {
        return Err(D3d11FrameError::LargerThanPool {
            width,
            height,
            pool_width,
            pool_height,
        });
    }
    let even = |value: u32, room: u32| {
        let up = value.next_multiple_of(2);
        if up <= room { up } else { value & !1 }
    };
    let size = (even(width, pool_width), even(height, pool_height));
    if size.0 == 0 || size.1 == 0 {
        return Err(D3d11FrameError::Empty);
    }
    Ok(size)
}

/// what the pool does for the next picture, given the slots of its size and how many are free
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SlotChoice {
    Reuse,
    Grow,
    Exhausted,
}

pub const fn choose_slot(live: usize, free: usize) -> SlotChoice {
    if free > 0 {
        SlotChoice::Reuse
    } else if live < SHARED_SLOTS_MAX {
        SlotChoice::Grow
    } else {
        SlotChoice::Exhausted
    }
}

/// how many free slots of a size to drop so the pool keeps at most the idle limit
pub const fn surplus_free_slots(free: usize) -> usize {
    free.saturating_sub(SHARED_SLOTS_KEPT)
}
