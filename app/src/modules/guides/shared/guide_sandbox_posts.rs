use gpui::{App, Context};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent};
use rise_core::foundation::foundation_date::Date;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::date_format::iso8601_date_formatter::ISO8601DateFormatter;
use rise_i18n::tr;

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::post::stores::post_actions::post_types::{
    AuthorInfo, AuthorInfoMore, GetPostFeedResponse, PostCollaboratorKind, PostCollaboratorStatus,
    PostCollaboratorUser, PostStatsCountry, PostStatsDay, PostViewerCollaboration, RiseStoredPostStats,
};

/// two posts for the statistics and co-author lessons
/// reader's own post with a month of numbers and panda's post inviting the reader
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuideSandboxPostsScene {
    pub coauthor_status: String,
    pub coauthor_left: bool,
    pub coauthor_shows_in_grid: bool,
}

impl Default for GuideSandboxPostsScene {
    fn default() -> Self {
        Self {
            coauthor_status: PostCollaboratorStatus::Pending.raw_value().to_owned(),
            coauthor_left: false,
            coauthor_shows_in_grid: true,
        }
    }
}

impl GuideSandbox {
    pub fn owns_post(&self, id: &str) -> bool {
        self.is_armed
            && self.posts_scene.is_some()
            && (id == Self::own_post_id() || id == Self::coauthor_post_id())
    }

    /// staff acting as author of every post count as the invited co-author here
    /// so menus show the co-author rows
    pub fn viewer_acts_as_author(&self, of_post: &str, cx: &mut App) -> bool {
        AuthServicesStore::shared(cx).read(cx).viewer_acts_as_author() && !self.owns_post(of_post)
    }

    /// the post as the page and menus read it, none for any real post
    pub fn post(&self, id: &str, cx: &mut App) -> Option<GetPostFeedResponse> {
        if !self.owns_post(id) {
            return None;
        }
        if id == Self::own_post_id() {
            Some(self.own_post(cx))
        } else {
            Some(self.coauthor_post(cx))
        }
    }

    fn post_stamp(&self) -> (String, i64) {
        let date = Date::from_time_interval_since_1970(Self::scene_stamp() - 3.0 * 86_400.0);
        (
            ISO8601DateFormatter::new().string_from(&date),
            (date.time_interval_since_1970() * 1000.0) as i64,
        )
    }

    fn reader_author(&self, cx: &mut App) -> AuthorInfo {
        let store = AuthServicesStore::shared(cx);
        let auth = store.read(cx);
        let profile = auth.profile();
        AuthorInfo {
            id: Some(auth.effective_user_id_for_wire().unwrap_or_default()),
            name: profile.map(|profile| profile.name.clone()).unwrap_or_default(),
            tag: profile.map(|profile| profile.tag.clone()).unwrap_or_default(),
            more: AuthorInfoMore {
                logo: profile.map(|profile| profile.more.logo.clone()).unwrap_or_default(),
                is_premium: profile.is_some_and(|profile| profile.more.is_premium),
                is_official: false,
                premium_badge_emoji: profile.and_then(|profile| profile.more.premium_badge_emoji.clone()),
                name_color: profile.and_then(|profile| profile.more.name_color.clone()),
                ..AuthorInfoMore::default()
            },
            ..AuthorInfo::default()
        }
    }

    fn panda_author(&self) -> AuthorInfo {
        AuthorInfo {
            id: Some(Self::panda_user_id()),
            name: tr("guide_scene_sender"),
            tag: tr("guide_scene_sender_tag"),
            more: AuthorInfoMore {
                logo: String::new(),
                is_premium: false,
                is_official: false,
                premium_badge_emoji: None,
                name_color: None,
                ..AuthorInfoMore::default()
            },
            ..AuthorInfo::default()
        }
    }

    fn own_post(&self, cx: &mut App) -> GetPostFeedResponse {
        let (stamp_iso, stamp_ms) = self.post_stamp();
        let author = self.reader_author(cx);
        let content = tr("guide_scene_post_text");
        GetPostFeedResponse {
            id: Self::own_post_id(),
            original_content: content.clone(),
            content,
            views_count: 1_284,
            likes_count: 96,
            favorites_count: 14,
            comments_count: 11,
            main_comments_count: 11,
            repost_count: 5,
            share_count: 8,
            created_at: stamp_iso.clone(),
            updated_at: stamp_iso,
            created_at_ms: Some(stamp_ms),
            updated_at_ms: Some(stamp_ms),
            author_id: author.id.clone().unwrap_or_default(),
            author,
            ..GetPostFeedResponse::default()
        }
    }

    fn coauthor_post(&self, cx: &mut App) -> GetPostFeedResponse {
        let (stamp_iso, stamp_ms) = self.post_stamp();
        let scene = self.posts_scene.clone().unwrap_or_default();
        let reader = self.reader_author(cx);
        let content = tr("guide_scene_coauthor_post_text");
        let mut post = GetPostFeedResponse {
            id: Self::coauthor_post_id(),
            original_content: content.clone(),
            content,
            views_count: 342,
            likes_count: 27,
            comments_count: 4,
            main_comments_count: 4,
            created_at: stamp_iso.clone(),
            updated_at: stamp_iso,
            created_at_ms: Some(stamp_ms),
            updated_at_ms: Some(stamp_ms),
            author: self.panda_author(),
            author_id: Self::panda_user_id(),
            ..GetPostFeedResponse::default()
        };
        if !scene.coauthor_left {
            post.coauthors = vec![PostCollaboratorUser {
                id: reader.id.clone().unwrap_or_default(),
                logo: reader.more.logo.clone(),
                name: reader.name.clone(),
                tag: reader.tag.clone(),
                ..PostCollaboratorUser::default()
            }];
            let accepted = scene.coauthor_status == PostCollaboratorStatus::Accepted.raw_value();
            post.coauthors_count = if accepted { 1 } else { 0 };
            post.pending_coauthors_count = if accepted { 0 } else { 1 };
            post.my_collaboration = Some(PostViewerCollaboration {
                kind: PostCollaboratorKind::Coauthor.raw_value().to_owned(),
                show_in_grid: scene.coauthor_shows_in_grid,
                status: scene.coauthor_status,
            });
        }
        post
    }

    /// a month of views that grows after posting then fades, with a few countries
    pub fn post_stats(&self, id: &str) -> Option<RiseStoredPostStats> {
        if !self.owns_post(id) || id != Self::own_post_id() {
            return None;
        }
        let calendar = Calendar::gregorian();
        let mut formatter = DateFormatter::new();
        formatter.set_calendar(calendar.clone());
        formatter.set_locale("en_US_POSIX");
        formatter.set_date_format("yyyy-MM-dd");
        let today = calendar.start_of_day(&Date::now());
        let shape: [i64; 30] = [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 612, 388, 196, 88,
        ];
        let last = shape.len() as i64 - 1;
        let days = shape
            .iter()
            .enumerate()
            .map(|(index, &views)| {
                let date = calendar
                    .date_by_adding(CalendarComponent::Day, index as i64 - last, &today)
                    .unwrap_or(today);
                PostStatsDay {
                    day: formatter.string_from(&date),
                    views,
                    likes: views / 13,
                    comments: views / 110,
                    favorites: views / 90,
                    shares: views / 160,
                    follows: views / 200,
                }
            })
            .collect();
        Some(RiseStoredPostStats {
            views_total: 1_284,
            unique_viewers: 917,
            likes_total: 96,
            favorites_total: 14,
            comments_total: 11,
            shares_total: 8,
            reposts_total: 5,
            follows_gained: 6,
            watch_count: 0,
            watched_ms_sum: 0,
            avg_dwell_ms: 7_400,
            days,
            countries: vec![
                PostStatsCountry { country: "KZ".to_owned(), unique_views: 402 },
                PostStatsCountry { country: "RU".to_owned(), unique_views: 281 },
                PostStatsCountry { country: "UZ".to_owned(), unique_views: 97 },
                PostStatsCountry { country: "US".to_owned(), unique_views: 64 },
            ],
        })
    }

    pub fn accept_coauthor_invite(&mut self, cx: &mut Context<Self>) {
        let Some(scene) = self.posts_scene.as_mut() else {
            return;
        };
        scene.coauthor_status = PostCollaboratorStatus::Accepted.raw_value().to_owned();
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::CoauthorAccepted);
    }

    pub fn set_coauthor_shows_in_grid(&mut self, shows: bool, cx: &mut Context<Self>) {
        let Some(scene) = self.posts_scene.as_mut() else {
            return;
        };
        scene.coauthor_shows_in_grid = shows;
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::CoauthorGridChanged);
    }

    pub fn leave_coauthorship(&mut self, cx: &mut Context<Self>) {
        let Some(scene) = self.posts_scene.as_mut() else {
            return;
        };
        scene.coauthor_left = true;
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::CoauthorLeft);
    }
}
