use gpui::{App, Entity};
use rise_i18n::{FormatArg, tr_args};
use rise_platform::photo_library::{LibraryAsset, LibraryAssetKind};

use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions};
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellInteractionsStore;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::selection::media_selection_state::MediaSelectionState;

impl MediaPickerConfiguration {
    pub fn announce_picked_overflow(
        &self,
        refused: &LibraryAsset,
        selection: &Entity<MediaSelectionState>,
        cx: &mut App,
    ) {
        if self.premium_selection_ceiling.is_some() {
            self.offer_more_if_count_refused(refused, selection, cx);
            return;
        }
        let message_key = self.selection_limit_message_key.as_deref().unwrap_or("share_error_too_many_items");
        Notifier::send(tr_args(message_key, &[FormatArg::Int(self.max_selection_count)]), NotifierSendOptions::default(), cx);
    }

    /// only the count ceiling is a tier, a kind lock or video ceiling is the same on premium
    /// shared by grid and editor so a refused pick never fails silently
    pub fn offer_more_if_count_refused(
        &self,
        asset: &LibraryAsset,
        selection: &Entity<MediaSelectionState>,
        cx: &mut App,
    ) {
        let selection = selection.read(cx);
        let is_kind_locked = selection.is_kind_locked(asset);
        let is_video_ceiling_reached = selection.is_video_ceiling_reached(asset);
        let max_reached = selection.max_reached();
        if self.premium_selection_ceiling.is_none()
            && let Some(message_key) = &self.selection_limit_message_key
            && !selection.contains(&asset.id)
            && !is_kind_locked
            && !is_video_ceiling_reached
            && max_reached
        {
            let title = tr_args(message_key, &[FormatArg::Int(self.max_selection_count)]);
            Notifier::send(title, NotifierSendOptions::default(), cx);
            return;
        }
        let Some(premium_ceiling) = self.premium_selection_ceiling else {
            return;
        };
        if selection.locked_kind() == Some(LibraryAssetKind::Video)
            || is_kind_locked
            || is_video_ceiling_reached
            || !max_reached
        {
            return;
        }
        let maximum = self.max_selection_count;
        PremiumUpsellInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.offer_client_ceiling("media_picker_selection", maximum, Some(premium_ceiling), cx);
        });
    }
}
