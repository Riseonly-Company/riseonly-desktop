use gpui::{Bounds, Pixels, px};

use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementPart, SharedElementVisibilityPolicy, SharedElementVisibilityPolicyCandidate,
};

struct Best<T> {
    pick: T,
    score: f32,
    distance: f32,
}

impl SharedElementVisibilityPolicy {
    const TIE_TOLERANCE: f32 = 1.0;

    /// visible part of the list viewport, below the floating header and above the bottom chrome
    pub fn band(viewport: Bounds<Pixels>, top_inset: f32, bottom_edge: Option<f32>, safe_bottom: f32) -> Bounds<Pixels> {
        let top = f32::from(viewport.top()) + top_inset;
        let bottom = (f32::from(viewport.bottom()) - safe_bottom).min(bottom_edge.unwrap_or(f32::INFINITY));
        Bounds::new(
            gpui::point(viewport.origin.x, px(top)),
            gpui::size(viewport.size.width, px((bottom - top).max(0.0))),
        )
    }

    /// most visible post in the band, media frame scores alone and text frame only for posts without media
    /// ties go to the frame closest to the center and a zero best score picks nothing
    pub fn pick_current(
        candidates: &[SharedElementVisibilityPolicyCandidate],
        band: Bounds<Pixels>,
    ) -> Option<(String, SharedElementPart)> {
        let mut best: Option<Best<(&str, SharedElementPart)>> = None;

        for candidate in candidates {
            let (frame, part) = if let Some(media) = candidate.media_frame {
                (media, SharedElementPart::Media)
            } else if let Some(text) = candidate.text_frame {
                (text, SharedElementPart::Text)
            } else {
                continue;
            };

            let score = Self::visible_area(frame, band);
            if !(score > 0.0) {
                continue;
            }
            let distance = (f32::from(frame.center().y) - f32::from(band.center().y)).abs();
            if Self::wins(best.as_ref(), score, distance) {
                best = Some(Best { pick: (candidate.item_id.as_str(), part), score, distance });
            }
        }

        best.map(|best| (best.pick.0.to_string(), best.pick.1))
    }

    /// carousel page with the largest area inside the clip
    /// ties go to the page closest to the clip center and none when no page overlaps
    pub fn pick_page(page_frames: &[(usize, Bounds<Pixels>)], clip: Bounds<Pixels>) -> Option<usize> {
        let mut best: Option<Best<usize>> = None;

        for &(index, frame) in page_frames {
            let score = Self::visible_area(frame, clip);
            if !(score > 0.0) {
                continue;
            }
            let distance = (f32::from(frame.center().x) - f32::from(clip.center().x)).abs();
            if Self::wins(best.as_ref(), score, distance) {
                best = Some(Best { pick: index, score, distance });
            }
        }

        best.map(|best| best.pick)
    }

    fn wins<T>(current: Option<&Best<T>>, score: f32, distance: f32) -> bool {
        match current {
            None => true,
            Some(current) => {
                score - current.score >= Self::TIE_TOLERANCE
                    || ((score - current.score).abs() < Self::TIE_TOLERANCE && distance < current.distance)
            }
        }
    }

    fn visible_area(frame: Bounds<Pixels>, band: Bounds<Pixels>) -> f32 {
        if !frame.intersects(&band) {
            return 0.0;
        }
        let overlap = frame.intersect(&band);
        let area = f32::from(overlap.size.width) * f32::from(overlap.size.height);
        if area.is_finite() { area } else { 0.0 }
    }
}
