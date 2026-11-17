use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{AnyWindowHandle, App, Bounds, IntoElement, Pixels, RenderOnce, Styled, Window, canvas};
use rise_ui::guide_anchor_ui::guide_scroll_host::GuideScrollHost;

use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementReturnScrollMath, SharedElementReturnScrollMathBand,
};
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementReturnRequest, SharedElementReturnResolution, SharedElementReturnViewport,
};
use crate::core::widgets::navigations::bottom_chrome_insets::BottomChromeInsets;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tabs::ChatProfileViewportLayoutPolicy;
use crate::modules::user::components::profile_content::shared_element_return_scroller_environment::SharedElementReturnScrollerRef;

/// return positioning of a chat profile grid (media, gif) under the media viewer
/// before the flyback the tab scrolls so the current cell sits between pinned tabs and bottom chrome
pub(crate) struct ChatProfileGridReturnPositioner {
    scroller_ref: RefCell<Option<SharedElementReturnScrollerRef>>,
    probe_window: Cell<Option<AnyWindowHandle>>,
    probe_window_height: Cell<f32>,
    cell_width: Cell<f32>,
    columns: Cell<usize>,
    grid_content_origin_y: Cell<Option<f32>>,
    epoch: Cell<u64>,
}

impl ChatProfileGridReturnPositioner {
    pub(crate) const SPACING: f32 = 2.0;
    pub(crate) const HORIZONTAL_PADDING: f32 = 2.0;

    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            scroller_ref: RefCell::new(None),
            probe_window: Cell::new(None),
            probe_window_height: Cell::new(0.0),
            cell_width: Cell::new(0.0),
            columns: Cell::new(3),
            grid_content_origin_y: Cell::new(None),
            epoch: Cell::new(0),
        })
    }

    pub(crate) fn configure(&self, scroller: Option<SharedElementReturnScrollerRef>) {
        *self.scroller_ref.borrow_mut() = scroller;
    }

    /// called by the probe pinned to the grid top leading corner, inside its horizontal padding
    pub(crate) fn columns(&self) -> usize {
        self.columns.get()
    }

    pub(crate) fn record_layout(&self, probe: Bounds<Pixels>, container_width: f32, columns: usize, window: &Window) {
        let handle = window.window_handle();
        self.probe_window.set(Some(handle));
        self.probe_window_height.set(f32::from(window.viewport_size().height));
        let columns = columns.max(1);
        self.columns.set(columns);
        if container_width.is_finite() && container_width > 1.0 {
            let gutters = Self::SPACING * (columns - 1) as f32;
            self.cell_width.set(((container_width - gutters) / columns as f32).max(1.0));
        }
        let Some(scroll) = Self::enclosing_scroll_host(probe, handle) else { return };
        let Some(frame) = scroll.frame() else { return };
        let origin_y = f32::from(probe.origin.y - frame.origin.y) + f32::from(scroll.content_offset().y);
        if !origin_y.is_finite() || origin_y < 0.0 {
            return;
        }
        self.grid_content_origin_y.set(Some(origin_y));
    }

    pub(crate) fn prepare_flyback(&self, index: i64, cx: &mut App) -> Option<Bounds<Pixels>> {
        let index = usize::try_from(index).ok()?;
        let cell_width = self.cell_width.get();
        if cell_width <= 1.0 {
            return None;
        }
        let scroller_ref = self.scroller_ref.borrow().clone()?;
        let scroller = scroller_ref.scroller()?;
        if !scroller.can_prepare_return(scroller_ref.tab_key, cx) {
            return None;
        }
        let viewport = scroller.return_viewport_snapshot(scroller_ref.tab_key, cx)?;
        let item_rect = SharedElementReturnScrollMath::grid_item_rect_with_origin_x(
            index,
            self.columns.get(),
            cell_width,
            cell_width,
            Self::SPACING,
            self.grid_content_origin_y.get().unwrap_or_else(ChatProfileViewportLayoutPolicy::page_content_top_inset),
            Self::HORIZONTAL_PADDING,
        );
        let epoch = self.epoch.get().wrapping_add(1);
        self.epoch.set(epoch);
        let request = SharedElementReturnRequest {
            tab_key: scroller_ref.tab_key,
            item_content_rect: item_rect,
            band: self.return_band(viewport, scroller_ref.bottom_inset, cx),
            epoch,
            keeps_position: false,
        };
        scroller.prepare_return(request, cx)?;
        match scroller.resolve_return(epoch, item_rect, cx) {
            SharedElementReturnResolution::Ready { predicted_frame } => {
                scroller.finish_return(epoch, cx);
                Some(predicted_frame)
            }
            SharedElementReturnResolution::Deferred | SharedElementReturnResolution::Unavailable => {
                scroller.cancel_live_return(epoch, cx);
                None
            }
        }
    }

    fn return_band(
        &self,
        viewport: SharedElementReturnViewport,
        bottom_inset: f32,
        cx: &mut App,
    ) -> SharedElementReturnScrollMathBand {
        let top = ChatProfileViewportLayoutPolicy::page_content_top_inset();
        let outer_max_y = f32::from(viewport.screen_origin.y) + viewport.height;
        let mut inset = if bottom_inset.is_finite() { bottom_inset.max(0.0) } else { 0.0 };
        if let Some(chrome_top) = BottomChromeInsets::shared(cx).read(cx).top_edge_in_window(&[]) {
            let chrome_inset = outer_max_y - chrome_top;
            if chrome_inset.is_finite() {
                inset = inset.max(chrome_inset);
            }
        }
        if self.probe_window.get().is_some() {
            let safe_inset = outer_max_y - self.probe_window_height.get();
            if safe_inset.is_finite() {
                inset = inset.max(safe_inset);
            }
        }
        SharedElementReturnScrollMathBand { top, bottom_inset: inset.min((viewport.height - top).max(0.0)) }
    }

    fn enclosing_scroll_host(probe: Bounds<Pixels>, window: AnyWindowHandle) -> Option<GuideScrollHost> {
        let area = |host: &GuideScrollHost| {
            host.frame().map_or(f32::INFINITY, |frame| f32::from(frame.size.width) * f32::from(frame.size.height))
        };
        GuideScrollHost::live()
            .into_iter()
            .filter(|host| host.is_in_window() && host.window() == Some(window))
            .filter(|host| {
                host.frame().is_some_and(|frame| {
                    frame.left() <= probe.origin.x
                        && probe.origin.x <= frame.right()
                        && frame.top() <= probe.origin.y
                        && probe.origin.y <= frame.bottom()
                })
            })
            .min_by(|lhs, rhs| {
                area(lhs).total_cmp(&area(rhs)).then_with(|| {
                    f32::from(rhs.max_content_offset().y).total_cmp(&f32::from(lhs.max_content_offset().y))
                })
            })
    }
}

/// zero size probe under a chat profile grid reporting layout to the positioner
/// paints nothing and has no hitbox so it never catches input over the grid
#[derive(IntoElement)]
pub(crate) struct ChatProfileGridLayoutProbe {
    positioner: Rc<ChatProfileGridReturnPositioner>,
    container_width: f32,
    columns: usize,
}

impl ChatProfileGridLayoutProbe {
    pub(crate) fn new(positioner: Rc<ChatProfileGridReturnPositioner>, container_width: f32) -> Self {
        Self { positioner, container_width, columns: 3 }
    }

    pub(crate) fn columns(mut self, columns: usize) -> Self {
        self.columns = columns;
        self
    }
}

impl RenderOnce for ChatProfileGridLayoutProbe {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let positioner = self.positioner;
        let container_width = self.container_width;
        let columns = self.columns;
        canvas(
            move |bounds, window, _| positioner.record_layout(bounds, container_width, columns, window),
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_0()
    }
}
