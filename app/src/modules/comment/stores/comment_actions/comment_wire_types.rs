use rise_canon::{rise_proto, rise_request, rise_response};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_moderation::ContentStatus;

use crate::modules::comment::stores::comment_actions::comment_types::{
    CommentEntities, CommentMedia, CommentMediaType, GetCommentsResponse, UserCommentEntry, UserCommentPostPreview,
};
use crate::modules::post::stores::post_actions::post_types::{AuthorInfo, AuthorInfoMore};
use crate::modules::post::stores::post_actions::post_wire_types::GetFeedPostItemDTOExt;

pub type CommentAuthorMoreDTO = rise_proto::comment::ProtoAuthorMore;
pub type CommentAuthorDTO = rise_proto::comment::ProtoAuthorInfo;
pub type CommentMediaDTO = rise_proto::comment::ProtoCommentMedia;
pub type GetCommentItemDTO = rise_proto::comment::ProtoCommentResponse;
pub type GetCommentsGatewayDTO = rise_response::comment::GetComments;
pub type GetCommentRepliesGatewayDTO = rise_response::comment::GetCommentReplies;
pub type UserCommentPostPreviewDTO = rise_proto::comment::ProtoUserCommentPostPreview;
pub type GetUserCommentItemDTO = rise_proto::comment::ProtoUserCommentItem;
pub type GetUserCommentsGatewayDTO = rise_response::comment::GetUserComments;
pub type CreateCommentGatewayDTO = rise_response::comment::CreateComment;
pub type CommentReactionGatewayDTO = rise_response::comment::LikeComment;

impl From<&CommentMedia> for rise_request::comment::create_comment::Media {
    fn from(media: &CommentMedia) -> Self {
        Self {
            associated_emojis: Some(media.associated_emojis.clone()),
            duration_ms: media.duration_ms,
            file_url: Some(media.file_url.clone()),
            height: Some(media.height),
            pack_id: media.pack_id.clone(),
            sticker_id: media.sticker_id.clone(),
            sticker_type: media.sticker_type.clone(),
            thumbnail_url: media.thumbnail_url.clone(),
            r#type: Some(media.r#type.raw_value().to_owned()),
            width: Some(media.width),
        }
    }
}

pub trait CommentAuthorMoreDTOExt {
    fn as_author_more(&self) -> AuthorInfoMore;
}

impl CommentAuthorMoreDTOExt for CommentAuthorMoreDTO {
    fn as_author_more(&self) -> AuthorInfoMore {
        let language = trimming_whitespaces_and_newlines(self.p_lang.as_deref().unwrap_or_default());
        AuthorInfoMore {
            p_lang: if language.is_empty() { Vec::new() } else { vec![language.to_owned()] },
            who: self.who.clone().unwrap_or_default(),
            role: self.role.clone().unwrap_or_default(),
            logo: self.logo.clone().unwrap_or_default(),
            is_premium: self.is_premium,
            is_official: self.is_official,
            phone: None,
            premium_badge_emoji: self.premium_badge_emoji.clone(),
            name_color: self.name_color.clone(),
        }
    }
}

pub trait CommentAuthorDTOExt {
    fn as_author_info(&self, id: &str) -> AuthorInfo;
}

impl CommentAuthorDTOExt for CommentAuthorDTO {
    fn as_author_info(&self, id: &str) -> AuthorInfo {
        AuthorInfo {
            id: if id.is_empty() { None } else { Some(id.to_owned()) },
            name: self.name.clone(),
            tag: self.tag.clone(),
            more: self.more.as_ref().map(CommentAuthorMoreDTOExt::as_author_more).unwrap_or_default(),
            is_deleted: self.is_deleted,
            ..AuthorInfo::default()
        }
    }
}

pub trait CommentMediaDTOExt {
    fn as_media(&self) -> Option<CommentMedia>;
}

impl CommentMediaDTOExt for CommentMediaDTO {
    fn as_media(&self) -> Option<CommentMedia> {
        let media_type = CommentMediaType::from_raw(&self.r#type.to_lowercase())?;
        if self.file_url.is_empty() {
            return None;
        }
        Some(CommentMedia {
            r#type: media_type,
            file_url: self.file_url.clone(),
            thumbnail_url: self.thumbnail_url.clone(),
            width: self.width,
            height: self.height,
            duration_ms: self.duration_ms,
            sticker_id: self.sticker_id.clone(),
            sticker_type: self.sticker_type.clone(),
            pack_id: self.pack_id.clone(),
            associated_emojis: self.associated_emojis.clone(),
        })
    }
}

pub trait RiseCommentWireRow {
    fn id(&self) -> i64;
    fn content(&self) -> &str;
    fn original_content(&self) -> &str;
    fn created_at(&self) -> &str;
    fn updated_at(&self) -> &str;
    fn post_id(&self) -> i64;
    fn author_id(&self) -> &str;
    fn parent_id(&self) -> Option<i64>;
    fn replies_count(&self) -> i64;
    fn likes_count(&self) -> i64;
    fn dislikes_count(&self) -> i64;
    fn user_liked(&self) -> bool;
    fn user_disliked(&self) -> bool;
    fn addressed_to_name(&self) -> Option<&str>;
    fn addressed_to_tag(&self) -> Option<&str>;
    fn author(&self) -> Option<&rise_proto::comment::ProtoAuthorInfo>;
    fn media(&self) -> Option<&rise_proto::comment::ProtoCommentMedia>;
    fn entities(&self) -> Option<&str>;
    fn is_first(&self) -> bool;
    fn moderation_status(&self) -> &str;

    fn as_comment(&self) -> Option<GetCommentsResponse> {
        if self.id() == 0 {
            return None;
        }
        let mapped_author =
            self.author().map(|author| author.as_author_info(self.author_id())).unwrap_or_default();
        let (entities, custom_emojis) = CommentEntities::normalized(self.entities());
        let content = self.content().to_owned();
        let original_content =
            if self.original_content().is_empty() { content.clone() } else { self.original_content().to_owned() };
        let author_id = if self.author_id().is_empty() {
            mapped_author.id.clone().unwrap_or_default()
        } else {
            self.author_id().to_owned()
        };
        let moderation_status = if self.moderation_status().is_empty() {
            ContentStatus::fallback().wire().to_owned()
        } else {
            self.moderation_status().to_owned()
        };
        Some(GetCommentsResponse {
            id: self.id().to_string(),
            content,
            original_content,
            created_at: self.created_at().to_owned(),
            updated_at: self.updated_at().to_owned(),
            post_id: if self.post_id() == 0 { String::new() } else { self.post_id().to_string() },
            author_id,
            parent_id: self.parent_id().map(|parent_id| parent_id.to_string()),
            replies_count: self.replies_count(),
            likes_count: self.likes_count(),
            dislikes_count: self.dislikes_count(),
            user_liked: self.user_liked(),
            user_disliked: self.user_disliked(),
            addressed_to_name: self.addressed_to_name().map(str::to_owned),
            addressed_to_tag: self.addressed_to_tag().map(str::to_owned),
            author: mapped_author,
            media: self.media().and_then(CommentMediaDTOExt::as_media),
            entities,
            custom_emojis,
            moderation_status,
            is_first: self.is_first(),
            ..GetCommentsResponse::default()
        })
    }
}

macro_rules! rise_comment_wire_row {
    ($row:ty) => {
        impl RiseCommentWireRow for $row {
            fn id(&self) -> i64 {
                self.id
            }
            fn content(&self) -> &str {
                &self.content
            }
            fn original_content(&self) -> &str {
                &self.original_content
            }
            fn created_at(&self) -> &str {
                &self.created_at
            }
            fn updated_at(&self) -> &str {
                &self.updated_at
            }
            fn post_id(&self) -> i64 {
                self.post_id
            }
            fn author_id(&self) -> &str {
                &self.author_id
            }
            fn parent_id(&self) -> Option<i64> {
                self.parent_id
            }
            fn replies_count(&self) -> i64 {
                self.replies_count
            }
            fn likes_count(&self) -> i64 {
                self.likes_count
            }
            fn dislikes_count(&self) -> i64 {
                self.dislikes_count
            }
            fn user_liked(&self) -> bool {
                self.user_liked
            }
            fn user_disliked(&self) -> bool {
                self.user_disliked
            }
            fn addressed_to_name(&self) -> Option<&str> {
                self.addressed_to_name.as_deref()
            }
            fn addressed_to_tag(&self) -> Option<&str> {
                self.addressed_to_tag.as_deref()
            }
            fn author(&self) -> Option<&rise_proto::comment::ProtoAuthorInfo> {
                self.author.as_ref()
            }
            fn media(&self) -> Option<&rise_proto::comment::ProtoCommentMedia> {
                self.media.as_ref()
            }
            fn entities(&self) -> Option<&str> {
                self.entities.as_deref()
            }
            fn is_first(&self) -> bool {
                self.is_first
            }
            fn moderation_status(&self) -> &str {
                &self.moderation_status
            }
        }
    };
}

rise_comment_wire_row!(rise_proto::comment::ProtoCommentResponse);
rise_comment_wire_row!(rise_proto::comment::ProtoCommentWithPreview);
rise_comment_wire_row!(rise_proto::comment::ProtoUserCommentItem);

pub trait UserCommentPostPreviewDTOExt {
    fn as_post_preview(&self) -> Option<UserCommentPostPreview>;
}

impl UserCommentPostPreviewDTOExt for UserCommentPostPreviewDTO {
    fn as_post_preview(&self) -> Option<UserCommentPostPreview> {
        if self.id == 0 {
            return None;
        }
        Some(UserCommentPostPreview {
            id: self.id.to_string(),
            title: self.title.clone(),
            image: self.image.clone(),
            author_id: self.author_id.clone(),
            author: self.author.as_ref().map(|author| author.as_author_info(&self.author_id)).unwrap_or_default(),
        })
    }
}

pub trait GetUserCommentItemDTOExt {
    fn as_user_comment_entry(&self) -> Option<UserCommentEntry>;
}

impl GetUserCommentItemDTOExt for GetUserCommentItemDTO {
    fn as_user_comment_entry(&self) -> Option<UserCommentEntry> {
        let mapped_comment = self.as_comment()?;
        Some(UserCommentEntry {
            comment: mapped_comment,
            post: self.post.as_ref().and_then(UserCommentPostPreviewDTOExt::as_post_preview),
            post_card: self.post_card.as_ref().and_then(GetFeedPostItemDTOExt::as_feed_response),
            parent: self.parent.as_ref().and_then(RiseCommentWireRow::as_comment),
        })
    }
}

pub trait GetUserCommentsGatewayDTOExt {
    fn as_user_comment_entries(&self) -> Vec<UserCommentEntry>;
}

impl GetUserCommentsGatewayDTOExt for GetUserCommentsGatewayDTO {
    fn as_user_comment_entries(&self) -> Vec<UserCommentEntry> {
        self.items.iter().filter_map(GetUserCommentItemDTOExt::as_user_comment_entry).collect()
    }
}
