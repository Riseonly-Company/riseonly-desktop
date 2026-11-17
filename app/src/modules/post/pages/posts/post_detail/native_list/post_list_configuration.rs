use std::rc::Rc;

use gpui::{AnyView, App, Window};

use crate::modules::post::engine::rise_post_engine_models::RisePostViewSurface;
use crate::modules::post::pages::posts::post_detail::post_list_shared_element_destination::PostListSharedElementDestination;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostListConfigurationWindow {
    GridOrder { anchor_id: String, anchor_index: usize },
    Top { restore_id: Option<String> },
}

#[derive(Clone)]
pub struct PostListConfiguration {
    pub window: PostListConfigurationWindow,
    pub destination: Option<Rc<PostListSharedElementDestination>>,
    pub view_surface: RisePostViewSurface,
    pub header: Option<PostListHeader>,
    pub opened_post_id: Option<String>,
}

impl PostListConfiguration {
    pub fn profile_grid(
        destination: Rc<PostListSharedElementDestination>,
        anchor_id: String,
        anchor_index: usize,
    ) -> PostListConfiguration {
        PostListConfiguration {
            window: PostListConfigurationWindow::GridOrder { anchor_id: anchor_id.clone(), anchor_index },
            destination: Some(destination),
            view_surface: RisePostViewSurface::ProfileList,
            header: None,
            opened_post_id: Some(anchor_id),
        }
    }

    pub fn is_feed(&self) -> bool {
        matches!(self.window, PostListConfigurationWindow::Top { .. })
    }
}

/// a row above the posts, eg the home feed stories tray
#[derive(Clone)]
pub struct PostListHeader {
    pub id: String,
    pub height: f32,
    pub content: Rc<dyn Fn(&mut Window, &mut App) -> AnyView>,
}
