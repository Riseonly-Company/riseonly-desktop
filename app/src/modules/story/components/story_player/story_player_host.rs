use gpui::{
    AppContext as _, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement as _,
    Pixels, Stateful, Div, Styled, Subscription, Window, anchored, black, div, px, white,
};
use rise_i18n::tr;
use rise_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::lifecycle::on_change;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::window_cover::WindowCoverFrame;

use crate::core::widgets::navigations::bottom_chrome_insets::BottomChromeInsetsModifiers;
use crate::modules::story::components::story_player::story_player::StoryPlayer;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;

/// own root host so player notifies and loading fade dirty only this host,
/// columns under keep cached frames
pub struct StoryPlayerHost {
    player: Option<(Uuid, Entity<StoryPlayer>)>,
    loading_alpha: Animated<f32>,
    observed_request_id: Option<Uuid>,
    observed_loading: bool,
    _service_subscription: Subscription,
}

/// desktop frame of the player,
/// a 9:16 card centered under the window top band,
/// not the phone full screen cover
struct StoryPlayerFrame {
    left: Pixels,
    top: Pixels,
    width: Pixels,
    height: Pixels,
}

impl StoryPlayerHost {
    const FRAME_ASPECT: f32 = 9.0 / 16.0;
    const FRAME_VERTICAL_MARGIN: f32 = 16.0;
    const FRAME_SIDE_MARGIN: f32 = 72.0;
    const BACKDROP_OPACITY: f32 = 0.9;
    const STEP_BUTTON_SIDE: f32 = 36.0;
    const STEP_BUTTON_GAP: f32 = 16.0;
    const STEP_GLYPH_SIDE: f32 = 16.0;

    pub fn new(cx: &mut Context<Self>) -> Self {
        let service = StoryServicesStore::shared(cx);
        let (observed_request_id, observed_loading) = Self::observed(service.read(cx));
        let service_subscription = cx.observe(&service, |host, service, cx| {
            let observed = Self::observed(service.read(cx));
            if observed != (host.observed_request_id, host.observed_loading) {
                (host.observed_request_id, host.observed_loading) = observed;
                cx.notify();
            }
        });
        Self {
            player: None,
            loading_alpha: Animated::new(if observed_loading { 1.0 } else { 0.0 }),
            observed_request_id,
            observed_loading,
            _service_subscription: service_subscription,
        }
    }

    fn observed(service: &StoryServicesStore) -> (Option<Uuid>, bool) {
        (service.player_request.as_ref().map(|request| request.id), service.is_loading_player)
    }

    fn step_button(player: &Entity<StoryPlayer>, frame: &StoryPlayerFrame, forward: bool) -> Stateful<Div> {
        let (name, glyph, label) = if forward {
            ("story-player-next", "chevron.right", tr("story_player_next"))
        } else {
            ("story-player-previous", "chevron.left", tr("story_player_previous"))
        };
        let left = if forward {
            f32::from(frame.left) + f32::from(frame.width) + Self::STEP_BUTTON_GAP
        } else {
            f32::from(frame.left) - Self::STEP_BUTTON_GAP - Self::STEP_BUTTON_SIDE
        };
        let top = f32::from(frame.top) + (f32::from(frame.height) - Self::STEP_BUTTON_SIDE) / 2.0;
        let player = player.clone();
        div()
            .id(name)
            .absolute()
            .left(px(left))
            .top(px(top))
            .size(px(Self::STEP_BUTTON_SIDE))
            .occlude()
            .aria_label(label.clone())
            .tooltip_label(label)
            .child(
                plain(format!("{name}-button"))
                    .on_press(move |_, cx| player.update(cx, |player, cx| player.step_from_frame_button(forward, cx)))
                    .child(
                        div()
                            .size(px(Self::STEP_BUTTON_SIDE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized(glyph, px(Self::STEP_GLYPH_SIDE), white(), false))
                            .liquid_chrome_circle(None),
                    ),
            )
    }

    fn frame(cover: &WindowCoverFrame) -> StoryPlayerFrame {
        let band = f32::from(cover.safe_top);
        let cover_width = f32::from(cover.size.width);
        let below_band = (f32::from(cover.size.height) - band).max(1.0);
        let fit_height = (below_band - Self::FRAME_VERTICAL_MARGIN * 2.0).max(1.0);
        let fit_width = (cover_width - Self::FRAME_SIDE_MARGIN * 2.0).max(1.0);
        let width = (fit_height * Self::FRAME_ASPECT).min(fit_width);
        let height = width / Self::FRAME_ASPECT;
        StoryPlayerFrame {
            left: px(((cover_width - width) / 2.0).max(0.0)),
            top: px(band + ((below_band - height) / 2.0).max(0.0)),
            width: px(width),
            height: px(height),
        }
    }
}

impl Render for StoryPlayerHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_loading_player = self.observed_loading;

        let mounted = self.player.as_ref().map(|(id, _)| *id);
        if mounted != self.observed_request_id {
            let request = StoryServicesStore::shared(cx).read(cx).player_request.clone();
            tracing::warn!(target: "riseonly::X1107", ?mounted, live = request.is_some(), "story player host remount");
            self.player = request.map(|request| {
                let id = request.id;
                (id, cx.new(|cx| StoryPlayer::new(request, window, cx)))
            });
        }

        {
            let host = cx.entity().downgrade();
            on_change("story-player-host-loading", is_loading_player, false, window, cx, move |_, loading, _, cx| {
                let target = if *loading { 1.0 } else { 0.0 };
                let _ = host.update(cx, |host, cx| {
                    host.loading_alpha.set_in(target, ease_in_out(0.2), RiseClock::monotonic(), cx);
                    cx.notify();
                });
            });
        }
        let loading_alpha = self.loading_alpha.frame_value(RiseClock::monotonic(), window);
        let player = self.player.as_ref().map(|(_, player)| player.clone());

        let loading_overlay = (is_loading_player || loading_alpha > 0.0).then(|| {
            div()
                .id("story-player-loading")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .opacity(loading_alpha)
                .bg(black().opacity(0.55))
                .occlude()
                .on_click(|_, _, cx| {
                    tracing::warn!(target: "riseonly::X1107", "story loading overlay click");
                    StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.dismiss_player(cx));
                })
                .child(LoaderUi::new().size(LoaderUiSize::Large).color(Some(white())))
        });

        let cover = WindowCoverFrame::resolve(window, cx);
        let frame = Self::frame(&cover);
        let backdrop = player.clone().map(|backdrop_player| {
            div()
                .id("story-player-backdrop")
                .absolute()
                .inset_0()
                .bg(black().opacity(Self::BACKDROP_OPACITY))
                .occlude()
                .on_click(move |_, _, cx| backdrop_player.update(cx, |player, cx| {
                    tracing::warn!(target: "riseonly::X1107", "story backdrop click");
                    player.dismiss(true, cx);
                    cx.notify();
                }))
        });
        let step_buttons = player.as_ref().map(|player| {
            let shown = player.read(cx);
            let stepping = !shown.viewers_open();
            let (has_previous, has_next) = (stepping && shown.has_previous_step(), stepping && shown.has_next_step());
            let mut buttons = Vec::new();
            if has_previous {
                buttons.push(Self::step_button(player, &frame, false));
            }
            if has_next {
                buttons.push(Self::step_button(player, &frame, true));
            }
            buttons
        });
        anchored().position(cover.origin).child(
            div()
                .relative()
                .w(cover.size.width)
                .h(cover.size.height)
                .children(backdrop)
                .children(loading_overlay)
                .children(player.map(|player| {
                    div()
                        .absolute()
                        .left(frame.left)
                        .top(frame.top)
                        .w(frame.width)
                        .h(frame.height)
                        .overflow_hidden()
                        .child(player.occludes_bottom_chrome())
                }))
                .children(step_buttons.into_iter().flatten()),
        )
    }
}
