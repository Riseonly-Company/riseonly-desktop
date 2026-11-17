use std::mem::{offset_of, size_of};

use crate::video::drm_prime::*;

const LINEAR: u64 = 0;
const Y_TILED: u64 = 0x0100_0000_0000_0002;

fn object(fd: i32) -> DrmObjectDescriptor {
    DrmObjectDescriptor {
        fd,
        size: 3_133_440,
        format_modifier: Y_TILED,
    }
}

fn plane(object_index: i32, offset: isize, pitch: isize) -> DrmPlaneDescriptor {
    DrmPlaneDescriptor {
        object_index,
        offset,
        pitch,
    }
}

fn layer(format: u32, planes: &[DrmPlaneDescriptor]) -> DrmLayerDescriptor {
    let mut layer = DrmLayerDescriptor {
        format,
        nb_planes: planes.len() as i32,
        ..Default::default()
    };
    layer.planes[..planes.len()].copy_from_slice(planes);
    layer
}

fn frame(objects: &[DrmObjectDescriptor], layers: &[DrmLayerDescriptor]) -> DrmFrameDescriptor {
    let mut frame = DrmFrameDescriptor {
        nb_objects: objects.len() as i32,
        nb_layers: layers.len() as i32,
        ..Default::default()
    };
    frame.objects[..objects.len()].copy_from_slice(objects);
    frame.layers[..layers.len()].copy_from_slice(layers);
    frame
}

/// what vaapi_map_to_drm_esh gives for a 1920x1080 nv12 surface, one object and two layers
fn separate_layers() -> DrmFrameDescriptor {
    frame(
        &[object(17)],
        &[
            layer(DRM_FORMAT_R8, &[plane(0, 0, 2048)]),
            layer(DRM_FORMAT_GR88, &[plane(0, 2_228_224, 2048)]),
        ],
    )
}

#[test]
#[cfg(target_pointer_width = "64")]
fn the_mirrored_descriptor_matches_libavutil_on_64_bit() {
    assert_eq!(size_of::<DrmObjectDescriptor>(), 24);
    assert_eq!(size_of::<DrmPlaneDescriptor>(), 24);
    assert_eq!(size_of::<DrmLayerDescriptor>(), 104);
    assert_eq!(offset_of!(DrmFrameDescriptor, objects), 8);
    assert_eq!(offset_of!(DrmFrameDescriptor, nb_layers), 104);
    assert_eq!(offset_of!(DrmFrameDescriptor, layers), 112);
    assert_eq!(size_of::<DrmFrameDescriptor>(), 528);
}

#[test]
fn fourcc_codes_match_drm_fourcc_h() {
    assert_eq!(DRM_FORMAT_NV12, 0x3231_564e);
    assert_eq!(DRM_FORMAT_R8, 0x2020_3852);
    assert_eq!(DRM_FORMAT_GR88, 0x3838_5247);
    assert_eq!(fourcc_name(DRM_FORMAT_NV12), "NV12");
    assert_eq!(fourcc_name(0x0000_0001), "????");
}

#[test]
fn separate_r8_and_gr88_layers_are_one_nv12_frame() {
    let layout = DmaBufLayout::from_descriptor(&separate_layers(), 1920, 1080).unwrap();

    assert_eq!(layout.format, DRM_FORMAT_NV12);
    assert_eq!(layout.planes.len(), 2);
    assert_eq!(layout.luma().layer_format, DRM_FORMAT_R8);
    assert_eq!(layout.luma().fd, 17);
    assert_eq!(layout.luma().offset, 0);
    assert_eq!(layout.chroma().layer_format, DRM_FORMAT_GR88);
    assert_eq!(layout.chroma().offset, 2_228_224);
    assert_eq!(layout.chroma().pitch, 2048);
    assert_eq!(layout.common_modifier(), Some(Y_TILED));
}

#[test]
fn a_composed_nv12_layer_with_two_planes_is_accepted() {
    let descriptor = frame(
        &[object(5)],
        &[layer(DRM_FORMAT_NV12, &[plane(0, 0, 1920), plane(0, 2_073_600, 1920)])],
    );

    let layout = DmaBufLayout::from_descriptor(&descriptor, 1920, 1080).unwrap();
    assert_eq!(layout.format, DRM_FORMAT_NV12);
    assert_eq!(layout.chroma().layer_format, DRM_FORMAT_NV12);
}

#[test]
fn ten_bit_layers_are_p010() {
    let descriptor = frame(
        &[object(5)],
        &[
            layer(DRM_FORMAT_R16, &[plane(0, 0, 4096)]),
            layer(DRM_FORMAT_GR1616, &[plane(0, 4_423_680, 4096)]),
        ],
    );

    let layout = DmaBufLayout::from_descriptor(&descriptor, 1920, 1080).unwrap();
    assert_eq!(layout.format, DRM_FORMAT_P010);
}

#[test]
fn planes_in_different_objects_keep_their_own_fd_and_modifier() {
    let mut chroma_object = object(23);
    chroma_object.format_modifier = LINEAR;
    let descriptor = frame(
        &[object(22), chroma_object],
        &[
            layer(DRM_FORMAT_R8, &[plane(0, 0, 2048)]),
            layer(DRM_FORMAT_GR88, &[plane(1, 0, 2048)]),
        ],
    );

    let layout = DmaBufLayout::from_descriptor(&descriptor, 1920, 1080).unwrap();
    assert_eq!(layout.luma().fd, 22);
    assert_eq!(layout.chroma().fd, 23);
    assert_eq!(layout.common_modifier(), None);
}

#[test]
fn an_empty_descriptor_is_refused() {
    assert_eq!(
        DmaBufLayout::from_descriptor(&DrmFrameDescriptor::default(), 1, 1),
        Err(DrmPrimeError::Empty { what: "objects" })
    );
}

#[test]
fn counts_above_the_header_limit_are_refused_before_indexing() {
    let mut descriptor = separate_layers();
    descriptor.nb_layers = 5;

    assert_eq!(
        DmaBufLayout::from_descriptor(&descriptor, 1920, 1080),
        Err(DrmPrimeError::TooMany {
            what: "layers",
            count: 5
        })
    );
}

#[test]
fn a_closed_object_is_refused() {
    let mut descriptor = separate_layers();
    descriptor.objects[0].fd = -1;

    assert_eq!(
        DmaBufLayout::from_descriptor(&descriptor, 1920, 1080),
        Err(DrmPrimeError::BadObject { index: 0 })
    );
}

#[test]
fn a_plane_naming_a_missing_object_is_refused() {
    let mut descriptor = separate_layers();
    descriptor.layers[1].planes[0].object_index = 1;

    assert_eq!(
        DmaBufLayout::from_descriptor(&descriptor, 1920, 1080),
        Err(DrmPrimeError::ObjectIndexOutOfRange {
            plane: 1,
            object_index: 1
        })
    );
}

#[test]
fn a_plane_without_a_pitch_is_refused() {
    let mut descriptor = separate_layers();
    descriptor.layers[0].planes[0].pitch = 0;

    assert_eq!(
        DmaBufLayout::from_descriptor(&descriptor, 1920, 1080),
        Err(DrmPrimeError::BadPlane { plane: 0 })
    );
}

#[test]
fn a_single_plane_rgb_export_is_not_a_video_frame() {
    let argb = fourcc(b"AR24");
    let descriptor = frame(&[object(3)], &[layer(argb, &[plane(0, 0, 7680)])]);

    let error = DmaBufLayout::from_descriptor(&descriptor, 1920, 1080).unwrap_err();
    assert_eq!(error, DrmPrimeError::UnsupportedLayout(vec![argb]));
    assert!(error.to_string().contains("AR24"), "{error}");
}

#[test]
fn the_frame_keeps_its_mapping_until_dropped() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Mapping(Arc<AtomicBool>);
    impl Drop for Mapping {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let released = Arc::new(AtomicBool::new(false));
    let layout = DmaBufLayout::from_descriptor(&separate_layers(), 1920, 1080).unwrap();
    let held = DmaBufFrame::new(layout.clone(), Mapping(released.clone()));

    assert_eq!(held.layout(), &layout);
    assert!(!released.load(Ordering::SeqCst));
    drop(held);
    assert!(released.load(Ordering::SeqCst));
}
