use crate::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BottomSheetInteractionStyle {
    Standard,
    TelegramWebApp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BottomSheetReleaseDecision {
    Dismiss,
    Snap { index: usize },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BottomSheetSnapAnimationCurve {
    EaseOut,
    EaseInOut,
    TelegramSpring { initial_velocity: f32 },
}

impl BottomSheetSnapAnimationCurve {
    pub fn solve(&self, linear_progress: f32, duration: f64) -> f32 {
        let progress = if linear_progress.is_nan() { 0.0 } else { linear_progress.clamp(0.0, 1.0) };
        if progress >= 1.0 {
            return 1.0;
        }

        match *self {
            Self::EaseOut => {
                let remaining = 1.0 - progress;
                1.0 - remaining * remaining * remaining
            }
            Self::EaseInOut => progress * progress * (3.0 - 2.0 * progress),
            Self::TelegramSpring { initial_velocity } => {
                let mass = 5.0_f64;
                let stiffness = 900.0_f64;
                let damping = 124.0_f64;
                let decay = damping / (2.0 * mass);
                let damped_frequency = (stiffness / mass - decay * decay).sqrt();
                let time = f64::from(progress) * duration.max(0.01);
                let velocity = f64::from(initial_velocity.max(0.0));
                let displacement = (-decay * time).exp()
                    * (-(damped_frequency * time).cos()
                        + ((velocity - decay) / damped_frequency) * (damped_frequency * time).sin());
                (1.0 + displacement) as f32
            }
        }
    }
}

/// namespace of the release rules
pub enum BottomSheetReleasePolicy {}

impl BottomSheetReleasePolicy {
    pub fn resolve(
        style: BottomSheetInteractionStyle,
        detents: &[f32],
        current_index: usize,
        resting_height: f32,
        translation_y: f32,
        velocity_y: f32,
    ) -> BottomSheetReleaseDecision {
        if detents.is_empty() {
            return BottomSheetReleaseDecision::Snap { index: 0 };
        }

        let index = current_index.min(detents.len() - 1);
        let translation = if translation_y.is_finite() { translation_y } else { 0.0 };
        let velocity = if velocity_y.is_finite() { velocity_y } else { 0.0 };

        match style {
            BottomSheetInteractionStyle::Standard => {
                Self::standard_decision(detents, index, resting_height, translation, velocity)
            }
            BottomSheetInteractionStyle::TelegramWebApp => {
                Self::telegram_web_app_decision(detents, index, resting_height, translation, velocity)
            }
        }
    }

    fn standard_decision(
        detents: &[f32],
        current_index: usize,
        resting_height: f32,
        translation_y: f32,
        velocity_y: f32,
    ) -> BottomSheetReleaseDecision {
        let minimum_height = detents[0];
        let dragged_down = translation_y > BottomSheetMetrics::MIN_DRAG_FOR_VELOCITY_DISMISS;
        let visible_height = resting_height - translation_y;
        let projected_height = visible_height - velocity_y * 0.12;

        if dragged_down {
            if visible_height < minimum_height * BottomSheetMetrics::DISMISS_FRACTION {
                return BottomSheetReleaseDecision::Dismiss;
            }
            if velocity_y > BottomSheetMetrics::FLICK_VELOCITY
                && (current_index == 0 || projected_height < minimum_height)
            {
                return BottomSheetReleaseDecision::Dismiss;
            }
        }

        let distance = |index: usize| (detents[index] - projected_height).abs();
        let mut target_index = 0;
        for candidate in 1..detents.len() {
            if distance(candidate) < distance(target_index) {
                target_index = candidate;
            }
        }
        BottomSheetReleaseDecision::Snap { index: target_index }
    }

    fn telegram_web_app_decision(
        detents: &[f32],
        current_index: usize,
        resting_height: f32,
        translation_y: f32,
        velocity_y: f32,
    ) -> BottomSheetReleaseDecision {
        let downward_distance = translation_y.max(0.0);
        let upward_distance = (-translation_y).max(0.0);
        let visible_height = resting_height - translation_y;

        if downward_distance > 0.0
            && visible_height < detents[0] * BottomSheetMetrics::DISMISS_FRACTION
        {
            return BottomSheetReleaseDecision::Dismiss;
        }

        if current_index == 0 {
            if downward_distance > 0.0 && (downward_distance > 60.0 || velocity_y > 300.0) {
                return BottomSheetReleaseDecision::Dismiss;
            }

            if detents.len() <= 1 {
                return BottomSheetReleaseDecision::Snap { index: current_index };
            }
            let expansion_distance = (detents[1] - detents[0]).max(0.0);
            if velocity_y < -300.0 || upward_distance > expansion_distance * 0.5 {
                return BottomSheetReleaseDecision::Snap { index: 1 };
            }
            return BottomSheetReleaseDecision::Snap { index: current_index };
        }

        if downward_distance > 0.0 && velocity_y > 1_800.0 {
            return BottomSheetReleaseDecision::Dismiss;
        }

        let lower_index = current_index - 1;
        let collapse_distance = (detents[current_index] - detents[lower_index]).max(0.0);
        if velocity_y > 300.0 || downward_distance > collapse_distance * 0.5 {
            return BottomSheetReleaseDecision::Snap { index: lower_index };
        }

        if current_index < detents.len() - 1 {
            let upper_index = current_index + 1;
            let expansion_distance = (detents[upper_index] - detents[current_index]).max(0.0);
            if velocity_y < -300.0 || upward_distance > expansion_distance * 0.5 {
                return BottomSheetReleaseDecision::Snap { index: upper_index };
            }
        }

        BottomSheetReleaseDecision::Snap { index: current_index }
    }
}
