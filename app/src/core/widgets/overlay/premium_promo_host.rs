use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, Render,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, linear_color_stop,
    linear_gradient, px, transparent_black, white,
};
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_measured_fraction::BottomSheetMeasuredFraction;
use rise_ui::button_ui::ButtonUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::official_badge::OfficialBadge;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::progress_view::progress_view;
use rise_ui::swiftui::shapes::pass_through;
use rise_ui::swiftui::transition::UnitPoint;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::premium_diamond::PremiumDiamond;
use crate::core::widgets::overlay::premium_promo_store::PremiumPromoStore;
use crate::modules::user::shared::premium_features::PremiumFeaturesConfig;

type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;

const SHEET_ID: &str = "premium_promo_sheet";
const SF_FILLED_SUFFIX: &str = ".fill";

pub struct PremiumPromoHost {
    store: Entity<PremiumPromoStore>,
    _store_observation: Subscription,
}

impl PremiumPromoHost {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = PremiumPromoStore::shared(cx);
        let store_observation = cx.observe(&store, |_, _, cx| cx.notify());
        Self {
            store,
            _store_observation: store_observation,
        }
    }
}

impl Render for PremiumPromoHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let read = self.store.clone();
        let write = self.store.clone();
        let redeem = self.store.clone();
        let dismiss = self.store.clone();
        let did_dismiss = self.store.clone();
        let store = self.store.read(cx);
        PremiumPromoSheet::new(
            Binding::new(
                move |cx| read.read(cx).is_presented(),
                move |is_presented, _, cx| {
                    if !is_presented {
                        write.update(cx, |store, cx| store.dismiss(cx));
                    }
                },
            ),
            store.is_verified_grant(),
            store.is_lifetime(),
            store.preview().map(|preview| preview.title.clone()),
            store.period_text(),
            store.error_text().map(str::to_owned),
            store.can_redeem(),
            store.is_redeeming(),
            move |_, cx| redeem.update(cx, |store, cx| store.redeem(cx)),
            move |_, cx| dismiss.update(cx, |store, cx| store.dismiss(cx)),
        )
        .on_did_dismiss(move |_, cx| did_dismiss.update(cx, |store, cx| store.sheet_did_dismiss(cx)))
    }
}

#[derive(IntoElement)]
pub struct PremiumPromoSheet {
    is_presented: Binding<bool>,
    is_verified_grant: bool,
    is_lifetime: bool,
    title: Option<String>,
    period_text: String,
    error_text: Option<String>,
    can_redeem: bool,
    is_redeeming: bool,
    on_redeem: SheetAction,
    on_dismiss: SheetAction,
    on_did_dismiss: SheetAction,
}

impl PremiumPromoSheet {
    const PREMIUM_FRACTION: f32 = 0.92;
    const MINIMUM_MEASURED_FRACTION: f32 = 0.34;
    const FOOTER_ALLOWANCE: f32 = 148.0;
    const FRACTION_TOLERANCE: f32 = 0.005;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        is_presented: Binding<bool>,
        is_verified_grant: bool,
        is_lifetime: bool,
        title: Option<String>,
        period_text: String,
        error_text: Option<String>,
        can_redeem: bool,
        is_redeeming: bool,
        on_redeem: impl Fn(&mut Window, &mut App) + 'static,
        on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            is_presented,
            is_verified_grant,
            is_lifetime,
            title,
            period_text,
            error_text,
            can_redeem,
            is_redeeming,
            on_redeem: Rc::new(on_redeem),
            on_dismiss: Rc::new(on_dismiss),
            on_did_dismiss: Rc::new(|_, _| {}),
        }
    }

    pub fn on_did_dismiss(mut self, on_did_dismiss: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_did_dismiss = Rc::new(on_did_dismiss);
        self
    }

    fn content(
        is_verified_grant: bool,
        is_lifetime: bool,
        title: Option<&str>,
        period_text: &str,
        error_text: Option<&str>,
        sheet_fraction: &Entity<f32>,
        theme: &AppTheme,
    ) -> AnyElement {
        let features = PremiumFeaturesConfig::features(theme.primary._100);
        let mut column = div().relative().w_full().pt_3().flex().flex_col().items_center().gap_5();

        column = if is_verified_grant {
            column.child(div().pt_2().child(OfficialBadge::new().size(px(108.0))))
        } else {
            column.child(PremiumDiamond::new().size(124.0))
        };

        let mut lines = div().w_full().flex().flex_col().items_center().gap(px(10.0)).child(
            MainText::new(tr(if is_verified_grant {
                "verified_promo_title"
            } else {
                "premium_promo_title"
            }))
            .px(26.0)
            .tac(TextAlignT::Center)
            .font_weight(FontWeight::BOLD),
        );
        if let Some(title) = title.filter(|title| !title.is_empty()) {
            lines = lines.child(
                div().px_5().child(
                    SecondaryText::new(title.to_owned())
                        .px(15.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary),
                ),
            );
        }
        if title.is_some() {
            lines = lines.child(Self::period_pill(is_verified_grant, is_lifetime, period_text, theme));
        }
        lines = lines.child(
            div().px_6().child(
                MainText::new(tr(if is_verified_grant {
                    "verified_promo_subtitle"
                } else {
                    "premium_promo_subtitle"
                }))
                .px(14.0)
                .tac(TextAlignT::Center)
                .color(theme.text.secondary),
            ),
        );
        column = column.child(lines);

        if let Some(error) = error_text.filter(|error| !error.is_empty()) {
            column = column.child(Self::notice(error, theme.semantic.error_100, theme));
        }

        let rows = if is_verified_grant {
            vec![
                GroupedBtn::new("verified", tr("verified_promo_badge"))
                    .icon("checkmark.seal.fill")
                    .icon_background(theme.primary._100)
                    .subtitle(tr("verified_promo_badge_sub"))
                    .show_chevron(false),
            ]
        } else {
            features
                .into_iter()
                .map(|feature| {
                    GroupedBtn::new(feature.id, feature.title)
                        .icon(feature.icon)
                        .icon_background(feature.color)
                        .subtitle(feature.subtitle)
                        .show_chevron(false)
                })
                .collect()
        };
        column = column
            .child(GroupedBtns::new("premium_promo_rows", rows))
            .child(div().h(px(Self::FOOTER_ALLOWANCE)));

        let fraction = sheet_fraction.clone();
        column = column.child(measure("premium_promo_content_height", move |bounds, window, cx| {
            Self::adopt_content_height(is_verified_grant, f32::from(bounds.size.height), &fraction, window, cx);
        }));

        div()
            .id("premium_promo_scroll")
            .size_full()
            .overflow_y_scroll()
            .child(column)
            .into_any_element()
    }

    fn adopt_content_height(
        is_verified_grant: bool,
        height: f32,
        sheet_fraction: &Entity<f32>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let target = if is_verified_grant {
            BottomSheetMeasuredFraction::value(height, Self::MINIMUM_MEASURED_FRACTION, window)
        } else {
            Self::PREMIUM_FRACTION
        };
        sheet_fraction.update(cx, |fraction, cx| {
            if (target - *fraction).abs() <= Self::FRACTION_TOLERANCE {
                return;
            }
            *fraction = target;
            cx.notify();
        });
    }

    fn period_pill(is_verified_grant: bool, is_lifetime: bool, period_text: &str, theme: &AppTheme) -> AnyElement {
        let symbol = if is_verified_grant {
            "checkmark.seal.fill"
        } else if is_lifetime {
            "infinity"
        } else {
            "clock.fill"
        };
        let (start, end) = theme.gradient_primary;
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px_4()
            .py_2()
            .rounded_full()
            .bg(linear_gradient(90.0, linear_color_stop(start, 0.0), linear_color_stop(end, 1.0)))
            .children(IconUi::sized(symbol, px(13.0), white(), symbol.ends_with(SF_FILLED_SUFFIX)))
            .child(
                MainText::new(SharedString::from(period_text.to_owned()))
                    .px(15.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(white()),
            )
            .into_any_element()
    }

    fn notice(text: &str, color: Hsla, theme: &AppTheme) -> AnyElement {
        div()
            .w_full()
            .px_3()
            .child(
                div()
                    .w_full()
                    .px_4()
                    .py_3()
                    .rounded(px(14.0))
                    .bg(theme.bg._200)
                    .child(
                        SecondaryText::new(text.to_owned())
                            .px(14.0)
                            .tac(TextAlignT::Center)
                            .color(color),
                    ),
            )
            .into_any_element()
    }

    fn footer(
        is_verified_grant: bool,
        can_redeem: bool,
        is_redeeming: bool,
        on_redeem: &SheetAction,
        on_dismiss: &SheetAction,
        theme: &AppTheme,
    ) -> AnyElement {
        let mut redeem = ButtonUi::new("premium_promo_redeem")
            .background_color(theme.primary._100)
            .height(52.0);
        if can_redeem && !is_redeeming {
            let on_redeem = on_redeem.clone();
            redeem = redeem.on_press(move |window, cx| on_redeem(window, cx));
        }
        let redeem = if is_redeeming {
            redeem.child(progress_view().tint(theme.content_on(theme.primary._100)))
        } else {
            redeem.child(
                MainText::new(tr(if is_verified_grant {
                    "verified_promo_cta"
                } else {
                    "premium_promo_cta"
                }))
                .px(16.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(white()),
            )
        };
        let on_dismiss = on_dismiss.clone();

        div()
            .relative()
            .w_full()
            .px_4()
            .pt(px(18.0))
            .pb_1()
            .child(SheetFooterFade::new(theme.bg._100))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().w_full().opacity(if can_redeem { 1.0 } else { 0.6 }).child(redeem))
                    .child(
                        div()
                            .id("premium_promo_later")
                            .w_full()
                            .min_h(px(40.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .on_click(move |_, window, cx| on_dismiss(window, cx))
                            .child(
                                SecondaryText::new(tr("premium_upsell_later"))
                                    .px(15.0)
                                    .tac(TextAlignT::Center)
                                    .color(theme.text.secondary),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl RenderOnce for PremiumPromoSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let sheet_fraction = window.use_keyed_state((ElementId::from(SHEET_ID), "sheet_fraction"), cx, |_, _| Self::PREMIUM_FRACTION);
        let snap_fraction = *sheet_fraction.read(cx);
        let sheet_background = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        let is_verified_grant = self.is_verified_grant;
        let is_lifetime = self.is_lifetime;
        let title = self.title;
        let period_text = self.period_text;
        let error_text = self.error_text;
        let can_redeem = self.can_redeem;
        let is_redeeming = self.is_redeeming;
        let on_redeem = self.on_redeem;
        let on_dismiss = self.on_dismiss;
        let on_did_dismiss = self.on_did_dismiss;

        div().size_full().child(
            BottomSheetUi::new(
                SHEET_ID,
                self.is_presented,
                move |_, cx| {
                    let theme_store = ThemeStore::shared(cx);
                    let theme = theme_store.read(cx).current_theme();
                    Self::content(
                        is_verified_grant,
                        is_lifetime,
                        title.as_deref(),
                        &period_text,
                        error_text.as_deref(),
                        &sheet_fraction,
                        theme,
                    )
                },
                move |_, cx| {
                    let theme_store = ThemeStore::shared(cx);
                    let theme = theme_store.read(cx).current_theme();
                    Self::footer(is_verified_grant, can_redeem, is_redeeming, &on_redeem, &on_dismiss, theme)
                },
            )
            .snap_fractions(vec![snap_fraction])
            .full_page(false)
            .dismisses_on_backdrop_tap(true)
            .full_page_shows_drag_indicator(true)
            .footer_shows_top_border(false)
            .footer_overlays_content(Some(true))
            .footer_horizontal_padding(Some(0.0))
            .footer_top_padding(Some(0.0))
            .content_scrollable(true)
            .sheet_background(Some(sheet_background))
            .footer_background(Some(transparent_black()))
            .on_did_dismiss(move |window, cx| on_did_dismiss(window, cx)),
        )
    }
}

#[derive(IntoElement)]
pub struct SheetFooterFade {
    color: Hsla,
    fade_height: f32,
    bottom_bleed: f32,
}

impl SheetFooterFade {
    pub fn new(color: Hsla) -> Self {
        Self {
            color,
            fade_height: 36.0,
            bottom_bleed: 60.0,
        }
    }

    pub fn fade_height(mut self, fade_height: f32) -> Self {
        self.fade_height = fade_height;
        self
    }

    pub fn bottom_bleed(mut self, bottom_bleed: f32) -> Self {
        self.bottom_bleed = bottom_bleed;
        self
    }
}

impl RenderOnce for SheetFooterFade {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        pass_through()
            .absolute()
            .left_0()
            .right_0()
            .top(px(-self.fade_height))
            .bottom(px(-self.bottom_bleed))
            .flex()
            .flex_col()
            .child(
                LinearGradient::colors([self.color.opacity(0.0), self.color], UnitPoint::TOP, UnitPoint::BOTTOM)
                    .view()
                    .w_full()
                    .h(px(self.fade_height)),
            )
            .child(div().w_full().flex_1().bg(self.color))
    }
}
