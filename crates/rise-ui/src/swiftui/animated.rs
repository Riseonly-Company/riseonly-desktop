use std::time::Duration;

use gpui::{px, App, Bounds, Corners, Hsla, Pixels, Point, Rgba, Size, Window};
use smallvec::SmallVec;

use super::motion::{duration_from_seconds, MotionSpec, Repeat, Spring, Timing};

/// monotonic time from some arbitrary start
pub type MotionTime = Duration;

/// animatable vector of a value
pub type Components = SmallVec<[f32; 8]>;

const BLEND_STEP: f64 = 1.0 / 600.0;

fn component(components: &[f32], index: usize) -> f32 {
    components.get(index).copied().unwrap_or(0.0)
}

/// value that can be animated, maps to a vector and back
pub trait Lerp: Clone {
    fn components(&self) -> Components;

    fn from_components(components: &[f32]) -> Self;

    fn resting_threshold() -> f32 {
        0.001
    }

    /// t can go outside 0 to 1, springs overshoot
    fn lerp(&self, to: &Self, t: f32) -> Self {
        let from = self.components();
        let to = to.components();
        let mixed: Components = from
            .iter()
            .enumerate()
            .map(|(index, start)| start + (component(&to, index) - start) * t)
            .collect();
        Self::from_components(&mixed)
    }
}

impl Lerp for f32 {
    fn components(&self) -> Components {
        smallvec::smallvec![*self]
    }

    fn from_components(components: &[f32]) -> Self {
        component(components, 0)
    }
}

impl Lerp for f64 {
    fn components(&self) -> Components {
        smallvec::smallvec![*self as f32]
    }

    fn from_components(components: &[f32]) -> Self {
        f64::from(component(components, 0))
    }
}

impl Lerp for Pixels {
    fn components(&self) -> Components {
        smallvec::smallvec![self.as_f32()]
    }

    fn from_components(components: &[f32]) -> Self {
        px(component(components, 0))
    }

    fn resting_threshold() -> f32 {
        0.01
    }
}

impl Lerp for Point<Pixels> {
    fn components(&self) -> Components {
        smallvec::smallvec![self.x.as_f32(), self.y.as_f32()]
    }

    fn from_components(components: &[f32]) -> Self {
        Point {
            x: px(component(components, 0)),
            y: px(component(components, 1)),
        }
    }

    fn resting_threshold() -> f32 {
        0.01
    }
}

impl Lerp for Size<Pixels> {
    fn components(&self) -> Components {
        smallvec::smallvec![self.width.as_f32(), self.height.as_f32()]
    }

    fn from_components(components: &[f32]) -> Self {
        Size {
            width: px(component(components, 0)),
            height: px(component(components, 1)),
        }
    }

    fn resting_threshold() -> f32 {
        0.01
    }
}

impl Lerp for Bounds<Pixels> {
    fn components(&self) -> Components {
        smallvec::smallvec![
            self.origin.x.as_f32(),
            self.origin.y.as_f32(),
            self.size.width.as_f32(),
            self.size.height.as_f32(),
        ]
    }

    fn from_components(components: &[f32]) -> Self {
        Bounds {
            origin: Point {
                x: px(component(components, 0)),
                y: px(component(components, 1)),
            },
            size: Size {
                width: px(component(components, 2)),
                height: px(component(components, 3)),
            },
        }
    }

    fn resting_threshold() -> f32 {
        0.01
    }
}

impl Lerp for Corners<Pixels> {
    fn components(&self) -> Components {
        smallvec::smallvec![
            self.top_left.as_f32(),
            self.top_right.as_f32(),
            self.bottom_right.as_f32(),
            self.bottom_left.as_f32(),
        ]
    }

    fn from_components(components: &[f32]) -> Self {
        Corners {
            top_left: px(component(components, 0)),
            top_right: px(component(components, 1)),
            bottom_right: px(component(components, 2)),
            bottom_left: px(component(components, 3)),
        }
    }

    fn resting_threshold() -> f32 {
        0.01
    }
}

impl Lerp for Hsla {
    fn components(&self) -> Components {
        let rgba = Rgba::from(*self);
        smallvec::smallvec![rgba.r, rgba.g, rgba.b, rgba.a]
    }

    fn from_components(components: &[f32]) -> Self {
        let channel = |index| component(components, index).clamp(0.0, 1.0);
        Hsla::from(Rgba {
            r: channel(0),
            g: channel(1),
            b: channel(2),
            a: channel(3),
        })
    }

    fn resting_threshold() -> f32 {
        1.0 / 1024.0
    }
}

/// spring that replaced a running one, params blend in and the end state is integrated once on retarget
#[derive(Clone, Debug, PartialEq)]
struct Blend {
    from: Spring,
    duration: f64,
    displacement: Components,
    velocity: Components,
}

/// animated value, keep it in a view or in use_keyed_state
/// read frame_value in render so frames are only asked while it moves
#[derive(Clone, Debug)]
pub struct Animated<T: Lerp> {
    from: Components,
    to: T,
    target: Components,
    start: MotionTime,
    spec: Option<MotionSpec>,
    velocity: Components,
    end: Option<MotionTime>,
    blend: Option<Blend>,
}

impl<T: Lerp> Animated<T> {
    pub fn new(value: T) -> Self {
        let target = value.components();
        Self {
            from: target.clone(),
            velocity: zeros(target.len()),
            to: value,
            target,
            start: Duration::ZERO,
            spec: None,
            end: Some(Duration::ZERO),
            blend: None,
        }
    }

    /// value the motion is heading to
    pub fn target(&self) -> &T {
        &self.to
    }

    pub fn spec(&self) -> Option<MotionSpec> {
        self.spec
    }

    /// true while frames are still needed at now
    pub fn is_running(&self, now: MotionTime) -> bool {
        self.spec.is_some() && self.end.is_none_or(|end| now < end)
    }

    pub fn value(&self, now: MotionTime) -> T {
        T::from_components(&self.sample(now).0)
    }

    /// velocity on screen at now, per component and second
    pub fn velocity(&self, now: MotionTime) -> Components {
        self.sample(now).1
    }

    /// value for this frame, asks for the next frame only while moving
    #[track_caller]
    pub fn frame_value(&self, now: MotionTime, window: &Window) -> T {
        if self.is_running(now) {
            window.request_animation_frame();
        }
        self.value(now)
    }

    /// jumps straight to target, no animation
    pub fn jump(&mut self, target: T) {
        *self = Self::new(target);
    }

    /// sets target with a spec, respects reduce motion
    pub fn set_in(&mut self, target: T, spec: impl Into<Option<MotionSpec>>, now: MotionTime, cx: &App) {
        self.set(target, spec, now, cx.reduce_motion());
    }

    /// retargets from what is on screen, springs keep velocity and curves start from rest
    /// reduce_motion or no spec means it lands at once, same target again does nothing
    pub fn set(&mut self, target: T, spec: impl Into<Option<MotionSpec>>, now: MotionTime, reduce_motion: bool) {
        let target_components = target.components();
        if target_components == self.target {
            return;
        }
        let spec = match spec.into() {
            Some(spec) if !reduce_motion => spec,
            _ => {
                self.jump(target);
                return;
            }
        };

        let (current, current_velocity) = self.sample(now);
        let previous_spring = self
            .spec
            .filter(|_| self.is_running(now))
            .and_then(|previous| previous.as_spring());

        self.from = current;
        self.velocity = match spec.timing {
            Timing::Spring(_) if spec.repeat == Repeat::Once => current_velocity,
            _ => zeros(self.from.len()),
        };
        self.to = target;
        self.target = target_components;
        self.start = now;
        self.spec = Some(spec);
        self.blend = None;

        let active = match spec.timing {
            Timing::Curve { duration, .. } => cycles_length(spec.repeat, duration.as_secs_f64()),
            Timing::Spring(spring) => match spec.repeat {
                Repeat::Once => Some(self.prepare_spring_run(spring, previous_spring)),
                repeat => cycles_length(repeat, spring.settling_time(1.0, 0.0, f64::from(T::resting_threshold()))),
            },
        };
        self.end = active.map(|seconds| now + spec.delay + duration_from_seconds(seconds));
    }

    fn prepare_spring_run(&mut self, spring: Spring, previous: Option<Spring>) -> f64 {
        let displacement: Components = self
            .from
            .iter()
            .enumerate()
            .map(|(index, value)| value - component(&self.target, index))
            .collect();
        let epsilon = f64::from(T::resting_threshold());

        let blending_from = previous.filter(|previous| spring.blend_duration > 0.0 && *previous != spring);
        let (settle_from, settle_velocity, offset) = match blending_from {
            Some(from) => {
                let (x, v) = integrate_blend(from, spring, spring.blend_duration, &displacement, &self.velocity, spring.blend_duration);
                self.blend = Some(Blend {
                    from,
                    duration: spring.blend_duration,
                    displacement: x.clone(),
                    velocity: v.clone(),
                });
                (x, v, spring.blend_duration)
            }
            None => (displacement, self.velocity.clone(), 0.0),
        };

        let settle = settle_from
            .iter()
            .zip(settle_velocity.iter())
            .map(|(x, v)| spring.settling_time(f64::from(*x), f64::from(*v), epsilon))
            .fold(0.0, f64::max);
        offset + settle
    }

    /// position and velocity at now
    fn sample(&self, now: MotionTime) -> (Components, Components) {
        let rest = || (self.target.clone(), zeros(self.target.len()));
        let Some(spec) = self.spec else {
            return rest();
        };
        if !self.is_running(now) {
            return rest();
        }

        let elapsed = now.saturating_sub(self.start);
        if elapsed < spec.delay {
            return (self.from.clone(), zeros(self.from.len()));
        }
        let t = (elapsed - spec.delay).as_secs_f64();

        match spec.timing {
            Timing::Curve { curve, duration } => {
                let length = duration.as_secs_f64();
                if length <= 0.0 {
                    return rest();
                }
                let (local, reversed) = cycle_position(spec.repeat, t, length);
                let progress = curve.progress(local) as f32;
                let slope = (curve.slope(local) / length) as f32;
                let (a, b) = if reversed { (&self.target, &self.from) } else { (&self.from, &self.target) };
                let position = a.iter().enumerate().map(|(i, s)| s + (component(b, i) - s) * progress).collect();
                let velocity = a.iter().enumerate().map(|(i, s)| (component(b, i) - s) * slope).collect();
                (position, velocity)
            }
            Timing::Spring(spring) if spec.repeat != Repeat::Once => {
                let length = spring.settling_time(1.0, 0.0, f64::from(T::resting_threshold()));
                if length <= 0.0 {
                    return rest();
                }
                let (local, reversed) = cycle_position(spec.repeat, t, length);
                let (x, v) = spring.state(1.0, 0.0, local * length);
                let (a, b) = if reversed { (&self.target, &self.from) } else { (&self.from, &self.target) };
                let position = a
                    .iter()
                    .enumerate()
                    .map(|(i, s)| component(b, i) + (s - component(b, i)) * x as f32)
                    .collect();
                let velocity = a
                    .iter()
                    .enumerate()
                    .map(|(i, s)| (s - component(b, i)) * v as f32)
                    .collect();
                (position, velocity)
            }
            Timing::Spring(spring) => {
                let (displacement, velocity) = match &self.blend {
                    Some(blend) if t < blend.duration => {
                        let start: Components = self
                            .from
                            .iter()
                            .enumerate()
                            .map(|(i, s)| s - component(&self.target, i))
                            .collect();
                        integrate_blend(blend.from, spring, blend.duration, &start, &self.velocity, t)
                    }
                    Some(blend) => evolve(spring, &blend.displacement, &blend.velocity, t - blend.duration),
                    None => {
                        let start: Components = self
                            .from
                            .iter()
                            .enumerate()
                            .map(|(i, s)| s - component(&self.target, i))
                            .collect();
                        evolve(spring, &start, &self.velocity, t)
                    }
                };
                let position = self
                    .target
                    .iter()
                    .enumerate()
                    .map(|(i, s)| s + component(&displacement, i))
                    .collect();
                (position, velocity)
            }
        }
    }
}

fn zeros(len: usize) -> Components {
    smallvec::smallvec![0.0; len]
}

/// seconds the motion stays active, none for forever repeat
fn cycles_length(repeat: Repeat, cycle: f64) -> Option<f64> {
    match repeat {
        Repeat::Once => Some(cycle),
        Repeat::Count { count, .. } => Some(cycle * f64::from(count.max(1))),
        Repeat::Forever { .. } => None,
    }
}

/// fraction of the current cycle at t and whether it runs backwards
fn cycle_position(repeat: Repeat, t: f64, length: f64) -> (f64, bool) {
    let raw = t / length;
    match repeat {
        Repeat::Once => (raw.min(1.0), false),
        Repeat::Count { autoreverses, .. } | Repeat::Forever { autoreverses } => {
            let cycle = raw.floor();
            let local = raw - cycle;
            let odd = (cycle as u64) % 2 == 1;
            (local, autoreverses && odd)
        }
    }
}

/// closed form spring applied to every component
fn evolve(spring: Spring, displacement: &[f32], velocity: &[f32], t: f64) -> (Components, Components) {
    let (a, a_velocity) = spring.state(1.0, 0.0, t);
    let (b, b_velocity) = spring.state(0.0, 1.0, t);
    let position = displacement
        .iter()
        .enumerate()
        .map(|(i, x)| (a * f64::from(*x) + b * f64::from(component(velocity, i))) as f32)
        .collect();
    let speed = displacement
        .iter()
        .enumerate()
        .map(|(i, x)| (a_velocity * f64::from(*x) + b_velocity * f64::from(component(velocity, i))) as f32)
        .collect();
    (position, speed)
}

/// rk4 over the blend, stiffness and damping move from one spring to the other over duration
fn integrate_blend(
    from: Spring,
    to: Spring,
    duration: f64,
    displacement: &[f32],
    velocity: &[f32],
    until: f64,
) -> (Components, Components) {
    let until = until.clamp(0.0, duration);
    let steps = (until / BLEND_STEP).ceil().max(1.0) as usize;
    let dt = until / steps as f64;
    let acceleration = |time: f64, x: f64, v: f64| {
        let spring = from.blended(&to, if duration > 0.0 { time / duration } else { 1.0 });
        -spring.stiffness() * x - spring.damping() * v
    };

    let mut positions = Components::new();
    let mut speeds = Components::new();
    for (i, start) in displacement.iter().enumerate() {
        let (mut x, mut v) = (f64::from(*start), f64::from(component(velocity, i)));
        let mut time = 0.0;
        for _ in 0..steps {
            let k1x = v;
            let k1v = acceleration(time, x, v);
            let k2x = v + k1v * dt / 2.0;
            let k2v = acceleration(time + dt / 2.0, x + k1x * dt / 2.0, k2x);
            let k3x = v + k2v * dt / 2.0;
            let k3v = acceleration(time + dt / 2.0, x + k2x * dt / 2.0, k3x);
            let k4x = v + k3v * dt;
            let k4v = acceleration(time + dt, x + k3x * dt, k4x);
            x += dt / 6.0 * (k1x + 2.0 * k2x + 2.0 * k3x + k4x);
            v += dt / 6.0 * (k1v + 2.0 * k2v + 2.0 * k3v + k4v);
            time += dt;
        }
        positions.push(x as f32);
        speeds.push(v as f32);
    }
    (positions, speeds)
}
