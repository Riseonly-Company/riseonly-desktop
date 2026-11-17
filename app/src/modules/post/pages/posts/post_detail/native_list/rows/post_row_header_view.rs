use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{
    App, AppContext as _, AsyncApp, Bounds, ClickEvent, Context, Div, ElementId, Entity, EntityId, FocusHandle,
    FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ObjectFit, ParentElement, Pixels, Render, Role, SharedString,
    StatefulInteractiveElement, Styled, StyledImage, Task, WeakEntity, Window, div, img, point, px, size,
};
use image::RgbaImage;
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::foundation::foundation_calendar::{Calendar, CalendarIdentifier, TimeZone};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::date_format::date_formatter::{DateFormatter, Style};
use rise_i18n::date_format::relative_date_time_formatter::{RelativeDateTimeFormatter, UnitsStyle};
use rise_i18n::language_store::Locale;
use rise_i18n_catalog::RELATIVE_TIME_BANDS;
use rise_media::lottie::sequence::LottieRasterizer;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::avatar_palette::AvatarPalette;
use rise_ui::IconUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::lottie_view::{LottieView, LottieViewModel};
use rise_ui::official_badge::OfficialBadge;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::repeating_timer::RepeatingTimer;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::view_anchor_probe::{ViewAnchorProbe, ViewAnchorProbeExt};
use rise_ui::view_screen_snapshot::ViewScreenSource;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageBitmapDecoder, CleverImageBitmapLoader, CleverImageRenderMemoryCache, CleverImageWorkLane,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuActionHandler;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::HoldContextMenuUIKitPresentation;
use crate::core::ui::custom_emoji_stickers_hooks::custom_emoji_stickers_hooks;
use crate::core::ui::lottie_sticker::rise_lottie_frame_renderer::RiseLottieFirstFrameCache;
use crate::core::ui::premium_diamond::PremiumDiamond;
use crate::core::ui::user_logo_ui::StoryRingUi;
use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit::StickerMediaDisplayURL;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::post::components::post::post::Post;
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_row_action_router::PostRowMenuItem;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostListHostedEnvironment, PostRowActionRouting, PostRowHeaderLayout, PostRowLayoutSpec, PostRowMainContext,
    PostRowPalette, PostRowPremiumBadge, PostRowSeed, PostRowSeedAuthor, PostRowSeedTrack, PostRowStoryRingState,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_layout_calculator::{
    PostRowMetrics, PostRowTextMetrics,
};
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_native_row_view::{
    PostRowChromeFonts, PostRowTextLineView, PostRowTextLineViewAlignment, PostRowTextLineViewSegment, post_row_frame,
};
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_hosted_leaf::{
    PostRowHostedLeafView, PostRowPremiumGlyphLeaf,
};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{StickerAssetType, StickerItem};

const MUSIC_LINE_SPACING: f32 = 4.0;
const EMOJI_BADGE_SCALE: f32 = 0.86;
const DATE_MENU_HIT_OUTSET: f32 = 4.0;
const META_SEGMENT_SPACING: f32 = 4.0;

fn child_id(entity: EntityId, name: &'static str) -> ElementId {
    ElementId::NamedInteger(name.into(), entity.as_u64())
}

/// what a router tap does once the row still has a router and a post
fn router_tap(
    router: Option<Weak<dyn PostRowActionRouting>>,
    post_id: Option<String>,
    action: fn(&dyn PostRowActionRouting, &str, &mut App),
) -> impl Fn(&mut Window, &mut App) + 'static {
    move |_: &mut Window, cx: &mut App| {
        let Some(router) = router.as_ref().and_then(Weak::upgrade) else {
            return;
        };
        let Some(post_id) = post_id.as_deref() else {
            return;
        };
        action(&*router, post_id, cx);
    }
}

pub struct PostRowHeaderView {
    avatar_view: Entity<PostRowAvatarView>,
    name_view: Entity<PostRowTextLineView>,
    premium_emoji_view: Entity<PostRowTextLineView>,
    meta_view: Entity<PostRowTextLineView>,
    music_text_view: Entity<PostRowTextLineView>,
    column_focus: FocusHandle,
    avatar_focus: FocusHandle,
    date_focus: FocusHandle,

    header_layout: Option<PostRowHeaderLayout>,
    post_id: Option<String>,
    router: Option<Weak<dyn PostRowActionRouting>>,
    created_at: Option<Date>,
    has_date_segment: bool,
    meta_segments: Vec<PostRowTextLineViewSegment>,
    locale: Locale,
    calendar: Calendar,
    time_zone: TimeZone,
    music_glyph_size: gpui::Size<Pixels>,
    premium_request: u64,
    premium_image_task: Option<Task<()>>,
    display_scale: f32,
    premium_glyph_leaf: Entity<PostRowHostedLeafView<PostRowPremiumGlyphLeaf>>,
    premium_glyph_leaf_is_mounted: bool,
    premium_glyph_item: Option<StickerItem>,
    hosted_environment: Option<PostListHostedEnvironment>,
    leaves_active: bool,

    official_badge_tint: Hsla,
    secondary_tint: Hsla,
    premium_image: Option<CleverImage>,
    premium_image_is_hidden: bool,
    premium_emoji_is_hidden: bool,
    meta_is_hidden: bool,
    music_line_is_hidden: bool,
    avatar_label: SharedString,
    column_label: SharedString,
    music_label: SharedString,
    date_menu_frame: Option<Bounds<Pixels>>,
    date_frame_is_pending: bool,
    avatar_probe: ViewAnchorProbe,
    date_probe: ViewAnchorProbe,
    more_probe: ViewAnchorProbe,
    root_probe: ViewAnchorProbe,
}

impl PostRowHeaderView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            avatar_view: cx.new(|cx| PostRowAvatarView::new(true, PostRowAvatarViewStroke::Border, cx)),
            name_view: cx.new(|_| PostRowTextLineView::new()),
            premium_emoji_view: cx.new(|_| PostRowTextLineView::new()),
            meta_view: cx.new(|_| PostRowTextLineView::new()),
            music_text_view: cx.new(|_| PostRowTextLineView::new()),
            column_focus: cx.focus_handle().tab_stop(true),
            avatar_focus: cx.focus_handle().tab_stop(true),
            date_focus: cx.focus_handle().tab_stop(true),
            header_layout: None,
            post_id: None,
            router: None,
            created_at: None,
            has_date_segment: false,
            meta_segments: Vec::new(),
            locale: Locale::app_locale(),
            calendar: Calendar::current(),
            time_zone: TimeZone::current(),
            music_glyph_size: size(px(0.0), px(0.0)),
            premium_request: 0,
            premium_image_task: None,
            display_scale: 3.0,
            premium_glyph_leaf: cx.new(PostRowHostedLeafView::new),
            premium_glyph_leaf_is_mounted: false,
            premium_glyph_item: None,
            hosted_environment: None,
            leaves_active: false,
            official_badge_tint: gpui::transparent_black(),
            secondary_tint: gpui::transparent_black(),
            premium_image: None,
            premium_image_is_hidden: true,
            premium_emoji_is_hidden: true,
            meta_is_hidden: true,
            music_line_is_hidden: true,
            avatar_label: SharedString::default(),
            column_label: SharedString::default(),
            music_label: SharedString::default(),
            date_menu_frame: None,
            date_frame_is_pending: false,
            avatar_probe: ViewAnchorProbe::new(),
            date_probe: ViewAnchorProbe::new(),
            more_probe: ViewAnchorProbe::new(),
            root_probe: ViewAnchorProbe::new(),
        }
    }

    pub fn configure(
        &mut self,
        seed: &PostRowSeed,
        layout: &PostRowHeaderLayout,
        context: &PostRowMainContext,
        cx: &mut Context<Self>,
    ) {
        self.post_id = Some(seed.post_id.clone());
        self.router = Some(Rc::downgrade(&context.router));
        self.header_layout = Some(layout.clone());
        self.locale = context.locale.clone();
        self.calendar = context.calendar.clone();
        self.time_zone = context.time_zone.clone();
        self.display_scale = context.spec.display_scale;
        let palette = &context.palette;
        let spec = &context.spec;
        let author = &seed.author;

        let ring_state = Self::story_state(author, context, cx);
        self.avatar_view.update(cx, |avatar, cx| {
            avatar.configure(
                &author.logo_url,
                &author.name,
                author.id.as_deref(),
                author.is_deleted,
                PostRowMetrics::AVATAR_SIZE,
                palette,
                spec,
                context.account_user_id.as_deref(),
                context.account_epoch.as_deref(),
                cx,
            );
            avatar.set_story_ring(ring_state, cx);
        });
        self.avatar_label = author.name.clone().into();

        let name_font = PostRowChromeFonts::text(PostRowMetrics::NAME_FONT_SIZE, FontWeight::NORMAL, spec);
        let name_color = context.name_color(author);
        self.name_view.update(cx, |view, cx| {
            view.set_text(&author.name, name_font, name_color, PostRowTextLineViewAlignment::Leading, cx);
        });

        self.official_badge_tint = palette.official_badge;

        self.configure_premium_badge(&author.premium_badge, layout, context, cx);

        let secondary_font = PostRowChromeFonts::text(PostRowMetrics::SECONDARY_FONT_SIZE, FontWeight::NORMAL, spec);
        let mut segments = Vec::new();
        if !author.tag.is_empty() {
            segments.push(PostRowTextLineViewSegment {
                text: format!("@{}", author.tag),
                font: secondary_font,
                color: palette.text_secondary,
                truncates: true,
            });
            segments.push(PostRowTextLineViewSegment {
                text: "\u{b7}".to_owned(),
                font: secondary_font,
                color: palette.text_secondary,
                truncates: false,
            });
        }
        self.created_at = seed.created_at;
        let mut date_text = None;
        if let Some(date) = seed.created_at.as_ref() {
            let text = PostRowDateText::relative(date, &self.locale, &self.calendar);
            segments.push(PostRowTextLineViewSegment {
                text: text.clone(),
                font: secondary_font,
                color: palette.text_secondary,
                truncates: false,
            });
            date_text = Some(text);
        }
        self.has_date_segment = date_text.is_some();
        self.meta_segments = segments.clone();
        self.meta_is_hidden = layout.meta_line.is_none();
        self.meta_view.update(cx, |view, cx| {
            view.set_segments(segments, META_SEGMENT_SPACING, PostRowTextLineViewAlignment::Leading, cx);
        });
        self.date_frame_is_pending = true;
        let is_tracked = self.has_date_segment;
        self.track_on_ticker(is_tracked, cx);

        match seed.track.as_ref().filter(|_| layout.music_line.is_some()) {
            Some(track) => {
                let text = PostRowMusicLineText::text(track, &spec.locale_identifier);
                self.music_line_is_hidden = false;
                self.music_glyph_size = PostRowTextMetrics::symbol_size(
                    "music.note",
                    PostRowMetrics::MUSIC_GLYPH_POINT_SIZE,
                    FontWeight::MEDIUM,
                );
                let color = palette.text_secondary;
                let font = secondary_font;
                self.music_text_view.update(cx, |view, cx| {
                    view.set_text(&text, font, color, PostRowTextLineViewAlignment::Leading, cx);
                });
                self.music_label = text.into();
            }
            None => self.music_line_is_hidden = true,
        }

        self.secondary_tint = palette.text_secondary;

        let tag_text = (!author.tag.is_empty()).then(|| format!("@{}", author.tag));
        self.column_label = [Some(author.name.clone()), tag_text, date_text]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ")
            .into();

        cx.notify();
    }

    pub fn prepare_for_reuse(&mut self, cx: &mut Context<Self>) {
        self.avatar_view.update(cx, |avatar, cx| avatar.prepare_for_reuse(cx));
        self.premium_request = self.premium_request.wrapping_add(1);
        self.premium_image = None;
        self.leaves_active = false;
        self.clear_premium_glyph(cx);
        self.hosted_environment = None;
        self.header_layout = None;
        self.post_id = None;
        self.created_at = None;
        self.has_date_segment = false;
        self.meta_segments = Vec::new();
        self.date_menu_frame = None;
        self.date_frame_is_pending = false;
        self.track_on_ticker(false, cx);
        cx.notify();
    }

    fn track_on_ticker(&self, is_tracked: bool, cx: &mut Context<Self>) {
        let header = cx.weak_entity();
        PostRowDateTicker::shared(cx).update(cx, |ticker, cx| ticker.track(header, is_tracked, cx));
    }

    fn refresh_relative_date(&mut self, cx: &mut Context<Self>) {
        if !self.has_date_segment {
            return;
        }
        let Some(created_at) = self.created_at.as_ref() else {
            return;
        };
        let text = PostRowDateText::relative(created_at, &self.locale, &self.calendar);
        let Some(last) = self.meta_segments.last_mut() else {
            return;
        };
        if text == last.text {
            return;
        }
        last.text = text;
        let segments = self.meta_segments.clone();
        self.meta_view.update(cx, |view, cx| {
            view.set_segments(segments, META_SEGMENT_SPACING, PostRowTextLineViewAlignment::Leading, cx);
        });
        self.date_frame_is_pending = true;
        cx.notify();
    }

    fn date_segment_frame(&self, cx: &App) -> Option<Bounds<Pixels>> {
        if !self.has_date_segment {
            return None;
        }
        self.meta_view.read(cx).drawn_segment_frames().last().copied()
    }

    fn configure_premium_badge(
        &mut self,
        badge: &PostRowPremiumBadge,
        layout: &PostRowHeaderLayout,
        context: &PostRowMainContext,
        cx: &mut Context<Self>,
    ) {
        self.premium_request = self.premium_request.wrapping_add(1);
        self.premium_image_task = None;
        self.hosted_environment = Some(context.hosted.clone());
        if layout.premium_badge.is_none() {
            self.clear_premium_glyph(cx);
            self.premium_image_is_hidden = true;
            self.premium_emoji_is_hidden = true;
            return;
        }
        match badge {
            PostRowPremiumBadge::None | PostRowPremiumBadge::UnresolvedCustom { .. } => {
                self.clear_premium_glyph(cx);
                self.premium_image_is_hidden = true;
                self.premium_emoji_is_hidden = true;
            }
            PostRowPremiumBadge::Unicode(emoji) => {
                self.clear_premium_glyph(cx);
                self.premium_image_is_hidden = true;
                self.premium_emoji_is_hidden = false;
                let font = PostRowChromeFonts::fixed(
                    PostRowMetrics::BADGE_SIZE * EMOJI_BADGE_SCALE,
                    FontWeight::NORMAL,
                    false,
                );
                let color = context.palette.text_primary;
                self.premium_emoji_view.update(cx, |view, cx| {
                    view.set_text(emoji, font, color, PostRowTextLineViewAlignment::Center, cx);
                });
            }
            PostRowPremiumBadge::Diamond | PostRowPremiumBadge::MissedCustom { .. } => {
                self.clear_premium_glyph(cx);
                self.show_premium_diamond(context.spec.display_scale, cx);
            }
            PostRowPremiumBadge::Custom { id, width, .. } => {
                let item = custom_emoji_stickers_hooks().and_then(|hooks| (hooks.custom_emoji_item)(id, &**cx));
                let Some(item) = item else {
                    self.clear_premium_glyph(cx);
                    self.show_premium_diamond(context.spec.display_scale, cx);
                    return;
                };
                self.configure_custom_premium_glyph(item, *width, context, cx);
            }
        }
    }

    fn configure_custom_premium_glyph(
        &mut self,
        item: StickerItem,
        width: f32,
        context: &PostRowMainContext,
        cx: &mut Context<Self>,
    ) {
        self.premium_emoji_is_hidden = true;
        if self.premium_glyph_item.as_ref() != Some(&item) {
            self.premium_glyph_leaf_is_mounted = false;
            self.premium_image = None;
            self.premium_glyph_item = Some(item.clone());
        }
        let poster = PostRowPremiumGlyphPoster::image(
            &item,
            width,
            context.account_user_id.as_deref(),
            context.account_epoch.as_deref(),
        );
        if poster.is_some() {
            self.premium_image = poster;
        }
        if item.asset_type == StickerAssetType::StaticImage && self.premium_image.is_none() {
            let token = self.premium_request;
            let account_id = context.account_user_id.clone();
            let account_epoch = context.account_epoch.clone();
            self.premium_image_task = Some(cx.spawn(async move |this, cx| {
                let image = PostRowPremiumGlyphPoster::load_static_image(
                    &item,
                    width,
                    account_id.clone(),
                    account_epoch.clone(),
                    &*cx,
                )
                .await;
                let _ = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                    if this.premium_request != token
                        || this.premium_glyph_item.as_ref() != Some(&item)
                        || !SaiAccountCacheScope::is_current_account_epoch(
                            account_id.as_deref(),
                            account_epoch.as_deref(),
                        )
                    {
                        return;
                    }
                    if let Some(task) = this.premium_image_task.take() {
                        task.detach();
                    }
                    this.premium_image = image;
                    cx.notify();
                });
            }));
        }
        self.sync_premium_glyph_leaf(cx);
    }

    pub fn set_hosted_leaves_active(&mut self, is_active: bool, cx: &mut Context<Self>) {
        if self.leaves_active == is_active {
            return;
        }
        self.leaves_active = is_active;
        self.sync_premium_glyph_leaf(cx);
        cx.notify();
    }

    fn show_premium_diamond(&mut self, scale: f32, cx: &mut Context<Self>) {
        self.premium_emoji_is_hidden = true;
        self.premium_image_is_hidden = false;
        if let Some(image) = PostRowPremiumDiamond::cached_image(PostRowMetrics::BADGE_SIZE, scale) {
            self.premium_image = Some(image);
            return;
        }
        self.premium_image = None;
        let token = self.premium_request;
        let this = cx.weak_entity();
        PostRowPremiumDiamond::request(PostRowMetrics::BADGE_SIZE, scale, cx, move |image, cx| {
            let _ = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                if this.premium_request != token {
                    return;
                }
                this.premium_image = Some(image);
                cx.notify();
            });
        });
    }

    /// poster shows while the glyph is unmounted, a mounted glyph draws its own until the first frame
    fn sync_premium_glyph_leaf(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.premium_glyph_item.clone() else {
            return;
        };
        let can_mount = item.asset_type != StickerAssetType::StaticImage && self.leaves_active;
        let environment = self.hosted_environment.as_ref().filter(|_| can_mount);
        let is_mounted = environment.is_some();
        self.premium_glyph_leaf.update(cx, |leaf, cx| match environment {
            Some(environment) if leaf.is_mounted() => leaf.update(PostRowPremiumGlyphLeaf::new(item), environment, cx),
            Some(environment) => leaf.mount(PostRowPremiumGlyphLeaf::new(item), environment, cx),
            None => leaf.unmount(cx),
        });
        self.premium_glyph_leaf_is_mounted = is_mounted;
        self.premium_image_is_hidden = is_mounted;
    }

    fn clear_premium_glyph(&mut self, cx: &mut Context<Self>) {
        self.premium_image_task = None;
        self.premium_glyph_item = None;
        self.premium_glyph_leaf_is_mounted = false;
        self.premium_glyph_leaf.update(cx, |leaf, cx| leaf.unmount(cx));
    }

    fn story_state(author: &PostRowSeedAuthor, context: &PostRowMainContext, cx: &App) -> PostRowStoryRingState {
        if author.is_deleted {
            return PostRowStoryRingState::NoStories;
        }
        let Some(id) = author.id.as_deref().map(trimming_whitespaces_and_newlines) else {
            return PostRowStoryRingState::NoStories;
        };
        if id.is_empty() {
            return PostRowStoryRingState::NoStories;
        }
        (context.story_ring_state)(id, cx)
    }

    /// shows the app own menu anchored on the probed view
    fn present(
        this: &WeakEntity<Self>,
        probe: &ViewAnchorProbe,
        rect: Option<Bounds<Pixels>>,
        alignment: HoldContextMenuAlignment,
        items: Vec<PostRowMenuItem>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(entity) = this.upgrade() else {
            return;
        };
        let root_probe = entity.read(cx).root_probe.clone();
        let (Some(anchor), Some(root)) = (probe.window_frame(), root_probe.window_frame()) else {
            return;
        };
        let offset = anchor.origin - root.origin;
        let rect = match rect {
            Some(rect) => Bounds::new(offset + rect.origin, rect.size),
            None => Bounds::new(offset, anchor.size),
        };
        let source = ViewScreenSource::new(entity, root_probe);
        let actions = items.iter().map(|item| item.action.clone()).collect();
        let on_action: HoldContextMenuActionHandler = Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            if let Some(item) = items.iter().find(|item| item.action.id == id) {
                (item.perform)(cx);
            }
        });
        HoldContextMenuUIKitPresentation::present(&source, Some(rect), alignment, actions, on_action, window, cx);
    }

    /// items from the router for this rows post, built when the menu opens
    fn menu_items(
        router: &Option<Weak<dyn PostRowActionRouting>>,
        post_id: &Option<String>,
        build: fn(&dyn PostRowActionRouting, &str, &mut App) -> Vec<PostRowMenuItem>,
        cx: &mut App,
    ) -> Vec<PostRowMenuItem> {
        match (router.as_ref().and_then(Weak::upgrade), post_id.as_deref()) {
            (Some(router), Some(post_id)) => build(router.as_ref(), post_id, cx),
            _ => Vec::new(),
        }
    }

    fn avatar_element(&self, layout: &PostRowHeaderLayout, entity: EntityId, cx: &mut Context<Self>) -> Div {
        let this = cx.weak_entity();
        let probe = self.avatar_probe.clone();
        let hold_router = self.router.clone();
        let hold_post_id = self.post_id.clone();
        let on_press = router_tap(self.router.clone(), self.post_id.clone(), |router, post_id, cx| {
            router.avatar_tapped(post_id, cx);
        });
        post_row_frame(layout.avatar).child(
            div()
                .id(child_id(entity, "header-avatar-hint"))
                .track_focus(&self.avatar_focus)
                .role(Role::Button)
                .aria_label(self.avatar_label.clone())
                .tooltip_label(self.avatar_label.clone())
                .child(
                    plain(child_id(entity, "header-avatar"))
                        .w(layout.avatar.size.width)
                        .h(layout.avatar.size.height)
                        .on_press(on_press)
                        .child(self.avatar_view.clone()),
                )
                .view_anchor_probe(&self.avatar_probe)
                .on_mouse_down(MouseButton::Right, move |_: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                    let items = Self::menu_items(&hold_router, &hold_post_id, |router, post_id, cx| router.avatar_menu_items(post_id, cx), cx);
                    if items.is_empty() {
                        return;
                    }
                    cx.stop_propagation();
                    Self::present(&this, &probe, None, HoldContextMenuAlignment::Leading, items, window, cx);
                }),
        )
    }

    fn premium_badge_element(&self, rect: Bounds<Pixels>) -> Div {
        let mut badge = post_row_frame(rect);
        if !self.premium_image_is_hidden
            && let Some(image) = self.premium_image.clone()
        {
            badge = badge.child(img(image).w(rect.size.width).h(rect.size.height).object_fit(ObjectFit::Contain));
        }
        if !self.premium_emoji_is_hidden {
            badge = badge.child(self.premium_emoji_view.clone());
        }
        if self.premium_glyph_leaf_is_mounted {
            badge = badge.child(self.premium_glyph_leaf.clone());
        }
        badge
    }

    fn date_menu_element(&self, frame: Bounds<Pixels>, created_at: &Date, entity: EntityId, cx: &mut Context<Self>) -> Div {
        let exact = PostRowDateText::exact(created_at, &self.locale, &self.calendar, &self.time_zone);
        let this = cx.weak_entity();
        let probe = self.date_probe.clone();
        let outset = px(DATE_MENU_HIT_OUTSET);
        let target = Bounds {
            origin: point(frame.origin.x - outset, frame.origin.y - outset),
            size: size(frame.size.width + outset * 2.0, frame.size.height + outset * 2.0),
        };
        post_row_frame(target).child(
            div()
                .id(child_id(entity, "header-date"))
                .track_focus(&self.date_focus)
                .w(target.size.width)
                .h(target.size.height)
                .view_anchor_probe(&self.date_probe)
                .on_mouse_down(MouseButton::Right, move |_: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                    cx.stop_propagation();
                    let copied = exact.clone();
                    let item = PostRowMenuItem {
                        action: HoldContextMenuAction::new("copy_date", exact.clone()).system_image("calendar"),
                        perform: Rc::new(move |cx: &mut App| {
                            UIPasteboard::general().set_string(&copied, cx);
                        }),
                    };
                    let lifted = Bounds {
                        origin: point(px(0.0), outset - px(2.0)),
                        size: size(target.size.width, target.size.height - (outset - px(2.0)) * 2.0),
                    };
                    Self::present(&this, &probe, Some(lifted), HoldContextMenuAlignment::Leading, vec![item], window, cx);
                }),
        )
    }

    fn music_line_element(&self, rect: Bounds<Pixels>, entity: EntityId) -> Div {
        let glyph = self.music_glyph_size;
        let glyph_top = PostRowTextMetrics::pixel_round(
            (f32::from(rect.size.height) - f32::from(glyph.height)) / 2.0,
            self.display_scale,
        );
        let text_x = f32::from(glyph.width) + MUSIC_LINE_SPACING;
        let tap = router_tap(self.router.clone(), self.post_id.clone(), |router, post_id, cx| {
            router.open_audio_page(post_id, cx);
        });
        post_row_frame(rect).child(
            div()
                .id(child_id(entity, "header-music-hint"))
                .role(Role::Button)
                .aria_label(self.music_label.clone())
                .w(rect.size.width)
                .h(rect.size.height)
                .child(
                    plain(child_id(entity, "header-music"))
                        .relative()
                        .w(rect.size.width)
                        .h(rect.size.height)
                        .on_press(move |window: &mut Window, cx: &mut App| {
                            cx.stop_propagation();
                            tap(window, cx);
                        })
                        .child(
                            post_row_frame(Bounds {
                                origin: point(px(0.0), px(glyph_top)),
                                size: glyph,
                            })
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized(
                                "music.note",
                                px(PostRowMetrics::MUSIC_GLYPH_POINT_SIZE),
                                self.secondary_tint,
                                false,
                            )),
                        )
                        .child(
                            post_row_frame(Bounds {
                                origin: point(px(text_x), px(0.0)),
                                size: size(px((f32::from(rect.size.width) - text_x).max(0.0)), rect.size.height),
                            })
                            .child(self.music_text_view.clone()),
                        ),
                ),
        )
    }

    fn more_button_element(
        &self,
        layout: &PostRowHeaderLayout,
        entity: EntityId,
        cx: &mut Context<Self>,
    ) -> Div {
        let this = cx.weak_entity();
        let probe = self.more_probe.clone();
        let router = self.router.clone();
        let post_id = self.post_id.clone();
        post_row_frame(layout.more_button).child(
            div()
                .id(child_id(entity, "header-more"))
                .role(Role::Button)
                .cursor_pointer()
                .w(layout.more_button.size.width)
                .h(layout.more_button.size.height)
                .flex()
                .items_center()
                .justify_center()
                .view_anchor_probe(&self.more_probe)
                .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    let items = Self::menu_items(&router, &post_id, |router, post_id, cx| router.menu_items(post_id, cx), cx);
                    Self::present(&this, &probe, None, HoldContextMenuAlignment::Trailing, items, window, cx);
                })
                .children(IconUi::sized("ellipsis", px(16.0), self.secondary_tint, false)),
        )
    }
}

impl Render for PostRowHeaderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(layout) = self.header_layout.clone() else {
            return div().into_any_element();
        };
        if self.date_frame_is_pending {
            self.date_frame_is_pending = false;
            cx.on_next_frame(window, |this: &mut Self, _: &mut Window, cx: &mut Context<Self>| {
                let frame = this.date_segment_frame(cx);
                if this.date_menu_frame != frame {
                    this.date_menu_frame = frame;
                    cx.notify();
                }
            });
        }

        let entity = cx.entity_id();
        let column = layout.text_column;
        let local = |rect: Bounds<Pixels>| Bounds {
            origin: point(rect.origin.x - column.origin.x, rect.origin.y - column.origin.y),
            size: rect.size,
        };

        let column_tap = router_tap(self.router.clone(), self.post_id.clone(), |router, post_id, cx| {
            router.header_text_tapped(post_id, cx);
        });
        let mut column_view = post_row_frame(column)
            .id(child_id(entity, "header-column"))
            .track_focus(&self.column_focus)
            .cursor_pointer()
            .role(Role::Button)
            .aria_label(self.column_label.clone())
            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| column_tap(window, cx))
            .child(post_row_frame(local(layout.name)).child(self.name_view.clone()));
        if let Some(rect) = layout.official_badge {
            column_view = column_view.child(
                post_row_frame(local(rect))
                    .child(OfficialBadge::new().size(rect.size.width).color(self.official_badge_tint)),
            );
        }
        if let Some(rect) = layout.premium_badge {
            column_view = column_view.child(self.premium_badge_element(local(rect)));
        }
        if let Some(rect) = layout.meta_line.filter(|_| !self.meta_is_hidden) {
            let mut meta = post_row_frame(local(rect)).child(self.meta_view.clone());
            if let (Some(frame), Some(created_at)) = (self.date_menu_frame, self.created_at.as_ref()) {
                meta = meta.child(self.date_menu_element(frame, created_at, entity, cx));
            }
            column_view = column_view.child(meta);
        }
        if let Some(rect) = layout.music_line.filter(|_| !self.music_line_is_hidden) {
            column_view = column_view.child(self.music_line_element(local(rect), entity));
        }

        let avatar = self.avatar_element(&layout, entity, cx);
        let more = self.more_button_element(&layout, entity, cx);
        div()
            .relative()
            .size_full()
            .child(avatar)
            .child(column_view)
            .child(more)
            .view_anchor_probe(&self.root_probe)
            .into_any_element()
    }
}

/// poster under the hosted premium glyph, memory caches only
pub enum PostRowPremiumGlyphPoster {}

impl PostRowPremiumGlyphPoster {
    pub fn image(
        item: &StickerItem,
        side: f32,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> Option<CleverImage> {
        let pixels = Self::pixels(side);
        let thumbnail = item.thumbnail_url.as_deref().map(StickerMediaDisplayURL::resolve);
        match item.asset_type {
            StickerAssetType::Lottie => {
                if let Some(image) = thumbnail
                    .as_deref()
                    .and_then(|thumbnail| Self::raster(thumbnail, pixels, account_user_id, account_epoch))
                {
                    return Some(image);
                }
                let url = StickerMediaDisplayURL::resolve(&item.file_url);
                RiseLottieFirstFrameCache::memory_image(&url, size(px(side), px(side)), account_user_id)
                    .or_else(|| RiseLottieFirstFrameCache::latest_memory_image(&url, account_user_id))
            }
            StickerAssetType::Video => {
                Self::raster(thumbnail.as_deref()?, pixels, account_user_id, account_epoch)
            }
            StickerAssetType::StaticImage => {
                let url = StickerMediaDisplayURL::resolve(&item.file_url);
                Self::raster(&url, pixels, account_user_id, account_epoch)
                    .filter(|image| Self::is_usable(image, item))
            }
        }
    }

    pub async fn load_static_image(
        item: &StickerItem,
        side: f32,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &AsyncApp,
    ) -> Option<CleverImage> {
        let url = StickerMediaDisplayURL::resolve(&item.file_url);
        let pixels = Self::pixels(side);
        let cached = CleverImageBitmapLoader::load(
            &url,
            true,
            true,
            account_user_id.clone(),
            account_epoch.clone(),
            Some(pixels),
            CleverImageCachePolicy::Standard,
            CleverImageWorkLane::Visible,
            cx,
        )
        .await?;
        if Self::is_usable(&cached, item) {
            return Some(cached);
        }
        let repaired = CleverImageBitmapLoader::load(
            &url,
            false,
            true,
            account_user_id.clone(),
            account_epoch.clone(),
            Some(pixels),
            CleverImageCachePolicy::PrivateEphemeral,
            CleverImageWorkLane::Visible,
            cx,
        )
        .await?;
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
            || !Self::is_usable(&repaired, item)
        {
            return None;
        }
        CleverImageRenderMemoryCache::store(
            repaired.clone(),
            &url,
            pixels,
            account_user_id.as_deref(),
            account_epoch.as_deref(),
        );
        Some(repaired)
    }

    fn pixels(side: f32) -> i64 {
        ((side * 3.0).ceil() as i64).max(1)
    }

    fn is_usable(image: &CleverImage, item: &StickerItem) -> bool {
        let bitmap = image.size(0);
        item.width.max(item.height) <= 1 || bitmap.width.0.max(bitmap.height.0) > 1
    }

    fn raster(
        url: &str,
        pixels: i64,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> Option<CleverImage> {
        if !url.to_lowercase().starts_with("http") {
            return None;
        }
        CleverImageBitmapLoader::memory_image(
            url,
            true,
            account_user_id,
            account_epoch,
            Some(pixels),
            CleverImageCachePolicy::Standard,
            None,
            false,
        )
    }
}

/// avatar as a control, remote or local logo
/// letter or deleted placeholder, stroke and story ring
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostRowAvatarViewStroke {
    Border,
    BackgroundRing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PostRowAvatarViewImageState {
    Empty,
    Image,
    Pending,
    Failed,
    Letter,
    Deleted,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PostRowAvatarViewGlyph {
    symbol: &'static str,
    point_size: f32,
    tint: Hsla,
    is_filled: bool,
}

pub struct PostRowAvatarView {
    letter_view: Entity<PostRowTextLineView>,
    shows_story_ring: bool,
    stroke: PostRowAvatarViewStroke,

    load_task: Option<Task<()>>,
    load_generation: u64,
    image_key: Option<String>,
    image_state: PostRowAvatarViewImageState,
    palette: Option<PostRowPalette>,
    diameter: f32,
    placeholder_name: String,
    placeholder_user_id: Option<String>,
    placeholder_spec: Option<PostRowLayoutSpec>,

    image: Option<CleverImage>,
    image_background: Option<Hsla>,
    glyph: Option<PostRowAvatarViewGlyph>,
    letter_is_hidden: bool,
    stroke_color: Hsla,
    ring_state: PostRowStoryRingState,
}

impl PostRowAvatarView {
    pub fn new(shows_story_ring: bool, stroke: PostRowAvatarViewStroke, cx: &mut Context<Self>) -> Self {
        Self {
            letter_view: cx.new(|_| PostRowTextLineView::new()),
            shows_story_ring,
            stroke,
            load_task: None,
            load_generation: 0,
            image_key: None,
            image_state: PostRowAvatarViewImageState::Empty,
            palette: None,
            diameter: 0.0,
            placeholder_name: String::new(),
            placeholder_user_id: None,
            placeholder_spec: None,
            image: None,
            image_background: None,
            glyph: None,
            letter_is_hidden: true,
            stroke_color: gpui::transparent_black(),
            ring_state: PostRowStoryRingState::NoStories,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn configure(
        &mut self,
        source: &str,
        name: &str,
        user_id: Option<&str>,
        is_deleted: bool,
        diameter: f32,
        palette: &PostRowPalette,
        spec: &PostRowLayoutSpec,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        self.palette = Some(palette.clone());
        self.diameter = diameter;
        self.placeholder_name = name.to_owned();
        self.placeholder_user_id = user_id.map(str::to_owned);
        self.placeholder_spec = Some(spec.clone());
        self.stroke_color = match self.stroke {
            PostRowAvatarViewStroke::Border => palette.border100,
            PostRowAvatarViewStroke::BackgroundRing => palette.background100,
        };
        cx.notify();

        if is_deleted {
            self.cancel_load();
            self.image_key = None;
            self.show_deleted(palette);
            return;
        }
        let Some(url) = AvatarPalette::resolved_logo_url(Some(source)) else {
            self.cancel_load();
            self.image_key = None;
            self.show_letter(name, user_id, spec, cx);
            return;
        };

        let pixels = (diameter * 3.0).ceil() as i64;
        let key = format!(
            "{url}|{pixels}|{}|{}",
            account_user_id.unwrap_or("anonymous"),
            account_epoch.unwrap_or("none")
        );
        if self.image_key.as_deref() == Some(key.as_str()) {
            self.refresh_placeholder_colors(palette, cx);
            return;
        }
        self.cancel_load();
        self.image_key = Some(key);

        let is_remote = url.to_lowercase().starts_with("http");
        if is_remote
            && let Some(hit) = CleverImageBitmapLoader::memory_image(
                &url,
                true,
                account_user_id,
                account_epoch,
                Some(pixels),
                CleverImageCachePolicy::Standard,
                None,
                false,
            )
        {
            self.install(hit);
            return;
        }

        self.show_pending(palette, cx);
        self.load_generation = self.load_generation.wrapping_add(1);
        let generation = self.load_generation;
        let account_user_id = account_user_id.map(str::to_owned);
        let account_epoch = account_epoch.map(str::to_owned);
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let image = if is_remote {
                CleverImageBitmapLoader::load(
                    &url,
                    true,
                    true,
                    account_user_id,
                    account_epoch,
                    Some(pixels),
                    CleverImageCachePolicy::Standard,
                    CleverImageWorkLane::Visible,
                    &*cx,
                )
                .await
            } else {
                PostRowLocalImageDecoder::decode_async(url, pixels, &*cx).await
            };
            let _ = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                if this.load_generation != generation {
                    return;
                }
                match image {
                    Some(image) => this.install(image),
                    None => {
                        if let Some(palette) = this.palette.clone() {
                            this.show_failure(&palette);
                        }
                    }
                }
                cx.notify();
            });
        }));
    }

    pub fn set_story_ring(&mut self, ring_state: PostRowStoryRingState, cx: &mut Context<Self>) {
        if !self.shows_story_ring || self.ring_state == ring_state {
            return;
        }
        self.ring_state = ring_state;
        cx.notify();
    }

    pub fn prepare_for_reuse(&mut self, cx: &mut Context<Self>) {
        self.cancel_load();
        self.image_key = None;
        self.image_state = PostRowAvatarViewImageState::Empty;
        self.image = None;
        self.image_background = None;
        self.glyph = None;
        self.letter_is_hidden = true;
        self.ring_state = PostRowStoryRingState::NoStories;
        cx.notify();
    }

    fn cancel_load(&mut self) {
        self.load_generation = self.load_generation.wrapping_add(1);
        self.load_task = None;
    }

    fn install(&mut self, image: CleverImage) {
        self.image_state = PostRowAvatarViewImageState::Image;
        self.image = Some(image);
        self.image_background = None;
        self.glyph = None;
        self.letter_is_hidden = true;
    }

    /// the authors letter stands in until the photo decodes, after a cold
    /// launch the decode queue is long and grey discs look like missing avatars
    fn show_pending(&mut self, palette: &PostRowPalette, cx: &mut Context<Self>) {
        if let Some(spec) = self.placeholder_spec.clone() {
            let name = self.placeholder_name.clone();
            let user_id = self.placeholder_user_id.clone();
            self.show_letter(&name, user_id.as_deref(), &spec, cx);
            self.image_state = PostRowAvatarViewImageState::Pending;
            return;
        }
        self.image_state = PostRowAvatarViewImageState::Pending;
        self.image = None;
        self.image_background = Some(palette.background300);
        self.glyph = None;
        self.letter_is_hidden = true;
    }

    /// image load failure look, grey bg with a photo glyph at secondary 0.6
    fn show_failure(&mut self, palette: &PostRowPalette) {
        self.image_state = PostRowAvatarViewImageState::Failed;
        self.image = None;
        self.image_background = Some(palette.background300);
        self.glyph = Some(PostRowAvatarViewGlyph {
            symbol: "photo",
            point_size: 17.0,
            tint: palette.text_secondary.opacity(0.6),
            is_filled: false,
        });
        self.letter_is_hidden = true;
    }

    fn show_deleted(&mut self, palette: &PostRowPalette) {
        self.image_state = PostRowAvatarViewImageState::Deleted;
        self.image = None;
        self.image_background = Some(palette.background300);
        self.glyph = Some(PostRowAvatarViewGlyph {
            symbol: "person.fill.questionmark",
            point_size: (self.diameter * 0.38).max(9.0),
            tint: palette.text_secondary,
            is_filled: true,
        });
        self.letter_is_hidden = true;
    }

    fn show_letter(&mut self, name: &str, user_id: Option<&str>, spec: &PostRowLayoutSpec, cx: &mut Context<Self>) {
        self.image_state = PostRowAvatarViewImageState::Letter;
        self.image = None;
        let background = AvatarPalette::color_for_user(user_id, Some(name));
        self.image_background = Some(background);
        self.glyph = None;
        self.letter_is_hidden = false;
        let font = PostRowChromeFonts::fixed(
            (self.diameter * 0.46).max(11.0),
            FontWeight::SEMIBOLD,
            spec.is_bold_text_enabled,
        );
        let letter = AvatarPalette::display_letter(Some(name));
        self.letter_view.update(cx, |view, cx| {
            view.set_text(
                &letter,
                font,
                AvatarPalette::letter_color_on(background),
                PostRowTextLineViewAlignment::Center,
                cx,
            );
        });
    }

    fn refresh_placeholder_colors(&mut self, palette: &PostRowPalette, cx: &mut Context<Self>) {
        match self.image_state {
            PostRowAvatarViewImageState::Pending => self.show_pending(palette, cx),
            PostRowAvatarViewImageState::Failed => self.show_failure(palette),
            PostRowAvatarViewImageState::Empty
            | PostRowAvatarViewImageState::Image
            | PostRowAvatarViewImageState::Letter
            | PostRowAvatarViewImageState::Deleted => {}
        }
    }
}

impl Render for PostRowAvatarView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let side = self.diameter;
        let mut root = div().relative().size(px(side));
        if let Some(background) = self.image_background {
            root = root.child(div().absolute().size_full().circle().bg(background));
        }
        if let Some(image) = self.image.clone() {
            root = root.child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .size(px(side))
                    .rounded_full()
                    .overflow_hidden()
                    .child(
                        img(image)
                            .absolute()
                            .left_0()
                            .top_0()
                            .size(px(side))
                            .object_fit(ObjectFit::Cover)
                            .rounded(px(side / 2.0)),
                    ),
            );
        }
        if let Some(glyph) = self.glyph {
            root = root.child(div().absolute().size_full().flex().items_center().justify_center().children(
                IconUi::sized(glyph.symbol, px(glyph.point_size), glyph.tint, glyph.is_filled),
            ));
        }
        if !self.letter_is_hidden {
            let inset = side * 0.14;
            root = root.child(
                div()
                    .absolute()
                    .left(px(inset))
                    .top(px(inset))
                    .size(px(side - inset * 2.0))
                    .child(self.letter_view.clone()),
            );
        }
        let line_width = match self.stroke {
            PostRowAvatarViewStroke::Border => 0.5,
            PostRowAvatarViewStroke::BackgroundRing => 1.0,
        };
        root = root.child(centred_stroke(self.stroke_color, line_width, Shape::Circle));
        if self.shows_story_ring {
            let has_unseen = match self.ring_state {
                PostRowStoryRingState::Unseen => Some(true),
                PostRowStoryRingState::Seen => Some(false),
                PostRowStoryRingState::NoStories | PostRowStoryRingState::Unknown => None,
            };
            if let Some(has_unseen) = has_unseen {
                root = root.child(StoryRingUi::new(side, has_unseen, rise_ui::theme(cx).clone()));
            }
        }
        root
    }
}

/// off main decode of a local logo or cover file, downsampled
pub enum PostRowLocalImageDecoder {}

impl PostRowLocalImageDecoder {
    pub fn decode(source: &str, maximum_pixel_dimension: i64) -> Option<CleverImage> {
        let path = if source.starts_with("file://") {
            Url::from_string(source)?.to_file_path()?
        } else if source.starts_with('/') || Path::new(source).is_absolute() {
            PathBuf::from(source)
        } else {
            return None;
        };
        let data = std::fs::read(path).ok()?;
        CleverImageBitmapDecoder::decode(&data, Some(maximum_pixel_dimension))
    }

    pub async fn decode_async(source: String, maximum_pixel_dimension: i64, cx: &AsyncApp) -> Option<CleverImage> {
        CleverImageBitmapLoader::prepare_bitmap(move || Self::decode(&source, maximum_pixel_dimension), cx).await
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PostRowPremiumDiamondKey {
    point_size: u32,
    scale: u32,
}

type PostRowPremiumDiamondWaiter = Box<dyn FnOnce(CleverImage, &mut App)>;

#[derive(Default)]
struct PostRowPremiumDiamondState {
    images: HashMap<PostRowPremiumDiamondKey, CleverImage>,
    unavailable: HashSet<PostRowPremiumDiamondKey>,
    waiters: HashMap<PostRowPremiumDiamondKey, Vec<PostRowPremiumDiamondWaiter>>,
}

thread_local! {
    static PREMIUM_DIAMOND: RefCell<PostRowPremiumDiamondState> = RefCell::new(PostRowPremiumDiamondState::default());
}

/// still of the premium mark lottie at the reduce motion pose (progress 0.5
/// bare icon), rendered once per size
/// the native header only draws the still
pub enum PostRowPremiumDiamond {}

impl PostRowPremiumDiamond {
    pub fn cached_image(point_size: f32, scale: f32) -> Option<CleverImage> {
        let key = Self::key(point_size, scale);
        PREMIUM_DIAMOND.with_borrow(|state| state.images.get(&key).cloned())
    }

    /// delivers the still, renders off main at most once per size and never inside the callers pass
    pub fn request(point_size: f32, scale: f32, cx: &mut App, completion: impl FnOnce(CleverImage, &mut App) + 'static) {
        let key = Self::key(point_size, scale);
        if let Some(image) = PREMIUM_DIAMOND.with_borrow(|state| state.images.get(&key).cloned()) {
            completion(image, cx);
            return;
        }
        let starts_render = PREMIUM_DIAMOND.with_borrow_mut(|state| {
            if state.unavailable.contains(&key) {
                return false;
            }
            let is_running = state.waiters.contains_key(&key);
            state.waiters.entry(key).or_default().push(Box::new(completion));
            !is_running
        });
        if starts_render {
            Self::render(key, cx);
        }
    }

    /// renders the header still before the first premium row, call when the list settles
    pub fn warm(scale: f32, cx: &mut App) {
        Self::request(PostRowMetrics::BADGE_SIZE, scale, cx, |_, _| {});
    }

    fn key(point_size: f32, scale: f32) -> PostRowPremiumDiamondKey {
        PostRowPremiumDiamondKey { point_size: point_size.to_bits(), scale: scale.to_bits() }
    }

    fn render(key: PostRowPremiumDiamondKey, cx: &mut App) {
        let model = LottieView::load(PremiumDiamond::ANIMATION, cx);
        cx.spawn(async move |cx: &mut AsyncApp| {
            let image = match model.await {
                Some(model) => {
                    cx.background_executor()
                        .spawn(async move {
                            Self::make_image(&model, f32::from_bits(key.point_size), f32::from_bits(key.scale))
                        })
                        .await
                }
                None => None,
            };
            cx.update(|cx: &mut App| Self::finish(key, image, cx));
        })
        .detach();
    }

    fn finish(key: PostRowPremiumDiamondKey, image: Option<CleverImage>, cx: &mut App) {
        let pending = PREMIUM_DIAMOND.with_borrow_mut(|state| {
            let pending = state.waiters.remove(&key).unwrap_or_default();
            match image.as_ref() {
                Some(image) => {
                    state.images.insert(key, image.clone());
                }
                None => {
                    state.unavailable.insert(key);
                }
            }
            pending
        });
        let Some(image) = image else {
            return;
        };
        for completion in pending {
            completion(image.clone(), cx);
        }
    }

    fn make_image(model: &LottieViewModel, point_size: f32, scale: f32) -> Option<CleverImage> {
        let mut rasterizer = model.open_rasterizer()?;
        let frame_count = rasterizer.frame_count();
        if frame_count == 0 {
            return None;
        }
        let resolved_scale = if scale.is_finite() && scale > 0.0 { scale } else { 3.0 };
        let side = (point_size * resolved_scale).ceil().max(1.0) as u32;
        let mut pixels = vec![0_u32; (side as usize) * (side as usize)];
        rasterizer.render((frame_count - 1) / 2, side, &mut pixels);
        let mut bytes = Vec::with_capacity(pixels.len() * 4);
        for pixel in &pixels {
            let [blue, green, red, alpha] = pixel.to_le_bytes();
            bytes.extend_from_slice(&[red, green, blue, alpha]);
        }
        let rgba = RgbaImage::from_raw(side, side, bytes)?;
        Some(CleverImageBitmapDecoder::render_image(rgba))
    }
}

pub struct PostRowDateTicker {
    headers: HashMap<EntityId, WeakEntity<PostRowHeaderView>>,
    timer: Option<RepeatingTimer>,
}

impl PostRowDateTicker {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { headers: HashMap::new(), timer: None }
    }

    fn interval() -> Duration {
        let ms = RELATIVE_TIME_BANDS.iter().find(|band| band.shows_count).map_or(0, |band| band.divisor_ms);
        Duration::from_secs_f64((ms as f64 / 1_000.0).max(1.0))
    }

    pub fn track(&mut self, header: WeakEntity<PostRowHeaderView>, is_tracked: bool, cx: &mut Context<Self>) {
        if is_tracked {
            self.headers.insert(header.entity_id(), header);
        } else {
            self.headers.remove(&header.entity_id());
        }
        if self.headers.is_empty() {
            self.stop_timer();
            return;
        }
        if self.timer.is_some() {
            return;
        }
        let interval = Self::interval();
        let mut timer = RepeatingTimer::scheduled_timer(interval, true, cx, |this: &mut Self, cx| this.tick(cx));
        timer.set_tolerance(interval / 10);
        self.timer = Some(timer);
    }

    fn stop_timer(&mut self) {
        if let Some(timer) = self.timer.as_mut() {
            timer.invalidate();
        }
        self.timer = None;
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        self.headers.retain(|_, header| header.upgrade().is_some());
        if self.headers.is_empty() {
            self.stop_timer();
            return;
        }
        for header in self.headers.values() {
            let _ = header.update(cx, |header: &mut PostRowHeaderView, cx: &mut Context<PostRowHeaderView>| {
                header.refresh_relative_date(cx);
            });
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct PostRowDateTextKey {
    locale: String,
    calendar: CalendarIdentifier,
    time_zone: String,
}

thread_local! {
    static RELATIVE_FORMATTER: RefCell<Option<(PostRowDateTextKey, RelativeDateTimeFormatter)>> = const { RefCell::new(None) };
    static EXACT_FORMATTER: RefCell<Option<(PostRowDateTextKey, DateFormatter)>> = const { RefCell::new(None) };
    static MUSIC_LINE_CACHE: RefCell<HashMap<PostRowMusicLineTextKey, String>> = RefCell::new(HashMap::new());
}

/// relative and exact dates, formatters cached per locale and calendar
pub enum PostRowDateText {}

impl PostRowDateText {
    /// the now label for a stamp under a minute old or ahead of
    /// this device clock, the system formatter would print in 0 sec
    pub fn now_band_label(date: &Date, now: &Date) -> Option<String> {
        let band = RELATIVE_TIME_BANDS.first()?;
        if band.shows_count {
            return None;
        }
        let bound = band.upper_bound_ms?;
        if now.time_interval_since(date) * 1000.0 >= bound as f64 {
            return None;
        }
        Some(RiseFormat::elapsed_since_date(date, now))
    }

    pub fn relative(date: &Date, locale: &Locale, calendar: &Calendar) -> String {
        let now = Date::now();
        if let Some(label) = Self::now_band_label(date, &now) {
            return label;
        }
        let key = PostRowDateTextKey {
            locale: locale.identifier().to_owned(),
            calendar: calendar.identifier,
            time_zone: String::new(),
        };
        RELATIVE_FORMATTER.with_borrow_mut(|cache| {
            if let Some((cached_key, formatter)) = cache.as_ref()
                && *cached_key == key
            {
                return formatter.localized_string_for(date, &now);
            }
            let mut formatter = RelativeDateTimeFormatter::new();
            formatter.set_units_style(UnitsStyle::Short);
            formatter.set_locale(locale.identifier());
            formatter.set_calendar(calendar.clone());
            let text = formatter.localized_string_for(date, &now);
            *cache = Some((key, formatter));
            text
        })
    }

    pub fn exact(date: &Date, locale: &Locale, calendar: &Calendar, time_zone: &TimeZone) -> String {
        let key = PostRowDateTextKey {
            locale: locale.identifier().to_owned(),
            calendar: calendar.identifier,
            time_zone: time_zone.identifier(),
        };
        EXACT_FORMATTER.with_borrow_mut(|cache| {
            if let Some((cached_key, formatter)) = cache.as_ref()
                && *cached_key == key
            {
                return formatter.string_from(date);
            }
            let mut formatter = DateFormatter::new();
            formatter.set_locale(locale.identifier());
            formatter.set_calendar(calendar.clone());
            formatter.set_time_zone(time_zone.clone());
            formatter.set_date_style(Style::Long);
            formatter.set_time_style(Style::Medium);
            let text = formatter.string_from(date);
            *cache = Some((key, formatter));
            text
        })
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct PostRowMusicLineTextKey {
    track: PostRowSeedTrack,
    locale_identifier: String,
}

/// music line text for a seed track, memoized per track and language
pub enum PostRowMusicLineText {}

impl PostRowMusicLineText {
    pub fn text(track: &PostRowSeedTrack, locale_identifier: &str) -> String {
        let key = PostRowMusicLineTextKey { track: track.clone(), locale_identifier: locale_identifier.to_owned() };
        if let Some(hit) = MUSIC_LINE_CACHE.with_borrow(|cache| cache.get(&key).cloned()) {
            return hit;
        }
        let model = TrackModel {
            id: track.id.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            artist_id: String::new(),
            album_title: String::new(),
            album_id: String::new(),
            duration_seconds: 0,
            cover_colors: Vec::new(),
            cover_image_url: track.cover_url.clone(),
            audio_url: String::new(),
            is_exclusive: false,
            status: if track.is_original_audio {
                RiseMusicTrackStatus::OriginalAudio.wire().to_string()
            } else {
                String::new()
            },
            description: None,
            source_chat_id: None,
            source_message_id: None,
            artist_is_registered: false,
            remote_audio_url: None,
            transfer_phase: None,
            transferred_bytes: 0,
            transfer_total_bytes: 0,
            transfer_error: None,
            library_owner_user_id: None,
            playback_cache_id: None,
            is_liked: false,
        };
        let text = Post::music_line_text(&model);
        MUSIC_LINE_CACHE.with_borrow_mut(|cache| {
            if cache.len() >= 256 {
                cache.clear();
            }
            cache.insert(key, text.clone());
        });
        text
    }
}
