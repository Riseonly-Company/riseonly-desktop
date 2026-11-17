/// which corner the video watermark is in at a moment, and how visible
/// pure timing, clip splits into stretches of about 5 s and at least two
/// each stretch has a corner, mark fades only between them
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostWatermarkSchedule {
    pub seconds: f64,
    pub count: usize,
    pub span: f64,
    edge: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostWatermarkScheduleCorner {
    TopLeft,
    BottomRight,
    TopRight,
    BottomLeft,
}

impl PostWatermarkScheduleCorner {
    pub const ALL_CASES: [PostWatermarkScheduleCorner; 4] = [
        PostWatermarkScheduleCorner::TopLeft,
        PostWatermarkScheduleCorner::BottomRight,
        PostWatermarkScheduleCorner::TopRight,
        PostWatermarkScheduleCorner::BottomLeft,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostWatermarkScheduleMoment {
    pub corner: PostWatermarkScheduleCorner,
    pub opacity: f64,
}

impl PostWatermarkSchedule {
    pub const STRETCH: f64 = 5.0;
    pub const FADE: f64 = 0.25;
    pub const SHORTEST: f64 = 1.0;

    pub fn new(seconds: f64) -> Self {
        let length = if seconds.is_finite() { seconds.max(0.1) } else { 0.1 };
        let count = if length < Self::SHORTEST {
            1
        } else {
            ((length / Self::STRETCH).round() as usize).max(2)
        };
        let span = length / count as f64;
        Self { seconds: length, count, span, edge: Self::FADE.min(span / 4.0) }
    }

    pub fn corner(&self, index: usize) -> PostWatermarkScheduleCorner {
        let order = PostWatermarkScheduleCorner::ALL_CASES;
        if self.count > 2 && index == self.count - 1 && index % order.len() == 0 {
            return PostWatermarkScheduleCorner::TopRight;
        }
        order[index % order.len()]
    }

    pub fn moment(&self, time: f64) -> PostWatermarkScheduleMoment {
        let t = (if time.is_finite() { time } else { 0.0 }).max(0.0).min(self.seconds);
        let index = ((t / self.span) as usize).min(self.count - 1);
        let start = index as f64 * self.span;
        let end = if index == self.count - 1 { self.seconds } else { start + self.span };
        let fade_in = if index > 0 { (t - start) / self.edge } else { 1.0 };
        let fade_out = if index < self.count - 1 { (end - t) / self.edge } else { 1.0 };
        PostWatermarkScheduleMoment {
            corner: self.corner(index),
            opacity: fade_in.min(fade_out).max(0.0).min(1.0),
        }
    }

    /// fade length at each stretch edge, the export seam fades the badge over it
    pub fn edge(&self) -> f64 {
        self.edge
    }
}
