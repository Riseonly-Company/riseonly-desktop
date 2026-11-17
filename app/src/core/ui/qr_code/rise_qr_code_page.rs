use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AssetSource, Bounds, Context, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, Pixels, RenderImage,
    SharedString, Size, Subscription, Task, Window, size, white,
};
use image::ImageFormat;
use rise_core::foundation::foundation_date::RiseClock;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle, NotificationFeedbackGenerator, NotificationKind};
use rise_platform::system_share::ShareItem;
use rise_theme::{AppTheme, RiseThemeAppearance, ThemeStore};
use rise_ui::bottom_sheet_ui::modal_sheet_surface::ModalSheetMetrics;
use rise_ui::button_ui::ButtonUi;
use rise_ui::icon_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::qr_code::rise_qr_code::{RiseQRCodeBitmap, RiseQRCodeGenerator, RiseQRCodeMatrix, RiseQRCodeRenderer};
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::{Animated, MotionTime};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::push_deep_link_store::PushDeepLinkStore;
use crate::core::ui::qr_code::rise_qr_code_card::{
    RiseQRCodeCardFace, RiseQRCodeCardMetrics, RiseQRCodeSubject, RiseQRCodeTheme,
};
use crate::core::ui::qr_code::rise_qr_code_export::RiseQRCodeExport;
use crate::core::ui::qr_code::rise_qr_scanner::RiseQRScanResolver;
use crate::core::ui::qr_code::rise_qr_scanner_page::RiseQRPhotoPicker;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::sheets::share_sheet::SystemShare;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::{
    ChatDecorativeBackdrop, ChatWallpaperRasterFrame,
};
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::{
    ChatWallpaperPreset, ChatWallpaperStore,
};

#[derive(Clone, Copy, Debug)]
struct CircleButtonColors {
    foreground: Hsla,
    tint: Hsla,
    fill: Hsla,
}

#[derive(Clone, PartialEq, Debug)]
struct TaskKey {
    link: String,
    theme_id: String,
    side: f32,
}

pub struct RiseQRCodePage {
    subject: RiseQRCodeSubject,
    theme_id: String,
    matrix: Option<RiseQRCodeMatrix>,
    card_image: Option<Arc<RenderImage>>,
    card_side: f32,
    card_stack_height: f32,
    measured_fit: f32,
    code_area_size: Size<Pixels>,
    is_preparing_share: bool,
    appearance: Option<RiseThemeAppearance>,
    appearance_mix: Animated<f32>,
    selection_strokes: HashMap<String, Animated<f32>>,
    nav_store: Option<Entity<NavigationStore>>,
    on_close: Option<Rc<dyn Fn(&mut App)>>,
    focus_handle: FocusHandle,
    render_key: Option<TaskKey>,
    render_task: Option<Task<()>>,
    share_task: Option<Task<()>>,
    scan_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl RiseQRCodePage {
    const ESCAPE_KEY: &'static str = "escape";
    const MAXIMUM_CARD_WIDTH: f32 = 272.0;
    const MINIMUM_AVATAR: f32 = 60.0;
    const TILE_SIZE: (f32, f32) = (84.0, 104.0);
    const TILE_CORNER_RADIUS: f32 = 16.0;
    const APP_LOGO_ASSET: &'static str = "images/AppLogo.png";
    const SELECTION_DURATION: f64 = 0.18;
    const APPEARANCE_DURATION: f64 = 0.2;

    pub fn new(subject: RiseQRCodeSubject, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let theme_id = RiseQRCodeTheme::initial_id(cx);
        let theme_store = ThemeStore::shared(cx);
        let wallpaper_store = ChatWallpaperStore::shared(cx);
        let subscriptions = vec![
            cx.observe(&theme_store, |_, _, cx| cx.notify()),
            cx.observe(&wallpaper_store, |_, _, cx| cx.notify()),
        ];
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let appearance_mix = Animated::new(Self::mix_of(ThemeStore::shared(cx).read(cx).resolved_appearance()));
        Self {
            subject,
            theme_id,
            matrix: None,
            card_image: None,
            card_side: 0.0,
            card_stack_height: 0.0,
            measured_fit: 1.0,
            code_area_size: Size::default(),
            is_preparing_share: false,
            appearance: None,
            appearance_mix,
            selection_strokes: HashMap::new(),
            nav_store: None,
            on_close: None,
            focus_handle,
            render_key: None,
            render_task: None,
            share_task: None,
            scan_task: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn present_as_modal(&mut self, nav_store: Option<Entity<NavigationStore>>, on_close: Rc<dyn Fn(&mut App)>) {
        self.nav_store = nav_store;
        self.on_close = Some(on_close);
    }

    fn qr_theme(&self) -> RiseQRCodeTheme {
        RiseQRCodeTheme::theme(&self.theme_id)
    }

    fn card_appearance(&self, cx: &mut App) -> RiseThemeAppearance {
        self.appearance.unwrap_or_else(|| ThemeStore::shared(cx).read(cx).resolved_appearance())
    }

    fn mix_of(appearance: RiseThemeAppearance) -> f32 {
        if appearance == RiseThemeAppearance::Dark { 1.0 } else { 0.0 }
    }

    fn code_area(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let area_width = f32::from(self.code_area_size.width);
        let area_height = f32::from(self.code_area_size.height);
        let width = (area_width * RiseQRCodeCardMetrics::CANVAS_CARD_RATIO).min(Self::MAXIMUM_CARD_WIDTH);
        let side = RiseQRCodeCardMetrics::code_side(width);
        self.update_card_side(side, cx);

        let available = (area_height - 16.0).max(0.0);
        let natural_height = if self.measured_fit > 0.0 { self.card_stack_height / self.measured_fit } else { 0.0 };
        let fit = if natural_height > available && natural_height > 0.0 { available / natural_height } else { 1.0 };
        let shown_width = width * fit;
        let avatar = Self::MINIMUM_AVATAR.max(width * RiseQRCodeCardMetrics::AVATAR_RATIO) * fit;
        let weak = cx.weak_entity();

        div()
            .flex_1()
            .w_full()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .relative()
                    .flex_none()
                    .child(
                        zstack(Alignment::Top)
                            .child(
                                div()
                                    .pt(px(RiseQRCodeCardMetrics::avatar_overlap(shown_width, avatar)))
                                    .child(RiseQRCodeCardFace::new(
                                        self.card_image.clone(),
                                        self.subject.caption(),
                                        self.qr_theme(),
                                        shown_width,
                                    )),
                            )
                            .child(
                                UserLogoUi::new("qr-code-avatar")
                                    .source(self.subject.avatar_url.clone().map(SharedString::from))
                                    .name(Some(SharedString::from(self.subject.title.clone())))
                                    .user_id(self.subject.identity_seed.clone().map(SharedString::from))
                                    .size(avatar)
                                    .show_border(false)
                                    .show_story_ring(false)
                                    .open_story_on_press(false),
                            ),
                    )
                    .child(measure("qr-code-stack", {
                        let weak = weak.clone();
                        move |bounds: Bounds<Pixels>, _: &mut Window, cx: &mut App| {
                            let _ = weak.update(cx, |page, cx| page.stack_measured(f32::from(bounds.size.height), fit, cx));
                        }
                    })),
            )
            .child(measure("qr-code-area", move |bounds: Bounds<Pixels>, _: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |page, cx| {
                    if page.code_area_size != bounds.size {
                        page.code_area_size = bounds.size;
                        cx.notify();
                    }
                });
            }))
    }

    fn stack_measured(&mut self, height: f32, fit: f32, cx: &mut Context<Self>) {
        if (height - self.card_stack_height).abs() <= 0.5 && fit == self.measured_fit {
            return;
        }
        self.card_stack_height = height;
        self.measured_fit = fit;
        cx.notify();
    }

    fn update_card_side(&mut self, side: f32, cx: &mut Context<Self>) {
        if (side - self.card_side).abs() <= 0.5 {
            return;
        }
        self.card_side = side;
        cx.notify();
    }

    fn panel(
        &self,
        theme: &AppTheme,
        appearance: RiseThemeAppearance,
        now: MotionTime,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let weak = cx.weak_entity();
        let pattern_intensity = ChatWallpaperStore::shared(cx).read(cx).pattern_intensity();
        let is_dark = appearance == RiseThemeAppearance::Dark;
        let mix = self.appearance_mix.frame_value(now, window);
        let circle_colors =
            CircleButtonColors { foreground: theme.text.primary, tint: theme.primary._100, fill: theme.bg._300 };
        let tiles: Vec<AnyElement> = RiseQRCodeTheme::all()
            .iter()
            .map(|item| self.theme_tile(item, theme, mix, pattern_intensity, now, window, cx).into_any_element())
            .collect();

        vstack(16.0)
            .w_full()
            .px(px(16.0))
            .pt(px(14.0))
            .pb(px(14.0))
            .bg(theme.bg._200)
            .rounded_tl(px(22.0))
            .rounded_tr(px(22.0))
            .when(self.on_close.is_some(), |panel| {
                panel
                    .rounded_bl(px(ModalSheetMetrics::CORNER_RADIUS))
                    .rounded_br(px(ModalSheetMetrics::CORNER_RADIUS))
            })
            .child(
                hstack(None)
                    .w_full()
                    .child(Self::circle_button("qr-code-close", vec![("xmark", 1.0)], tr("close"), circle_colors, false, {
                        let weak = weak.clone();
                        move |_: &mut Window, cx: &mut App| {
                            let _ = weak.update(cx, |page, cx| page.pop(cx));
                        }
                    }))
                    .child(spacer_min_w(8.0))
                    .child(
                        MainText::new(tr("qr_code_title"))
                            .px(17.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary),
                    )
                    .child(spacer_min_w(8.0))
                    .child(Self::circle_button(
                        "qr-code-appearance",
                        vec![("sun.max", mix), ("moon", 1.0 - mix)],
                        tr(if is_dark { "theme_light_full" } else { "theme_dark_full" }),
                        circle_colors,
                        false,
                        {
                            let weak = weak.clone();
                            move |_: &mut Window, cx: &mut App| {
                                let _ = weak.update(cx, |page, cx| page.toggle_appearance(cx));
                            }
                        },
                    )),
            )
            .child(
                div()
                    .id("qr-code-theme-tiles")
                    .w(px(0.0))
                    .min_w_full()
                    .overflow_x_scroll()
                    .child(hstack(10.0).px(px(2.0)).children(tiles)),
            )
            .child(
                ButtonUi::new("qr-code-share")
                    .is_pending(self.is_preparing_share)
                    .gap(8.0)
                    .on_press({
                        let weak = weak.clone();
                        move |_: &mut Window, cx: &mut App| {
                            let _ = weak.update(cx, |page, cx| page.share(cx));
                        }
                    })
                    .child(MainText::new(tr("share_action")).px(16.0).font_weight(FontWeight::SEMIBOLD).color(white())),
            )
            .child({
                let in_modal = self.on_close.is_some();
                plain("qr-code-open-scanner")
                        .w_full()
                        .on_press(move |_: &mut Window, cx: &mut App| {
                            let _ = weak.update(cx, |page, cx| {
                                if in_modal {
                                    page.scan_image(cx);
                                } else {
                                    page.open_scanner(cx);
                                }
                            });
                        })
                        .child(
                            hstack(8.0)
                                .w_full()
                                .justify_center()
                                .h(theme.button.height_300)
                                .bg(theme.bg._300)
                                .rounded_full()
                                .children(IconUi::sized("qrcode.viewfinder", px(17.0), theme.primary._100, false))
                                .child(
                                    MainText::new(tr("qr_code_scan"))
                                        .px(16.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(theme.primary._100),
                                ),
                        )
            })
    }

    fn circle_button(
        id: &'static str,
        icons: Vec<(&'static str, f32)>,
        label: String,
        colors: CircleButtonColors,
        tinted: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        let color = if tinted { colors.tint } else { colors.foreground };
        div().id(SharedString::from(format!("{id}-label"))).aria_label(label).child(
            plain(id).on_press(action).child(
                zstack(Alignment::Center)
                    .size(px(36.0))
                    .rounded_full()
                    .bg(colors.fill)
                    .children(icons.into_iter().filter(|(_, opacity)| *opacity > 0.0).map(|(icon, opacity)| {
                        div().opacity(opacity).children(IconUi::sized(icon, px(16.0), color, false))
                    })),
            ),
        )
    }

    fn theme_tile(
        &self,
        item: &RiseQRCodeTheme,
        theme: &AppTheme,
        mix: f32,
        pattern_intensity: f64,
        now: MotionTime,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let (width, height) = Self::TILE_SIZE;
        let is_selected = item.id == self.theme_id;
        let stroke = self
            .selection_strokes
            .get(&item.id)
            .map_or(if is_selected { 1.0 } else { 0.0 }, |stroke| stroke.frame_value(now, window));
        let raster = |appearance: RiseThemeAppearance, opacity: f32| {
            div().opacity(opacity).child(ChatWallpaperRasterFrame::new(
                item.preset(appearance).resolved(appearance),
                ChatWallpaperPreset::resolved_pattern_intensity(pattern_intensity, appearance),
                appearance,
                size(px(width), px(height)),
            )
            .corner_radius(Self::TILE_CORNER_RADIUS))
        };
        let id = item.id.clone();
        let weak = cx.weak_entity();

        plain(SharedString::from(format!("qr-code-theme-{}", item.id)))
            .on_press(move |_: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |page, cx| page.select_theme(id.clone(), cx));
            })
            .child(
                div()
                    .relative()
                    .size_full()
                    .child(
                        zstack(Alignment::Center)
                            .w(px(width))
                            .h(px(height))
                            .continuous_corners(Self::TILE_CORNER_RADIUS)
                            .overflow_hidden()
                            .when(mix < 1.0, |tile| tile.child(raster(RiseThemeAppearance::Light, 1.0)))
                            .when(mix > 0.0, |tile| tile.child(raster(RiseThemeAppearance::Dark, mix)))
                            .child(
                                zstack(Alignment::Center)
                                    .size(px(40.0))
                                    .continuous_corners(10.0)
                                    .bg(white())
                                    .children(IconUi::sized("qrcode", px(22.0), item.accent(), false)),
                            ),
                    )
                    .when(stroke > 0.0, |tile| {
                        tile.child(centred_stroke(
                            theme.primary._100.opacity(stroke),
                            2.0,
                            Shape::RoundedRectangle {
                                corner_radius: Self::TILE_CORNER_RADIUS,
                                style: RoundedCornerStyle::Continuous,
                            },
                        ))
                    }),
            )
    }

    fn pop(&mut self, cx: &mut Context<Self>) {
        if let Some(on_close) = self.on_close.clone() {
            on_close(cx);
            return;
        }
        if let Some(nav_store) = self.nav_store.clone() {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }

    fn open_scanner(&mut self, cx: &mut Context<Self>) {
        if let Some(on_close) = self.on_close.clone() {
            on_close(cx);
        }
        if let Some(nav_store) = self.nav_store.clone() {
            nav_store.update(cx, |nav_store, cx| {
                nav_store.push(RootRoute::QrScanner, NavigationStoreSettlingPush::Drop, cx);
            });
        }
    }

    fn scan_image(&mut self, cx: &mut Context<Self>) {
        if self.scan_task.is_some() {
            return;
        }
        let weak = cx.weak_entity();
        let refused = weak.clone();
        self.scan_task = Some(
            RiseQRPhotoPicker::new(move |picked, cx| {
                let _ = weak.update(cx, |page, cx| match picked {
                    Some(path) => page.scan_file(path, cx),
                    None => page.scan_task = None,
                });
            })
            .on_failure(move |cx| {
                let _ = refused.update(cx, |page, cx| {
                    page.scan_task = None;
                    page.present_scan_notice(tr("error"), tr("desktop_file_dialog_unavailable"), cx);
                });
            })
            .present(cx),
        );
    }

    fn scan_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.scan_task = Some(cx.spawn(async move |page, cx| {
            let payload = RiseAccountRuntime::current()
                .spawn_blocking(move || {
                    let image = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
                    RiseQRScanResolver::payload(&image)
                })
                .await
                .ok()
                .flatten();
            let _ = page.update(cx, |page, cx| {
                page.scan_task = None;
                page.handle_scanned(payload, cx);
            });
        }));
    }

    fn handle_scanned(&mut self, payload: Option<String>, cx: &mut Context<Self>) {
        let Some(payload) = payload else {
            NotificationFeedbackGenerator::new().notification_occurred(NotificationKind::Warning);
            self.present_scan_notice(tr("qr_code_not_found_title"), tr("qr_code_not_found_message"), cx);
            return;
        };
        let Some(link) = RiseQRScanResolver::deep_link(&payload, AppEnvironment::current()) else {
            NotificationFeedbackGenerator::new().notification_occurred(NotificationKind::Warning);
            self.present_scan_notice(tr("qr_code_unsupported_title"), tr("qr_code_unsupported_message"), cx);
            return;
        };
        NotificationFeedbackGenerator::new().notification_occurred(NotificationKind::Success);
        self.pop(cx);
        cx.defer(move |cx| {
            PushDeepLinkStore::shared(cx).update(cx, |store, cx| {
                store.enqueue(link, cx);
            });
        });
    }

    fn present_scan_notice(&self, title: String, message: String, cx: &mut Context<Self>) {
        NativeConfirmAlert::present_notice(&title, &message, None, None, |_| {}, cx);
    }

    fn select_theme(&mut self, id: String, cx: &mut Context<Self>) {
        if id == self.theme_id {
            return;
        }
        let now = RiseClock::monotonic();
        let spec = ease_in_out(Self::SELECTION_DURATION);
        let previous = std::mem::replace(&mut self.theme_id, id.clone());
        self.selection_strokes.entry(previous).or_insert_with(|| Animated::new(1.0)).set_in(0.0, spec, now, cx);
        self.selection_strokes.entry(id).or_insert_with(|| Animated::new(0.0)).set_in(1.0, spec, now, cx);
        cx.notify();
        ImpactFeedbackGenerator::new(ImpactStyle::Light).impact_occurred();
    }

    /// shares a photo file and not a link next to an image
    /// a link as a second item made some apps pick it over the picture and send text
    /// the link is still in the code on the card
    fn share(&mut self, cx: &mut Context<Self>) {
        if self.is_preparing_share {
            return;
        }
        self.is_preparing_share = true;
        cx.notify();

        let subject = self.subject.clone();
        let theme = self.qr_theme();
        let exported = self.card_appearance(cx);
        let intensity = ChatWallpaperStore::shared(cx).read(cx).pattern_intensity();
        let source = self.matrix.clone();

        self.share_task = Some(cx.spawn(async move |page, cx| {
            let file = RiseQRCodeExport::jpeg_file(subject, theme, exported, intensity, source, cx).await;
            let _ = page.update(cx, |page, cx| {
                page.is_preparing_share = false;
                cx.notify();
                let Some(file) = file else {
                    Notifier::send(
                        tr("qr_code_share_failed"),
                        NotifierSendOptions {
                            r#type: NotifierType::Error,
                            position: NotifierPosition::Bottom,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                    return;
                };
                SystemShare::present(vec![ShareItem::File(file)], None, cx);
            });
        }));
    }

    fn toggle_appearance(&mut self, cx: &mut Context<Self>) {
        let next = if self.card_appearance(cx) == RiseThemeAppearance::Dark {
            RiseThemeAppearance::Light
        } else {
            RiseThemeAppearance::Dark
        };
        self.appearance = Some(next);
        self.appearance_mix.set_in(Self::mix_of(next), ease_in_out(Self::APPEARANCE_DURATION), RiseClock::monotonic(), cx);
        cx.notify();
        ImpactFeedbackGenerator::new(ImpactStyle::Light).impact_occurred();
    }

    fn key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.key == Self::ESCAPE_KEY {
            cx.stop_propagation();
            self.pop(cx);
        }
    }

    fn render_card_if_needed(&mut self, window: &Window, cx: &mut Context<Self>) {
        let key = TaskKey {
            link: self.subject.link.absolute_string().to_owned(),
            theme_id: self.theme_id.clone(),
            side: self.card_side,
        };
        if self.render_key.as_ref() == Some(&key) {
            return;
        }
        self.render_key = Some(key);
        self.render_task = None;
        if self.card_side <= 1.0 {
            return;
        }
        let link = self.subject.link.absolute_string().to_owned();
        let style = self.qr_theme().style();
        let side = self.card_side;
        let scale = window.scale_factor();
        let known = self.matrix.clone();
        let assets = Arc::clone(cx.asset_source());

        self.render_task = Some(cx.spawn(async move |page, cx| {
            let produced = RiseAccountRuntime::current()
                .spawn_blocking(move || -> Option<(RiseQRCodeMatrix, RiseQRCodeBitmap)> {
                    let matrix = known.or_else(|| RiseQRCodeGenerator::matrix(&link))?;
                    let logo = Self::app_logo(assets.as_ref());
                    let image = RiseQRCodeRenderer::image(&matrix, side, scale, &style, logo.as_ref())?;
                    Some((matrix, image))
                })
                .await
                .ok()
                .flatten();
            let Some((matrix, bitmap)) = produced else {
                return;
            };
            let _ = page.update(cx, |page, cx| {
                page.matrix = Some(matrix);
                page.card_image = RiseQRCodeCardFace::code_image(&bitmap);
                cx.notify();
            });
        }));
    }

    fn app_logo(assets: &dyn AssetSource) -> Option<RiseQRCodeBitmap> {
        let bytes = assets.load(Self::APP_LOGO_ASSET).ok().flatten()?;
        let logo = image::load_from_memory_with_format(&bytes, ImageFormat::Png).ok()?.to_rgba8();
        let (width, height) = logo.dimensions();
        RiseQRCodeBitmap::new(width, height, 1, logo.into_raw())
    }
}

impl Render for RiseQRCodePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.on_close.is_none() {
            self.nav_store = env::<NavigationStoreKey>(cx);
        }
        let theme = rise_ui::theme(cx).clone();
        let appearance = self.card_appearance(cx);
        let now = RiseClock::monotonic();
        if *self.appearance_mix.target() != Self::mix_of(appearance) {
            self.appearance_mix.jump(Self::mix_of(appearance));
        }
        let preset = self.qr_theme().preset(appearance);
        let backdrop_radius = if self.on_close.is_some() { ModalSheetMetrics::CORNER_RADIUS } else { 0.0 };
        let code_area = self.code_area(cx);
        self.render_card_if_needed(window, cx);

        div()
            .id("qr-code-page")
            .size_full()
            .relative()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|page, event: &KeyDownEvent, _, cx| page.key_down(event, cx)))
            .child(
                div().absolute().inset_0().child(
                    ChatDecorativeBackdrop::new()
                        .preview_preset(Some(preset))
                        .appearance_override(Some(appearance))
                        .corner_radius(backdrop_radius),
                ),
            )
            .child(
                vstack(0.0)
                    .size_full()
                    .child(code_area)
                    .child(self.panel(&theme, appearance, now, window, cx)),
            )
    }
}
