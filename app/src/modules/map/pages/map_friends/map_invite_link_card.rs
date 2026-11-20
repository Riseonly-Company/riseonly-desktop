use gpui::{App, FontWeight, Window, white};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_ui::button_ui::ButtonUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::skeleton_ui::SkeletonLine;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::modules::map::shared::map_invite_link::MapInviteLink;
use crate::modules::map::stores::map_actions::map_types::MapInvite;
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;

/// reader's personal map link, following it sends them a friend request
/// resetting it invalidates every copy handed out
#[derive(IntoElement)]
pub struct MapInviteLinkCard {
    invite: Option<MapInvite>,
}

impl MapInviteLinkCard {
    pub fn new(invite: Option<MapInvite>) -> Self {
        Self { invite }
    }

    fn present_reset_confirmation(cx: &mut App) {
        NativeConfirmAlert::present_delete(
            &tr("map_invite_reset_title"),
            &tr("map_invite_reset_message"),
            Some(tr("map_invite_reset_action")),
            None,
            false,
            None,
            None,
            |cx| {
                MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.reset_invite(cx));
            },
            cx,
        );
    }
}

impl RenderOnce for MapInviteLinkCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let is_disabled = self.invite.is_none();
        let link_line = match self.invite.as_ref() {
            Some(invite) => MainText::new(MapInviteLink::display_text(&invite.token))
                .px(13.0)
                .color(theme.text.secondary)
                .number_of_lines(1)
                .into_any_element(),
            None => div().py(px(2.0)).child(SkeletonLine::new(190.0, 12.0)).into_any_element(),
        };
        let share_invite = self.invite.clone();
        let copy_invite = self.invite;

        Shape::continuous(22.0).round(
            vstack(12.0)
                .align(HorizontalAlignment::Leading)
                .w_full()
                .p(px(16.0))
                .bg(theme.bg._200)
                .child(
                    hstack(12.0)
                        .w_full()
                        .items_center()
                        .child(
                            div()
                                .size(px(42.0))
                                .flex_none()
                                .rounded_full()
                                .bg(theme.primary._100)
                                .flex()
                                .items_center()
                                .justify_center()
                                .children(IconUi::sized("link", px(17.0), theme.content_on(theme.primary._100), false)),
                        )
                        .child(
                            vstack(3.0)
                                .align(HorizontalAlignment::Leading)
                                .min_w_0()
                                .child(
                                    MainText::new(tr("map_invite_card_title"))
                                        .px(16.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .number_of_lines(1),
                                )
                                .child(link_line),
                        )
                        .child(spacer_min_w(0.0)),
                )
                .child(MainText::new(tr("map_invite_card_hint")).px(13.0).color(theme.text.secondary))
                .child(
                    hstack(8.0)
                        .w_full()
                        .items_center()
                        .child(
                            div().flex_1().min_w_0().child(
                                ButtonUi::new("map-invite-share")
                                    .disabled(is_disabled)
                                    .height(40.0)
                                    .on_press(move |_, cx| {
                                        if let Some(invite) = share_invite.as_ref() {
                                            MapInteractionsStore::shared(cx)
                                                .update(cx, |interactions, cx| interactions.share_invite(invite, cx));
                                        }
                                    })
                                    .child(
                                        MainText::new(tr("map_invite_share"))
                                            .px(15.0)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .color(white())
                                            .number_of_lines(1)
                                            .minimum_scale_factor(0.6),
                                    )
                                    .guide_anchor(RiseGuideAnchor::MapInviteShare),
                            ),
                        )
                        .child(
                            div().flex_1().min_w_0().child(
                                ButtonUi::new("map-invite-copy")
                                    .disabled(is_disabled)
                                    .background_color(theme.text.secondary.opacity(0.16))
                                    .height(40.0)
                                    .on_press(move |_, cx| {
                                        if let Some(invite) = copy_invite.as_ref() {
                                            MapInteractionsStore::shared(cx)
                                                .update(cx, |interactions, cx| interactions.copy_invite(invite, cx));
                                        }
                                    })
                                    .child(
                                        MainText::new(tr("copy_link"))
                                            .px(15.0)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .color(theme.text.primary)
                                            .number_of_lines(1)
                                            .minimum_scale_factor(0.6),
                                    ),
                            ),
                        )
                        .child(
                            ButtonUi::new("map-invite-reset")
                                .disabled(is_disabled)
                                .background_color(theme.text.secondary.opacity(0.16))
                                .height(40.0)
                                .fit_content(true)
                                .padding_horizontal(14.0)
                                .on_press(|_, cx| MapInviteLinkCard::present_reset_confirmation(cx))
                                .child(
                                    div()
                                        .id("map-invite-reset-face")
                                        .aria_label(tr("map_invite_reset_action"))
                                        .tooltip_label(tr("map_invite_reset_action"))
                                        .children(IconUi::sized(
                                            "arrow.triangle.2.circlepath",
                                            px(14.0),
                                            theme.text.primary,
                                            false,
                                        )),
                                ),
                        ),
                ),
        )
    }
}
