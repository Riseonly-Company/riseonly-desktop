use std::rc::Rc;

use gpui::{App, Context, Entity, FontWeight, Hsla, Render, Rgba, SharedString, Subscription, Window, white};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_measured_fraction::BottomSheetMeasuredFraction;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::auth::components::web_login_approval::web_login_approval_store::WebLoginApprovalStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::user::stores::user::user_actions::user_types::ProfileModel;

type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct WebLoginApprovalSheet {
    store: Entity<WebLoginApprovalStore>,
    _subscriptions: Vec<Subscription>,
}

impl WebLoginApprovalSheet {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let store = WebLoginApprovalStore::shared(cx);
        let services = AuthServicesStore::shared(cx);
        let subscriptions = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.observe(&services, |_, _, cx| cx.notify()),
        ];
        Self { store, _subscriptions: subscriptions }
    }
}

impl Render for WebLoginApprovalSheet {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let profile = self.store.update(cx, |store, cx| store.profile(cx));
        let store = self.store.read(cx);
        let error_text = store.error_text().map(str::to_owned);
        let is_approving = store.is_approving();
        let is_denying = store.is_denying();

        let reader = self.store.clone();
        let writer = self.store.clone();
        let approving = self.store.clone();
        let denying = self.store.clone();
        WebLoginApprovalContent::new(
            Binding::new(
                move |cx| reader.read(cx).is_presented(),
                move |presented, _, cx| {
                    if !presented {
                        writer.update(cx, |store, cx| store.dismiss(cx));
                    }
                },
            ),
            profile,
            error_text,
            is_approving,
            is_denying,
            move |_, cx| approving.update(cx, |store, cx| store.approve(cx)),
            move |_, cx| denying.update(cx, |store, cx| store.deny(cx)),
        )
    }
}

#[derive(IntoElement)]
pub struct WebLoginApprovalContent {
    is_presented: Binding<bool>,
    profile: Option<ProfileModel>,
    error_text: Option<String>,
    is_approving: bool,
    is_denying: bool,
    on_approve: SheetAction,
    on_deny: SheetAction,
}

impl WebLoginApprovalContent {
    const MINIMUM_FRACTION: f32 = 0.5;
    const FOOTER_HEIGHT: f32 = 76.0;

    pub fn new(
        is_presented: Binding<bool>,
        profile: Option<ProfileModel>,
        error_text: Option<String>,
        is_approving: bool,
        is_denying: bool,
        on_approve: impl Fn(&mut Window, &mut App) + 'static,
        on_deny: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            is_presented,
            profile,
            error_text,
            is_approving,
            is_denying,
            on_approve: Rc::new(on_approve),
            on_deny: Rc::new(on_deny),
        }
    }

    fn accent() -> Hsla {
        Rgba { r: 1.0, g: 0.353, b: 0.353, a: 1.0 }.into()
    }

    fn content(&self, sheet_fraction: Entity<f32>, theme: &AppTheme) -> ScrollView {
        scroll_view("web-login-approval-scroll").shows_indicators(false).size_full().child(
            div().w_full().pb(px(Self::FOOTER_HEIGHT)).child(self.sheet_body(sheet_fraction, theme)),
        )
    }

    fn sheet_body(&self, sheet_fraction: Entity<f32>, theme: &AppTheme) -> Div {
        let secondary = theme.text.secondary;
        vstack(16.0)
            .relative()
            .w_full()
            .pb(px(8.0))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .mt(px(20.0))
                    .size(px(64.0))
                    .rounded_full()
                    .bg(theme.bg._200)
                    .children(IconUi::sized("qrcode.viewfinder", px(30.0), theme.text.primary, false)),
            )
            .child(
                MainText::new(tr("weblogin_title"))
                    .px(20.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD)
                    .w_full(),
            )
            .child(
                MainText::new(tr("weblogin_subtitle"))
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(secondary)
                    .number_of_lines(3)
                    .w_full(),
            )
            .children(self.profile.as_ref().map(|profile| Self::account_card(profile, theme)))
            .children(self.error_text.clone().map(|error| {
                div().w_full().px(px(20.0)).child(
                    MainText::new(error)
                        .px(13.0)
                        .tac(TextAlignT::Center)
                        .color(Self::accent())
                        .number_of_lines(3)
                        .w_full(),
                )
            }))
            .child(
                div().w_full().px(px(26.0)).pt(px(2.0)).child(
                    MainText::new(tr("weblogin_note"))
                        .px(12.0)
                        .tac(TextAlignT::Center)
                        .color(secondary)
                        .number_of_lines(3)
                        .w_full(),
                ),
            )
            .child(measure("web-login-approval-body-height", move |bounds, window, cx| {
                Self::adopt_content_height(f32::from(bounds.size.height), &sheet_fraction, window, cx);
            }))
    }

    fn account_card(profile: &ProfileModel, theme: &AppTheme) -> Div {
        div().w_full().px(px(20.0)).child(
            hstack(12.0)
                .w_full()
                .px(px(14.0))
                .py(px(10.0))
                .rounded(px(16.0))
                .bg(theme.bg._200)
                .child(
                    UserLogoUi::new("web-login-approval-avatar")
                        .source(profile.avatar_url.clone().map(SharedString::from))
                        .name(SharedString::from(profile.name.clone()))
                        .user_id(SharedString::from(profile.id.clone()))
                        .size(44.0)
                        .show_story_ring(false),
                )
                .child(
                    vstack(2.0)
                        .align(HorizontalAlignment::Leading)
                        .min_w_0()
                        .child(
                            MainText::new(profile.name.clone())
                                .px(15.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .number_of_lines(1),
                        )
                        .when(!profile.tag.is_empty(), |column| {
                            column.child(
                                MainText::new(format!("@{}", profile.tag))
                                    .px(13.0)
                                    .color(theme.text.secondary)
                                    .number_of_lines(1),
                            )
                        }),
                )
                .child(spacer()),
        )
    }

    fn adopt_content_height(height: f32, sheet_fraction: &Entity<f32>, window: &mut Window, cx: &mut App) {
        let measured = BottomSheetMeasuredFraction::value(
            height + Self::FOOTER_HEIGHT,
            Self::MINIMUM_FRACTION,
            window,
        );
        sheet_fraction.update(cx, |fraction, cx| {
            if (measured - *fraction).abs() > 0.005 {
                *fraction = measured;
                cx.notify();
            }
        });
    }

    fn footer(&self, theme: &AppTheme) -> Div {
        let busy = self.is_approving || self.is_denying;
        let on_deny = self.on_deny.clone();
        let on_approve = self.on_approve.clone();
        hstack(12.0)
            .w_full()
            .px(px(16.0))
            .py(px(12.0))
            .child(
                plain("web-login-approval-deny")
                    .flex_1()
                    .disabled(busy)
                    .on_press(move |window, cx| on_deny(window, cx))
                    .child(
                        Self::footer_label(theme.bg._200)
                            .when(self.is_denying, |label| {
                                label.child(progress_view().tint(theme.text.primary))
                            })
                            .when(!self.is_denying, |label| {
                                label.child(
                                    MainText::new(tr("weblogin_cancel")).px(16.0).font_weight(FontWeight::SEMIBOLD),
                                )
                            }),
                    ),
            )
            .child(
                plain("web-login-approval-approve")
                    .flex_1()
                    .disabled(busy)
                    .on_press(move |window, cx| on_approve(window, cx))
                    .child(
                        Self::footer_label(Self::accent())
                            .when(self.is_approving, |label| label.child(progress_view().tint(white())))
                            .when(!self.is_approving, |label| {
                                label.child(
                                    MainText::new(tr("weblogin_approve"))
                                        .px(16.0)
                                        .font_weight(FontWeight::BOLD)
                                        .color(white()),
                                )
                            }),
                    ),
            )
    }

    fn footer_label(background: Hsla) -> Div {
        div().flex().items_center().justify_center().w_full().h(px(52.0)).rounded_full().bg(background)
    }
}

impl RenderOnce for WebLoginApprovalContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._100;
        let sheet_fraction =
            window.use_keyed_state("web-login-approval-fraction", cx, |_, _| Self::MINIMUM_FRACTION);
        let fraction = *sheet_fraction.read(cx);

        let is_presented = self.is_presented.clone();
        let content = Rc::new(self);
        let footer = content.clone();
        BottomSheetUi::new(
            "web-login-approval-sheet",
            is_presented,
            move |_, cx| {
                let theme = rise_ui::theme(cx).clone();
                content.content(sheet_fraction.clone(), &theme)
            },
            move |_, cx| {
                let theme = rise_ui::theme(cx).clone();
                footer.footer(&theme)
            },
        )
        .snap_fractions(vec![fraction])
        .full_page(true)
        .full_page_shows_drag_indicator(true)
        .footer_shows_top_border(true)
        .content_scrollable(true)
        .full_page_covers_full_screen(false)
        .full_page_interaction_locked(false)
        .sheet_background(Some(background))
        .footer_background(Some(background))
    }
}
