use std::rc::Rc;

use gpui::{AnyElement, App, IntoElement as _, ParentElement as _, Pixels, Size, Styled as _, Window, div};

use crate::core::shared_element_transition::renderer::shared_element_cover_hooks::{
    SharedElementCoverPostHooks, SharedElementCoverRepostHooks,
};
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCover, SharedElementCoverContent,
};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

pub enum SharedElementCoverRasterizer {}

impl SharedElementCoverRasterizer {
    pub fn text_identity(post_id: &str) -> String {
        format!("text:{post_id}")
    }

    /// cover for text cells in publications, threads and tagged
    pub fn text_cover(
        post: &GetPostFeedResponse,
        cell_size: Size<Pixels>,
        display_scale: f32,
        _epoch: u64,
    ) -> Option<SharedElementCover> {
        SharedElementCoverPostHooks::installed()?;
        let cell = post.clone();
        Self::render(
            Rc::new(move |_: &mut Window, _: &mut App| {
                SharedElementCoverPostHooks::post_grid_text_ui(&cell)
                    .unwrap_or_else(|| div().into_any_element())
            }),
            Self::text_identity(&post.id),
            cell_size,
            display_scale,
        )
    }

    pub fn repost_text_cover(
        post: &GetPostFeedResponse,
        cell_size: Size<Pixels>,
        display_scale: f32,
        _epoch: u64,
    ) -> Option<SharedElementCover> {
        SharedElementCoverRepostHooks::installed()?;
        let cell = post.clone();
        Self::render(
            Rc::new(move |_: &mut Window, _: &mut App| {
                SharedElementCoverRepostHooks::profile_grid_repost_text_ui(&cell)
                    .unwrap_or_else(|| div().into_any_element())
            }),
            Self::text_identity(&post.id),
            cell_size,
            display_scale,
        )
    }

    fn render(
        content: Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>,
        identity: String,
        cell_size: Size<Pixels>,
        _display_scale: f32,
    ) -> Option<SharedElementCover> {
        let width = f32::from(cell_size.width);
        let height = f32::from(cell_size.height);
        let is_drawable = width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0;
        if !is_drawable {
            return None;
        }
        let framed: Rc<dyn Fn(&mut Window, &mut App) -> AnyElement> = Rc::new(move |window, cx| {
            div().size_full().overflow_hidden().child(content(window, cx)).into_any_element()
        });
        Some(SharedElementCover {
            image: SharedElementCoverContent::Element(framed),
            aspect: width / height,
            identity,
        })
    }
}
