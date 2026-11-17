use std::any::Any;
use std::os::raw::c_int;
use std::sync::Mutex;

use thiserror::Error;

pub const DRM_MAX_PLANES: usize = 4;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DrmObjectDescriptor {
    pub fd: c_int,
    pub size: usize,
    pub format_modifier: u64,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DrmPlaneDescriptor {
    pub object_index: c_int,
    pub offset: isize,
    pub pitch: isize,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DrmLayerDescriptor {
    pub format: u32,
    pub nb_planes: c_int,
    pub planes: [DrmPlaneDescriptor; DRM_MAX_PLANES],
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DrmFrameDescriptor {
    pub nb_objects: c_int,
    pub objects: [DrmObjectDescriptor; DRM_MAX_PLANES],
    pub nb_layers: c_int,
    pub layers: [DrmLayerDescriptor; DRM_MAX_PLANES],
}

/// fourcc_code of drm_fourcc.h, first byte is the lowest
pub const fn fourcc(code: &[u8; 4]) -> u32 {
    (code[0] as u32) | ((code[1] as u32) << 8) | ((code[2] as u32) << 16) | ((code[3] as u32) << 24)
}

pub const DRM_FORMAT_NV12: u32 = fourcc(b"NV12");
pub const DRM_FORMAT_P010: u32 = fourcc(b"P010");
pub const DRM_FORMAT_R8: u32 = fourcc(b"R8  ");
pub const DRM_FORMAT_GR88: u32 = fourcc(b"GR88");
pub const DRM_FORMAT_R16: u32 = fourcc(b"R16 ");
pub const DRM_FORMAT_GR1616: u32 = fourcc(b"GR32");
pub const DRM_FORMAT_MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;

/// the four characters of a fourcc, for logs
pub fn fourcc_name(format: u32) -> String {
    format
        .to_le_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                *byte as char
            } else {
                '?'
            }
        })
        .collect()
}

/// one plane of the frame as the importer needs it, which file and where and how wide a row is
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DmaBufPlane {
    pub layer_format: u32,
    pub fd: i32,
    pub object_size: u64,
    pub offset: u64,
    pub pitch: u32,
    pub modifier: u64,
}

/// a whole two plane ycbcr frame ready for a dma-buf import
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DmaBufLayout {
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub planes: Vec<DmaBufPlane>,
}

impl DmaBufLayout {
    pub fn luma(&self) -> &DmaBufPlane {
        &self.planes[0]
    }

    pub fn chroma(&self) -> &DmaBufPlane {
        &self.planes[1]
    }

    /// the modifier every plane shares, if they share one
    pub fn common_modifier(&self) -> Option<u64> {
        let first = self.planes.first()?.modifier;
        self.planes
            .iter()
            .all(|plane| plane.modifier == first)
            .then_some(first)
    }

    /// checks a descriptor and flattens its layers into luma and chroma planes
    pub fn from_descriptor(
        descriptor: &DrmFrameDescriptor,
        width: u32,
        height: u32,
    ) -> Result<Self, DrmPrimeError> {
        let objects = count(descriptor.nb_objects, "objects")?;
        let layers = count(descriptor.nb_layers, "layers")?;

        for (index, object) in descriptor.objects[..objects].iter().enumerate() {
            if object.fd < 0 {
                return Err(DrmPrimeError::BadObject { index });
            }
        }

        let mut planes = Vec::with_capacity(2);
        let mut layer_formats = Vec::with_capacity(layers);

        for layer in &descriptor.layers[..layers] {
            layer_formats.push(layer.format);
            let layer_planes = count(layer.nb_planes, "planes")?;

            for plane in &layer.planes[..layer_planes] {
                let index = planes.len();
                if index == DRM_MAX_PLANES {
                    return Err(DrmPrimeError::TooMany {
                        what: "planes",
                        count: index + 1,
                    });
                }

                let object_index = usize::try_from(plane.object_index)
                    .ok()
                    .filter(|object_index| *object_index < objects)
                    .ok_or(DrmPrimeError::ObjectIndexOutOfRange {
                        plane: index,
                        object_index: plane.object_index,
                    })?;
                let object = &descriptor.objects[object_index];

                let offset =
                    u64::try_from(plane.offset).map_err(|_| DrmPrimeError::BadPlane { plane: index })?;
                let pitch = u32::try_from(plane.pitch)
                    .ok()
                    .filter(|pitch| *pitch > 0)
                    .ok_or(DrmPrimeError::BadPlane { plane: index })?;

                planes.push(DmaBufPlane {
                    layer_format: layer.format,
                    fd: object.fd,
                    object_size: object.size as u64,
                    offset,
                    pitch,
                    modifier: object.format_modifier,
                });
            }
        }

        let format = composed_format(&layer_formats, planes.len())
            .ok_or_else(|| DrmPrimeError::UnsupportedLayout(layer_formats.clone()))?;

        Ok(Self {
            format,
            width,
            height,
            planes,
        })
    }
}

fn count(value: c_int, what: &'static str) -> Result<usize, DrmPrimeError> {
    match usize::try_from(value) {
        Ok(0) | Err(_) => Err(DrmPrimeError::Empty { what }),
        Ok(count) if count > DRM_MAX_PLANES => Err(DrmPrimeError::TooMany { what, count }),
        Ok(count) => Ok(count),
    }
}

fn composed_format(layer_formats: &[u32], plane_count: usize) -> Option<u32> {
    if plane_count != 2 {
        return None;
    }
    match layer_formats {
        [DRM_FORMAT_NV12] | [DRM_FORMAT_R8, DRM_FORMAT_GR88] => Some(DRM_FORMAT_NV12),
        [DRM_FORMAT_P010] | [DRM_FORMAT_R16, DRM_FORMAT_GR1616] => Some(DRM_FORMAT_P010),
        _ => None,
    }
}

/// why a mapped frame cant be described as a dma-buf, caller falls back to a cpu upload
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum DrmPrimeError {
    #[error("the descriptor has no {what}")]
    Empty { what: &'static str },
    #[error("the descriptor has {count} {what}, more than AV_DRM_MAX_PLANES")]
    TooMany { what: &'static str, count: usize },
    #[error("object {index} has no file descriptor")]
    BadObject { index: usize },
    #[error("plane {plane} names object {object_index}, which the descriptor does not have")]
    ObjectIndexOutOfRange { plane: usize, object_index: c_int },
    #[error("plane {plane} has a negative offset or no pitch")]
    BadPlane { plane: usize },
    #[error("layers {} are not a two-plane NV12 or P010 frame", layer_names(.0))]
    UnsupportedLayout(Vec<u32>),
}

fn layer_names(formats: &[u32]) -> String {
    formats
        .iter()
        .map(|format| fourcc_name(*format))
        .collect::<Vec<_>>()
        .join("+")
}

/// holder a dma-buf surface carries, the layout and the mapping that keeps the fds open
/// dropping the last clone closes them
pub struct DmaBufFrame {
    layout: DmaBufLayout,
    _mapping: Mutex<Box<dyn Any + Send>>,
}

impl DmaBufFrame {
    pub fn new<M: Any + Send>(layout: DmaBufLayout, mapping: M) -> Self {
        Self {
            layout,
            _mapping: Mutex::new(Box::new(mapping)),
        }
    }

    pub fn layout(&self) -> &DmaBufLayout {
        &self.layout
    }
}
