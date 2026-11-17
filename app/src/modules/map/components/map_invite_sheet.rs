use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, FontWeight, Hsla, IntoElement, ParentElement, Render, RenderOnce,
    SharedString, Styled, Subscription, Window, div, px, transparent_black, white,
};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::error_text_ui::system_red;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonCircle, SkeletonLine};
use rise_ui::swiftui::prelude::*;
use rise_ui::{ButtonUi, IconUi, MainText, TextAlignT};

use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::map::stores::map_actions::map_types::{
    MapInvitePreview, MapInviteRelation, MapInviteSheetState, MapInviteSheetStatePhase,
};
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;

type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;
type OpenMapAction = Rc<dyn Fn(&mut App)>;

/// sheet behind a mf token link, mounted once at the app root like the chat join sheet
/// a link only sends the owner a normal friend request
/// readers not on the map go through onboarding first
pub struct MapInviteSheetHost {
    on_open_map: OpenMapAction,
    interactions: Entity<MapInteractionsStore>,
    closing_state: Option<MapInviteSheetState>,
    _interactions_observation: Subscription,
}

impl MapInviteSheetHost {
    pub fn new(on_open_map: impl Fn(&mut App) + 'static, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let interactions = MapInteractionsStore::shared(cx);
        let closing_state = interactions.read(cx).invite_sheet.clone();
        let interactions_observation = cx.observe(&interactions, |this, interactions, cx| {
            if let Some(state) = interactions.read(cx).invite_sheet.clone()
                && this.closing_state.as_ref() != Some(&state)
            {
                this.closing_state = Some(state);
            }
            cx.notify();
        });
        Self {
            on_open_map: Rc::new(on_open_map),
            interactions,
            closing_state,
            _interactions_observation: interactions_observation,
        }
    }
}

impl Render for MapInviteSheetHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.interactions.read(cx).invite_sheet.clone().or_else(|| self.closing_state.clone());
        let reader = self.interactions.clone();
        let writer = self.interactions.clone();
        let is_presented = Binding::new(
            move |cx| reader.read(cx).invite_sheet.is_some(),
            move |presented, _, cx| {
                if !presented {
                    writer.update(cx, |interactions, cx| interactions.dismiss_invite_sheet(cx));
                }
            },
        );
        let retry = self.interactions.clone();
        let submit = self.interactions.clone();
        let host = cx.entity().downgrade();
        MapInviteSheet::new(
            is_presented,
            state,
            move |_, cx| retry.update(cx, |interactions, cx| interactions.retry_invite_preview(cx)),
            move |_, cx| submit.update(cx, |interactions, cx| interactions.submit_invite(cx)),
        )
        .on_dismiss(move |_, cx| {
            let _ = host.update(cx, |this, cx| {
                if this.interactions.read(cx).invite_sheet.is_none() {
                    this.closing_state = None;
                    cx.notify();
                }
                let on_open_map = this.on_open_map.clone();
                this.interactions.update(cx, |interactions, cx| {
                    interactions.invite_sheet_did_dismiss(move |cx| on_open_map(cx), cx);
                });
            });
        })
    }
}

#[derive(IntoElement)]
pub struct MapInviteSheet {
    is_presented: Binding<bool>,
    state: Option<MapInviteSheetState>,
    on_retry: SheetAction,
    on_submit: SheetAction,
    on_dismiss: SheetAction,
    full_page: bool,
}

impl MapInviteSheet {
    pub fn new(
        is_presented: Binding<bool>,
        state: Option<MapInviteSheetState>,
        on_retry: impl Fn(&mut Window, &mut App) + 'static,
        on_submit: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            is_presented,
            state,
            on_retry: Rc::new(on_retry),
            on_submit: Rc::new(on_submit),
            on_dismiss: Rc::new(|_, _| {}),
            full_page: true,
        }
    }

    pub fn on_dismiss(mut self, on_dismiss: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Rc::new(on_dismiss);
        self
    }

    pub fn full_page(mut self, full_page: bool) -> Self {
        self.full_page = full_page;
        self
    }
}

impl RenderOnce for MapInviteSheet {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let is_submitting = self.state.as_ref().is_some_and(|state| state.is_submitting);
        let content_state = self.state.clone();
        let footer_state = self.state;
        let on_retry = self.on_retry;
        let on_submit = self.on_submit;
        let on_dismiss = self.on_dismiss;
        let refused = self.is_presented.clone();
        BottomSheetUi::new(
            "map-invite-sheet",
            self.is_presented,
            move |_, _| match content_state.clone() {
                Some(state) => {
                    div().pt(px(18.)).pb(px(8.)).child(MapInviteSheetContent::new(state)).into_any_element()
                }
                None => div().into_any_element(),
            },
            move |_, _| match footer_state.clone() {
                Some(state) => div()
                    .w_full()
                    .pb(px(12.))
                    .child(MapInviteFooter { state, on_retry: on_retry.clone(), on_submit: on_submit.clone() })
                    .into_any_element(),
                None => div().into_any_element(),
            },
        )
        .snap_fractions(vec![0.46, 0.78])
        .fits_content(true)
        .full_page(self.full_page)
        .full_page_allows_swipe_dismiss(!is_submitting)
        .dismisses_on_backdrop_tap(!is_submitting)
        .footer_shows_top_border(false)
        .footer_overlays_content(Some(false))
        .footer_horizontal_padding(Some(14.0))
        .footer_top_padding(Some(8.0))
        .content_scrollable(true)
        .full_page_interaction_locked(is_submitting)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(transparent_black()))
        .on_dismiss(move |window, cx| on_dismiss(window, cx))
        .on_presentation_refused(move |window, cx| refused.set(false, window, cx))
    }
}

#[derive(IntoElement)]
pub struct MapInviteSheetContent {
    state: MapInviteSheetState,
}

impl MapInviteSheetContent {
    pub fn new(state: MapInviteSheetState) -> Self {
        Self { state }
    }

    /// same boxes as the loaded state so the sheet keeps its height when the preview lands
    fn loading() -> AnyElement {
        div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.))
            .child(div().pt(px(6.)).child(SkeletonCircle::new(88.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(9.))
                    .child(SkeletonLine::new(170., 19.))
                    .child(SkeletonLine::new(230., 13.)),
            )
            .into_any_element()
    }

    fn loaded(&self, preview: &MapInvitePreview, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let owner = &preview.owner;
        let mut column = div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.))
            .child(
                div().pt(px(6.)).child(
                    UserLogoUi::new(SharedString::from(format!("map-invite-owner-{}", owner.id)))
                        .source(SharedString::from(owner.logo.clone()))
                        .name(SharedString::from(owner.display_name().to_owned()))
                        .user_id(SharedString::from(owner.id.clone()))
                        .size(88.),
                ),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(5.))
                    .child(
                        MainText::new(owner.display_name().to_owned())
                            .px(21.)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::BOLD)
                            .number_of_lines(2),
                    )
                    .child(
                        MainText::new(tr(Self::subtitle_key(preview)))
                            .px(14.)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary),
                    ),
            );
        if let Some(error) = self.state.submission_error.as_ref().filter(|error| !error.is_empty()) {
            column = column.child(
                MainText::new(error.clone()).px(14.).tac(TextAlignT::Center).color(system_red(cx)),
            );
        }
        column.into_any_element()
    }

    fn failed(text: Option<&str>, theme: &AppTheme) -> AnyElement {
        div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.))
            .child(
                div().pt(px(6.)).child(
                    div()
                        .size(px(88.))
                        .rounded_full()
                        .bg(theme.text.secondary.opacity(0.14))
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(IconUi::sized("link.badge.plus", px(34.), theme.text.secondary, false)),
                ),
            )
            .child(
                MainText::new(text.map_or_else(|| tr("join_link_invalid"), str::to_owned))
                    .px(15.)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary),
            )
            .into_any_element()
    }

    fn subtitle_key(preview: &MapInvitePreview) -> &'static str {
        if !preview.viewer_is_member
            && preview.relation != MapInviteRelation::Own
            && preview.relation != MapInviteRelation::Requested
        {
            return "map_invite_join_first_subtitle";
        }
        match preview.relation {
            MapInviteRelation::Own => "map_invite_own_subtitle",
            MapInviteRelation::Friends => "map_invite_friends_subtitle",
            MapInviteRelation::Requested => "map_invite_requested_subtitle",
            MapInviteRelation::Incoming => "map_invite_incoming_subtitle",
            MapInviteRelation::None => {
                if preview.viewer_is_member {
                    "map_invite_subtitle"
                } else {
                    "map_invite_join_first_subtitle"
                }
            }
        }
    }
}

impl RenderOnce for MapInviteSheetContent {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let body = match &self.state.phase {
            MapInviteSheetStatePhase::Loading => Self::loading(),
            MapInviteSheetStatePhase::Preview(preview) => self.loaded(preview, &theme, cx),
            MapInviteSheetStatePhase::Failed(text) => Self::failed(text.as_deref(), &theme),
        };
        div().w_full().flex().flex_col().px(px(22.)).pt(px(10.)).pb(px(8.)).child(body)
    }
}

#[derive(IntoElement)]
struct MapInviteFooter {
    state: MapInviteSheetState,
    on_retry: SheetAction,
    on_submit: SheetAction,
}

impl MapInviteFooter {
    fn action_key(preview: &MapInvitePreview) -> &'static str {
        if !preview.viewer_is_member && preview.relation != MapInviteRelation::Requested {
            return if preview.relation == MapInviteRelation::Own {
                "map_join_action"
            } else {
                "map_invite_join_and_add"
            };
        }
        match preview.relation {
            MapInviteRelation::Own | MapInviteRelation::Friends => "map_invite_open_map",
            MapInviteRelation::Requested => "map_request_cancel",
            MapInviteRelation::Incoming => "map_request_accept",
            MapInviteRelation::None => {
                if preview.viewer_is_member {
                    "map_invite_add"
                } else {
                    "map_invite_join_and_add"
                }
            }
        }
    }

    fn label(text: String, color: Hsla) -> MainText {
        MainText::new(text).px(16.).font_weight(FontWeight::SEMIBOLD).color(color).number_of_lines(1)
    }
}

impl RenderOnce for MapInviteFooter {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        match &self.state.phase {
            MapInviteSheetStatePhase::Loading => SkeletonBlock::new(None::<f32>, 50., 25.).into_any_element(),
            MapInviteSheetStatePhase::Preview(preview) => {
                let requested = preview.relation == MapInviteRelation::Requested;
                let on_submit = self.on_submit.clone();
                let mut button = ButtonUi::new(ElementId::Name("map-invite-submit".into()))
                    .is_pending(self.state.is_submitting)
                    .height(50.)
                    .loader_color(white())
                    .on_press(move |window, cx| on_submit(window, cx));
                if requested {
                    button = button.background_color(theme.text.secondary.opacity(0.18));
                }
                let color = if requested { theme.text.primary } else { white() };
                button.child(Self::label(tr(Self::action_key(preview)), color)).into_any_element()
            }
            MapInviteSheetStatePhase::Failed(_) => {
                let on_retry = self.on_retry.clone();
                ButtonUi::new(ElementId::Name("map-invite-retry".into()))
                    .height(50.)
                    .on_press(move |window, cx| on_retry(window, cx))
                    .child(Self::label(tr("common_retry"), white()))
                    .into_any_element()
            }
        }
    }
}

