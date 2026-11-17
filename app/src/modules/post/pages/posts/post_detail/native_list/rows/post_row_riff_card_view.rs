use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Bounds, ClickEvent, Context, Entity, EntityId, InteractiveElement, IntoElement, ParentElement, Pixels,
    Point, Render, SharedString, Size, StatefulInteractiveElement, Styled, Window, black, div,
};
use rise_ui::swiftui::measure::measure;

use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_playback_focus::PostListFocusFrameView;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostListHostedEnvironment, PostRowMainContext, PostRowMediaBoxLayout, PreparedPostRow,
};
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_hosted_leaf::{
    PostRowHostedLeafView, PostRowLeafHitPolicy, PostRowRiffCardLeaf,
};
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_media_overlays::PostRowSoundButton;
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_media_view::{
    PostRowMediaPageView, PostRowMediaPageViewContent, PostRowMediaTap,
};
use crate::modules::shorts::components::shorts_media_prefetch::ShortsMediaPrefetchPolicy;

pub struct PostRowRiffCardView {
    pub on_open: Option<PostRowMediaTap>,
    pub on_toggle_sound: Option<PostRowMediaTap>,

    poster_page: Entity<PostRowMediaPageView>,
    leaf: Entity<PostRowHostedLeafView<PostRowRiffCardLeaf>>,
    is_muted: bool,
    sound_button_is_hidden: bool,
    tap_is_enabled: bool,
    has_video: bool,
    is_hidden: bool,
    box_size: Size<Pixels>,
    window_frame: Option<Bounds<Pixels>>,
    element_key: String,
}

impl PostRowRiffCardView {
    /// row_view is the mounted row entity the card draws in
    pub fn new(row_view: EntityId, cx: &mut Context<Self>) -> Self {
        let poster_page = cx.new(|cx| PostRowMediaPageView::new(row_view, cx));
        let leaf: Entity<PostRowHostedLeafView<PostRowRiffCardLeaf>> = cx.new(PostRowHostedLeafView::new);
        leaf.update(cx, |leaf, _| leaf.hit_policy = PostRowLeafHitPolicy::Everywhere);
        Self {
            on_open: None,
            on_toggle_sound: None,
            poster_page,
            leaf,
            is_muted: true,
            sound_button_is_hidden: true,
            tap_is_enabled: true,
            has_video: false,
            is_hidden: false,
            box_size: Size::default(),
            window_frame: None,
            element_key: cx.entity_id().to_string(),
        }
    }

    pub fn is_leaf_mounted(&self, cx: &App) -> bool {
        self.leaf.read(cx).is_mounted()
    }

    /// the box fill rule the poster and the player gravity follow
    pub fn fills(&self, cx: &App) -> bool {
        self.poster_page.read(cx).resolved_fills()
    }

    /// riff focus token reports the poster frame, it covers the whole card
    pub fn focus_frame_view(&self, cx: &App) -> Rc<PostListFocusFrameView> {
        self.poster_page.read(cx).focus_frame_view().clone()
    }

    pub fn set_hidden(&mut self, is_hidden: bool, cx: &mut Context<Self>) {
        if self.is_hidden != is_hidden {
            self.is_hidden = is_hidden;
            cx.notify();
        }
    }

    pub fn configure(
        &mut self,
        prepared: &Arc<PreparedPostRow>,
        layout: &PostRowMediaBoxLayout,
        context: &PostRowMainContext,
        cx: &mut Context<Self>,
    ) {
        let seed = &prepared.seed;
        let slide_layout = layout.slides.first();
        let content = PostRowMediaPageViewContent {
            poster_url: PostRowMediaPageView::trimmed_source(seed.media.slides.first().and_then(|slide| slide.poster_url())),
            is_video: false,
            natural_aspect: slide_layout.and_then(|slide| slide.natural_aspect),
            layout_fills: slide_layout.is_none_or(|slide| slide.fills),
            box_aspect: layout.box_aspect,
            box_size: layout.frame.size,
            is_own: seed.is_own,
            account_user_id: context.account_user_id.clone(),
            account_epoch: context.account_epoch.clone(),
            memory_buckets: vec![None, Some(ShortsMediaPrefetchPolicy::POSTER_PIXEL_DIMENSION)],
            stores_viewer_image: false,
            draws_placeholder: false,
        };
        let palette = context.palette.clone();
        self.poster_page.update(cx, |page, cx| {
            page.index = 0;
            page.configure(content, palette, cx);
        });
        self.box_size = layout.frame.size;
        self.has_video = prepared.post.resolved_video_url().is_some_and(|url| !url.is_empty());
        self.is_muted = context.feed_audio_muted;
        self.refresh_chrome(cx);
    }

    pub fn set_muted(&mut self, muted: bool, cx: &mut Context<Self>) {
        self.is_muted = muted;
        cx.notify();
    }

    pub fn mount_leaf(&mut self, content: PostRowRiffCardLeaf, environment: &PostListHostedEnvironment, cx: &mut Context<Self>) {
        self.leaf.update(cx, |leaf, cx| leaf.mount(content, environment, cx));
        self.refresh_chrome(cx);
    }

    pub fn update_leaf(&mut self, content: PostRowRiffCardLeaf, environment: &PostListHostedEnvironment, cx: &mut Context<Self>) {
        self.leaf.update(cx, |leaf, cx| leaf.update(content, environment, cx));
    }

    pub fn unmount_leaf(&mut self, cx: &mut Context<Self>) {
        if !self.leaf.read(cx).is_mounted() {
            return;
        }
        self.leaf.update(cx, |leaf, cx| leaf.unmount(cx));
        self.refresh_chrome(cx);
    }

    pub fn set_visible_in_viewport(&mut self, is_visible: bool, cx: &mut Context<Self>) {
        self.poster_page.update(cx, |page, cx| page.set_visible_in_viewport(is_visible, cx));
    }

    pub fn prepare_for_reuse(&mut self, cx: &mut Context<Self>) {
        self.unmount_leaf(cx);
        self.poster_page.update(cx, |page, cx| page.prepare_for_reuse(cx));
        self.has_video = false;
        self.refresh_chrome(cx);
    }

    fn gesture_recognizer(&self, position: Point<Pixels>) -> bool {
        if self.sound_button_is_hidden {
            return true;
        }
        let Some(frame) = self.window_frame else {
            return true;
        };
        !PostRowMediaPageView::sound_button_frame(self.box_size).contains(&(position - frame.origin))
    }

    fn refresh_chrome(&mut self, cx: &mut Context<Self>) {
        let is_mounted = self.leaf.read(cx).is_mounted();
        self.sound_button_is_hidden = !self.has_video || is_mounted;
        self.tap_is_enabled = !is_mounted;
        cx.notify();
    }
}

impl Render for PostRowRiffCardView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.is_hidden {
            return div().into_any_element();
        }
        let weak_self = cx.entity().downgrade();
        let measure_weak = weak_self.clone();
        let sound_weak = weak_self.clone();
        let sound_frame = PostRowMediaPageView::sound_button_frame(self.box_size);
        let leaf_is_mounted = self.leaf.read(cx).is_mounted();
        div()
            .id(SharedString::from(format!("post-row-riff-card-{}", self.element_key)))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(black())
            .child(measure(
                SharedString::from(format!("post-row-riff-card-measure-{}", self.element_key)),
                move |bounds: Bounds<Pixels>, _, cx| {
                    measure_weak
                        .update(cx, |this, _| this.window_frame = Some(bounds))
                        .ok();
                },
            ))
            .child(div().absolute().inset_0().child(self.poster_page.clone()))
            .when(leaf_is_mounted, |card| card.child(self.leaf.clone()))
            .when(!self.sound_button_is_hidden, |card| {
                card.child(
                    div()
                        .absolute()
                        .left(sound_frame.origin.x)
                        .top(sound_frame.origin.y)
                        .w(sound_frame.size.width)
                        .h(sound_frame.size.height)
                        .child(
                            PostRowSoundButton::new(SharedString::from(format!("post-row-riff-sound-{}", self.element_key)))
                                .set_muted(self.is_muted)
                                .on_tap(move |cx| {
                                    let Some(on_toggle_sound) =
                                        sound_weak.upgrade().and_then(|card| card.read(cx).on_toggle_sound.clone())
                                    else {
                                        return;
                                    };
                                    on_toggle_sound(cx);
                                }),
                        ),
                )
            })
            .when(self.tap_is_enabled, |card| {
                card.on_click(move |event: &ClickEvent, _, cx| {
                    let Some(card) = weak_self.upgrade() else {
                        return;
                    };
                    let position = event.position();
                    let on_open = {
                        let view = card.read(cx);
                        let receives = !view.leaf.read(cx).is_mounted() && view.gesture_recognizer(position);
                        view.on_open.clone().filter(|_| receives)
                    };
                    if let Some(on_open) = on_open {
                        on_open(cx);
                    }
                })
            })
            .into_any_element()
    }
}
