use std::rc::Rc;
use std::sync::Arc;

use gpui::{AnyElement, App, Corners, Pixels, RenderImage};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;

use crate::message_bubble_geometry::{MessageBubbleRadii, MessageBubbleShape, MessageBubbleTailSide};
use crate::swiftui::environment::{EnvKey, env};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HoldContextMenuAlignment {
    Leading,
    Trailing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HoldContextMenuAction {
    pub id: String,
    pub title: String,
    pub system_image: Option<String>,
    pub destructive: bool,
    pub disabled: bool,
    pub section_break: bool,
    pub dismisses_menu: bool,
    pub submenu: Option<Vec<HoldContextMenuAction>>,
    pub guide_anchor: Option<RiseGuideAnchor>,
    pub checked: bool,
    pub leading_image: Option<Arc<RenderImage>>,
    pub subtitle: Option<String>,
    pub leading_image_corner_radius: Option<f32>,
}

impl HoldContextMenuAction {
    /// new action with every defaulted field at its default, builders set the rest
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            system_image: None,
            destructive: false,
            disabled: false,
            section_break: false,
            dismisses_menu: true,
            submenu: None,
            guide_anchor: None,
            checked: false,
            leading_image: None,
            subtitle: None,
            leading_image_corner_radius: None,
        }
    }

    pub fn system_image(mut self, system_image: impl Into<String>) -> Self {
        self.system_image = Some(system_image.into());
        self
    }

    pub fn destructive(mut self, destructive: bool) -> Self {
        self.destructive = destructive;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn section_break(mut self, section_break: bool) -> Self {
        self.section_break = section_break;
        self
    }

    pub fn dismisses_menu(mut self, dismisses_menu: bool) -> Self {
        self.dismisses_menu = dismisses_menu;
        self
    }

    pub fn submenu(mut self, submenu: Vec<HoldContextMenuAction>) -> Self {
        self.submenu = Some(submenu);
        self
    }

    pub fn guide_anchor(mut self, guide_anchor: impl Into<Option<RiseGuideAnchor>>) -> Self {
        self.guide_anchor = guide_anchor.into();
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn leading_image(mut self, leading_image: impl Into<Option<Arc<RenderImage>>>) -> Self {
        self.leading_image = leading_image.into();
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<Option<String>>) -> Self {
        self.subtitle = subtitle.into();
        self
    }

    pub fn leading_image_corner_radius(mut self, radius: impl Into<Option<f32>>) -> Self {
        self.leading_image_corner_radius = radius.into();
        self
    }
}

/// trailing view under the source, its details page replaces the action list
#[derive(Clone)]
pub struct HoldContextMenuAccessory {
    pub summary_height: f32,
    pub on_prefetch: Rc<dyn Fn(&mut App)>,
    pub summary_view: Rc<dyn Fn(Rc<dyn Fn(&mut App)>) -> AnyElement>,
    pub details_view: Rc<dyn Fn(Rc<dyn Fn(&mut App)>, Rc<dyn Fn(String, &mut App)>) -> AnyElement>,
}

impl HoldContextMenuAccessory {
    pub fn new(
        summary_height: f32,
        summary_view: Rc<dyn Fn(Rc<dyn Fn(&mut App)>) -> AnyElement>,
        details_view: Rc<dyn Fn(Rc<dyn Fn(&mut App)>, Rc<dyn Fn(String, &mut App)>) -> AnyElement>,
    ) -> Self {
        Self {
            summary_height,
            on_prefetch: Rc::new(|_| {}),
            summary_view,
            details_view,
        }
    }

    pub fn on_prefetch(mut self, on_prefetch: Rc<dyn Fn(&mut App)>) -> Self {
        self.on_prefetch = on_prefetch;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HoldContextMenuEmojiPlacement {
    Top,
    Bottom,
}

pub enum HoldContextMenuDefaults {}

impl HoldContextMenuDefaults {
    pub const EMOJIS: [&'static str; 7] = ["❤️", "🤝", "👍", "🥰", "🤯", "🔥", "👎"];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoldSourceBubbleClip {
    pub top_leading: Pixels,
    pub top_trailing: Pixels,
    pub bottom_leading: Pixels,
    pub bottom_trailing: Pixels,
    pub tail_side: MessageBubbleTailSide,
    pub draws_tail: bool,
}

impl HoldSourceBubbleClip {
    /// corner radii with no tail by default, with_tail sets it
    pub fn new(top_leading: Pixels, top_trailing: Pixels, bottom_leading: Pixels, bottom_trailing: Pixels) -> Self {
        Self::with_tail(
            top_leading,
            top_trailing,
            bottom_leading,
            bottom_trailing,
            MessageBubbleTailSide::None,
            false,
        )
    }

    pub fn with_tail(
        top_leading: Pixels,
        top_trailing: Pixels,
        bottom_leading: Pixels,
        bottom_trailing: Pixels,
        tail_side: MessageBubbleTailSide,
        draws_tail: bool,
    ) -> Self {
        Self {
            top_leading,
            top_trailing,
            bottom_leading,
            bottom_trailing,
            tail_side,
            draws_tail,
        }
    }

    /// corner radii as gpui rounds them, left is leading
    pub fn uneven_shape(&self) -> Corners<Pixels> {
        Corners {
            top_left: self.top_leading,
            top_right: self.top_trailing,
            bottom_right: self.bottom_trailing,
            bottom_left: self.bottom_leading,
        }
    }

    pub fn bubble_shape(&self) -> MessageBubbleShape {
        MessageBubbleShape {
            radii: MessageBubbleRadii {
                top_leading: self.top_leading,
                top_trailing: self.top_trailing,
                bottom_leading: self.bottom_leading,
                bottom_trailing: self.bottom_trailing,
            },
            tail_side: self.tail_side,
            draws_tail: self.draws_tail,
        }
    }
}

pub enum HoldContextSocialMenu {}

impl HoldContextSocialMenu {
    pub fn user_list_actions() -> Vec<HoldContextMenuAction> {
        Self::user_list_actions_viewer_blocked(false)
    }

    /// actions for a user list, depends on whether the viewer is blocked
    pub fn user_list_actions_viewer_blocked(viewer_blocked: bool) -> Vec<HoldContextMenuAction> {
        vec![
            HoldContextMenuAction::new("profile", tr("ctx_open_profile")).system_image("person.crop.circle"),
            HoldContextMenuAction::new("message", tr("ctx_message")).system_image("bubble.left.and.bubble.right"),
            HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"),
            HoldContextMenuAction::new("mention", tr("ctx_mention")).system_image("at"),
            if viewer_blocked {
                HoldContextMenuAction::new("unblock", tr("contact_unblock_action")).system_image("hand.raised.slash")
            } else {
                HoldContextMenuAction::new("block", tr("ctx_block"))
                    .system_image("hand.raised.fill")
                    .destructive(true)
            },
            HoldContextMenuAction::new("report", tr("ctx_report"))
                .system_image("exclamationmark.bubble")
                .destructive(true),
        ]
    }

    pub fn group_member_actions() -> Vec<HoldContextMenuAction> {
        let mut a = Self::user_list_actions();
        a.insert(3, HoldContextMenuAction::new("restrict", tr("ctx_restrict")).system_image("nosign"));
        a.insert(
            4,
            HoldContextMenuAction::new("remove", tr("ctx_remove_member"))
                .system_image("person.crop.circle.badge.minus")
                .destructive(true),
        );
        a
    }

    pub fn media_gallery_actions() -> Vec<HoldContextMenuAction> {
        vec![
            HoldContextMenuAction::new("goto", tr("ctx_go_to_message")).system_image("arrow.up.left.and.arrow.down.right"),
            HoldContextMenuAction::new("forward", tr("ctx_forward")).system_image("arrowshape.turn.up.right"),
            HoldContextMenuAction::new("save", tr("ctx_save")).system_image("square.and.arrow.down"),
            HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"),
            HoldContextMenuAction::new("delete", tr("ctx_delete")).system_image("trash").destructive(true),
        ]
    }
}

/// whether a hold menu over this subtree freezes its scrolling
pub struct HoldContextMenuLocksScrollKey;

impl EnvKey for HoldContextMenuLocksScrollKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        false
    }

    fn fingerprint(_value: &Self::Value) -> Option<u64> {
        None
    }
}

pub fn hold_context_menu_locks_scroll(cx: &App) -> bool {
    env::<HoldContextMenuLocksScrollKey>(cx)
}
