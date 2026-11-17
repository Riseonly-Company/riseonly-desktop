use std::f64::consts::{PI, TAU};
use std::time::Duration;

pub const DEFAULT_ANIMATION_DURATION: Duration = Duration::from_millis(350);

pub const DEFAULT_SPRING_RESPONSE: f64 = 0.55;
pub const DEFAULT_DAMPING_FRACTION: f64 = 0.825;

pub const DEFAULT_INTERACTIVE_RESPONSE: f64 = 0.15;
pub const DEFAULT_INTERACTIVE_DAMPING_FRACTION: f64 = 0.86;
pub const DEFAULT_INTERACTIVE_BLEND_DURATION: f64 = 0.25;

pub const SPRING_SETTLE_EPSILON: f64 = 0.001;

const SPRING_SETTLE_LIMIT: f64 = 60.0;

const SETTLE_SCAN_STEP: f64 = 0.001;
const SOLVE_EPSILON: f64 = 1e-6;
const SLOPE_STEP: f64 = 1e-4;
const MIN_RESPONSE: f64 = 1e-4;

/// seconds to a Duration, negative or nan or infinite means no time instead of a panic
pub fn duration_from_seconds(seconds: f64) -> Duration {
    if seconds.is_finite() && seconds > 0.0 {
        Duration::from_secs_f64(seconds)
    } else {
        Duration::ZERO
    }
}

/// unit cubic bezier timing curve
/// through 0 0 and 1 1 with two control points
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubicBezier(pub f64, pub f64, pub f64, pub f64);

impl CubicBezier {
    pub const EASE_IN_OUT: Self = Self(0.42, 0.0, 0.58, 1.0);
    pub const EASE_IN: Self = Self(0.42, 0.0, 1.0, 1.0);
    pub const EASE_OUT: Self = Self(0.0, 0.0, 0.58, 1.0);
    pub const LINEAR: Self = Self(0.0, 0.0, 1.0, 1.0);

    fn polynomial(p1: f64, p2: f64) -> (f64, f64, f64) {
        let c = 3.0 * p1;
        let b = 3.0 * (p2 - p1) - c;
        let a = 1.0 - c - b;
        (a, b, c)
    }

    fn sample((a, b, c): (f64, f64, f64), t: f64) -> f64 {
        ((a * t + b) * t + c) * t
    }

    fn sample_derivative((a, b, c): (f64, f64, f64), t: f64) -> f64 {
        (3.0 * a * t + 2.0 * b) * t + c
    }

    pub fn solve_parameter(&self, x: f64) -> f64 {
        let px = Self::polynomial(self.0, self.2);

        let mut t = x;
        for _ in 0..8 {
            let error = Self::sample(px, t) - x;
            if error.abs() < SOLVE_EPSILON {
                return t;
            }
            let derivative = Self::sample_derivative(px, t);
            if derivative.abs() < SOLVE_EPSILON {
                break;
            }
            t -= error / derivative;
        }

        let (mut low, mut high) = (0.0, 1.0);
        t = x.clamp(low, high);
        for _ in 0..64 {
            let sampled = Self::sample(px, t);
            if (sampled - x).abs() < SOLVE_EPSILON {
                break;
            }
            if x > sampled {
                low = t;
            } else {
                high = t;
            }
            t = low + (high - low) / 2.0;
        }
        t
    }

    /// progress at time fraction x, clamped to 0 to 1
    /// a custom curve may leave 0 to 1
    pub fn progress(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        Self::sample(Self::polynomial(self.1, self.3), self.solve_parameter(x))
    }

    /// slope of progress at x
    /// finite difference because the exact ratio is 0 over 0 at curve ends like ease out
    pub fn slope(&self, x: f64) -> f64 {
        let low = (x - SLOPE_STEP).max(0.0);
        let high = (x + SLOPE_STEP).min(1.0);
        if high <= low {
            return 0.0;
        }
        (self.progress(high) - self.progress(low)) / (high - low)
    }

    /// the curve as a gpui easing clamped to 0 to 1 since with_easing asserts it
    /// only for one shot motion that never gets interrupted, everything else uses Animated
    pub fn easing(self) -> impl Fn(f32) -> f32 {
        move |delta| self.progress(f64::from(delta)).clamp(0.0, 1.0) as f32
    }
}

/// spring with mass 1, stiffness from response and damping from damping fraction
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub response: f64,
    pub damping_fraction: f64,
    pub blend_duration: f64,
}

impl Spring {
    pub const fn new(response: f64, damping_fraction: f64) -> Self {
        Self {
            response,
            damping_fraction,
            blend_duration: 0.0,
        }
    }

    pub const MASS: f64 = 1.0;

    pub fn angular_frequency(&self) -> f64 {
        TAU / self.response.max(MIN_RESPONSE)
    }

    pub fn stiffness(&self) -> f64 {
        self.angular_frequency().powi(2)
    }

    pub fn damping(&self) -> f64 {
        4.0 * PI * self.damping_fraction.max(0.0) / self.response.max(MIN_RESPONSE)
    }

    /// displacement from target and velocity after t seconds, closed form damped oscillator
    pub fn state(&self, x0: f64, v0: f64, t: f64) -> (f64, f64) {
        let omega = self.angular_frequency();
        let zeta = self.damping_fraction.max(0.0);

        if (zeta - 1.0).abs() < 1e-9 {
            let decay = (-omega * t).exp();
            let b = v0 + omega * x0;
            return (decay * (x0 + b * t), decay * (v0 - omega * b * t));
        }

        if zeta < 1.0 {
            let sigma = zeta * omega;
            let omega_d = omega * (1.0 - zeta * zeta).sqrt();
            let decay = (-sigma * t).exp();
            let a = x0;
            let b = (v0 + sigma * x0) / omega_d;
            let (sin, cos) = (omega_d * t).sin_cos();
            let x = decay * (a * cos + b * sin);
            let v = decay * ((b * omega_d - sigma * a) * cos - (sigma * b + a * omega_d) * sin);
            return (x, v);
        }

        let root = (zeta * zeta - 1.0).sqrt();
        let r1 = -omega * (zeta - root);
        let r2 = -omega * (zeta + root);
        let c2 = (v0 - r1 * x0) / (r2 - r1);
        let c1 = x0 - c2;
        let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
        (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
    }

    /// time after which displacement stays below epsilon forever
    /// energy never grows under damping, so once it is small enough the spring cant swing back out
    pub fn settling_time(&self, x0: f64, v0: f64, epsilon: f64) -> f64 {
        let omega = self.angular_frequency();
        let bound = |x: f64, v: f64| (v * v + omega * omega * x * x).sqrt() / omega;
        let displacement = |t: f64| self.state(x0, v0, t).0.abs();

        let mut contained = 0.0;
        loop {
            let (x, v) = self.state(x0, v0, contained);
            if bound(x, v) < epsilon {
                break;
            }
            contained += SETTLE_SCAN_STEP;
            if contained >= SPRING_SETTLE_LIMIT {
                return SPRING_SETTLE_LIMIT;
            }
        }

        let mut inside = contained;
        while inside > 0.0 {
            let outside = (inside - SETTLE_SCAN_STEP).max(0.0);
            if displacement(outside) >= epsilon {
                let (mut low, mut high) = (outside, inside);
                for _ in 0..40 {
                    let middle = (low + high) / 2.0;
                    if displacement(middle) >= epsilon {
                        low = middle;
                    } else {
                        high = middle;
                    }
                }
                return high;
            }
            inside = outside;
        }
        0.0
    }

    /// how long a unit move from rest takes to settle
    pub fn settling_duration(&self) -> Duration {
        duration_from_seconds(self.settling_time(1.0, 0.0, SPRING_SETTLE_EPSILON))
    }

    /// spring blend of the way from self to the other, how a replaced spring blends
    pub fn blended(&self, to: &Spring, blend: f64) -> Spring {
        let blend = blend.clamp(0.0, 1.0);
        Spring {
            response: self.response + (to.response - self.response) * blend,
            damping_fraction: self.damping_fraction + (to.damping_fraction - self.damping_fraction) * blend,
            blend_duration: to.blend_duration,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Timing {
    Curve { curve: CubicBezier, duration: Duration },
    Spring(Spring),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Once,
    Count { count: u32, autoreverses: bool },
    Forever { autoreverses: bool },
}

/// animation value, timing plus delay and repeat count or forever
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionSpec {
    pub timing: Timing,
    pub delay: Duration,
    pub repeat: Repeat,
}

impl MotionSpec {
    pub const fn curve(curve: CubicBezier, duration: Duration) -> Self {
        Self {
            timing: Timing::Curve { curve, duration },
            delay: Duration::ZERO,
            repeat: Repeat::Once,
        }
    }

    pub const fn spring(spring: Spring) -> Self {
        Self {
            timing: Timing::Spring(spring),
            delay: Duration::ZERO,
            repeat: Repeat::Once,
        }
    }

    pub const EASE_IN_OUT: Self = Self::curve(CubicBezier::EASE_IN_OUT, DEFAULT_ANIMATION_DURATION);
    pub const EASE_IN: Self = Self::curve(CubicBezier::EASE_IN, DEFAULT_ANIMATION_DURATION);
    pub const EASE_OUT: Self = Self::curve(CubicBezier::EASE_OUT, DEFAULT_ANIMATION_DURATION);
    pub const LINEAR: Self = Self::curve(CubicBezier::LINEAR, DEFAULT_ANIMATION_DURATION);

    pub const SPRING: Self = Self::spring(Spring::new(0.5, 1.0));
    pub const SMOOTH: Self = Self::spring(Spring::new(0.5, 1.0));
    pub const SNAPPY: Self = Self::spring(Spring::new(0.5, 0.85));
    pub const BOUNCY: Self = Self::spring(Spring::new(0.5, 0.7));
    pub const INTERACTIVE_SPRING: Self = Self::spring(Spring {
        response: DEFAULT_INTERACTIVE_RESPONSE,
        damping_fraction: DEFAULT_INTERACTIVE_DAMPING_FRACTION,
        blend_duration: DEFAULT_INTERACTIVE_BLEND_DURATION,
    });

    pub fn delay(mut self, seconds: f64) -> Self {
        self.delay = duration_from_seconds(seconds);
        self
    }

    /// repeats count times, optionally reversing
    pub fn repeat_count(mut self, count: u32, autoreverses: bool) -> Self {
        self.repeat = Repeat::Count { count, autoreverses };
        self
    }

    /// repeats forever, only for an indicator that is visible and stops when it leaves
    pub fn repeat_forever(mut self, autoreverses: bool) -> Self {
        self.repeat = Repeat::Forever { autoreverses };
        self
    }

    pub fn as_spring(&self) -> Option<Spring> {
        match self.timing {
            Timing::Spring(spring) => Some(spring),
            Timing::Curve { .. } => None,
        }
    }

    pub fn is_spring(&self) -> bool {
        self.as_spring().is_some()
    }
}

pub fn ease_in_out(duration: f64) -> MotionSpec {
    MotionSpec::curve(CubicBezier::EASE_IN_OUT, duration_from_seconds(duration))
}

pub fn ease_in(duration: f64) -> MotionSpec {
    MotionSpec::curve(CubicBezier::EASE_IN, duration_from_seconds(duration))
}

pub fn ease_out(duration: f64) -> MotionSpec {
    MotionSpec::curve(CubicBezier::EASE_OUT, duration_from_seconds(duration))
}

pub fn linear(duration: f64) -> MotionSpec {
    MotionSpec::curve(CubicBezier::LINEAR, duration_from_seconds(duration))
}

/// cubic bezier timing curve with duration
pub fn timing_curve(x1: f64, y1: f64, x2: f64, y2: f64, duration: f64) -> MotionSpec {
    MotionSpec::curve(CubicBezier(x1, y1, x2, y2), duration_from_seconds(duration))
}

pub fn spring(response: f64, damping_fraction: f64) -> MotionSpec {
    MotionSpec::spring(Spring::new(response, damping_fraction))
}

pub fn spring_blended(response: f64, damping_fraction: f64, blend_duration: f64) -> MotionSpec {
    MotionSpec::spring(Spring {
        response,
        damping_fraction,
        blend_duration: blend_duration.max(0.0),
    })
}

pub fn interactive_spring(response: f64, damping_fraction: f64, blend_duration: f64) -> MotionSpec {
    spring_blended(response, damping_fraction, blend_duration)
}

/// spring from duration and bounce, negative bounce is overdamped
pub fn spring_bounce(duration: f64, bounce: f64) -> MotionSpec {
    let damping_fraction = if bounce >= 0.0 { 1.0 - bounce } else { 1.0 / (1.0 + bounce).max(MIN_RESPONSE) };
    spring(duration, damping_fraction)
}

pub fn snappy(duration: f64) -> MotionSpec {
    spring_bounce(duration, 0.15)
}

pub fn smooth(duration: f64) -> MotionSpec {
    spring_bounce(duration, 0.0)
}

pub fn bouncy(duration: f64) -> MotionSpec {
    spring_bounce(duration, 0.3)
}
