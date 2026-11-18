use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, Entity, Global};
use rise_canon::{RiseGuideSignal, RisePrivacyAudience, RisePrivacyRule};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_errors::ErrorCategory;
use rise_i18n::tr;

use crate::core::desktop_platform::rise_app_events::RiseAppEvents;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SAI_WS_DID_CONNECT;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::{
    SaiDebouncer, SaiDebouncerOptions, sai_debounced_action,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::ui::emoji_picker_ui::emoji_picker_catalog::EmojiPickerCatalog;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::MessageReaction;
use crate::modules::user::shared::profile_editor_draft::{ProfileEditorDraft, ProfileEditorDraftStore};
use crate::modules::user::shared::social_platform::SocialLink;
use crate::modules::user::stores::user::user_actions::user_actions::{UpdateProfileActionArgs, UserActionsStore};
use crate::modules::user::stores::user::user_actions::user_types::{
    PrivacyExceptionAction, PrivacyRulePatch, ProfileModel, ProfileSocialLink, UserListContinuation, UserListEntry,
    UserListKind, UserListRelationship, UserRelationshipSearchContract,
};
use crate::modules::user::stores::user::user_actions::user_wire_types::{
    PrivacyExceptionWireDTO, UpdateProfileMoreBody, UpdateProfileSocialLinkBody,
};
use crate::modules::user::stores::user::user_services::user_guide_sandbox_hooks::user_guide_sandbox_hooks;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;
use crate::modules::user::stores::user_profile_photos::user_profile_photos_interactions::user_profile_photos_interactions::UserProfilePhotosInteractionsStore;

pub type UserInteractionsDoneHandler = Rc<dyn Fn(&mut App)>;
pub type UserInteractionsTextHandler = Rc<dyn Fn(String, &mut App)>;

const FOLLOW_TOGGLE_DELAY_MILLISECONDS: i64 = 600;
const FOLLOW_TOGGLE_GROUP_KEY: &str = "follow_toggle";
const FOLLOW_TOGGLE_MAX_CACHE_DATA: i64 = 30;
const LIST_SEARCH_DELAY_MILLISECONDS: i64 = 400;
const LIST_SEARCH_GROUP_KEY: &str = "userListSearch";
const LIST_SEARCH_MAX_CACHE_DATA: i64 = 10;
const SAVE_FAILED_NOTICE_DURATION: f64 = 5.0;

#[derive(Clone)]
struct PrivacyExceptionStep {
    id: String,
    remove: bool,
}

pub struct UserInteractionsStore {
    pub(crate) is_deleting_account: bool,
}

impl UserInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { is_deleting_account: false }
    }

    pub fn actions(cx: &mut App) -> Entity<UserActionsStore> {
        UserActionsStore::shared(cx)
    }

    pub fn service(cx: &mut App) -> Entity<UserServicesStore> {
        UserServicesStore::shared(cx)
    }

    pub fn ensure_my_profile_loaded(&mut self, cx: &mut Context<Self>) {
        let auth = AuthServicesStore::shared(cx);
        if !auth.read(cx).is_authenticated() {
            return;
        }
        let has_profile = auth.read(cx).profile().is_some();

        let actions = Self::actions(cx);
        let services = Self::service(cx);
        let slice = Rc::clone(&actions.read(cx).my_profile_sai);
        if has_profile && slice.status() == SaiWsSliceStatus::Fulfilled && services.read(cx).is_my_profile_fresh() {
            return;
        }
        if slice.is_pending() {
            return;
        }

        actions.update(cx, |actions, cx| actions.get_my_profile_action(cx));
    }

    pub fn ensure_chat_mute_overrides_loaded(&mut self, cx: &mut Context<Self>) {
        if !AuthServicesStore::shared(cx).read(cx).is_authenticated() {
            return;
        }
        if Self::service(cx).read(cx).chat_mute_overrides_did_load() {
            return;
        }
        let actions = Self::actions(cx);
        if actions.read(cx).get_chat_mute_overrides_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.get_chat_mute_overrides_action(cx));
    }

    pub fn add_chat_mute_exception(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.add_chat_mute_override_action(chat_id, cx));
    }

    pub fn remove_chat_mute_exception(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.remove_chat_mute_override_action(chat_id, cx));
    }

    pub fn refresh_my_profile(&mut self, cx: &mut Context<Self>) {
        if !AuthServicesStore::shared(cx).read(cx).is_authenticated() {
            return;
        }
        let actions = Self::actions(cx);
        if actions.read(cx).my_profile_sai.is_pending() {
            return;
        }

        actions.update(cx, |actions, cx| actions.get_my_profile_action(cx));
    }

    pub fn load_foreign_profile_tag(&mut self, raw_tag: String, cx: &mut Context<Self>) {
        let tag = trimming_whitespaces_and_newlines(&raw_tag).to_owned();
        if tag.is_empty() {
            return;
        }

        let is_cached_and_fresh = Self::service(cx).update(cx, |service, cx| {
            service.cached_foreign_profile_tag(&tag, cx).is_some() && service.is_foreign_profile_fresh_tag(&tag, cx)
        });
        if is_cached_and_fresh {
            return;
        }
        if Self::service(cx).update(cx, |service, cx| service.is_foreign_profile_pending_tag(&tag, cx)) {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| actions.get_profile_by_tag_action(&tag, None, None, cx));
    }

    pub fn refresh_foreign_profile_tag(&mut self, raw_tag: String, cx: &mut Context<Self>) {
        let tag = trimming_whitespaces_and_newlines(&raw_tag).to_owned();
        if tag.is_empty() || Self::service(cx).update(cx, |service, cx| service.is_foreign_profile_pending_tag(&tag, cx)) {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| actions.get_profile_by_tag_action(&tag, None, None, cx));
    }

    pub fn load_foreign_profile_user_id(&mut self, raw_user_id: String, cx: &mut Context<Self>) {
        let user_id = trimming_whitespaces_and_newlines(&raw_user_id).to_owned();
        if user_id.is_empty() {
            return;
        }

        let is_cached_and_fresh = Self::service(cx).update(cx, |service, cx| {
            service.cached_foreign_profile_user_id(&user_id, None, cx).is_some()
                && service.is_foreign_profile_fresh_user_id(&user_id, cx)
        });
        if is_cached_and_fresh {
            return;
        }
        if Self::service(cx).update(cx, |service, cx| service.is_foreign_profile_pending_user_id(&user_id, cx)) {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| actions.get_profile_by_id_action(&user_id, None, None, cx));
    }

    pub fn refresh_foreign_profile_user_id(&mut self, raw_user_id: String, cx: &mut Context<Self>) {
        let user_id = trimming_whitespaces_and_newlines(&raw_user_id).to_owned();
        if user_id.is_empty() || Self::service(cx).update(cx, |service, cx| service.is_foreign_profile_pending_user_id(&user_id, cx)) {
            return;
        }
        if (user_guide_sandbox_hooks().owns_user_id)(&user_id, cx) {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| actions.get_profile_by_id_action(&user_id, None, None, cx));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_profile_edits(
        &mut self,
        name: Option<String>,
        tag: Option<String>,
        description: Option<String>,
        hb: Option<String>,
        avatar_url: Option<String>,
        avatar_asset_id: Option<String>,
        banner_url: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.save_profile_edits_reporting(
            name,
            tag,
            description,
            hb,
            avatar_url,
            avatar_asset_id,
            banner_url,
            None,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_profile_edits_reporting(
        &mut self,
        name: Option<String>,
        tag: Option<String>,
        description: Option<String>,
        hb: Option<String>,
        avatar_url: Option<String>,
        avatar_asset_id: Option<String>,
        banner_url: Option<String>,
        on_done: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let auth = AuthServicesStore::shared(cx);
        let previous = auth.read(cx).profile().cloned();
        let mut optimistic: Option<ProfileModel> = None;
        if let Some(mut current) = previous.clone() {
            if let Some(name) = &name {
                current.name = name.clone();
            }
            if let Some(tag) = &tag {
                current.tag = tag.clone();
            }
            if let Some(description) = &description {
                current.description = Some(description.clone());
            }
            if let Some(hb) = &hb {
                current.hb = if hb.is_empty() { None } else { Some(hb.clone()) };
            }
            if let Some(avatar_url) = &avatar_url {
                current.avatar_url = Some(avatar_url.clone());
                current.more.logo = avatar_url.clone();
            }
            if let Some(banner_url) = &banner_url {
                current.cover_image_url = Some(banner_url.clone());
            }
            let applied = current.clone();
            auth.update(cx, |auth, cx| auth.apply_updated_session_profile(applied, cx));
            optimistic = Some(current);
        }

        let touched = description.is_some() || hb.is_some() || avatar_url.is_some() || banner_url.is_some();
        let more = touched.then(|| UpdateProfileMoreBody {
            banner: banner_url.clone(),
            description: description.clone(),
            hb: hb.clone(),
            logo: avatar_url.clone(),
            logo_asset_id: avatar_asset_id.clone(),
            ..UpdateProfileMoreBody::default()
        });

        if avatar_url.is_some()
            && let Some(account_id) = auth.read(cx).profile().map(|profile| profile.id.clone())
        {
            UserProfilePhotosInteractionsStore::shared(cx)
                .update(cx, |photos, cx| photos.invalidate_album(&account_id, cx));
        }

        let (error_name, error_tag) = (name.clone(), tag.clone());
        let failed = on_done.clone();
        let on_error: Rc<dyn Fn(&anyhow::Error, &mut App)> = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::revert_profile_edits(optimistic.as_ref(), previous.as_ref(), cx);
            if CanonErrors::is_connection_loss(error) {
                Self::keep_unsent_editor_fields(
                    error_name.as_deref(),
                    error_tag.as_deref(),
                    description.as_deref(),
                    hb.as_deref(),
                    previous.as_ref(),
                    cx,
                );
            } else {
                Self::announce_profile_save_refusal(error, cx);
            }
            if let Some(failed) = &failed {
                failed(false, cx);
            }
        });
        let on_success = on_done.map(|done| Rc::new(move |cx: &mut App| done(true, cx)) as Rc<dyn Fn(&mut App)>);

        Self::actions(cx).update(cx, |actions, cx| {
            actions.update_profile_action(
                UpdateProfileActionArgs {
                    name,
                    tag,
                    more,
                    on_success,
                    on_error: Some(on_error),
                    ..UpdateProfileActionArgs::default()
                },
                cx,
            );
        });
        GuideSignalBus::emit(RiseGuideSignal::ProfileSaved);
    }

    fn keep_unsent_editor_fields(
        name: Option<&str>,
        tag: Option<&str>,
        description: Option<&str>,
        hb: Option<&str>,
        previous: Option<&ProfileModel>,
        cx: &mut App,
    ) {
        if (name.is_some() || tag.is_some() || description.is_some() || hb.is_some())
            && let Some(previous) = previous
            && AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.as_str())
                == Some(previous.id.as_str())
        {
            let birthday = match hb {
                Some(hb) if hb.is_empty() => None,
                Some(hb) => Some(hb.to_owned()),
                None => previous.hb.clone(),
            };
            let has_birthday =
                birthday.as_deref().is_some_and(|birthday| !trimming_whitespaces_and_newlines(birthday).is_empty());
            ProfileEditorDraftStore::save(
                &ProfileEditorDraft {
                    name: name.map_or_else(|| previous.name.clone(), str::to_owned),
                    tag: tag.map_or_else(|| previous.tag.clone(), str::to_owned),
                    description: description
                        .map(str::to_owned)
                        .or_else(|| previous.description.clone())
                        .unwrap_or_default(),
                    hb_iso: if has_birthday { birthday } else { None },
                    has_hb: has_birthday,
                },
                &previous.id,
            );
        }
        Notifier::send(
            tr("save_failed"),
            NotifierSendOptions {
                content: Some(tr("sync_error_unavailable")),
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                duration: SAVE_FAILED_NOTICE_DURATION,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn revert_profile_edits(optimistic: Option<&ProfileModel>, previous: Option<&ProfileModel>, cx: &mut App) {
        let (Some(optimistic), Some(previous)) = (optimistic, previous) else {
            return;
        };
        let auth = AuthServicesStore::shared(cx);
        let Some(mut current) = auth.read(cx).profile().cloned() else {
            return;
        };
        if current.id != previous.id {
            return;
        }
        if current.name == optimistic.name {
            current.name = previous.name.clone();
        }
        if current.tag == optimistic.tag {
            current.tag = previous.tag.clone();
        }
        if current.description == optimistic.description {
            current.description = previous.description.clone();
        }
        if current.hb == optimistic.hb {
            current.hb = previous.hb.clone();
        }
        if current.avatar_url == optimistic.avatar_url {
            current.avatar_url = previous.avatar_url.clone();
            current.more.logo = previous.more.logo.clone();
        }
        if current.cover_image_url == optimistic.cover_image_url {
            current.cover_image_url = previous.cover_image_url.clone();
        }
        auth.update(cx, |auth, cx| auth.apply_updated_session_profile(current, cx));
    }

    fn announce_profile_save_refusal(error: &anyhow::Error, cx: &mut App) {
        let failure = CanonErrors::failure(error);
        if failure.as_ref().is_some_and(|failure| failure.subscription_lifts_limit())
            && PremiumUpsellStore::shared(cx).read(cx).prompt().is_some()
        {
            return;
        }
        Notifier::send(
            failure.as_ref().and_then(CanonErrors::text_for).unwrap_or_else(|| tr("save_failed")),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub fn save_social_links(&mut self, links: &[ProfileSocialLink], cx: &mut Context<Self>) {
        let normalized = SocialLink::normalize(links);
        let body_links = normalized
            .iter()
            .map(|link| UpdateProfileSocialLinkBody { title: link.title.clone(), url: link.url.clone() })
            .collect::<Vec<_>>();
        Self::service(cx).update(cx, |service, cx| service.apply_social_links_optimistic(normalized, cx));

        let on_success: Rc<dyn Fn(&mut App)> = Rc::new(|cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |service, cx| service.commit_social_links_optimistic(cx));
        });
        let on_error: Rc<dyn Fn(&anyhow::Error, &mut App)> = Rc::new(|_: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |service, cx| service.revert_social_links_optimistic(cx));
        });
        let more = UpdateProfileMoreBody { social_links: Some(body_links), ..UpdateProfileMoreBody::default() };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.update_profile_action(
                UpdateProfileActionArgs {
                    more: Some(more),
                    on_success: Some(on_success),
                    on_error: Some(on_error),
                    ..UpdateProfileActionArgs::default()
                },
                cx,
            );
        });
    }

    pub fn set_quick_reaction(&mut self, raw_emoji: &str, cx: &mut Context<Self>) {
        let is_custom = raw_emoji.starts_with(MessageReaction::CUSTOM_EMOJI_PREFIX);
        let emoji =
            if is_custom { Some(raw_emoji.to_owned()) } else { EmojiPickerCatalog::normalized_selectable_emoji(Some(raw_emoji)) };
        let Some(emoji) = emoji else {
            return;
        };
        let Some(current_reaction) =
            AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.quick_reaction.clone())
        else {
            return;
        };
        let services = Self::service(cx);
        if current_reaction == emoji || services.read(cx).quick_reaction_mutation_pending {
            return;
        }

        services.update(cx, |service, cx| service.apply_quick_reaction_optimistic(&emoji, cx));
        let on_success: Rc<dyn Fn(&mut App)> = Rc::new(|cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |service, cx| service.commit_quick_reaction_optimistic(cx));
        });
        let on_error: Rc<dyn Fn(&anyhow::Error, &mut App)> = Rc::new(|_: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |service, cx| service.revert_quick_reaction_optimistic(cx));
        });
        Self::actions(cx).update(cx, |actions, cx| {
            actions.update_profile_action(
                UpdateProfileActionArgs {
                    quick_reaction: Some(emoji),
                    on_success: Some(on_success),
                    on_error: Some(on_error),
                    ..UpdateProfileActionArgs::default()
                },
                cx,
            );
        });
    }

    pub fn set_name_color(&mut self, raw_value: Option<&str>, cx: &mut Context<Self>) {
        let value = raw_value.map(|raw| trimming_whitespaces_and_newlines(raw).to_uppercase()).unwrap_or_default();
        let Some((is_premium, current_color)) = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .map(|profile| (profile.more.is_premium, profile.more.name_color.clone().unwrap_or_default()))
        else {
            return;
        };
        let services = Self::service(cx);
        if !is_premium
            || !(value.is_empty() || PremiumNameColor::parse(Some(value.as_str())).is_some())
            || current_color == value
            || services.read(cx).name_color_mutation_pending
        {
            return;
        }

        let optimistic = if value.is_empty() { None } else { Some(value.clone()) };
        services.update(cx, |service, cx| service.apply_name_color_optimistic(optimistic, cx));
        let on_success: Rc<dyn Fn(&mut App)> = Rc::new(|cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |service, cx| service.commit_name_color_optimistic(cx));
        });
        let on_error: Rc<dyn Fn(&anyhow::Error, &mut App)> = Rc::new(|_: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |service, cx| service.revert_name_color_optimistic(cx));
            Notifier::send(
                tr("settings_premium_name_color_save_failed"),
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Top,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });
        Self::actions(cx).update(cx, |actions, cx| {
            actions.update_profile_action(
                UpdateProfileActionArgs {
                    name_color: Some(value),
                    allows_deferred_send: false,
                    on_success: Some(on_success),
                    on_error: Some(on_error),
                    ..UpdateProfileActionArgs::default()
                },
                cx,
            );
        });
    }

    pub fn dismiss_premium_badge_failure(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.clear_premium_badge_failure(cx));
    }

    pub fn set_premium_badge(&mut self, raw_value: Option<&str>, cx: &mut Context<Self>) {
        let value = raw_value.map(|raw| trimming_whitespaces_and_newlines(raw).to_owned()).unwrap_or_default();
        let is_custom = value.starts_with(MessageReaction::CUSTOM_EMOJI_PREFIX);
        let selectable =
            value.is_empty() || is_custom || EmojiPickerCatalog::normalized_selectable_emoji(Some(value.as_str())).is_some();
        let optimistic = if value.is_empty() { None } else { Some(value.clone()) };
        let services = Self::service(cx);

        if (user_guide_sandbox_hooks().owns_profile_look_scene)(cx) {
            if !selectable {
                return;
            }
            services.update(cx, |service, cx| {
                service.apply_premium_badge_optimistic(optimistic, cx);
                service.commit_premium_badge_optimistic(cx);
            });
            GuideSignalBus::emit(RiseGuideSignal::ProfileBadgeChanged);
            return;
        }
        let Some((is_premium, current_badge)) = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .map(|profile| (profile.more.is_premium, profile.more.premium_badge_emoji.clone().unwrap_or_default()))
        else {
            return;
        };
        if !is_premium || !selectable || current_badge == value || services.read(cx).premium_badge_mutation_pending {
            return;
        }

        services.update(cx, |service, cx| service.apply_premium_badge_optimistic(optimistic, cx));
        let on_success: Rc<dyn Fn(&mut App)> = Rc::new(|cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |service, cx| service.commit_premium_badge_optimistic(cx));
        });
        let on_error: Rc<dyn Fn(&anyhow::Error, &mut App)> = Rc::new(|_: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |service, cx| service.revert_premium_badge_optimistic(cx));
            Notifier::send(
                tr("settings_premium_badge_save_failed"),
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Top,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });
        Self::actions(cx).update(cx, |actions, cx| {
            actions.update_profile_action(
                UpdateProfileActionArgs {
                    premium_badge_emoji: Some(value),
                    allows_deferred_send: false,
                    on_success: Some(on_success),
                    on_error: Some(on_error),
                    ..UpdateProfileActionArgs::default()
                },
                cx,
            );
        });
    }

    pub fn ensure_privacy_loaded(&mut self, cx: &mut Context<Self>) {
        PrivacyReconnectRetry::install_if_needed(cx);
        let actions = Self::actions(cx);
        if actions.read(cx).get_privacy_sai.is_pending() {
            return;
        }
        if Self::service(cx).read(cx).optimistic_privacy().is_some() {
            return;
        }

        actions.update(cx, |actions, cx| actions.get_privacy_action(cx));
    }

    pub fn update_privacy_rule(&mut self, rule: RisePrivacyRule, audience: RisePrivacyAudience, cx: &mut Context<Self>) {
        if rule == RisePrivacyRule::Messages
            && audience != RisePrivacyAudience::All
            && !AuthServicesStore::shared(cx).read(cx).profile().is_some_and(|profile| profile.more.is_premium)
        {
            PremiumUpsellStore::shared(cx).update(cx, |upsell, cx| {
                upsell.offer_premium_feature(String::new(), rule.raw_value(), false, cx);
            });
            return;
        }
        let mut patch = PrivacyRulePatch::default();
        let value = Some(audience.raw_value().to_owned());

        match rule {
            RisePrivacyRule::Plan => patch.plan_rule = value,
            RisePrivacyRule::Goal => patch.goal_rule = value,
            RisePrivacyRule::Friend => patch.friend_rule = value,
            RisePrivacyRule::Subs => patch.subs_rule = value,
            RisePrivacyRule::Hb => patch.hb_rule = value,
            RisePrivacyRule::Phone => patch.phone_rule = value,
            RisePrivacyRule::Description => patch.description_rule = value,
            RisePrivacyRule::LastSeen => patch.last_seen_rule = value,
            RisePrivacyRule::OnlineStatus => patch.online_status_rule = value,
            RisePrivacyRule::ProfilePhoto => patch.profile_photo_rule = value,
            RisePrivacyRule::Calls => patch.calls_rule = value,
            RisePrivacyRule::Forwards => patch.forwards_rule = value,
            RisePrivacyRule::Groups => patch.groups_rule = value,
            RisePrivacyRule::Comments => patch.comments_rule = value,
            RisePrivacyRule::Reposts => patch.reposts_rule = value,
            RisePrivacyRule::Achievements => patch.achievements_rule = value,
            RisePrivacyRule::Stories => patch.stories_rule = value,
            RisePrivacyRule::Messages => patch.messages_rule = value,
            RisePrivacyRule::Coauthor => patch.coauthor_rule = value,
            RisePrivacyRule::Tag => patch.tag_rule = value,
            RisePrivacyRule::Sparks => patch.sparks_rule = value,
            RisePrivacyRule::Gifts => patch.gifts_rule = value,
            RisePrivacyRule::GiftSend => patch.gift_send_rule = value,
        }

        Self::update_privacy_patch(patch, cx);
    }

    fn update_privacy_patch(patch: PrivacyRulePatch, cx: &mut App) {
        let on_error: Rc<dyn Fn(&anyhow::Error, &mut App)> =
            Rc::new(|error: &anyhow::Error, cx: &mut App| Self::announce_profile_save_refusal(error, cx));
        UserActionsStore::shared(cx).update(cx, |actions, cx| actions.update_privacy_action(patch, Some(on_error), cx));
    }

    pub fn save_privacy_exceptions(
        &mut self,
        rule: RisePrivacyRule,
        action: PrivacyExceptionAction,
        users: &[PrivacyExceptionWireDTO],
        on_error: UserInteractionsTextHandler,
        cx: &mut Context<Self>,
    ) {
        let privacy_type = rule.privacy_type().to_owned();
        let users = users
            .iter()
            .map(|user| PrivacyExceptionWireDTO {
                action: action.raw_value().to_owned(),
                privacy_type: privacy_type.clone(),
                target_user_id: user.target_user_id.clone(),
                target_user_logo: user.target_user_logo.clone(),
                target_user_name: user.target_user_name.clone(),
                target_user_tag: user.target_user_tag.clone(),
            })
            .collect::<Vec<_>>();
        let services = Self::service(cx);
        let current = services.update(cx, |service, cx| service.privacy_exceptions(&privacy_type, action, cx));
        let current_ids = current.iter().map(|user| user.target_user_id.as_str()).collect::<HashSet<_>>();
        let wanted_ids = users.iter().map(|user| user.target_user_id.as_str()).collect::<HashSet<_>>();
        let added = users
            .iter()
            .filter(|user| !current_ids.contains(user.target_user_id.as_str()))
            .map(|user| user.target_user_id.clone())
            .collect::<Vec<_>>();
        let removed = current
            .iter()
            .filter(|user| !wanted_ids.contains(user.target_user_id.as_str()))
            .map(|user| user.target_user_id.clone())
            .collect::<Vec<_>>();
        if added.is_empty() && removed.is_empty() {
            return;
        }

        services.update(cx, |service, cx| {
            service.apply_privacy_exceptions_optimistic(&privacy_type, action, users, cx);
        });
        let steps = removed
            .into_iter()
            .map(|id| PrivacyExceptionStep { id, remove: true })
            .chain(added.into_iter().map(|id| PrivacyExceptionStep { id, remove: false }))
            .collect::<Vec<_>>();
        self.run_privacy_exception_steps(steps, privacy_type, action, false, on_error, cx);
    }

    fn run_privacy_exception_steps(
        &mut self,
        steps: Vec<PrivacyExceptionStep>,
        privacy_type: String,
        action: PrivacyExceptionAction,
        retried_rate_limit: bool,
        on_error: UserInteractionsTextHandler,
        cx: &mut Context<Self>,
    ) {
        let Some(step) = steps.first().cloned() else {
            Self::service(cx).update(cx, |service, cx| {
                let settings = service.optimistic_privacy().cloned();
                service.handle_update_privacy_success(settings, cx);
            });
            Self::actions(cx).update(cx, |actions, cx| actions.get_privacy_action(cx));
            return;
        };
        let rest = steps[1..].to_vec();
        let account_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let this = cx.entity().downgrade();

        let (success_this, success_type, success_on_error) = (this.clone(), privacy_type.clone(), Rc::clone(&on_error));
        let on_success: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            let (this, rest, privacy_type, on_error) =
                (success_this.clone(), rest.clone(), success_type.clone(), Rc::clone(&success_on_error));
            cx.defer(move |cx| {
                let _ = this.update(cx, |store, cx| {
                    store.run_privacy_exception_steps(rest, privacy_type, action, retried_rate_limit, on_error, cx);
                });
            });
        });

        let failure_type = privacy_type.clone();
        let failure: Rc<dyn Fn(&anyhow::Error, &mut App)> = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            let failure = CanonErrors::failure(error);
            let retry_after = failure
                .as_ref()
                .filter(|failure| CanonErrors::category(failure.code.as_deref()) == ErrorCategory::RateLimit)
                .and_then(|failure| CanonErrors::retry_after_seconds(&failure.details))
                .filter(|seconds| *seconds > 0);
            if !retried_rate_limit && let Some(seconds) = retry_after {
                let (this, steps, privacy_type, on_error) =
                    (this.clone(), steps.clone(), failure_type.clone(), Rc::clone(&on_error));
                let (account_id, account_epoch) = (account_id.clone(), account_epoch.clone());
                let delay = Duration::from_secs(u64::try_from(seconds).unwrap_or(0));
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(delay).await;
                    if !SaiAccountCacheScope::is_current_account_epoch(account_id.as_deref(), account_epoch.as_deref()) {
                        return;
                    }
                    let _ = this.update(cx, |store, cx| {
                        store.run_privacy_exception_steps(steps, privacy_type, action, true, on_error, cx);
                    });
                })
                .detach();
                return;
            }
            let title = failure.as_ref().and_then(CanonErrors::text_for).unwrap_or_else(|| tr("privacy_exceptions_error"));
            let refusal = CanonErrors::refusal_text(error);
            let on_error = Rc::clone(&on_error);
            cx.defer(move |cx| {
                UserServicesStore::shared(cx).update(cx, |service, cx| service.rollback_privacy_optimistic(cx));
                UserActionsStore::shared(cx).update(cx, |actions, cx| actions.get_privacy_action(cx));
                Notifier::send(
                    title,
                    NotifierSendOptions {
                        r#type: NotifierType::Error,
                        position: NotifierPosition::Top,
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
                on_error(refusal, cx);
            });
        });

        Self::actions(cx).update(cx, |actions, cx| {
            actions.set_privacy_exception_action(
                &step.id,
                &privacy_type,
                action.raw_value(),
                step.remove,
                on_success,
                failure,
                cx,
            );
        });
    }

    pub fn update_subs_count_hidden(&mut self, hidden: bool, cx: &mut Context<Self>) {
        let patch = PrivacyRulePatch { subs_count_hidden: Some(hidden), ..PrivacyRulePatch::default() };
        Self::update_privacy_patch(patch, cx);
    }

    pub fn set_phone_public(&mut self, is_public: bool, cx: &mut Context<Self>) {
        let Some(current) = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.is_phone_public)
        else {
            return;
        };
        if current == is_public || Self::service(cx).read(cx).phone_public_mutation_pending {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| actions.set_phone_public_action(is_public, cx));
    }

    pub fn update_message_requests_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let patch = PrivacyRulePatch { message_requests_enabled: Some(enabled), ..PrivacyRulePatch::default() };
        Self::update_privacy_patch(patch, cx);
    }

    pub fn toggle_subscribe(&mut self, user_id: &str, tag: &str, cx: &mut Context<Self>) {
        self.toggle_subscribe_fallback(user_id, tag, None, None, cx);
    }

    pub fn toggle_subscribe_relationship(
        &mut self,
        user_id: &str,
        tag: &str,
        relationship: UserListRelationship,
        cx: &mut Context<Self>,
    ) {
        self.toggle_subscribe_fallback(user_id, tag, Some(relationship), None, cx);
    }

    pub fn toggle_subscribe_entry(
        &mut self,
        entry: &UserListEntry,
        kind: UserListKind,
        owner_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = entry.author.id.clone().filter(|id| !id.is_empty()) else {
            return;
        };

        let viewer_id =
            AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default();
        let fallback = entry.resolved_relationship(kind, !viewer_id.is_empty() && viewer_id == owner_id);
        self.toggle_subscribe_fallback(&user_id, &entry.author.tag, Some(fallback), None, cx);
    }

    fn toggle_subscribe_fallback(
        &mut self,
        user_id: &str,
        tag: &str,
        fallback: Option<UserListRelationship>,
        on_success: Option<UserInteractionsDoneHandler>,
        cx: &mut Context<Self>,
    ) {
        let Some(me) = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()) else {
            return;
        };
        if me == user_id {
            return;
        }
        let services = Self::service(cx);
        if services.read(cx).is_follow_interaction_pending(user_id) {
            return;
        }

        let now_following = services.update(cx, |service, cx| {
            service.capture_follow_baseline_if_needed(user_id, fallback, cx);
            service.note_follow_profile_tag(user_id, tag, cx);

            let current = service.current_follow(user_id, cx);
            let now_following = !current.is_subbed;
            service.apply_follow_optimistic(user_id, now_following, cx);
            now_following
        });
        if now_following {
            GuideSignalBus::emit(RiseGuideSignal::FollowCreated);
        }

        let sync_user_id = user_id.to_owned();
        SaiDebouncer::debounced_action_id(
            user_id,
            Box::new(move |cx: &mut App| {
                UserServicesStore::shared(cx)
                    .update(cx, |service, cx| service.sync_follow(&sync_user_id, on_success, cx));
            }),
            SaiDebouncerOptions {
                delay_milliseconds: FOLLOW_TOGGLE_DELAY_MILLISECONDS,
                group_key: FOLLOW_TOGGLE_GROUP_KEY.to_owned(),
                max_cache_data: FOLLOW_TOGGLE_MAX_CACHE_DATA,
            },
            cx,
        );
    }

    pub fn accept_friend_request(&mut self, request_id: i64, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.accept_friend_request_action(request_id, cx));
    }

    pub fn reject_friend_request(&mut self, request_id: i64, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.reject_friend_request_action(request_id, cx));
    }

    pub fn delete_friend(&mut self, friend_id: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.delete_friend_action(friend_id, cx));
    }

    pub fn ensure_list_loaded(
        &mut self,
        kind: UserListKind,
        user_id: &str,
        search: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if user_id.is_empty() {
            return;
        }

        let cache_key = UserServicesStore::list_cache_key(kind, user_id, search);
        let is_loaded_and_fresh = Self::service(cx).update(cx, |service, cx| {
            service.list_did_load(kind, user_id, search, cx) && service.is_list_fresh(&cache_key)
        });
        if is_loaded_and_fresh {
            return;
        }
        if SaiWsPendingRegistry::shared(cx).read(cx).in_flight_request_id(&cache_key).is_some() {
            return;
        }

        Self::fetch_list_first_page(kind, user_id, search, None, None, cx);
    }

    pub fn refresh_list(&mut self, kind: UserListKind, user_id: &str, search: Option<&str>, cx: &mut Context<Self>) {
        if user_id.is_empty() {
            return;
        }
        Self::fetch_list_first_page(kind, user_id, search, None, None, cx);
    }

    pub fn apply_list_search(&mut self, kind: UserListKind, user_id: &str, query: &str, cx: &mut Context<Self>) {
        let normalized = UserServicesStore::normalized_list_query(Some(query));
        let key = format!("userListSearch_{}_{}", kind.raw_value(), user_id);
        let user_id = user_id.to_owned();
        sai_debounced_action(
            &key,
            Box::new(move |cx: &mut App| {
                if !UserRelationshipSearchContract::should_dispatch(normalized.as_deref()) {
                    return;
                }
                match normalized {
                    None => {
                        UserInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.ensure_list_loaded(kind, &user_id, None, cx));
                    }
                    Some(normalized) => {
                        Self::fetch_list_first_page(kind, &user_id, Some(&normalized), None, None, cx);
                    }
                }
            }),
            SaiDebouncerOptions {
                delay_milliseconds: LIST_SEARCH_DELAY_MILLISECONDS,
                group_key: LIST_SEARCH_GROUP_KEY.to_owned(),
                max_cache_data: LIST_SEARCH_MAX_CACHE_DATA,
            },
            cx,
        );
    }

    pub fn load_more_list(&mut self, kind: UserListKind, user_id: &str, search: Option<&str>, cx: &mut Context<Self>) {
        let services = Self::service(cx);
        if !services.update(cx, |service, cx| service.list_has_more(kind, user_id, search, cx)) {
            return;
        }
        let Some(continuation) =
            services.update(cx, |service, cx| service.list_continuation(kind, user_id, search, cx))
        else {
            return;
        };

        let (cursor, relative_id) = match continuation {
            UserListContinuation::Cursor(value) => (Some(value), None),
            UserListContinuation::LegacyRelativeId(value) => (None, Some(value)),
        };

        let cache_key = UserServicesStore::list_cache_key(kind, user_id, search);
        let pagination = SaiWs::pagination_state(&cache_key, cx);
        if !pagination.read(cx).trailing().can_request_automatically() {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| match kind {
            UserListKind::Friends => actions.get_user_friends_action(
                user_id,
                cursor.as_deref(),
                relative_id,
                UserActionsStore::DEFAULT_LIST_LIMIT,
                search,
                None,
                None,
                cx,
            ),
            UserListKind::Subscribers => actions.get_user_subscribers_action(
                user_id,
                cursor.as_deref(),
                relative_id,
                UserActionsStore::DEFAULT_LIST_LIMIT,
                search,
                None,
                None,
                cx,
            ),
            UserListKind::Subscriptions => actions.get_user_subscriptions_action(
                user_id,
                cursor.as_deref(),
                relative_id,
                UserActionsStore::DEFAULT_LIST_LIMIT,
                search,
                None,
                None,
                cx,
            ),
        });
    }

    pub fn set_wallet_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.update_wallet_settings_action(Some(enabled), None, cx));
    }

    pub fn set_mute_all_chats(&mut self, mute_all: bool, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.set_mute_all_chats_action(mute_all, cx));
    }

    fn fetch_list_first_page(
        kind: UserListKind,
        user_id: &str,
        search: Option<&str>,
        on_success: Option<UserInteractionsDoneHandler>,
        on_error: Option<Rc<dyn Fn(&anyhow::Error, &mut App)>>,
        cx: &mut App,
    ) {
        let cache_key = UserServicesStore::list_cache_key(kind, user_id, search);
        let fetched: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |service, cx| service.note_list_fetched(&cache_key, cx));
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        UserActionsStore::shared(cx).update(cx, |actions, cx| match kind {
            UserListKind::Friends => actions.get_user_friends_action(
                user_id,
                None,
                None,
                UserActionsStore::DEFAULT_LIST_LIMIT,
                search,
                Some(fetched),
                on_error,
                cx,
            ),
            UserListKind::Subscribers => actions.get_user_subscribers_action(
                user_id,
                None,
                None,
                UserActionsStore::DEFAULT_LIST_LIMIT,
                search,
                Some(fetched),
                on_error,
                cx,
            ),
            UserListKind::Subscriptions => actions.get_user_subscriptions_action(
                user_id,
                None,
                None,
                UserActionsStore::DEFAULT_LIST_LIMIT,
                search,
                Some(fetched),
                on_error,
                cx,
            ),
        });
    }
}

#[derive(Default)]
struct PrivacyReconnectRetry {
    installed: bool,
}

impl Global for PrivacyReconnectRetry {}

impl PrivacyReconnectRetry {
    fn install_if_needed(cx: &mut App) {
        let state = cx.default_global::<Self>();
        if state.installed {
            return;
        }
        state.installed = true;

        let (did_connect, mut on_did_connect) = mpsc::unbounded::<()>();
        RiseAppEvents::default_center()
            .observe(SAI_WS_DID_CONNECT, move |_| {
                let _ = did_connect.unbounded_send(());
            })
            .detach();
        cx.spawn(async move |cx| {
            while on_did_connect.next().await.is_some() {
                cx.update(Self::retry_if_failed);
            }
        })
        .detach();
    }

    fn retry_if_failed(cx: &mut App) {
        let actions = UserActionsStore::shared(cx);
        let rejected_without_copy = {
            let slice = &actions.read(cx).get_privacy_sai;
            slice.status() == SaiWsSliceStatus::Rejected && slice.data().is_none()
        };
        if !rejected_without_copy {
            return;
        }
        actions.update(cx, |actions, cx| actions.get_privacy_action(cx));
    }
}
