use rise_i18n::tr;

use crate::modules::share_extension::shared::rise_share_models::{RiseShareTarget, RiseShareTargetKind};

/// self chat arrives under the owner name and avatar from the gateway, the ui shows it as Saved
pub enum ShareTargetPresentation {}

impl ShareTargetPresentation {
    pub fn title(target: &RiseShareTarget) -> String {
        if target.kind != RiseShareTargetKind::Favourites {
            return target.title.clone();
        }
        tr("chats_favourites")
    }

    pub fn avatar_name(target: &RiseShareTarget) -> String {
        Self::title(target)
    }

    pub fn is_favourites(target: &RiseShareTarget) -> bool {
        target.kind == RiseShareTargetKind::Favourites
    }
}
