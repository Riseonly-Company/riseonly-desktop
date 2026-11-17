use gpui::{
    AnyElement, App, Bounds, Context, ElementId, Empty, Entity, FontWeight, Hsla, Pixels, Render, SharedString, Subscription, Window,
    anchored, black, deferred, point, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces;
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::modal_sheet_surface::ModalSheetMetrics;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::scroll_chrome_shadow_ui::{ScrollChromeShadowStyle, ScrollChromeShadowStyleAnchor, ScrollChromeShadowUi};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonCircle, SkeletonLine};
use rise_ui::stepped_slider_ui::SteppedSliderUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::bottom_sheet_ui::sheet_floating_header::{SheetFloatingHeader, SheetFloatingHeaderMetrics};
use crate::core::ui::purchase_celebration::{PurchaseCelebration, PurchaseCelebrationSheet};
use crate::core::ui::qr_code::rise_purchase_continue_sheet::RisePurchaseContinue;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::NotifierType;
use crate::modules::sparks::components::sparks_icon_view::{SparksAmountLabel, SparksBurstLayer, SparksHeroArt, SparksIconView};
use crate::modules::sparks::engine::rise_sparks_engine_models::{RiseSparksLoadPhase, RiseStoredSparksSender};
use crate::modules::sparks::engine::rise_sparks_purchases::RiseSparksPackOffer;
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_types::{SparksOffersPhase, SparksPurchasePhase};
use crate::modules::sparks::stores::sparks::sparks_interactions::sparks_interactions::SparksInteractionsStore;
use crate::modules::sparks::stores::sparks::sparks_services::sparks_services::SparksServicesStore;

fn theme_colors(cx: &mut App) -> (Hsla, Hsla) {
    let store = ThemeStore::shared(cx);
    let theme = store.read(cx).current_theme();
    (theme.primary._100, theme.text.secondary)
}

/// hosts the two sparks sheets once at the app root so a bubble or post or wallet
/// page opens them through the store
pub struct SparksSheetsHost {
    service: Entity<SparksServicesStore>,
    interactions: Entity<SparksInteractionsStore>,
    theme_store: Entity<ThemeStore>,
    _observations: [Subscription; 2],
}

impl SparksSheetsHost {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let service = SparksServicesStore::shared(cx);
        let interactions = SparksInteractionsStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let observations = [cx.observe(&service, |_, _, cx| cx.notify()), cx.observe(&theme_store, |_, _, cx| cx.notify())];
        Self { service, interactions, theme_store, _observations: observations }
    }
}

impl Render for SparksSheetsHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sheet_background = self.theme_store.read(cx).current_theme().bg._100;

        let reader = self.service.clone();
        let closer = self.interactions.clone();
        let purchase_sheet = BottomSheetUi::new(
            "sparks-purchase-sheet",
            Binding::new(
                move |cx| reader.read(cx).is_purchase_sheet_presented,
                move |presented, _, cx| {
                    if !presented {
                        closer.update(cx, |interactions, cx| interactions.close_purchase_sheet(cx));
                    }
                },
            ),
            |_, _| SparksPurchaseSheetContent::new(),
            |_, _| Empty,
        )
        .snap_fractions(vec![0.93])
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .sheet_background(Some(sheet_background));

        let reader = self.service.clone();
        let closer = self.interactions.clone();
        let send_sheet = BottomSheetUi::new(
            "sparks-send-sheet",
            Binding::new(
                move |cx| reader.read(cx).is_send_sheet_presented,
                move |presented, _, cx| {
                    if !presented {
                        closer.update(cx, |interactions, cx| interactions.close_send_sheet(cx));
                    }
                },
            ),
            |_, _| SparksSendSheetContent::new(),
            |_, _| SparksSendSheetFooter::new(),
        )
        .snap_fractions(vec![0.8])
        .fits_content(true)
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .sheet_background(Some(sheet_background))
        .footer_background(Some(sheet_background));

        let reader = self.service.clone();
        let closer = self.interactions.clone();
        let done = self.interactions.clone();
        let success_sheet = PurchaseCelebrationSheet::new(
            PurchaseCelebration::Sparks,
            Binding::new(
                move |cx| reader.read(cx).is_success_sheet_presented,
                move |presented, _, cx| {
                    if !presented {
                        closer.update(cx, |interactions, cx| interactions.close_success_sheet(cx));
                    }
                },
            ),
            tr("sparks_purchase_success_continue"),
            move |_, cx| done.update(cx, |interactions, cx| interactions.close_success_sheet(cx)),
            |_, _| SparksIconView::new().size(PurchaseCelebration::ICON_TIP_TO_TIP * 512.0 / 381.0),
            |_, _| SparksSuccessLines::new(),
        );

        div().absolute().inset_0().child(purchase_sheet).child(send_sheet).child(success_sheet).child(SparksBurstLayer::new())
    }
}

/// state of whether the title is under the header, from frames of the scroll and the large title
struct SparksSheetScaffoldState {
    scroll_top: f32,
    frame: Option<Bounds<Pixels>>,
    title_bottom: Option<f32>,
    shadow: Animated<f32>,
}

impl SparksSheetScaffoldState {
    fn title_under_header(&self) -> bool {
        self.title_bottom.is_some_and(|bottom| bottom - self.scroll_top < SheetFloatingHeaderMetrics::HEIGHT)
    }
}

/// marks the large title inside the scroll, header shows its own copy once this one is under it
#[derive(Clone)]
struct SparksSheetTitleAnchor {
    state: Entity<SparksSheetScaffoldState>,
}

fn sparks_sheet_title(title: impl IntoElement, anchor: &SparksSheetTitleAnchor) -> impl IntoElement {
    let state = anchor.state.clone();
    div().relative().child(title).child(measure("sparks-sheet-title-frame", move |bounds, _, cx| {
        let bottom = f32::from(bounds.bottom());
        state.update(cx, |state, cx| {
            if state.title_bottom != Some(bottom) {
                state.title_bottom = Some(bottom);
                cx.notify();
            }
        });
    }))
}

type SparksSheetScaffoldContent = Box<dyn FnOnce(SparksSheetTitleAnchor, &mut Window, &mut App) -> AnyElement>;

/// sheet header over scrolling content, close on left
/// title once the large one slid under, balance right if known
/// content_scrollable syncs sheet pan and scroll
#[derive(IntoElement)]
struct SparksSheetScaffold {
    id: ElementId,
    title: SharedString,
    balance: Option<i64>,
    on_close: Box<dyn Fn(&mut Window, &mut App)>,
    content: SparksSheetScaffoldContent,
    presented: bool,
}

impl SparksSheetScaffold {
    fn new<C: IntoElement>(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        balance: Option<i64>,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
        content: impl FnOnce(SparksSheetTitleAnchor, &mut Window, &mut App) -> C + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            balance,
            on_close: Box::new(on_close),
            content: Box::new(move |anchor: SparksSheetTitleAnchor, window: &mut Window, cx: &mut App| {
                content(anchor, window, cx).into_any_element()
            }),
            presented: true,
        }
    }

    /// false while the sheet leaves so the header shadow goes at once and not after the dialog fade
    fn presented(mut self, presented: bool) -> Self {
        self.presented = presented;
        self
    }
}

impl SparksSheetScaffold {
    const SHADOW_PRIORITY: usize = 0;
}

impl RenderOnce for SparksSheetScaffold {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state((self.id.clone(), "state"), cx, |_, _| SparksSheetScaffoldState {
            scroll_top: 0.0,
            frame: None,
            title_bottom: None,
            shadow: Animated::new(0.0),
        });
        let title_under_header = state.read(cx).title_under_header();
        let presented = self.presented;
        let now = RiseClock::monotonic();
        state.update(cx, |state, cx| {
            if presented {
                state.shadow.set_in(if title_under_header { 1.0 } else { 0.0 }, ease_out(0.18), now, cx);
            } else {
                state.shadow.jump(0.0);
            }
        });
        let shadow_opacity = state.read(cx).shadow.frame_value(now, window);

        let content = (self.content)(SparksSheetTitleAnchor { state: state.clone() }, window, cx);

        let on_close = self.on_close;
        let mut header = SheetFloatingHeader::new((self.id.clone(), "header"), self.title, move |window, cx| on_close(window, cx))
            .shows_title(title_under_header);
        if let Some(balance) = self.balance {
            header = header.trailing(SparksAmountLabel::new(balance).px(15.0).icon_size(16.0));
        }

        let shadow_extent = SheetFloatingHeaderMetrics::CHROME_EXTENT - SheetFloatingHeaderMetrics::HEIGHT;
        let frame = state.read(cx).frame;
        let shadow = frame.filter(|_| shadow_opacity > 0.0).map(|frame| {
            let inset = px(ModalSheetMetrics::CONTENT_INSET);
            deferred(
                anchored().position(point(frame.left() - inset, frame.top() + px(SheetFloatingHeaderMetrics::HEIGHT))).child(
                    pass_through()
                        .w(frame.size.width + inset * 2.0)
                        .h(px(shadow_extent))
                        .opacity(shadow_opacity)
                        .child(ScrollChromeShadowUi::new(ScrollChromeShadowStyle {
                            anchor: ScrollChromeShadowStyleAnchor::FromTop,
                            extent: shadow_extent,
                            overflow_below_header: 0.0,
                            peak_opacity: 0.16,
                            color: black(),
                            top_plateau_end: 0.02,
                            top_plateau_opacity_factor: 0.75,
                            mid_location: 0.35,
                            mid_opacity_factor: 0.3,
                            ..ScrollChromeShadowStyle::default()
                        })),
                ),
            )
            .with_priority(Self::SHADOW_PRIORITY)
        });

        let frame_state = state;
        div()
            .relative()
            .size_full()
            .child(
                scroll_view((self.id, "scroll"))
                    .shows_indicators(false)
                    .child(div().w_full().pt(px(SheetFloatingHeaderMetrics::HEIGHT)).child(content)),
            )
            .child(div().absolute().top_0().left_0().right_0().child(header).children(shadow))
            .child(measure("sparks-sheet-scroll-frame", move |bounds, _, cx| {
                let top = f32::from(bounds.top());
                frame_state.update(cx, |state, cx| {
                    if state.scroll_top != top || state.frame != Some(bounds) {
                        state.scroll_top = top;
                        state.frame = Some(bounds);
                        cx.notify();
                    }
                });
            }))
    }
}

struct SparksPurchaseSheetFades {
    status: Animated<f32>,
    others_dim: Animated<f32>,
}

#[derive(IntoElement)]
pub struct SparksPurchaseSheetContent;

impl SparksPurchaseSheetContent {
    const ROW_HEIGHT: f32 = 56.0;

    pub fn new() -> Self {
        Self
    }

    fn header(primary: Hsla, needed: Option<i64>, title_anchor: &SparksSheetTitleAnchor) -> Vec<AnyElement> {
        let mut views = vec![
            SparksHeroArt::new().size(112.0).into_any_element(),
            sparks_sheet_title(
                MainText::new(tr("sparks_buy_button")).px(22.0).tac(TextAlignT::Center).font_weight(FontWeight::BOLD),
                title_anchor,
            )
            .into_any_element(),
            SecondaryText::new(tr("sparks_purchase_subtitle"))
                .px(14.0)
                .tac(TextAlignT::Center)
                .w_full()
                .pl(px(28.0))
                .pr(px(28.0))
                .into_any_element(),
        ];
        if let Some(needed) = needed {
            views.push(
                MainText::new(tr("sparks_purchase_needed").replace("{amount}", &RiseFormat::compact_count(needed)))
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(primary)
                    .into_any_element(),
            );
        }
        views
    }

    fn status(phase: &SparksPurchasePhase) -> Option<AnyElement> {
        match phase {
            SparksPurchasePhase::Pending => Some(
                SecondaryText::new(tr("sparks_purchase_pending"))
                    .px(13.0)
                    .tac(TextAlignT::Center)
                    .w_full()
                    .pl(px(24.0))
                    .pr(px(24.0))
                    .into_any_element(),
            ),
            SparksPurchasePhase::Failed(message) => Some(
                MainText::new(message.clone())
                    .px(13.0)
                    .tac(TextAlignT::Center)
                    .color(NotifierType::Error.accent_color())
                    .w_full()
                    .pl(px(24.0))
                    .pr(px(24.0))
                    .into_any_element(),
            ),
            SparksPurchasePhase::Idle | SparksPurchasePhase::Purchasing { .. } | SparksPurchasePhase::ContinueOnPhone { .. } => {
                None
            }
        }
    }

    /// what the phone handoff modal of a sparks purchase says
    /// the root purchase continue host takes it once when the handoff opens
    pub(crate) fn continue_on_phone_content(service: &Entity<SparksServicesStore>, cx: &App) -> RisePurchaseContinue {
        RisePurchaseContinue::purchase("sparks-purchase-on-phone", tr("sparks_purchase_on_phone"))
            .link(service.read(cx).phone_link())
    }

    fn packs(
        phase: &SparksOffersPhase,
        offers: &[RiseSparksPackOffer],
        purchasing: Option<&str>,
        others_dim: f32,
        primary: Hsla,
        interactions: &Entity<SparksInteractionsStore>,
    ) -> AnyElement {
        match phase {
            SparksOffersPhase::Idle | SparksOffersPhase::Loading => vstack(8.0)
                .w_full()
                .px(px(16.0))
                .children((0..6).map(|_| SkeletonBlock::new(None::<f32>, Self::ROW_HEIGHT, 14.0)))
                .into_any_element(),
            SparksOffersPhase::Failed(message) => {
                let retry = interactions.clone();
                vstack(10.0)
                    .w_full()
                    .px(px(24.0))
                    .child(SecondaryText::new(message.clone()).px(14.0).tac(TextAlignT::Center))
                    .child(
                        ButtonUi::new("sparks-offers-retry")
                            .fit_content(true)
                            .padding_horizontal(20.0)
                            .on_press(move |_, cx| retry.update(cx, |interactions, cx| interactions.load_offers(cx)))
                            .child(MainText::new(tr("common_retry")).px(15.0).font_weight(FontWeight::SEMIBOLD).color(white())),
                    )
                    .into_any_element()
            }
            SparksOffersPhase::Ready => vstack(8.0)
                .w_full()
                .px(px(16.0))
                .children(offers.iter().map(|offer| Self::pack_row(offer, purchasing, others_dim, primary, interactions)))
                .into_any_element(),
        }
    }

    fn pack_row(
        offer: &RiseSparksPackOffer,
        purchasing: Option<&str>,
        others_dim: f32,
        primary: Hsla,
        interactions: &Entity<SparksInteractionsStore>,
    ) -> impl IntoElement + use<> {
        let is_this_row = purchasing == Some(offer.product_id.as_str());
        let trailing = if is_this_row {
            progress_view().tint(primary).into_any_element()
        } else {
            MainText::new(offer.display_price.clone())
                .px(15.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(primary)
                .number_of_lines(1)
                .into_any_element()
        };
        let interactions = interactions.clone();
        let pressed = offer.clone();
        div().w_full().opacity(if purchasing.is_some() && !is_this_row { others_dim } else { 1.0 }).child(
            plain(ElementId::Name(SharedString::from(format!("sparks-pack/{}", offer.product_id))))
                .disabled(purchasing.is_some())
                .on_press(move |_, cx| interactions.update(cx, |interactions, cx| interactions.purchase(&pressed, cx)))
                .w_full()
                .child(
                    hstack(10.0)
                        .w_full()
                        .px(px(16.0))
                        .h(px(Self::ROW_HEIGHT))
                        .continuous_corners(14.0)
                        .bg(primary.opacity(0.08))
                        .child(SparksAmountLabel::new(offer.amount).px(17.0).icon_size(24.0))
                        .child(spacer_min_w(8.0))
                        .child(trailing),
                ),
        )
    }
}

impl Default for SparksPurchaseSheetContent {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for SparksPurchaseSheetContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (primary, _) = theme_colors(cx);
        let interactions = SparksInteractionsStore::shared(cx);
        let service = SparksServicesStore::shared(cx);
        let (presented, balance, needed, purchase_phase, offers_phase, offers) = {
            let service = service.read(cx);
            (
                service.is_purchase_sheet_presented,
                service.has_balance().then(|| service.balance().available),
                service.needed_amount,
                service.purchase_phase.clone(),
                service.offers_phase.clone(),
                service.offers.clone(),
            )
        };

        let purchasing = match &purchase_phase {
            SparksPurchasePhase::Purchasing { product_id } => Some(product_id.clone()),
            _ => None,
        };
        let fades = window.use_keyed_state("sparks-purchase-sheet-fades", cx, |_, _| SparksPurchaseSheetFades {
            status: Animated::new(1.0),
            others_dim: Animated::new(1.0),
        });
        let now = RiseClock::monotonic();
        let is_purchasing = purchasing.is_some();
        fades.update(cx, |fades, cx| {
            fades.others_dim.set_in(if is_purchasing { 0.5 } else { 1.0 }, ease_out(0.2), now, cx);
        });
        let restart = fades.clone();
        on_change("sparks-purchase-sheet-phase", purchase_phase.clone(), false, window, cx, move |_, _, _, cx| {
            restart.update(cx, |fades, cx| {
                fades.status.jump(0.0);
                fades.status.set_in(1.0, ease_out(0.2), RiseClock::monotonic(), cx);
                cx.notify();
            });
        });
        let (status_opacity, others_dim) = {
            let fades = fades.read(cx);
            (fades.status.frame_value(now, window), fades.others_dim.frame_value(now, window))
        };

        let closer = interactions.clone();
        SparksSheetScaffold::new(
            "sparks-purchase-scaffold",
            tr("sparks_buy_button"),
            balance,
            move |_, cx| closer.update(cx, |interactions, cx| interactions.close_purchase_sheet(cx)),
            move |title_anchor, _, _| {
                vstack(14.0)
                    .w_full()
                    .pb(px(28.0))
                    .children(Self::header(primary, needed, &title_anchor))
                    .children(Self::status(&purchase_phase).map(|status| div().w_full().opacity(status_opacity).child(status)))
                    .child(Self::packs(&offers_phase, &offers, purchasing.as_deref(), others_dim, primary, &interactions))
                    .child(SecondaryText::new(tr("sparks_purchase_terms")).px(12.0).tac(TextAlignT::Center).w_full().pl(px(24.0)).pr(px(24.0)).pt(px(4.0)))
                    .child(SecondaryText::new(tr("sparks_purchase_refund_note")).px(12.0).tac(TextAlignT::Center).w_full().pl(px(24.0)).pr(px(24.0)))
            },
        )
        .presented(presented)
    }
}

#[derive(IntoElement)]
struct SparksSuccessLines;

impl SparksSuccessLines {
    fn new() -> Self {
        Self
    }
}

impl RenderOnce for SparksSuccessLines {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let service = SparksServicesStore::shared(cx);
        let service = service.read(cx);
        vstack(0.0)
            .w_full()
            .child(MainText::new(tr("sparks_purchase_success")).px(22.0).tac(TextAlignT::Center).font_weight(FontWeight::BOLD).pl(px(24.0)).pr(px(24.0)))
            .child(div().pt(px(8.0)).child(SparksAmountLabel::new(service.success_amount).px(20.0).icon_size(22.0)))
            .children(service.has_balance().then(|| {
                SecondaryText::new(
                    tr("sparks_balance_line").replace("{amount}", &RiseFormat::compact_count(service.balance().available)),
                )
                .px(14.0)
                .tac(TextAlignT::Center)
                .pt(px(6.0))
                .pl(px(24.0))
                .pr(px(24.0))
            }))
    }
}

/// what the sender rows of the send sheet draw from
struct SparksTopSenders {
    phase: RiseSparksLoadPhase,
    sparks_hidden: bool,
    senders: Vec<RiseStoredSparksSender>,
    hidden_count: i64,
}

#[derive(IntoElement)]
pub struct SparksSendSheetContent;

/// what a sender row needs to open the sender, sheet closes then the profile is pushed
#[derive(Clone)]
struct SparksSenderOpener {
    nav_store: Option<Entity<NavigationStore>>,
    interactions: Entity<SparksInteractionsStore>,
}

impl SparksSendSheetContent {
    pub fn new() -> Self {
        Self
    }

    fn send_controls(
        amount: i64,
        maximum: i64,
        anonymous: bool,
        title_anchor: &SparksSheetTitleAnchor,
        interactions: &Entity<SparksInteractionsStore>,
        top_senders: AnyElement,
    ) -> Vec<AnyElement> {
        let mut views = vec![
            div()
                .pt(px(4.0))
                .child(sparks_sheet_title(
                    MainText::new(tr("sparks_send_title")).px(20.0).tac(TextAlignT::Center).font_weight(FontWeight::BOLD),
                    title_anchor,
                ))
                .into_any_element(),
            SparksAmountLabel::new(amount).px(34.0).icon_size(32.0).weight(FontWeight::BOLD).into_any_element(),
        ];
        if maximum > 1 {
            let (changed, committed) = (interactions.clone(), interactions.clone());
            views.push(
                div()
                    .w_full()
                    .px(px(24.0))
                    .child(
                        SteppedSliderUi::new(
                            "sparks-send-amount",
                            amount.min(maximum) as f64,
                            1.0..=maximum as f64,
                            1.0,
                            move |value, _, cx| {
                                changed.update(cx, |interactions, cx| interactions.set_send_amount(value.round() as i64, cx));
                            },
                            move |value, _, cx| {
                                committed.update(cx, |interactions, cx| interactions.set_send_amount(value.round() as i64, cx));
                            },
                        )
                        .shows_step_ticks(false),
                    )
                    .into_any_element(),
            );
        } else if maximum < 1 {
            let buyer = interactions.clone();
            views.push(
                ButtonUi::new("sparks-send-buy")
                    .fit_content(true)
                    .padding_horizontal(20.0)
                    .on_press(move |_, cx| {
                        buyer.update(cx, |interactions, cx| {
                            interactions.close_send_sheet(cx);
                            interactions.open_purchase_sheet(Some(amount), cx);
                        });
                    })
                    .child(MainText::new(tr("sparks_buy_button")).px(15.0).font_weight(FontWeight::SEMIBOLD).color(white()))
                    .into_any_element(),
            );
        }
        views.push(Self::anonymous_row(anonymous, interactions).into_any_element());
        views.push(top_senders);
        views
    }

    fn anonymous_row(anonymous: bool, interactions: &Entity<SparksInteractionsStore>) -> impl IntoElement + use<> {
        let writer = interactions.clone();
        hstack(12.0)
            .w_full()
            .px(px(20.0))
            .child(
                vstack(2.0)
                    .align(HorizontalAlignment::Leading)
                    .child(MainText::new(tr("sparks_send_anonymous")).px(15.0).font_weight(FontWeight::MEDIUM))
                    .child(SecondaryText::new(tr("sparks_send_anonymous_hint")).px(12.0)),
            )
            .child(spacer_min_w(8.0))
            .child(SwitchUi::new(
                "sparks-send-anonymous",
                Binding::new(
                    move |_| anonymous,
                    move |value, _, cx| writer.update(cx, |interactions, cx| interactions.set_send_anonymous(value, cx)),
                ),
            ))
    }

    /// title_anchor is set when the list is the whole sheet, its heading is then the sheet title
    fn top_senders(
        top: SparksTopSenders,
        primary: Hsla,
        secondary: Hsla,
        title_anchor: Option<&SparksSheetTitleAnchor>,
        opener: &SparksSenderOpener,
    ) -> AnyElement {
        let title = MainText::new(tr("sparks_top_senders_title")).px(15.0).font_weight(FontWeight::SEMIBOLD);
        let mut list = vstack(10.0).align(HorizontalAlignment::Leading).w_full().px(px(20.0)).child(match title_anchor {
            Some(title_anchor) => sparks_sheet_title(title, title_anchor).into_any_element(),
            None => title.into_any_element(),
        });
        match top.phase {
            RiseSparksLoadPhase::Idle | RiseSparksLoadPhase::Loading => {
                list = list.children((0..3).map(|_| {
                    hstack(10.0)
                        .w_full()
                        .h(px(44.0))
                        .child(SkeletonCircle::new(36.0))
                        .child(SkeletonLine::new(140.0, SkeletonLine::default().diameter))
                        .child(spacer())
                        .child(SkeletonLine::new(48.0, SkeletonLine::default().diameter))
                }));
            }
            RiseSparksLoadPhase::Failed(message) => {
                list = list.child(SecondaryText::new(message).px(13.0));
            }
            RiseSparksLoadPhase::Loaded => {
                if top.sparks_hidden {
                    list = list.child(SecondaryText::new(tr("sparks_top_hidden")).px(13.0));
                } else if top.senders.is_empty() {
                    list = list.child(SecondaryText::new(tr("sparks_top_empty")).px(13.0));
                } else {
                    list = list.children(
                        top.senders.iter().enumerate().map(|(index, sender)| Self::sender_row(index, sender, primary, secondary, opener)),
                    );
                    if top.hidden_count > 0 {
                        list = list.child(
                            div().flex().items_center().justify_center().w_full().h(px(36.0)).child(
                                SecondaryText::new(
                                    tr("sparks_top_senders_more").replace("{n}", &RiseFormat::compact_count(top.hidden_count)),
                                )
                                .px(13.0)
                                .tac(TextAlignT::Center),
                            ),
                        );
                    }
                }
            }
        }
        list.into_any_element()
    }

    fn sender_row(
        index: usize,
        sender: &RiseStoredSparksSender,
        primary: Hsla,
        secondary: Hsla,
        opener: &SparksSenderOpener,
    ) -> AnyElement {
        let content = Self::sender_row_content(index, sender, primary, secondary);
        let Some(user) = sender.user.as_ref() else {
            return content.into_any_element();
        };
        let route = RootRoute::UserPage { user_id: Some(user.id.clone()), tag: Some(user.tag.clone()) };
        let opener = opener.clone();
        plain(("sparks-top-sender-open", index))
            .w_full()
            .on_press(move |_, cx| {
                opener.interactions.update(cx, |interactions, cx| interactions.close_send_sheet(cx));
                if let Some(nav_store) = opener.nav_store.as_ref() {
                    let route = route.clone();
                    nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
                }
            })
            .child(content)
            .into_any_element()
    }

    fn sender_row_content(
        index: usize,
        sender: &RiseStoredSparksSender,
        primary: Hsla,
        secondary: Hsla,
    ) -> impl IntoElement + use<> {
        let leading = match &sender.user {
            Some(user) => UserLogoUi::new(("sparks-top-sender", index))
                .source(SharedString::from(user.logo.clone()))
                .name(SharedString::from(user.name.clone()))
                .user_id(SharedString::from(user.id.clone()))
                .size(36.0)
                .show_border(false)
                .show_story_ring(false)
                .open_story_on_press(false)
                .into_any_element(),
            None => div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .frame_size(36.0, 36.0)
                .rounded_full()
                .bg(secondary.opacity(0.18))
                .children(IconUi::sized("person.fill.questionmark", px(15.0), secondary, true))
                .into_any_element(),
        };
        let name = sender.user.as_ref().map_or_else(|| tr("sparks_anonymous_sender"), |user| user.name.clone());
        hstack(10.0)
            .w_full()
            .h(px(44.0))
            .child(leading)
            .child(
                MainText::new(name)
                    .px(15.0)
                    .font_weight(if sender.is_me { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                    .color(sender.is_me.then_some(primary))
                    .number_of_lines(1)
                    .min_w_0(),
            )
            .child(spacer_min_w(8.0))
            .child(SparksAmountLabel::new(sender.amount).px(14.0).icon_size(14.0))
    }
}

impl Default for SparksSendSheetContent {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for SparksSendSheetContent {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (primary, secondary) = theme_colors(cx);
        let interactions = SparksInteractionsStore::shared(cx);
        let service = SparksServicesStore::shared(cx);
        let (view_only, balance, maximum, amount, anonymous, top) = {
            let service = service.read(cx);
            (
                service.send_view_only,
                service.has_balance().then(|| service.spendable_now()),
                service.maximum_send_amount(),
                service.send_amount,
                service.send_anonymous,
                SparksTopSenders {
                    phase: service.top_phase.clone(),
                    sparks_hidden: service.top.as_ref().is_some_and(|top| top.sparks_hidden),
                    senders: service.top_sender_rows(cx),
                    hidden_count: service.hidden_top_senders_count(cx),
                },
            )
        };

        let opener = SparksSenderOpener { nav_store: env::<NavigationStoreKey>(cx), interactions: interactions.clone() };
        let closer = interactions.clone();
        SparksSheetScaffold::new(
            "sparks-send-scaffold",
            tr(if view_only { "sparks_top_senders_title" } else { "sparks_send_title" }),
            balance,
            move |_, cx| closer.update(cx, |interactions, cx| interactions.close_send_sheet(cx)),
            move |title_anchor, _, _| {
                let column = vstack(14.0).w_full().pb(px(16.0));
                if view_only {
                    column.child(
                        div().w_full().pt(px(4.0)).child(Self::top_senders(top, primary, secondary, Some(&title_anchor), &opener)),
                    )
                } else {
                    let top_senders = Self::top_senders(top, primary, secondary, None, &opener);
                    column.children(Self::send_controls(amount, maximum, anonymous, &title_anchor, &interactions, top_senders))
                }
            },
        )
    }
}

#[derive(IntoElement)]
pub struct SparksSendSheetFooter;

impl SparksSendSheetFooter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SparksSendSheetFooter {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for SparksSendSheetFooter {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let interactions = SparksInteractionsStore::shared(cx);
        let service = SparksServicesStore::shared(cx);
        let (view_only, disabled, amount) = {
            let service = service.read(cx);
            (service.send_view_only, service.maximum_send_amount() < 1, service.send_amount)
        };
        if view_only {
            return Empty.into_any_element();
        }

        let label = |text: &str| MainText::new(text.to_owned()).px(16.0).font_weight(FontWeight::SEMIBOLD).color(white());
        let template = tr("sparks_send_button");
        let parts: Vec<&str> = template.split("{amount}").collect();
        let prefix = parts.first().map(|part| trimming_whitespaces(part)).filter(|part| !part.is_empty());
        let suffix = parts.last().filter(|_| parts.len() > 1).map(|part| trimming_whitespaces(part)).filter(|part| !part.is_empty());

        div()
            .w_full()
            .px(px(16.0))
            .py(px(10.0))
            .child(
                ButtonUi::new("sparks-send-button")
                    .disabled(disabled)
                    .gap(6.0)
                    .on_press(move |_, cx| interactions.update(cx, |interactions, cx| interactions.send_from_sheet(cx)))
                    .child(
                        hstack(6.0)
                            .children(prefix.map(label))
                            .child(SparksIconView::new().size(18.0))
                            .child(label(&RiseFormat::compact_count(amount)))
                            .children(suffix.map(label)),
                    ),
            )
            .into_any_element()
    }
}
