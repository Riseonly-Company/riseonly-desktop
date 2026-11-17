use gpui::{AnyElement, App, Context, Entity, FontWeight, Render, Subscription, Task, Window};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::bg_wrapper_ui::BgWrapperUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::lottie_view::LottieView;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::pressable_ui::PressableUi;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::skeleton_ui::SkeletonLine;
use rise_ui::swiftui::prelude::*;

use crate::modules::auth::stores::auth_interactions::auth_interactions::AuthInteractionsStore;
use crate::modules::moderation::engine::rise_moderation_presentation::{
    ModerationLoadState, RiseModerationPresentation,
};
use crate::modules::moderation::engine::rise_moderation_wire::RiseStoredUserBlock;
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    AppealStatus, BlockTimeFormatting, ReportReasonExt,
};
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;
use crate::modules::user::pages::profile::settings::moderation_settings::appeal_sheet::AppealSheetStore;

#[derive(Clone, PartialEq, Eq, Hash)]
struct AppealLookup {
    block: Option<RiseStoredUserBlock>,
    refresh: u64,
    resolution: u64,
    session_id: Option<Uuid>,
}

struct AppealResult {
    lookup: AppealLookup,
    status: AppealStatus,
}

pub struct BlockedAccountPage {
    presentation: Entity<RiseModerationPresentation>,
    auth: Entity<AuthInteractionsStore>,
    moderation: Entity<ModerationInteractionsStore>,
    appeal_refresh: u64,
    appeal_result: Option<AppealResult>,
    observed_session_id: Option<Uuid>,
    task_lookup: Option<AppealLookup>,
    appeal_task: Option<Task<()>>,
    viewport_height: f32,
    _subscriptions: Vec<Subscription>,
}

impl BlockedAccountPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let presentation = RiseModerationPresentation::shared(cx);
        let auth = AuthInteractionsStore::shared(cx);
        let moderation = ModerationInteractionsStore::shared(cx);

        let subscriptions = vec![
            cx.observe(&presentation, |this, _, cx| {
                this.session_id_did_change(cx);
                this.sync_appeal_lookup(cx);
                cx.notify();
            }),
            cx.observe(&moderation, |this, _, cx| {
                this.sync_appeal_lookup(cx);
                cx.notify();
            }),
            cx.observe(&auth, |_, _, cx| cx.notify()),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.reconcile_account_block(cx);
                    this.appeal_refresh = this.appeal_refresh.wrapping_add(1);
                    this.sync_appeal_lookup(cx);
                    cx.notify();
                }
            }),
        ];

        let page = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = page.update(cx, |this, cx| {
                this.reconcile_account_block(cx);
                this.sync_appeal_lookup(cx);
            });
        });

        let observed_session_id = presentation.read(cx).session_id();
        Self {
            presentation,
            auth,
            moderation,
            appeal_refresh: 0,
            appeal_result: None,
            observed_session_id,
            task_lookup: None,
            appeal_task: None,
            viewport_height: 0.0,
            _subscriptions: subscriptions,
        }
    }

    fn appeal_lookup(&self, cx: &App) -> AppealLookup {
        let presentation = self.presentation.read(cx);
        AppealLookup {
            block: presentation.active_full_block(),
            refresh: self.appeal_refresh,
            resolution: self.moderation.read(cx).blocked_account_appeal_revision(),
            session_id: presentation.session_id(),
        }
    }

    fn reconcile_account_block(&self, cx: &mut Context<Self>) {
        self.auth.update(cx, |auth, cx| auth.reconcile_account_block(cx));
    }

    fn session_id_did_change(&mut self, cx: &mut Context<Self>) {
        let presentation = self.presentation.read(cx);
        let session_id = presentation.session_id();
        let is_active = presentation.is_active();
        if session_id == self.observed_session_id {
            return;
        }
        self.observed_session_id = session_id;
        if session_id.is_some() && is_active {
            self.reconcile_account_block(cx);
        }
    }

    /// a new appeal id drops the running lookup and clears its answer
    fn sync_appeal_lookup(&mut self, cx: &mut Context<Self>) {
        let lookup = self.appeal_lookup(cx);
        if self.task_lookup.as_ref() == Some(&lookup) {
            return;
        }
        self.task_lookup = Some(lookup.clone());
        self.appeal_task = None;
        self.appeal_result = None;
        cx.notify();

        let Some(block) = lookup.block.clone().filter(|block| !block.allows_appeal()) else {
            return;
        };
        let status = self
            .moderation
            .update(cx, |moderation, cx| moderation.blocked_account_appeal_status(&block, cx));
        self.appeal_task = Some(cx.spawn(async move |this, cx| {
            let Some(status) = status.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.appeal_lookup(cx) != lookup {
                    return;
                }
                this.appeal_result = Some(AppealResult { lookup, status });
                cx.notify();
            });
        }));
    }

    fn content(
        &self,
        block: Option<&RiseStoredUserBlock>,
        state: &ModerationLoadState,
        is_loading: bool,
        can_play_panda: bool,
        theme: &AppTheme,
    ) -> Div {
        let allows_appeal = block.map(RiseStoredUserBlock::allows_appeal);
        let description = if allows_appeal == Some(false) {
            "blocked_account_description_no_appeal"
        } else {
            "blocked_account_description"
        };

        let card = if is_loading {
            Some(Self::skeleton_card(theme))
        } else if let Some(block) = block {
            Some(Self::reason_card(block, theme))
        } else {
            state.error_text().map(|message| self.failed_card(message, theme))
        };

        vstack(14.0)
            .w_full()
            .max_w(px(Self::COLUMN_MAX_WIDTH))
            .mx_auto()
            .px(px(24.0))
            .child(div().flex_none().pb(px(2.0)).child(Self::panda_hero(can_play_panda, theme)))
            .child(
                MainText::new(tr("blocked_account_title"))
                    .px(24.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD)
                    .w_full(),
            )
            .child(SecondaryText::new(tr(description)).px(15.0).tac(TextAlignT::Center).w_full())
            .children(card)
            .when(allows_appeal == Some(true), |column| {
                column.child(
                    div().w_full().pt(px(2.0)).child(
                        SecondaryText::new(tr("blocked_account_appeal_hint"))
                            .px(13.0)
                            .tac(TextAlignT::Center)
                            .w_full(),
                    ),
                )
            })
    }

    const COLUMN_MAX_WIDTH: f32 = 480.0;

    const PANDA_POSE: &'static str = "no";

    fn panda_hero(can_play: bool, theme: &AppTheme) -> AnyElement {
        if can_play {
            div()
                .flex_none()
                .size(px(160.0))
                .child(LottieView::new(Self::PANDA_POSE).size(160.0).looping(true).speed(0.8))
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(84.0))
                .rounded_full()
                .bg(theme.semantic.error_100.opacity(0.14))
                .children(IconUi::sized("nosign", px(36.0), theme.semantic.error_100, false))
                .into_any_element()
        }
    }

    fn reason_text(block: &RiseStoredUserBlock) -> String {
        if let Some(key) = block.canon_reason_key() {
            return tr(&key);
        }
        if let Some(reason) = block.resolved_report_reason() {
            return tr(reason.title_key());
        }
        tr("blocked_account_no_reason")
    }

    fn object_text(block: &RiseStoredUserBlock) -> String {
        block
            .snapshot_title
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|object| !object.is_empty())
            .map_or_else(|| tr("settings_section_account"), str::to_owned)
    }

    fn card_surface(spacing: f32, theme: &AppTheme) -> Div {
        vstack(spacing).w_full().mt(px(6.0)).p(px(16.0)).bg(theme.bg._200).rounded(px(16.0))
    }

    fn reason_card(block: &RiseStoredUserBlock, theme: &AppTheme) -> Div {
        let error = theme.semantic.error_100;
        let expiry = if block.is_permanent() {
            Some(tr("blocks_permanent"))
        } else {
            BlockTimeFormatting::remaining(Some(block.block_end_timestamp))
                .map(|remaining| tr("blocks_lifts").replace("{when}", &remaining))
        };
        let symbol = if block.is_permanent() { "infinity" } else { "clock.fill" };

        Self::card_surface(12.0, theme)
            .align(HorizontalAlignment::Leading)
            .child(Self::fact_row(tr("blocks_why_label"), Self::reason_text(block), theme))
            .child(Self::fact_row(tr("blocks_target_label"), Self::object_text(block), theme))
            .child(
                hstack(8.0)
                    .w_full()
                    .children(IconUi::sized(symbol, px(12.0), error, false))
                    .children(expiry.map(|text| {
                        MainText::new(text).px(13.0).font_weight(FontWeight::SEMIBOLD).color(error)
                    }))
                    .child(spacer_min_w(0.0)),
            )
    }

    fn fact_row(label: String, value: String, theme: &AppTheme) -> Div {
        vstack(4.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(MainText::new(label).px(12.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.secondary))
            .child(MainText::new(value).px(14.0).color(theme.text.primary).w_full())
    }

    fn skeleton_card(theme: &AppTheme) -> Div {
        Self::card_surface(12.0, theme)
            .align(HorizontalAlignment::Leading)
            .child(Self::skeleton_fact_row(tr("blocks_why_label"), 220.0, theme))
            .child(Self::skeleton_fact_row(tr("blocks_target_label"), 160.0, theme))
            .child(
                hstack(8.0)
                    .w_full()
                    .child(div().flex_none().size(px(12.0)).invisible())
                    .child(Self::skeleton_value(13.0, 96.0)),
            )
    }

    fn skeleton_fact_row(label: String, width: f32, theme: &AppTheme) -> Div {
        vstack(4.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(MainText::new(label).px(12.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.secondary))
            .child(Self::skeleton_value(14.0, width))
    }

    fn skeleton_value(px_size: f32, width: f32) -> Div {
        div()
            .relative()
            .w_full()
            .child(
                div()
                    .invisible()
                    .child(MainText::new(" ").px(px_size).font_weight(FontWeight::SEMIBOLD)),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .child(SkeletonLine::new(width, px_size)),
            )
    }

    fn failed_card(&self, message: &str, theme: &AppTheme) -> Div {
        let auth = self.auth.clone();
        Self::card_surface(10.0, theme)
            .child(SecondaryText::new(message.to_owned()).px(13.0).tac(TextAlignT::Center).w_full())
            .child(
                PressableUi::new("blocked-account-retry")
                    .on_press(move |_, cx| auth.update(cx, |auth, cx| auth.reconcile_account_block(cx)))
                    .child(
                        div().px(px(16.0)).py(px(8.0)).child(
                            MainText::new(tr("retry"))
                                .px(14.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.primary._100),
                        ),
                    ),
            )
    }

    fn appeal_row(&self, block: Option<&RiseStoredUserBlock>, theme: &AppTheme, cx: &App) -> Div {
        let row = hstack(0.0).w_full().flex_none().child(spacer_min_w(0.0));
        let lookup = self.appeal_lookup(cx);
        let result = self.appeal_result.as_ref().filter(|result| result.lookup == lookup);

        let row = match (block, result) {
            (Some(block), _) if block.allows_appeal() => {
                let block = block.clone();
                row.child(
                    PressableUi::new("blocked-account-appeal")
                        .on_press(move |_, cx| {
                            AppealSheetStore::shared(cx)
                                .update(cx, |store, cx| store.present_block(block.clone(), cx));
                        })
                        .child(
                            div().px(px(20.0)).py(px(12.0)).child(
                                MainText::new(tr("appeal_submit"))
                                    .px(13.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.text.primary),
                            ),
                        ),
                )
            }
            (Some(_), Some(result)) => row.child(
                div().py(px(12.0)).child(
                    SecondaryText::new(tr("appeal_sheet_title") + ": " + &tr(result.status.i18n_key()))
                        .px(13.0)
                        .tac(TextAlignT::Center),
                ),
            ),
            _ => row.child(
                div().py(px(12.0)).invisible().child(
                    MainText::new(tr("appeal_submit")).px(13.0).font_weight(FontWeight::SEMIBOLD),
                ),
            ),
        };
        row.child(spacer_min_w(0.0))
    }
}

impl Render for BlockedAccountPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let presentation = self.presentation.read(cx);
        let block = presentation.active_full_block();
        let state = presentation.my_blocks_state().clone();
        let is_loading =
            block.is_none() && matches!(state, ModerationLoadState::Idle | ModerationLoadState::Loading);
        let is_logging_out = self.auth.read(cx).is_logging_out();
        let can_play_panda = LottieView::can_play(Self::PANDA_POSE, cx);

        let page = cx.weak_entity();
        let auth = self.auth.clone();

        BgWrapperUi::new().child(
            vstack(0.0)
                .size_full()
                .child(
                    div()
                        .relative()
                        .w_full()
                        .flex_1()
                        .min_h_0()
                        .child(measure("blocked-account-viewport", move |bounds, _, cx| {
                            let height = f32::from(bounds.size.height);
                            let _ = page.update(cx, |this, cx| {
                                if this.viewport_height != height {
                                    this.viewport_height = height;
                                    cx.notify();
                                }
                            });
                        }))
                        .child(
                            scroll_view("blocked-account-scroll").shows_indicators(false).size_full().child(
                                vstack(0.0)
                                    .w_full()
                                    .min_h(px(self.viewport_height))
                                    .child(spacer_min_h(0.0))
                                    .child(self.content(block.as_ref(), &state, is_loading, can_play_panda, &theme))
                                    .child(spacer_min_h(0.0)),
                            ),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .max_w(px(Self::COLUMN_MAX_WIDTH))
                        .mx_auto()
                        .flex_none()
                        .px(px(24.0))
                        .pt(px(16.0))
                        .pb(px(20.0))
                        .child(
                        ButtonUi::new("blocked-account-logout")
                            .disabled(is_logging_out)
                            .is_pending(is_logging_out)
                            .background_color(theme.primary._100)
                            .height(50.0)
                            .loader_color(theme.content_on(theme.primary._100))
                            .on_press(move |_, cx| auth.update(cx, |auth, cx| auth.logout(cx)))
                            .child(
                                MainText::new(tr("settings_logout_title"))
                                    .px(16.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.content_on(theme.primary._100)),
                            ),
                    ),
                )
                .child(self.appeal_row(block.as_ref(), &theme, cx)),
        )
    }
}
