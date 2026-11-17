use gpui::{Bounds, Pixels, Point, Size, point, px};

use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementCoverSchedule, SharedElementCubicTiming, SharedElementGeometry,
};

impl SharedElementGeometry {
    /// smallest rect of this aspect that covers the clip, centered in it
    pub fn fill_rect(aspect: f32, clip: Bounds<Pixels>) -> Bounds<Pixels> {
        let (width, height) = (f32::from(clip.size.width), f32::from(clip.size.height));
        if !Self::is_usable(aspect) || width <= 0.0 || height <= 0.0 {
            return clip;
        }
        let fitted = if aspect >= width / height {
            gpui::size(px(height * aspect), px(height))
        } else {
            gpui::size(px(width), px(width / aspect))
        };
        Self::rect(clip.center(), fitted)
    }

    /// largest rect of this aspect inside the clip, centered in it
    pub fn fit_rect(aspect: f32, clip: Bounds<Pixels>) -> Bounds<Pixels> {
        let (width, height) = (f32::from(clip.size.width), f32::from(clip.size.height));
        if !Self::is_usable(aspect) || width <= 0.0 || height <= 0.0 {
            return clip;
        }
        let fitted = if aspect >= width / height {
            gpui::size(px(width), px(width / aspect))
        } else {
            gpui::size(px(height * aspect), px(height))
        };
        Self::rect(clip.center(), fitted)
    }

    /// width of the clip and height from the content aspect, pinned top leading
    pub fn anchored_width_fit_rect(content_size: Size<Pixels>, clip: Bounds<Pixels>) -> Bounds<Pixels> {
        let content_width = f32::from(content_size.width);
        let content_height = f32::from(content_size.height);
        if !(content_width > 0.0) || !content_height.is_finite() || content_height < 0.0 {
            return clip;
        }
        let width = f32::from(clip.size.width);
        Bounds::new(clip.origin, gpui::size(px(width), px(width * content_height / content_width)))
    }

    pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * t
    }

    pub fn lerp_rect(a: Bounds<Pixels>, b: Bounds<Pixels>, t: f32) -> Bounds<Pixels> {
        Bounds::new(
            point(
                px(Self::lerp(f32::from(a.origin.x), f32::from(b.origin.x), t)),
                px(Self::lerp(f32::from(a.origin.y), f32::from(b.origin.y), t)),
            ),
            gpui::size(
                px(Self::lerp(f32::from(a.size.width), f32::from(b.size.width), t)),
                px(Self::lerp(f32::from(a.size.height), f32::from(b.size.height), t)),
            ),
        )
    }

    pub fn smoothstep(x: f32) -> f32 {
        let t = x.max(0.0).min(1.0);
        t * t * (3.0 - 2.0 * t)
    }

    pub fn rect(center: Point<Pixels>, size: Size<Pixels>) -> Bounds<Pixels> {
        Bounds::new(point(center.x - size.width / 2.0, center.y - size.height / 2.0), size)
    }

    /// largest distance between matching edges, infinite when either rect is null
    /// a rect with any non finite edge counts as null
    pub fn max_edge_delta(a: Bounds<Pixels>, b: Bounds<Pixels>) -> f32 {
        if Self::is_null(a) || Self::is_null(b) {
            return f32::INFINITY;
        }
        let left = (f32::from(a.left()) - f32::from(b.left())).abs();
        let top = (f32::from(a.top()) - f32::from(b.top())).abs();
        let right = (f32::from(a.right()) - f32::from(b.right())).abs();
        let bottom = (f32::from(a.bottom()) - f32::from(b.bottom())).abs();
        left.max(top).max(right).max(bottom)
    }

    fn is_null(rect: Bounds<Pixels>) -> bool {
        ![rect.origin.x, rect.origin.y, rect.size.width, rect.size.height]
            .into_iter()
            .all(|value| f32::from(value).is_finite())
    }

    fn is_usable(aspect: f32) -> bool {
        aspect.is_finite() && aspect > 0.0
    }
}

impl SharedElementCubicTiming {
    /// curve value for normalized time t, assumes control x values in 0 to 1
    pub fn progress(&self, t: f32) -> f32 {
        if t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        let s = self.solve(t, Self::bezier_x, Self::bezier_x_slope);
        self.bezier_y(s)
    }

    /// normalized time where the curve reaches u, assumes a monotone curve
    pub fn time(&self, u: f32) -> f32 {
        if u <= 0.0 {
            return 0.0;
        }
        if u >= 1.0 {
            return 1.0;
        }
        let s = self.solve(u, Self::bezier_y, Self::bezier_y_slope);
        self.bezier_x(s)
    }

    fn bezier_x(&self, s: f32) -> f32 {
        Self::component(s, self.c1.x, self.c2.x)
    }

    fn bezier_y(&self, s: f32) -> f32 {
        Self::component(s, self.c1.y, self.c2.y)
    }

    fn bezier_x_slope(&self, s: f32) -> f32 {
        Self::component_slope(s, self.c1.x, self.c2.x)
    }

    fn bezier_y_slope(&self, s: f32) -> f32 {
        Self::component_slope(s, self.c1.y, self.c2.y)
    }

    fn component(s: f32, p1: f32, p2: f32) -> f32 {
        let inv = 1.0 - s;
        3.0 * inv * inv * s * p1 + 3.0 * inv * s * s * p2 + s * s * s
    }

    fn component_slope(s: f32, p1: f32, p2: f32) -> f32 {
        let inv = 1.0 - s;
        3.0 * inv * inv * p1 + 6.0 * inv * s * (p2 - p1) + 3.0 * s * s * (1.0 - p2)
    }

    /// newton first and bisection when the slope is flat or newton leaves 0 to 1
    fn solve(&self, target: f32, value: fn(&Self, f32) -> f32, slope: fn(&Self, f32) -> f32) -> f32 {
        let mut s = target;
        for _ in 0..8 {
            let error = value(self, s) - target;
            if error.abs() < 1e-12 {
                return s;
            }
            let d = slope(self, s);
            if d.abs() < 1e-6 {
                break;
            }
            let next = s - error / d;
            if !(0.0..=1.0).contains(&next) {
                break;
            }
            s = next;
        }

        let mut lower = 0.0_f32;
        let mut upper = 1.0_f32;
        s = target;
        for _ in 0..64 {
            let current = value(self, s);
            if (current - target).abs() < 1e-12 {
                return s;
            }
            if current < target {
                lower = s;
            } else {
                upper = s;
            }
            if upper - lower < 1e-12 {
                break;
            }
            s = (lower + upper) / 2.0;
        }
        s
    }
}

impl SharedElementCoverSchedule {
    /// linear opacity keyframes for a smoothstep on a cubic move, gives values and key times
    /// samples span start to 1, leading 0 key added when start is above 0
    /// start_progress clamps to 0..0.99, samples to at least 2
    pub fn opacity_keyframes(
        timing: SharedElementCubicTiming,
        start_progress: f32,
        samples: usize,
    ) -> (Vec<f32>, Vec<f32>) {
        let start = start_progress.max(0.0).min(0.99);
        let count = samples.max(2);
        let mut values = Vec::with_capacity(count + 1);
        let mut key_times = Vec::with_capacity(count + 1);

        if start > 0.0 {
            values.push(0.0);
            key_times.push(0.0);
        }
        for index in 0..count {
            let u = if index == count - 1 {
                1.0
            } else {
                start + (1.0 - start) * index as f32 / (count - 1) as f32
            };
            values.push(SharedElementGeometry::smoothstep((u - start) / (1.0 - start)));
            key_times.push(timing.time(u));
        }
        (values, key_times)
    }
}
