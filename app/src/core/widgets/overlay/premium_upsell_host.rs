use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder,
};
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::bottom_sheet_ui::bottom_sheet_measured_fraction::{
    BottomSheetMeasuredFraction, BottomSheetMeasuredFractionModifiers,
};
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::lifecycle::on_change;

use crate::app::router::navigation_store::{NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::premium_diamond::PremiumDiamond;
use crate::core::widgets::overlay::download_limit_help_modal::DownloadLimitHelpModal;
use crate::core::widgets::overlay::premium_upsell_store::{
    PremiumUpsellInteractionsStore, PremiumUpsellPrompt, PremiumUpsellStore,
};

type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;

const SHEET_ID: &str = "premium_upsell_sheet";

const INITIAL_FRACTION: f32 = 0.58;
const MINIMUM_FRACTION: f32 = 0.3;

pub struct PremiumUpsellHost {
    store: Entity<PremiumUpsellStore>,
    _store_observation: Subscription,
}

impl PremiumUpsellHost {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = PremiumUpsellStore::shared(cx);
        let store_observation = cx.observe(&store, |_, _, cx| cx.notify());
        Self {
            store,
            _store_observation: store_observation,
        }
    }

    fn presentation_binding(&self) -> Binding<bool> {
        let store = self.store.clone();
        Binding::new(
            move |cx| store.read(cx).prompt().is_some(),
            |is_presented, _, cx| {
                if !is_presented {
                    PremiumUpsellInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.dismiss(cx));
                }
            },
        )
    }
}

impl Render for PremiumUpsellHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        PremiumUpsellSheet::new(
            SHEET_ID,
            self.presentation_binding(),
            self.store.read(cx).prompt().cloned(),
            |_, cx| {
                PremiumUpsellInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.dismiss(cx));
            },
            move |_, cx| {
                PremiumUpsellInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.proceed_to_paywall(cx));
                if let Some(nav_store) = nav_store.as_ref() {
                    nav_store.update(cx, |nav_store, cx| {
                        nav_store.push(RootRoute::SubscriptionSettings, NavigationStoreSettlingPush::Drop, cx);
                    });
                }
            },
        )
    }
}

#[derive(IntoElement)]
pub struct PremiumUpsellSheet {
    id: ElementId,
    is_presented: Binding<bool>,
    prompt: Option<PremiumUpsellPrompt>,
    on_dismiss: SheetAction,
    on_upgrade: SheetAction,
    title: Option<SharedString>,
    shows_explanation: bool,
}

impl PremiumUpsellSheet {
    /// id keys the sheet and its own state
    /// several sheets can sit beside the root host each with its own help flag
    pub fn new(
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        prompt: Option<PremiumUpsellPrompt>,
        on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
        on_upgrade: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            is_presented,
            prompt,
            on_dismiss: Rc::new(on_dismiss),
            on_upgrade: Rc::new(on_upgrade),
            title: None,
            shows_explanation: true,
        }
    }

    pub fn title(mut self, title: Option<SharedString>) -> Self {
        self.title = title;
        self
    }

    pub fn shows_explanation(mut self, shows_explanation: bool) -> Self {
        self.shows_explanation = shows_explanation;
        self
    }

    fn default_title(prompt: Option<&PremiumUpsellPrompt>) -> String {
        if prompt.is_some_and(|prompt| prompt.is_download_limit) {
            return tr("download_limit_title");
        }
        if prompt.is_some_and(|prompt| prompt.is_feature) {
            return tr("server_error_premium_required");
        }
        tr("premium_upsell_title")
    }

    fn content(
        id: &ElementId,
        prompt: Option<&PremiumUpsellPrompt>,
        title: Option<SharedString>,
        shows_explanation: bool,
        shows_download_help: &Entity<bool>,
        theme: &AppTheme,
    ) -> AnyElement {
        let mut column = div().w_full().pt_2().pb_6().flex().flex_col().items_center().gap_5();

        if prompt.is_some_and(|prompt| prompt.is_download_limit) {
            let open_help = shows_download_help.clone();
            column = column.child(
                div()
                    .w_full()
                    .px_4()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .child(
                        ButtonUi::new((id.clone(), "download_help"))
                            .background_color(theme.bg._200)
                            .height(32.0)
                            .fit_content(true)
                            .on_press(move |_, cx| {
                                open_help.update(cx, |shows, cx| {
                                    *shows = true;
                                    cx.notify();
                                });
                            })
                            .child(
                                div()
                                    .w_8()
                                    .flex()
                                    .justify_center()
                                    .child(MainText::new("?").px(18.0).font_weight(FontWeight::SEMIBOLD)),
                            ),
                    ),
            );
        }

        column = column.child(PremiumDiamond::new().size(96.0));

        let mut lines = div().w_full().flex().flex_col().items_center().gap_2().child(
            MainText::new(title.unwrap_or_else(|| Self::default_title(prompt).into()))
                .px(24.0)
                .tac(TextAlignT::Center)
                .font_weight(FontWeight::BOLD),
        );
        if let Some(reason) = prompt.map(|prompt| prompt.reason.clone())
            && !reason.is_empty()
        {
            lines = lines.child(
                div().px_5().child(
                    MainText::new(reason)
                        .px(15.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary),
                ),
            );
        }
        if shows_explanation && let Some(recovery) = prompt.and_then(|prompt| prompt.recovery_text.clone()) {
            lines = lines.child(
                div().px_5().child(
                    MainText::new(recovery)
                        .px(14.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary),
                ),
            );
        }
        if shows_explanation
            && prompt.is_some_and(|prompt| prompt.can_upgrade)
            && !prompt.is_some_and(|prompt| prompt.is_feature)
        {
            lines = lines.child(
                div().px_5().child(
                    MainText::new(tr("premium_upsell_subtitle"))
                        .px(14.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary),
                ),
            );
        }

        column.child(lines).into_any_element()
    }

    fn footer(
        prompt: Option<&PremiumUpsellPrompt>,
        hides_cta: bool,
        on_dismiss: &SheetAction,
        on_upgrade: &SheetAction,
        theme: &AppTheme,
    ) -> AnyElement {
        let can_upgrade = prompt.is_some_and(|prompt| prompt.can_upgrade);
        let is_download_limit = prompt.is_some_and(|prompt| prompt.is_download_limit);
        let mut column = div().w_full().px(gpui::px(6.0)).flex().flex_col().gap(gpui::px(10.0));

        if can_upgrade {
            let on_upgrade = on_upgrade.clone();
            column = column.child(
                div().w_full().when(hides_cta, |cta| cta.opacity(0.0)).child(
                ButtonUi::new("premium_upsell_cta")
                    .background_color(theme.primary._100)
                    .height(52.0)
                    .on_press(move |window, cx| on_upgrade(window, cx))
                    .child(
                        MainText::new(tr("premium_upsell_cta"))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.content_on(theme.primary._100)),
                    ),
                ),
            );
        } else if !is_download_limit {
            let on_dismiss = on_dismiss.clone();
            column = column.child(
                ButtonUi::new("premium_upsell_done")
                    .background_color(theme.primary._100)
                    .height(52.0)
                    .on_press(move |window, cx| on_dismiss(window, cx))
                    .child(
                        MainText::new(tr("release_notes_done"))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.content_on(theme.primary._100)),
                    ),
            );
        }

        if can_upgrade || is_download_limit {
            let on_dismiss = on_dismiss.clone();
            let label = tr(if is_download_limit {
                "download_limit_wait"
            } else {
                "premium_upsell_later"
            });
            column = column.child(
                div()
                    .id("premium_upsell_later")
                    .w_full()
                    .min_h(gpui::px(40.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .on_click(move |_, window, cx| on_dismiss(window, cx))
                    .child(
                        SecondaryText::new(label)
                            .px(15.0)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary),
                    ),
            );
        }

        column.into_any_element()
    }
}

impl RenderOnce for PremiumUpsellSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let shows_download_help = window.use_keyed_state(
            (id.clone(), "shows_download_help"),
            cx,
            |_, _| false,
        );

        let hide_on_new_prompt = shows_download_help.clone();
        on_change(
            (id.clone(), "prompt-id"),
            self.prompt.as_ref().map(|prompt| prompt.id),
            false,
            window,
            cx,
            move |_, _, _, cx| {
                hide_on_new_prompt.update(cx, |shows, cx| {
                    *shows = false;
                    cx.notify();
                });
            },
        );
        let hide_on_dismiss = shows_download_help.clone();
        on_change(
            (id.clone(), "is-presented"),
            self.is_presented.get(cx),
            false,
            window,
            cx,
            move |_, presented, _, cx| {
                if *presented {
                    return;
                }
                hide_on_dismiss.update(cx, |shows, cx| {
                    *shows = false;
                    cx.notify();
                });
            },
        );

        let content_height = window.use_keyed_state((id.clone(), "content_height"), cx, |_, _| 0.0_f32);
        let footer_height = window.use_keyed_state((id.clone(), "footer_height"), cx, |_, _| 0.0_f32);
        let sheet_fraction = BottomSheetMeasuredFraction::attached(
            *content_height.read(cx),
            *footer_height.read(cx),
            MINIMUM_FRACTION,
            window,
        )
        .unwrap_or(INITIAL_FRACTION);
        let report_content = Rc::new(store_height(content_height));
        let report_footer = Rc::new(store_height(footer_height));
        let footer_id = id.clone();

        let sheet_background = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        let content_id = id.clone();
        let is_download_limit = self.prompt.as_ref().is_some_and(|prompt| prompt.is_download_limit);
        let help_binding = Binding::entity(
            shows_download_help.clone(),
            |shows: &bool| *shows,
            |shows: &mut bool, value, _| *shows = value,
        );
        let footer_help = shows_download_help.clone();
        let content_prompt = self.prompt.clone();
        let footer_prompt = self.prompt;
        let title = self.title;
        let shows_explanation = self.shows_explanation;
        let on_dismiss = self.on_dismiss;
        let on_upgrade = self.on_upgrade;

        div().size_full().child(
            BottomSheetUi::new(
                id.clone(),
                self.is_presented,
                move |_, cx| {
                    let theme_store = ThemeStore::shared(cx);
                    let theme = theme_store.read(cx).current_theme();
                    div()
                        .id((content_id.clone(), "scroll"))
                        .size_full()
                        .overflow_y_scroll()
                        .child(
                            Self::content(
                                &content_id,
                                content_prompt.as_ref(),
                                title.clone(),
                                shows_explanation,
                                &shows_download_help,
                                theme,
                            )
                            .on_sheet_part_height((content_id.clone(), "content_part"), {
                                let report = report_content.clone();
                                move |height: f32, window: &mut Window, cx: &mut App| report(height, window, cx)
                            }),
                        )
                },
                move |_, cx| {
                    let theme_store = ThemeStore::shared(cx);
                    let theme = theme_store.read(cx).current_theme();
                    let hides_cta = *footer_help.read(cx);
                    Self::footer(footer_prompt.as_ref(), hides_cta, &on_dismiss, &on_upgrade, theme).on_sheet_part_height(
                        (footer_id.clone(), "footer_part"),
                        {
                            let report = report_footer.clone();
                            move |height: f32, window: &mut Window, cx: &mut App| report(height, window, cx)
                        },
                    )
                },
            )
            .snap_fractions(vec![sheet_fraction])
            .modal_sized_by_detent(true)
            .full_page(false)
            .dismisses_on_backdrop_tap(true)
            .full_page_shows_drag_indicator(true)
            .footer_shows_top_border(false)
            .content_scrollable(true)
            .sheet_background(Some(sheet_background))
            .footer_background(Some(sheet_background)),
        )
        .when(is_download_limit, |root| {
            root.child(DownloadLimitHelpModal::new((id.clone(), "download_limit_help"), help_binding))
        })
    }
}

fn store_height(height: Entity<f32>) -> impl Fn(f32, &mut Window, &mut App) + 'static {
    move |value, _, cx| {
        height.update(cx, |height, cx| {
            *height = value;
            cx.notify();
        });
    }
}
