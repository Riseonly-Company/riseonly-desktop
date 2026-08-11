use gpui::{
    App, Hsla, ImageSource, Img, IntoElement, ObjectFit, Resource, SharedString, SharedUri,
    StyledImage, div, img, prelude::*,
};
use rise_theme::AppTheme;

use super::rise_image_cache::RiseImageCache;

/// A remote image, over the application's bounded cache.
///
/// No disk cache — gpui's loader is memory-only, so a relaunch refetches — and no
/// downsampling: a 4000px source in a 600px card costs its full native decode.
pub struct ImageUi;

impl ImageUi {
    /// An image fetched over the network, filling the box and clipping the
    /// overflow. The caller still sets the box: this decides nothing about size.
    pub fn remote(url: impl Into<SharedUri>, cx: &App) -> Img {
        let source = ImageSource::Resource(Resource::Uri(url.into()));
        Self::with_cache(img(source), cx).object_fit(ObjectFit::Cover)
    }

    /// An image that ships inside the bundle.
    pub fn bundled(path: impl Into<SharedString>, cx: &App) -> Img {
        let source = ImageSource::Resource(Resource::Embedded(path.into()));
        Self::with_cache(img(source), cx).object_fit(ObjectFit::Cover)
    }

    /// A remote image with the two states it can be in besides loaded.
    ///
    /// Both placeholders are drawn at the box's own size, so the row's geometry
    /// is settled before the bytes arrive and never corrected afterwards.
    pub fn remote_with_states(theme: &AppTheme, url: impl Into<SharedUri>, cx: &App) -> Img {
        let placeholder = theme.bg._300;
        let failed_tint = theme.text.secondary;
        let icon_size = theme.icon.large;

        Self::remote(url, cx)
            .with_loading(move || Self::placeholder(placeholder).into_any_element())
            .with_fallback(move || {
                Self::failed(placeholder, failed_tint, icon_size).into_any_element()
            })
    }

    fn with_cache(element: Img, cx: &App) -> Img {
        match RiseImageCache::shared(cx) {
            Some(cache) => element.image_cache(&cache),
            // gpui's own global cache stands in, unbounded: storybook and tests only, never the app.
            None => element,
        }
    }

    fn placeholder(color: Hsla) -> impl IntoElement {
        div().size_full().bg(color)
    }

    fn failed(background: Hsla, tint: Hsla, icon_size: gpui::Pixels) -> impl IntoElement {
        let mut cell = div()
            .size_full()
            .bg(background)
            .flex()
            .items_center()
            .justify_center();

        if let Some(icon) = crate::IconUi::sized("photo", icon_size, tint, false) {
            cell = cell.child(icon);
        }

        cell
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_failure_state_names_an_icon_the_bundle_carries() {
        assert!(
            crate::IconUi::asset_path("photo").is_some(),
            "a broken image would draw an empty box that looks like a design decision"
        );
    }

    #[gpui::test]
    fn an_image_built_without_a_cache_installed_still_renders(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            assert!(RiseImageCache::shared(cx).is_none());
            let _ = ImageUi::remote("https://example.invalid/a.png", cx);
        });
    }
}
