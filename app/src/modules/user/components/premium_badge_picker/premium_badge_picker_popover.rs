use std::time::Duration;

use gpui::{
    App, Bounds, Context, Edges, Entity, FocusHandle, IntoElement, Pixels, Render, Size, Subscription,
    Window, black, point, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::haptics::{ImpactStyle, impact};
use rise_ui::keyboard_dismiss::LayerFocus;
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::motion::{ease_out, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelPageMotion;
use rise_ui::swiftui::transition::{TransitionModifiers, UnitPoint};
use rise_ui::window_screen_frame_reader::WindowScreenFrameReader;
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};

use crate::app::router::main_tab_chrome::main_tab_bar_underlap;
use crate::modules::user::components::premium_badge_picker::premium_badge_emoji_picker::PremiumBadgeEmojiPicker;


#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PremiumBadgePickerPopoverSession {
    pub generation: u64,
    pub anchor: Bounds<Pixels>,
}

pub struct PremiumBadgePickerPopover {
    session: Option<PremiumBadgePickerPopoverSession>,
    generation: u64,
}

impl PremiumBadgePickerPopover {
    const HOVER_LABEL_ALLOWANCE: Duration = Duration::from_millis(1500);

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { session: None, generation: 0 }
    }

    pub fn session(&self) -> Option<PremiumBadgePickerPopoverSession> {
        self.session
    }

    pub fn present(&mut self, anchor: Bounds<Pixels>, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.session = Some(PremiumBadgePickerPopoverSession { generation: self.generation, anchor });
        TooltipLabelPageMotion::note(Self::HOVER_LABEL_ALLOWANCE, cx);
        impact(ImpactStyle::Light);
        cx.notify();
    }

    /// reopening while the old panel animates out must not let the old dismissal close the new one,
    /// so close carries its generation
    pub fn dismiss_if_current(&mut self, generation: u64, cx: &mut Context<Self>) {
        if self.session.map(|session| session.generation) != Some(generation) {
            return;
        }
        self.session = None;
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.session = None;
        cx.notify();
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PremiumBadgePopoverLayoutPlacement {
    pub rect: Bounds<Pixels>,
    pub anchor_unit: UnitPoint,
    pub placed_above: bool,
}

pub enum PremiumBadgePopoverLayout {}

impl PremiumBadgePopoverLayout {
    pub const PANEL_MAX_WIDTH: f32 = 360.0;
    pub const PANEL_MIN_HEIGHT: f32 = 260.0;
    pub const PANEL_MAX_HEIGHT: f32 = 420.0;
    pub const PANEL_FLOOR_HEIGHT: f32 = 160.0;
    pub const EDGE_INSET: f32 = 12.0;
    pub const ANCHOR_GAP: f32 = 10.0;

    /// anchor for when the badge own rect is momentarily unknown, only the drop animation origin is lost
    pub fn detached_anchor(window: &Window) -> Bounds<Pixels> {
        let viewport = window.viewport_size();
        Bounds::new(point(viewport.width / 2.0, viewport.height * 0.3), size(px(1.0), px(1.0)))
    }

    pub fn resolve(
        anchor: Bounds<Pixels>,
        container: Size<Pixels>,
        safe_area: Edges<f32>,
        bottom_obstruction: f32,
    ) -> PremiumBadgePopoverLayoutPlacement {
        let (container_width, container_height) = (f32::from(container.width), f32::from(container.height));
        let anchor_min_x = f32::from(anchor.origin.x);
        let anchor_min_y = f32::from(anchor.origin.y);
        let anchor_mid_x = anchor_min_x + f32::from(anchor.size.width) / 2.0;
        let anchor_max_y = anchor_min_y + f32::from(anchor.size.height);

        let width = 120.0_f32.max(Self::PANEL_MAX_WIDTH.min(container_width - Self::EDGE_INSET * 2.0));

        let top_limit = safe_area.top + Self::EDGE_INSET;
        let bottom_limit = container_height - safe_area.bottom.max(bottom_obstruction) - Self::EDGE_INSET;

        let available_below = bottom_limit - (anchor_max_y + Self::ANCHOR_GAP);
        let available_above = (anchor_min_y - Self::ANCHOR_GAP) - top_limit;
        let placed_above = available_below < Self::PANEL_MIN_HEIGHT && available_above > available_below;

        let available = if placed_above { available_above } else { available_below };
        let height =
            Self::PANEL_FLOOR_HEIGHT.max(Self::PANEL_MAX_HEIGHT.min(Self::PANEL_MIN_HEIGHT.max(available)));

        let unclamped_x = anchor_mid_x - width / 2.0;
        let x = Self::EDGE_INSET
            .max(unclamped_x)
            .min(Self::EDGE_INSET.max(container_width - width - Self::EDGE_INSET));

        let unclamped_y = if placed_above {
            anchor_min_y - Self::ANCHOR_GAP - height
        } else {
            anchor_max_y + Self::ANCHOR_GAP
        };
        let y = top_limit.max(unclamped_y).min(top_limit.max(bottom_limit - height));

        let unit_x = if width > 0.0 { ((anchor_mid_x - x) / width).clamp(0.0, 1.0) } else { 0.5 };

        PremiumBadgePopoverLayoutPlacement {
            rect: Bounds::new(point(px(x), px(y)), size(px(width), px(height))),
            anchor_unit: UnitPoint::new(unit_x, if placed_above { 1.0 } else { 0.0 }),
            placed_above,
        }
    }
}

/// badge picker as a panel dropping out of the badge
/// borrows the hold menu chrome and spring but not its machinery
/// which doesnt fit a glyph
pub struct PremiumBadgePickerPopoverHost {
    appeared: Animated<f32>,
    frame: Bounds<Pixels>,
    focus_handle: FocusHandle,
    _popover_observation: Subscription,
}

impl PremiumBadgePickerPopoverHost {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let popover = PremiumBadgePickerPopover::shared(cx);
        Self {
            appeared: Animated::new(0.0),
            frame: Bounds::default(),
            focus_handle: cx.focus_handle(),
            _popover_observation: cx.observe(&popover, |_, _, cx| cx.notify()),
        }
    }

    fn panel(
        &self,
        session: PremiumBadgePickerPopoverSession,
        host: &Entity<Self>,
        popover: &Entity<PremiumBadgePickerPopover>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let generation = session.generation;
        let reduce_motion_on = reduce_motion(cx);
        let tab_underlap = main_tab_bar_underlap(cx);
        let frame = self.frame;
        let now = RiseClock::monotonic();
        let appeared = self.appeared.frame_value(now, window);
        let rows_settling = self.appeared.is_running(now);
        let focus_handle = self.focus_handle.clone();
        let anchor = Bounds::new(
            point(session.anchor.origin.x - frame.origin.x, session.anchor.origin.y - frame.origin.y),
            session.anchor.size,
        );
        let placement = PremiumBadgePopoverLayout::resolve(anchor, frame.size, Edges::default(), tab_underlap);

        let opacity = appeared.clamp(0.0, 1.0);
        let closed_scale = if reduce_motion_on { 1.0 } else { 0.62 };
        let scale = closed_scale + (1.0 - closed_scale) * appeared;

        let (backdrop_host, backdrop_popover) = (host.clone(), popover.clone());
        let (committed_host, committed_popover) = (host.clone(), popover.clone());
        let (escape_host, escape_popover) = (host.clone(), popover.clone());
        let appear_host = host.clone();
        let disappear_host = host.clone();
        let layer_focus = LayerFocus::keyed("premium-badge-picker-focus-return", window, cx);
        let (layer_appearing, layer_leaving) = (layer_focus.clone(), layer_focus);
        let appear_focus = focus_handle.clone();
        let disappear_focus = focus_handle.clone();

        div()
            .absolute()
            .inset_0()
            .track_focus(&focus_handle)
            .layer_keys(LayerCancel::dismiss(move |_, cx| {
                Self::close(&escape_host, &escape_popover, generation, cx);
            }))
            .child(
                div()
                    .id("premium-badge-picker-backdrop")
                    .absolute()
                    .inset_0()
                    .bg(black().opacity(0.16 * opacity))
                    .occlude()
                    .on_click(move |_, _, cx| Self::close(&backdrop_host, &backdrop_popover, generation, cx)),
            )
            .child(
                div()
                    .absolute()
                    .left(placement.rect.origin.x)
                    .top(placement.rect.origin.y)
                    .w(placement.rect.size.width)
                    .h(placement.rect.size.height)
                    .child(
                        div()
                            .size_full()
                            .occlude()
                            .opacity(opacity)
                            .child(PremiumBadgeEmojiPicker::new().chrome(true).rows_settling(rows_settling).on_committed(move |_, cx| {
                                Self::close(&committed_host, &committed_popover, generation, cx);
                            }))
                            .scale_effect_anchored(scale, placement.anchor_unit),
                    ),
            )
            .on_appear("premium-badge-picker-panel", move |window, cx| {
                LayerFocus::did_appear(&layer_appearing, &appear_focus, window, cx);
                let spec = if reduce_motion(cx) {
                    ease_out(0.18)
                } else {
                    spring(HoldContextMenuMetrics::OPEN_RESPONSE, HoldContextMenuMetrics::OPEN_DAMPING)
                };
                appear_host.update(cx, |host, cx| {
                    host.appeared.set(1.0, spec, RiseClock::monotonic(), false);
                    cx.notify();
                });
            })
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    LayerFocus::will_disappear(&layer_leaving, &disappear_focus, window, cx);
                }
                disappear_host.update(cx, |host, _| host.appeared.jump(0.0));
            })
            .into_any_element()
    }

    fn close(host: &Entity<Self>, popover: &Entity<PremiumBadgePickerPopover>, generation: u64, cx: &mut App) {
        if *host.read(cx).appeared.target() <= 0.0 {
            popover.update(cx, |popover, cx| popover.dismiss_if_current(generation, cx));
            return;
        }
        let spec = if reduce_motion(cx) {
            ease_out(0.14)
        } else {
            spring(HoldContextMenuMetrics::CLOSE_RESPONSE, HoldContextMenuMetrics::CLOSE_DAMPING)
        };
        host.update(cx, |host, cx| {
            host.appeared.set(0.0, spec, RiseClock::monotonic(), false);
            cx.notify();
        });
        let popover = popover.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs_f64(0.32)).await;
            cx.update(|cx| popover.update(cx, |popover, cx| popover.dismiss_if_current(generation, cx)));
        })
        .detach();
    }
}

impl Render for PremiumBadgePickerPopoverHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let popover = PremiumBadgePickerPopover::shared(cx);
        let host = cx.entity();
        let session = popover.read(cx).session();
        let panel = session.map(|session| self.panel(session, &host, &popover, window, cx));

        div()
            .relative()
            .size_full()
            .child(WindowScreenFrameReader::new("premium-badge-picker-popover-frame").on_change(
                move |frame: Bounds<Pixels>, _: &mut Window, cx: &mut App| {
                    host.update(cx, |host, cx| {
                        if host.frame != frame {
                            host.frame = frame;
                            cx.notify();
                        }
                    });
                },
            ))
            .children(panel)
    }
}
