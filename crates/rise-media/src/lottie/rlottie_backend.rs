//! The only file that knows rlottie exists, behind the `rlottie` feature.
//!
//! `resource_path` is passed empty, so an animation can never reference a file
//! on this machine; only embedded base64 assets resolve.

use std::ffi::{CString, c_char, c_void};

use super::sequence::LottieRasterizer;

unsafe extern "C" {
    fn lottie_animation_from_data(
        data: *const c_char,
        key: *const c_char,
        resource_path: *const c_char,
    ) -> *mut c_void;

    fn lottie_animation_destroy(animation: *mut c_void);

    fn lottie_animation_get_totalframe(animation: *const c_void) -> usize;

    fn lottie_animation_get_framerate(animation: *const c_void) -> f64;

    fn lottie_animation_get_size(animation: *const c_void, width: *mut usize, height: *mut usize);

    fn lottie_animation_render(
        animation: *mut c_void,
        frame_num: usize,
        buffer: *mut u32,
        width: usize,
        height: usize,
        bytes_per_line: usize,
    );
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RlottieError {
    /// The payload has an interior NUL and cannot be passed to a `const char *`.
    NotNulTerminatable,
    /// rlottie returned no animation: malformed, or a document it will not accept.
    Rejected,
    /// Parsed, but the animation has no drawable size.
    ZeroSized,
}

/// One rlottie animation handle.
///
/// Not `Sync`: rlottie keeps per-render state on the handle, so a caller must
/// hold it exclusively.
pub struct RlottieRasterizer {
    animation: *mut c_void,
    frame_count: usize,
    frame_rate: f64,
}

// SAFETY: the handle is owned exclusively by this value and every call through it takes &mut self, so it is never used from two threads at once.
unsafe impl Send for RlottieRasterizer {}

impl RlottieRasterizer {
    /// `json` must already have been through `container::to_animation_json`:
    /// that function is the validation boundary in front of this C++ parser.
    pub fn open(json: &[u8], cache_key: &str) -> Result<Self, RlottieError> {
        let json = CString::new(json).map_err(|_| RlottieError::NotNulTerminatable)?;
        let key = CString::new(cache_key).map_err(|_| RlottieError::NotNulTerminatable)?;
        let resource_path = CString::new("").expect("an empty string has no interior NUL");

        // SAFETY: all three pointers are valid, NUL-terminated, and outlive the call.
        let animation = unsafe {
            lottie_animation_from_data(json.as_ptr(), key.as_ptr(), resource_path.as_ptr())
        };

        if animation.is_null() {
            return Err(RlottieError::Rejected);
        }

        // SAFETY: `animation` is non-null and was produced by the call above.
        let (frame_count, frame_rate, width, height) = unsafe {
            let mut width = 0usize;
            let mut height = 0usize;
            lottie_animation_get_size(animation, &mut width, &mut height);
            (
                lottie_animation_get_totalframe(animation),
                lottie_animation_get_framerate(animation),
                width,
                height,
            )
        };

        if width == 0 || height == 0 {
            // SAFETY: still the handle from above, and it is not used again.
            unsafe { lottie_animation_destroy(animation) };
            return Err(RlottieError::ZeroSized);
        }

        Ok(Self {
            animation,
            frame_count,
            frame_rate,
        })
    }
}

impl Drop for RlottieRasterizer {
    fn drop(&mut self) {
        // SAFETY: constructed non-null, destroyed exactly once, and no render can be in flight because rendering takes &mut self.
        unsafe { lottie_animation_destroy(self.animation) };
    }
}

impl LottieRasterizer for RlottieRasterizer {
    fn frame_count(&self) -> usize {
        self.frame_count
    }

    fn frame_rate(&self) -> f64 {
        self.frame_rate
    }

    fn render(&mut self, frame: usize, dimension: u32, out: &mut [u32]) {
        let dimension = dimension as usize;
        debug_assert_eq!(out.len(), dimension * dimension);

        // SAFETY: `out` is exactly dimension × dimension u32s, matching the width, height and stride below, and a [u32] is four-byte aligned.
        unsafe {
            lottie_animation_render(
                self.animation,
                frame,
                out.as_mut_ptr(),
                dimension,
                dimension,
                dimension * 4,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::container;
    use super::*;

    const RED_SQUARE: &str = r#"{
        "v":"5.5.7","fr":60,"ip":0,"op":60,"w":100,"h":100,"nm":"square","ddd":0,
        "assets":[],
        "layers":[{
            "ddd":0,"ind":1,"ty":4,"nm":"shape","sr":1,
            "ks":{"o":{"a":0,"k":100},"r":{"a":0,"k":0},"p":{"a":0,"k":[50,50,0]},
                  "a":{"a":0,"k":[0,0,0]},"s":{"a":0,"k":[100,100,100]}},
            "ao":0,
            "shapes":[{
                "ty":"gr",
                "it":[
                    {"ty":"rc","d":1,"s":{"a":0,"k":[100,100]},"p":{"a":0,"k":[0,0]},"r":{"a":0,"k":0}},
                    {"ty":"fl","c":{"a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100},"r":1},
                    {"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},
                     "s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}}
                ]
            }],
            "ip":0,"op":60,"st":0,"bm":0
        }],
        "markers":[]
    }"#;

    #[test]
    fn the_linked_rlottie_actually_rasterises_a_document() {
        let json = container::to_animation_json(RED_SQUARE.as_bytes()).unwrap();
        let mut rasterizer = RlottieRasterizer::open(&json, "red-square").unwrap();

        assert_eq!(rasterizer.frame_count(), 60);
        assert_eq!(rasterizer.frame_rate(), 60.0);

        let mut pixels = vec![0u32; 64 * 64];
        rasterizer.render(0, 64, &mut pixels);

        let centre = pixels[64 * 32 + 32];
        assert_eq!(
            centre >> 24,
            255,
            "the centre of an opaque square came out transparent: \
             the library linked, but it is not drawing"
        );
        assert!(
            (centre >> 16) & 0xFF > 200,
            "premultiplied ARGB puts red in bits 16..23; got {centre:#010x}"
        );
        assert!(
            (centre & 0xFF) < 40,
            "the blue channel is high, so the byte order is not what the atlas expects"
        );
    }

    #[test]
    fn every_frame_of_a_real_animation_is_produced() {
        let json = container::to_animation_json(RED_SQUARE.as_bytes()).unwrap();
        let mut rasterizer = RlottieRasterizer::open(&json, "red-square").unwrap();

        for frame in 0..rasterizer.frame_count() {
            let mut pixels = vec![0u32; 32 * 32];
            rasterizer.render(frame, 32, &mut pixels);
            assert!(
                pixels.iter().any(|pixel| pixel >> 24 > 0),
                "frame {frame} is empty"
            );
        }
    }

    #[test]
    fn a_document_rlottie_will_not_accept_is_rejected_rather_than_crashing() {
        assert_eq!(
            RlottieRasterizer::open(b"{}", "empty").err(),
            Some(RlottieError::Rejected)
        );
    }

    #[test]
    fn a_zero_sized_animation_is_refused_before_anything_divides_by_it() {
        let zero = br#"{"v":"5.5.7","fr":60,"ip":0,"op":60,"w":0,"h":0,"layers":[]}"#;

        assert_eq!(
            RlottieRasterizer::open(zero, "zero").err(),
            Some(RlottieError::ZeroSized)
        );
    }

    #[test]
    fn a_payload_with_an_interior_nul_never_reaches_the_c_parser() {
        assert_eq!(
            RlottieRasterizer::open(b"{\0\"layers\":[]}", "nul").err(),
            Some(RlottieError::NotNulTerminatable)
        );
    }

    #[test]
    fn a_sequence_can_be_driven_end_to_end_by_the_real_rasteriser() {
        use super::super::frame_cache::FrameCache;
        use super::super::raster_policy::SequenceKey;

        let json = container::to_animation_json(RED_SQUARE.as_bytes()).unwrap();
        let cache = FrameCache::new();
        let sequence = cache
            .open(
                SequenceKey::new("red-square", 64),
                Box::new(RlottieRasterizer::open(&json, "red-square").unwrap()),
            )
            .unwrap();

        assert_eq!(sequence.presentation_stride(), 2);
        assert_eq!(sequence.first_visible_frame(), Some(0));

        let first = sequence.frame(0).unwrap();
        cache.release_decoded_frames();
        let replayed = sequence.frame(0).unwrap();

        assert_eq!(
            first.pixels(),
            replayed.pixels(),
            "the LZ4 round trip changed a real rasterised frame"
        );
        assert!(sequence.compressed_bytes() > 0);
    }
}
