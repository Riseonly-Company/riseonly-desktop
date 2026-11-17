use std::rc::Rc;

use gpui::{
    App, Context, ElementId, Entity, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    Styled, Subscription, Window, black, div, px, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::{ThemeStore, alpha};
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::shorts::stores::shorts::shorts_interactions::shorts_interactions::ShortsInteractionsStore;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;
use crate::modules::user::stores::user::user_actions::user_types::FollowButtonState;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

/// follow button of the reel chrome and the comments header, compacts to a glyph when too narrow
#[derive(IntoElement)]
pub struct ShortsFollowButton {
    id: ElementId,
    post: Rc<GetPostFeedResponse>,
    on_dark: bool,
    is_enabled: bool,
    compacts_when_tight: bool,
}

impl ShortsFollowButton {
    pub fn new(post: impl Into<Rc<GetPostFeedResponse>>) -> Self {
        let post = post.into();
        Self {
            id: ElementId::Name(format!("shorts-follow-{}", post.id).into()),
            post,
            on_dark: true,
            is_enabled: true,
            compacts_when_tight: false,
        }
    }

    pub fn on_dark(mut self, on_dark: bool) -> Self {
        self.on_dark = on_dark;
        self
    }

    pub fn is_enabled(mut self, is_enabled: bool) -> Self {
        self.is_enabled = is_enabled;
        self
    }

    pub fn compacts_when_tight(mut self, compacts_when_tight: bool) -> Self {
        self.compacts_when_tight = compacts_when_tight;
        self
    }

    fn state(post: &GetPostFeedResponse, cx: &mut App) -> FollowButtonState {
        ShortsInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.author_relationship(post, cx))
            .follow_button_state()
    }

    fn glyph(state: FollowButtonState) -> &'static str {
        match state {
            FollowButtonState::Subscribe | FollowButtonState::SubscribeBack => "person.badge.plus",
            FollowButtonState::Following => "person.fill.checkmark",
            FollowButtonState::Friends => "person.2.fill",
        }
    }

    fn toggle_follow(post: &GetPostFeedResponse, cx: &mut App) {
        ShortsInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_follow_author(post, cx));
    }

    fn pill(&self, state: FollowButtonState, titled: bool, fill_opacity: f32, cx: &mut App) -> impl IntoElement + use<> {
        let theme = ThemeStore::shared(cx).read(cx).current_theme();
        let foreground: Hsla = if self.on_dark { white() } else { theme.text.primary };
        let stroke: Hsla = if self.on_dark { alpha(white(), 0.9) } else { theme.border._200 };
        let fill: Hsla = if self.on_dark { alpha(black(), 0.38) } else { theme.bg._200 };

        let mut row = div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.0))
            .px(px(if titled { 12.0 } else { 9.0 }))
            .h(px(28.0))
            .continuous_corners(8.0)
            .bg(alpha(fill, fill.a * fill_opacity));
        if titled {
            if state == FollowButtonState::Friends {
                row = row.children(IconUi::sized("person.2.fill", px(10.0), foreground, true));
            }
            row = row.child(
                MainText::new(tr(state.title_key()))
                    .px(12.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(foreground)
                    .line_limit(1)
                    .minimum_scale_factor(0.85),
            );
        } else {
            row = row.children(IconUi::sized(Self::glyph(state), px(13.0), foreground, true));
        }
        row.child(centred_stroke(stroke, 0.5, Shape::continuous(8.0)))
    }
}

/// state of the follow button, eased fill and measured widths
struct ShortsFollowButtonState {
    last_state: Option<FollowButtonState>,
    fill_opacity: Animated<f32>,
    available_width: f32,
    titled_width: f32,
    _subscriptions: Vec<Subscription>,
}

impl ShortsFollowButtonState {
    fn new(cx: &mut Context<Self>) -> Self {
        let shorts_store: Entity<ShortsServicesStore> = ShortsServicesStore::shared(cx);
        let user_store: Entity<UserServicesStore> = UserServicesStore::shared(cx);
        let theme_store: Entity<ThemeStore> = ThemeStore::shared(cx);
        Self {
            last_state: None,
            fill_opacity: Animated::new(1.0),
            available_width: 0.0,
            titled_width: 0.0,
            _subscriptions: vec![
                cx.observe(&shorts_store, |_, _, cx| cx.notify()),
                cx.observe(&user_store, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }
}

impl RenderOnce for ShortsFollowButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let holder = window.use_keyed_state(self.id.clone(), cx, |_, cx: &mut Context<ShortsFollowButtonState>| {
            ShortsFollowButtonState::new(cx)
        });
        let state = Self::state(&self.post, cx);
        let now = RiseClock::monotonic();
        holder.update(cx, |holder, cx| {
            let target = if state.is_primary() { 0.0 } else { 1.0 };
            match holder.last_state {
                None => holder.fill_opacity.jump(target),
                Some(last) if last != state => holder.fill_opacity.set_in(target, ease_in_out(0.2), now, cx),
                Some(_) => {}
            }
            holder.last_state = Some(state);
        });
        let (fill_opacity, is_running, available_width, titled_width) = {
            let holder = holder.read(cx);
            (
                holder.fill_opacity.frame_value(now, window),
                holder.fill_opacity.is_running(now),
                holder.available_width,
                holder.titled_width,
            )
        };
        if is_running {
            window.request_animation_frame();
        }

        let titled = !self.compacts_when_tight || titled_width <= available_width;
        let pill = self.pill(state, titled, fill_opacity, cx);

        let is_enabled = self.is_enabled;
        let post = self.post.clone();
        let button = plain(ElementId::from((self.id.clone(), "button")))
            .on_press(move |_, cx| {
                if !is_enabled {
                    return;
                }
                let _ = selection_changed();
                if state != FollowButtonState::Friends {
                    Self::toggle_follow(&post, cx);
                    return;
                }
                let confirmed = post.clone();
                NativeConfirmAlert::present_delete(
                    &tr("profile_remove_friend_title"),
                    &tr("profile_remove_friend_message"),
                    Some(tr("profile_remove_friend_confirm")),
                    None,
                    false,
                    None,
                    None,
                    move |cx| Self::toggle_follow(&confirmed, cx),
                    cx,
                );
            })
            .child(pill);
        let button = div()
            .id(ElementId::from((self.id.clone(), "hint")))
            .flex_none()
            .tooltip_label(tr(state.title_key()))
            .child(button);

        if !self.compacts_when_tight {
            return button.into_any_element();
        }

        let copy = self.pill(state, true, fill_opacity, cx);
        let available = holder.clone();
        let reported = holder;
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .flex()
            .child(measure(ElementId::from((self.id.clone(), "available")), move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                available.update(cx, |holder, cx| {
                    if holder.available_width != width {
                        holder.available_width = width;
                        cx.notify();
                    }
                });
            }))
            .child(button)
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .opacity(0.0)
                    .whitespace_nowrap()
                    .child(copy)
                    .child(measure(ElementId::from((self.id.clone(), "titled")), move |bounds, _, cx| {
                        let width = f32::from(bounds.size.width);
                        reported.update(cx, |holder, cx| {
                            if holder.titled_width != width {
                                holder.titled_width = width;
                                cx.notify();
                            }
                        });
                    })),
            )
            .into_any_element()
    }
}
