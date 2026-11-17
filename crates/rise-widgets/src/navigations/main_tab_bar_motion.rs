use std::f64::consts::PI;

use rise_ui::swiftui::motion::{MotionSpec, ease_out, spring};

/// seconds on the monotonic clock
pub type CFTimeInterval = f64;

/// how one change of the bar moves
/// the bar is drawn from trajectories worked out ahead
/// so a motion plays to its end even while a new page keeps the main thread busy
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MainTabBarMotion {
    Immediate,
    Spring { response: f64, damping: f64 },
    EaseOut { duration: f64 },
}

impl MainTabBarMotion {
    /// same motion for what the views still draw beside the bar
    pub fn animation(&self) -> Option<MotionSpec> {
        match *self {
            Self::Immediate => None,
            Self::Spring { response, damping } => Some(spring(response, damping)),
            Self::EaseOut { duration } => Some(ease_out(duration)),
        }
    }

    pub fn logical_duration(&self) -> CFTimeInterval {
        match *self {
            Self::Immediate => 0.0,
            Self::Spring { response, damping } => {
                let decay = MainTabBarSpring::new(response, damping).decay();
                100f64.ln() / decay
            }
            Self::EaseOut { duration } => duration,
        }
    }
}

/// damped spring of unit mass
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MainTabBarSpring {
    pub omega: f64,
    pub zeta: f64,
}

impl MainTabBarSpring {
    pub fn new(response: f64, damping: f64) -> Self {
        Self {
            omega: 2.0 * PI / response.max(0.001),
            zeta: damping.clamp(0.01, 0.999),
        }
    }

    pub fn decay(&self) -> f64 {
        self.zeta * self.omega
    }

    pub fn damped_omega(&self) -> f64 {
        self.omega * (1.0 - self.zeta * self.zeta).sqrt()
    }

    /// offset from the target and its velocity, time after it started at offset with velocity
    pub fn state(&self, offset: f64, velocity: f64, time: f64) -> (f64, f64) {
        let wd = self.damped_omega();
        let decay = self.decay();
        let b = (velocity + decay * offset) / wd;
        let envelope = (-decay * time).exp();
        let c = (wd * time).cos();
        let s = (wd * time).sin();
        (
            envelope * (offset * c + b * s),
            envelope * ((-decay * offset + wd * b) * c + (-decay * b - wd * offset) * s),
        )
    }

    /// time until the swing stays within tolerance of the target
    pub fn settle_time(&self, offset: f64, velocity: f64, tolerance: f64) -> f64 {
        let b = (velocity + self.decay() * offset) / self.damped_omega();
        let amplitude = (offset * offset + b * b).sqrt();
        if amplitude <= tolerance {
            return 0.0;
        }
        (amplitude / tolerance).ln() / self.decay()
    }
}

pub enum MainTabBarEaseOut {}

impl MainTabBarEaseOut {
    const X1: f64 = 0.0;
    const Y1: f64 = 0.0;
    const X2: f64 = 0.58;
    const Y2: f64 = 1.0;

    /// progress and its derivative over u in 0 to 1
    pub fn progress(u: f64) -> (f64, f64) {
        let u = u.clamp(0.0, 1.0);
        let mut t = u;
        for _ in 0..8 {
            let x = Self::bezier(t, Self::X1, Self::X2) - u;
            let dx = Self::bezier_slope(t, Self::X1, Self::X2);
            if x.abs() <= 1e-7 || dx.abs() <= 1e-7 {
                break;
            }
            t = (t - x / dx).clamp(0.0, 1.0);
        }
        let dx = Self::bezier_slope(t, Self::X1, Self::X2);
        let dy = Self::bezier_slope(t, Self::Y1, Self::Y2);
        (Self::bezier(t, Self::Y1, Self::Y2), if dx > 1e-7 { dy / dx } else { 0.0 })
    }

    fn bezier(t: f64, p1: f64, p2: f64) -> f64 {
        let m = 1.0 - t;
        3.0 * m * m * t * p1 + 3.0 * m * t * t * p2 + t * t * t
    }

    fn bezier_slope(t: f64, p1: f64, p2: f64) -> f64 {
        let m = 1.0 - t;
        3.0 * m * m * p1 + 6.0 * m * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Segment {
    start: CFTimeInterval,
    from: f64,
    velocity: f64,
    to: f64,
    motion: MainTabBarMotion,
}

/// one value of the bar over time, what it is and how fast it moves at any moment
/// a new motion starts from where the old one is at its speed, so nothing jumps or stops dead
#[derive(Clone, Debug, PartialEq)]
pub struct MainTabBarTrack {
    pub tolerance: f64,
    segments: Vec<Segment>,
}

impl MainTabBarTrack {
    pub fn new(value: f32, tolerance: f64) -> Self {
        let value = f64::from(value);
        Self {
            tolerance,
            segments: vec![Segment { start: 0.0, from: value, velocity: 0.0, to: value, motion: MainTabBarMotion::Immediate }],
        }
    }

    fn last(&self) -> &Segment {
        &self.segments[self.segments.len() - 1]
    }

    pub fn target(&self) -> f32 {
        self.last().to as f32
    }

    pub fn value(&self, time: CFTimeInterval) -> f32 {
        self.state(time).0 as f32
    }

    /// when the last motion comes to rest, chained ones included
    pub fn settle_time(&self) -> CFTimeInterval {
        let last = self.last();
        last.start + Self::duration(last, self.tolerance)
    }

    /// replaces whatever was under way or chained
    pub fn r#move(&mut self, value: f32, motion: MainTabBarMotion, time: CFTimeInterval) {
        let (from, velocity) = self.state(time);
        self.segments = vec![Segment { start: time, from, velocity, to: f64::from(value), motion }];
    }

    /// starts at time from where the motion before it is by then
    pub fn chain(&mut self, value: f32, motion: MainTabBarMotion, time: CFTimeInterval) {
        let value = f64::from(value);
        self.segments.retain(|segment| segment.start <= time);
        if self.segments.is_empty() {
            self.segments =
                vec![Segment { start: time, from: value, velocity: 0.0, to: value, motion: MainTabBarMotion::Immediate }];
        }
        let (from, velocity) = self.state(time);
        self.segments.push(Segment { start: time, from, velocity, to: value, motion });
    }

    /// drops the motions chained after time, the one under way runs to its target
    pub fn cancel_chain(&mut self, time: CFTimeInterval) {
        let kept: Vec<Segment> = self.segments.iter().copied().filter(|segment| segment.start <= time).collect();
        if !kept.is_empty() {
            self.segments = kept;
        }
    }

    fn state(&self, time: CFTimeInterval) -> (f64, f64) {
        let segment = self.segments.iter().rev().find(|segment| segment.start <= time).unwrap_or(&self.segments[0]);
        Self::sample(segment, (time - segment.start).max(0.0))
    }

    fn sample(segment: &Segment, elapsed: f64) -> (f64, f64) {
        match segment.motion {
            MainTabBarMotion::Immediate => (segment.to, 0.0),
            MainTabBarMotion::EaseOut { duration } => {
                if duration <= 0.0 || elapsed >= duration {
                    return (segment.to, 0.0);
                }
                let (value, slope) = MainTabBarEaseOut::progress(elapsed / duration);
                let travel = segment.to - segment.from;
                (segment.from + travel * value, travel * slope / duration)
            }
            MainTabBarMotion::Spring { response, damping } => {
                let spring = MainTabBarSpring::new(response, damping);
                let (offset, velocity) = spring.state(segment.from - segment.to, segment.velocity, elapsed);
                (segment.to + offset, velocity)
            }
        }
    }

    fn duration(segment: &Segment, tolerance: f64) -> f64 {
        match segment.motion {
            MainTabBarMotion::Immediate => 0.0,
            MainTabBarMotion::EaseOut { duration } => duration,
            MainTabBarMotion::Spring { response, damping } => {
                let spring = MainTabBarSpring::new(response, damping);
                spring.settle_time(segment.from - segment.to, segment.velocity, tolerance).min(3.0)
            }
        }
    }
}
