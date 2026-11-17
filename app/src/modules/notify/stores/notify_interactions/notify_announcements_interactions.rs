use gpui::{App, Context};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;

use super::notify_interactions::NotifyInteractionsStore;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::push_deep_link_store::PushDeepLinkStore;
use crate::core::push_notifications::universal_link_parser::UniversalLinkParser;
use crate::modules::notify::engine::core::rise_announcements_wire_models::{RiseAnnouncement, RiseAnnouncementSlide};
use crate::modules::notify::engine::rise_launch_interruptions::RiseLaunchInterruptions;
use crate::modules::notify::stores::notify_actions::notify_actions::NotifyActionsStore;

impl NotifyInteractionsStore {
    pub fn call_activity_did_change(&mut self, was_active: bool, is_active: bool, cx: &mut Context<Self>) {
        if !was_active || is_active {
            return;
        }
        NotifyActionsStore::refresh_announcements_after_call_action(cx);
    }

    pub fn announcement_appeared(&mut self, announcement: &RiseAnnouncement, cx: &mut Context<Self>) {
        RiseLaunchInterruptions::shared(cx).update(cx, |interruptions, cx| {
            interruptions.note_announcement_shown(announcement.id, cx);
        });
        NotifyActionsStore::mark_announcement_shown_action(announcement.id, cx);
    }

    pub fn dismiss_announcement(&mut self, announcement: &RiseAnnouncement, seen_slides: i64, cx: &mut Context<Self>) {
        NotifyActionsStore::dismiss_announcement_action(announcement.id, seen_slides, cx);
    }

    pub fn open_announcement_action(
        &mut self,
        announcement: &RiseAnnouncement,
        slide: &RiseAnnouncementSlide,
        seen_slides: i64,
        cx: &mut Context<Self>,
    ) {
        NotifyActionsStore::open_announcement_action_action(announcement.id, seen_slides, cx);
        Self::open_slide_action_url(&slide.action_url, cx);
    }

    pub(super) fn open_slide_action_url(action_url: &str, cx: &mut App) {
        let Some(url) = Url::from_string(trimming_whitespaces_and_newlines(action_url)) else {
            return;
        };
        if let Some(link) = UniversalLinkParser::parse(&url, AppEnvironment::current()) {
            PushDeepLinkStore::shared(cx).update(cx, |store, cx| {
                store.enqueue(link, cx);
            });
            return;
        }
        let is_web = url
            .scheme()
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http"));
        if !is_web {
            return;
        }
        rise_platform::open_url::open_url(&url, cx);
    }

    pub fn open_announcement(&mut self, announcement_id: i64, cx: &mut Context<Self>) {
        if announcement_id <= 0 {
            return;
        }
        RiseLaunchInterruptions::shared(cx).update(cx, |interruptions, cx| {
            interruptions.note_announcement_requested(announcement_id, cx);
        });
        NotifyActionsStore::present_announcement_action(announcement_id, cx);
    }

    pub fn note_account_created(&mut self, cx: &mut Context<Self>) {
        RiseLaunchInterruptions::shared(cx).update(cx, |interruptions, cx| {
            interruptions.note_account_created(cx);
        });
    }

    pub fn open_announcement_entity_id(&mut self, entity_id: &str, cx: &mut Context<Self>) {
        let Ok(announcement_id) = trimming_whitespaces_and_newlines(entity_id).parse::<i64>() else {
            return;
        };
        self.open_announcement(announcement_id, cx);
    }
}
