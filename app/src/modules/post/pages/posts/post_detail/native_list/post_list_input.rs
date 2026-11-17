use std::collections::HashMap;
use std::rc::Rc;

use futures::future::LocalBoxFuture;
use gpui::{App, Entity, Pixels};
use rise_core::foundation::foundation_calendar::{Calendar, TimeZone};
use rise_i18n::language_store::Locale;
use rise_theme::ThemeStore;
use rise_ui::swiftui::open_url::OpenURLAction;

use crate::app::router::navigation_store::NavigationStore;
use crate::core::media_viewer::media_viewer_presentation_state::MediaViewerPresentationState;
use crate::core::shared_element_transition::kernel::shared_element_types::SharedElementSurface;
use crate::modules::post::components::repost::repost_overlay::RepostTransitionNamespace;
use crate::modules::post::engine::rise_post_engine_models::{RisePostPagePresentation, RisePostPageScope};
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::PostListHostedEnvironment;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::shorts::components::shorts_zoom_namespace::ShortsZoomNamespaceId;

/// everything the bridge hands the list controller in one update
#[derive(Clone)]
pub struct PostListInput {
    pub scope: RisePostPageScope,
    pub page: RisePostPagePresentation,
    pub anchor_id: String,
    pub anchor_index: usize,
    pub anchor_post: Option<GetPostFeedResponse>,
    pub is_active: bool,
    pub floating_top_inset: Pixels,
    pub bottom_inset: Pixels,
    pub safe_bottom: Pixels,
    pub load_more: Rc<dyn Fn(&mut App)>,
    pub refresh: Option<Rc<dyn Fn(&mut App) -> LocalBoxFuture<'static, ()>>>,
    pub reactivate: Rc<dyn Fn(&mut App)>,
    pub consume_comment_focus: Option<Rc<dyn Fn(&GetPostFeedResponse, &mut App)>>,
    pub visible_anchor_did_change: Option<Rc<dyn Fn(Option<&str>, &mut App)>>,
    pub navigation_store: Entity<NavigationStore>,
    pub media_viewer_presentation: Entity<MediaViewerPresentationState>,
    pub open_url: OpenURLAction,
    pub theme_store: Entity<ThemeStore>,
    pub repost_transition_namespace: Option<RepostTransitionNamespace>,
    pub shorts_zoom_namespace: Option<ShortsZoomNamespaceId>,
    pub locale: Locale,
    pub calendar: Calendar,
    pub time_zone: TimeZone,
    pub remount_at_top: Option<Rc<dyn Fn(&mut App)>>,
    pub refresh_after_remount: Option<Rc<std::cell::Cell<bool>>>,
}

impl PostListInput {
    /// environment for hosted leaves, fallback rows and header rows
    pub fn hosted_environment(&self, surface: Option<SharedElementSurface>) -> PostListHostedEnvironment {
        PostListHostedEnvironment {
            navigation_store: self.navigation_store.clone(),
            media_viewer_presentation: self.media_viewer_presentation.clone(),
            open_url: self.open_url.clone(),
            theme_store: self.theme_store.clone(),
            repost_transition_namespace: self.repost_transition_namespace.clone(),
            shorts_zoom_namespace: self.shorts_zoom_namespace,
            locale: self.locale.clone(),
            calendar: self.calendar.clone(),
            time_zone: self.time_zone.clone(),
            shared_element_surface: surface,
        }
    }

    pub fn is_equivalent(&self, other: &PostListInput) -> bool {
        self.scope == other.scope
            && self.anchor_id == other.anchor_id
            && self.anchor_index == other.anchor_index
            && self.page.revision == other.page.revision
            && Self::shares_items(&self.page.items, &other.page.items)
            && self.page.status == other.page.status
            && self.page.has_more == other.page.has_more
            && self.page.is_loading_more == other.page.is_loading_more
            && self.page.is_refreshing == other.page.is_refreshing
            && self.anchor_post == other.anchor_post
            && self.is_active == other.is_active
            && self.floating_top_inset == other.floating_top_inset
            && self.bottom_inset == other.bottom_inset
            && self.safe_bottom == other.safe_bottom
            && self.has_same_hosted_identity(other)
    }

    /// optimistic engagement republishes page items at the same revision
    /// (page load time) so the items themselves are the only change signal
    pub fn shares_items(lhs: &[GetPostFeedResponse], rhs: &[GetPostFeedResponse]) -> bool {
        if lhs.len() != rhs.len() {
            return false;
        }
        if lhs.is_empty() || std::ptr::eq(lhs.as_ptr(), rhs.as_ptr()) {
            return true;
        }
        lhs == rhs
    }

    /// stores and values hosted content reads, a change rebuilds the row context
    pub fn has_same_hosted_identity(&self, other: &PostListInput) -> bool {
        self.navigation_store == other.navigation_store
            && self.media_viewer_presentation == other.media_viewer_presentation
            && self.theme_store == other.theme_store
            && self.repost_transition_namespace == other.repost_transition_namespace
            && self.shorts_zoom_namespace == other.shorts_zoom_namespace
            && self.locale == other.locale
            && self.calendar == other.calendar
            && self.time_zone == other.time_zone
    }
}

/// rendered rows of one page snapshot, memoized per page revision and anchor
/// engine ids are presentation ids and a repeated one is not an engine row
#[derive(Clone, Default)]
pub struct PostListRows {
    pub items: Vec<GetPostFeedResponse>,
    pub ids: Vec<String>,
    pub presentation_ids: Vec<String>,
    pub first_index_by_id: HashMap<String, usize>,
    pub index_by_presentation_id: HashMap<String, usize>,
}

impl PostListRows {
    pub fn empty() -> Self {
        Self::default()
    }

    /// feeds pass an empty anchor_id and no anchor_post, so the page items stay as they are
    pub fn make(
        page_items: &[GetPostFeedResponse],
        anchor_id: &str,
        anchor_post: Option<&GetPostFeedResponse>,
    ) -> PostListRows {
        let mut items: Vec<GetPostFeedResponse> =
            if anchor_id.is_empty() || page_items.iter().any(|item| item.id == anchor_id) {
                page_items.to_vec()
            } else {
                let mut prepended = Vec::with_capacity(page_items.len() + 1);
                prepended.extend(anchor_post.cloned());
                prepended.extend_from_slice(page_items);
                prepended
            };
        if let Some(anchor_post) = anchor_post {
            for item in &mut items {
                if item.id == anchor_id && *item != *anchor_post {
                    *item = anchor_post.clone();
                }
            }
        }
        let mut ids = Vec::with_capacity(items.len());
        let mut presentation_ids = Vec::with_capacity(items.len());
        let mut first_index_by_id = HashMap::with_capacity(items.len());
        let mut index_by_presentation_id = HashMap::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let id = item.id.clone();
            let presentation_id = item.presentation_id();
            first_index_by_id.entry(id.clone()).or_insert(index);
            index_by_presentation_id.entry(presentation_id.clone()).or_insert(index);
            ids.push(id);
            presentation_ids.push(presentation_id);
        }
        PostListRows { items, ids, presentation_ids, first_index_by_id, index_by_presentation_id }
    }

    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// true when the engine draws the row at index (not a repeat of an earlier presentation id)
    pub fn is_engine_row(&self, index: usize) -> bool {
        self.presentation_ids
            .get(index)
            .is_some_and(|presentation_id| self.index_by_presentation_id.get(presentation_id) == Some(&index))
    }
}
