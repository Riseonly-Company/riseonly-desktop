use std::ffi::{CString, c_char, c_void};

use super::sequence::LottieRasterizer;
use crate::decode_guard::decode_guard::{
    DecodeGuardError, check_lottie_frame, check_lottie_json, check_lottie_timeline, native_call,
};

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
    NotNulTerminatable,
    Rejected,
    ZeroSized,
    Guard(DecodeGuardError),
}

/// one rlottie animation handle, not Sync since rlottie keeps render state on it
/// frame sequences and poster cache use the inherent methods, the trait impl only serves the kit drafts
pub struct RlottieRasterizer {
    animation: *mut c_void,
    frame_count: usize,
    frame_rate: f64,
}

unsafe impl Send for RlottieRasterizer {}

impl RlottieRasterizer {
    /// json must already have gone through to_animation_json
    pub fn open(json: &[u8], cache_key: &str) -> Result<Self, RlottieError> {
        let terminated = CString::new(json).map_err(|_| RlottieError::NotNulTerminatable)?;
        let key = CString::new(cache_key).map_err(|_| RlottieError::NotNulTerminatable)?;
        check_lottie_json(json).map_err(RlottieError::Guard)?;

        let animation = native_call(|| unsafe {
            lottie_animation_from_data(terminated.as_ptr(), key.as_ptr(), c"".as_ptr())
        })
        .map_err(RlottieError::Guard)?;

        if animation.is_null() {
            return Err(RlottieError::Rejected);
        }

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
            unsafe { lottie_animation_destroy(animation) };
            return Err(RlottieError::ZeroSized);
        }

        if let Err(refusal) = check_lottie_timeline(frame_count as u64, frame_rate) {
            unsafe { lottie_animation_destroy(animation) };
            return Err(RlottieError::Guard(refusal));
        }

        Ok(Self {
            animation,
            frame_count,
            frame_rate,
        })
    }

    /// total frames in the animation
    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    /// authored frame rate before any presentation stride
    pub fn frame_rate(&self) -> f64 {
        self.frame_rate
    }

    pub fn render(&mut self, frame: usize, dimension: u32, out: &mut [u32]) {
        out.fill(0);
        let side = dimension as usize;
        if out.len() != side * side || check_lottie_frame(dimension).is_err() {
            tracing::warn!(target: "riseonly", dimension, "a Lottie frame buffer was refused");
            return;
        }

        let rendered = native_call(|| unsafe {
            lottie_animation_render(self.animation, frame, out.as_mut_ptr(), side, side, side * 4);
        });
        if rendered.is_err() {
            out.fill(0);
        }
    }
}

impl Drop for RlottieRasterizer {
    fn drop(&mut self) {
        unsafe { lottie_animation_destroy(self.animation) };
    }
}

impl LottieRasterizer for RlottieRasterizer {
    fn frame_count(&self) -> usize {
        RlottieRasterizer::frame_count(self)
    }

    fn frame_rate(&self) -> f64 {
        RlottieRasterizer::frame_rate(self)
    }

    fn render(&mut self, frame: usize, dimension: u32, out: &mut [u32]) {
        RlottieRasterizer::render(self, frame, dimension, out);
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
            Some(RlottieError::Guard(DecodeGuardError::LottieHeaderInvalid))
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
        use std::sync::Arc;

        use super::super::raster_policy::{self, SequenceKey};
        use super::super::sequence::{DecodedFrameCache, FrameSequence};

        let json = container::to_animation_json(RED_SQUARE.as_bytes()).unwrap();
        let decoded = Arc::new(DecodedFrameCache::new(raster_policy::MAXIMUM_DECODED_BYTES));
        let sequence = FrameSequence::open(
            SequenceKey::new("red-square", 64),
            Box::new(RlottieRasterizer::open(&json, "red-square").unwrap()),
            decoded.clone(),
        )
        .unwrap();

        assert_eq!(sequence.presentation_stride(), 2);
        assert_eq!(sequence.first_visible_frame(), Some(0));

        let first = sequence.frame(0).unwrap();
        decoded.clear();
        let replayed = sequence.frame(0).unwrap();

        assert_eq!(
            first.pixels(),
            replayed.pixels(),
            "the LZ4 round trip changed a real rasterised frame"
        );
        assert!(sequence.compressed_bytes() > 0);
    }
}
