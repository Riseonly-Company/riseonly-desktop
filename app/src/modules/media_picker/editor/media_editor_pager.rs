use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Edges, Entity, ImageSource, ObjectFit, RenderImage, Size, WeakEntity, Window, img, size};
use rise_platform::photo_library::{ImageRequestId, LibraryAsset};
use rise_ui::swiftui::modifiers::ViewModifier;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerImageMemoryCache;
use crate::modules::chat::components::chat::bubble::media_letterbox_photo::MediaAspectFit;
use crate::modules::media_picker::editor::editor_image_layout::EditorImageLayout;
use crate::modules::media_picker::editor::editor_mode::EditorMode;
use crate::modules::media_picker::library::photo_library_fetcher::PhotoLibraryFetcher;

#[derive(Clone)]
pub struct MediaEditorPagerPage {
    pub asset: LibraryAsset,
    pub index: i64,
    pub image: Option<Arc<RenderImage>>,
}

/// horizontal paging between library assets in the picker editor
/// only the offset layer and neighbour previews read this
/// so a finger step changes nothing in the editor body
pub struct MediaEditorPager {
    pub offset: f32,
    pub chrome_height: f32,
    previous: Option<MediaEditorPagerPage>,
    next: Option<MediaEditorPagerPage>,
    arrival_image: Option<Arc<RenderImage>>,

    generation: u64,
    arrival_token: u64,
    requests: Vec<Option<ImageRequestId>>,
    fetcher: Option<WeakEntity<PhotoLibraryFetcher>>,
}

impl MediaEditorPager {
    pub const PAGE_GAP: f32 = 20.0;

    const ARRIVAL_SECONDS: f64 = 0.35;

    pub fn new() -> Self {
        Self {
            offset: 0.0,
            chrome_height: 0.0,
            previous: None,
            next: None,
            arrival_image: None,
            generation: 0,
            arrival_token: 0,
            requests: Vec::new(),
            fetcher: None,
        }
    }

    pub fn previous(&self) -> Option<&MediaEditorPagerPage> {
        self.previous.as_ref()
    }

    pub fn next(&self) -> Option<&MediaEditorPagerPage> {
        self.next.as_ref()
    }

    pub fn arrival_image(&self) -> Option<&Arc<RenderImage>> {
        self.arrival_image.as_ref()
    }

    pub fn prepare_neighbours(
        &mut self,
        asset: &LibraryAsset,
        hint_index: Option<i64>,
        fetcher: &Entity<PhotoLibraryFetcher>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_requests(cx);
        self.generation = self.generation.wrapping_add(1);
        self.fetcher = Some(fetcher.downgrade());
        cx.notify();
        let Some(index) = Self::resolve_index(asset, hint_index, fetcher.read(cx)) else {
            self.previous = None;
            self.next = None;
            return;
        };
        self.previous = self.page(index - 1, fetcher, window, cx);
        self.next = self.page(index + 1, fetcher, window, cx);
    }

    pub fn show_arrival(&mut self, image: Option<Arc<RenderImage>>, cx: &mut Context<Self>) {
        self.arrival_token = self.arrival_token.wrapping_add(1);
        self.arrival_image = image;
        cx.notify();
        let expected = self.arrival_token;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(Self::ARRIVAL_SECONDS)).await;
            let _ = this.update(cx, |pager, cx| {
                if pager.arrival_token != expected {
                    return;
                }
                pager.arrival_image = None;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn clear_arrival(&mut self, cx: &mut Context<Self>) {
        self.arrival_token = self.arrival_token.wrapping_add(1);
        if self.arrival_image.is_none() {
            return;
        }
        self.arrival_image = None;
        cx.notify();
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.cancel_requests(cx);
        self.generation = self.generation.wrapping_add(1);
        self.arrival_token = self.arrival_token.wrapping_add(1);
        self.offset = 0.0;
        self.previous = None;
        self.next = None;
        self.arrival_image = None;
        cx.notify();
    }

    fn page(
        &mut self,
        index: i64,
        fetcher: &Entity<PhotoLibraryFetcher>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<MediaEditorPagerPage> {
        let asset = fetcher.read(cx).asset(index)?.clone();
        let key = format!("ph://{}", asset.id);
        if let Some(cached) =
            MediaViewerImageMemoryCache::memory_only(&key, SaiAccountCacheScope::active_user_id().as_deref())
        {
            return Some(MediaEditorPagerPage { asset, index, image: Some(cached) });
        }
        let expected = self.generation;
        let id = asset.id.clone();
        let this = cx.weak_entity();
        let request = fetcher.update(cx, |fetcher, cx| {
            fetcher.request_hero_thumbnail(
                &asset,
                window,
                move |image, cx| {
                    let Some(image) = image else {
                        return;
                    };
                    let _ = this.update(cx, |pager, cx| {
                        if pager.generation != expected {
                            return;
                        }
                        for page in [pager.previous.as_mut(), pager.next.as_mut()].into_iter().flatten() {
                            if page.asset.id == id {
                                page.image = Some(Arc::clone(&image));
                            }
                        }
                        cx.notify();
                    });
                },
                cx,
            )
        });
        self.requests.push(request);
        Some(MediaEditorPagerPage { asset, index, image: None })
    }

    fn cancel_requests(&mut self, cx: &mut Context<Self>) {
        let requests = std::mem::take(&mut self.requests);
        let Some(fetcher) = self.fetcher.as_ref().and_then(WeakEntity::upgrade) else {
            return;
        };
        fetcher.update(cx, |fetcher, _| {
            for request in requests {
                fetcher.cancel_image_request(request);
            }
        });
    }

    fn resolve_index(asset: &LibraryAsset, hint: Option<i64>, fetcher: &PhotoLibraryFetcher) -> Option<i64> {
        if let Some(hint) = hint
            && fetcher.asset(hint).is_some_and(|hinted| hinted.id == asset.id)
        {
            return Some(hint);
        }
        let found = fetcher.fetch_result().iter().position(|candidate| candidate.id == asset.id)?;
        i64::try_from(found).ok()
    }
}

impl Default for MediaEditorPager {
    fn default() -> Self {
        Self::new()
    }
}

/// moves only the current pages stage with the finger, editor chrome stays put
pub struct MediaEditorPageOffset {
    pub pager: Option<Entity<MediaEditorPager>>,
}

impl ViewModifier for MediaEditorPageOffset {
    fn body(self, content: AnyElement, _window: &mut Window, _cx: &mut App) -> AnyElement {
        match self.pager {
            Some(pager) => MediaEditorPageOffsetBody { pager, content }.into_any_element(),
            None => content,
        }
    }
}

#[derive(IntoElement)]
struct MediaEditorPageOffsetBody {
    pager: Entity<MediaEditorPager>,
    content: AnyElement,
}

impl RenderOnce for MediaEditorPageOffsetBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        div().relative().size_full().left(px(self.pager.read(cx).offset)).child(self.content)
    }
}

/// neighbour previews sliding in beside the current page
/// laid out with the stages initial mode insets so the handoff to the next editor doesnt jump
#[derive(IntoElement)]
pub struct MediaEditorPagerNeighbours {
    pub pager: Entity<MediaEditorPager>,
    pub size: Size<f32>,
    pub safe_insets: Edges<f32>,
}

impl MediaEditorPagerNeighbours {
    fn preview(&self, image: &Arc<RenderImage>, shift: f32, pager: &MediaEditorPager) -> AnyElement {
        let pixel_size = image.size(0);
        let image_size = size(pixel_size.width.0 as f32, pixel_size.height.0 as f32);
        let insets = EditorImageLayout::compute_insets(
            self.size,
            image_size,
            self.safe_insets.top,
            self.safe_insets.bottom,
            0.0,
            0.0,
            EditorMode::Initial,
            pager.chrome_height,
        );
        let rect = MediaAspectFit::centered_fit_rect(
            size(px(image_size.width), px(image_size.height)),
            size(px(self.size.width), px(self.size.height)),
            Edges { top: px(insets.top), left: px(insets.left), bottom: px(insets.bottom), right: px(insets.right) },
        );
        img(ImageSource::Render(Arc::clone(image)))
            .absolute()
            .left(rect.origin.x + px(shift + pager.offset))
            .top(rect.origin.y)
            .w(rect.size.width)
            .h(rect.size.height)
            .object_fit(ObjectFit::Contain)
            .into_any_element()
    }
}

impl RenderOnce for MediaEditorPagerNeighbours {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let pager = self.pager.read(cx);
        let stride = self.size.width + MediaEditorPager::PAGE_GAP;
        let arrival = pager.arrival_image().map(|image| self.preview(image, 0.0, pager));
        let previous = pager
            .previous()
            .and_then(|page| page.image.as_ref())
            .map(|image| self.preview(image, -stride, pager));
        let next = pager
            .next()
            .and_then(|page| page.image.as_ref())
            .map(|image| self.preview(image, stride, pager));
        pass_through()
            .relative()
            .w(px(self.size.width))
            .h(px(self.size.height))
            .children(arrival)
            .children(previous)
            .children(next)
    }
}
