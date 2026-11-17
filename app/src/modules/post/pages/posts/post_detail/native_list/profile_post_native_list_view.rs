use std::rc::Rc;

use gpui::{App, Empty, RenderOnce, SharedString, Window, prelude::*, px};
use rise_core::foundation::foundation_calendar::{Calendar, TimeZone};
use rise_i18n::language_store::LanguageStore;
use rise_theme::ThemeStore;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::swiftui::open_url::open_url;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::media_viewer::media_viewer_presentation_state::media_viewer_presentation;
use crate::core::widgets::wrappers::list_shell_ui::{list_shell_floating_top_inset, list_shell_safe_area_insets};
use crate::modules::post::components::repost::repost_overlay::RepostTransitionNamespaceKey;
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostPagePresentation, RisePostPageScope, RisePostProfileKind,
};
use crate::modules::post::pages::posts::post_detail::native_list::post_feed_native_list_view::dismantle_post_list_when_released;
use crate::modules::post::pages::posts::post_detail::native_list::post_list_configuration::PostListConfiguration;
use crate::modules::post::pages::posts::post_detail::native_list::post_list_controller::PostListController;
use crate::modules::post::pages::posts::post_detail::native_list::post_list_input::PostListInput;
use crate::modules::post::pages::posts::post_detail::post_list_shared_element_destination::PostListSharedElementDestination;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::shorts::components::shorts_zoom_namespace::ShortsZoomNamespaceKey;

/// profile grid order post list on the native engine, same inputs as the profile post list body
#[derive(IntoElement)]
pub struct ProfilePostNativeListView {
    anchor_id: String,
    anchor_index: i64,
    anchor_post: Option<GetPostFeedResponse>,
    user_id: String,
    tag: String,
    kind: RisePostProfileKind,
    page: RisePostPagePresentation,
    destination: Rc<PostListSharedElementDestination>,
    latched: Binding<bool>,
    consume_comment_focus: Rc<dyn Fn(&GetPostFeedResponse, &mut App)>,
}

impl ProfilePostNativeListView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        anchor_id: impl Into<String>,
        anchor_index: i64,
        anchor_post: Option<GetPostFeedResponse>,
        user_id: impl Into<String>,
        tag: impl Into<String>,
        kind: RisePostProfileKind,
        page: RisePostPagePresentation,
        destination: Rc<PostListSharedElementDestination>,
        latched: Binding<bool>,
        consume_comment_focus: impl Fn(&GetPostFeedResponse, &mut App) + 'static,
    ) -> Self {
        Self {
            anchor_id: anchor_id.into(),
            anchor_index,
            anchor_post,
            user_id: user_id.into(),
            tag: tag.into(),
            kind,
            page,
            destination,
            latched,
            consume_comment_focus: Rc::new(consume_comment_focus),
        }
    }
}

impl RenderOnce for ProfilePostNativeListView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let latched = self.latched;
        let session = self.destination.session;
        ProfilePostNativeListRepresentable {
            anchor_id: self.anchor_id,
            anchor_index: self.anchor_index,
            anchor_post: self.anchor_post,
            user_id: self.user_id,
            tag: self.tag,
            kind: self.kind,
            page: self.page,
            destination: self.destination,
            consume_comment_focus: self.consume_comment_focus,
        }
        .on_appear(SharedString::from(format!("profile-post-native-list-appear-{session}")), move |window, cx| {
            if !latched.get(cx) {
                latched.set(true, window, cx);
            }
        })
    }
}

#[derive(IntoElement)]
struct ProfilePostNativeListRepresentable {
    anchor_id: String,
    anchor_index: i64,
    anchor_post: Option<GetPostFeedResponse>,
    user_id: String,
    tag: String,
    kind: RisePostProfileKind,
    page: RisePostPagePresentation,
    destination: Rc<PostListSharedElementDestination>,
    consume_comment_focus: Rc<dyn Fn(&GetPostFeedResponse, &mut App)>,
}

impl RenderOnce for ProfilePostNativeListRepresentable {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(navigation_store) = env::<NavigationStoreKey>(cx) else {
            return Empty.into_any_element();
        };
        let floating_top_inset = list_shell_floating_top_inset(cx);
        let shell_safe_area_insets = list_shell_safe_area_insets(cx);
        let session = self.destination.session;
        let destination = self.destination;
        let initial_anchor_id = self.anchor_id.clone();
        let anchor_index = self.anchor_index;
        let controller = window.use_keyed_state(
            SharedString::from(format!("profile-post-native-list-{session}")),
            cx,
            move |window, cx| {
                let controller = PostListController::new(
                    PostListConfiguration::profile_grid(
                        destination,
                        initial_anchor_id,
                        usize::try_from(anchor_index).unwrap_or(0),
                    ),
                    window,
                    cx,
                );
                dismantle_post_list_when_released(&controller, window, cx);
                controller
            },
        );

        let user_id = self.user_id;
        let tag = self.tag;
        let kind = self.kind;
        let load_user_id = user_id.clone();
        let load_tag = tag.clone();
        let reactivate_user_id = user_id.clone();
        let reactivate_tag = tag.clone();
        let input = PostListInput {
            scope: RisePostPageScope::Profile { user_id, tag, kind },
            page: self.page,
            anchor_id: self.anchor_id,
            anchor_index: usize::try_from(anchor_index).unwrap_or(0),
            anchor_post: self.anchor_post,
            is_active: true,
            floating_top_inset: px(floating_top_inset),
            bottom_inset: px(shell_safe_area_insets.bottom),
            safe_bottom: px(shell_safe_area_insets.bottom),
            load_more: Rc::new(move |cx: &mut App| {
                PostInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_more_profile(&load_user_id, &load_tag, kind, cx));
            }),
            refresh: None,
            reactivate: Rc::new(move |cx: &mut App| {
                PostInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.activate_profile(&reactivate_user_id, &reactivate_tag, kind, cx));
            }),
            consume_comment_focus: Some(self.consume_comment_focus),
            visible_anchor_did_change: None,
            navigation_store,
            media_viewer_presentation: media_viewer_presentation(cx),
            open_url: open_url(cx),
            theme_store: ThemeStore::shared(cx),
            repost_transition_namespace: env::<RepostTransitionNamespaceKey>(cx),
            shorts_zoom_namespace: env::<ShortsZoomNamespaceKey>(cx),
            locale: LanguageStore::shared(cx).locale(),
            calendar: Calendar::current(),
            time_zone: TimeZone::current(),
            remount_at_top: None,
            refresh_after_remount: None,
        };
        let controller = controller.read(cx).clone();
        controller.submit(input, window, cx);

        controller.view().into_any_element()
    }
}
