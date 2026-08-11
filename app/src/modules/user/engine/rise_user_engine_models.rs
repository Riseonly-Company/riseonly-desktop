use super::core::rise_user_rpc::{GoalDto, MoreDto, PlanDto, ProfileDto, SocialLinkDto};

pub const DEFAULT_QUICK_REACTION: &str = "👍";

/// Which profile a page is asking for. `Own` is not `Id(my id)`: the owner's
/// payload is a different message with the fields only an owner may see, and it
/// stays addressable before the account id is known.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum ProfileKey {
    Own,
    Id(String),
    Tag(String),
}

impl ProfileKey {
    pub fn tag(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Self::Tag(trimmed.to_lowercase()))
    }

    pub fn id(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Self::Id(trimmed.to_owned()))
    }

    pub fn is_own(&self) -> bool {
        matches!(self, Self::Own)
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SocialLink {
    pub title: String,
    pub url: String,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct GoalItem {
    pub id: String,
    pub title: String,
    pub progress: f32,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PlanItem {
    pub id: String,
    pub title: String,
    pub date_ms: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FollowButtonState {
    Subscribe,
    Following,
    SubscribeBack,
    Friends,
}

impl FollowButtonState {
    pub fn title_key(self) -> &'static str {
        match self {
            Self::Friends => "profile_friends",
            Self::Following => "profile_following",
            Self::SubscribeBack => "profile_subscribe_back",
            Self::Subscribe => "profile_subscribe",
        }
    }

    pub fn is_primary(self) -> bool {
        matches!(self, Self::Subscribe | Self::SubscribeBack)
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Relationship {
    pub is_friend: bool,
    pub is_subbed: bool,
    pub is_subscriber: bool,
    pub friend_request_id: Option<i64>,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct ProfileModel {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub description: String,
    pub avatar_url: Option<String>,
    pub cover_image_url: Option<String>,
    pub who: String,
    pub hb: Option<String>,
    pub gender: Option<String>,
    pub is_premium: bool,
    pub is_official: bool,
    pub is_blocked: bool,
    pub viewer_has_blocked: bool,
    pub posts_count: i64,
    pub subs_count: i64,
    pub subscribers_count: i64,
    pub friends_count: i64,
    pub level: i32,
    pub streak: i32,
    pub rating: i32,
    pub relationship: Relationship,
    pub social_links: Vec<SocialLink>,
    pub goals: Vec<GoalItem>,
    pub plans: Vec<PlanItem>,
    pub p_lang: Vec<String>,
    pub stack: Vec<String>,
    pub quick_reaction: String,
    pub is_phone_public: bool,
    pub phone: Option<String>,
    pub user_chat_id: Option<String>,
}

impl ProfileModel {
    pub fn placeholder(id: String, tag: String) -> Self {
        Self {
            id,
            tag,
            quick_reaction: DEFAULT_QUICK_REACTION.to_owned(),
            ..Self::default()
        }
    }

    pub fn from_dto(dto: ProfileDto) -> Self {
        let more = dto.more.unwrap_or_default();

        Self {
            id: dto.id,
            name: dto.name.unwrap_or_default(),
            tag: dto.tag.unwrap_or_default(),
            description: more.description.clone(),
            avatar_url: non_empty(&more.logo),
            cover_image_url: non_empty(&more.banner),
            who: more.who.clone(),
            hb: more.hb.as_deref().and_then(non_empty),
            gender: dto.gender,
            is_premium: dto.is_premium.unwrap_or(false),
            is_official: dto.is_official.unwrap_or(false),
            is_blocked: dto.is_blocked.unwrap_or(false),
            viewer_has_blocked: dto.viewer_has_blocked.unwrap_or(false),
            posts_count: counter(dto.posts_count, more.posts_count),
            subs_count: dto.subs_count.max(0),
            subscribers_count: counter(dto.subscribers_count, more.subscribers),
            friends_count: counter(dto.friends_count, more.friends),
            level: more.level.max(0),
            streak: more.streak.max(0),
            rating: more.rating.max(0),
            relationship: Relationship {
                is_friend: dto.is_friend.unwrap_or(false),
                is_subbed: dto.is_subbed.unwrap_or(false),
                is_subscriber: dto.is_subscriber.unwrap_or(false),
                friend_request_id: dto.friend_request_id.filter(|id| *id > 0),
            },
            social_links: social_links(&more),
            goals: more.goals.iter().enumerate().map(goal).collect(),
            plans: more.plans_struct.iter().enumerate().map(plan).collect(),
            p_lang: more.p_lang,
            stack: more.stack,
            quick_reaction: dto
                .quick_reaction
                .filter(|reaction| !reaction.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_QUICK_REACTION.to_owned()),
            is_phone_public: dto.is_phone_public.unwrap_or(false),
            phone: dto.phone.and_then(|phone| non_empty(&phone)),
            user_chat_id: dto.user_chat_id.and_then(|id| non_empty(&id)),
        }
    }

    pub fn follow_button_state(&self) -> FollowButtonState {
        if self.relationship.is_friend {
            return FollowButtonState::Friends;
        }
        if self.relationship.is_subbed {
            return FollowButtonState::Following;
        }
        if self.relationship.is_subscriber {
            return FollowButtonState::SubscribeBack;
        }
        FollowButtonState::Subscribe
    }

    pub fn is_empty(&self) -> bool {
        self.id.trim().is_empty() && self.tag.trim().is_empty()
    }

    /// Toggling a follow moves the viewed account's own follower count, and
    /// friendship follows from the pair: following someone who follows you makes
    /// you friends, and dropping either side ends it.
    pub fn apply_follow(&mut self, following: bool) {
        if self.relationship.is_subbed == following {
            return;
        }

        let was_friend = self.relationship.is_friend;

        self.relationship.is_subbed = following;
        self.relationship.is_friend = following && self.relationship.is_subscriber;
        self.subscribers_count = adjust(self.subscribers_count, !following, following);
        self.friends_count = adjust(self.friends_count, was_friend, self.relationship.is_friend);
    }

    pub fn reconcile(&mut self, authoritative: Relationship) {
        let was_friend = self.relationship.is_friend;
        let was_subbed = self.relationship.is_subbed;

        self.relationship = authoritative;
        self.subscribers_count = adjust(
            self.subscribers_count,
            was_subbed,
            self.relationship.is_subbed,
        );
        self.friends_count = adjust(self.friends_count, was_friend, self.relationship.is_friend);
    }
}

fn adjust(count: i64, before: bool, after: bool) -> i64 {
    match (before, after) {
        (false, true) => count.saturating_add(1),
        (true, false) => (count - 1).max(0),
        _ => count,
    }
}

fn counter(primary: i64, fallback: i32) -> i64 {
    if primary > 0 {
        primary
    } else {
        i64::from(fallback).max(0)
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn social_links(more: &MoreDto) -> Vec<SocialLink> {
    if !more.social_links.is_empty() {
        return more.social_links.iter().map(social_link).collect();
    }

    more.media_links
        .iter()
        .filter_map(|url| non_empty(url))
        .map(|url| SocialLink {
            title: String::new(),
            url,
        })
        .collect()
}

fn social_link(dto: &SocialLinkDto) -> SocialLink {
    SocialLink {
        title: dto.title.trim().to_owned(),
        url: dto.url.trim().to_owned(),
    }
}

fn goal((index, dto): (usize, &GoalDto)) -> GoalItem {
    GoalItem {
        id: stable_id(dto.id.as_deref(), "goal", index),
        title: dto.title.trim().to_owned(),
        progress: dto.progress.unwrap_or_default().clamp(0.0, 1.0) as f32,
    }
}

fn plan((index, dto): (usize, &PlanDto)) -> PlanItem {
    PlanItem {
        id: stable_id(dto.id.as_deref(), "plan", index),
        title: dto.title.trim().to_owned(),
        date_ms: dto.date_ms.unwrap_or_default().max(0),
    }
}

// A row without a server id still needs one that survives a re-render, and its
// position inside an immutable payload is the only stable thing left.
fn stable_id(id: Option<&str>, prefix: &str, index: usize) -> String {
    id.map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{prefix}:{index}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dto(value: serde_json::Value) -> ProfileDto {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_tag_key_is_case_insensitive_and_an_id_key_is_not() {
        assert_eq!(ProfileKey::tag("Aianov"), ProfileKey::tag("aianov"));
        assert_ne!(ProfileKey::id("U1"), ProfileKey::id("u1"));
        assert_eq!(ProfileKey::tag("   "), None);
        assert_eq!(ProfileKey::id(""), None);
    }

    #[test]
    fn the_avatar_and_banner_come_from_more_not_from_the_profile_row() {
        let profile = ProfileModel::from_dto(dto(json!({
            "id": "u1",
            "more": {"logo": "https://cdn/a.png", "banner": "https://cdn/b.png"}
        })));

        assert_eq!(profile.avatar_url.as_deref(), Some("https://cdn/a.png"));
        assert_eq!(
            profile.cover_image_url.as_deref(),
            Some("https://cdn/b.png")
        );
    }

    #[test]
    fn an_empty_media_string_is_absent_rather_than_an_empty_url() {
        let profile = ProfileModel::from_dto(dto(json!({
            "id": "u1", "more": {"logo": "", "banner": "   "}
        })));

        assert_eq!(profile.avatar_url, None);
        assert_eq!(profile.cover_image_url, None);
    }

    #[test]
    fn counters_fall_back_to_more_when_the_row_sends_proto_defaults() {
        let profile = ProfileModel::from_dto(dto(json!({
            "id": "u1", "more": {"subscribers": 12, "friends": 3, "posts_count": 7}
        })));

        assert_eq!(profile.subscribers_count, 12);
        assert_eq!(profile.friends_count, 3);
        assert_eq!(profile.posts_count, 7);
    }

    #[test]
    fn the_row_wins_over_more_when_both_carry_a_count() {
        let profile = ProfileModel::from_dto(dto(json!({
            "id": "u1", "subscribers_count": 40, "more": {"subscribers": 12}
        })));

        assert_eq!(profile.subscribers_count, 40);
    }

    #[test]
    fn the_button_reads_the_relationship_in_the_reference_order() {
        let mut profile = ProfileModel::placeholder("u1".into(), "a".into());
        assert_eq!(profile.follow_button_state(), FollowButtonState::Subscribe);

        profile.relationship.is_subscriber = true;
        assert_eq!(
            profile.follow_button_state(),
            FollowButtonState::SubscribeBack
        );

        profile.relationship.is_subbed = true;
        assert_eq!(
            profile.follow_button_state(),
            FollowButtonState::Following,
            "friendship is the server's own flag, not the pair of follows read together"
        );

        profile.relationship.is_friend = true;
        assert_eq!(profile.follow_button_state(), FollowButtonState::Friends);
    }

    #[test]
    fn following_someone_who_follows_you_makes_you_friends_and_moves_both_counts() {
        let mut profile = ProfileModel::placeholder("u1".into(), "a".into());
        profile.relationship.is_subscriber = true;
        profile.subscribers_count = 10;
        profile.friends_count = 2;

        profile.apply_follow(true);

        assert!(profile.relationship.is_friend);
        assert_eq!(profile.subscribers_count, 11);
        assert_eq!(profile.friends_count, 3);

        profile.apply_follow(false);

        assert!(!profile.relationship.is_friend);
        assert_eq!(profile.subscribers_count, 10);
        assert_eq!(profile.friends_count, 2);
    }

    #[test]
    fn a_repeated_toggle_in_the_same_direction_does_not_move_a_count_twice() {
        let mut profile = ProfileModel::placeholder("u1".into(), "a".into());
        profile.subscribers_count = 5;

        profile.apply_follow(true);
        profile.apply_follow(true);

        assert_eq!(profile.subscribers_count, 6);
    }

    #[test]
    fn a_count_never_goes_negative() {
        let mut profile = ProfileModel::placeholder("u1".into(), "a".into());
        profile.relationship.is_subbed = true;

        profile.apply_follow(false);

        assert_eq!(profile.subscribers_count, 0);
    }

    #[test]
    fn reconciliation_moves_the_counts_by_the_difference_it_actually_corrects() {
        let mut profile = ProfileModel::placeholder("u1".into(), "a".into());
        profile.subscribers_count = 10;
        profile.friends_count = 1;
        profile.relationship.is_subbed = true;

        profile.reconcile(Relationship {
            is_friend: false,
            is_subbed: false,
            is_subscriber: false,
            friend_request_id: None,
        });

        assert_eq!(profile.subscribers_count, 9);
        assert_eq!(profile.friends_count, 1);
    }

    #[test]
    fn a_row_without_an_id_still_gets_one_that_survives_a_re_render() {
        let profile = ProfileModel::from_dto(dto(json!({
            "id": "u1",
            "more": {
                "goals": [{"title": "Ship"}, {"id": "g9", "title": "Rest", "progress": 0.5}],
                "plans_struct": [{"title": "Launch", "date_ms": 42}]
            }
        })));

        assert_eq!(profile.goals[0].id, "goal:0");
        assert_eq!(profile.goals[1].id, "g9");
        assert_eq!(profile.goals[1].progress, 0.5);
        assert_eq!(profile.plans[0].id, "plan:0");
        assert_eq!(profile.plans[0].date_ms, 42);
    }

    #[test]
    fn progress_outside_the_unit_range_is_clamped_rather_than_drawn() {
        let profile = ProfileModel::from_dto(dto(json!({
            "id": "u1", "more": {"goals": [{"title": "A", "progress": 4.0}]}
        })));

        assert_eq!(profile.goals[0].progress, 1.0);
    }

    #[test]
    fn bare_media_links_become_social_links_only_when_the_typed_list_is_empty() {
        let bare = ProfileModel::from_dto(dto(json!({
            "id": "u1", "more": {"media_links": ["https://t.me/a", ""]}
        })));
        assert_eq!(bare.social_links.len(), 1);
        assert_eq!(bare.social_links[0].title, "");

        let typed = ProfileModel::from_dto(dto(json!({
            "id": "u1",
            "more": {
                "media_links": ["https://t.me/a"],
                "social_links": [{"title": "TG", "url": "https://t.me/b"}]
            }
        })));
        assert_eq!(typed.social_links.len(), 1);
        assert_eq!(typed.social_links[0].title, "TG");
    }

    #[test]
    fn a_profile_without_a_quick_reaction_gets_the_products_default() {
        let profile = ProfileModel::from_dto(dto(json!({"id": "u1", "quick_reaction": "  "})));
        assert_eq!(profile.quick_reaction, DEFAULT_QUICK_REACTION);
    }
}
