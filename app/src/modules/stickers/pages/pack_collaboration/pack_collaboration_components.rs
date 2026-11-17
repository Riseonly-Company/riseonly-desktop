use std::rc::Rc;

use gpui::{AnyElement, App, ElementId, FontWeight, Hsla, SharedString, Window};
use rise_i18n::tr;
use rise_canon::rise_proto::stickers::ProtoPackUserRef;
use rise_theme::AppTheme;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::user_logo_ui::UserLogoUi;

pub type RisePackUserRef = ProtoPackUserRef;

/// label of a user in the pack collaboration screens
pub enum PackCollaborationUserLabel {}

impl PackCollaborationUserLabel {
    pub fn display_name(user: Option<&RisePackUserRef>) -> String {
        let Some(user) = user else {
            return tr("chat_deleted_account");
        };
        if !user.name.is_empty() {
            return user.name.clone();
        }
        if !user.tag.is_empty() {
            return format!("@{}", user.tag);
        }
        tr("chat_deleted_account")
    }
}

/// one person of a pack,
/// avatar and premium name and tag and a trailing slot for time or role or chevron
#[derive(IntoElement)]
pub struct PackCollaborationUserRow {
    id: ElementId,
    user: Option<RisePackUserRef>,
    user_id: String,
    subtitle: Option<String>,
    on_tap: ButtonAction,
    allows_hit_testing: bool,
    trailing: Vec<AnyElement>,
}

impl PackCollaborationUserRow {
    pub fn new(
        id: impl Into<ElementId>,
        user: Option<RisePackUserRef>,
        user_id: String,
        on_tap: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            user,
            user_id,
            subtitle: None,
            on_tap: Rc::new(on_tap),
            allows_hit_testing: true,
            trailing: Vec::new(),
        }
    }

    /// turns hit testing off where a hold container owns the taps the row button would swallow
    pub fn allows_hit_testing(mut self, allows: bool) -> Self {
        self.allows_hit_testing = allows;
        self
    }

    pub fn subtitle(mut self, subtitle: Option<String>) -> Self {
        self.subtitle = subtitle;
        self
    }

    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing.push(element.into_any_element());
        self
    }

    fn tag_line(&self) -> Option<String> {
        let tag = &self.user.as_ref()?.tag;
        (!tag.is_empty()).then(|| format!("@{tag}"))
    }

    fn name_color(&self, theme: &AppTheme) -> Hsla {
        self.user
            .as_ref()
            .filter(|user| user.is_premium)
            .and_then(|user| PremiumNameColor::parse(user.name_color.as_deref()))
            .unwrap_or(theme.text.primary)
    }
}

impl RenderOnce for PackCollaborationUserRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let mut name = hstack(6.0).child(
            MainText::new(PackCollaborationUserLabel::display_name(self.user.as_ref()))
                .px(16.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(self.name_color(&theme))
                .number_of_lines(1),
        );
        if let Some(user) = self.user.as_ref().filter(|user| user.is_premium) {
            name = name.child(
                PremiumBadge::new()
                    .user_id(Some(SharedString::from(user.id.clone())))
                    .emoji(user.premium_badge_emoji.clone().map(SharedString::from))
                    .size(14.0),
            );
        }
        let mut identity = vstack(3.0).align(HorizontalAlignment::Leading).min_w_0().child(name);
        if let Some(line) = self.subtitle.clone().or_else(|| self.tag_line()) {
            identity = identity.child(MainText::new(line).px(13.0).color(theme.text.secondary).number_of_lines(1));
        }
        let (logo, user_name) =
            self.user.as_ref().map_or((String::new(), String::new()), |user| (user.logo.clone(), user.name.clone()));
        let content = hstack(12.0)
            .w_full()
            .px(px(14.0))
            .py(px(10.0))
            .child(
                UserLogoUi::new(SharedString::from(format!("pack-user-logo-{}", self.user_id)))
                    .source(SharedString::from(logo))
                    .name(SharedString::from(user_name))
                    .user_id(SharedString::from(self.user_id.clone()))
                    .size(46.0),
            )
            .child(identity)
            .child(spacer_min_w(8.0))
            .children(self.trailing);
        if !self.allows_hit_testing {
            return content.into_any_element();
        }
        let on_tap = self.on_tap;
        plain(self.id).on_press(move |window, cx| on_tap(window, cx)).child(content).into_any_element()
    }
}

/// hairline between rows of a grouped card, inset past the avatar
#[derive(IntoElement)]
pub struct PackCollaborationRowDivider;

impl PackCollaborationRowDivider {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for PackCollaborationRowDivider {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        div().w_full().pl(px(70.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55)))
    }
}
