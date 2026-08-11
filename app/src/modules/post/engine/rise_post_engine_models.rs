use super::rise_post_rpc::{AuthorDto, LikedByDto, PostDto};

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PostAuthor {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub logo: String,
    pub is_premium: bool,
    pub is_official: bool,
}

impl PostAuthor {
    fn from_dto(dto: Option<&AuthorDto>, fallback_id: Option<&str>) -> Self {
        let Some(dto) = dto else {
            return Self {
                id: fallback_id.unwrap_or_default().to_owned(),
                ..Self::default()
            };
        };

        let more = dto.more.as_ref();
        let id = dto
            .id
            .as_deref()
            .filter(|id| !id.is_empty())
            .or(fallback_id)
            .unwrap_or_default()
            .to_owned();

        Self {
            id,
            name: dto.name.clone().unwrap_or_default(),
            tag: dto.tag.clone().unwrap_or_default(),
            logo: more
                .map(|more| more.logo.clone())
                .or_else(|| dto.logo.clone())
                .unwrap_or_default(),
            is_premium: more
                .map(|more| more.is_premium)
                .or(dto.is_premium)
                .unwrap_or(false),
            is_official: more
                .map(|more| more.is_official)
                .or(dto.is_official)
                .unwrap_or(false),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct LikedBy {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub logo: String,
}

impl LikedBy {
    fn from_dto(dto: &LikedByDto) -> Option<Self> {
        let id = dto.id.clone().filter(|id| !id.is_empty())?;
        Some(Self {
            id,
            name: dto.name.clone().unwrap_or_default(),
            tag: dto.tag.clone().unwrap_or_default(),
            logo: dto.logo.clone().unwrap_or_default(),
        })
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct LinkPreview {
    pub url: String,
    pub title: String,
    pub description: String,
    pub image_url: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PostToggle {
    Like,
    Favorite,
    Repost,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct PostEntry {
    pub id: String,
    pub title: String,
    pub content: String,
    pub hashtags: Vec<String>,
    pub created_at: String,
    pub created_at_ms: Option<i64>,
    pub author: PostAuthor,

    pub images: Vec<String>,
    pub image_aspects: Vec<Option<f32>>,
    pub video_thumbnail: Option<String>,
    pub is_short: bool,

    pub likes_count: i64,
    pub favorites_count: i64,
    pub comments_count: i64,
    pub repost_count: i64,
    pub is_liked: bool,
    pub is_favorited: bool,
    pub is_reposted: bool,
    pub can_comment: bool,
    pub liked_by: Vec<LikedBy>,
    pub link_preview: Option<LinkPreview>,

    pub is_moderation_disabled: bool,
    pub is_processing_media: bool,
}

impl PostEntry {
    pub fn row_id(&self) -> &str {
        &self.id
    }

    pub fn has_written_content(&self) -> bool {
        !self.title.is_empty() || !self.content.is_empty() || !self.hashtags.is_empty()
    }

    pub fn from_dto(dto: &PostDto) -> Option<Self> {
        let id = dto.id.clone()?.into_string();
        if id.is_empty() {
            return None;
        }

        let image_aspects = dto
            .image_sizes
            .iter()
            .map(|size| {
                (size.width > 0 && size.height > 0).then(|| size.width as f32 / size.height as f32)
            })
            .chain(std::iter::repeat(None))
            .take(dto.images.len())
            .collect();

        Some(Self {
            id,
            title: dto.title.clone().unwrap_or_default(),
            content: dto.content.clone().unwrap_or_default(),
            hashtags: dto.hashtags.clone(),
            created_at: dto.created_at.clone().unwrap_or_default(),
            created_at_ms: dto.created_at_ms,
            author: PostAuthor::from_dto(dto.author.as_ref(), dto.author_id.as_deref()),

            images: dto.images.clone(),
            image_aspects,
            video_thumbnail: dto
                .video_thumbnail
                .clone()
                .filter(|url| !url.trim().is_empty()),
            is_short: dto.is_short,

            likes_count: dto.likes_count,
            favorites_count: dto.favorites_count,
            comments_count: dto.comments_count,
            repost_count: dto.repost_count,
            is_liked: dto.is_liked.unwrap_or(false),
            is_favorited: dto.is_favorited.unwrap_or(false),
            is_reposted: dto.is_reposted.unwrap_or(false),
            can_comment: dto.can_comment.unwrap_or(true),
            liked_by: dto.liked_by.iter().filter_map(LikedBy::from_dto).collect(),
            link_preview: dto.link_preview.as_ref().map(|preview| LinkPreview {
                url: preview.url.clone(),
                title: preview.title.clone().unwrap_or_default(),
                description: preview.description.clone().unwrap_or_default(),
                image_url: preview.image_url.clone().unwrap_or_default(),
            }),

            is_moderation_disabled: dto.moderation_status.as_deref() == Some("disabled"),
            is_processing_media: dto.media_status.as_deref() == Some("processing"),
        })
    }

    pub fn apply_toggle(&mut self, toggle: PostToggle, enabled: bool) {
        let target = self;

        match toggle {
            PostToggle::Like => {
                if target.is_liked == enabled {
                    return;
                }
                target.is_liked = enabled;
                target.likes_count = step(target.likes_count, enabled);
            }
            PostToggle::Favorite => {
                if target.is_favorited == enabled {
                    return;
                }
                target.is_favorited = enabled;
                target.favorites_count = step(target.favorites_count, enabled);
            }
            PostToggle::Repost => {
                if target.is_reposted == enabled {
                    return;
                }
                target.is_reposted = enabled;
                target.repost_count = step(target.repost_count, enabled);
            }
        }
    }

    pub fn reconcile(
        &mut self,
        is_liked: Option<bool>,
        is_favorited: Option<bool>,
        is_reposted: Option<bool>,
        likes: Option<i64>,
        favorites: Option<i64>,
        reposts: Option<i64>,
    ) {
        let target = self;

        if let Some(value) = is_liked {
            target.is_liked = value;
        }
        if let Some(value) = is_favorited {
            target.is_favorited = value;
        }
        if let Some(value) = is_reposted {
            target.is_reposted = value;
        }
        if let Some(value) = likes {
            target.likes_count = value.max(0);
        }
        if let Some(value) = favorites {
            target.favorites_count = value.max(0);
        }
        if let Some(value) = reposts {
            target.repost_count = value.max(0);
        }
    }
}

fn step(count: i64, enabled: bool) -> i64 {
    if enabled {
        count + 1
    } else {
        (count - 1).max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dto(value: serde_json::Value) -> PostDto {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_row_with_no_id_is_dropped_rather_than_drawn_with_an_empty_one() {
        assert!(PostEntry::from_dto(&dto(json!({"title": "orphan"}))).is_none());
        assert!(PostEntry::from_dto(&dto(json!({"id": ""}))).is_none());
    }

    #[test]
    fn an_author_is_read_from_whichever_shape_arrived() {
        let nested = PostEntry::from_dto(&dto(json!({
            "id": 1,
            "author": {"id": "u1", "name": "Aianov", "more": {"logo": "https://cdn/a.png", "isOfficial": true}}
        })))
        .unwrap();
        assert_eq!(nested.author.logo, "https://cdn/a.png");
        assert!(nested.author.is_official);

        let flat = PostEntry::from_dto(&dto(json!({
            "id": 1,
            "author": {"id": "u1", "logo": "https://cdn/b.png", "isPremium": true}
        })))
        .unwrap();
        assert_eq!(flat.author.logo, "https://cdn/b.png");
        assert!(flat.author.is_premium);
    }

    #[test]
    fn an_author_id_falls_back_to_the_row_when_the_block_omits_it() {
        let entry = PostEntry::from_dto(&dto(
            json!({"id": 1, "authorId": "u9", "author": {"name": "A"}}),
        ))
        .unwrap();
        assert_eq!(entry.author.id, "u9");
    }

    #[test]
    fn image_aspects_stay_aligned_with_the_images_they_describe() {
        let entry = PostEntry::from_dto(&dto(json!({
            "id": 1,
            "images": ["a.png", "b.png", "c.png"],
            "imageSizes": [{"width": 200, "height": 100}, {"width": 0, "height": 0}]
        })))
        .unwrap();

        assert_eq!(entry.image_aspects.len(), entry.images.len());
        assert_eq!(entry.image_aspects[0], Some(2.0));
        assert_eq!(
            entry.image_aspects[1], None,
            "width=0 && height=0 is the proto's not-probed sentinel"
        );
        assert_eq!(
            entry.image_aspects[2], None,
            "a shorter sizes array must not shift the images after it"
        );
    }

    #[test]
    fn a_repost_is_an_interaction_and_never_a_row_of_its_own() {
        let entry = PostEntry::from_dto(&dto(json!({
            "id": 2,
            "isReposted": true,
            "repostCount": 3
        })))
        .unwrap();

        assert_eq!(
            entry.row_id(),
            "2",
            "the only row a repost can produce is the post it points at"
        );
        assert!(entry.is_reposted);
        assert_eq!(entry.repost_count, 3);
    }

    #[test]
    fn a_repost_toggle_moves_the_posts_own_counter() {
        let mut entry = PostEntry::from_dto(&dto(json!({
            "id": 1, "repostCount": 10
        })))
        .unwrap();

        entry.apply_toggle(PostToggle::Repost, true);

        assert!(entry.is_reposted);
        assert_eq!(entry.repost_count, 11);
    }

    #[test]
    fn toggling_what_is_already_set_changes_nothing() {
        let mut entry = PostEntry::from_dto(&dto(json!({
            "id": 1, "likesCount": 5, "isLiked": true
        })))
        .unwrap();

        entry.apply_toggle(PostToggle::Like, true);
        assert_eq!(entry.likes_count, 5, "a double tap must not count twice");
    }

    #[test]
    fn a_counter_never_goes_negative_while_the_round_trip_is_out() {
        let mut entry =
            PostEntry::from_dto(&dto(json!({"id": 1, "likesCount": 0, "isLiked": true}))).unwrap();

        entry.apply_toggle(PostToggle::Like, false);
        assert_eq!(entry.likes_count, 0);
    }

    #[test]
    fn the_server_block_replaces_the_guess() {
        let mut entry = PostEntry::from_dto(&dto(json!({"id": 1, "likesCount": 10}))).unwrap();

        entry.apply_toggle(PostToggle::Like, true);
        assert_eq!(entry.likes_count, 11);

        entry.reconcile(Some(true), None, None, Some(37), None, None);
        assert_eq!(entry.likes_count, 37);
        assert!(entry.is_liked);
    }

    #[test]
    fn reconciling_ignores_the_counters_the_server_did_not_send() {
        let mut entry = PostEntry::from_dto(&dto(json!({
            "id": 1, "likesCount": 10, "favoritesCount": 4
        })))
        .unwrap();

        entry.reconcile(None, None, None, Some(11), None, None);
        assert_eq!(entry.likes_count, 11);
        assert_eq!(entry.favorites_count, 4);
    }

    #[test]
    fn moderation_and_media_state_are_read_from_their_strings() {
        let disabled =
            PostEntry::from_dto(&dto(json!({"id": 1, "moderationStatus": "disabled"}))).unwrap();
        assert!(disabled.is_moderation_disabled);

        let processing =
            PostEntry::from_dto(&dto(json!({"id": 1, "mediaStatus": "processing"}))).unwrap();
        assert!(processing.is_processing_media);

        let ordinary = PostEntry::from_dto(&dto(json!({"id": 1}))).unwrap();
        assert!(!ordinary.is_moderation_disabled);
        assert!(!ordinary.is_processing_media);
    }

    #[test]
    fn commenting_is_allowed_unless_the_server_says_otherwise() {
        assert!(
            PostEntry::from_dto(&dto(json!({"id": 1})))
                .unwrap()
                .can_comment
        );
        assert!(
            !PostEntry::from_dto(&dto(json!({"id": 1, "canComment": false})))
                .unwrap()
                .can_comment
        );
    }

    #[test]
    fn a_liker_with_no_id_is_dropped_from_the_summary() {
        let entry = PostEntry::from_dto(&dto(json!({
            "id": 1,
            "likedBy": [{"id": "u1", "name": "A"}, {"name": "nobody"}]
        })))
        .unwrap();

        assert_eq!(entry.liked_by.len(), 1);
        assert_eq!(entry.liked_by[0].id, "u1");
    }
}
