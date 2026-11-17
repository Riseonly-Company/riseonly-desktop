use gpui::{App, Entity, IntoElement, RenderOnce, Styled, Window, div, px};

use crate::modules::user::components::profile_content::profile_scroll_coordinator::ProfileScrollCoordinator;

/// strip doesnt swallow clicks over the profile top
/// gpui events reach every hitbox until one stops it
/// strip underlap stops nothing
#[derive(IntoElement)]
pub struct ProfileUnderlapTouchRouter {
    pub coordinator: Entity<ProfileScrollCoordinator>,
    pub height: f32,
}

impl ProfileUnderlapTouchRouter {
    pub fn is_needed() -> bool {
        false
    }

    pub fn new(coordinator: Entity<ProfileScrollCoordinator>, height: f32) -> Self {
        Self { coordinator, height }
    }
}

impl RenderOnce for ProfileUnderlapTouchRouter {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div().relative().top(px(-self.height)).h(px(self.height))
    }
}
