use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    App, Bounds, Context, Entity, ExternalPaths, FocusHandle, FontWeight, Hsla, KeyDownEvent, PathBuilder,
    Pixels, SharedString, Size, Subscription, Task, WeakEntity, Window, black, canvas, point, white,
};
use image::DynamicImage;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_platform::capture_permission::{self, CaptureKind, settings_url};
use rise_platform::file_dialog::{self, FileFilter, FilterLabel, OpenRequest, Selection};
use rise_platform::haptics::{NotificationFeedbackGenerator, NotificationKind};
use rise_platform::pasteboard::UIPasteboard;
use rise_platform::HostOs;
use rise_ui::icon_ui::icon_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::app::router::navigation_types::RootRoute;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::push_notifications::push_deep_link_store::{PushDeepLink, PushDeepLinkStore};
use crate::core::ui::qr_code::rise_qr_scanner::{
    RiseQRScanResolver, RiseQRScannerController, RiseQRScannerControllerPhase,
};
use crate::modules::chat::components::chat::video_note::chat_video_note_recording_overlay::VideoNoteCameraPreview;

const LOG_NAME: &str = "RiseQRScannerPage";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AlertKind {
    NotFound,
    Unsupported,
    FileDialogUnavailable,
}

impl AlertKind {
    fn title(self) -> String {
        match self {
            Self::NotFound => tr("qr_code_not_found_title"),
            Self::Unsupported => tr("qr_code_unsupported_title"),
            Self::FileDialogUnavailable => tr("error"),
        }
    }

    fn message(self) -> String {
        match self {
            Self::NotFound => tr("qr_code_not_found_message"),
            Self::Unsupported => tr("qr_code_unsupported_message"),
            Self::FileDialogUnavailable => tr("desktop_file_dialog_unavailable"),
        }
    }
}

pub struct RiseQRScannerPage {
    controller: Entity<RiseQRScannerController>,
    is_picking_photo: bool,
    alert: Option<AlertKind>,
    size: Size<Pixels>,
    nav_store: Option<Entity<NavigationStore>>,
    focus_handle: FocusHandle,
    start_task: Option<Task<()>>,
    scan_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl RiseQRScannerPage {
    const ESCAPE_KEY: &'static str = "escape";
    const PASTE_KEY: &'static str = "v";
    const SCRIM_OPACITY: f32 = 0.55;
    const WINDOW_CORNER_RADIUS: f32 = 24.0;
    const BRACKETS_LINE_WIDTH: f32 = 4.0;
    const GLASS_FILL_OPACITY: f32 = 0.18;
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let controller = cx.new(RiseQRScannerController::new);
        let weak = cx.weak_entity();
        controller.update(cx, |controller, _| {
            controller.on_payload = Some(Rc::new(move |payload: String, cx: &mut App| {
                let _ = weak.update(cx, |page, cx| page.handle_payload(payload, cx));
            }));
        });
        let subscriptions = vec![
            cx.observe(&controller, |_, _, cx| cx.notify()),
            cx.observe_window_activation(window, |page, window, cx| page.scene_phase_changed(window.is_window_active(), cx)),
            cx.on_release(|page, cx| page.controller.update(cx, |controller, cx| controller.stop(cx))),
        ];
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let start_task = Some(controller.update(cx, |controller, cx| controller.start(cx)));
        Self {
            controller,
            is_picking_photo: false,
            alert: None,
            size: Size::default(),
            nav_store: None,
            focus_handle,
            start_task,
            scan_task: None,
            _subscriptions: subscriptions,
        }
    }

    fn scene_phase_changed(&mut self, is_active: bool, cx: &mut Context<Self>) {
        if !is_active {
            self.controller.update(cx, |controller, cx| controller.stop(cx));
            return;
        }
        let phase = self.controller.read(cx).phase();
        if !matches!(phase, RiseQRScannerControllerPhase::Idle | RiseQRScannerControllerPhase::Denied)
            || self.alert.is_some()
        {
            return;
        }
        self.start_task = Some(self.controller.update(cx, |controller, cx| controller.start(cx)));
    }

    fn scrim(window_side: f32, size: Size<Pixels>) -> impl IntoElement {
        let band = f32::from(size.width) + f32::from(size.height);
        div()
            .absolute()
            .inset_0()
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .flex_none()
                    .size(px(window_side + band * 2.0))
                    .border(px(band))
                    .border_color(black().opacity(Self::SCRIM_OPACITY))
                    .rounded(px(Self::WINDOW_CORNER_RADIUS + band)),
            )
    }

    fn brackets(window_side: f32) -> impl IntoElement {
        canvas(
            |_, _, _| {},
            move |bounds: Bounds<Pixels>, _, window, _| {
                if let Ok(path) = RiseQRScannerBrackets::default().path(bounds) {
                    window.paint_path(path, white());
                }
            },
        )
        .size(px(window_side))
    }

    fn chrome(&self, window_side: f32, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let state = self.controller.read(cx);
        let is_running = state.phase() == RiseQRScannerControllerPhase::Running;
        let torch_enabled = state.torch_enabled();
        let has_torch = state.has_torch();
        let controller = self.controller.clone();
        let weak = cx.weak_entity();

        vstack(0.0)
            .absolute()
            .inset_0()
            .child(
                hstack(None)
                    .px(px(16.0))
                    .pt(px(8.0))
                    .child(Self::glass_button("qr-scanner-back", "chevron.backward", tr("back"), 44.0, false, {
                        let page = weak.clone();
                        move |_: &mut Window, cx: &mut App| {
                            let _ = page.update(cx, |page, cx| page.pop(cx));
                        }
                    }))
                    .child(spacer_min_w(0.0)),
            )
            .child(spacer())
            .child(
                div()
                    .px(px(32.0))
                    .pt(px(window_side / 2.0 + 24.0))
                    .opacity(if is_running { 1.0 } else { 0.0 })
                    .child(MainText::new(tr("qr_code_scan_hint")).px(15.0).tac(TextAlignT::Center).color(white())),
            )
            .child(spacer())
            .child(
                hstack(28.0)
                    .justify_center()
                    .pb(px(24.0))
                    .child(Self::glass_button("qr-scanner-photo", "photo", tr("qr_code_pick_photo"), 72.0, false, {
                        let page = weak.clone();
                        move |_: &mut Window, cx: &mut App| {
                            let _ = page.update(cx, |page, cx| page.pick_photo(cx));
                        }
                    }))
                    .when(has_torch && is_running, |row| {
                        row.child(Self::glass_button(
                            "qr-scanner-torch",
                            if torch_enabled { "flashlight.on.fill" } else { "flashlight.off.fill" },
                            tr("qr_code_torch"),
                            72.0,
                            torch_enabled,
                            move |_: &mut Window, cx: &mut App| {
                                controller.update(cx, |controller, cx| controller.toggle_torch(cx));
                            },
                        ))
                    }),
            )
    }

    fn camera_denied_notice(width: f32) -> impl IntoElement {
        let can_open_settings = settings_url(HostOs::current(), CaptureKind::Camera).is_some();
        vstack(10.0)
            .w(px(width.max(0.0)))
            .items_center()
            .child(
                MainText::new(tr("qr_code_camera_denied_title"))
                    .px(17.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(white()),
            )
            .child(
                MainText::new(tr("qr_code_camera_denied_message"))
                    .px(15.0)
                    .tac(TextAlignT::Center)
                    .color(white().opacity(0.7)),
            )
            .when(can_open_settings, |notice| {
                notice.child(
                    plain("qr-scanner-open-settings")
                        .mt(px(6.0))
                        .on_press(|_, cx| {
                            capture_permission::open_settings(cx, CaptureKind::Camera);
                        })
                        .child(
                            div()
                                .px(px(20.0))
                                .h(px(40.0))
                                .flex()
                                .items_center()
                                .rounded_full()
                                .bg(white().opacity(Self::GLASS_FILL_OPACITY))
                                .child(
                                    MainText::new(tr("media_picker_open_settings"))
                                        .px(15.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(white()),
                                ),
                        ),
                )
            })
    }

    fn glass_button(
        id: &'static str,
        icon: &'static str,
        label: String,
        size: f32,
        highlighted: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        let foreground: Hsla = if highlighted { black() } else { white() };
        let fill: Hsla = if highlighted { white() } else { white().opacity(Self::GLASS_FILL_OPACITY) };
        div().id(SharedString::from(format!("{id}-label"))).aria_label(label).child(
            plain(id).on_press(action).child(
                zstack(Alignment::Center)
                    .size(px(size))
                    .rounded_full()
                    .bg(fill)
                    .children(IconUi::sized(icon, px(size * 0.38), foreground, false)),
            ),
        )
    }

    fn handle_payload(&mut self, payload: String, cx: &mut Context<Self>) {
        let Some(link) = RiseQRScanResolver::deep_link(&payload, AppEnvironment::current()) else {
            NotificationFeedbackGenerator::new().notification_occurred(NotificationKind::Warning);
            self.present_alert(AlertKind::Unsupported, cx);
            return;
        };
        NotificationFeedbackGenerator::new().notification_occurred(NotificationKind::Success);
        self.controller.update(cx, |controller, cx| controller.stop(cx));
        self.route(link, cx);
    }

    fn present_alert(&mut self, kind: AlertKind, cx: &mut Context<Self>) {
        self.alert = Some(kind);
        cx.notify();
        let weak = cx.weak_entity();
        NativeConfirmAlert::present_notice(
            &kind.title(),
            &kind.message(),
            None,
            None,
            move |cx| {
                let _ = weak.update(cx, |page, cx| page.alert_dismissed(cx));
            },
            cx,
        );
    }

    fn alert_dismissed(&mut self, cx: &mut Context<Self>) {
        self.alert = None;
        cx.notify();
        if self.controller.read(cx).phase() == RiseQRScannerControllerPhase::Idle {
            self.start_task = Some(self.controller.update(cx, |controller, cx| controller.start(cx)));
        } else {
            self.controller.update(cx, |controller, _| controller.resume_delivery());
        }
    }

    fn pick_photo(&mut self, cx: &mut Context<Self>) {
        if self.is_picking_photo {
            return;
        }
        self.is_picking_photo = true;
        let weak = cx.weak_entity();
        let refused = weak.clone();
        self.scan_task = Some(RiseQRPhotoPicker::new(move |picked, cx| {
            let _ = weak.update(cx, |page, cx| {
                page.is_picking_photo = false;
                if let Some(path) = picked {
                    page.scan_file(path, cx);
                }
            });
        })
        .on_failure(move |cx| {
            let _ = refused.update(cx, |page, cx| page.present_alert(AlertKind::FileDialogUnavailable, cx));
        })
        .present(cx));
    }

    fn scan_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.scan_picked(move || image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok(), cx);
    }

    fn scan_picked(&mut self, load: impl FnOnce() -> Option<DynamicImage> + Send + 'static, cx: &mut Context<Self>) {
        self.scan_task = Some(cx.spawn(async move |page, cx| {
            let payload = RiseAccountRuntime::current()
                .spawn_blocking(move || load().and_then(|image| RiseQRScanResolver::payload(&image)))
                .await
                .ok()
                .flatten();
            let _ = page.update(cx, |page, cx| match payload {
                Some(payload) => page.handle_payload(payload, cx),
                None => {
                    NotificationFeedbackGenerator::new().notification_occurred(NotificationKind::Warning);
                    page.present_alert(AlertKind::NotFound, cx);
                }
            });
        }));
    }

    /// desktop addition to the photo path, an image dropped on the scanner
    fn dropped(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        if let Some(path) = paths.paths().first() {
            self.scan_file(path.clone(), cx);
        }
    }

    /// desktop addition to the photo path, an image pasted from the clipboard
    fn pasted(&mut self, cx: &mut Context<Self>) {
        let Some(image) = UIPasteboard::general().image(cx) else {
            return;
        };
        self.scan_picked(move || image::load_from_memory(&image.bytes).ok(), cx);
    }

    fn key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.key == Self::ESCAPE_KEY {
            cx.stop_propagation();
            self.pop(cx);
        } else if keystroke.key == Self::PASTE_KEY && keystroke.modifiers.secondary() {
            cx.stop_propagation();
            self.pasted(cx);
        }
    }

    fn pop(&mut self, cx: &mut Context<Self>) {
        if let Some(nav_store) = self.nav_store.clone() {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }

    /// pops the scanner and the qr card below it first
    /// so the deep link lands on the stack the scanner was opened from and not on top of that card
    fn route(&mut self, link: PushDeepLink, cx: &mut Context<Self>) {
        if let Some(nav_store) = self.nav_store.clone() {
            nav_store.update(cx, |nav_store, cx| {
                nav_store.pop(cx);
                if matches!(nav_store.path.last(), Some(RootRoute::QrCode { .. })) {
                    nav_store.pop(cx);
                }
            });
        }
        cx.defer(move |cx| {
            PushDeepLinkStore::shared(cx).update(cx, |store, cx| {
                store.enqueue(link, cx);
            });
        });
    }

    fn page_measured(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        if self.size != bounds.size {
            self.size = bounds.size;
            cx.notify();
        }
    }
}

impl Render for RiseQRScannerPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.nav_store = env::<NavigationStoreKey>(cx);
        let size = self.size;
        let window_side = (f32::from(size.width) - 80.0).min(f32::from(size.height) * 0.42).max(0.0);
        let controller = self.controller.read(cx);
        let phase = controller.phase();
        let session = controller.session.clone();
        let weak: WeakEntity<Self> = cx.weak_entity();

        div()
            .id("qr-scanner-page")
            .size_full()
            .relative()
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .bg(black())
            .when(phase == RiseQRScannerControllerPhase::Running, |page| {
                page.child(div().absolute().inset_0().child(VideoNoteCameraPreview::new(session)))
            })
            .child(Self::scrim(window_side, size))
            .child(Self::brackets(window_side))
            .child(self.chrome(window_side, cx))
            .when(phase == RiseQRScannerControllerPhase::Denied, |page| {
                page.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Self::camera_denied_notice(window_side - 32.0)),
                )
            })
            .child(measure("qr-scanner-size", move |bounds, _, cx| {
                let _ = weak.update(cx, |page, cx| page.page_measured(bounds, cx));
            }))
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|page, event: &KeyDownEvent, _, cx| page.key_down(event, cx)))
            .on_drop(cx.listener(|page, paths: &ExternalPaths, _, cx| page.dropped(paths, cx)))
    }
}

#[derive(Clone, Copy, Debug)]
struct RiseQRScannerBrackets {
    length: f32,
    inset: f32,
}

impl Default for RiseQRScannerBrackets {
    fn default() -> Self {
        Self { length: 34.0, inset: 2.0 }
    }
}

impl RiseQRScannerBrackets {
    fn path(self, rect: Bounds<Pixels>) -> anyhow::Result<gpui::Path<Pixels>> {
        let min_x = f32::from(rect.origin.x) + self.inset;
        let min_y = f32::from(rect.origin.y) + self.inset;
        let max_x = f32::from(rect.origin.x + rect.size.width) - self.inset;
        let max_y = f32::from(rect.origin.y + rect.size.height) - self.inset;
        let arm = self.length.min((max_x - min_x).min(max_y - min_y) / 3.0);
        let at = |x: f32, y: f32| point(px(x), px(y));
        let mut path = PathBuilder::stroke(px(RiseQRScannerPage::BRACKETS_LINE_WIDTH));

        path.move_to(at(min_x, min_y + arm));
        path.line_to(at(min_x, min_y));
        path.line_to(at(min_x + arm, min_y));

        path.move_to(at(max_x - arm, min_y));
        path.line_to(at(max_x, min_y));
        path.line_to(at(max_x, min_y + arm));

        path.move_to(at(max_x, max_y - arm));
        path.line_to(at(max_x, max_y));
        path.line_to(at(max_x - arm, max_y));

        path.move_to(at(min_x + arm, max_y));
        path.line_to(at(min_x, max_y));
        path.line_to(at(min_x, max_y - arm));

        path.build()
    }
}

/// system picker twin, the open panel needs no photo library permission
/// hands back only the one picked image as a file the scanner reads once the panel is gone
pub struct RiseQRPhotoPicker {
    on_pick: Box<dyn FnOnce(Option<PathBuf>, &mut App)>,
    on_failure: Option<Box<dyn FnOnce(&mut App)>>,
}

impl RiseQRPhotoPicker {
    const PICKABLE_IMAGE_EXTENSIONS: [&'static str; 8] = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"];

    pub fn new(on_pick: impl FnOnce(Option<PathBuf>, &mut App) + 'static) -> Self {
        Self { on_pick: Box::new(on_pick), on_failure: None }
    }

    /// desktop only, the open panel refused to open after on_pick got no file
    pub fn on_failure(mut self, on_failure: impl FnOnce(&mut App) + 'static) -> Self {
        self.on_failure = Some(Box::new(on_failure));
        self
    }

    pub fn present<T: 'static>(self, cx: &mut Context<T>) -> Task<()> {
        let filter = FileFilter::new(FilterLabel::IMAGES, Self::PICKABLE_IMAGE_EXTENSIONS);
        let selection = file_dialog::open(cx, &OpenRequest::files(filter).allowing_multiple(false));
        let on_pick = self.on_pick;
        let on_failure = self.on_failure;
        cx.spawn(async move |_, cx| {
            let (picked, failure) = match selection.await {
                Selection::Picked(picked) => (picked.accepted.into_iter().next(), None),
                Selection::Cancelled => (None, None),
                Selection::Failed(error) => (None, Some(error.to_string())),
            };
            cx.update(|cx| {
                on_pick(picked, cx);
                if let Some(detail) = failure {
                    Sai::log(LOG_NAME, move || detail, SaiLogKind::Debug, cx);
                    if let Some(on_failure) = on_failure {
                        on_failure(cx);
                    }
                }
            });
        })
    }
}
