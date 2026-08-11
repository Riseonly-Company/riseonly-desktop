use crate::core::animations::Animation;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OnboardingSlide {
    pub index: usize,
    pub animation: &'static str,
    pub title_key: &'static str,
    pub description_key: &'static str,
}

pub const SLIDES: [OnboardingSlide; 4] = [
    OnboardingSlide {
        index: 0,
        animation: Animation::CRYSTAL_BALL,
        title_key: "onboarding_slide1_title",
        description_key: "onboarding_slide1_description",
    },
    OnboardingSlide {
        index: 1,
        animation: Animation::COOL_EMOJI,
        title_key: "onboarding_slide2_title",
        description_key: "onboarding_slide2_description",
    },
    OnboardingSlide {
        index: 2,
        animation: Animation::SHIELD,
        title_key: "onboarding_slide3_title",
        description_key: "onboarding_slide3_description",
    },
    OnboardingSlide {
        index: 3,
        animation: Animation::PARTY,
        title_key: "onboarding_slide4_title",
        description_key: "onboarding_slide4_description",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slides_are_the_references_four_in_its_order() {
        assert_eq!(SLIDES.len(), 4);
        for (position, slide) in SLIDES.iter().enumerate() {
            assert_eq!(slide.index, position);
            assert!(slide.title_key.starts_with("onboarding_slide"));
            assert!(slide.description_key.ends_with("_description"));
        }
    }

    #[test]
    fn every_slide_names_an_animation_the_product_ships() {
        for slide in SLIDES {
            assert!(
                Animation::ALL.contains(&slide.animation),
                "{} is not in the shipped set, so the slide would draw nothing",
                slide.animation
            );
        }
    }

    #[test]
    fn no_two_slides_share_a_string_or_an_animation() {
        let mut keys: Vec<&str> = SLIDES.iter().map(|slide| slide.title_key).collect();
        keys.sort_unstable();
        let count = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), count);

        let mut animations: Vec<&str> = SLIDES.iter().map(|slide| slide.animation).collect();
        animations.sort_unstable();
        let count = animations.len();
        animations.dedup();
        assert_eq!(animations.len(), count);
    }
}
