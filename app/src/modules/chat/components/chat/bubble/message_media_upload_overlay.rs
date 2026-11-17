use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, ElementId, FontWeight, IntoElement, Pixels, RenderOnce, SharedString, Size, Window, black, size, white};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::{FormatArg, format_printf, tr};
use rise_ui::IconUi;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize, LoaderUiType};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::{CubicBezier, MotionSpec};
use rise_ui::swiftui::prelude::*;

use crate::core::lib::byte_format::ByteFormat;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::stores::message::message_actions::message_types::MessageMediaItem;

const RING_ANIMATION: MotionSpec = MotionSpec::curve(CubicBezier::LINEAR, Duration::from_millis(120));

#[derive(IntoElement)]
pub struct MessageMediaUploadOverlay {
    id: ElementId,
    pub item: MessageMediaItem,
    pub cell_size: Size<Pixels>,
    pub on_cancel: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl MessageMediaUploadOverlay {
    const MAX_RING_SIZE: f32 = 52.0;
    const MAX_RING_LINE: f32 = 2.5;
    const MAX_ICON_SIZE: f32 = 17.0;

    pub fn new(id: impl Into<ElementId>, item: MessageMediaItem) -> Self {
        Self {
            id: id.into(),
            item,
            cell_size: size(px(120.0), px(120.0)),
            on_cancel: None,
        }
    }

    pub fn cell_size(mut self, cell_size: Size<Pixels>) -> Self {
        self.cell_size = cell_size;
        self
    }

    pub fn on_cancel(mut self, on_cancel: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Rc::new(on_cancel));
        self
    }

    fn child_id(&self, suffix: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(suffix))
    }

    /// metrics as ring then line then icon then hit
    fn metrics(&self) -> (f32, f32, f32, f32) {
        let fit = f32::from(self.cell_size.width).min(f32::from(self.cell_size.height)).max(1.0);
        let t = (fit / 130.0).min(1.0);
        let ring = (Self::MAX_RING_SIZE * t).min(Self::MAX_RING_SIZE).max(26.0);
        let line = (Self::MAX_RING_LINE * t).max(1.8);
        let icon = (Self::MAX_ICON_SIZE * t).min(Self::MAX_ICON_SIZE).max(11.0);
        (ring, line, icon, ring + 14.0)
    }

    fn ring_progress(&self) -> f64 {
        if self.item.is_compressing {
            return (self.item.compression_progress / 100.0).min(1.0).max(0.08);
        }
        if self.is_awaiting_server() {
            return 1.0;
        }
        (self.item.upload_progress / 100.0).min(1.0).max(0.08)
    }

    fn is_awaiting_server(&self) -> bool {
        if self.item.is_compressing {
            return false;
        }
        if self.item.total_bytes > 0 && self.item.uploaded_bytes >= self.item.total_bytes {
            return true;
        }
        self.item.upload_progress >= 99.5
    }

    fn top_left_status(&self) -> String {
        if self.item.is_compressing {
            let progress = self.item.compression_progress.max(0.0).min(100.0).round() as i64;
            return if progress > 0 {
                format!("{} {}%", tr("chat_media_upload_processing"), progress)
            } else {
                tr("chat_media_upload_processing")
            };
        }
        if self.is_awaiting_server() {
            return tr("chat_media_upload_sending");
        }
        if self.item.total_bytes > 0 {
            return format_printf(
                &tr("chat_media_upload_progress"),
                &[
                    FormatArg::Text(ByteFormat::telegram(self.item.uploaded_bytes).into()),
                    FormatArg::Text(ByteFormat::telegram(self.item.total_bytes).into()),
                ],
            );
        }
        tr("chat_media_upload_loading")
    }
}

impl RenderOnce for MessageMediaUploadOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (ring, line, icon, hit) = self.metrics();
        let queued = self.item.is_upload_queued();

        let now = RiseClock::monotonic();
        let ring_target = if queued { 0.36 } else { self.ring_progress() as f32 };
        let ring_state = window.use_keyed_state(self.child_id("ring-progress"), cx, |_, _| Animated::new(ring_target));
        ring_state.update(cx, |animated, cx| animated.set_in(ring_target, RING_ANIMATION, now, cx));
        let shown_progress = ring_state.read(cx).frame_value(now, window);

        let mut overlay = zstack(Alignment::TopLeading)
            .size_full()
            .child(div().size_full().bg(black().opacity(if queued { 0.22 } else { 0.30 })));

        if !queued {
            overlay = overlay.child(
                vstack(0.0)
                    .align(HorizontalAlignment::Leading)
                    .size_full()
                    .child(
                        div()
                            .m(px(8.0))
                            .px(px(8.0))
                            .py(px(4.0))
                            .capsule()
                            .bg(black().opacity(0.45))
                            .swiftui_font(Font::system_weight(11.0, FontWeight::SEMIBOLD))
                            .text_color(white().opacity(0.95))
                            .whitespace_nowrap()
                            .truncate()
                            .child(SharedString::from(self.top_left_status())),
                    )
                    .child(spacer_min_h(0.0)),
            );
        }

        let on_cancel = self.on_cancel.clone();
        let cancel = zstack(Alignment::Center)
            .child(div().circle().bg(black().opacity(0.48)).frame_size(ring, ring))
            .child(
                LoaderUi::new()
                    .size(LoaderUiSize::Custom(ring))
                    .progress(f64::from(shown_progress))
                    .r#type(LoaderUiType::Progress)
                    .thickness(line)
                    .track_color(Some(white().opacity(0.28)))
                    .ring_animated(true)
                    .speed(3.0)
                    .gap(3.0),
            )
            .children(IconUi::sized("xmark", px(icon), white(), false))
            .frame_size(hit, hit)
            .circle()
            .bubble_tap_target_priority(self.child_id("cancel"), move |window: &mut Window, cx: &mut App| {
                if let Some(on_cancel) = &on_cancel {
                    on_cancel(window, cx);
                }
            });

        overlay.child(
            zstack(Alignment::Center)
                .size_full()
                .child(cancel),
        )
    }
}
