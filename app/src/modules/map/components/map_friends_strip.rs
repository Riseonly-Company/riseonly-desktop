use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, FontWeight, SharedString, Window};
use rise_canon::RiseGuideAnchor;
use rise_i18n::{tr, tr_plural};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::icon_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;
use rise_ui::unread_count_badge_ui::UnreadCountBadgeUi;

use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::map::engine::rise_map_reactions_repository::MapReactionInboxEntry;
use crate::modules::map::stores::map_actions::map_types::{MapLiveFriend, MapState};

/// member's bottom panel, friend graph button plus the friends currently on the map
#[derive(IntoElement)]
pub struct MapFriendsStrip {
    state: MapState,
    friends: Vec<MapLiveFriend>,
    reaction_inbox: Vec<MapReactionInboxEntry>,
    on_open_friends: Rc<dyn Fn(&mut Window, &mut App)>,
    on_focus: Rc<dyn Fn(String, &mut Window, &mut App)>,
}

impl MapFriendsStrip {
    const INSET: f32 = 12.0;
    const AVATAR_SIDE: f32 = 46.0;

    pub fn new(
        state: MapState,
        friends: Vec<MapLiveFriend>,
        reaction_inbox: Vec<MapReactionInboxEntry>,
        on_open_friends: impl Fn(&mut Window, &mut App) + 'static,
        on_focus: impl Fn(String, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { state, friends, reaction_inbox, on_open_friends: Rc::new(on_open_friends), on_focus: Rc::new(on_focus) }
    }

    fn subtitle(&self) -> String {
        if !self.state.settings.sharing_enabled {
            return tr("map_sharing_paused");
        }
        if self.state.friends_count == 0 {
            tr("map_friends_invite_hint")
        } else {
            tr_plural("map_friends_count", self.state.friends_count)
        }
    }
}

impl RenderOnce for MapFriendsStrip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let mut reaction_counts: HashMap<&str, i64> = HashMap::new();
        for entry in &self.reaction_inbox {
            let count = reaction_counts.entry(entry.sender_id.as_str()).or_insert(entry.count);
            *count = (*count).max(entry.count);
        }

        let on_open_friends = self.on_open_friends.clone();
        let open_button = plain("map-friends-open")
            .on_press(move |window, cx| on_open_friends(window, cx))
            .child(
                div()
                    .id("map-friends-open-face")
                    .aria_label(tr("map_friends_title"))
                    .tooltip_label(tr("map_friends_title"))
                    .relative()
                    .size(px(Self::AVATAR_SIDE))
                    .flex_none()
                    .child(
                        div()
                            .size_full()
                            .rounded_full()
                            .bg(theme.primary._100)
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized("plus", px(18.0), theme.content_on(theme.primary._100), false)),
                    )
                    .when(self.state.incoming_requests_count > 0, |button| {
                        button.child(
                            div()
                                .absolute()
                                .top(px(-4.0))
                                .right(px(-5.0))
                                .child(UnreadCountBadgeUi::new(self.state.incoming_requests_count)),
                        )
                    }),
            )
            .guide_anchor(RiseGuideAnchor::MapFriendsOpen);

        let mut row = hstack(12.0).w_full().min_h(px(Self::AVATAR_SIDE)).items_center().child(open_button);
        if self.friends.is_empty() {
            row = row
                .child(
                    vstack(3.0)
                        .items_start()
                        .child(
                            MainText::new(tr("map_friends_title"))
                                .px(16.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .number_of_lines(1),
                        )
                        .child(MainText::new(self.subtitle()).px(13.0).color(theme.text.secondary).number_of_lines(1)),
                )
                .child(spacer_min_w(0.0));
        } else {
            let first_id = self.friends.first().map(|friend| friend.id().to_owned());
            let mut avatars = div()
                .id("map-friends-strip-row")
                .flex_1()
                .min_w_0()
                .h(px(Self::AVATAR_SIDE))
                .flex()
                .flex_row()
                .gap(px(10.0))
                .overflow_x_scroll();
            for friend in self.friends {
                let unread_count = reaction_counts.get(friend.id()).copied().unwrap_or(0);
                let on_focus = self.on_focus.clone();
                let friend_id = friend.id().to_owned();
                let anchor = (first_id.as_deref() == Some(friend.id())).then_some(RiseGuideAnchor::MapFriendsStripFirst);
                avatars = avatars.child(
                    plain(SharedString::from(format!("map-friends-strip-{}", friend.id())))
                        .on_press(move |window, cx| on_focus(friend_id.clone(), window, cx))
                        .child(
                            div()
                                .id(SharedString::from(format!("map-friends-strip-face-{}", friend.id())))
                                .aria_label(friend.user.display_name().to_owned())
                                .tooltip_label(friend.user.display_name().to_owned())
                                .relative()
                                .flex_none()
                                .size(px(Self::AVATAR_SIDE))
                                .child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .opacity(if friend.position.is_none() { 0.45 } else { 1.0 })
                                        .child(
                                            UserLogoUi::new(SharedString::from(format!(
                                                "map-friends-strip-logo-{}",
                                                friend.id()
                                            )))
                                            .source(SharedString::from(friend.user.logo.clone()))
                                            .name(SharedString::from(friend.user.display_name().to_owned()))
                                            .user_id(SharedString::from(friend.user.id.clone()))
                                            .size(Self::AVATAR_SIDE),
                                        ),
                                )
                                .child(div().absolute().bottom_0().right_0().child(UnreadCountBadgeUi::new(unread_count))),
                        )
                        .guide_anchor(anchor),
                );
            }
            row = row.child(avatars);
        }

        row.p(px(Self::INSET)).liquid_chrome_capsule()
    }
}
