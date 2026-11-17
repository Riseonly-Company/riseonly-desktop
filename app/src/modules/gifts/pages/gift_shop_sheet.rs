use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, AnyView, App, Axis, Bounds, Context, ElementId, Empty, Entity, FontWeight, Hsla, Pixels, ScrollHandle,
    SharedString, Size, StyleRefinement, Subscription, Task, Window, black, canvas, point, white,
};
use rise_canon::rise_key::gift_colors;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::{character_prefix, trimming_whitespaces};
use rise_i18n::{FormatArg, format_printf, tr, tr_plural_with};
use rise_platform::haptics::selection_changed;
use rise_theme::ThemeStore;
use rise_theme::theme::app_theme::{AppTheme, color};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::IconUi;
use rise_ui::animated_tabs_ui::animated_tabs_types::{AnimatedTabsMetrics, TabConfig};
use rise_ui::button_ui::ButtonUi;
use rise_ui::input_ui::Confirm;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonShimmerExt, SkeletonUi};
use rise_ui::swiftui::animated::{Animated, Lerp};
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::rich_text::{AttributedString, RichFont, RichText};
use rise_ui::swiftui::scroll_view::{ScrollView, scroll_view};
use rise_ui::swiftui::transition::UnitPoint;
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::{NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_field_rule::{RiseFocusedTextInput, RiseFocusedTextInputOutcome};
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::ui::animated_tabs_ui::animated_tabs::AnimatedTabs;
use crate::core::ui::bottom_sheet_ui::sheet_floating_header::{SheetFloatingHeader, SheetFloatingHeaderMetrics};
use crate::core::ui::premium_diamond::PremiumDiamond;
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::gifts::components::gift_animation_view::GiftAnimationView;
use crate::modules::gifts::components::gift_card_view::{
    GiftBackdropColors, GiftCardView, GiftCornerRibbon, GiftPatternLayer, GiftRarityPill, GiftRarityStyle,
    GiftSerialFormat,
};
use crate::modules::gifts::components::gift_cosmetic_playback::GiftCosmeticGeometry;
use crate::modules::gifts::components::gift_cosmetic_preview::{
    GiftBackdropSurface, GiftCosmeticPreview, GiftCosmeticStage, GiftEquippedBadge,
};
use crate::modules::gifts::components::gift_info_table::{GiftOddsSummary, GiftOddsSummaryPlaceholder};
use crate::modules::gifts::components::gift_shop_shelf_grid::{
    GiftScrollWindowProbe, GiftShopCatalogIsActiveKey, GiftShopShelfGrid,
};
use crate::modules::gifts::components::gift_shop_sparkle_field::GiftShopSparkleField;
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftAttributeDTO, RiseGiftAttributeExt, RiseGiftAttributeKind, RiseGiftCatalogItemDTO, RiseGiftCatalogItemExt,
    RiseGiftCatalogItemResponseExt, RiseGiftInstanceDTO, RiseGiftInstanceExt, RiseGiftItemKind, RiseGiftRarityClass,
};
use crate::modules::gifts::shared::gift_market_kinds::GiftMarketKinds;
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::{
    GIFT_SHOP_SHEET_FOOTER_VERTICAL_PADDING, GiftRecipient, GiftShopFilter, GiftsInteractionsStore, RiseGiftsLimits,
};
use crate::modules::gifts::stores::gifts::gifts_services::gifts_services::GiftsServicesStore;
use crate::modules::post::components::post::post_skeleton::PostGridSkeleton;
use crate::modules::sparks::components::sparks_icon_view::{SparksAmountLabel, SparksIconView};
use crate::modules::sparks::stores::sparks::sparks_interactions::sparks_interactions::SparksInteractionsStore;
use crate::modules::sparks::stores::sparks::sparks_services::sparks_services::SparksServicesStore;
use crate::modules::user::stores::subscription::premium_gift_store::PremiumGiftStore;
use crate::modules::user::stores::subscription::subscription_actions::subscription_actions::SubscriptionActionsStore;
use crate::modules::user::stores::subscription::subscription_services::subscription_services::{
    Product, SubscriptionServicesStore,
};

fn current_theme(cx: &mut App) -> AppTheme {
    let store = ThemeStore::shared(cx);
    store.read(cx).current_theme().clone()
}

fn is_shop_for_self(cx: &mut App) -> bool {
    let store = GiftsInteractionsStore::shared(cx);
    store.update(cx, |store, cx| store.is_shop_for_self(cx))
}

fn is_current_user(id: &str, cx: &mut App) -> bool {
    let auth = AuthServicesStore::shared(cx);
    auth.read(cx).is_current_user(id)
}

/// scroll reader of a sheet page, scrolls over the page scroll handle
/// gpui has no anchors inside a scroll so the caller measures the target window bounds
pub(crate) struct GiftSheetScrollReader {
    handle: ScrollHandle,
    motion: Animated<f32>,
    is_moving: bool,
}

impl GiftSheetScrollReader {
    pub(crate) fn new() -> Self {
        Self { handle: ScrollHandle::new(), motion: Animated::new(0.0), is_moving: false }
    }

    pub(crate) fn handle(&self) -> ScrollHandle {
        self.handle.clone()
    }

    /// writes the running animation into the scroll, called from render before prepaint reads
    /// the offset
    pub(crate) fn frame(&mut self, window: &Window) {
        if !self.is_moving {
            return;
        }
        let now = RiseClock::monotonic();
        let top = self.motion.frame_value(now, window);
        let offset = self.handle.offset();
        self.handle.set_offset(point(offset.x, px(-top)));
        if !self.motion.is_running(now) {
            self.is_moving = false;
        }
    }

    pub(crate) fn cancel(&mut self) {
        self.is_moving = false;
    }

    /// puts the point anchor_y of item on the same point of the viewport, 0 top and 1 bottom
    /// clamped to the content
    /// duration is the call site ease out
    pub(crate) fn scroll_to(&mut self, item: Bounds<Pixels>, anchor_y: f32, duration: f64, cx: &mut Context<Self>) {
        let viewport = self.handle.bounds();
        let offset = self.handle.offset();
        let top = -f32::from(offset.y);
        let item_point = f32::from(item.top()) + anchor_y * f32::from(item.size.height);
        let viewport_point = f32::from(viewport.top()) + anchor_y * f32::from(viewport.size.height);
        let target = (top + item_point - viewport_point).clamp(0.0, f32::from(self.handle.max_offset().y).max(0.0));
        self.motion.jump(top);
        self.motion.set_in(target, ease_out(duration), RiseClock::monotonic(), cx);
        self.is_moving = true;
        cx.notify();
    }

    /// scrolls item into the viewport moving as little as it takes, duration is the call site ease out
    pub(crate) fn reveal(&mut self, item: Bounds<Pixels>, animated: bool, duration: f64, cx: &mut Context<Self>) {
        let viewport = self.handle.bounds();
        let below = f32::from(item.bottom() - viewport.bottom());
        let above = f32::from(viewport.top() - item.top());
        let offset = self.handle.offset();
        let top = -f32::from(offset.y);
        let target = if below > 0.0 {
            top + below
        } else if above > 0.0 {
            top - above
        } else {
            return;
        };
        let target = target.clamp(0.0, f32::from(self.handle.max_offset().y).max(0.0));
        if animated {
            self.motion.jump(top);
            self.motion.set_in(target, ease_out(duration), RiseClock::monotonic(), cx);
            self.is_moving = true;
        } else {
            self.is_moving = false;
            self.handle.set_offset(point(offset.x, px(-target)));
        }
        cx.notify();
    }
}

#[derive(Default)]
struct GiftShopSheetContentState {
    viewport: Size<f32>,
}

#[derive(IntoElement)]
pub struct GiftShopSheetContent;

impl GiftShopSheetContent {
    pub fn new() -> Self {
        Self
    }

    /// each visible page has one scroll view for sheet dragging
    fn page_scroll(id: impl Into<ElementId>, handle: Option<ScrollHandle>, content: impl IntoElement) -> ScrollView {
        let mut scroll = scroll_view(id).shows_indicators(false);
        if let Some(handle) = handle {
            scroll = scroll.scroll_handle(handle);
        }
        scroll.child(
            vstack(0.0)
                .w_full()
                .pb(px(24.0))
                .child(div().w_full().flex_none().h(px(SheetFloatingHeaderMetrics::HEIGHT)))
                .child(content),
        )
    }

    fn header_title(
        sent_instance: Option<&RiseGiftInstanceDTO>,
        selected_gift: Option<&RiseGiftCatalogItemDTO>,
        for_self: bool,
    ) -> Option<String> {
        if sent_instance.is_some() {
            return None;
        }
        if let Some(gift) = selected_gift {
            return Some(gift.name.clone());
        }
        Some(tr(if for_self { "gift_market_title_self" } else { "gift_market_title_gift" }))
    }
}

/// the shop content as its own cached view in the sheets host
pub struct GiftShopSheetContentView;

impl GiftShopSheetContentView {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GiftShopSheetContentView {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for GiftShopSheetContentView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        tracing::warn!(target: "riseonly::X2374", "gift shop content render");
        GiftShopSheetContent::new()
    }
}

impl Default for GiftShopSheetContent {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for GiftShopSheetContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let interactions = GiftsInteractionsStore::shared(cx);
        let premium = PremiumGiftStore::shared(cx);
        let state = window.use_keyed_state("gift-shop-sheet", cx, |_, _| GiftShopSheetContentState::default());
        let viewport = state.read(cx).viewport;
        let for_self = is_shop_for_self(cx);

        let store = interactions.read(cx);
        let recipient = store.shop_recipient.clone();
        let sent_instance = store.sent_instance.clone();
        let selected_gift = store.selected_gift.clone();
        let is_presented = store.is_shop_presented();
        let delivered_count = premium.read(cx).delivered_count();

        let page_key = if let Some(sent) = &sent_instance {
            format!("sent:{}", sent.id)
        } else if let Some(gift) = &selected_gift {
            format!("form:{}", gift.id)
        } else {
            "catalog".to_owned()
        };
        let is_on_form = selected_gift.is_some() && sent_instance.is_none();
        let is_on_catalog = page_key == "catalog";

        let closer = interactions.clone();
        on_change("gift-shop-delivered", delivered_count, false, window, cx, move |_, _, _, cx| {
            closer.update(cx, |store, cx| store.close_shop(cx));
        });

        let recipient_key = recipient.as_ref().map(|recipient| recipient.id.as_str()).unwrap_or_default();
        let catalog_scroll = Self::page_scroll(
            SharedString::from(format!("gift-shop-catalog/{recipient_key}")),
            None,
            GiftShopCatalogPage::new(viewport),
        );
        let catalog = div()
            .absolute()
            .inset_0()
            .when(!is_on_catalog, |layer| layer.invisible())
            .child(env_scope::<GiftShopCatalogIsActiveKey>(is_on_catalog && is_presented).child(catalog_scroll));

        let detail = (!is_on_catalog).then(|| {
            let reader_id = SharedString::from(format!("gift-shop-page/{page_key}"));
            let reader = window.use_keyed_state(reader_id.clone(), cx, |_, _| GiftSheetScrollReader::new());
            reader.update(cx, |reader, _| reader.frame(window));
            let handle = reader.read(cx).handle();
            let page = if let Some(sent) = sent_instance.clone() {
                GiftSentView::new(sent, recipient.clone()).into_any_element()
            } else if let Some(gift) = selected_gift.clone() {
                GiftSendForm::new(gift, recipient.clone(), reader).into_any_element()
            } else {
                Empty.into_any_element()
            };
            div().absolute().inset_0().child(Self::page_scroll(reader_id, Some(handle), page))
        });

        let title = Self::header_title(sent_instance.as_ref(), selected_gift.as_ref(), for_self);
        let leading = interactions.clone();
        let header = SheetFloatingHeader::new("gift-shop-header", title.clone().unwrap_or_default(), move |_, cx| {
            leading.update(cx, |store, cx| {
                if is_on_form {
                    store.back_to_catalog(cx);
                } else {
                    store.close_shop(cx);
                }
            });
        })
        .shows_title(title.is_some())
        .leading_system_image(if is_on_form { "chevron.backward" } else { "xmark" })
        .trailing(GiftShopBalance);

        div()
            .relative()
            .size_full()
            .child(catalog)
            .children(detail)
            .child(measure("gift-shop-viewport", move |bounds, _, cx| {
                let viewport = Size { width: f32::from(bounds.size.width), height: f32::from(bounds.size.height) };
                state.update(cx, |state, cx| {
                    if state.viewport != viewport {
                        state.viewport = viewport;
                        cx.notify();
                    }
                });
            }))
            .child(div().absolute().top_0().left_0().right_0().child(header))
    }
}

#[derive(IntoElement)]
struct GiftShopBalance;

impl RenderOnce for GiftShopBalance {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = current_theme(cx);
        let sparks = SparksServicesStore::shared(cx);
        let sparks = sparks.read(cx);
        if !sparks.has_balance() {
            return Empty.into_any_element();
        }
        plain("gift-shop-balance")
            .on_press(|_, cx| {
                SparksInteractionsStore::shared(cx).update(cx, |store, cx| store.open_purchase_sheet(None, cx));
            })
            .child(
                vstack(1.0)
                    .align(HorizontalAlignment::Trailing)
                    .child(MainText::new(tr("gift_shop_balance_title")).px(13.0).color(theme.text.secondary).line_limit(1))
                    .child(
                        SparksAmountLabel::new(sparks.spendable_now())
                            .px(16.0)
                            .icon_size(16.0)
                            .weight(FontWeight::BOLD)
                            .color(theme.text.primary),
                    ),
            )
            .into_any_element()
    }
}

#[derive(IntoElement)]
struct GiftShopCatalogPage {
    viewport: Size<f32>,
}

impl GiftShopCatalogPage {
    fn new(viewport: Size<f32>) -> Self {
        Self { viewport }
    }

    fn shows_premium(recipient: &GiftRecipient, cx: &mut App) -> bool {
        if is_current_user(&recipient.id, cx) {
            return false;
        }
        let premium = PremiumGiftStore::shared(cx);
        let premium = premium.read(cx);
        premium.is_loading_products() || !premium.products().is_empty()
    }

    fn section_title(title: String, theme: &AppTheme) -> Div {
        div()
            .w_full()
            .px(px(24.0))
            .text_center()
            .swiftui_font(Font::system_weight(26.0, FontWeight::BOLD))
            .text_color(theme.text.primary)
            .child(SharedString::from(title))
    }

    fn section_text(id: &'static str, key: &str, name: &str, theme: &AppTheme) -> Div {
        div()
            .w_full()
            .px(px(28.0))
            .pt(px(8.0))
            .text_center()
            .swiftui_font(Font::system_size(15.0))
            .text_color(theme.text.primary)
            .child(RichText::new(id, Self::emphasizing_name(&tr(key), name)))
    }

    fn more_link(
        id: &'static str,
        title: String,
        theme: &AppTheme,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> PlainButton {
        let primary = theme.primary._100;
        plain(id).on_press(action).mt(px(4.0)).child(
            hstack(3.0)
                .swiftui_font(Font::system_size(15.0))
                .text_color(primary)
                .child(SharedString::from(title))
                .children(IconUi::sized("chevron.right", px(12.0), primary, false)),
        )
    }

    fn emphasizing_name(template: &str, name: &str) -> AttributedString {
        let token = "\u{FFFC}";
        let mut sentence = AttributedString::new(format_printf(template, &[FormatArg::Text(token.into())]));
        if let Some(range) = sentence.range_of(token) {
            let mut emphasized = AttributedString::new(name);
            emphasized.update_all(|attributes| {
                attributes.font = Some(RichFont::system_weight(15.0, FontWeight::SEMIBOLD));
            });
            sentence.replace_subrange(range, emphasized);
        }
        sentence
    }
}

impl RenderOnce for GiftShopCatalogPage {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = current_theme(cx);
        let interactions = GiftsInteractionsStore::shared(cx);
        let recipient = interactions.read(cx).shop_recipient.clone();
        let for_self = is_shop_for_self(cx);
        let nav_store = env::<NavigationStoreKey>(cx);

        let mut page = vstack(0.0).w_full();
        if let Some(recipient) = &recipient {
            page = page.child(GiftShopHero::new(recipient.clone()));
            if Self::shows_premium(recipient, cx) {
                let closer = interactions.clone();
                page = page
                    .child(Self::section_title(tr("premium_gift_title"), &theme).mt(px(10.0)))
                    .child(Self::section_text("gift-shop-premium-desc", "gift_shop_premium_desc", &recipient.name, &theme))
                    .child(Self::more_link("gift-shop-premium-more", tr("gift_shop_premium_more"), &theme, move |_, cx| {
                        closer.update(cx, |store, cx| store.close_shop(cx));
                        if let Some(nav_store) = nav_store.as_ref() {
                            nav_store.update(cx, |nav_store, cx| {
                                nav_store.push(RootRoute::SubscriptionSettings, NavigationStoreSettlingPush::Drop, cx);
                            });
                        }
                    }))
                    .child(div().w_full().px(px(16.0)).pt(px(18.0)).pb(px(26.0)).child(GiftShopPremiumPlans));
            }
        }
        if for_self {
            page = page.child(
                div()
                    .w_full()
                    .px(px(28.0))
                    .pt(px(8.0))
                    .text_center()
                    .swiftui_font(Font::system_size(15.0))
                    .text_color(theme.text.primary)
                    .child(SharedString::from(tr("gift_market_self_items_desc"))),
            );
        } else {
            page = page.child(Self::section_title(tr("gift_shop_send_title"), &theme).mt(px(10.0)));
            if let Some(recipient) = &recipient {
                page = page.child(Self::section_text("gift-shop-gifts-desc", "gift_shop_gifts_desc", &recipient.name, &theme));
            }
        }
        let opener = interactions;
        page.child(Self::more_link("gift-shop-sparks-more", tr("gift_shop_sparks_more"), &theme, move |_, cx| {
            opener.update(cx, |store, cx| store.open_sparks_info(cx));
        }))
        .child(div().w_full().pt(px(18.0)).child(GiftShopShelves::new(self.viewport)))
    }
}

#[derive(IntoElement)]
struct GiftShopHero {
    recipient: GiftRecipient,
}

impl GiftShopHero {
    const AVATAR_SIZE: f32 = 100.0;
    const FIELD_HEIGHT: f32 = 210.0;
    const HEIGHT: f32 = 140.0;

    fn new(recipient: GiftRecipient) -> Self {
        Self { recipient }
    }
}

impl RenderOnce for GiftShopHero {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_near_viewport = window.use_keyed_state("gift-shop-hero-near", cx, |_, _| true);
        let is_active = *is_near_viewport.read(cx);
        let recipient = self.recipient;
        let source: Option<SharedString> = (!recipient.avatar.is_empty()).then(|| recipient.avatar.clone().into());
        div()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .flex_none()
            .h(px(Self::HEIGHT))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px((Self::HEIGHT - Self::FIELD_HEIGHT) / 2.0))
                    .h(px(Self::FIELD_HEIGHT))
                    .child(GiftShopSparkleField::new().source_radius(Self::AVATAR_SIZE / 2.0).is_active(is_active))
                    .child(div().absolute().inset_0().child(GiftScrollWindowProbe::new(
                        "gift-shop-hero-probe",
                        move |band, height, cx| {
                            let near = band.is_some_and(|band| *band.end() > -height && *band.start() < 2.0 * height);
                            is_near_viewport.update(cx, |current, cx| {
                                if near != *current {
                                    *current = near;
                                    cx.notify();
                                }
                            });
                        },
                    ))),
            )
            .child(
                UserLogoUi::new("gift-shop-hero-logo")
                    .source(source)
                    .name(Some(SharedString::from(recipient.name)))
                    .user_id(Some(SharedString::from(recipient.id)))
                    .size(Self::AVATAR_SIZE)
                    .show_story_ring(false),
            )
    }
}

#[derive(IntoElement)]
struct GiftShopShelves {
    viewport: Size<f32>,
}

impl GiftShopShelves {
    const TABS_BOTTOM_PADDING: f32 = 12.0;

    fn new(viewport: Size<f32>) -> Self {
        Self { viewport }
    }

    fn minimum_page_height(&self) -> f32 {
        let below = self.viewport.height
            - SheetFloatingHeaderMetrics::HEIGHT
            - f32::from(AnimatedTabsMetrics::BAR_HEIGHT)
            - Self::TABS_BOTTOM_PADDING;
        below.max(220.0)
    }
}

impl RenderOnce for GiftShopShelves {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let viewport = self.viewport;
        let kinds = RiseGiftItemKind::SHOP_ORDER;
        let tabs = kinds
            .iter()
            .map(|kind| {
                let kind = *kind;
                TabConfig::new(kind.shop_index() as i64, tr(kind.shop_tab_key()), move |window, cx| {
                    GiftShopShelfPageView::page(kind, viewport, window, cx)
                })
            })
            .collect();
        let active_tab = Binding::entity(
            GiftsInteractionsStore::shared(cx),
            |store| store.shop_kind.shop_index() as i64,
            move |store, index, cx| {
                let Some(kind) = usize::try_from(index).ok().and_then(|index| kinds.get(index)) else {
                    return;
                };
                store.select_shop_kind(*kind, cx);
            },
        );
        AnimatedTabs::new("gift-shop-shelves", tabs, active_tab)
            .horizontal_padding(16.0)
            .top_padding(0.0)
            .bottom_padding(Self::TABS_BOTTOM_PADDING)
            .sizes_pages_vertically_to_content(true)
            .minimum_page_height(self.minimum_page_height())
    }
}

#[derive(IntoElement)]
struct GiftShopShelf {
    kind: RiseGiftItemKind,
    viewport: Size<f32>,
}

impl GiftShopShelf {
    fn new(kind: RiseGiftItemKind, viewport: Size<f32>) -> Self {
        Self { kind, viewport }
    }
}

impl RenderOnce for GiftShopShelf {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        if self.kind == RiseGiftItemKind::Item {
            GiftShopItemShelf { viewport: self.viewport }.into_any_element()
        } else {
            GiftShopCosmeticShelf { kind: self.kind, viewport: self.viewport }.into_any_element()
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct GiftShopShelfPageProps {
    kind: RiseGiftItemKind,
    viewport: Size<f32>,
}

/// one page of the shelves as its own view, the page on screen renders with the shop and a
/// page beside it is cached
struct GiftShopShelfPageView {
    props: Option<GiftShopShelfPageProps>,
    natural_height: Rc<Cell<f32>>,
    renders_cached: Rc<Cell<bool>>,
    warm: bool,
    warm_task: Option<Task<()>>,
    _observations: [Subscription; 3],
}

impl GiftShopShelfPageView {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = GiftsServicesStore::shared(cx);
        let interactions = GiftsInteractionsStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            props: None,
            natural_height: Rc::new(Cell::new(0.0)),
            renders_cached: Rc::new(Cell::new(false)),
            warm: false,
            warm_task: None,
            _observations: [
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&interactions, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }

    const WARM_DELAY: Duration = Duration::from_millis(450);
    const WARM_STAGGER: Duration = Duration::from_millis(60);

    fn warm_up(&mut self, is_active: bool, rank: u32, cx: &mut Context<Self>) {
        if self.warm {
            return;
        }
        if is_active {
            self.warm = true;
            self.warm_task = None;
            return;
        }
        if self.warm_task.is_some() {
            return;
        }
        let delay = Self::WARM_DELAY + Self::WARM_STAGGER * rank;
        self.warm_task = Some(cx.spawn(async move |page, cx| {
            cx.background_executor().timer(delay).await;
            page.update(cx, |page, cx| {
                page.warm_task = None;
                if !page.warm {
                    page.warm = true;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn set_props(&mut self, props: GiftShopShelfPageProps) -> bool {
        if self.props == Some(props) {
            return false;
        }
        self.props = Some(props);
        true
    }

    fn page(kind: RiseGiftItemKind, viewport: Size<f32>, window: &mut Window, cx: &mut App) -> AnyElement {
        let active_kind = GiftsInteractionsStore::shared(cx).read(cx).shop_kind;
        let is_active = active_kind == kind;
        let rank = RiseGiftItemKind::SHOP_ORDER
            .iter()
            .take_while(|other| **other != kind)
            .filter(|other| **other != active_kind)
            .count() as u32;
        let id = SharedString::from(format!("gift-shop-shelf-page-view/{}", kind.slug()));
        let view = window.use_keyed_state(id, cx, |_, cx: &mut Context<GiftShopShelfPageView>| {
            tracing::warn!(target: "riseonly::X2374", kind = kind.slug(), "gift shop shelf page view created");
            GiftShopShelfPageView::new(cx)
        });
        let (stale, height) = view.update(cx, |view, cx| {
            view.warm_up(is_active, rank, cx);
            (view.set_props(GiftShopShelfPageProps { kind, viewport }), view.natural_height.get())
        });
        let cached = !is_active && !stale && height > 0.0;
        view.read(cx).renders_cached.set(cached);
        if cached {
            view.cached(StyleRefinement::default().w_full().h(px(height))).replay_when_moved().into_any_element()
        } else {
            AnyView::from(view).into_any_element()
        }
    }
}

impl Render for GiftShopShelfPageView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        tracing::warn!(target: "riseonly::X2374", warm = self.warm, cached = self.renders_cached.get(), "gift shop shelf page render");
        let Some(props) = self.props.filter(|_| self.warm) else {
            return div().w_full();
        };
        let natural_height = self.natural_height.clone();
        let renders_cached = self.renders_cached.clone();
        let page = cx.entity().downgrade();
        div().relative().w_full().child(GiftShopShelf::new(props.kind, props.viewport)).child(
            canvas(
                move |bounds, window, cx| {
                    let height = f32::from(bounds.size.height);
                    if (natural_height.get() - height).abs() <= 0.5 {
                        return;
                    }
                    natural_height.set(height);
                    if renders_cached.get() {
                        window.defer(cx, move |_, cx| {
                            page.update(cx, |_, cx| cx.notify()).ok();
                        });
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
    }
}

fn select_gift(gift: &RiseGiftCatalogItemDTO, cx: &mut App) {
    let gift = gift.clone();
    GiftsInteractionsStore::shared(cx).update(cx, |store, cx| store.select(gift, cx));
}

fn shelf_error() -> Div {
    div()
        .w_full()
        .py(px(40.0))
        .child(SecondaryText::new(tr("retry_later_error")).px(14.0).tac(TextAlignT::Center))
}

#[derive(IntoElement)]
struct GiftShopItemShelf {
    viewport: Size<f32>,
}

struct GiftShopItemShelfMotion {
    filter: GiftShopFilter,
    fade: Animated<f32>,
    height: Animated<f32>,
    natural_height: Rc<Cell<f32>>,
    height_before_change: f32,
    awaits_new_height: bool,
}

impl RenderOnce for GiftShopItemShelf {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let services = GiftsServicesStore::shared(cx);
        let services = services.read(cx);
        let state = services.catalog(RiseGiftItemKind::Item).clone();
        let item_shelf_has_filters = services.item_shelf_has_filters();
        let shop_filter = GiftsInteractionsStore::shared(cx).read(cx).shop_filter;

        let content = if !state.loaded {
            if state.failed {
                shelf_error().into_any_element()
            } else {
                vstack(14.0)
                    .w_full()
                    .when(item_shelf_has_filters, |stack| stack.child(GiftShopFilters::new(true)))
                    .child(GiftShopShelfSkeleton { kind: RiseGiftItemKind::Item })
                    .into_any_element()
            }
        } else if state.items.is_empty() {
            ChatProfileEmptyState::new("gift", tr("gift_shop_empty")).into_any_element()
        } else {
            let has_both_shelves =
                state.items.iter().any(|gift| gift.is_limited()) && state.items.iter().any(|gift| !gift.is_limited());
            let shelf: Vec<RiseGiftCatalogItemDTO> = if shop_filter == GiftShopFilter::Collectible && has_both_shelves {
                state.items.iter().filter(|gift| gift.is_limited()).cloned().collect()
            } else {
                state.items
            };
            let template_price = shelf.first().map(|gift| gift.price_sparks).unwrap_or(0);
            let motion = window.use_keyed_state("gift-shop-item-shelf-motion", cx, move |_, _| {
                tracing::warn!(target: "riseonly::X2374", "gift shop item shelf motion created");
                GiftShopItemShelfMotion {
                    filter: shop_filter,
                    fade: Animated::new(1.0),
                    height: Animated::new(0.0),
                    natural_height: Rc::new(Cell::new(0.0)),
                    height_before_change: 0.0,
                    awaits_new_height: false,
                }
            });
            let now = RiseClock::monotonic();
            motion.update(cx, |motion, cx| {
                let natural_height = motion.natural_height.get();
                if motion.filter != shop_filter {
                    motion.filter = shop_filter;
                    motion.fade.jump(0.0);
                    motion.fade.set_in(1.0, ease_out(0.18), now, cx);
                    motion.height.jump(natural_height);
                    motion.height_before_change = natural_height;
                    motion.awaits_new_height = true;
                } else if motion.awaits_new_height {
                    if (natural_height - motion.height_before_change).abs() > 0.5 {
                        motion.height.set_in(natural_height, ease_out(0.18), now, cx);
                        motion.awaits_new_height = false;
                    } else if !motion.fade.is_running(now) {
                        motion.awaits_new_height = false;
                    }
                }
            });
            let motion = motion.read(cx);
            let shelf_opacity = motion.fade.frame_value(now, window);
            let clamped_height = (motion.awaits_new_height || motion.height.is_running(now))
                .then(|| motion.height.frame_value(now, window));
            let natural_height = motion.natural_height.clone();
            vstack(14.0)
                .w_full()
                .when(has_both_shelves, |stack| stack.child(GiftShopFilters::new(false)))
                .child(
                    div().w_full().when_some(clamped_height, |clamp, height| clamp.h(px(height)).overflow_hidden()).child(
                    div().relative().w_full().px(px(GiftShopCellMetrics::GRID_INSET)).opacity(shelf_opacity).child(
                        GiftShopShelfGrid::new(
                            "gift-shop-shelf-item",
                            shelf,
                            RiseGiftItemKind::Item.shop_columns() as i64,
                            move |_, _| GiftShopCellBody::new(RiseGiftItemKind::Item, template_price, div().size_full()),
                            |gift: &RiseGiftCatalogItemDTO, _, _| {
                                let selected = gift.clone();
                                plain(SharedString::from(format!("gift-shop-cell/{}", gift.id)))
                                    .w_full()
                                    .disabled(gift.is_sold_out())
                                    .on_press(move |_, cx| select_gift(&selected, cx))
                                    .child(GiftShopCell { gift: gift.clone() })
                            },
                        )
                        .spacing(GiftShopCellMetrics::GRID_SPACING)
                        .initial_rows(GiftShopCellMetrics::initial_rows(RiseGiftItemKind::Item, self.viewport)),
                    )
                    .child(
                        canvas(
                            move |bounds, _, _| natural_height.set(f32::from(bounds.size.height)),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    ),
                    ),
                )
                .into_any_element()
        };
        div().w_full().pt(px(4.0)).child(content)
    }
}

#[derive(IntoElement)]
struct GiftShopCosmeticShelf {
    kind: RiseGiftItemKind,
    viewport: Size<f32>,
}

impl RenderOnce for GiftShopCosmeticShelf {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = current_theme(cx);
        let kind = self.kind;
        let state = GiftsServicesStore::shared(cx).read(cx).catalog(kind).clone();
        let recipient = GiftsInteractionsStore::shared(cx).read(cx).shop_recipient.clone();
        let preview_size = GiftShopCellMetrics::preview_size(kind, self.viewport);

        let content = if !state.loaded {
            if state.failed {
                shelf_error().into_any_element()
            } else {
                GiftShopShelfSkeleton { kind }.into_any_element()
            }
        } else if state.items.is_empty() {
            ChatProfileEmptyState::new("gift", tr("gift_shop_empty")).into_any_element()
        } else {
            let template_price = state.items.first().map(|gift| gift.price_sparks).unwrap_or(0);
            div()
                .w_full()
                .px(px(GiftShopCellMetrics::GRID_INSET))
                .child(
                    GiftShopShelfGrid::new(
                        SharedString::from(format!("gift-shop-shelf/{}", kind.slug())),
                        state.items,
                        kind.shop_columns() as i64,
                        move |_, _| {
                            div()
                                .relative()
                                .w_full()
                                .child(GiftShopCellBody::new(kind, template_price, div().size_full()))
                                .child(measure(
                                    SharedString::from(format!("gift-shop-card-size/{}", kind.slug())),
                                    move |bounds, _, _| GiftShopCardSize::remember(kind, bounds.size),
                                ))
                        },
                        move |gift: &RiseGiftCatalogItemDTO, window, cx| {
                            let props = GiftShopCosmeticLayerProps {
                                gift: gift.clone(),
                                kind,
                                recipient: recipient.clone(),
                                preview_size,
                            };
                            GiftShopCosmeticCellView::cell(props, window, cx)
                        },
                    )
                    .spacing(GiftShopCellMetrics::GRID_SPACING)
                    .initial_rows(GiftShopCellMetrics::initial_rows(kind, self.viewport)),
                )
                .into_any_element()
        };
        vstack(16.0)
            .w_full()
            .pt(px(4.0))
            .children(kind.shop_description_key().map(|key| {
                div()
                    .w_full()
                    .px(px(28.0))
                    .text_center()
                    .swiftui_font(Font::system_size(15.0))
                    .text_color(theme.text.secondary)
                    .child(SharedString::from(tr(key)))
            }))
            .child(content)
    }
}

struct GiftShopFilterChipFade {
    selected: Animated<f32>,
}

/// all gifts or only the numbered ones
#[derive(IntoElement)]
struct GiftShopFilters {
    is_placeholder: bool,
}

impl GiftShopFilters {
    fn new(is_placeholder: bool) -> Self {
        Self { is_placeholder }
    }

    fn chip(
        filter: GiftShopFilter,
        id: &'static str,
        title: String,
        is_placeholder: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let theme = current_theme(cx);
        let interactions = GiftsInteractionsStore::shared(cx);
        let selected = interactions.read(cx).shop_filter == filter;
        let target = if selected { 1.0 } else { 0.0 };
        let fade = window.use_keyed_state(SharedString::from(format!("{id}/fade")), cx, move |_, _| GiftShopFilterChipFade {
            selected: Animated::new(target),
        });
        let now = RiseClock::monotonic();
        fade.update(cx, |fade, cx| fade.selected.set_in(target, ease_out(0.18), now, cx));
        let progress = fade.read(cx).selected.frame_value(now, window);

        let label = MainText::new(title)
            .px(15.0)
            .font_weight(FontWeight::SEMIBOLD)
            .color(theme.text.secondary.lerp(&theme.text.primary, progress))
            .line_limit(1);
        plain(id)
            .disabled(is_placeholder)
            .on_press(move |_, cx| {
                if selected {
                    return;
                }
                let _ = selection_changed();
                interactions.update(cx, |store, cx| {
                    store.shop_filter = filter;
                    cx.notify();
                });
            })
            .child(
                div()
                    .px(px(14.0))
                    .py(px(8.0))
                    .capsule()
                    .bg(theme.bg._200.opacity(progress))
                    .child(redacted(label, is_placeholder, &theme)),
            )
            .into_any_element()
    }
}

impl RenderOnce for GiftShopFilters {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_placeholder = self.is_placeholder;
        hstack(6.0)
            .child(Self::chip(
                GiftShopFilter::All,
                "gift-shop-filter-all",
                tr("gift_shop_filter_all"),
                is_placeholder,
                window,
                cx,
            ))
            .child(Self::chip(
                GiftShopFilter::Collectible,
                "gift-shop-filter-collectible",
                tr("gift_shop_filter_collectible"),
                is_placeholder,
                window,
                cx,
            ))
    }
}

/// a label that keeps its size under a plain block while loading
fn redacted(label: impl IntoElement, is_placeholder: bool, theme: &AppTheme) -> AnyElement {
    if !is_placeholder {
        return label.into_any_element();
    }
    div()
        .relative()
        .child(div().invisible().child(label))
        .child(div().absolute().inset_0().rounded(px(4.0)).bg(SkeletonUi::shape_color(theme)))
        .into_any_element()
}

/// premium plans as gifts, a card per period with the price and the saving against a month
/// in the corner
/// a tap selects the plan and asks for the phone app, the one place it can be paid for
#[derive(IntoElement)]
struct GiftShopPremiumPlans;

impl GiftShopPremiumPlans {
    fn buy(product_id: &str, cx: &mut App) {
        if Self::is_busy(cx) {
            return;
        }
        let _ = selection_changed();
        let product_id = product_id.to_owned();
        PremiumGiftStore::shared(cx).update(cx, |store, cx| {
            store.select_product(product_id, cx);
            store.purchase_selected(cx);
        });
    }

    fn is_busy(cx: &mut App) -> bool {
        let verifying = SubscriptionActionsStore::shared(cx).read(cx).verify_gift_purchase_sai.is_pending();
        PremiumGiftStore::shared(cx).read(cx).is_purchasing() || verifying
    }
}

impl RenderOnce for GiftShopPremiumPlans {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_busy = Self::is_busy(cx);
        let store = PremiumGiftStore::shared(cx);
        let products = store.read(cx).shelf_products();
        if products.is_empty() {
            return PostGridSkeleton::default()
                .columns(3)
                .spacing(10.0)
                .cell_height(176.0)
                .rows(1)
                .into_any_element();
        }
        let selected_product_id = store.read(cx).selected_product_id().map(str::to_owned);
        let on_phone = store.read(cx).continue_on_phone().is_some();
        let services = SubscriptionServicesStore::shared(cx);
        let mut row = hstack(10.0).w_full().align(VerticalAlignment::Top);
        for product in products {
            let months = store.read(cx).months_for(&product);
            let saving = store.read(cx).savings_percent(&product);
            let price = services.read(cx).display_price(&product);
            let is_chosen = selected_product_id.as_deref() == Some(product.id.as_str());
            let is_pending = is_busy && is_chosen;
            let is_selected = on_phone && is_chosen;
            let product_id = product.id.clone();
            row = row.child(
                plain(SharedString::from(format!("gift-shop-plan/{}", product.id)))
                    .flex_1()
                    .min_w(px(0.0))
                    .disabled(is_busy)
                    .on_press(move |_, cx| Self::buy(&product_id, cx))
                    .child(GiftShopPremiumCell { product, months, saving, price, is_pending, is_selected }),
            );
        }
        let last_error = store.read(cx).last_error().filter(|error| !error.is_empty()).map(str::to_owned);
        tracing::warn!(target: "riseonly::X2374", on_phone, is_busy, "gift shop premium plans render");
        vstack(10.0)
            .w_full()
            .child(row)
            .children(last_error.map(|error| {
                div().w_full().px(px(12.0)).child(SecondaryText::new(error).px(13.0).tac(TextAlignT::Center))
            }))
            .on_disappear("gift-shop-premium-plans", move |_, _| {
                tracing::warn!(target: "riseonly::X2374", "gift shop premium plans left the frame");
            })
            .into_any_element()
    }
}

#[derive(IntoElement)]
struct GiftShopPremiumCell {
    product: Product,
    months: i64,
    saving: Option<i64>,
    price: String,
    is_pending: bool,
    is_selected: bool,
}

impl RenderOnce for GiftShopPremiumCell {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = current_theme(cx);
        let shape = Shape::continuous(20.0);
        let months = self.months;
        let id = self.product.id;
        let body = vstack(2.0)
            .w_full()
            .px(px(6.0))
            .pt(px(18.0))
            .pb(px(14.0))
            .child(div().pb(px(8.0)).child(PremiumDiamond::new().size(50.0).looping(false)))
            .child(
                MainText::new(tr_plural_with("premium_gift_months", months, FormatArg::from(months)))
                    .px(17.0)
                    .font_weight(FontWeight::BOLD)
                    .color(theme.text.primary)
                    .line_limit(1)
                    .minimum_scale_factor(0.75),
            )
            .child(MainText::new("Premium").px(14.0).color(theme.text.secondary).line_limit(1))
            .when(!self.price.is_empty(), |stack| {
                stack.child(
                    div().pt(px(10.0)).child(
                        div().px(px(10.0)).py(px(6.0)).capsule().bg(theme.primary._100.opacity(0.15)).child(
                            MainText::new(self.price)
                                .px(14.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .monospaced_digits(true)
                                .color(theme.primary._100)
                                .line_limit(1)
                                .minimum_scale_factor(0.7),
                        ),
                    ),
                )
            });
        shape
            .round(div().relative().w_full().bg(theme.bg._200).overflow_hidden())
            .child(body)
            .children(self.saving.map(|saving| {
                div().absolute().inset_0().child(GiftCornerRibbon::new(
                    SharedString::from(format!("gift-shop-plan-ribbon/{id}")),
                    format!("-{saving}%"),
                    LinearGradient::colors(
                        [color("#C164FF"), color("#6F7BFF")],
                        UnitPoint::TOP_LEADING,
                        UnitPoint::BOTTOM_TRAILING,
                    ),
                    white(),
                ))
            }))
            .when(self.is_selected, |cell| {
                cell.child(shape.round(div().absolute().inset_0()).stroke_border(theme.primary._100, 2.0))
            })
            .when(self.is_pending, |cell| {
                cell.child(
                    zstack(Alignment::Center)
                        .absolute()
                        .inset_0()
                        .bg(black().opacity(0.28))
                        .child(progress_view().tint(white())),
                )
            })
    }
}

/// insets and shapes of a shelf cell, shared by the cells the hidden template that gives the
/// grid its row height and the first guess of how many rows fill the sheet
enum GiftShopCellMetrics {}

impl GiftShopCellMetrics {
    const CORNER_RADIUS: f32 = 20.0;
    const TOP_INSET: f32 = 14.0;
    const BOTTOM_INSET: f32 = 12.0;
    const PRICE_GAP: f32 = 10.0;
    const PLATE_HEIGHT: f32 = 44.0;
    const GRID_SPACING: f32 = 10.0;
    const GRID_INSET: f32 = 16.0;

    fn preview_inset(kind: RiseGiftItemKind) -> f32 {
        match kind {
            RiseGiftItemKind::Nameplate | RiseGiftItemKind::ProfileEffect => 8.0,
            RiseGiftItemKind::Item | RiseGiftItemKind::AvatarFrame => 6.0,
        }
    }

    /// rows that fill the viewport from the shelf top
    /// the price line is left out of a row height so the guess errs towards a row too many not
    /// an empty band
    fn initial_rows(kind: RiseGiftItemKind, viewport: Size<f32>) -> i64 {
        let Some(preview) = Self::preview_size(kind, viewport) else {
            return 0;
        };
        if viewport.height <= 0.0 {
            return 0;
        }
        let stride = preview.height + Self::TOP_INSET + Self::BOTTOM_INSET + Self::PRICE_GAP + Self::GRID_SPACING;
        (viewport.height / stride).ceil() as i64
    }

    /// preview box of a cell in a sheet of this viewport as the cell body lays it out
    fn preview_size(kind: RiseGiftItemKind, viewport: Size<f32>) -> Option<Size<f32>> {
        let columns = kind.shop_columns() as f32;
        let cell_width = (viewport.width - 2.0 * Self::GRID_INSET - Self::GRID_SPACING * (columns - 1.0)) / columns;
        let width = cell_width - 2.0 * Self::preview_inset(kind);
        if width <= 0.0 {
            return None;
        }
        let height = match kind {
            RiseGiftItemKind::Nameplate => Self::PLATE_HEIGHT,
            RiseGiftItemKind::ProfileEffect => width / GiftCosmeticGeometry::BANNER_ASPECT,
            RiseGiftItemKind::Item | RiseGiftItemKind::AvatarFrame => width,
        };
        Some(Size { width, height })
    }
}

/// laid out size of a cosmetic card of each kind as the hidden template last measured it
/// the template is laid out every shelf frame so its known before the cards it seeds
struct GiftShopCardSize;

thread_local! {
    static GIFT_SHOP_CARD_SIZES: RefCell<HashMap<RiseGiftItemKind, Size<f32>>> = RefCell::new(HashMap::new());
}

impl GiftShopCardSize {
    fn remember(kind: RiseGiftItemKind, size: Size<Pixels>) {
        let size = Size { width: f32::from(size.width), height: f32::from(size.height) };
        GIFT_SHOP_CARD_SIZES.with_borrow_mut(|sizes| {
            sizes.insert(kind, size);
        });
    }

    /// the remembered size while it is still the size of a card around preview
    fn seed(kind: RiseGiftItemKind, preview: Option<Size<f32>>) -> Option<Size<f32>> {
        let width = preview?.width + 2.0 * GiftShopCellMetrics::preview_inset(kind);
        let size = GIFT_SHOP_CARD_SIZES.with_borrow(|sizes| sizes.get(&kind).copied())?;
        ((size.width - width).abs() <= 1.0 && size.height > 0.0).then_some(size)
    }
}

/// what every shelf cell is around its preview, the preview in its kind shape and the price
/// under it
/// given an empty preview it is the grid template as tall as each cell
#[derive(IntoElement)]
struct GiftShopCellBody {
    kind: RiseGiftItemKind,
    price: i64,
    price_on_backdrop: bool,
    is_placeholder: bool,
    preview: AnyElement,
}

impl GiftShopCellBody {
    fn new(kind: RiseGiftItemKind, price: i64, preview: impl IntoElement) -> Self {
        Self { kind, price, price_on_backdrop: false, is_placeholder: false, preview: preview.into_any_element() }
    }

    fn price_on_backdrop(mut self, price_on_backdrop: bool) -> Self {
        self.price_on_backdrop = price_on_backdrop;
        self
    }

    fn is_placeholder(mut self, is_placeholder: bool) -> Self {
        self.is_placeholder = is_placeholder;
        self
    }
}

impl RenderOnce for GiftShopCellBody {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let inset = GiftShopCellMetrics::preview_inset(self.kind);
        let shaped = match self.kind {
            RiseGiftItemKind::Nameplate => div().w_full().h(px(GiftShopCellMetrics::PLATE_HEIGHT)),
            RiseGiftItemKind::ProfileEffect => div().w_full().aspect_ratio(GiftCosmeticGeometry::BANNER_ASPECT),
            RiseGiftItemKind::Item | RiseGiftItemKind::AvatarFrame => div().w_full().aspect_ratio(1.0),
        };
        vstack(GiftShopCellMetrics::PRICE_GAP)
            .w_full()
            .pt(px(GiftShopCellMetrics::TOP_INSET))
            .pb(px(GiftShopCellMetrics::BOTTOM_INSET))
            .child(div().w_full().px(px(inset)).child(shaped.relative().child(self.preview)))
            .child(GiftPricePill {
                amount: self.price,
                on_backdrop: self.price_on_backdrop,
                is_placeholder: self.is_placeholder,
            })
    }
}

/// two rows of shelf cards in the shape and size of the real ones while the catalogue loads
#[derive(IntoElement)]
struct GiftShopShelfSkeleton {
    kind: RiseGiftItemKind,
}

impl RenderOnce for GiftShopShelfSkeleton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = current_theme(cx);
        let shape = Shape::continuous(GiftShopCellMetrics::CORNER_RADIUS);
        let kind = self.kind;
        let columns = kind.shop_columns();
        let grid = div()
            .grid()
            .grid_cols(columns as u16)
            .gap(px(GiftShopCellMetrics::GRID_SPACING))
            .w_full()
            .children((0..columns * 2).map(|_| {
                shape.round(div().w_full().min_w(px(0.0)).bg(theme.bg._200)).child(
                    GiftShopCellBody::new(kind, 0, div().absolute().inset_0().child(SkeletonBlock::new(None::<f32>, None::<f32>, 14.0)))
                        .is_placeholder(true),
                )
            }));
        div().w_full().px(px(GiftShopCellMetrics::GRID_INSET)).child(grid.skeleton_shimmer(None))
    }
}

#[derive(IntoElement)]
struct GiftShopCell {
    gift: RiseGiftCatalogItemDTO,
}

impl RenderOnce for GiftShopCell {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = current_theme(cx);
        let shape = Shape::continuous(GiftShopCellMetrics::CORNER_RADIUS);
        let gift = self.gift;
        let media = gift.preview_model.as_ref().and_then(|model| model.media.clone());
        let opacity = if gift.is_sold_out() { 0.45 } else { 1.0 };
        shape
            .round(div().relative().w_full().bg(theme.bg._200))
            .child(GiftShopCellBody::new(
                RiseGiftItemKind::Item,
                gift.price_sparks,
                div().absolute().inset_0().opacity(opacity).child(
                    GiftAnimationView::new(SharedString::from(format!("gift-shop-animation/{}", gift.id)), media).large(false),
                ),
            ))
            .child(div().absolute().inset_0().clip_shape(shape).child(GiftShopRibbon { gift }))
    }
}

#[derive(IntoElement)]
struct GiftShopCosmeticCell {
    gift: RiseGiftCatalogItemDTO,
    kind: RiseGiftItemKind,
    recipient: Option<GiftRecipient>,
    preview_size: Option<Size<f32>>,
}

impl RenderOnce for GiftShopCosmeticCell {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let shape = Shape::continuous(GiftShopCellMetrics::CORNER_RADIUS);
        let gift = self.gift;
        let on_backdrop =
            GiftBackdropColors::resolve(gift.preview_backdrop.as_ref(), RiseGiftRarityClass::Common).is_rolled;
        let opacity = if gift.is_sold_out() { 0.45 } else { 1.0 };
        let props = GiftShopCosmeticLayerProps {
            gift: gift.clone(),
            kind: self.kind,
            recipient: self.recipient,
            preview_size: self.preview_size,
        };
        let backdrop = GiftShopCosmeticLayerView::layer(GiftShopCosmeticLayer::Backdrop, props.clone(), window, cx);
        let preview = GiftShopCosmeticLayerView::layer(GiftShopCosmeticLayer::Preview, props, window, cx);
        div()
            .relative()
            .w_full()
            .child(div().absolute().inset_0().clip_shape(shape).child(backdrop))
            .child(
                GiftShopCellBody::new(
                    self.kind,
                    gift.price_sparks,
                    div().absolute().inset_0().overflow_hidden().opacity(opacity).child(preview),
                )
                .price_on_backdrop(on_backdrop),
            )
            .child(div().absolute().inset_0().clip_shape(shape).child(GiftShopRibbon { gift }))
    }
}

/// one cosmetic card with its button as its own cached view like the wardrobe cells
/// wheel steps rerendered every card on every scroll frame
/// now a card the scroll only moves replays its last frame, no height of its own
struct GiftShopCosmeticCellView {
    props: Option<GiftShopCosmeticLayerProps>,
    _theme: Subscription,
}

impl GiftShopCosmeticCellView {
    fn new(cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        Self { props: None, _theme: cx.observe(&theme_store, |_, _, cx| cx.notify()) }
    }

    fn set_props(&mut self, props: GiftShopCosmeticLayerProps) -> bool {
        if self.props.as_ref() == Some(&props) {
            return false;
        }
        self.props = Some(props);
        true
    }

    fn cell(props: GiftShopCosmeticLayerProps, window: &mut Window, cx: &mut App) -> AnyElement {
        let id = SharedString::from(format!("gift-shop-cosmetic-cell-view/{}/{}", props.kind.slug(), props.gift.id));
        let view = window.use_keyed_state(id, cx, |_, cx: &mut Context<GiftShopCosmeticCellView>| {
            GiftShopCosmeticCellView::new(cx)
        });
        let stale = view.update(cx, |view, _| view.set_props(props));
        if stale {
            AnyView::from(view).into_any_element()
        } else {
            view.cached(StyleRefinement::default().absolute().top_0().left_0().right_0().bottom_0())
                .replay_when_moved()
                .into_any_element()
        }
    }
}

impl Render for GiftShopCosmeticCellView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let card = self.props.clone().map(|props| {
            let selected = props.gift.clone();
            plain(SharedString::from(format!("gift-shop-cell/{}", props.gift.id)))
                .w_full()
                .disabled(props.gift.is_sold_out())
                .on_press(move |_, cx| select_gift(&selected, cx))
                .child(GiftShopCosmeticCell {
                    gift: props.gift,
                    kind: props.kind,
                    recipient: props.recipient,
                    preview_size: props.preview_size,
                })
        });
        div().absolute().top_0().left_0().right_0().bottom_0().children(card)
    }
}

/// the two costly layers of a cosmetic card, each its own cached view like the wardrobe cells
/// wheel steps re-render the host and every card rebuilt its backdrop and preview each time
#[derive(Clone, Copy, PartialEq, Eq)]
enum GiftShopCosmeticLayer {
    Backdrop,
    Preview,
}

impl GiftShopCosmeticLayer {
    fn slug(self) -> &'static str {
        match self {
            GiftShopCosmeticLayer::Backdrop => "backdrop",
            GiftShopCosmeticLayer::Preview => "preview",
        }
    }
}

#[derive(Clone, PartialEq)]
struct GiftShopCosmeticLayerProps {
    gift: RiseGiftCatalogItemDTO,
    kind: RiseGiftItemKind,
    recipient: Option<GiftRecipient>,
    preview_size: Option<Size<f32>>,
}

struct GiftShopCosmeticLayerView {
    layer: GiftShopCosmeticLayer,
    props: Option<GiftShopCosmeticLayerProps>,
    _theme: Subscription,
}

impl GiftShopCosmeticLayerView {
    fn new(layer: GiftShopCosmeticLayer, cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        Self { layer, props: None, _theme: cx.observe(&theme_store, |_, _, cx| cx.notify()) }
    }

    fn set_props(&mut self, props: GiftShopCosmeticLayerProps) -> bool {
        if self.props.as_ref() == Some(&props) {
            return false;
        }
        self.props = Some(props);
        true
    }

    fn layer(
        layer: GiftShopCosmeticLayer,
        props: GiftShopCosmeticLayerProps,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let id = SharedString::from(format!(
            "gift-shop-cosmetic-{}-view/{}/{}",
            layer.slug(),
            props.kind.slug(),
            props.gift.id
        ));
        let view = window.use_keyed_state(id, cx, |_, cx: &mut Context<GiftShopCosmeticLayerView>| {
            GiftShopCosmeticLayerView::new(layer, cx)
        });
        let stale = view.update(cx, |view, _| view.set_props(props));
        if stale {
            AnyView::from(view).into_any_element()
        } else {
            view.cached(StyleRefinement::default().size_full()).replay_when_moved().into_any_element()
        }
    }
}

impl Render for GiftShopCosmeticLayerView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let Some(props) = self.props.as_ref() else {
            return div().size_full();
        };
        let gift = &props.gift;
        let content = match self.layer {
            GiftShopCosmeticLayer::Backdrop => GiftBackdropSurface::new(
                SharedString::from(format!("gift-shop-backdrop/{}", gift.id)),
                gift.preview_backdrop.clone(),
            )
            .pattern(gift.preview_pattern.clone())
            .pattern_strength(GiftPatternLayer::COSMETIC_STRENGTH)
            .size_hint(GiftShopCardSize::seed(props.kind, props.preview_size))
            .into_any_element(),
            GiftShopCosmeticLayer::Preview => GiftCosmeticPreview::new(
                SharedString::from(format!("gift-shop-preview/{}", gift.id)),
                gift.kind(),
                props.recipient.clone(),
                gift.preview_model.as_ref().and_then(|model| model.media.clone()),
            )
            .size_hint(props.preview_size)
            .into_any_element(),
        };
        div().size_full().child(content)
    }
}

/// left of the edition while it lasts then sold out, nothing for an unnumbered gift
#[derive(IntoElement)]
struct GiftShopRibbon {
    gift: RiseGiftCatalogItemDTO,
}

impl RenderOnce for GiftShopRibbon {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let gift = self.gift;
        let id = SharedString::from(format!("gift-shop-ribbon/{}", gift.id));
        if gift.is_sold_out() {
            GiftCornerRibbon::new(id, tr("gift_sold_out"), color("#E5484D"), white()).into_any_element()
        } else if gift.is_limited() {
            GiftCornerRibbon::new(
                id,
                format_printf(
                    &tr("gift_serial_of"),
                    &[
                        FormatArg::Text(GiftSerialFormat::ribbon_count(gift.supply_left).into()),
                        FormatArg::Text(GiftSerialFormat::ribbon_count(gift.supply_total).into()),
                    ],
                ),
                GiftRarityStyle::tint(RiseGiftRarityClass::Rare),
                white(),
            )
            .into_any_element()
        } else {
            Empty.into_any_element()
        }
    }
}

#[derive(IntoElement)]
struct GiftPricePill {
    amount: i64,
    on_backdrop: bool,
    is_placeholder: bool,
}

impl RenderOnce for GiftPricePill {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = current_theme(cx);
        let color_of_price: Hsla = if self.on_backdrop { white() } else { theme.text.primary };
        let label = SparksAmountLabel::new(self.amount)
            .px(14.0)
            .icon_size(16.0)
            .weight(FontWeight::BOLD)
            .color(color_of_price);
        div()
            .px(px(12.0))
            .py(px(6.0))
            .capsule()
            .bg(color("#F5B82E").opacity(0.16))
            .child(redacted(label, self.is_placeholder, &theme))
    }
}

/// caption focus state and the caption place for scroll to
#[derive(Default)]
struct GiftSendFormState {
    caption_focused: bool,
    caption_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

#[derive(IntoElement)]
struct GiftSendForm {
    gift: RiseGiftCatalogItemDTO,
    recipient: Option<GiftRecipient>,
    reader: Entity<GiftSheetScrollReader>,
}

impl GiftSendForm {
    fn new(gift: RiseGiftCatalogItemDTO, recipient: Option<GiftRecipient>, reader: Entity<GiftSheetScrollReader>) -> Self {
        Self { gift, recipient, reader }
    }

    fn expected_odds_kinds(gift: &RiseGiftCatalogItemDTO) -> Vec<RiseGiftAttributeKind> {
        let mut kinds = vec![RiseGiftAttributeKind::Model];
        if gift.preview_backdrop.is_some() {
            kinds.push(RiseGiftAttributeKind::Backdrop);
        }
        if gift.preview_pattern.is_some() {
            kinds.push(RiseGiftAttributeKind::Pattern);
        }
        kinds
    }

    fn caption_binding(store: Entity<GiftsInteractionsStore>) -> Binding<String> {
        let reader = store.clone();
        Binding::new(
            move |cx| reader.read(cx).caption.clone(),
            move |raw: String, window, cx| {
                let kept = character_prefix(&raw, RiseGiftsLimits::caption_max_chars()).to_owned();
                let is_composing = kept != raw
                    && RiseFocusedTextInput::clamp(&raw, &kept, window, cx) == RiseFocusedTextInputOutcome::Composing;
                let value = if is_composing { raw } else { kept };
                store.update(cx, |store, cx| {
                    store.caption = value;
                    cx.notify();
                });
            },
        )
    }

    fn reveal_caption(
        state: &Entity<GiftSendFormState>,
        reader: &Entity<GiftSheetScrollReader>,
        animated: bool,
        cx: &mut App,
    ) {
        if !state.read(cx).caption_focused {
            return;
        }
        let Some(bounds) = state.read(cx).caption_bounds.get() else {
            return;
        };
        reader.update(cx, |reader, cx| reader.reveal(bounds, animated, 0.2, cx));
    }

    fn caption_field(
        interactions: &Entity<GiftsInteractionsStore>,
        state: &Entity<GiftSendFormState>,
        reader: &Entity<GiftSheetScrollReader>,
        gift: &RiseGiftCatalogItemDTO,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let caption = interactions.read(cx).caption.clone();
        let (typed_state, typed_reader) = (state.clone(), reader.clone());
        on_change("gift-send-caption-text", caption, false, window, cx, move |_, _, window, cx| {
            window.defer(cx, move |_, cx| Self::reveal_caption(&typed_state, &typed_reader, false, cx));
        });

        let (focus_state, focus_reader) = (state.clone(), reader.clone());
        let (measure_state, measure_reader) = (state.clone(), reader.clone());
        let caption_bounds = state.read(cx).caption_bounds.clone();
        let sender = interactions.clone();
        let can_send = !gift.is_sold_out();
        div()
            .relative()
            .flex()
            .w_full()
            .p(px(12.0))
            .rounded(px(14.0))
            .bg(theme.bg._200)
            .capture_action(move |_: &Confirm, _, cx| {
                cx.stop_propagation();
                if can_send {
                    sender.update(cx, |store, cx| store.send(cx));
                }
            })
            .child(
                ShapedTextField::new(
                    "gift-send-caption",
                    tr("gift_caption_placeholder"),
                    Self::caption_binding(interactions.clone()),
                )
                .axis(Axis::Vertical)
                .max_visible_lines(4)
                .font_size(15.0)
                .on_cancel(|window, cx| ScreenFocus::release_text_field(window, cx))
                .on_focus_change(move |focused, _, cx| {
                    focus_state.update(cx, |state, _| state.caption_focused = focused);
                    if focused {
                        Self::reveal_caption(&focus_state, &focus_reader, true, cx);
                    } else {
                        focus_reader.update(cx, |reader, _| reader.cancel());
                    }
                }),
            )
            .child(measure("gift-send-caption-frame", move |bounds, _, cx| {
                let grew = caption_bounds.get().is_some_and(|last| last.size.height != bounds.size.height);
                caption_bounds.set(Some(bounds));
                if grew {
                    Self::reveal_caption(&measure_state, &measure_reader, false, cx);
                }
            }))
    }
}

impl RenderOnce for GiftSendForm {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = current_theme(cx);
        let gift = self.gift;
        let recipient = self.recipient;
        let interactions = GiftsInteractionsStore::shared(cx);
        let services = GiftsServicesStore::shared(cx);
        let details = services.read(cx).catalog_details(cx).get(&gift.id).cloned();
        let details_failed = services.read(cx).catalog_details_failed(cx).contains(&gift.id);
        let for_self = is_shop_for_self(cx);
        let state = window.use_keyed_state(
            SharedString::from(format!("gift-send-form/{}", gift.id)),
            cx,
            |_, _| GiftSendFormState::default(),
        );
        let ribbon = gift.is_limited().then(|| {
            format_printf(
                &tr("gift_left"),
                &[
                    FormatArg::Text(GiftSerialFormat::ribbon_count(gift.supply_left).into()),
                    FormatArg::Text(GiftSerialFormat::ribbon_count(gift.supply_total).into()),
                ],
            )
        });
        let media = gift.preview_model.as_ref().and_then(|model| model.media.clone());

        let artwork = if gift.kind().equippable() {
            let backdrop = gift
                .preview_backdrop
                .clone()
                .or_else(|| details.as_ref().and_then(|details| details.likeliest(RiseGiftAttributeKind::Backdrop).cloned()));
            let pattern = gift
                .preview_pattern
                .clone()
                .or_else(|| details.as_ref().and_then(|details| details.likeliest(RiseGiftAttributeKind::Pattern).cloned()));
            div().w_full().pt(px(8.0)).child(
                GiftCosmeticStage::new("gift-send-stage", gift.kind(), recipient.clone(), media)
                    .backdrop(backdrop)
                    .pattern(pattern)
                    .ribbon(ribbon)
                    .animated(true),
            )
        } else {
            div().w_full().pt(px(8.0)).child(
                div().w_full().h(px(240.0)).child(GiftCardView::new("gift-send-card", media).ribbon(ribbon)),
            )
        };

        let mut form = vstack(16.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .px(px(16.0))
            .text_color(theme.text.primary)
            .child(artwork)
            .child(
                vstack(6.0)
                    .align(HorizontalAlignment::Leading)
                    .child(MainText::new(gift.name.clone()).px(22.0).font_weight(FontWeight::BOLD))
                    .when(!gift.description.is_empty(), |stack| {
                        stack.child(SecondaryText::new(gift.description.clone()).px(14.0))
                    }),
            );

        match details.as_ref().filter(|details| !details.attributes.is_empty()) {
            Some(details) => {
                form = form.child(GiftOddsSummary::new(gift.id, details.attributes.clone()));
            }
            None if details.is_none() && !details_failed => {
                form = form.child(GiftOddsSummaryPlaceholder::new(Self::expected_odds_kinds(&gift)));
            }
            None => {}
        }

        if !for_self {
            let caption = Self::caption_field(&interactions, &state, &self.reader, &gift, &theme, window, cx);
            form = form.child(caption).child(
                hstack(12.0)
                    .w_full()
                    .align(VerticalAlignment::Top)
                    .child(
                        vstack(2.0)
                            .align(HorizontalAlignment::Leading)
                            .flex_1()
                            .min_w(px(0.0))
                            .child(MainText::new(tr("gift_anonymous")).px(15.0).font_weight(FontWeight::MEDIUM))
                            .children(recipient.as_ref().map(|recipient| {
                                SecondaryText::new(format_printf(
                                    &tr("gift_anonymous_hint"),
                                    &[FormatArg::Text(recipient.name.as_str().into())],
                                ))
                                .px(12.0)
                            })),
                    )
                    .child(SwitchUi::new(
                        "gift-send-anonymous",
                        Binding::entity(interactions, |store| store.anonymous, |store, value, _| store.anonymous = value),
                    )),
            );
        }
        form
    }
}

#[derive(IntoElement)]
pub struct GiftOddsList {
    pub attributes: Vec<RiseGiftAttributeDTO>,
}

impl GiftOddsList {
    pub fn new(attributes: Vec<RiseGiftAttributeDTO>) -> Self {
        Self { attributes }
    }
}

impl RenderOnce for GiftOddsList {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = current_theme(cx);
        let groups = RiseGiftAttributeKind::ALL.into_iter().filter_map(|kind| {
            let items: Vec<&RiseGiftAttributeDTO> =
                self.attributes.iter().filter(|attribute| attribute.attribute_kind() == Some(kind)).collect();
            (!items.is_empty()).then(|| {
                vstack(6.0)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .child(SecondaryText::new(tr(&format!("gift_kind_{}", kind.slug()))).px(12.0))
                    .children(items.into_iter().map(|attribute| {
                        hstack(8.0)
                            .w_full()
                            .children(attribute.colors.get(gift_colors::CENTER).map(|center| {
                                div().flex_none().size(px(12.0)).rounded_full().bg(color(center))
                            }))
                            .child(MainText::new(attribute.display_name()).px(14.0).number_of_lines(1))
                            .child(spacer())
                            .child(
                                MainText::new(GiftSerialFormat::percent_share(attribute.share()))
                                    .px(14.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.text.secondary)
                                    .number_of_lines(1)
                                    .monospaced_digits(true),
                            )
                    }))
            })
        });
        vstack(10.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .p(px(12.0))
            .rounded(px(14.0))
            .bg(theme.bg._200)
            .child(MainText::new(tr("gift_odds_title")).px(15.0).font_weight(FontWeight::SEMIBOLD))
            .children(groups.collect::<Vec<_>>())
            .child(SecondaryText::new(tr("gift_odds_note")).px(12.0))
    }
}

#[derive(IntoElement)]
struct GiftSentView {
    instance: RiseGiftInstanceDTO,
    recipient: Option<GiftRecipient>,
}

impl GiftSentView {
    fn new(instance: RiseGiftInstanceDTO, recipient: Option<GiftRecipient>) -> Self {
        Self { instance, recipient }
    }
}

impl RenderOnce for GiftSentView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let instance = self.instance;
        let recipient = self.recipient;
        let for_self = recipient.as_ref().is_some_and(|recipient| is_current_user(&recipient.id, cx));
        let ribbon = instance
            .is_numbered()
            .then(|| GiftSerialFormat::ribbon(instance.serial, Some(instance.supply_total)));
        let artwork = if instance.is_cosmetic() {
            div()
                .relative()
                .w_full()
                .mt(px(12.0))
                .child(
                    GiftCosmeticStage::new(
                        "gift-sent-stage",
                        instance.kind(),
                        recipient.clone(),
                        instance.model().and_then(|model| model.media.clone()),
                    )
                    .effect(instance.effect().and_then(|effect| effect.media.clone()))
                    .backdrop(instance.backdrop().cloned())
                    .pattern(instance.pattern().cloned())
                    .rarity(instance.rarity_class())
                    .ribbon(ribbon)
                    .animated(true),
                )
                .when(instance.is_equipped(), |stage| {
                    stage.child(div().absolute().top(px(12.0)).left(px(12.0)).child(GiftEquippedBadge::new()))
                })
        } else {
            div()
                .mt(px(12.0))
                .w(px(240.0))
                .h(px(240.0))
                .child(GiftCardView::instance("gift-sent-card", &instance, None, false).ribbon(ribbon))
        };
        vstack(14.0)
            .w_full()
            .px(px(16.0))
            .child(artwork)
            .child(
                MainText::new(tr(if for_self { "gift_market_bought_title" } else { "gift_sent_title" }))
                    .px(22.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD),
            )
            .children(recipient.as_ref().filter(|_| !for_self).map(|recipient| {
                SecondaryText::new(format_printf(
                    &tr("gift_sent_subtitle"),
                    &[FormatArg::Text(recipient.name.as_str().into())],
                ))
                .px(14.0)
                .tac(TextAlignT::Center)
            }))
            .children(
                instance.kind().bought_offer_key().filter(|_| for_self && !instance.is_equipped()).map(|key| {
                    SecondaryText::new(format_printf(&tr(key), &[FormatArg::Text(instance.name.as_str().into())]))
                        .px(15.0)
                        .tac(TextAlignT::Center)
                }),
            )
            .child(GiftRarityPill::new(instance.rarity_class()).chance_ppm(Some(instance.chance_ppm)))
    }
}

/// the pay button under the form, under the result it is done or later and put on for a
/// cosmetic bought for oneself
#[derive(IntoElement)]
pub struct GiftShopSheetFooter;

impl GiftShopSheetFooter {
    pub const VERTICAL_PADDING: f32 = GIFT_SHOP_SHEET_FOOTER_VERTICAL_PADDING;

    pub fn new() -> Self {
        Self
    }

    fn close_action(store: Entity<GiftsInteractionsStore>) -> impl Fn(&mut Window, &mut App) + 'static {
        move |_, cx| store.update(cx, |store, cx| store.close_shop(cx))
    }

    fn label(text: String, tint: Hsla) -> MainText {
        MainText::new(text).px(16.0).font_weight(FontWeight::SEMIBOLD).color(tint)
    }

    pub fn window_lift() -> Option<f32> {
        GIFT_SHOP_FOOTER_WINDOW_LIFT.with(Cell::get)
    }

    fn pads(content: impl IntoElement) -> Div {
        div()
            .relative()
            .w_full()
            .px(px(16.0))
            .py(px(Self::VERTICAL_PADDING))
            .child(content)
            .child(measure("gift-shop-sheet-footer-lift", |bounds, window, _| {
                let lift = f32::from(window.viewport_size().height - bounds.top());
                GIFT_SHOP_FOOTER_WINDOW_LIFT.with(|cell| cell.set(Some(lift)));
            }))
    }
}

thread_local! {
    static GIFT_SHOP_FOOTER_WINDOW_LIFT: Cell<Option<f32>> = const { Cell::new(None) };
}

impl Default for GiftShopSheetFooter {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for GiftShopSheetFooter {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = GiftsInteractionsStore::shared(cx);
        let for_self = is_shop_for_self(cx);
        let interactions = store.read(cx);
        if let Some(sent) = interactions.sent_instance.clone() {
            if sent.is_cosmetic() && for_self && !sent.is_equipped() {
                let theme = current_theme(cx);
                let is_equipping = store.read(cx).is_equipping;
                let equipper = store.clone();
                return Self::pads(
                    hstack(10.0)
                        .w_full()
                        .child(
                            div().flex_1().min_w(px(0.0)).child(
                                ButtonUi::new("gift-equip-later")
                                    .background_color(theme.bg._200)
                                    .on_press(Self::close_action(store.clone()))
                                    .child(Self::label(tr("gift_equip_later"), theme.text.primary)),
                            ),
                        )
                        .child(
                            div().flex_1().min_w(px(0.0)).child(
                                ButtonUi::new("gift-equip-put-on")
                                    .is_pending(is_equipping)
                                    .on_press(move |_, cx| {
                                        let sent = sent.clone();
                                        equipper.update(cx, |store, cx| store.set_equipped(sent, true, None, cx));
                                    })
                                    .child(Self::label(tr("gift_equip_put_on"), white())),
                            ),
                        ),
                )
                .into_any_element();
            }
            return Self::pads(
                ButtonUi::new("gift-shop-done")
                    .on_press(Self::close_action(store.clone()))
                    .child(Self::label(tr("done"), white())),
            )
            .into_any_element();
        }
        let Some(gift) = interactions.selected_gift.clone() else {
            return Empty.into_any_element();
        };
        let is_sending = interactions.is_sending;
        let template = tr(if for_self { "gift_market_buy_button" } else { "gift_send_button" });
        let parts: Vec<&str> = template.split("{amount}").collect();
        let prefix = parts.first().map(|part| trimming_whitespaces(part)).filter(|part| !part.is_empty());
        let suffix = (parts.len() > 1)
            .then(|| parts.last().map(|part| trimming_whitespaces(part)))
            .flatten()
            .filter(|part| !part.is_empty());
        let sender = store.clone();
        Self::pads(
            ButtonUi::new("gift-shop-send")
                .disabled(gift.is_sold_out())
                .is_pending(is_sending)
                .gap(6.0)
                .on_press(move |_, cx| sender.update(cx, |store, cx| store.send(cx)))
                .child(
                    hstack(6.0)
                        .children(prefix.map(|prefix| Self::label(prefix.to_owned(), white())))
                        .child(SparksIconView::new().size(18.0))
                        .child(Self::label(RiseFormat::compact_count(gift.price_sparks), white()))
                        .children(suffix.map(|suffix| Self::label(suffix.to_owned(), white()))),
                ),
        )
        .into_any_element()
    }
}
