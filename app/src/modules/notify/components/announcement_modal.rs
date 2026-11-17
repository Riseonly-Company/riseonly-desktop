use std::rc::Rc;

use gpui::{App, Context, Entity, RenderOnce, SharedString, Window};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change};
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::shapes::{RoundedCornerStyle, Shape, SwiftUiShapes, centred_stroke};
use rise_ui::swiftui::transition::{Presence, Transition};
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};

use crate::core::push_notifications::push_deep_link_store::{PushDeepLink, PushDeepLinkStore};
use crate::modules::notify::components::slide_deck_modal_focus::SlideDeckModalFocus;
use crate::modules::notify::components::slide_deck_ui::{SlideDeckHeader, SlideDeckUi};
use crate::modules::notify::engine::core::rise_announcements_wire_models::{RiseAnnouncement, RiseAnnouncementSlide};
use crate::modules::notify::engine::rise_launch_interruptions::RiseLaunchInterruptions;
use crate::modules::notify::stores::notify_interactions::notify_interactions::NotifyInteractionsStore;

const MEDIA_KIND_NONE: &str = "none";

struct AnnouncementHostState {
    slide_index: i64,
    shown_announcement_id: Option<i64>,
    leaving_through_action: bool,
    announcement: Option<RiseAnnouncement>,
    presence: Presence<RiseAnnouncement>,
}

/// operator broadcast drawn with the changelog slide deck
/// never blocking, close is always drawn unlike a forced update note
#[derive(IntoElement, Default)]
pub struct AnnouncementHost;

impl AnnouncementHost {
    pub fn new() -> Self {
        Self
    }

    fn mark_shown(state: &Entity<AnnouncementHostState>, announcement: &RiseAnnouncement, cx: &mut App) {
        if state.read(cx).shown_announcement_id == Some(announcement.id) {
            return;
        }
        state.update(cx, |state, _| state.shown_announcement_id = Some(announcement.id));
        NotifyInteractionsStore::shared(cx).update(cx, |store, cx| store.announcement_appeared(announcement, cx));
    }

    fn dismiss(state: &Entity<AnnouncementHostState>, announcement: &RiseAnnouncement, cx: &mut App) {
        let seen_slides = state.read(cx).slide_index + 1;
        NotifyInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.dismiss_announcement(announcement, seen_slides, cx));
    }

    fn open_action(
        state: &Entity<AnnouncementHostState>,
        announcement: &RiseAnnouncement,
        slide: &RiseAnnouncementSlide,
        cx: &mut App,
    ) {
        state.update(cx, |state, _| state.leaving_through_action = true);
        let seen_slides = state.read(cx).slide_index + 1;
        NotifyInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.open_announcement_action(announcement, slide, seen_slides, cx));
    }
}

impl RenderOnce for AnnouncementHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let interruptions = RiseLaunchInterruptions::shared(cx);
        let announcement = interruptions.read(cx).announcement(cx);
        let deep_links = PushDeepLinkStore::shared(cx);
        let pending_route = deep_links.read(cx).pending_route().cloned();
        let state = window.use_keyed_state("announcement-host", cx, |_, _| AnnouncementHostState {
            slide_index: 0,
            shown_announcement_id: None,
            leaving_through_action: false,
            announcement: None,
            presence: Presence::new(None).with_transition(Transition::opacity()),
        });

        let next = announcement.clone();
        state.update(cx, |state: &mut AnnouncementHostState, cx: &mut Context<AnnouncementHostState>| {
            if state.announcement != next {
                state.announcement = next.clone();
                state.presence.set_in(next, ease_out(0.22), now, cx);
                cx.notify();
            }
        });

        let changed_state = state.clone();
        let changed_announcement = announcement.clone();
        on_change(
            "announcement-host-announcement",
            announcement.as_ref().map(|announcement| announcement.id),
            true,
            window,
            cx,
            move |_, new, _, cx| {
                let Some(announcement) = changed_announcement.filter(|announcement| Some(announcement.id) == *new)
                else {
                    return;
                };
                changed_state.update(cx, |state, _| {
                    state.slide_index = 0;
                    state.leaving_through_action = false;
                });
                Self::mark_shown(&changed_state, &announcement, cx);
            },
        );

        let route_state = state.clone();
        on_change("announcement-host-route", pending_route, false, window, cx, move |_, route, _, cx| {
            let (Some(route), Some(announcement)) = (route, announcement) else {
                return;
            };
            if route_state.read(cx).leaving_through_action || matches!(route, PushDeepLink::Announcement { .. }) {
                return;
            }
            Self::dismiss(&route_state, &announcement, cx);
        });

        let shared: &App = cx;
        let overlay = state.read(shared).presence.render_transitioned(now, window, |announcement| {
            let slide_index = Binding::entity(
                state.clone(),
                |state: &AnnouncementHostState| state.slide_index,
                |state: &mut AnnouncementHostState, index, _| state.slide_index = index,
            );
            let dismiss_state = state.clone();
            let dismiss_announcement = announcement.clone();
            let action_state = state.clone();
            let action_announcement = announcement.clone();
            div().absolute().inset_0().child(AnnouncementModal::new(
                announcement.clone(),
                slide_index,
                Rc::new(move |_, cx| Self::dismiss(&dismiss_state, &dismiss_announcement, cx)),
                Rc::new(move |slide, _, cx| Self::open_action(&action_state, &action_announcement, slide, cx)),
            ))
        });

        div().absolute().inset_0().children(overlay)
    }
}

#[derive(IntoElement)]
pub struct AnnouncementModal {
    announcement: RiseAnnouncement,
    slide_index: Binding<i64>,
    on_dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
    on_action: Rc<dyn Fn(&RiseAnnouncementSlide, &mut Window, &mut App)>,
    slides: Vec<RiseAnnouncementSlide>,
    has_media: bool,
}

impl AnnouncementModal {
    pub fn new(
        announcement: RiseAnnouncement,
        slide_index: Binding<i64>,
        on_dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
        on_action: Rc<dyn Fn(&RiseAnnouncementSlide, &mut Window, &mut App)>,
    ) -> Self {
        let has_media =
            announcement.slides.iter().any(|slide| !slide.media_url.is_empty() && slide.media_kind != MEDIA_KIND_NONE);
        let slides = if announcement.slides.is_empty() {
            vec![Self::synthetic_slide(&announcement)]
        } else {
            announcement.slides.clone()
        };
        Self { announcement, slide_index, on_dismiss, on_action, slides, has_media }
    }

    fn synthetic_slide(announcement: &RiseAnnouncement) -> RiseAnnouncementSlide {
        RiseAnnouncementSlide::new(
            0,
            0,
            String::new(),
            MEDIA_KIND_NONE.to_owned(),
            String::new(),
            announcement.body.clone(),
            String::new(),
            String::new(),
        )
    }
}

impl RenderOnce for AnnouncementModal {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let modal_focus = SlideDeckModalFocus::keyed("announcement-modal-focus", window, cx);
        let focus_handle = modal_focus.read(cx).handle.clone();
        let escape_dismiss = self.on_dismiss.clone();

        div()
            .id("announcement-modal")
            .size_full()
            .relative()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.bg._100.opacity(0.82))
            .track_focus(&focus_handle)
            .layer_keys(LayerCancel::dismiss(move |window, cx| escape_dismiss(window, cx)))
            .child(
                div().flex().flex_col().w_full().max_h_full().max_w(px(460.0)).px(px(20.0)).child(
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .w_full()
                        .min_h_0()
                        .child(
                            vstack(0.0)
                                .w_full()
                                .min_h_0()
                                .pt(px(22.0))
                                .pb(px(16.0))
                                .bg(theme.bg._200)
                                .continuous_corners(22.0)
                                .overflow_hidden()
                                .child(SlideDeckHeader::new(
                                    "announcement-header",
                                    SharedString::from(self.announcement.title.clone()),
                                    theme.clone(),
                                    Some(self.on_dismiss.clone()),
                                ))
                                .child(SlideDeckUi::new(
                                    "announcement-deck",
                                    self.slides,
                                    self.slide_index,
                                    false,
                                    self.has_media,
                                    theme.clone(),
                                    self.on_action,
                                    self.on_dismiss,
                                )),
                        )
                        .child(centred_stroke(
                            theme.bg._400,
                            1.0,
                            Shape::RoundedRectangle { corner_radius: 22.0, style: RoundedCornerStyle::Continuous },
                        )),
                ),
            )
            .on_disappear("announcement-modal-lifecycle", move |window, cx| {
                if let Some(window) = window {
                    SlideDeckModalFocus::hand_back(&modal_focus, window, cx);
                }
            })
    }
}
