use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use gpui::{AnyElement, IntoElement};
use rise_ui::swiftui::environment::EnvKey;
use rise_ui::swiftui::zoom_navigation_transition::{
    ZoomTransitionNamespace, ZoomTransitionSourceId, matched_transition_source,
};

use crate::app::router::navigation_types::RootRoute;

/// zoom namespace id, root view owns one per window
pub type ShortsZoomNamespaceId = ZoomTransitionNamespace;

pub struct ShortsZoomNamespaceKey;

impl EnvKey for ShortsZoomNamespaceKey {
    type Value = Option<ShortsZoomNamespaceId>;

    fn default_value() -> Self::Value {
        None
    }
}

pub struct ShortsZoomSource {
    pub id: RootRoute,
    pub namespace: Option<ShortsZoomNamespaceId>,
}

impl ShortsZoomSource {
    pub fn new(id: RootRoute, namespace: Option<ShortsZoomNamespaceId>) -> Self {
        Self { id, namespace }
    }

    /// wraps the content for the zoom transition
    pub fn body(&self, content: impl IntoElement) -> AnyElement {
        match (self.namespace, Self::source_id(&self.id)) {
            (Some(namespace), Some(id)) => {
                matched_transition_source(content, namespace, id).into_any_element()
            }
            _ => content.into_any_element(),
        }
    }

    /// route as the id of the source,
    /// only the case and the post count since a pushed route keeps an old snapshot of the post
    pub fn source_id(route: &RootRoute) -> Option<ZoomTransitionSourceId> {
        let (case, post_id) = match route {
            RootRoute::ShortViewer { post, .. } => (0_u8, post.id.as_str()),
            RootRoute::PostDetail { post_id: Some(post_id), .. } => (1_u8, post_id.as_str()),
            _ => return None,
        };
        let mut hasher = DefaultHasher::new();
        case.hash(&mut hasher);
        post_id.hash(&mut hasher);
        Some(hasher.finish())
    }
}
