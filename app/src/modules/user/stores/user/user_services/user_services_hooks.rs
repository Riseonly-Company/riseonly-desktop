use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::user::stores::user::user_actions::user_types::UserListRelationship;

pub type UserServicesRelationshipWriter = Box<dyn Fn(&str, UserListRelationship, &mut App) + Send + Sync>;

pub struct UserServicesPostHooks {
    pub set_author_relationship_on_held_posts: UserServicesRelationshipWriter,
    pub set_like_user_relationship: UserServicesRelationshipWriter,
    pub set_author_relationship_action: UserServicesRelationshipWriter,
    pub in_flight_own_publication_count: Box<dyn Fn(&str, &mut App) -> i64 + Send + Sync>,
}

pub struct UserServicesShortsHooks {
    pub set_author_relationship: UserServicesRelationshipWriter,
}

pub struct UserServicesRtcHooks {
    pub refresh_listener_volumes: Box<dyn Fn(&mut App) + Send + Sync>,
}

pub(crate) static POST: FeatureHookSlot<UserServicesPostHooks> = FeatureHookSlot::new("POST");
pub(crate) static SHORTS: FeatureHookSlot<UserServicesShortsHooks> = FeatureHookSlot::new("SHORTS");
pub(crate) static RTC: FeatureHookSlot<UserServicesRtcHooks> = FeatureHookSlot::new("RTC");

pub enum UserServicesHooks {}

impl UserServicesHooks {
    pub fn install_post(hooks: UserServicesPostHooks) {
        let _ = POST.set(hooks);
    }

    pub fn install_shorts(hooks: UserServicesShortsHooks) {
        let _ = SHORTS.set(hooks);
    }

    pub fn install_rtc(hooks: UserServicesRtcHooks) {
        let _ = RTC.set(hooks);
    }

    pub fn set_author_relationship_on_held_posts(author_id: &str, relationship: UserListRelationship, cx: &mut App) {
        if let Some(hooks) = POST.get() {
            (hooks.set_author_relationship_on_held_posts)(author_id, relationship, cx);
        }
    }

    pub fn set_like_user_relationship(user_id: &str, relationship: UserListRelationship, cx: &mut App) {
        if let Some(hooks) = POST.get() {
            (hooks.set_like_user_relationship)(user_id, relationship, cx);
        }
    }

    pub fn set_author_relationship_action(author_id: &str, relationship: UserListRelationship, cx: &mut App) {
        if let Some(hooks) = POST.get() {
            (hooks.set_author_relationship_action)(author_id, relationship, cx);
        }
    }

    pub fn in_flight_own_publication_count(user_id: &str, cx: &mut App) -> i64 {
        POST.get().map_or(0, |hooks| (hooks.in_flight_own_publication_count)(user_id, cx))
    }

    pub fn set_author_relationship(author_id: &str, relationship: UserListRelationship, cx: &mut App) {
        if let Some(hooks) = SHORTS.get() {
            (hooks.set_author_relationship)(author_id, relationship, cx);
        }
    }

    pub fn refresh_listener_volumes(cx: &mut App) {
        if let Some(hooks) = RTC.get() {
            (hooks.refresh_listener_volumes)(cx);
        }
    }
}
