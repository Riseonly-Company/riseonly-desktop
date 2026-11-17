use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use futures::future::{LocalBoxFuture, join_all};
use gpui::{App, AsyncApp, Context, Empty, Entity, RenderOnce, SharedString, Window, div, prelude::*, px};
use rise_core::foundation::foundation_calendar::{Calendar, TimeZone};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::language_store::LanguageStore;
use rise_theme::ThemeStore;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::open_url::open_url;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{CleverImageBitmapLoader, CleverImagePrefetchItem, CleverImageWorkLane};
use crate::core::media_viewer::media_viewer_presentation_state::media_viewer_presentation;
use crate::core::widgets::wrappers::list_shell_ui::{list_shell_floating_top_inset, list_shell_safe_area_insets};
use crate::modules::post::components::post_collection_ui::PostPageRefreshErrorUi;
use crate::modules::post::components::post_feed_media_prefetch::PostFeedMediaPrefetchPolicy;
use crate::modules::post::components::repost::repost_overlay::RepostTransitionNamespaceKey;
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostCacheSource, RisePostPagePresentation, RisePostPageScope, RisePostPageStatus, RisePostViewSurface,
};
use crate::modules::post::pages::posts::post_detail::native_list::post_list_configuration::{
    PostListConfiguration, PostListConfigurationWindow, PostListHeader,
};
use crate::modules::post::pages::posts::post_detail::native_list::post_list_controller::PostListController;
use crate::modules::post::pages::posts::post_detail::native_list::post_list_input::PostListInput;
use crate::modules::shorts::components::shorts_zoom_namespace::ShortsZoomNamespaceKey;

/// post page on the native engine from its first row, with refresh
/// pagination during inertia and an optional hosted header row
/// the screen keeps its own loading, empty and error views for a page without items
#[derive(IntoElement)]
pub struct PostFeedNativeListView {
    id: SharedString,
    scope: RisePostPageScope,
    page: RisePostPagePresentation,
    view_surface: RisePostViewSurface,
    header: Option<PostListHeader>,
    top_padding: f32,
    bottom_inset: f32,
    is_active: bool,
    restore_anchor_id: Binding<Option<String>>,
    load_more: Rc<dyn Fn(&mut App)>,
    refresh: Option<Rc<dyn Fn(&mut App) -> LocalBoxFuture<'static, ()>>>,
    reactivate: Rc<dyn Fn(&mut App)>,
    on_retry: Option<Rc<dyn Fn(&mut App)>>,
    error_text: String,
    remount_at_top: Option<Rc<dyn Fn(&mut App)>>,
    refresh_after_remount: Option<Rc<std::cell::Cell<bool>>>,
}

impl PostFeedNativeListView {
    pub fn new(
        id: impl Into<SharedString>,
        scope: RisePostPageScope,
        page: RisePostPagePresentation,
        view_surface: RisePostViewSurface,
        restore_anchor_id: Binding<Option<String>>,
        load_more: impl Fn(&mut App) + 'static,
        reactivate: impl Fn(&mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            scope,
            page,
            view_surface,
            header: None,
            top_padding: 0.0,
            bottom_inset: 0.0,
            is_active: true,
            restore_anchor_id,
            load_more: Rc::new(load_more),
            refresh: None,
            reactivate: Rc::new(reactivate),
            on_retry: None,
            error_text: String::new(),
            remount_at_top: None,
            refresh_after_remount: None,
        }
    }

    pub fn header(mut self, header: Option<PostListHeader>) -> Self {
        self.header = header;
        self
    }

    pub fn top_padding(mut self, top_padding: f32) -> Self {
        self.top_padding = top_padding;
        self
    }

    pub fn bottom_inset(mut self, bottom_inset: f32) -> Self {
        self.bottom_inset = bottom_inset;
        self
    }

    pub fn is_active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }

    pub fn refresh(mut self, refresh: Rc<dyn Fn(&mut App) -> LocalBoxFuture<'static, ()>>) -> Self {
        self.refresh = Some(refresh);
        self
    }

    pub fn on_retry(mut self, on_retry: impl Fn(&mut App) + 'static) -> Self {
        self.on_retry = Some(Rc::new(on_retry));
        self
    }

    pub fn error_text(mut self, error_text: impl Into<String>) -> Self {
        self.error_text = error_text.into();
        self
    }

    pub fn remount_at_top(mut self, remount_at_top: impl Fn(&mut App) + 'static) -> Self {
        self.remount_at_top = Some(Rc::new(remount_at_top));
        self
    }

    pub fn refresh_after_remount(mut self, refresh_after_remount: Rc<std::cell::Cell<bool>>) -> Self {
        self.refresh_after_remount = Some(refresh_after_remount);
        self
    }
}

impl RenderOnce for PostFeedNativeListView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let floating_top_inset = list_shell_floating_top_inset(cx);
        let is_failed = self.page.status == RisePostPageStatus::Failed;
        let pill_id = SharedString::from(format!("{}-refresh-error", self.id));
        let error_text = self.error_text;
        let on_retry = self.on_retry;
        div()
            .relative()
            .size_full()
            .child(PostFeedNativeListRepresentable {
                id: self.id,
                scope: self.scope,
                page: self.page,
                view_surface: self.view_surface,
                header: self.header,
                top_padding: self.top_padding,
                bottom_inset: self.bottom_inset,
                is_active: self.is_active,
                restore_anchor_id: self.restore_anchor_id,
                load_more: self.load_more,
                refresh: self.refresh,
                reactivate: self.reactivate,
                remount_at_top: self.remount_at_top,
                refresh_after_remount: self.refresh_after_remount,
            })
            .children(on_retry.filter(|_| is_failed).map(|on_retry| {
                div().absolute().top_0().left_0().right_0().flex().justify_center().pt(px(floating_top_inset + 8.0)).child(
                    PostPageRefreshErrorUi::new(pill_id, error_text, move |cx| on_retry(cx)),
                )
            }))
    }
}

#[derive(IntoElement)]
struct PostFeedNativeListRepresentable {
    id: SharedString,
    scope: RisePostPageScope,
    page: RisePostPagePresentation,
    view_surface: RisePostViewSurface,
    header: Option<PostListHeader>,
    top_padding: f32,
    bottom_inset: f32,
    is_active: bool,
    restore_anchor_id: Binding<Option<String>>,
    load_more: Rc<dyn Fn(&mut App)>,
    refresh: Option<Rc<dyn Fn(&mut App) -> LocalBoxFuture<'static, ()>>>,
    reactivate: Rc<dyn Fn(&mut App)>,
    remount_at_top: Option<Rc<dyn Fn(&mut App)>>,
    refresh_after_remount: Option<Rc<std::cell::Cell<bool>>>,
}

impl RenderOnce for PostFeedNativeListRepresentable {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(navigation_store) = env::<NavigationStoreKey>(cx) else {
            return Empty.into_any_element();
        };
        let floating_top_inset = list_shell_floating_top_inset(cx);
        let shell_safe_area_insets = list_shell_safe_area_insets(cx);
        let view_surface = self.view_surface;
        let header = self.header;
        let restore_anchor_id = self.restore_anchor_id;
        let fade = window.use_keyed_state(SharedString::from(format!("{}-restored-fade", self.id)), cx, |_, _| {
            PostFeedRestoredCover::default()
        });
        let incoming = self.page;
        let previous = fade.update(cx, |cover, _| cover.last_page.replace(incoming.clone()));
        if restore_anchor_id.get(cx).is_none()
            && let Some(previous) = previous
            && PostFeedRestoredFade::replaces_restored_rows(&previous, &incoming)
        {
            PostFeedRestoredFade::cover(&fade, previous, &incoming, cx);
        }
        let held = fade.read(cx).held.clone();
        let is_covered = held.is_some();
        let page = held.unwrap_or(incoming);
        let initial_anchor = restore_anchor_id.clone();
        let initial_page = page.clone();
        let controller = window.use_keyed_state(self.id.clone(), cx, move |window, cx| {
            let restore_id = initial_anchor.get(cx).and_then(|anchor| {
                initial_page.items.iter().find(|item| item.presentation_id() == anchor).map(|item| item.id.clone())
            });
            let controller = PostListController::new(
                PostListConfiguration {
                    window: PostListConfigurationWindow::Top { restore_id },
                    destination: None,
                    view_surface,
                    header,
                    opened_post_id: None,
                },
                window,
                cx,
            );
            dismantle_post_list_when_released(&controller, window, cx);
            controller
        });

        let anchor = restore_anchor_id;
        let window_handle = window.window_handle();
        let visible_anchor_did_change: Rc<dyn Fn(Option<&str>, &mut App)> =
            Rc::new(move |id: Option<&str>, cx: &mut App| {
                let id = id.map(str::to_owned);
                let anchor = anchor.clone();
                cx.defer(move |cx| {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        if anchor.get(cx) != id {
                            anchor.set(id, window, cx);
                        }
                    });
                });
            });

        let input = PostListInput {
            scope: self.scope,
            page,
            anchor_id: String::new(),
            anchor_index: 0,
            anchor_post: None,
            is_active: self.is_active,
            floating_top_inset: px(floating_top_inset + self.top_padding),
            bottom_inset: px(self.bottom_inset),
            safe_bottom: px(shell_safe_area_insets.bottom),
            load_more: self.load_more,
            refresh: self.refresh,
            reactivate: self.reactivate,
            consume_comment_focus: None,
            visible_anchor_did_change: Some(visible_anchor_did_change),
            navigation_store,
            media_viewer_presentation: media_viewer_presentation(cx),
            open_url: open_url(cx),
            theme_store: ThemeStore::shared(cx),
            repost_transition_namespace: env::<RepostTransitionNamespaceKey>(cx),
            shorts_zoom_namespace: env::<ShortsZoomNamespaceKey>(cx),
            locale: LanguageStore::shared(cx).locale(),
            calendar: Calendar::current(),
            time_zone: TimeZone::current(),
            remount_at_top: self.remount_at_top,
            refresh_after_remount: self.refresh_after_remount,
        };
        let controller = controller.read(cx).clone();
        controller.submit(input, window, cx);

        div()
            .size_full()
            .child(controller.view())
            .when(is_covered, |root| {
                root.capture_any_mouse_down(move |_, _, cx| {
                    fade.update(cx, |cover, cx| cover.reveal(cover.generation, Duration::ZERO, cx));
                })
            })
            .into_any_element()
    }
}

/// stops the controller when its keyed state is released, ie when the view left the tree
pub(crate) fn dismantle_post_list_when_released(
    controller: &Rc<PostListController>,
    window: &Window,
    cx: &mut Context<Rc<PostListController>>,
) {
    let window_handle = window.window_handle();
    let teardown = controller.clone();
    cx.on_release(move |_, cx| {
        cx.defer(move |cx| {
            let in_window = teardown.clone();
            if window_handle.update(cx, move |_, window, cx| in_window.stop(Some(window), cx)).is_err() {
                teardown.stop(None, cx);
            }
        });
    })
    .detach();
}

/// disk page replaced whole by the first network page
/// old rows stay over the controller until the fresh
/// page is warm so the swap never shows an empty frame
pub enum PostFeedRestoredFade {}

impl PostFeedRestoredFade {
    pub const WARMED_ROWS: usize = 3;
    pub const WARM_TIMEOUT: f64 = 1.5;
    pub const MINIMUM_HOLD: f64 = 0.18;

    pub fn replaces_restored_rows(previous: &RisePostPagePresentation, next: &RisePostPagePresentation) -> bool {
        previous.cache_source == RisePostCacheSource::PersistentStore
            && next.cache_source == RisePostCacheSource::Network
            && !previous.items.is_empty()
            && !next.items.is_empty()
            && previous.items.iter().take(3).map(|item| item.presentation_id()).collect::<Vec<_>>()
                != next.items.iter().take(3).map(|item| item.presentation_id()).collect::<Vec<_>>()
    }

    /// cover waits for the media of the first rows of incoming before revealing
    /// a fixed timer showed fresh rows with photos still loading
    /// the wait has a timeout
    fn cover(
        fade: &Entity<PostFeedRestoredCover>,
        previous: RisePostPagePresentation,
        incoming: &RisePostPagePresentation,
        cx: &mut App,
    ) {
        let mut seen = HashSet::new();
        let mut items: Vec<CleverImagePrefetchItem> = Vec::new();
        for post in incoming.items.iter().take(Self::WARMED_ROWS) {
            for item in PostFeedMediaPrefetchPolicy::items(post, cx) {
                if seen.insert(item.clone()) {
                    items.push(item);
                }
            }
        }
        let user_id = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        let not_before = RiseClock::monotonic() + Duration::from_secs_f64(Self::MINIMUM_HOLD);
        let generation = fade.update(cx, |cover, _| {
            cover.generation = cover.generation.wrapping_add(1);
            cover.is_revealing = false;
            let shown = cover.held.take().unwrap_or(previous);
            cover.held = Some(shown);
            cover.generation
        });

        let timeout_fade = fade.downgrade();
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_secs_f64(Self::WARM_TIMEOUT)).await;
            let _ = timeout_fade.update(cx, |cover, cx| cover.reveal(generation, not_before, cx));
        })
        .detach();

        let warm_fade = fade.downgrade();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let loader_cx = cx.clone();
            let loads = items.into_iter().map(|item| {
                let user_id = user_id.clone();
                let epoch = epoch.clone();
                let loader_cx = loader_cx.clone();
                async move {
                    let _ = CleverImageBitmapLoader::load(
                        &item.url_string,
                        item.persist_to_disk,
                        true,
                        user_id,
                        epoch,
                        item.maximum_pixel_dimension,
                        item.cache_policy,
                        CleverImageWorkLane::Visible,
                        &loader_cx,
                    )
                    .await;
                }
            });
            join_all(loads).await;
            let _ = warm_fade.update(cx, |cover, cx| cover.reveal(generation, not_before, cx));
        })
        .detach();
    }
}

/// holds the last live page and the restored page on screen
#[derive(Default)]
pub struct PostFeedRestoredCover {
    last_page: Option<RisePostPagePresentation>,
    held: Option<RisePostPagePresentation>,
    is_revealing: bool,
    generation: u64,
}

impl PostFeedRestoredCover {
    fn reveal(&mut self, generation: u64, not_before: Duration, cx: &mut Context<Self>) {
        if self.generation != generation || self.is_revealing || self.held.is_none() {
            return;
        }
        self.is_revealing = true;
        let delay = not_before.saturating_sub(RiseClock::monotonic());
        cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let _ = this.update(cx, |cover, cx| {
                if cover.generation != generation {
                    return;
                }
                cover.held = None;
                cx.notify();
            });
        })
        .detach();
    }
}
