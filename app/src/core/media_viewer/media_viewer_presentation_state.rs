use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use futures::future::{Either, select};

use gpui::{App, AppContext, AsyncApp, Bounds, Context, Entity, Global, Pixels, WeakEntity, Window, WindowId};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::environment::{EnvKey, EnvScope, env, env_scope};
use rise_ui::window_screen_frame_reader::{MediaCellScreenFrameRegistry, WindowScreenCoordinates};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media_viewer::media_viewer_countdown_badge::MediaViewerCountdown;
use crate::core::media_viewer::media_viewer_image_cache::{MediaViewerBitmapLoader, MediaViewerImageMemoryCache};
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::media_viewer::media_viewer_types::{
    MediaViewerAccessoryAction, MediaViewerChrome, MediaViewerDeleteMode, MediaViewerFilmstripScope,
    MediaViewerGalleryItem, MediaViewerHeroSkin, MediaViewerTransition,
};
use crate::core::ui::bottom_sheet_ui::experimental_bottom_sheet_presentation_host::SheetLayerStack;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageDiskCache};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;

/// where the viewer draws
/// a viewer opened from inside a sheet must go on top of
/// the whole layer chain, the root host sits under sheets
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaViewerPresentationRoute {
    Inline,
    Modal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaViewerGalleryScope {
    Complete,
    Paged,
}

pub type MediaViewerItemHandler = Rc<dyn Fn(&MediaViewerGalleryItem, &mut App)>;
pub type MediaViewerDeleteHandler = Rc<dyn Fn(&MediaViewerGalleryItem, MediaViewerDeleteMode, &mut App)>;
pub type MediaViewerFlybackTargetProvider = Rc<dyn Fn(&str, &mut App) -> Option<Bounds<Pixels>>>;
/// handler bound to its item, handed to a button
pub type MediaViewerBoundAction = Rc<dyn Fn(&mut App)>;

/// callbacks of present, all empty unless the presenting screen gives one
#[derive(Clone, Default)]
pub struct MediaViewerPresentationHandlers {
    pub on_delete: Option<MediaViewerDeleteHandler>,
    pub on_forward: Option<MediaViewerItemHandler>,
    pub on_report: Option<MediaViewerItemHandler>,
    pub on_show_in_context: Option<MediaViewerItemHandler>,
    pub on_reply: Option<MediaViewerItemHandler>,
    pub on_set_as_main: Option<MediaViewerItemHandler>,
    pub on_delete_photo: Option<MediaViewerItemHandler>,
    pub accessory: Option<MediaViewerAccessoryAction>,
    pub flyback_target_provider: Option<MediaViewerFlybackTargetProvider>,
}

pub struct MediaViewerPresentationState {
    items: Vec<MediaViewerGalleryItem>,
    gallery_scope: MediaViewerGalleryScope,
    presentation_route: MediaViewerPresentationRoute,
    current_index: usize,
    source_frame: Option<Bounds<Pixels>>,
    hero_lift_id: Option<String>,
    bubble_cell_hidden: bool,
    hero_skin: Option<MediaViewerHeroSkin>,
    hero_warm_ui_image: Option<CleverImage>,
    hero_warm_source: Option<String>,
    hero_warm_item_id: Option<String>,
    is_presented: bool,
    presentation_id: Option<Uuid>,
    closing_presentation_id: Option<Uuid>,
    presentation_account_user_id: Option<String>,
    presentation_account_epoch: Option<String>,
    chrome: MediaViewerChrome,
    gallery_position: Option<i64>,
    gallery_total: Option<i64>,
    gallery_counter_item_id: Option<String>,
    gallery_counter_follows_index: bool,
    on_delete: Option<MediaViewerDeleteHandler>,
    on_forward: Option<MediaViewerItemHandler>,
    on_report: Option<MediaViewerItemHandler>,
    on_show_in_context: Option<MediaViewerItemHandler>,
    on_reply: Option<MediaViewerItemHandler>,
    on_set_as_main: Option<MediaViewerItemHandler>,
    on_delete_photo: Option<MediaViewerItemHandler>,
    accessory: Option<MediaViewerAccessoryAction>,
    dismissal_request_id: Option<Uuid>,
    countdown: Option<MediaViewerCountdown>,
    closes_without_flyback: bool,
    on_close_begin: Option<Rc<dyn Fn(&mut App)>>,
    flyback_target_provider: Option<MediaViewerFlybackTargetProvider>,
    prepared_flyback_frame: Option<(String, Bounds<Pixels>)>,
}

impl Default for MediaViewerPresentationState {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaViewerPresentationState {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            gallery_scope: MediaViewerGalleryScope::Complete,
            presentation_route: MediaViewerPresentationRoute::Inline,
            current_index: 0,
            source_frame: None,
            hero_lift_id: None,
            bubble_cell_hidden: false,
            hero_skin: None,
            hero_warm_ui_image: None,
            hero_warm_source: None,
            hero_warm_item_id: None,
            is_presented: false,
            presentation_id: None,
            closing_presentation_id: None,
            presentation_account_user_id: None,
            presentation_account_epoch: None,
            chrome: MediaViewerChrome::CHAT,
            gallery_position: None,
            gallery_total: None,
            gallery_counter_item_id: None,
            gallery_counter_follows_index: false,
            on_delete: None,
            on_forward: None,
            on_report: None,
            on_show_in_context: None,
            on_reply: None,
            on_set_as_main: None,
            on_delete_photo: None,
            accessory: None,
            dismissal_request_id: None,
            countdown: None,
            closes_without_flyback: false,
            on_close_begin: None,
            flyback_target_provider: None,
            prepared_flyback_frame: None,
        }
    }

    pub fn items(&self) -> &[MediaViewerGalleryItem] {
        &self.items
    }

    pub fn gallery_scope(&self) -> MediaViewerGalleryScope {
        self.gallery_scope
    }

    pub fn presentation_route(&self) -> MediaViewerPresentationRoute {
        self.presentation_route
    }

    pub fn current_index(&self) -> usize {
        self.current_index
    }

    pub fn set_current_index(&mut self, current_index: usize, cx: &mut Context<Self>) {
        let previous_index = self.current_index;
        self.current_index = current_index;
        if previous_index != current_index {
            self.step_gallery_counter(previous_index);
        }
        cx.notify();
    }

    pub fn set_gallery_counter_follows_index(&mut self, follows: bool) {
        self.gallery_counter_follows_index = follows;
    }

    fn step_gallery_counter(&mut self, previous_index: usize) {
        if !self.gallery_counter_follows_index || self.gallery_scope != MediaViewerGalleryScope::Paged {
            return;
        }
        let (Some(position), Some(total)) = (self.gallery_position, self.gallery_total) else {
            return;
        };
        let previous_id = self.items.get(previous_index).map(|item| item.id.as_str());
        if previous_id.is_none() || self.gallery_counter_item_id.as_deref() != previous_id {
            return;
        }
        let delta = self.current_index as i64 - previous_index as i64;
        let next = position.saturating_add(delta);
        match self.current_item_id() {
            Some(item_id) if next > 0 && next <= total => {
                self.gallery_position = Some(next);
                self.gallery_counter_item_id = Some(item_id);
            }
            _ => {
                self.gallery_position = None;
                self.gallery_total = None;
                self.gallery_counter_item_id = None;
            }
        }
    }

    pub fn source_frame(&self) -> Option<Bounds<Pixels>> {
        self.source_frame
    }

    pub fn hero_lift_id(&self) -> Option<&str> {
        self.hero_lift_id.as_deref()
    }

    pub fn bubble_cell_hidden(&self) -> bool {
        self.bubble_cell_hidden
    }

    pub fn hero_skin(&self) -> Option<&MediaViewerHeroSkin> {
        self.hero_skin.as_ref()
    }

    pub fn hero_warm_ui_image(&self) -> Option<&CleverImage> {
        self.hero_warm_ui_image.as_ref()
    }

    pub fn is_presented(&self) -> bool {
        self.is_presented
    }

    /// true while the viewer is up over the page and not flying back, media under it is out of sight
    pub fn covers_page(&self) -> bool {
        self.is_presented
            && !self.items().is_empty()
            && !self.presentation_id().is_some_and(|id| self.is_closing_presentation(id))
    }

    pub fn presentation_id(&self) -> Option<Uuid> {
        self.presentation_id
    }

    pub fn chrome(&self) -> MediaViewerChrome {
        self.chrome
    }

    pub fn gallery_position(&self) -> Option<i64> {
        self.gallery_position
    }

    pub fn gallery_total(&self) -> Option<i64> {
        self.gallery_total
    }

    pub fn gallery_counter_item_id(&self) -> Option<&str> {
        self.gallery_counter_item_id.as_deref()
    }

    pub fn accessory(&self) -> Option<&MediaViewerAccessoryAction> {
        self.accessory.as_ref()
    }

    pub fn dismissal_request_id(&self) -> Option<Uuid> {
        self.dismissal_request_id
    }

    pub fn countdown(&self) -> Option<&MediaViewerCountdown> {
        self.countdown.as_ref()
    }

    pub fn set_countdown(&mut self, countdown: Option<MediaViewerCountdown>, cx: &mut Context<Self>) {
        self.countdown = countdown;
        cx.notify();
    }

    pub fn closes_without_flyback(&self) -> bool {
        self.closes_without_flyback
    }

    pub fn set_closes_without_flyback(&mut self, closes_without_flyback: bool, cx: &mut Context<Self>) {
        self.closes_without_flyback = closes_without_flyback;
        cx.notify();
    }

    pub fn set_on_close_begin(&mut self, on_close_begin: Option<Rc<dyn Fn(&mut App)>>, _cx: &mut Context<Self>) {
        self.on_close_begin = on_close_begin;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn present(
        &mut self,
        items: Vec<MediaViewerGalleryItem>,
        index: usize,
        source_frame: Option<Bounds<Pixels>>,
        hero_lift_id: Option<String>,
        hero_skin: Option<MediaViewerHeroSkin>,
        chrome: MediaViewerChrome,
        gallery_scope: MediaViewerGalleryScope,
        handlers: MediaViewerPresentationHandlers,
        cx: &mut Context<Self>,
    ) {
        if items.is_empty() {
            return;
        }
        self.presentation_id = Some(Uuid::new_v4());
        self.closing_presentation_id = None;
        self.dismissal_request_id = None;
        self.closes_without_flyback = false;
        self.on_close_begin = None;
        self.presentation_account_user_id = SaiAccountCacheScope::active_user_id();
        self.presentation_account_epoch = SaiAccountCacheScope::active_epoch();
        let idx = index.min(items.len() - 1);
        let selected_id = items[idx].id.clone();
        let selected_registry_id = items[idx].frame_registry_id.clone();
        self.items = items;
        self.current_index = idx;
        self.source_frame = source_frame;
        self.hero_lift_id = Some(hero_lift_id.unwrap_or(selected_registry_id));
        self.bubble_cell_hidden = false;
        let hero_source = hero_skin.as_ref().map(|skin| skin.display_source.clone());
        self.hero_skin = hero_skin;
        if self.hero_warm_ui_image.is_some()
            && Self::is_same_source(self.hero_warm_source.as_deref(), hero_source.as_deref())
        {
            self.hero_warm_item_id = Some(selected_id);
        } else {
            self.clear_hero_warm_image();
        }
        self.chrome = chrome;
        self.gallery_scope = gallery_scope;
        let MediaViewerPresentationHandlers {
            on_delete,
            on_forward,
            on_report,
            on_show_in_context,
            on_reply,
            on_set_as_main,
            on_delete_photo,
            accessory,
            flyback_target_provider,
        } = handlers;
        self.on_delete = on_delete;
        self.on_forward = on_forward;
        self.on_report = on_report;
        self.on_show_in_context = on_show_in_context;
        self.on_reply = on_reply;
        self.on_set_as_main = on_set_as_main;
        self.on_delete_photo = on_delete_photo;
        self.accessory = accessory;
        self.flyback_target_provider = flyback_target_provider;
        self.prepared_flyback_frame = None;
        self.gallery_position = None;
        self.gallery_total = None;
        self.gallery_counter_item_id = None;
        self.gallery_counter_follows_index = false;
        self.presentation_route = Self::route_for_current_presentation(cx);
        self.is_presented = true;
        cx.notify();
    }

    fn route_for_current_presentation(cx: &App) -> MediaViewerPresentationRoute {
        if SheetLayerStack::top_is_root(cx) {
            MediaViewerPresentationRoute::Inline
        } else {
            MediaViewerPresentationRoute::Modal
        }
    }

    /// swaps in a longer list without moving the viewer page
    /// used when a profile album grows after opening on the avatar
    /// dont use present, it resets bubble_cell_hidden and the avatar reappears
    pub fn replace_items(
        &mut self,
        next: Vec<MediaViewerGalleryItem>,
        selecting: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if !self.is_presented || next.is_empty() {
            return;
        }
        let selected_id = selecting
            .map(str::to_owned)
            .or_else(|| self.items.get(self.current_index).map(|item| item.id.clone()));
        if next == self.items && selected_id.is_none() {
            return;
        }
        let found = selected_id
            .as_deref()
            .and_then(|selected_id| next.iter().position(|item| item.id == selected_id));
        let count = next.len();
        self.items = next;
        match found {
            Some(index) => self.current_index = index,
            None => self.current_index = self.current_index.min(count - 1),
        }
        cx.notify();
    }

    /// absolute position and total, none means not known yet
    /// only the owner of the paging writes it, dont guess it from items since that is a window
    pub fn set_gallery_counter(&mut self, position: Option<i64>, total: Option<i64>, cx: &mut Context<Self>) {
        let counter = match (position, total, self.current_item_id()) {
            (Some(position), Some(total), Some(item_id))
                if self.gallery_scope == MediaViewerGalleryScope::Paged && position > 0 && position <= total =>
            {
                Some((position, total, item_id))
            }
            _ => None,
        };
        let Some((position, total, item_id)) = counter else {
            self.gallery_position = None;
            self.gallery_total = None;
            self.gallery_counter_item_id = None;
            cx.notify();
            return;
        };
        if self.gallery_position != Some(position) {
            self.gallery_position = Some(position);
        }
        if self.gallery_total != Some(total) {
            self.gallery_total = Some(total);
        }
        if self.gallery_counter_item_id.as_deref() != Some(item_id.as_str()) {
            self.gallery_counter_item_id = Some(item_id);
        }
        cx.notify();
    }

    pub fn update_source_frame(&mut self, frame: Bounds<Pixels>, cx: &mut Context<Self>) {
        self.source_frame = Some(frame);
        cx.notify();
    }

    pub fn set_bubble_cell_hidden(&mut self, hidden: bool, cx: &mut Context<Self>) {
        self.bubble_cell_hidden = hidden;
        cx.notify();
    }

    pub fn sync_hero_target(
        &mut self,
        registry_id: &str,
        source_frame: Option<Bounds<Pixels>>,
        skin: MediaViewerHeroSkin,
        cache_policy: CleverImageCachePolicy,
        cx: &mut Context<Self>,
    ) {
        self.hero_lift_id = Some(registry_id.to_owned());
        if let Some(source_frame) = source_frame {
            self.source_frame = Some(source_frame);
        }
        let display_source = skin.display_source.clone();
        self.hero_skin = Some(skin);
        self.adopt_warm_image(&display_source, cache_policy, cx);
        cx.notify();
    }

    /// the warm bitmap, only for the row it was read for
    pub fn hero_warm_image(&self, for_item_id: &str) -> Option<CleverImage> {
        if self.hero_warm_item_id.as_deref() == Some(for_item_id) {
            self.hero_warm_ui_image.clone()
        } else {
            None
        }
    }

    pub fn adopt_warm_image(&mut self, url_string: &str, cache_policy: CleverImageCachePolicy, cx: &mut Context<Self>) {
        if let Some(img) =
            MediaViewerImageMemoryCache::image(url_string, SaiAccountCacheScope::active_user_id().as_deref(), cache_policy)
        {
            self.set_hero_warm_ui_image(Some(img), url_string, cx);
        } else if self.hero_warm_item_id != self.current_item_id() {
            self.clear_hero_warm_image();
            cx.notify();
        }
    }

    /// installs a bitmap read from source as the warm image of the current row
    pub fn set_hero_warm_ui_image(&mut self, image: Option<CleverImage>, source: &str, cx: &mut Context<Self>) {
        let Some(image) = image else {
            self.clear_hero_warm_image();
            cx.notify();
            return;
        };
        self.hero_warm_ui_image = Some(image);
        self.hero_warm_source = Some(source.to_owned());
        self.hero_warm_item_id = self.current_item_id();
        cx.notify();
    }

    fn current_item_id(&self) -> Option<String> {
        if self.items.is_empty() {
            return None;
        }
        Some(self.items[self.current_index.min(self.items.len() - 1)].id.clone())
    }

    fn clear_hero_warm_image(&mut self) {
        self.hero_warm_ui_image = None;
        self.hero_warm_source = None;
        self.hero_warm_item_id = None;
    }

    fn is_same_source(lhs: Option<&str>, rhs: Option<&str>) -> bool {
        let (Some(lhs), Some(rhs)) = (
            lhs.map(CleverImageDiskCache::normalized_cache_key),
            rhs.map(CleverImageDiskCache::normalized_cache_key),
        ) else {
            return false;
        };
        !lhs.is_empty() && lhs == rhs
    }

    pub fn seed_warm_image_from_cache(&mut self, cx: &mut Context<Self>) {
        let Some(src) = self.hero_skin.as_ref().map(|skin| skin.display_source.clone()) else {
            return;
        };
        let cache_policy = self
            .items
            .get(self.current_index)
            .map(|item| item.image_cache_policy)
            .unwrap_or_default();
        self.adopt_warm_image(&src, cache_policy, cx);
    }

    pub fn resolved_source_frame(&self, registry_id: &str) -> Option<Bounds<Pixels>> {
        if let Some((prepared_id, frame)) = &self.prepared_flyback_frame
            && prepared_id == registry_id
        {
            return Some(*frame);
        }
        MediaCellScreenFrameRegistry::frame(registry_id).or(self.source_frame)
    }

    /// closing needs a visible destination and not the opening frame kept for hero rendering
    /// a grid prepared target wins before its cell registers again after scrolling
    pub fn resolved_flyback_source_frame(&self, registry_id: &str, viewport: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
        let frame = match &self.prepared_flyback_frame {
            Some((prepared_id, frame)) if prepared_id == registry_id => *frame,
            _ => {
                let view = MediaCellScreenFrameRegistry::attached_view(registry_id)?;
                if !view.is_visible_in_active_clipped_viewport_ignoring_playback_scope() {
                    return None;
                }
                WindowScreenCoordinates::screen_frame(&view)?
            }
        };
        if frame.is_empty()
            || Self::is_infinite(&frame)
            || viewport.is_empty()
            || Self::is_infinite(&viewport)
            || !frame.intersects(&viewport)
        {
            return None;
        }
        Some(frame)
    }

    fn is_infinite(frame: &Bounds<Pixels>) -> bool {
        ![frame.origin.x, frame.origin.y, frame.size.width, frame.size.height]
            .iter()
            .all(|value| value.as_f32().is_finite())
    }

    pub fn prepare_flyback(state: &Entity<Self>, registry_id: &str, cx: &mut App) {
        let provider = {
            let this = state.read(cx);
            if let Some((prepared_id, _)) = &this.prepared_flyback_frame
                && prepared_id == registry_id
            {
                return;
            }
            this.flyback_target_provider.clone()
        };
        let Some(provider) = provider else {
            return;
        };
        let Some(frame) = provider(registry_id, cx) else {
            return;
        };
        let registry_id = registry_id.to_owned();
        state.update(cx, |this, cx| {
            this.prepared_flyback_frame = Some((registry_id, frame));
            cx.notify();
        });
    }

    pub fn is_active_presentation(&self, id: Uuid) -> bool {
        self.is_presented
            && self.presentation_id == Some(id)
            && SaiAccountCacheScope::is_current_account_epoch(
                self.presentation_account_user_id.as_deref(),
                self.presentation_account_epoch.as_deref(),
            )
    }

    pub fn begin_close(&mut self, cx: &mut Context<Self>) -> Option<Uuid> {
        let id = self.presentation_id?;
        if !self.is_active_presentation(id) || self.closing_presentation_id.is_some() {
            return None;
        }
        self.closing_presentation_id = Some(id);
        let hook = self.on_close_begin.take();
        if let Some(hook) = hook {
            hook(cx);
        }
        cx.notify();
        Some(id)
    }

    pub fn is_closing_presentation(&self, id: Uuid) -> bool {
        self.is_active_presentation(id) && self.closing_presentation_id == Some(id)
    }

    pub fn cancel_close(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.presentation_id != Some(id) || self.closing_presentation_id != Some(id) {
            return;
        }
        self.closing_presentation_id = None;
        cx.notify();
    }

    pub fn complete_flyback_handoff(&mut self, cx: &mut Context<Self>) {
        self.presentation_id = None;
        self.closing_presentation_id = None;
        self.closes_without_flyback = false;
        self.on_close_begin = None;
        self.presentation_account_user_id = None;
        self.presentation_account_epoch = None;
        self.bubble_cell_hidden = false;
        self.source_frame = None;
        self.hero_lift_id = None;
        self.hero_skin = None;
        self.clear_hero_warm_image();
        self.on_delete = None;
        self.on_forward = None;
        self.on_report = None;
        self.on_show_in_context = None;
        self.on_reply = None;
        self.on_set_as_main = None;
        self.on_delete_photo = None;
        self.accessory = None;
        self.flyback_target_provider = None;
        self.prepared_flyback_frame = None;
        self.dismissal_request_id = None;
        self.gallery_position = None;
        self.gallery_total = None;
        self.gallery_counter_item_id = None;
        self.gallery_counter_follows_index = false;
        self.gallery_scope = MediaViewerGalleryScope::Complete;
        self.is_presented = false;
        self.items = Vec::new();
        cx.notify();
    }

    const HERO_WARM_NETWORK_DEADLINE: Duration = Duration::from_millis(600);

    pub async fn prepare_hero_warm_image(
        state: &Entity<Self>,
        from_url_string: &str,
        persist_to_disk: bool,
        cache_policy: CleverImageCachePolicy,
        cx: &mut AsyncApp,
    ) {
        state.update(cx, |this, cx| {
            this.clear_hero_warm_image();
            cx.notify();
        });
        if let Some(hit) = MediaViewerImageMemoryCache::image(
            from_url_string,
            SaiAccountCacheScope::active_user_id().as_deref(),
            cache_policy,
        ) {
            let source = from_url_string.to_owned();
            state.update(cx, |this, cx| {
                this.hero_warm_ui_image = Some(hit);
                this.hero_warm_source = Some(source);
                cx.notify();
            });
            return;
        }
        let network_if_missing = !cx.update(RiseTransportStatus::is_waiting_for_network);
        let load = Box::pin(MediaViewerBitmapLoader::load_into_memory(
            from_url_string,
            network_if_missing,
            persist_to_disk,
            cache_policy,
            cx,
        ));
        let deadline = Box::pin(cx.background_executor().timer(Self::HERO_WARM_NETWORK_DEADLINE));
        let loaded = match select(load, deadline).await {
            Either::Left((loaded, _)) => loaded,
            Either::Right(_) => None,
        };
        if let Some(img) = loaded {
            let source = from_url_string.to_owned();
            state.update(cx, |this, cx| {
                this.hero_warm_ui_image = Some(img);
                this.hero_warm_source = Some(source);
                this.hero_warm_item_id = None;
                cx.notify();
            });
        }
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.presentation_id else {
            return;
        };
        self.closing_presentation_id = None;
        self.is_presented = false;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(MediaViewerTransition::MOVE_DURATION))
                .await;
            this.update(cx, |this, cx| {
                if this.presentation_id != Some(id) || this.is_presented {
                    return;
                }
                this.complete_flyback_handoff(cx);
            })
            .ok();
        })
        .detach();
    }

    pub fn can_forward(&self, item: &MediaViewerGalleryItem) -> bool {
        item.can_forward && self.on_forward.is_some()
    }

    pub fn forward(state: &Entity<Self>, item: &MediaViewerGalleryItem, cx: &mut App) {
        let handler = {
            let this = state.read(cx);
            if !this.can_forward(item) {
                return;
            }
            this.on_forward.clone()
        };
        if let Some(handler) = handler {
            handler(item, cx);
        }
    }

    pub fn can_report(&self, _item: &MediaViewerGalleryItem) -> bool {
        self.chrome.can_report && self.on_report.is_some()
    }

    pub fn report(state: &Entity<Self>, item: &MediaViewerGalleryItem, cx: &mut App) {
        let handler = {
            let this = state.read(cx);
            if !this.can_report(item) {
                return;
            }
            this.on_report.clone()
        };
        if let Some(handler) = handler {
            handler(item, cx);
        }
    }

    pub fn can_show_in_context(&self, item: &MediaViewerGalleryItem) -> bool {
        item.can_show_in_context && self.on_show_in_context.is_some()
    }

    pub fn show_in_context(state: &Entity<Self>, item: &MediaViewerGalleryItem, cx: &mut App) {
        let handler = {
            let this = state.read(cx);
            if !this.can_show_in_context(item) {
                return;
            }
            this.on_show_in_context.clone()
        };
        if let Some(handler) = handler {
            handler(item, cx);
        }
    }

    pub fn show_in_context_action(&self, for_item: &MediaViewerGalleryItem) -> Option<MediaViewerBoundAction> {
        if !self.can_show_in_context(for_item) {
            return None;
        }
        Some(Self::bound(self.on_show_in_context.clone(), for_item))
    }

    pub fn can_reply(&self, item: &MediaViewerGalleryItem) -> bool {
        item.can_reply && self.on_reply.is_some()
    }

    pub fn reply(state: &Entity<Self>, item: &MediaViewerGalleryItem, cx: &mut App) {
        let handler = {
            let this = state.read(cx);
            if !this.can_reply(item) {
                return;
            }
            this.on_reply.clone()
        };
        if let Some(handler) = handler {
            handler(item, cx);
        }
    }

    pub fn reply_action(&self, for_item: &MediaViewerGalleryItem) -> Option<MediaViewerBoundAction> {
        if !self.can_reply(for_item) {
            return None;
        }
        Some(Self::bound(self.on_reply.clone(), for_item))
    }

    pub fn delete_action(&self, for_item: &MediaViewerGalleryItem, mode: MediaViewerDeleteMode) -> MediaViewerBoundAction {
        let handler = self.on_delete.clone();
        let item = for_item.clone();
        Rc::new(move |cx: &mut App| {
            if let Some(handler) = &handler {
                handler(&item, mode, cx);
            }
        })
    }

    fn bound(handler: Option<MediaViewerItemHandler>, item: &MediaViewerGalleryItem) -> MediaViewerBoundAction {
        let item = item.clone();
        Rc::new(move |cx: &mut App| {
            if let Some(handler) = &handler {
                handler(&item, cx);
            }
        })
    }

    /// owner only, not already the main photo and no promote in flight
    pub fn can_set_as_main(&self, item: &MediaViewerGalleryItem) -> bool {
        self.chrome.is_own_profile_album && item.is_album_row && item.can_set_as_main && self.on_set_as_main.is_some()
    }

    pub fn set_as_main(state: &Entity<Self>, item: &MediaViewerGalleryItem, cx: &mut App) {
        let handler = {
            let this = state.read(cx);
            if !this.can_set_as_main(item) {
                return;
            }
            this.on_set_as_main.clone()
        };
        if let Some(handler) = handler {
            handler(item, cx);
        }
    }

    pub fn can_delete_profile_photo(&self, item: &MediaViewerGalleryItem) -> bool {
        self.chrome.is_own_profile_album && item.is_album_row && self.on_delete_photo.is_some()
    }

    pub fn delete_profile_photo(state: &Entity<Self>, item: &MediaViewerGalleryItem, cx: &mut App) {
        if !state.read(cx).can_delete_profile_photo(item) {
            return;
        }
        let weak = state.downgrade();
        let item = item.clone();
        NativeConfirmAlert::present_delete(
            &tr("profile_photos_delete_confirm"),
            &tr("story_delete_confirm_message"),
            None,
            None,
            false,
            None,
            None,
            move |cx: &mut App| {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                let handler = {
                    let this = state.read(cx);
                    if !this.can_delete_profile_photo(&item) {
                        return;
                    }
                    this.on_delete_photo.clone()
                };
                if let Some(handler) = handler {
                    handler(&item, cx);
                }
            },
            cx,
        );
    }

    /// asks the host to fly the current page back to its source and then dismiss
    pub fn request_animated_dismissal(&mut self, cx: &mut Context<Self>) {
        if !self.is_presented {
            return;
        }
        self.dismissal_request_id = Some(Uuid::new_v4());
        cx.notify();
    }

    pub fn consume_dismissal_request(&mut self, cx: &mut Context<Self>) {
        self.dismissal_request_id = None;
        cx.notify();
    }

    /// slice of items the rail may draw, by filmstrip scope
    /// for current message its the contiguous run around the open page with the same message id
    /// always contains current_index when items arent empty
    pub fn filmstrip_range(&self) -> Range<usize> {
        if self.items.is_empty() {
            return 0..0;
        }
        let current = self.current_index.min(self.items.len() - 1);
        match self.chrome.filmstrip_scope {
            MediaViewerFilmstripScope::WholeGallery => 0..self.items.len(),
            MediaViewerFilmstripScope::CurrentMessage => {
                let key = &self.items[current].message_id;
                let mut lower = current;
                while lower > 0 && &self.items[lower - 1].message_id == key {
                    lower -= 1;
                }
                let mut upper = current + 1;
                while upper < self.items.len() && &self.items[upper].message_id == key {
                    upper += 1;
                }
                lower..upper
            }
        }
    }

    pub fn filmstrip_group_key(&self) -> String {
        if self.items.is_empty() {
            return String::new();
        }
        match self.chrome.filmstrip_scope {
            MediaViewerFilmstripScope::WholeGallery => "gallery".to_owned(),
            MediaViewerFilmstripScope::CurrentMessage => {
                let current = self.current_index.min(self.items.len() - 1);
                format!("message-{}", self.items[current].message_id)
            }
        }
    }

    /// media count of the open item message if its an album
    /// none for a single picture or outside chat galleries
    /// forward and delete act on the whole message
    pub fn album_item_count(&self, for_item: &MediaViewerGalleryItem) -> Option<usize> {
        if self.chrome.filmstrip_scope != MediaViewerFilmstripScope::CurrentMessage || for_item.message_id.is_empty() {
            return None;
        }
        let range = self.filmstrip_range();
        if range.len() <= 1 || self.items[range.start].message_id != for_item.message_id {
            return None;
        }
        Some(range.len())
    }

    pub fn remove_items_for_deleted_message(&mut self, message_id: &str, cx: &mut Context<Self>) {
        self.items.retain(|item| item.message_id != message_id);
        if !self.items.is_empty() {
            self.current_index = self.current_index.min(self.items.len() - 1);
        }
        cx.notify();
    }

    pub fn remaining_item_count(&self, after_deleting_message: &str) -> usize {
        self.items
            .iter()
            .filter(|item| item.message_id != after_deleting_message)
            .count()
    }
}

pub type ChatMediaPresentationState = MediaViewerPresentationState;

/// environment key for the viewer presentation, none until a host scopes its own state
/// then the default is the one shared instance
pub struct MediaViewerPresentationKey;

impl EnvKey for MediaViewerPresentationKey {
    type Value = Option<Entity<MediaViewerPresentationState>>;

    fn default_value() -> Self::Value {
        None
    }
}

struct MediaViewerPresentationDefault(Entity<MediaViewerPresentationState>);

impl Global for MediaViewerPresentationDefault {}

/// state each window root view scopes over its tree, by window
#[derive(Default)]
struct MediaViewerPresentationByWindow(HashMap<WindowId, WeakEntity<MediaViewerPresentationState>>);

impl Global for MediaViewerPresentationByWindow {}

/// called by the window root view for the state its overlay hosts draw
pub fn register_window_media_viewer_presentation(
    window: &Window,
    state: &Entity<MediaViewerPresentationState>,
    cx: &mut App,
) {
    let registry = cx.default_global::<MediaViewerPresentationByWindow>();
    registry.0.retain(|_, weak| weak.upgrade().is_some());
    registry.0.insert(window.window_handle().window_id(), state.downgrade());
}

fn window_media_viewer_presentation(cx: &App) -> Option<Entity<MediaViewerPresentationState>> {
    let registry = cx.try_global::<MediaViewerPresentationByWindow>()?;
    let open = cx.windows();
    cx.active_window()
        .into_iter()
        .chain(cx.window_stack().unwrap_or_default())
        .chain(open.iter().copied())
        .filter(|window| open.contains(window))
        .find_map(|window| registry.0.get(&window.window_id()).and_then(WeakEntity::upgrade))
}

pub fn media_viewer_presentation(cx: &mut App) -> Entity<MediaViewerPresentationState> {
    if let Some(scoped) = env::<MediaViewerPresentationKey>(cx) {
        return scoped;
    }
    if let Some(hosted) = window_media_viewer_presentation(cx) {
        return hosted;
    }
    if let Some(shared) = cx.try_global::<MediaViewerPresentationDefault>() {
        return shared.0.clone();
    }
    let entity = cx.new(|_| MediaViewerPresentationState::new());
    cx.set_global(MediaViewerPresentationDefault(entity.clone()));
    entity
}

pub fn chat_media_presentation(cx: &mut App) -> Entity<MediaViewerPresentationState> {
    media_viewer_presentation(cx)
}

pub fn media_viewer_presentation_scope(
    state: Entity<MediaViewerPresentationState>,
) -> EnvScope<MediaViewerPresentationKey> {
    env_scope::<MediaViewerPresentationKey>(Some(state))
}
