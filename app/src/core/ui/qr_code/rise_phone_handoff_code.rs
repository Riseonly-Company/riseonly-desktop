use std::sync::Arc;

use gpui::{App, AssetSource, Context, ElementId, RenderImage, SharedString, Task, Window, img};
use image::ImageFormat;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_ui::qr_code::rise_qr_code::{RiseQRCodeBitmap, RiseQRCodeGenerator, RiseQRCodeRenderer, RiseQRCodeStyle};
use rise_ui::swiftui::prelude::*;

use crate::core::ui::qr_code::rise_qr_code_card::RiseQRCodeCardFace;

const APP_LOGO_ASSET: &str = "images/AppLogo.png";

/// what the raster was made for, the link and side and the window scale factor
#[derive(Clone, PartialEq)]
struct RisePhoneHandoffCodeKey {
    link: SharedString,
    side: f32,
    scale: f32,
}

#[derive(Default)]
struct RisePhoneHandoffCodeState {
    key: Option<RisePhoneHandoffCodeKey>,
    image: Option<Arc<RenderImage>>,
    render_task: Option<Task<()>>,
}

impl RisePhoneHandoffCodeState {
    fn render_if_needed(&mut self, key: RisePhoneHandoffCodeKey, cx: &mut Context<Self>) {
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.key = Some(key.clone());
        let assets = Arc::clone(cx.asset_source());
        self.render_task = Some(cx.spawn(async move |state, cx| {
            let Some(image) = RisePhoneHandoffCode::raster(key.link, key.side, key.scale, assets).await else {
                return;
            };
            let _ = state.update(cx, |state, cx| {
                state.image = Some(image);
                cx.notify();
            });
        }));
    }
}

fn app_logo(assets: &dyn AssetSource) -> Option<RiseQRCodeBitmap> {
    let bytes = assets.load(APP_LOGO_ASSET).ok().flatten()?;
    let logo = image::load_from_memory_with_format(&bytes, ImageFormat::Png).ok()?.to_rgba8();
    let (width, height) = logo.dimensions();
    RiseQRCodeBitmap::new(width, height, 1, logo.into_raw())
}

#[derive(IntoElement)]
pub struct RisePhoneHandoffCode {
    id: ElementId,
    link: SharedString,
    side: f32,
    prerendered: Option<Arc<RenderImage>>,
}

impl RisePhoneHandoffCode {
    pub fn new(id: impl Into<ElementId>, link: impl Into<SharedString>, side: f32) -> Self {
        Self { id: id.into(), link: link.into(), side, prerendered: None }
    }

    /// a code the owner already rastered with raster, drawn as is with no work of its own
    pub fn prerendered(mut self, image: Option<Arc<RenderImage>>) -> Self {
        self.prerendered = image;
        self
    }

    /// the code of link with the app logo, side points at scale, rastered off the main thread
    pub async fn raster(
        link: SharedString,
        side: f32,
        scale: f32,
        assets: Arc<dyn AssetSource>,
    ) -> Option<Arc<RenderImage>> {
        let bitmap = RiseAccountRuntime::current()
            .spawn_blocking(move || -> Option<RiseQRCodeBitmap> {
                let matrix = RiseQRCodeGenerator::matrix(&link)?;
                let logo = app_logo(assets.as_ref());
                RiseQRCodeRenderer::image(&matrix, side, scale, &RiseQRCodeStyle::fallback(), logo.as_ref())
            })
            .await
            .ok()
            .flatten()?;
        RiseQRCodeCardFace::code_image(&bitmap)
    }
}

impl RenderOnce for RisePhoneHandoffCode {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let image = match self.prerendered {
            Some(image) => Some(image),
            None => {
                let key = RisePhoneHandoffCodeKey { link: self.link, side: self.side, scale: window.scale_factor() };
                let state = window.use_keyed_state(self.id, cx, |_, _| RisePhoneHandoffCodeState::default());
                state.update(cx, |state, cx| state.render_if_needed(key, cx));
                state.read(cx).image.clone()
            }
        };
        div()
            .flex_none()
            .frame_size(self.side, self.side)
            .continuous_corners(14.0)
            .overflow_hidden()
            .children(image.map(|image| img(image).size_full()))
    }
}
